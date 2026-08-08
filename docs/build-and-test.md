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

A separate workflow and a **separate workspace** (MDEDIT-111). `fuzz/` holds four targets: the marker
locator against its tiling gate, JSON's validator, JSON's formatter against the `meaningful_bytes`
oracle it shares with the property test, and the YAML lexer.

```bash
cargo +nightly fuzz run markers_segment -- -max_total_time=60
```

Excluded from the workspace **on purpose**: `libfuzzer-sys` builds a C++ runtime and is Unix-only, so
a Windows `cargo check` would try to build it and fail. The four crates it covers are
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

## Benchmarks and perf assertions

```bash
cargo bench -p mdedit-markers     # segmentation cost against the keystroke budget
cargo bench -p mdedit-document    # what an undo group and a transaction cost against typing
```

Not CI jobs. Run by hand when touching segmentation.

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
