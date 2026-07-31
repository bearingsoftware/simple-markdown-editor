# simple-markdown-editor — agent & contributor guide

Native WYSIWYG Markdown editor in Rust. Windows first, portable later.

This project follows the Bearing Software baseline at
[`bearingsoftware/code-standards`](https://github.com/bearingsoftware/code-standards), pinned to
**`v1.2.0`**. Read the baseline first — it owns the generic rules (git/PR workflow, work tracking,
error handling, testing, security, observability). This file holds **only** project specifics.
Don't duplicate baseline rules here.

## Baseline (read these)

- Cross-cutting: `code-standards/standards/_cross-cutting.md`
- Architecture: `code-standards/standards/architecture.md`
- Rust: `code-standards/standards/rust.md` · review: `code-standards/review/rust.md`
- Reviewing a PR: `code-standards/review/_template.md`

Rust landed in the baseline at `v1.2.0` ([#20](https://github.com/bearingsoftware/code-standards/pull/20),
[#21](https://github.com/bearingsoftware/code-standards/pull/21), MDEDIT-1). Tool configs are vendored
from that tag — see [Configs](#configs).

## Work tracking

- **Jira project:** `MDEDIT` — https://chattersley.atlassian.net/browse/MDEDIT
- **Confluence space:** `MDEDIT` — https://chattersley.atlassian.net/wiki/spaces/MDEDIT/overview
  (`Features` and `Bugs` parent pages)

Commit scope is the ticket key: `feat(MDEDIT-12): add marker locator`. Statuses are
`To Do → In Progress → In Review → Done` (this project's board adds `In Review` over the baseline
flow).

## Repo state

**Document core only — no renderer, no window, no file I/O.** Everything that exists is headless and
platform-agnostic.

```
crates/offsets/         # mdedit-offsets: ByteOffset, ByteSpan, CharOffset, LineIndex, LayoutPos,
                        #   BlockId
crates/language/        # mdedit-language: what the editor needs from a language — spans with a
                        #   role, block extents, and whether syntax is ever concealed. Depends on
                        #   `mdedit-offsets` and nothing else, deliberately
crates/json/            # mdedit-json: an error-tolerant, span-producing JSON lexer. No serde —
                        #   a value has no spans and a strict parser is useless mid-edit
crates/yaml/            # mdedit-yaml: the same for YAML, hand-written for evidenced reasons —
                        #   read its crate docs before proposing a parser crate
crates/markers/         # mdedit-markers: Markdown's implementation of that trait. Partitions
                        #   source into marker and content spans, and reports block extents — one
                        #   parse for both
crates/document/        # mdedit-document: rope buffer keeping that partition current, plus
                        #   selection, the reveal-on-cursor predicate, and the heading trail to a
                        #   position
crates/text/            # mdedit-text: the text-layout trait. No platform types cross it.
crates/text-dwrite/     # mdedit-text-dwrite: DirectWrite backend. Windows-only; one of the two
                        #   places `unsafe` is permitted. Compiles to nothing elsewhere.
crates/menu-win/        # mdedit-menu-win: the native menu bar, and the one unsafe call that
                        #   attaches it. Windows-only. Where shell integration will go.
crates/file/            # mdedit-file: reads and writes files without changing an unedited byte
crates/session/         # mdedit-session: tabs as handles, and the session file that restores them
crates/app/             # mdedit-app: the window. winit + wgpu + glyph atlas, the decoration
                        #   step, per-block layout and its cache, the settings file, the status
                        #   bar, the tab strip and file pane, and the keystroke latency harness.
                        #   Binary: `mdedit`.
spikes/font-fallback/   # MDEDIT-5 probe; outside the workspace, Windows-only, throwaway
spikes/gpu-baseline/    # MDEDIT-19 probe: what an empty wgpu window costs. Re-runnable
docs/research/          # 7 briefs; read README.md first
  README.md             # verdicts, recommended stack, top 3 risks
  01-editor-landscape.md
  02-gui-toolkits.md    # why not Slint; toolkit comparison
  03-architecture.md    # THE design decision + marker locator spec
  04-parsing-benchmarks.md
  05-windows-platform.md
  06-performance-targets.md
```

Read `docs/research/README.md` before proposing architecture — the major decisions are already made
and evidenced. Don't relitigate them without new evidence.

Known gaps, so nobody assumes they are done:

- **No portable text implementation.** `mdedit-text-dwrite` is the only backend, so the trait has
  one implementation and therefore no correctness oracle yet. parley fills that in when
  [parley#689](https://github.com/linebender/parley/pull/689) ships released, and until then the
  app does not run at all off Windows.
- **IME composition is implemented but unverified at a real keyboard.** The pre-edit is drawn at the
  caret, the candidate window is told where the caret is, a commit is one edit and a cancel is none
  — and all of that is unit-tested. What is *not* confirmed is what it looks like and where the
  candidate list actually lands, because composition cannot be driven programmatically: with
  Microsoft IME (Japanese) installed and the layout active, injected input leaves the IME closed
  (`IMC_GETOPENSTATUS` stays 0) and no `WindowEvent::Ime` ever arrives. MDEDIT-6 carries the
  seven-step script for a human.
- **The pre-edit does not reflow the line it is on.** It is drawn over the following text on its own
  background rather than pushing it along, because it is deliberately not in the buffer and so
  cannot take part in layout. Fine at the end of a line, which is where composition usually happens;
  visibly an overlay in the middle of one.
- **There is a File and Edit menu, a status bar, and the open files shown either as a tab strip or
  as a left pane.** No scrollbar, no View menu, no context menu on right-click, no recent files. The
  plain-markup toggle is Ctrl+/ and the strip/pane toggle is Ctrl+B; neither has a menu item, because
  there is still no View menu to put one in.
- **Which chrome is showing is not remembered.** Ctrl+B swaps the strip for the pane and the editor
  starts on the strip every time — the config file is read once and never written back, so there is
  nowhere to put it that survives a restart. A `chrome` setting is the obvious home when someone
  wants one.
- **A pane row truncates its name; it does not ellipsise it, and there is no hover.** MDEDIT-28's
  plan asked for eliding with the full path on hover. Truncation is what the tab strip already does
  and what this matches; an ellipsis needs its own measurement pass, and a tooltip needs a hover
  affordance that does not exist yet.
- **The pane lists what is open, not what is on disk.** A file *tree* is a different feature with
  filesystem-watching decisions attached, and deliberately not this one.
- **The status bar's word count lags, deliberately.** It is recomputed on the same idle timer that
  runs the whole-document re-parse 150 ms after typing stops, because counting words is a walk of the
  buffer and doing it per frame would undo MDEDIT-17 and MDEDIT-18. While you type, the number shown
  is the one from before you started. A document nobody has typed in yet is counted once, on its
  first frame.
- **The breadcrumb shows headings, and nothing else.** A caret above the document's first heading
  gets an empty breadcrumb rather than a root, and a long trail is truncated rather than ellipsised
  or scrolled. Nothing in the bar is clickable.
- **Line and column count characters.** Not bytes, and not display columns — a tab is one column
  here. `ByteOffset` and `CharOffset` exist as separate types so this cannot be fudged; see
  `Document::line_and_column`.
- **The drawing mode is global and not remembered.** Toggling to plain markup switches every tab,
  and the editor opens previewed every time — the session file records handles rather than view
  state, and there is no config to put a default in until MDEDIT-22.
- **Menu accelerators are labels, not bindings.** The items show "Ctrl+S" and so on, but they do not
  fire: muda's accelerators need the message loop to call `TranslateAcceleratorW`, and winit's does
  not. That is deliberate — the key handler owns those chords, and two things acting on one Ctrl+Z
  would undo twice.
- **The clipboard is text only, and a paste keeps its own line endings.** An image on the clipboard
  is ignored, because a Markdown buffer cannot hold one and deciding where to write the file would be
  a feature rather than a paste. Pasted text goes in verbatim, so pasting CRLF into an LF file leaves
  mixed endings — the byte-preserving choice, and the opposite of what VS Code does. Revisit if it
  annoys anyone.
- **No double-click to select a word**, and no triple-click for a line. Every selection is a drag or
  a shift — and there is no menu to reach copy and paste from, only the chords.
- **Startup costs ~300 ms before anything is drawn**, and none of it is the document: a 5-byte file
  reaches first paint no faster than a 245 KB one. It is winit's window plus wgpu's device and
  pipeline bring-up. That is what stands between the editor and the ≤50 ms file-open target, and it
  is measured on every run — `mdedit` prints `launch -> first paint` on exit.
- **A keystroke re-parses a region, not the document.** MDEDIT-18: an edit re-parses the block it is
  in plus a block of margin either side, and splices that into the existing analysis. A region
  cannot see what the rest of the document does to it — an unclosed fence above, a link reference
  definition below — so the whole document is re-parsed 150 ms after the typing stops. Documents
  under 16 KB skip all of this and parse whole, exactly as they used to.
- **A chunk is a whole top-level block.** A document that is one enormous list or one enormous
  paragraph is one chunk, and no cheaper than before — only its *drawing* is culled, line by line.
  Splitting inside a block needs the layout engine to lay out part of one, which none of them does.
- **Off-screen heights are estimated**, at a line count times a nominal line height, so a long
  wrapped paragraph nobody has scrolled to yet is under-estimated. The estimate is corrected the
  moment a chunk is measured, and the viewport is anchored so the correction does not move what is
  on screen — but the document's total height, and therefore how far it can scroll, is approximate
  until it has all been looked at. This is what a scrollbar would make visible, and there is none.
- **Scrolling is vertical only, and instant.** No horizontal scroll (long lines wrap), no smooth or
  momentum scrolling, and no scrollbar to show where in the document the view is.
- **Undo groups break on caret movement, not on a timer.** Deliberate — see `crates/document/src/history.rs`.
  It means a long pause mid-word still undoes as one step, which is the one place the behaviour
  differs from editors that use a clock.
- **One setting, and no UI to change it.** `%APPDATA%\SimpleMarkdownEditor\config.json` holds
  `line_numbers`, read once at startup and never written back, so changing it means editing the file
  and restarting. **No setting may name a path** — see `crates/app/src/config.rs` for why that is a
  rule rather than a caution. JSON because `serde_json` was already here; the lack of comments is the
  known cost, and TOML is the thing to revisit if this grows past a handful of keys.
- **A line number counts source lines, not rows.** A wrapped paragraph is numbered once, at its first
  row, which is what every editor does. The gutter is sized for the largest number the document can
  show, with a two-digit floor, so it does not visibly resize as you pass line 9.
- **No encoding picker.** `mdedit-file` can read any encoding it is told to, but the app only ever
  calls `load`, which honours a BOM and otherwise assumes UTF-8. A windows-1252 file opens as an
  error rather than as text until there is UI to choose.
- **`.json` files open highlighted and never concealed**, by extension only — a JSON file named
  `data.txt` opens as Markdown, deliberately. Content sniffing is the trust boundary the 2023
  Notepad++ CVEs were all in, so nothing reads a byte of a file to decide what it is. The fix when
  somebody wants one is a language picker, which is a UI decision rather than a sniffing one.
- **`.yaml` and `.yml` open highlighted too, from a hand-written lexer — and the parser decision is
  evidenced, so read `crates/yaml/src/lib.rs` before proposing a crate for it.** Measured 2026-07-31:
  `syntect` **fails MSRV 1.87** (`plist` and `time` need 1.88); `tree-sitter` passes but is a C
  parser at a file-parsing trust boundary, which is the CVE class this project's language choice
  exists to remove; and a real YAML parser (`saphyr-parser`) **failed 3 of 4** mid-edit documents —
  a half-typed flow sequence, inconsistent indentation, and a tab — which is what a file being
  written looks like.
- **The YAML lexer highlights; it does not parse.** It understands comments, document markers, keys,
  sequences, quoted scalars, anchors, aliases, tags, flow collections and block scalars including
  their indentation. It approximates a key as text before `:` at end-of-line or `: ` before a
  comment, which is wrong for the explicit `? key` form and for a colon-space inside a flow scalar.
  Both mis-*colour* rather than mis-*cover*, and tiling is property-tested either way.
- **JSON and YAML always take a whole-document parse.** Both `analyze_region`s return `None`. JSON
  nests without limit and a region carries no record of how deep it started; YAML's indentation is
  significant, so a region cannot see that it has become the body of the block above. Neither has a
  blank line to resynchronise on the way Markdown does, so MDEDIT-18's reconcile could not repair a
  wrong regional parse. Correct and slower, and **the slower is measured: a 357 KB JSON types at p50
  6.2 ms / p99 13.4 ms and a 302 KB YAML at 6.3 / 13.9, against budgets of 3 ms and 8.3 ms.** So both
  miss the keystroke target by about double, and that is the whole cost of forgoing MDEDIT-18 for
  them. Markdown is unaffected (3.0 / 9.6 on the same machine, unchanged). MDEDIT-36 carries the fix,
  which needs the region
  parser to be told what depth it starts at.
- **There is a language boundary, and one language behind it.** MDEDIT-29 put Markdown behind
  `mdedit_language::Language` so JSON and YAML can follow, and changed nothing observable — the whole
  test suite passed unmodified, which was the acceptance criterion. A language answers three things:
  its spans, its block extents, and whether its syntax is ever concealed. That last one is the
  rendering model in a single bool: Markdown hides its syntax because the syntax is scaffolding for
  prose, and JSON must not because its braces *are* the document. Nothing but Markdown implements it
  yet, so the trait has no correctness oracle — the same position `mdedit-text` is in.
- **A language that cannot say where to restart gets a whole-document parse.** `analyze_region`
  returns `Option`, and `None` means "no safe boundary here". Correct and merely slower, which is the
  right trade — JSON nests without limit, so an edit deep inside an object may have no boundary short
  of the top.
- **Blocks are extents, not a tree.** `Document::blocks` is a flat list with a depth on each entry.
  Nothing walks parent or child links because nothing has needed to; `block_at` scans the list.
  Fine at present sizes, and the thing to revisit when virtualization needs to diff block lists.
- **No perf assertions.** The bench in `crates/markers/benches` measures; nothing gates a regression.
- **CI does not run the app**, only the headless crates. There is no way to open a window on a CI
  runner, so rendering is verified by hand on Windows.

## Stack

Decided in [`docs/research/02-gui-toolkits.md`](docs/research/02-gui-toolkits.md). No GUI toolkit —
none of them has a rich-text editing widget, so the text view is ours regardless.

```
Window/input      winit + muda (native menus) + rfd (dialogs)
Renderer          wgpu, hand-written quad + glyph pipeline    [NOT vello]
Text layout       parley + harfrust + fontique + skrifa       [layout only, NOT parley's editor]
Buffer            ropey 1.x                                   [pin 1.x; 2.0 has been beta ~21 months]
Markdown parse    pulldown-cmark                              [full re-parse; no incremental]
Code fences       syntect + two-face, lazy, per-block cached
Accessibility     accesskit
```

Rejected, with evidence, so nobody re-proposes them: **Slint** (plain-text-only `TextEdit`, rich
text explicitly display-only, unusable past ~5k lines, broken text rendering at fractional DPI),
**Tauri/webview** (webview typing latency ~20–31 ms vs a ≤3 ms target; open CJK IME freeze on
Windows), **GPUI** (unresolved GPL-contamination issue against its Apache-2.0 claim; publicly
deprioritized by Zed), **Xilem/Vello** (alpha; Vello mandates compute shaders), **iced** (no
per-run font size — no variable heading sizes), **tree-sitter-md** (self-disclaims correctness;
open stack-corrupting overflow in `serialize()`).

## Build & test

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features # broken intra-doc links are denied, and CI runs this
cargo deny check                   # licences + advisories
cargo bench -p mdedit-markers      # segmentation cost against the keystroke budget
cargo bench -p mdedit-document     # what an undo group costs against typing the same edits
```

CI runs fmt, clippy, tests and docs on both Linux and Windows. Check the MSRV before pushing —
`cargo +1.87.0 check --all-features` — because the local toolchain is newer and will happily accept
things the floor rejects. Let-chains are the trap: they stabilised in 1.88.

Check the Linux build too — `cargo check --target x86_64-unknown-linux-gnu --all-features`. Windows
is the only platform anything runs on, so it is easy to add a dependency or a `cfg` that only
compiles here. `rfd` needing GTK headers and a `cfg`-gated enum going dead both got through to CI
that way. `cargo deny` and the MSRV check
pinned to `rust-version` (1.87, raised from 1.85 by wgpu 30) run on Linux only — neither is platform-sensitive. The bench is not a CI
job; it is run by hand when touching segmentation.

`cargo test --release -- --ignored` is reserved for the perf assertions described under
**Performance** below. None exist yet, and they will not run in CI: GitHub runners are virtualised
and on AC power, which is the condition the research says is least informative. They belong in a
local harness whose numbers go in the PR body.

Line endings are pinned to LF by `.gitattributes`, because `rustfmt.toml` sets
`newline_style = "Unix"` and Windows checkouts default to `core.autocrlf=true`.

## Rust, beyond the baseline

`standards/rust.md` owns the general rules. Only these are project decisions:

- **No `async` in the editor core.** The UI thread is a synchronous frame loop; I/O goes to a worker
  thread. Adding a runtime to the core needs a design decision on the ticket.
- **`unsafe` anywhere outside a platform crate is a review block**, not a discussion — stricter than
  the baseline's `deny` + scoped-allow. Two crates qualify: `mdedit-text-dwrite` for DirectWrite, and
  `mdedit-menu-win` for the one call that attaches a menu bar to a window. A third needs a reason on
  the ticket, not a `SAFETY` comment in `mdedit-app`.
- **Newtype every offset**: `ByteOffset`, `CharOffset`, `LineIndex`, `BlockId`, `LayoutPos`. This
  codebase carries at least four coordinate spaces at once — source bytes, rope chars, laid-out glyph
  positions, and block indices — and silently mixing two `usize`s is the likeliest bug class in the
  whole project. They live in `mdedit-offsets`, which depends on nothing: the text layer needs
  `ByteOffset` to back an `ITextStoreACP` and must not reach a Markdown parser to get it.
- **A byte range is a `ByteSpan`, not a `Range`.** `Range<ByteOffset>` cannot index a `str` — the
  orphan rule bars `impl Index<Range<ByteOffset>> for str` — so spans carry their own type and slice
  via `ByteSpan::slice`. `Range<LineIndex>` is fine, because nothing is indexed by a line range.
  Converting raw offsets in or out (`ByteSpan::from`, `ByteSpan::range`) is deliberate and confined
  to boundaries: `pulldown-cmark` event ranges, `ropey`'s character indices, and test fixtures.

## Project-specific rules

**The byte-preservation invariant — the one rule that outranks everything:**

- **The buffer is the source of truth. Never re-serialize the document.** Opening a file and saving
  it must produce an identical byte sequence. MarkText's six-year-old
  [#2189](https://github.com/marktext/marktext/issues/2189) is what happens if this slips.
- Inline content is **never** serialized — it is decorated in place. Emphasis token (`*` vs `_`) and
  hard-break style are lost by every Rust Markdown parser; we sidestep that by not rewriting those
  bytes.
- Only the block under the cursor may be re-serialized, and only on exit, spliced back at its span.
- A diff that introduces a whole-document serializer needs an explicit design decision on the ticket.

**Marker locator (`crate::markdown::markers`):**

- No Rust Markdown parser emits delimiter nodes, so we compute them. This is the highest-risk
  component — see [`docs/research/03-architecture.md`](docs/research/03-architecture.md).
- **Property-tested against the CommonMark spec suite with one invariant: concatenating all marker
  and content spans, in order, reproduces the source byte-for-byte.** If that test is red, nothing
  else in the editor is trustworthy. It is the gate, not a nicety.
- Known-wrong upstream spans to work around: nested emphasis, escapes, collapsed links
  (`pulldown-cmark#1077`), image titles with no recoverable offset (`#441`), fence info strings
  (`#271`).

**Rendering & virtualization:**

- **Never materialize the whole document.** Render visible blocks only. `file open → first paint`
  is O(1): mmap plus viewport parse. Every large-file failure in the competition comes from
  violating this.
- **A tab is a lightweight handle until it becomes visible.** Session restore persists handles, not
  content. Copy Obsidian's `DeferredView` model — it is the only implementation in this category.
- Make the **renderer** incremental, not the parser. Diff the block list, patch changed blocks.
- **The layout cache is keyed by content, never by position.** `BlockId` is an index into one
  analysis: insert a character in the first paragraph and every block after it is renumbered, so a
  cache keyed on it would serve the wrong layout for the whole document below an edit. The key is a
  hash of the block's bytes plus its kind and the wrapping width — the same bytes lay out the same
  way wherever they are. A block holding the caret is laid out fresh and never cached, because what
  it draws depends on where in it the caret sits, and that is not in the key.
- Tables and code fences are **model-backed block widgets**, not decorations. Pure decoration over a
  linear character stream cannot do 2D layout; Obsidian's forums are a five-year record of that
  failure.

**Text layer:**

- Text layout sits behind a narrow internal trait with **two implementations: DirectWrite (Windows)
  and parley (portable).** Not a precaution — **DirectWrite is required on Windows today.** MDEDIT-5
  confirmed on real hardware that fontique resolves `script = Hani` to a single fallback family and
  stops, so Simplified-Chinese-only characters render as tofu even though SimSun is installed
  ([parley#644](https://github.com/linebender/parley/issues/644), open, cause is architectural).
  A Markdown editor that cannot render 汉 is not shippable. The trait also de-risks parley's release
  churn (five breaking releases in seven months, no 1.0) and gives a correctness oracle.
- **Do not use parley for text layout on Windows** until `spikes/font-fallback` passes there against a
  *released* fontique — passing against a patched branch does not count, since there is nothing to
  depend on. Its layout side remains the portable implementation for macOS and Linux.
  [parley#689](https://github.com/linebender/parley/pull/689) is the fix, and **it has been verified
  to work**: with its branch patched in, the probe passes all eight samples on Windows including the
  mixed-script line. Its Windows tier delegates to `IDWriteFontFallback::MapCharacters` — the same API
  we call directly — so the two paths converge rather than diverge.
  **The requirement stands only until that PR ships.** As of 2026-07-27 it is unmerged, conflicted
  with main and unapproved, so nothing is available to depend on. When it lands in a released
  fontique, re-run the probe against the release and, if green, parley becomes viable on Windows and
  this restriction lifts.
- Pin parley to an exact version. Wrap it; don't leak its types into the editor core.
- IME is IMM32 via winit for v1, but **the buffer sits behind a trait that can later back an
  `ITextStoreACP`** — retrofitting TSF's offsets and lock protocol onto a buffer not designed for it
  is expensive.

**Windows integration:**

- **Opening N files from Explorer must reuse one process.** Register a **DropTarget verb**
  (CLSID + `LocalServer32`), not a command-line verb — command-line verbs default to
  `MultiSelectModel=Document`, which spawns one process per file and silently drops the verb past
  15 items. Notepad++'s shell extension gets this wrong and has since 2023.
- **Declare Per-Monitor DPI v2 in the manifest**, via `embed-manifest` — winit deliberately won't.
  Re-run glyph rasterisation and hit-testing on `WM_DPICHANGED`, and use the suggested rectangle.
- Read the dark-mode setting from the **registry**, not `ShouldAppsUseDarkMode()` (unreliable from
  1903+). Refresh on `WM_SETTINGCHANGE` / `ImmersiveColorSet`.
- **Per-user install. Never elevate.** Elevation breaks drag-drop from a non-elevated Explorer, and
  the documented mitigation is one Microsoft says isn't safe.

**Security (this project's specific threat model):**

- **Any auto-updater verifies signatures, pinned.** Notepad++ shipped an unverified updater, got its
  host compromised for ~6 months, and landed on CISA KEV with a nation-state actor delivering Cobalt
  Strike through it. This is the single highest-consequence design decision in the project.
- **Treat our own config and session files as untrusted input.** Never route a config string into a
  process launch; normalise every path before a prefix check. Notepad++ shipped three consecutive
  bypasses in 2026 by patching symptoms instead of the boundary.
- File parsing is a trust boundary. The 2023 Notepad++ CVEs were all heap overflows in hand-rolled
  encoding detection — the class Rust removes, provided we don't reintroduce it in `unsafe`.

**Performance ([`docs/research/06-performance-targets.md`](docs/research/06-performance-targets.md)):**

- Targets are commitments, not aspirations: cold start ≤100 ms, keystroke→pixel p50 ≤3 ms,
  **p99 ≤8.3 ms**, file open → first paint ≤50 ms at any size. Baseline RSS is now split into a
  floor we do not control and a budget we do — MDEDIT-19 measured an empty wgpu window at 124 MB,
  so the old ≤50 MB total was unreachable. See `docs/research/06-performance-targets.md`.
- **Measure p99 and measure on battery.** Every published editor benchmark is a mean on AC power,
  which is the least informative condition — Emacs goes 5.3 ms → 50.5 ms on power-save.
- **Two keystroke numbers, and the second one has a floor.** The harness reports *keystroke → quads
  ready*, which is what this process spends, and *keystroke → present*, which adds the wait for
  vblank. Under `PresentMode::Fifo` the second cannot go below a refresh interval however fast the
  frame was, so once a frame is cheap it stops saying anything about our own work. Compare builds
  on the first; use the second to confirm frames are still being presented.
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

## Configs

Vendored from the baseline tag `v1.2.0`:

| Tool | Project file | Baseline source |
|------|--------------|-----------------|
| rustfmt | `rustfmt.toml` | `configs/rustfmt.toml` |
| clippy | `clippy.toml` + `[workspace.lints]` in `Cargo.toml` | `configs/clippy.toml` |
| cargo-deny | `deny.toml` | `configs/deny.toml` |

`clippy.toml` tunes lints but cannot enable them — the lint set lives in `[workspace.lints]`, copied
from the comment block at the foot of the baseline config. Set `msrv` to match `rust-version`.

Layer enforcement needs no config: workspace crate `[dependencies]` are the allow-list and Cargo
rejects cyclic crate deps at build time (`standards/architecture.md`).

Bump the pinned tag deliberately; re-vendor the configs when it changes.
