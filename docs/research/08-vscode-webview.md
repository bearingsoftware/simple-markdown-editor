# 08 — The headless core in a VS Code webview

MDEDIT-206, a spike. The question: can the crates that need no window (`offsets`, `language`,
`markers`, `document`, `highlight`) run as WebAssembly inside a VS Code custom-editor webview, and
would that be a front end worth building? Evidence and a recommendation, nothing shipped. Code and
re-run commands: [`spikes/vscode-wasm/`](../../spikes/vscode-wasm/README.md); saved outputs in its
`results/`.

**Method, and what to trust.** Compile, size and Node timings were measured on one Windows machine
shared with other builds, and timings swung up to 2x between runs. The numbers below are the quiet
runs unless a range is given; `results/` holds the last (noisier) run. Webview numbers come from a
real VS Code 1.145.3 launched by `@vscode/test-electron` with its own profile and extensions folder,
so the installed VS Code was not touched. The wasm is built `opt-level = "z"`, `lto`, `panic = "abort"`,
and each size is also given after `wasm-opt -Oz` (binaryen 132 from npm). Where a probe ran in
Node or jsdom rather than a webview it says so.

## Probe results at a glance

| # | Probe | Answer |
| --- | --- | --- |
| 1 | Compiles for `wasm32-unknown-unknown` | **Yes, all five, no change under `crates/`.** |
| 2 | Size | 279 KB without highlighting, 1.74 MB with (after `wasm-opt -Oz`); 120 KB / 870 KB gzipped; `.vsix` 133 KB / 880 KB. |
| 3 | Speed | 100 KB analysis 3.6 ms Node / ~4.4-7 ms webview (native 2.3 ms); 1 MB 35 ms Node / ~69 ms webview (native 20-25 ms). Mid-document insert at 1 MB, scoped: 3.3-3.5 ms in the webview (2.0 Node, 2.5 native). |
| 4 | DOM preview | Yes; 8 structural tests plus a dump. |
| 5 | Caret mapping | Yes for ASCII, multi-byte, astral, after a concealed marker, CRLF; 8 tests, exhaustive over a mixed document. |
| 6 | Edit and undo | **Insert is exact and undo works, but VS Code rewrites mixed line endings** before the editor ever sees them. |
| 7 | Virtualisation | Buffer in both host and webview; 1 MB postMessage is 10-40 ms one way, so only deltas may cross. |

## 1. Compile

| Crate | wasm32-unknown-unknown |
| --- | --- |
| `mdedit-offsets` | As is. |
| `mdedit-language` | As is. |
| `mdedit-markers` | As is, **with a behavioural catch**: `analyze` contains a parser panic with `catch_unwind` (`crates/markers/src/lib.rs`), and `wasm32-unknown-unknown` is `panic = "abort"`, so a panicking parse traps the instance instead of degrading to "all content". A trapped instance leaves a dead tab, because `main.js` guards only boot (`try`/`catch` around `loadWasm`), not the message handlers; the host `TextDocument` keeps the text, so nothing is lost, and re-instantiating the core from the host's text is the containment a build must supply. Not verified: pulldown-cmark 0.13.6 has no known panicking input (#1129 is fixed from 0.13.5), so there was nothing to trigger it with. |
| `mdedit-document` | As is. `ropey` 1.6 with `simd` falls back cleanly. |
| `mdedit-highlight` | As is. syntect with `regex-fancy` compiles; the build script runs on the host and embeds the syntax dump. No thread is involved (the worker lives in `mdedit-app`). |

**Needed for wasm: none.** No file under `crates/` changed; the workspace's own `Cargo.lock` was
copied so versions match. The root workspace is unaffected (`cargo metadata --no-deps` shows the same
18 members and no spike crate; `exclude = ["spikes/*", "fuzz"]` already keeps the spike out).

A second catch: the workspace's `ropey` takes `default-features = false`, so only `\n` (and so `\r\n`)
ends a line, while VS Code also ends a line at a lone `\r`. Files with lone-CR endings would
disagree on line numbers. Reasoned from the feature set, not tested.

## 2. Size

| Build | Raw | `wasm-opt -Oz` | gzip (raw / opt) | `.vsix` |
| --- | --- | --- | --- | --- |
| without highlighting | 423,613 | 279,447 | 135,150 / 119,974 | 136,124 |
| with highlighting | 2,674,120 | 1,737,640 | 942,745 / 869,554 | 900,767 |

Highlighting is 85% of the bytes: it is the syntax dump (the embedded bincode set) plus
syntect/fancy-regex. `vsce package` ran locally and was never published.

## 3. Speed

Fixture: the `document()` generator from `crates/markers/benches/segment.rs`, repeated to 100 KB
(102,604 bytes) and 1 MB (1,048,740). 100 runs at 100 KB and 40 at 1 MB after three warm-ups, so p99
at 1 MB is close to the maximum.

| | native (opt-level 3) | Node 24 (V8) | real webview, strict CSP |
| --- | --- | --- | --- |
| analysis 100 KB, median / p99 | 2.0 / 2.3 ms | 3.6 / 4.3 ms | 4.4-7.0 / 7.9-10.9 ms |
| analysis 1 MB, median / p99 | 20.1 / 21.4 ms | 35.1 / 42-60 ms | 68.8-69.4 / 89.7-94.6 ms |
| edit + scoped re-analysis, 100 KB | 0.28 / 0.43 ms | 0.25 / 0.39 ms | 0.3 / 0.5 ms |
| edit + scoped re-analysis, 1 MB | 2.5 / 3.2 ms | 1.9-2.1 / 2.4-3.4 ms | 3.3-3.5 / 3.7-20 ms |
| `reconcile` (whole-document), 1 MB | 22.2 / 27.1 ms | 38 / 60 ms | 62-69 / 71-217 ms |

The native column is a quiet run (nothing else building; `results/native.txt`, a second quiet run in
`results/native-run-2.txt`, and a first run that was contended and discarded). wasm is about 1.5-1.7x
native for analysis in Node and about 3x in the webview.

**The scoped edit is the exception: native (2.5 ms at 1 MB) is slower than Node's wasm (2.0) and
about level with the webview's.** The cause was not isolated. `Document::replace` does a partial
re-parse, but `resegment_scope` first calls `self.text.to_string()` (the whole rope, 1 MB) and
rebuilds the whole `Vec<Segment>`, so each edit makes several megabyte-class allocations. One such
copy (`String::clone` of 1 MB) took 0.35-0.6 ms natively against 0.003 ms at 100 KB, a 100x jump for
10x the size, which looks like page-fault cost on large Windows-heap allocations; wasm's allocator
keeps its linear memory and does not pay it. That fits the numbers and is not proven (a profile would
settle it). If right, the native editor has the same cost at 1 MB, which is not a wasm problem. The webview figures were
taken with a second editor beside it and other builds running, so they are an upper band.
Marshalling the result to a `Uint32Array` adds 0.2 ms at 100 KB and 1.4-1.9 ms at 1 MB.

Start-up: compiling the module is lazy and costs under 1 ms in Node (0.5 ms to `initSync` without
highlighting, 2 ms with); in the webview `init` took 4.6-5.3 ms for the 1.74 MB module. The first
analysis call is cold: 17 ms at 100 KB. **The first highlight call loads the syntax set: 113-420 ms
in Node** (about 3-8 ms afterwards for 20 lines), which is the same laziness `mdedit-highlight`
already has natively.

**What the scoped edit is, and what still runs after it.** It is `Document::replace`'s partial
re-parse (`scope_for` and `resegment_scope` in `crates/document`), the same path the native editor
uses, not a wasm-specific shortcut. But the document is then marked scoped, and a whole-document
`reconcile` is owed when typing stops (`Document::reconcile`). At 1 MB that is 62-69 ms median in the
webview (p99 71-217 ms; 38 ms in Node, 22 ms native), and in the webview it runs on the main thread.
Against the 8.3 ms keystroke p99 target that is 8 to 25 times over, a pause at the moment the user
stops typing and is most likely to start again. At 100 KB it is 3.8 ms (p99 7), inside the target.
A build would choose one of three, with a number each: (a) **a worker** that owns the `Session` and
posts results back, which keeps the main thread free and costs a second wasm instance plus the message
hops measured in 7; (b) **a size cap**, a custom editor that declines files over about 200 KB, where
`reconcile` is under one frame; (c) **accept it**, documented as "a pause of about 70 ms, up to 220,
after typing in a 1 MB file". The recommendation below makes this a decision to settle before the
build starts.

Per keystroke, then, the cost is three things: the scoped `replace` (0.3 ms at 100 KB, 3.3-3.5 ms at
1 MB in the webview), the flat marshal back to JS (0.2 ms and 1.4-1.9 ms), and the DOM re-render,
**which is not measured** (the spike re-renders the whole document; a virtualised one would re-render
a block).

## 4. DOM preview

`extension/media/render.js` builds DOM from the flat marker/content partition and the block list
(blocks are nested by span containment). Headings, emphasis, strong, strikethrough, inline code,
links, lists, block quotes and fenced code become elements; every marker becomes an empty hidden
`<span class="marker">` carrying its span, so no marker byte is ever rendered text. Bullets and
numbers are substituted by `<ul>`/`<ol>` (an `<ol start>` is read from the first marker). Tables,
images and HTML are drawn as source, bytes intact. A highlighter hook colours fence text.

`test/dom.test.mjs`, 8 tests, **jsdom in Node, not a webview**, against the real wasm. They assert
structure, that no marker syntax leaks into text, that lists and fences come out right, and that every
text node holds exactly the bytes of its span. `results/dom-dump.html` is the dump.

## 5. Caret mapping

Every text node sits alone in an element carrying `data-b0`/`data-b1`, its source byte span.
`caret.js` maps a DOM `(node, offset)` to a byte (`byteFromDom`), a byte back to a DOM position, and
a byte to a VS Code `Position` (`positionFromByte`, with a wasm twin `Session.position` that uses the
rope in O(log n)). `test/caret.test.mjs`, 8 tests, **jsdom in Node**:

- ASCII; 2- and 3-byte characters (`é`, `汉`).
- An astral character: 2 UTF-16 units in the DOM, 4 bytes in the source, 2 units in the VS Code column.
- Right after a concealed marker: the position resolves to the content side, because a marker has no
  text to put a caret in. A byte inside a marker snaps forward.
- CRLF: the `\r\n` stays in the DOM text and counts one line; a byte between the CR and LF clamps to
  before the CR, which is what `validatePosition` does.
- Exhaustive: every `(text node, offset)` in a mixed document (accents, CJK, emoji, CRLF, lists,
  quote, link) agrees with an independent reference, round-trips through `domFromByte`, and the wasm
  `Session.position` agrees with the JS conversion at every character boundary.

## 6. Edit and undo (real VS Code, `test/vscode/suite.cjs`, 10 of 10 pass)

The webview turns `beforeinput` into one byte-span replace, converts both ends to a UTF-16 `Range`
with `Session.position`, applies the replace to its own `Session`, and posts it; the host applies it
with `WorkspaceEdit`. Fixture: mixed CRLF and LF, trailing spaces, `*star*` beside `_under_`, 2-, 3-
and 4-byte characters.

- **Exact insertion: yes.** Three inserts (after `*star*`, after a 4-byte emoji on a later line, at
  the start of a line after CRLF) each left the document's bytes equal to the previous bytes plus
  exactly the inserted span, and the webview `Session`'s bytes equalled the document's.
- **Undo: yes.** Three `undo` commands in the source view restored the pre-edit bytes (one undo per
  edit) and the webview re-rendered them. A source-view edit reached the webview.
- **The finding that matters: VS Code does not hand over the file's bytes.** The fixture on disk is
  98 bytes with 3 CRs. `TextDocument.getText()` on open returned 95 bytes with none (`eol` LF), and
  after an edit and `save` the file on disk equalled that normalised text. So for a file with mixed
  line endings, **the first edit rewrites every line ending in the file**, before our code runs and
  with no way to opt out from an extension. The edit itself is exact; what it is exact *against* is
  already not the user's file. This breaks the byte-preservation invariant in `AGENTS.md` for that
  class of file (it does not affect opening; **uniform CRLF survival is untested** and is assumed, because only the
  mixed case was run). The behaviour is VS Code's, not the webview's, and is documented rather than
  merely observed: a `TextDocument` has a single `eol` ("the end of line sequence that is
  predominately used in this document",
  [`TextDocument.eol`](https://code.visualstudio.com/api/references/vscode-api#TextDocument)), so a
  document cannot hold two. Here the fixture has three CRLF and three LF, a tie, and `eol` came back
  LF. The mitigation is a detector run on the file's bytes before the custom editor takes it
  (`extension/media/eol.js`, `test/eol.test.mjs`, 6 Node tests): more than one of CRLF, lone LF and
  lone CR is mixed, so a tie, one stray ending and a lone CR all count.

## 7. Virtualisation and what crosses `postMessage`

Where the buffer lives: **both.** The `TextDocument` in the extension host is the source of truth,
because VS Code owns save, dirty state, undo, other extensions and the source view beside it. The
webview holds a wasm `Session` copy because analysis needs the text next to the code that parses it;
asking the host for spans per keystroke would put an asynchronous hop in the typing path.

What crosses: on open and on any change not made by the webview, **the whole text** (the spike does
this; so would a build, with a threshold). For the webview's own edits, **one replace**: a `Range` and
the inserted string, a few dozen bytes. Host to webview for a foreign edit should likewise be a
delta (`contentChanges`), not text.

Measured in the real webview, 1 MB string: webview to host 10.8 ms median (19 ms max; the clock is
shared wall time across two processes) in one run and 40.7 ms in another; **round trip 23.5 ms
median (36.6 p99) in one run and 68.8 ms (213 p99) in the first.** Under the strict CSP the module
itself arrived as a message in 19.7 ms; `fetch` is refused (below).

A 10 MB file, per design:

- **Whole text per change, either direction:** 10 MB each way is roughly 100-400 ms plus a transient
  second copy in each process. Not acceptable per keystroke; acceptable once on open.
- **Deltas, buffer in both:** the keystroke is a small message plus the scoped re-analysis, which at
  1 MB cost 2-3.5 ms and grows with size here (0.25 ms at 100 KB, so about 20 ms at 10 MB in a straight
  extrapolation, over the 8.3 ms p99 target). That cost is `Document`'s `Vec<Segment>` splice, not the
  parse; it needs looking at before 10 MB is a claim.
- **Whole-document analysis on open:** about 350-700 ms at 10 MB by scaling the 1 MB runs. It needs the
  same "analyse the viewport first" treatment as the native editor's open path.
- **The spike re-renders the entire document into the DOM on every change.** That is fine at test
  sizes and is the real 10 MB problem: the DOM must hold only the visible blocks, with spacer elements
  for the rest. Not built and not measured. That is the largest piece of a real build.

## CSP with WebAssembly

The brief's CSP, `default-src 'none'; script-src ${cspSource} 'wasm-unsafe-eval'; style-src ${cspSource};`,
**was enough and was not changed.** `'wasm-unsafe-eval'` lets `WebAssembly.instantiate` run. What it
does not allow is `fetch(wasmUri)`, which needs `connect-src` and is refused ("Failed to fetch"). Two
ways round it, both measured: (a) the host reads the file and posts the bytes (19.7 ms, no CSP change,
used); (b) add `connect-src ${cspSource}` and `fetch` (128 ms in the same run, and a wider CSP).
The spike instantiates asynchronously (`init({ module_or_path })`); `initSync` was not tried in the webview, and Chrome is documented to refuse synchronous compile of large modules on a main thread.

## Named, not solved

**IME.** A `contenteditable` gets composition events but the spike's `beforeinput` handler only
handles `insertText` and cancels it; composition (`insertCompositionText`, CJK, dead keys) would have
to be applied from the composition's end or the DOM left to mutate and read back. Both are known to be
fragile. A hidden `<textarea>` input with the text drawn separately (the native editor's approach, via
IMM32) is the robust route and would turn the webview into a canvas-like renderer; the cost is
re-doing caret and selection drawing.

**Accessibility.** DOM gives headings, lists and quotes real roles for free, which is a head start the
wgpu editor does not have. Concealed markers are `hidden`, so a screen reader sees the rendered text
and not the syntax, which is the right default but means a blind user cannot find or edit a marker
without the source toggle. Not tested with a screen reader.

**Cross-block selection.** The browser selection spans DOM nodes, and `byteFromDom` maps each end, so
a drag across blocks gives a byte range. What it does not give is what a delete over a range that
includes concealed markers means (the markers are in the range but not on screen). Not handled.

**Tables and images.** Drawn as source; a table grid and an image need their own widgets, and image
paths from a document are the trust boundary `mdedit-image` already guards. Not started.

**Theming against VS Code tokens.** `main.css` uses `--vscode-*` variables, which is enough to follow
light/dark. Mapping the highlight tokens and the editor's own palette onto the workbench's is a design
task, not a risk.

**A browser build.** `render.js`, `caret.js` and the core have no VS Code dependency; only
`extension.js` and the host half of `main.js` do. A browser build would need a different host for the
buffer, save and undo (the File System Access API, and the core's own undo, which exists). The same
front end could serve it, but the byte-preservation problem above would not exist there, which is an
argument for the browser being the better place for this front end than VS Code.

## Recommendation

**Build only after two things are settled: the IME decision and the `reconcile` answer.** If both come
back acceptably, build the narrow version below; if either does not, drop the webview route and spend
the effort on the native editor.

**On parley#689, the argument rather than the assertion.** This route draws text with the browser's own
layout, so it does not touch the text trait or fontique and does not depend on parley#689 landing.
Equally, it does not close the native editor's portability gap: that gap is a missing portable text
backend for the wgpu editor, and a VS Code front end gives no Linux or macOS build of *that* editor.
They are two independent efforts, one waiting on an upstream fix and one not. Neither is a reason to
delay the other.

What the numbers say:

1. **The wasm is small and compiles unchanged:** 279 KB (1.74 MB with highlighting), no edit to
   `crates/`. The port is nearly free.
2. **A keystroke is inside budget at 1 MB only if the DOM cost is small and `reconcile` is dealt with.**
   Replace is 3.3-3.5 ms and marshalling 1.4-1.9 ms at 1 MB, already about 5 of the 8.3 ms before any
   DOM work, which is unmeasured; and `reconcile` is 62-69 ms on the main thread when typing stops.
3. **VS Code rewrites mixed line endings on the first edit** (98 bytes to 95; documented as a single
   `eol` per document). It stops this being a replacement for the native editor on the byte-preservation
   rule.

**What the IME decision must settle** (it decides the structure of everything else): whether text is
typed into a `contenteditable` that the DOM mutates and the core reads back, or into a hidden
`<textarea>` with the document drawn separately. For each, it has to show composition working for one
CJK input method and one dead-key layout on Windows, with the `Session` receiving exactly one replace
per committed composition (not per intermediate string), the caret and selection drawn correctly across
concealed markers, and the screen-reader behaviour stated. A `contenteditable` keeps DOM roles for
accessibility and is the likelier fit for a preview-style editor; a hidden input is the robust one for
IME and costs the accessibility head start. Pick by running both against those cases, not by argument.

**What the reconcile answer must settle:** choose (a) worker, (b) size cap or (c) accept, from section
3, with a measured number in the ticket (the 1 MB p99 pause, or the worker's added latency).

What a build ticket would need to say, once both are answered:

- Scope: a **preview-and-light-edit custom editor, `priority: "option"`**, never the default.
  **Refuse files with mixed line endings** using the detector (open them in the text editor with a
  notice). State plainly that the byte-preservation invariant holds for uniform-ending files only, and
  that uniform-CRLF survival has to be tested first.
- DOM virtualisation first (only visible blocks, spacers for the rest), with a stated 10 MB target
  (open to first paint, keystroke p99) and the per-edit `to_string` and `Vec<Segment>` cost measured at
  10 MB; it is O(document) per edit today, natively as well.
- Messages are deltas both ways; whole text only on open and resync. Version every message.
- **Containment:** the core can trap (`panic = "abort"`, and `main.js` guards only boot). A build wraps
  every call into the core and, on a trap, re-instantiates from the host `TextDocument`, which keeps
  the text, so nothing is lost. Either also pin pulldown-cmark past #1129 or build the wasm with
  unwinding (`-Zbuild-std`), a build decision of its own.
- Highlighting as a second, lazily loaded module (870 KB gzipped), loaded on the first fenced block,
  so the common start is 120 KB; the first highlight call costs 113-420 ms for the syntax set.
