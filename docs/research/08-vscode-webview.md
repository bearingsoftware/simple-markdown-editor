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
| 3 | Speed | 100 KB analysis 3.6 ms Node / ~4.4-7 ms webview (native 2.3 ms); 1 MB 35 ms Node / ~69 ms webview (native 24 ms). Mid-document insert at 1 MB: 2 ms scoped. |
| 4 | DOM preview | Yes; 8 structural tests plus a dump. |
| 5 | Caret mapping | Yes for ASCII, multi-byte, astral, after a concealed marker, CRLF; 8 tests, exhaustive over a mixed document. |
| 6 | Edit and undo | **Insert is exact and undo works, but VS Code rewrites mixed line endings** before the editor ever sees them. |
| 7 | Virtualisation | Buffer in both host and webview; 1 MB postMessage is 10-40 ms one way, so only deltas may cross. |

## 1. Compile

| Crate | wasm32-unknown-unknown |
| --- | --- |
| `mdedit-offsets` | As is. |
| `mdedit-language` | As is. |
| `mdedit-markers` | As is, **with a behavioural catch**: `analyze` contains a parser panic with `catch_unwind` (`crates/markers/src/lib.rs`), and `wasm32-unknown-unknown` is `panic = "abort"`, so a panicking parse traps the instance instead of degrading to "all content". Not verified: pulldown-cmark 0.13.6 has no known panicking input (#1129 is fixed from 0.13.5), so there was nothing to trigger it with. |
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
| analysis 100 KB, median / p99 | 2.3 / 2.7 ms | 3.6 / 4.3 ms | 4.4-7.0 / 7.9-10.9 ms |
| analysis 1 MB, median / p99 | 24.4 / 30.2 ms | 35.1 / 42-60 ms | 68.8-69.4 / 89.7-94.6 ms |
| edit + scoped re-analysis, 100 KB | 0.31 / 0.44 ms | 0.25 / 0.39 ms | 0.3 / 0.5 ms |
| edit + scoped re-analysis, 1 MB | 2.7 / 5.9 ms | 1.9-2.1 / 2.4-3.4 ms | 3.3-3.5 / 3.7-20 ms |
| `reconcile` (whole-document), 1 MB | 23.8 / 31.8 ms | 38 / 60 ms | 62-69 / 71-217 ms |

wasm is about 1.5x native in Node and about 2.5-3x native in the webview. The webview figures were
taken with a second editor beside it and other builds running, so they are an upper band.
Marshalling the result to a `Uint32Array` adds 0.2 ms at 100 KB and 1.4-1.9 ms at 1 MB.

Start-up: compiling the module is lazy and costs under 1 ms in Node (0.5 ms to `initSync` without
highlighting, 2 ms with); in the webview `init` took 4.6-5.3 ms for the 1.74 MB module. The first
analysis call is cold: 17 ms at 100 KB. **The first highlight call loads the syntax set: 113-420 ms
in Node** (about 3-8 ms afterwards for 20 lines), which is the same laziness `mdedit-highlight`
already has natively.

Against the project's own targets (keystroke to pixel p99 8.3 ms): a keystroke through the scoped
path is inside it at 100 KB and 1 MB for the analysis alone (the DOM update is not measured, see 7);
a full analysis is not, from about 200 KB up, so it has to stay off the keystroke path exactly as it
is natively.

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
  class of file (it does not affect opening, or a file with uniform endings; only the mixed case was
  tested, so uniform CRLF is assumed to survive). It is VS Code's behaviour, not the webview's: VS
  Code's own editor does the same.

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

**Build now, narrowly. Parley#689 is not on the critical path**: this route uses the browser's text
layout, not the text trait, so waiting for the portable text backend buys nothing here.

The two or three numbers that decide it:

1. **The wasm is small and compiles unchanged:** 279 KB (1.74 MB with highlighting) for the whole core,
   no edit to `crates/`. The cost of the port is almost nil.
2. **1 MB analyses in 35 ms (Node) to 69 ms (webview) and a keystroke re-analyses in 2-3.5 ms**, so
   files up to about 1 MB are comfortably interactive and 10 MB is not yet.
3. **VS Code rewrites mixed line endings on the first edit** (98 bytes to 95). This is what stops it
   being a replacement for the native editor on the byte-preservation rule, and is what the build
   ticket has to work around rather than around the editor.

What a build ticket would need to say:

- Scope it as a **preview-and-light-edit custom editor, `priority: "option"`**, not a default, and
  **refuse files with mixed line endings** (open them in the text editor with a notice) until a way
  to keep their bytes is found. Say plainly that the invariant holds for uniform-ending files only.
- Viewport virtualisation of the DOM is the first piece of work, with a 10 MB target stated as a
  number (open to first paint, keystroke p99), and the `Vec<Segment>` splice cost measured at 10 MB.
- Messages are deltas both ways; whole text only on open and resync. Version every message.
- Decide IME first (contenteditable against hidden input), because it decides the structure of
  everything else; accessibility and cross-block deletion follow from it.
- Panic containment does not work on wasm (`panic = "abort"`); either the pulldown-cmark panic stays
  fixed by pinning, or the wasm profile gets `panic = "unwind"` with `-Zbuild-std`, which is a build
  decision of its own.
- Ship highlighting lazily as a second module (870 KB gzipped), loaded on the first fenced block, so
  the common start is 120 KB.
