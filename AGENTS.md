# simple-markdown-editor — agent & contributor guide

Native WYSIWYG Markdown editor in Rust. Windows first, portable later.

This project follows the Bearing Software baseline at
[`bearingsoftware/code-standards`](https://github.com/bearingsoftware/code-standards), pinned to
**`v1.2.0`**. Read the baseline first — it owns the generic rules (git/PR workflow, work tracking,
error handling, testing, security, observability). This file holds **only** project specifics.
Don't duplicate baseline rules here.

**Keep this file under 20 KB.** It is loaded into an agent's context at the start of every session,
so every byte is paid for on every task. It reached 111 KB once; the history that made it that big
now lives in [`docs/decisions/`](docs/decisions/README.md), one file per area. Put a new decision
there and a link here only if the link is load-bearing.

## Baseline (read these)

- Cross-cutting: `code-standards/standards/_cross-cutting.md`
- Architecture: `code-standards/standards/architecture.md`
- Rust: `code-standards/standards/rust.md` · review: `code-standards/review/rust.md`
- Reviewing a PR: `code-standards/review/_template.md`

Tool configs are vendored from that tag — see [Configs](#configs).

## Where everything is written down

| Read this | For |
| --- | --- |
| [`docs/decisions/`](docs/decisions/README.md) | **Why the editor is the way it is.** One entry per decision, named for its ticket. Read the file for the area you are about to change. |
| [`docs/research/README.md`](docs/research/README.md) | The briefs. **Read before proposing architecture** — the major decisions are made and evidenced. Don't relitigate without new evidence. |
| [`docs/build-and-test.md`](docs/build-and-test.md) | What is behind the commands below: nextest, fuzzing, mutants, coverage, and what CI cannot see. |
| Confluence | Per-ticket plans, living documents. Linked from each Jira issue. |

## Work tracking

- **Jira project:** `MDEDIT` — https://chattersley.atlassian.net/browse/MDEDIT
- **Confluence space:** `MDEDIT` — https://chattersley.atlassian.net/wiki/spaces/MDEDIT/overview
  (`Features` and `Bugs` parent pages)
- **Build order:** [after the editing wave](https://chattersley.atlassian.net/wiki/spaces/MDEDIT/pages/2234777601)

Commit scope is the ticket key: `feat(MDEDIT-12): add marker locator`. Statuses are
`To Do → In Progress → In Review → Done` (this project's board adds `In Review` over the baseline
flow).

## Repo state

**A working editor on Windows.** It does not run at all off Windows — there is no portable text backend, which is the first known gap.

The crates below the app are headless and platform-agnostic, and that is the line worth keeping:
everything that can be tested without a window is.

```
crates/offsets/       # ByteOffset, ByteSpan, CharOffset, LineIndex, LayoutPos, BlockId. Depends on
                      #   nothing, deliberately
crates/language/      # What the editor needs from a language: spans with a role, block extents,
                      #   whether syntax is ever concealed, what is wrong with it
crates/json/          # Error-tolerant, span-producing JSON lexer, plus a separate strict check
crates/yaml/          # The same for YAML, hand-written — read its crate docs before proposing a
                      #   parser crate
crates/xml/           # The same for XML. Nesting is a counter, not a stack; the one language that
                      #   answers analyze_region with None
crates/toml/          # The same for TOML. Its own lexer, not INI's; a table header at column zero
                      #   is a safe restart point
crates/ini/           # The same for INI and .cfg. There is no INI specification, so its crate docs
                      #   state the dialect it reads
crates/highlight/     # Fenced-code highlighting, by info string only. syntect + fancy-regex, never onig
crates/markers/       # Markdown's implementation. Partitions source into marker and content spans
                      #   and reports block extents, in one parse
crates/image/         # An image destination's trust boundary, and a PNG decoder behind size caps.
                      #   Read its `resolve` docs before touching a path that came from a document
crates/document/      # Rope buffer, selection, concealment predicate, heading trail, word and line
                      #   extents, undo transactions, many-site replace, inline formatting toggles
crates/text/          # The text-layout trait. No platform types cross it
crates/text-dwrite/   # DirectWrite backend. Windows-only; one of three crates allowed `unsafe`
crates/menu-win/      # The native menu bar and its appearance, plus the small Win32 reads not worth
                      #   a crate of their own. Windows-only
crates/shell-win/     # The DropTarget verb and its COM local server. Windows-only
crates/file/          # Reads and writes without changing an unedited byte; the mtime+length stamp
crates/session/       # Tabs as handles, the session file, crash-recovery snapshots, recent files
crates/app/           # The window: winit + wgpu, glyph atlas, layout cache, chrome, find bar,
                      #   settings, latency harness. Binary: `mdedit`
assets/icons/         # The app icon and the script that builds `mdedit.ico` without ImageMagick
fuzz/                 # A target per span-producing lexer, excluded from the workspace on purpose.
                      #   Linux-only
spikes/               # MDEDIT-5 font fallback, MDEDIT-19 GPU baseline, MDEDIT-20 startup,
                      #   MDEDIT-67 clipboard threads, MDEDIT-123 WARP-in-CI. Re-runnable probes
docs/                 # decisions/, research/, build-and-test.md
```

**Known gaps and every decision behind the above are in [`docs/decisions/`](docs/decisions/README.md).**
Nothing in this file supersedes them; they are the detail this one deliberately does not carry.

## Stack

Decided in [`docs/research/02-gui-toolkits.md`](docs/research/02-gui-toolkits.md). The text view is ours; no toolkit has a rich-text editing widget.

```
Window/input      winit + muda (native menus) + rfd (dialogs)
Renderer          wgpu, hand-written quad + glyph pipeline    [NOT vello]
Text layout       parley + harfrust + fontique + skrifa       [layout only, NOT parley's editor]
Buffer            ropey 1.x                                   [pin 1.x; 2.0 has been beta ~21 months]
Word boundaries   unicode-segmentation                        [UAX #29; one definition of "word"]
Markdown parse    pulldown-cmark                              [full re-parse; no incremental]
Code fences       syntect + two-face, lazy, per-block cached
Accessibility     accesskit
```

Rejected, with evidence, so nobody re-proposes them: **Slint**, **Tauri/webview**, **GPUI**, **Xilem/Vello**, **iced**, **tree-sitter-md**. The evidence is in [`docs/research/02-gui-toolkits.md`](docs/research/02-gui-toolkits.md).

## Build & test

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features   # what CI runs; needs `cargo install cargo-nextest --locked`
cargo test --all-features --doc    # nextest does not run doctests, so this is the other half
cargo doc --no-deps --all-features # broken intra-doc links are denied, and CI runs this
cargo deny check                   # licences + advisories

cargo +1.90.0 check --all-features                            # MSRV (raised from 1.87 by muda 0.21)
cargo check --target x86_64-unknown-linux-gnu --all-features  # Windows-only code creeps in easily
```

**The toolchain is pinned in `rust-toolchain.toml`**, so a clippy release cannot red an unrelated PR;
the two commands above that mean a different version say `+1.90.0` and `+nightly` outright, because a
bare `cargo` obeys the pin. See [`docs/build-and-test.md`](docs/build-and-test.md).

Both of the last two are easy to skip and expensive to miss. **CI cannot open a window**, so
rendering and hit-testing are verified by hand on Windows — which is where this project's defects
actually come from.

**But rendering needs a device, not a window** (MDEDIT-123). Read [`docs/research/07-rendering-in-ci.md`](docs/research/07-rendering-in-ci.md) before assuming the sentence above closes the question.

**`crates/app/tests/render.rs` draws a real frame** (MDEDIT-153): the real pipelines and bind groups, one
frame on WARP, asserting ink. **Add to it rather than assuming rendering is unreachable.** What it
still cannot see is whether the result *looks* right — see `docs/build-and-test.md`.

**Some tests ask the real font instead of the stub**, all Windows-only: `icons.rs` (MDEDIT-92) for what is in a font, `table_metrics.rs` for column widths, `geometry.rs` (MDEDIT-133) for the frame as a whole, and a test apiece in `quotes.rs`, `images.rs` and `thematic_break.rs`. An assertion goes there **only if real metrics are what answer it**, and it asserts a relation, never a magnitude, because a font update moves every advance in the file.

**`mdedit-app`'s integration tests are one binary**, `crates/app/tests/main.rs` (MDEDIT-182). **A new test file is declared in `main.rs` or it never compiles** — `every_test_file_is_declared_here` fails on one that was forgotten. One file's tests: `cargo nextest run -p mdedit-app -E 'test(/^quotes::/)'`.

See [`docs/build-and-test.md`](docs/build-and-test.md) for fuzzing, mutation testing, coverage,
benchmarks and why each job runs where it does.

## Rust, beyond the baseline

`standards/rust.md` owns the general rules. Only these are project decisions:

- **No `async` in the editor core**, and adding a runtime needs a design decision on the ticket. The UI thread is a synchronous frame loop. **There are exactly two worker threads: one decodes pictures** (`decoding.rs`, MDEDIT-156) **and one highlights fenced code** (`highlighting.rs`, MDEDIT-190). A third needs a number from a cost harness, not an argument.
- **`unsafe` anywhere outside a platform crate is a review block**, not a discussion — stricter than
  the baseline's `deny` + scoped-allow. **Three** crates qualify: `mdedit-text-dwrite` for
  DirectWrite, `mdedit-menu-win` for the call that attaches a menu bar to a window, and
  `mdedit-shell-win` for COM class registration and `IDropTarget`. A **fourth** needs a reason on a
  ticket, not a `SAFETY` comment in `mdedit-app`.

  **`mdedit-menu-win` is where a stray Win32 call goes**, despite its name: one Win32 call is not a reason to mint a crate.
- **Newtype every offset**: `ByteOffset`, `CharOffset`, `LineIndex`, `BlockId`, `LayoutPos`. They live in `mdedit-offsets`, which depends on nothing.
- **A byte range is a `ByteSpan`, not a `Range`.** `Range<ByteOffset>` cannot index a `str`, so spans carry their own type and slice via `ByteSpan::slice`. Converting raw offsets in or out is deliberate and confined to boundaries: `pulldown-cmark` event ranges, `ropey`'s character indices, and test fixtures.

## Project-specific rules

### The byte-preservation invariant — the one rule that outranks everything

- **The buffer is the source of truth. Never re-serialize the document.** Opening a file and saving it must produce an identical byte sequence.
- Inline content is **never** serialized — it is decorated in place, so emphasis and hard-break style survive.
- Only the block under the cursor may be re-serialized, and only on exit, spliced back at its span.
- A diff that introduces a whole-document serializer needs an explicit design decision on the ticket.
- **Three rewrites are sanctioned**, each because the user explicitly asked for it, each one `replace`
  inside a transaction so one Ctrl+Z restores every byte, and **none reachable from a save or a
  timer**: JSON's Tools → Format Document (MDEDIT-48), an encoding change (MDEDIT-42), and a
  line-ending conversion (MDEDIT-59). The line is between "the editor reformatted my file" —
  forbidden, permanently — and "I asked it to". Citing them to justify a fourth is not what they say.

### Marker locator (`mdedit-markers`)

- No Rust Markdown parser emits delimiter nodes, so we compute them. This is the highest-risk
  component — see [`docs/research/03-architecture.md`](docs/research/03-architecture.md).
- **Property-tested against the CommonMark spec suite with one invariant: concatenating all marker
  and content spans, in order, reproduces the source byte-for-byte.** If that test is red, nothing
  else in the editor is trustworthy. It is the gate, not a nicety.
- Known-wrong upstream spans to work around: nested emphasis, escapes, collapsed links
  (`pulldown-cmark#1077`), image titles with no recoverable offset (`#441`), fence info strings
  (`#271`).

### Rendering & virtualization

- **Never materialize the whole document.** Render visible blocks only; `file open → first paint` is O(1), mmap plus viewport parse.
- **A tab is a lightweight handle until it becomes visible.** Session restore persists handles, not content.
- Make the **renderer** incremental, not the parser. Diff the block list, patch changed blocks.
- **The layout cache is keyed by content, never by position.** `BlockId` is an index into one analysis, so a cache keyed on it would serve the wrong layout for everything below an edit. The key is a hash of the block's bytes plus its kind and the wrapping width. The block holding the caret is cached like any other.
- **Preview draws no Markdown syntax at all, including under the caret** (MDEDIT-118). So a marker is
  not editable in place, and the caret steps over a concealed one rather than through its bytes.
  `Ctrl+/` is the escape hatch and the only mode that draws a marker.
- **A concealed marker is substituted, never revealed** (MDEDIT-141). Where the marker *is* the meaning — a list bullet, a number — `view::substitute` draws a stand-in rather than dropping the run. Read `StyledRun::substituted` before changing what a glyph points at. A wrapped item hangs by **placing** each item's layout (`list.rs`).
- **All chrome draws after the document**, with no depth buffer or scissor, so painter's order is the answer. Z-order rules for hit-tested panels: [`docs/decisions/chrome.md`](docs/decisions/chrome.md).
- Tables and code fences are **model-backed block widgets**, not decorations. A table places its cells into one layout whose lines are its rows, and `hit.rs` takes a grid's row spans, not newline counts. Read `crates/app/src/table.rs` before proposing one for code fences.
- **A caret is an offset *and* an affinity** (MDEDIT-188). Only a point gesture sets it; everything else gets `Downstream`.

### Text layer

- Text layout sits behind a narrow internal trait with **two implementations: DirectWrite (Windows)
  and parley (portable).** Not a precaution — **DirectWrite is required on Windows today**, because
  fontique resolves `script = Hani` to a single fallback family and stops
  ([parley#644](https://github.com/linebender/parley/issues/644)). A Markdown editor that cannot
  render 汉 is not shippable.
- **Do not use parley for text layout on Windows** until `spikes/font-fallback` passes there against a *released* fontique. Fix tracked in [parley#689](https://github.com/linebender/parley/pull/689).
- Pin parley to an exact version. Wrap it; don't leak its types into the editor core.
- IME is IMM32 via winit for v1, but **the buffer sits behind a trait that can later back an `ITextStoreACP`**.

### Windows integration

- **Opening N files from Explorer must reuse one process.** A **DropTarget verb** (CLSID + `LocalServer32`), not a command-line verb. Done on MDEDIT-61.
- **Declare Per-Monitor DPI v2 in the manifest**, via `embed-manifest`. Re-run glyph rasterisation and hit-testing on `WM_DPICHANGED`, and use the suggested rectangle.
- Read the dark-mode setting from the **registry**, not `ShouldAppsUseDarkMode()`. Refresh on `WM_SETTINGCHANGE` / `ImmersiveColorSet`.
- **Per-user install. Never elevate.** Elevation breaks drag-drop from a non-elevated Explorer.

### Security (this project's specific threat model)

- **Any auto-updater verifies signatures, pinned.** Notepad++ shipped an unverified updater, got its
  host compromised for ~6 months, and landed on CISA KEV with a nation-state actor delivering Cobalt
  Strike through it. The single highest-consequence design decision in the project.
- **Treat our own config and session files as untrusted input.** Never route a config string into a
  process launch; normalise every path before a prefix check. **No setting may name a path.**
- File parsing is a trust boundary. The 2023 Notepad++ CVEs were all heap overflows in hand-rolled
  encoding detection — the class Rust removes, provided we don't reintroduce it in `unsafe`.

### Performance ([`docs/research/06-performance-targets.md`](docs/research/06-performance-targets.md))

- Targets are commitments, not aspirations: cold start ≤100 ms, keystroke→pixel p50 ≤3 ms, **p99 ≤8.3 ms**, file open → first paint ≤50 ms at any size. Cold start is *recorded* rather than budgeted (MDEDIT-20).
- **Measure p99 and measure on battery.** Means on AC power are the least informative condition.
- **Two keystroke numbers, and the second has a floor.** Compare builds on *keystroke → quads ready*; use *keystroke → present* only to confirm frames are still presented.
- Baseline RSS has a floor we do not control, so the budget is what sits above it.
- A change that regresses a target needs a number in the PR body, not an assurance.

## High-signal findings to watch for in review

- **Whole-document serialization** sneaking in — breaks byte preservation.
- **Marker-locator property test weakened or skipped** to make a case pass.
- **A block materialized outside the viewport** — reintroduces the large-file freeze.
- **parley or DirectWrite types leaking past the text trait.**
- `unwrap()` / `expect()` on a path reachable from user input or file contents.
- `unsafe` outside the platform layer, or without a `// SAFETY:` comment.
- Offsets passed as bare `usize` between coordinate spaces.
- An updater or plugin path that loads code without signature verification.
- `async` creeping into the editor core.
- **A test that drives the thing it is meant to be checking.** Several here were vacuous until a mutation proved it.

## Configs

Vendored from the baseline tag `v1.2.0`:

| Tool | Project file | Baseline source |
|------|--------------|-----------------|
| rustfmt | `rustfmt.toml` | `configs/rustfmt.toml` |
| clippy | `clippy.toml` + `[workspace.lints]` in `Cargo.toml` | `configs/clippy.toml` |
| cargo-deny | `deny.toml` | `configs/deny.toml` |

`clippy.toml` tunes lints but cannot enable them; the lint set lives in `[workspace.lints]`. Set `msrv` to match `rust-version`.

Layer enforcement needs no config: workspace crate `[dependencies]` are the allow-list (`standards/architecture.md`).

Bump the pinned tag deliberately; re-vendor the configs when it changes.
