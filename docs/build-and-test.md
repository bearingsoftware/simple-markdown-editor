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
against the `meaningful_bytes` oracle it shares with the property test, the YAML lexer, the XML one,
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

The `*_cost.rs` files are the other half of this: ignored tests that print a number and assert
nothing — `json_typing_cost`, `validate_cost`, `wrap_cost`, `find_cost`, `list_cost` and
`frame_cost` in `crates/app/tests`, plus `decode_cost` in `crates/image` and `load_cost` in
`crates/file`. Each uses only long-standing API on purpose, so the same file can be copied into a
worktree of an older commit and the two runs compared.

The last two are the ones that answered MDEDIT-151 — whether the worker thread `AGENTS.md` described
was worth building, given it did not exist. They said no for saving, **yes for image decode** (now
MDEDIT-156), and *nothing conclusive* for loading, which measured 40–99 ms at 16 MB against a 50 ms
target. That is why these two print best **and** worst of their iterations where the older harnesses
print one figure: five runs of the decode harness gave 65, 84, 70, 159 and 130 ms for the same work.
**Run a cost harness on an idle machine** — the 159 came from a run overlapping a workspace build, and
a laptop that has just compiled the workspace is not the machine the earlier figures came off either.
A harness written to settle an argument is worth keeping for the next one.

`cargo test --release -- --ignored` is otherwise reserved for perf assertions. **None exist yet**, and they will
not run in CI: GitHub runners are virtualised and on AC power, which
[`docs/research/06-performance-targets.md`](research/06-performance-targets.md) identifies as the least
informative condition. They belong in a local harness whose numbers go in the PR body.

## Line endings

Pinned to LF by `.gitattributes`, because `rustfmt.toml` sets `newline_style = "Unix"` and Windows
checkouts default to `core.autocrlf=true`.

## The gap none of this closes

**CI cannot open a window**, and it never will. But it took MDEDIT-146 to show how much of "rendering"
that sentence was quietly being read to cover, and the answer was too much — see
[what the window was never needed for](#what-the-window-was-never-needed-for) below.

Hit-testing and *appearance* are still verified by hand on Windows, and that is where this project's
defects come from — five in the toolbar wave alone, every one with a green suite behind it, and three
of those were cases where the headless test stub *itself* was the lie.

`crates/app/tests/icons.rs` is the one file that asks the real font stack rather than the stub, and it
exists because measurement needs no window.

**Whether *rendering* needs one was MDEDIT-123, and the answer is no** — measured on a hosted runner,
which turns out to offer no hardware adapter at all and does not need to. `Editor` is generic over
`TextLayout`, so an `Editor<DirectWriteText>` builds a real frame headlessly in 3 ms; and WARP runs
this project's own shader, atlas and quads offscreen at 550 ms for the first frame (the driver
compiles the shader on first draw) and 2.5 ms for every one after it.

The full measurement, including the two machines that produced byte-identical pixels and why that is
evidence rather than a licence to commit a golden hash, is in
[`docs/research/07-rendering-in-ci.md`](research/07-rendering-in-ci.md).

### What the window was never needed for

`crates/app/tests/render.rs` (MDEDIT-153) asks WARP for a device, builds the **real** bind-group
layout, samplers, pipelines and bind groups out of `crate::gpu`, draws one frame into an offscreen
`Rgba8Unorm` texture and reads it back. Under a second, and it runs in the ordinary suite rather than
behind `#[ignore]` — a test CI has to be told to run is a test that stops being run.

**It exists because MDEDIT-146 aborted on a user's first run, past twelve green jobs and 1602 tests:**

```text
In Device::create_bind_group, label = 'globals'
  Sampler binding 2 expects filtering = false, but given a sampler with filtering = true
```

A `Linear` sampler bound into a layout declaring `NonFiltering`. No amount of reading found it — two
review passes went by — because it is not visible in any one function. It lives in the *interaction*
between the layout, the sampler and the moment they are bound together, and one draw call finds it in
0.19 s. That is why the test calls the production functions rather than a replica: a replica of that
wiring would have been written correctly and proved nothing.

**It asserts ink, not survival**, and the distinction is the whole lesson. MDEDIT-146's by-hand check
ran twice and passed twice *without ever drawing a picture* — session restore won the active tab, so
"it did not crash" was satisfied by a process that never reached the code under test. So each pipeline
draws into its own region and each region is checked, and the test **requires** an adapter rather than
skipping when one is absent: WARP ships with Windows, so no adapter means something is wrong.

**What it still does not do is look at the result.** Nothing is golden, no hash is committed, no
reference image exists — finding 3 of the brief explains why that should stay true. Whether a picture
is the right size, whether a table's text is centred in its row, whether a caret is in a sensible
place: those are still eyes on Windows.

## The cache saves on `main` and nowhere else

MDEDIT-132, and the largest single CI win this repository had available. Measured on the `check` job
of run 31839765178:

```
20:51:02  Restore Key: v0-rust-check-Linux-x64-b989424d
20:51:02  No cache found.
20:53:54  real work finishes                       (3m11s)
20:53:54  Saving cache …
21:10:31  done — 515,435,977 bytes at 0.4–0.5 MB/s (16m37s)
```

**Sixteen and a half minutes of a twenty-minute job spent uploading a cache whose restore had already
missed**, and the miss is structural rather than unlucky: rust-cache's workspace hash covers
`Cargo.lock` and every `crates/*/Cargo.toml`, so any change touching a manifest is a guaranteed full
miss followed by a full upload that the next manifest change also misses. Across the self-hosted legs
that was roughly 920 MB a pull request — `check` 491–546 MB, `status` 394–449 MB, `msrv` 158 MB,
`mutants` 66–70 MB, `fuzz` 31–40 MB × 5.

So every `Swatinem/rust-cache` step carries `save-if: ${{ github.ref == 'refs/heads/main' }}`. Pull
requests restore what `main` last saved and upload nothing. What that gives up is a second push to a
branch restoring the first push's cache, which is worth much less than sixteen minutes —
`concurrency` cancels the earlier run, frequently before it reaches the save step at all.

**That was not the end of it, and turning saving off is what made the rest measurable.** With the
save gone the restore became the whole job, so the two heaviest jobs also stopped caching `target/`:

| `check` (ubuntu) | restore | work | save | job |
| --- | --- | --- | --- | --- |
| before | miss, instant | 3m11s cold | **16m37s**, 515 MB | **19m56s** |
| `save-if` only | **5m02s**, 695 MB @ 2.3 MB/s | 2m58s | none | **8m00s** |
| `+ cache-targets: false` | instant | 4m32s cold | none | **4m32s** |

Five minutes to skip three is not a cache. `msrv` went 4m21s → 1m33s the same way, and had least to
gain in the first place: it runs a bare `cargo check` and was paying 158 MB for a `target/` cache.
`~/.cargo` is still cached — the registry and git checkouts are small and stay valid across the
manifest changes that invalidate everything else.

**Note the asymmetry, because it is what makes the arithmetic work**: uploads measure 0.4–0.5 MB/s
from these runners and downloads 2.3 MB/s, so a save costs roughly five times what restoring the same
bytes does. Both are the GitHub Actions backend rather than the box, so this was all true before the
runners became ephemeral; the conversion only stopped a warm local `target/` from partly masking it.

Left caching `target/`: `fuzz` at 31–40 MB a target and `mutants` at 66–70 MB, both small enough that
the round trip is not what costs them, and `project-status`, which re-runs the whole suite under
instrumentation on `main` where saving is wanted. A shared `shared-key`, so the Linux jobs restore one
cache instead of five job-scoped ones, remains untried.

## One CI wrinkle worth knowing

The mutants job installs `cargo-mutants` **only when it is not already on the runner**, and for years
the comment beside it said `cache-bin: true` was what kept it there. **That was never what was
happening.** In run 31839765178 rust-cache logged `No cache found` at 20:51:02 and six seconds later
the guard reported the binary present, resolving in 26 ms — it came off a long-lived runner's
`~/.cargo/bin`, not off the GitHub cache. `mutants.yml` runs on `pull_request` only, so no
`main`-scoped cache could ever have existed for it to restore.

The guard still earns its place, for the reason it was written: `cargo install` *stops* rather than
skips when it finds its own binary in place — `binary 'cargo-mutants' already exists in destination`,
exit 101, a red X on a job whose whole point is to report rather than block. The first sharded run
walked straight into it: the prebuilt binary was on the box, would not run, and the fallback install
refused because the file was there. Both fallbacks pass `--force` now.

**The prebuilt route was re-tested on 2026-09-16, and it still does not work** (MDEDIT-132, answered
by MDEDIT-172). `taiki-e/install-action` installs happily and the binary then dies with `version
'GLIBC_2.39' not found`, so the job asks it for `--version` and treats a binary that cannot run
exactly like one that is missing: it builds from source instead, and prints what the binary said, so
the next person gets an answer rather than "no usable prebuilt binary". The install step is
`continue-on-error`, so it cannot publish a failing check on a job that must not block.

**The source build is paid once per run, not once per shard.** The plan job installs `cargo-mutants`
and uploads the binary; each shard downloads it, and falls back to building from source only if that
binary will not run there. Six shards compiling ~250 crates each would take more from the pool than
the sharding gives back.

## An allow-list nobody is reminded to extend

`mutants.yml` used to name its packages one `--package` at a time, and `mdedit-xml`, `mdedit-toml` and
`mdedit-ini` shipped in MDEDIT-94/95/96 without reaching the list. So a pull request touching a
brand-new lexer mutated nothing and still reported green — the quietest way for a check to stop being
one. That was fixed by adding the three names, and this section then said the right answer was
`--exclude` for the three `cfg(windows)` crates instead, "which fails loudly when a crate appears
rather than silently skipping it".

**The trap was left in place and it caught three more (MDEDIT-155): `mdedit-app`, `mdedit-image` and
`mdedit-text`.** Worse than the first time. `mdedit-app` is where nearly every defect of MDEDIT-146
through MDEDIT-154 lived, and `mdedit-image` holds the destination resolver — the trust boundary
between a Markdown file and a UNC path that leaks an NTLM hash. Four pull requests cited
"cargo-mutants passes on the diff" as evidence their new tests were not vacuous; every one of those
runs had mutated nothing.

It is now an **exclude list**: the three Windows crates by name, everything else in by default and
for good. A crate is covered the day it is created.

**And the job now says when it tested nothing.** `cargo-mutants` exits 0 both for "killed every
mutant" and for "generated none", which is what let this hide for so long. The absence of
`mutants.out/` is the signal, and it becomes a warning annotation — a warning rather than a failure,
because zero mutants is often correct for a diff of tests, comments, docs or CI. The check is guarded
on a clean exit, or a run that died early would report the same thing and swallow a real failure.

**When reading that job, the question is not "is it green".** It is "how many mutants did it run".

**What it costs, and what it found the first time it ran for real** (MDEDIT-156, the first diff to
touch Rust source after the fix): `60 mutants tested in 35m: 17 missed, 31 caught, 12 unviable`, in a
44-minute job. That is the runtime MDEDIT-155 could not measure, because every local attempt was
killed for memory even at `--jobs 1`. It is not a required check, so a slow run holds nothing up — but
a *red* one still shows on the pull request, which is why the step below decides its own exit status
rather than leaving it to `continue-on-error`.

**That 35 minutes is most of what the job is allowed** (MDEDIT-171). The self-hosted pool stops every
job at 60 minutes, whatever the workflow asks for, and GitHub reports it about nine minutes later as
*"the self-hosted runner lost communication"* — with no step conclusion and no log. PR #166 lost both
attempts that way, at 69 minutes each, and the message reads like a crash rather than a limit. So the
job asks for 55 minutes, and the Mutate step has a **40-minute budget** of its own, set once as
`budget` in the step: when it runs out, cargo-mutants is interrupted, keeps what it found, and the job
says so as an error — a partial run is not a result. A diff of about sixty mutants fits; a bigger one
does not, and **the answer is to shard it, not to raise the budget.** There is no room above it to
raise it into.

**So it is sharded (MDEDIT-172), and the shape of that was measured rather than argued.** MDEDIT-171
made running out of time honest; it did not make the answer complete, and the next pull request, #168,
generated **82 mutants and got through 35**. The rest were run on a laptop, and two survivors came out
of the part CI never reached — so the untested remainder was not a formality.

A `plan` job now counts what the diff generates (`--list` parses rather than builds, so it costs
seconds) and emits a matrix of **fifteen mutants a shard, at most six**; each shard runs `--shard k/n`
over the same diff with the same 40-minute budget, its own baseline, its own annotations and its own
report artifact. Three runs over #168's diff set every number in it:

| Shards | Sharding | At once | Result |
| --- | --- | --- | --- |
| 3 × 30 | slice (the default) | 3 | Shard 0 took all 28 `mdedit-app` mutants and ran out of time; the other two finished early on `mdedit-document` |
| 5 × 20 | round-robin | 5 | Two shards managed 16–17 mutants in 41 minutes, three managed 9. 60 of 82 tested |
| 6 × 15 | round-robin | 2 | **All 82: 81 caught, 1 unviable, none missed, none timed out.** 46 minutes of wall clock |

Two things came out of that. **A mutant's cost is not uniform** — one in `mdedit-app` rebuilds the
largest crate and runs the largest test suite at roughly 90 seconds, against nearer 40 for one in
`mdedit-document` — so consecutive mutants are the expensive ones together, and round-robin gives
every shard the same mix. And **the pool's runners are not independent machines**: five shards started
together on five different runner VMs of one Proxmox host and got a fifth of it each, which is why
`max-parallel` is 2. Sharding wider makes each shard slower, which is the opposite of what a per-shard
budget needs.

Six shards of fifteen is 90 mutants before a diff is told to split, and #168 is the largest this
project has produced. `workflow_dispatch` takes a `base` ref so a real diff can be re-run by hand;
without one a dispatch diffs against `origin/` and mutates nothing.

The 17 were worth the 35 minutes. One of them was **a security cap with no test behind it**:
`within_the_pixel_caps` refuses a picture too wide, too tall, or too many pixels, and every test
exceeded the *width* — so `||` short-circuited and the other two clauses had never been evaluated by
anything. A 1×9000 PNG was accepted and would have aborted the process at texture creation. Another
was a decode path where **the tests used one route into the cache and the event loop used another**,
so the production route could be emptied without a single test noticing.

**Reproduce a survivor by hand before writing a test for it.** Both of those were confirmed by editing
the source, running the suite and watching it stay green — which is also how you tell a real survivor
from an equivalent mutant, and how you know afterwards that the new test actually kills it.

**The job reads what happened from `mutants.out`, not from the exit code** (MDEDIT-157). The second
run on that same pull request found no survivors at all and still went red: cargo-mutants exits **3**
for a timeout, and the job only knew 0 and 2. That was the mild half. The serious half is that
cargo-mutants ranks timeout *above* missed, so a run with survivors and one slow mutant also exits 3
— and the job reported it as broken without ever saying anything had survived.

So it now counts `missed.txt` and `timeout.txt`, one mutant per line, and warns on each. Exit 1, 4 and
above still fail, because those mean the job is broken rather than that it found something. And if
the exit code and the files disagree — 2 with nothing in `missed.txt` — it fails loudly rather than
guessing, because the install step takes whatever cargo-mutants is current and a release that renamed
those files would otherwise turn every survivor into a silent pass.

**A timeout is an unknown, not a pass.** The auto-set timeout is the baseline's test time times four,
and the baseline runs before the pool gets busy: PR #163's came out at 22 s and three mutants timed
out under load. None was a hang. Two fail in under a second on a laptop, so they were catches. **The
third was a survivor**: `user_event` replaced with `()`, which no test calls, and which the previous
run on the same pull request had reported as MISSED. One timeout in three was hiding a survivor, so
treating timeouts as "just the runner" would have been wrong a third of the time.
`--minimum-test-timeout 60` puts a floor under the timeout, so a slow survivor is reported as a
survivor. Reproduce a timed-out mutant locally before reading it either way.
