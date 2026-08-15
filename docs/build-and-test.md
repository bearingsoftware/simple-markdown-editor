# Build and test, in detail

The commands are in [`AGENTS.md`](../AGENTS.md). This is what is behind them.

## nextest, and why every test gets its own process

CI runs `cargo nextest run --profile ci --all-features` (MDEDIT-111). Plain `cargo test` still works
and is fine for a quick loop.

Process-per-test is the structural answer to the MDEDIT-67 class: two threads of one process reading
the Windows clipboard corrupt the heap, and that aborted a whole test binary — `context_menu` died
with `STATUS_HEAP_CORRUPTION` 15 runs in 20. Isolation means one bad test takes down one test.

There are **no retries** configured, deliberately. A flaky test that passes on the second attempt is a
flaky test nobody fixes. See `.config/nextest.toml`.

## What runs where, and why

CI runs fmt, clippy, tests, doctests and docs on **both Linux and Windows**. Windows is the primary
target and gates every PR rather than being a nightly afterthought; it also catches what Linux cannot,
which is line endings, path handling and the whole platform layer.

`cargo deny` and the MSRV check run on **Linux only** — neither is platform-sensitive.

Where each job runs is a cost decision, not a technical one (MDEDIT-115). This repository is private,
so every GitHub-hosted minute is billed and Windows bills at twice the Linux rate. Everything that can
run on the self-hosted Linux box does; the Windows leg stays hosted because there is no self-hosted
Windows runner and dropping it is not an option.

## Two checks that are easy to skip and expensive to miss

**MSRV.** `cargo +1.87.0 check --all-features`. The local toolchain is newer and will happily accept
things the floor rejects. Let-chains are the trap — they stabilised in 1.88.

**The Linux target.** `cargo check --target x86_64-unknown-linux-gnu --all-features`. Windows is the
only platform anything *runs* on, so it is easy to add a dependency or a `cfg` that only compiles
here. `rfd` needing GTK headers and a `cfg`-gated enum going dead both reached CI that way.

## Fuzzing

A separate workflow and a **separate workspace** (MDEDIT-111). `fuzz/` holds a target per
span-producing lexer: the marker locator against its tiling gate, JSON's validator, JSON's formatter
against the `meaningful_bytes` oracle it shares with the property test, the YAML lexer, the XML one
the TOML one and the INI one.

Three of them assert more than tiling. **XML's** input is what a fuzzer is unusually well suited to
producing — an unclosed tag, a stray angle bracket — which is exactly what that lexer exists to
survive. **TOML's** and **INI's** assert the restart claim: both answer `analyze_region`, so each
target restarts at **every** block and compares against the whole-document parse. A generated
document is where that claim breaks if it is going to.

That comparison is **per byte, not per segment**, and the fuzzer is what established the difference —
80 seconds into TOML's first run, on `"\n\u{8}"`. Adjacent runs of one kind coalesce, so a
whole-document parse can emit one segment *spanning* a block boundary that a region beginning there
cannot reproduce. Both cover the same bytes with the same kinds, so nothing was wrong; the assertion
was. `Document::resegment_scope` trims a straddling segment for the same reason, and neither
language's own property test had reached the case.

```bash
cargo +nightly fuzz run markers_segment -- -max_total_time=60
```

Excluded from the workspace **on purpose**: `libfuzzer-sys` builds a C++ runtime and is Unix-only, so
a Windows `cargo check` would try to build it and fail. The crates it covers are
platform-agnostic, which makes Linux-only the right home rather than a compromise.

Five minutes a target on a PR, half an hour on the Sunday schedule, corpus cached between runs, the
crashing input uploaded as an artifact. AddressSanitizer comes with it — the only place this project
gets one, because rustc has no sanitizer support on Windows.

It earned its keep immediately: `markers_segment` found a `pulldown-cmark` panic within four minutes
of its first real run (MDEDIT-114, reported upstream as
[pulldown-cmark#1129](https://github.com/pulldown-cmark/pulldown-cmark/issues/1129)). That target is
`continue-on-error` until upstream fixes it, because `libfuzzer-sys` installs a panic hook that
*aborts before unwinding* — so the editor's own `catch_unwind` containment is invisible to it, and the
target dies on somebody else's unwrap.

## Mutation testing

Runs on the diff, and **reports rather than blocks**.

```bash
cargo mutants --in-diff <(git diff origin/main...HEAD)
```

It changes a `<` to a `<=`, deletes a line, replaces a return value, and says what the tests still let
through — the mechanical version of the second reviewer a solo project does not have, and the class of
finding that produced MDEDIT-110. `--in-diff` keeps it to minutes; a whole-workspace run is hours and
would never be read.

A **surviving mutant is a question, not a defect.** Sometimes the right answer is a better assertion,
and sometimes it is that the mutated line genuinely does not matter. Promote the job to blocking only
if it earns it.

Doing this by hand before opening a PR is worth more than the job is: put the defect back, watch the
new test fail, put the fix back. Several tests in this repo were vacuous until that was done to them.

## Coverage

After CI passes on `main`, `.github/workflows/project-status.yml` measures unit coverage with
`cargo llvm-cov` and pushes it to the status board. `scripts/build-status.sh <sha>` builds both
`status.json` and the lcov bundle from one measurement and can be run locally.

The figure is the **Linux** one — the Windows-gated code has no regions to measure there, which is
worth knowing before concluding the platform layer is untested.

**The floors are what make the number a signal** (MDEDIT-130). `scripts/coverage-floors.json` is a
committed constant, never computed from the run being measured — a derived floor equals the actual
and can never be breached. The board turns *actual < floor* into a red dot; the script says the same
thing on stderr, so a local run does not have to wait for one. Raising a floor after coverage
improves is the ratchet, and lowering one is meant to be visible in a diff.

`menu-win` and `shell-win` carry no floor: on a Linux runner nearly all of both is `cfg`'d out, so
their percentages describe the Linux slice rather than the crate, and either would move on a `cfg`
change with no test touched. A floor whose crate produced no regions is inert, not a false breach.

## Benchmarks and perf assertions

```bash
cargo bench -p mdedit-markers     # segmentation cost against the keystroke budget
cargo bench -p mdedit-document    # what an undo group and a transaction cost against typing

# What one parser option costs (MDEDIT-131). Needs the feature; nothing else does.
cargo bench -p mdedit-markers --features bench --bench options
```

Not CI jobs. Run by hand when touching segmentation.

### Comparing two configurations: measure in pairs, and read the control first

**A criterion baseline cannot compare two builds on a machine that reclocks.** MDEDIT-126 tried and
failed: `segment/parse-only` constructs a `Parser` with no options and so cannot see any flag — an
accidental control group — and across six interleaved runs pinned to two CPUs its floor still drifted
−40% to +21%. Within-run confidence intervals were tight at ±5%, which is exactly what made the bad
runs look credible. **The drift is between runs, where criterion cannot see it.** That ticket shipped
with no number rather than a wrong one.

Putting both configurations in one process helps and is not enough on its own — two benches are still
seconds apart, and measured that way the same comparison read +3% in one run and +76% in the next.

`benches/options.rs` is the shape that works, and it is not a criterion bench:

- the two configurations are timed **microseconds apart, alternating**, and what is reported is the
  **ratio** — whatever the clock is doing, it does the same thing to both halves of a pair, so drift
  cancels instead of accumulating;
- **the control pairs a configuration against itself.** Identical work, so the answer must be 1.00,
  and anything else is the harness measuring its own noise. If the control is not tight around 1,
  nothing else in the run means anything.

It works: across two runs whose *absolute* times differed by 3× — 6.27 ms and 2.13 ms for the same
analysis — the measured ratio moved from ×2.09 to ×1.99. Copy the shape for any future
this-versus-that measurement rather than reaching for a criterion baseline.

`cargo test --release -- --ignored` is reserved for perf assertions. **None exist yet**, and they will
not run in CI: GitHub runners are virtualised and on AC power, which
[`docs/research/06-performance-targets.md`](research/06-performance-targets.md) identifies as the least
informative condition. They belong in a local harness whose numbers go in the PR body.

## Line endings

Pinned to LF by `.gitattributes`, because `rustfmt.toml` sets `newline_style = "Unix"` and Windows
checkouts default to `core.autocrlf=true`.

## The gap none of this closes

**CI cannot open a window.** Rendering and hit-testing are verified by hand on Windows, and that is
where this project's defects come from — five in the toolbar wave alone, every one with a green suite
behind it, and three of those were cases where the headless test stub *itself* was the lie.

`crates/app/tests/icons.rs` is the one file that asks the real font stack rather than the stub, and it
exists because measurement needs no window. Whether *rendering* needs one is MDEDIT-123.

## One CI wrinkle worth knowing

The mutants job installs `cargo-mutants` **only when it is not already on the runner**. `cache-bin`
restores `~/.cargo/bin` between runs, and `cargo install` refuses to overwrite a binary it finds
there — `binary 'cargo-mutants' already exists in destination`, exit 101. That put a red X on every
pull request for a job whose whole point is to report rather than block, which is exactly the way to
teach people to ignore it.

`--force` would also fix it, by rebuilding from source every run. That is the cost the cache exists
to avoid.
