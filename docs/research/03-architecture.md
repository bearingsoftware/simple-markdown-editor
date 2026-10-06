# 03 — Architecture

The core design decision, and the one piece of infrastructure that does not exist
in the Rust ecosystem and must be built.

## The decision

Three viable ways to implement Typora-style hybrid inline editing. The choice
determines everything downstream, especially whether files round-trip
byte-for-byte.

| Approach | Used by | Wins | Loses |
|---|---|---|---|
| **A. Decorate the source** | CodeMirror 6, Obsidian Live Preview, Zettlr, vim `conceal` | File is never rewritten; cursor arithmetic trivially correct; viewport virtualization comes free | Reveal edge cases at boundaries; hard to render true block-level widgets |
| **B. Document model + serializer** | ProseMirror, MarkText/Muya, Typora | Real structured editing — table cell nav, list re-nesting, block drag, collaboration | **Structurally lossy.** Cannot round-trip byte-for-byte |
| **C. Transparent textarea over rendered div** | OverType | Free undo/redo, IME, mobile keyboards, a11y; ~200 lines | **Monospace only**; no heading size variation; markers stay visible |

### Chosen: a hybrid weighted toward A

- **Buffer is the source of truth** (ropey). Never re-serialize the document.
- **Inline level: pure decoration.** Never serialize inline content.
- **Block level: a parsed index over the buffer**, keyed by source spans.
- **Tables and code fences: model-backed block widgets** with their own layout and
  caret handling, serialized *only that block*, spliced back at its span on exit.

Result: 99% of the document is byte-identical on save, always. All normalization
is confined to the block currently under the cursor.

## Why not a pure document model

For a Markdown editor, "never modify bytes the user didn't touch" is close to a
product requirement. Pure document-model architectures actively fight it.

MarkText [#2189](https://github.com/marktext/marktext/issues/2189) — 48 reactions,
open six years — is the demonstration. Maintainers' own explanation:

> "we don't store the document as string with spaces etc but as block structure
> with minimal information. As a side effect, we need to build the text file when
> saving the document. In some situations, we don't know the formatting of the
> original document and apply the default style according markdown specification."

> "It's currently not planned to change the markdown formatting as it's built-in
> into the editor."

Open a file, save it, get a noisy diff. Users experience this as "the editor
changed my file when I only opened it."

ProseMirror shows the losses compound: soft breaks become spaces *"because
newlines tend to behave awkwardly in the editor"*;
[`<br>` tags silently dropped on Markdown extraction](https://github.com/ueberdosis/tiptap/issues/7731);
[inline-code brackets re-escaped every round trip so backslashes accumulate](https://github.com/ProseMirror/prosemirror/issues/694).
Note also that `prosemirror-markdown` was **archived in April 2026**.

## Why not pure decoration either

[Obsidian's Live Preview](https://help.obsidian.md/Live+preview+update) is the
honest reference for the failure modes, and **tables are the recurring wound**:

- [5+ second load times on large tables](https://forum.obsidian.md/t/large-tables-slow-to-render-in-live-preview/85013)
- [tables silently failing to render when text sits above the header row](https://forum.obsidian.md/t/tables-dont-work-in-live-preview-mode-with-text-above-it/60367)
- spontaneous reversion to source view

Anything needing true 2D layout fights an approach built on decorating a linear
character stream. **Decide up front that tables and code fences are model-backed
block widgets.** That is the specific place the hybrid earns its complexity — and
if you try to make tables work as pure decorations, you will rediscover Obsidian's
bug list from first principles.

## How decoration actually works

From the
[CM6 live-preview design doc](https://github.com/blueberrycongee/codemirror-live-markdown/blob/main/CODEMIRROR_LIVE_PREVIEW_DESIGN.md):

1. **Reveal-on-cursor is a single predicate.** `shouldShowSource(state, from, to)`
   returns true if any selection range overlaps `[from, to)`. That one function is
   the entire WYSIWYG illusion.
2. **Three decoration kinds.** `Decoration.mark` for inline formatting (delimiters
   stay in the document, styled away). `Decoration.replace({ widget, block: true })`
   for tables, code blocks, images, math. `Decoration.line` for whole-line styling.
3. **Line-height stability is the subtle killer.** Not `display: none` (collapses
   the line), not `visibility: hidden` (reserves wrong space) — CM6 uses
   `fontSize: 0.01em; lineHeight: inherit; overflow: hidden` so the character
   stays in layout metrics while being invisible.
4. **Position caches per element type**, so a selection move that doesn't change
   overlap status skips the rebuild entirely. Decoration rebuilding is suppressed
   outright between `mousedown` and `mouseup`.
5. **IME flicker is a named hard problem.** CJK input fires rapid selection changes
   that flash the mark animations; the fix is ignoring selection changes at the
   same character position.

Muya reaches the same design by a different route — see
[01](01-editor-landscape.md) for the `.mu-hide` CSS trick and the
`checkConflicted()` predicate.

[Atomic Editor](https://github.com/kenforthewin/atomic-editor) states the fidelity
property directly: *"all decorations are view-only — copy, save, and round-trip to
any markdown parser are identical to what you'd expect from a plain textarea."*

## The two-level split, and why it's right

Muya (MarkText's core) splits block and inline differently, and this is the model
worth copying:

**Block level: structured tree with syntax choices preserved as metadata.**

```js
{ name: 'atx-heading',    meta: { level: 1 },                              text: '# foo bar' }
{ name: 'setext-heading', meta: { level: 1, underline: '===' },            text: 'foo\nbar' }
{ name: 'code-block',     meta: { type: 'indented', lang: 'javascript' },  text: '...' }
{ name: 'bullet-list',    meta: { marker: '-', loose: false },             children: [...] }
{ name: 'order-list',     meta: { start: 1, loose: true, delimiter: '.' }, children: [...] }
```

Every `meta` field exists to carry a syntax choice a naive serializer would
normalize — `marker`, `delimiter`, `underline`, `type: 'indented'`, `loose`.

**Inline level: raw Markdown text, decorated.** Note `text: 'foo\nbar'` — inline
content is *unparsed source*. Tokens carry `range: {start, end}` and an explicit
`marker` field; the renderer decides per-token whether to reveal based on cursor
position.

**Why this shape.** Block structure is where you want a model — tables, list
nesting, block drag, code fences with their own chrome — exactly where pure
decoration falls over. Inline is where you want raw source — emphasis markers,
link syntax, escapes — exactly where serializers lose fidelity and users notice.
Each level uses the approach that is cheap for it.

## What Rust parsers lose (and why it doesn't matter here)

If you were serializing back, the parser's data model would decide fidelity:

| Syntax choice | pulldown-cmark | comrak | markdown-rs |
|---|---|---|---|
| Setext vs ATX heading | Lost | `NodeHeading { setext, closed }` | Lost |
| Bullet char `-`/`*`/`+` | Lost | `NodeList.bullet_char` | Lost |
| Ordered delimiter `.`/`)` | Lost | `NodeList.delimiter` | Lost |
| Fence char, fence length | Lost | `NodeCodeBlock { fence_char, fence_length, … }` | Lost |
| **Emphasis `*` vs `_`** | **Lost** | **Lost** | **Lost** |
| **Hard break `\` vs two spaces** | **Lost** | **Lost** | **Lost** |
| Link type (inline/ref/collapsed) | `LinkType` | — | `referenceType` |

Note the two nobody preserves: emphasis token and hard-break style. Under a
document-model architecture those are permanently lossy.

`pulldown-cmark-to-cmark` makes it concrete and bad: its
[`Options`](https://docs.rs/pulldown-cmark-to-cmark/latest/pulldown_cmark_to_cmark/struct.Options.html)
are **document-global scalars** — one `emphasis_token`, one `list_token`, and
`code_block_token_count` **defaults to 4**, so a source ` ``` ` fence is
re-emitted as ` ```` `. Mixed `*foo* _bar_` normalizes.

**Under decorate-the-source, none of this is load-bearing** — you never serialize,
so a parser's memory of `*` vs `_` is irrelevant. This inverts the usual parser
advice: comrak's syntax-preserving AST buys nothing, and you would pay 6× the
binary size and give up byte offsets (`sourcepos` is line/column, not bytes) for
it.

**Use pulldown-cmark.** Byte ranges natively, 440 KiB, fastest, MIT, and
`LinkDef.span` recovers reference definitions. Add comrak only if a later
"reformat document" command is wanted, where its fidelity is exactly right.

---

## The marker locator — the thing you cannot buy

**This is the highest-risk item in the project and nobody writes about it.**

Decoration needs the byte range of *each delimiter*, not just the construct.

In JavaScript this is free:

- `@lezer/markdown` defines `HeaderMark`, `QuoteMark`, `ListMark`, `LinkMark`,
  `EmphasisMark`, `CodeMark` as first-class tree nodes, tagged
  `t.processingInstruction` specifically so they can be styled separately from
  content.
- tree-sitter-markdown-inline does the same via
  `alias($._emphasis_open_star, $.emphasis_delimiter)`, plus `code_span_delimiter`,
  `link_text`, `link_destination`, `link_label`.

**No Rust CommonMark parser does this.** `pulldown-cmark`'s `Tag::Emphasis` is a
fieldless variant; comrak's `Emph`/`Strong` carry no delimiter; mdast has no mark
nodes.

Worse, [pulldown-cmark#441](https://github.com/pulldown-cmark/pulldown-cmark/issues/441)
is open: the whole image `![alt](url "title")` yields one range, and the title
arrives as `CowStr::Inlined` with **no recoverable offset** — the reporter's
workaround is to search the source for the string, which breaks when the text
repeats. Same class of problem for the
[fenced-code info string](https://github.com/pulldown-cmark/pulldown-cmark/issues/271).

### What to build

Given a construct's kind, byte range, and the source bytes, compute the delimiter
sub-ranges.

- Emphasis, headings, blockquotes, fences, list markers: boundary scan. Easy.
- Links, images, reference definitions: re-scan the bracket/paren structure.
- Known-wrong upstream spans to work around: nested emphasis, escapes, collapsed
  links ([pulldown-cmark#1077](https://github.com/pulldown-cmark/pulldown-cmark/issues/1077)).

Estimate: ~200–400 lines plus a large test corpus. Bounded, unglamorous, and it
is the thing standing between you and the whole architecture.

### Build it first, standalone

Property-test it against the CommonMark spec suite with a single invariant:

> **Concatenating all marker spans and content spans, in order, reproduces the
> source byte-for-byte.**

If that invariant holds, the architecture is sound. If it doesn't, nothing else
matters.

---

## Block-level incrementality

**Do not build this for v1.** See [04](04-parsing-benchmarks.md) — measured, a
full `pulldown-cmark` re-parse is 2.99 ms at 60 KB and 15.95 ms at 310 KB, which
is inside a frame budget on the main thread with no dirty-region logic at all.

Ship the naive full re-parse. Add dirty regions only when profiling demands it.

When that time comes, the design to copy is `@lezer/markdown`'s. Marijn
Haverbeke deliberately did *not* use Lezer's LR generator:

> "It does not in fact use the Lezer runtime (that runs LR parsers, and
> **Markdown can't really be parsed that way**), but it produces Lezer-style
> compact syntax trees and consumes fragments of such trees for its incremental
> parsing."

The reuse mechanism, from `BlockContext`:

```ts
if (!this.fragments!.moveTo(this.absoluteLineStart + start, this.absoluteLineStart) ||
    !this.fragments!.matches(this.block.hash)) return false
```

Seek the fragment cursor to this position **and verify the enclosing block's
context hash matches**. Reuse only when the surrounding container context
(blockquote depth, list nesting, open fence state) is identical. That is what
makes it sound for a context-sensitive format, and it is a cheap stand-in for
the expensive scanner-state byte-comparison tree-sitter does.

Two further details worth keeping:

```ts
const NotLast = [Type.CodeBlock, Type.ListItem, Type.OrderedList, Type.BulletList]
```

These types cannot terminate a reused region, because they may continue across a
blank line. And the governing constraint: *"Taken content must always end in a
block, because incremental parsing happens on block boundaries."*

**Inline parsing is fully deferred.** No inline nodes are created during the block
scan; the scan only accumulates text. Only when a leaf block closes does
`finishLeaf()` call `parseInline()`. Same two-phase split as tree-sitter-md — but
**one parser, one tree, one shared reuse policy**, so an unchanged paragraph's
inline nodes ride along inside a reused block node for free.

## Link reference definitions

The one construct that genuinely breaks block-locality: a definition anywhere in
the document affects references everywhere.

Every incremental design concedes ground here. `@lezer/markdown`'s README is
candid that being single-pass and incremental means it *"doesn't validate link
references"* — an incremental parser cannot know whether `[foo]` resolves because
the definition may appear 500 lines later.

**We can do better cheaply, because we aren't constrained to one pass.** Keep a
document-level side table of definitions, maintained incrementally, and invalidate
only the *rendering* (not the parsing) of reference links when it changes.

## Make the renderer incremental, not the parser

This is the MarkText/CM6 lesson, and it is where the frame budget actually goes.

MarkText gets acceptable interactivity from a Marked.js-based lexer purely because
its *renderer* is incremental — snabbdom vDOM diffing patches only changed nodes.
The parser is not the bottleneck; the DOM/paint layer is.

For us: diff the block list, patch only changed blocks in the UI layer.

## Virtualization is non-negotiable

MarkText [#4887](https://github.com/marktext/marktext/issues/4887) shows that even
a ground-up engine rewrite doesn't save you from freezing on a large file if you
materialize the whole document. A ~300,000-word file freezes v0.20.0-rc.1 for
4–5 minutes.

Two levels:

1. **Within a document** — render only visible blocks. `file open → first paint`
   should be O(1): mmap plus parse only the viewport. See
   [06](06-performance-targets.md).
2. **Across documents** — a tab is a lightweight handle until it becomes visible.
   Obsidian's `DeferredView` (v1.7.2+) is the only implementation in this app
   category and is the pattern to copy. Session restore persists handles, not
   content.

## Syntax highlighting for fenced code

**syntect 5.3 + [two-face](https://docs.rs/two-face) 0.5.1**, lazy, cached per
block by content hash.

Rationale: `tree-sitter-highlight` **is not incremental** — its
[signature](https://docs.rs/tree-sitter-highlight/latest/tree_sitter_highlight/struct.Highlighter.html)
takes `source: &[u8]` with **no `old_tree` parameter** and re-parses from scratch
every call. That surprises people. It's why Helix wrote `tree_house` instead.

The pattern worth naming: every project that *renders* code (bat, delta, Zola,
mdBook) stayed on syntect; every project that *edits* code (Helix, Zed, Lapce)
uses tree-sitter — because they need the tree for indentation, textobjects and
outlines anyway, not because it highlights faster.

We are rendering, not editing, code inside fences.

Two caveats:

- **Audit two-face's per-syntax licences before shipping.** The crate is
  MIT/Apache but embedded Sublime definitions are not uniformly permissive — see
  its `acknowledgement` module.
- Prefer the `onig` backend unless the C dependency blocks you. `fancy-regex` is
  about half the speed, absurdly slow in debug builds, and *drops some syntaxes
  entirely*.

  **The C dependency does block it, or at least outranks this preference, and
  that was not noticed when this was written.** Oniguruma is C, a fence's
  contents are untrusted file contents, and `AGENTS.md` rejects a C parser at a
  file-parsing trust boundary — it is the stated reason `tree-sitter` lost the
  YAML decision. So the two rules in this repository disagree, the trust
  boundary wins on precedent, and `fancy-regex`'s cost has to be measured rather
  than accepted from the sentence above. **MDEDIT-190** decides it with
  evidence.

Also cache `SyntaxReference` lookups — `SyntaxSet` search is a linear scan.

Measured syntect numbers (its own README, the only concrete highlighting figures
found anywhere): 9,200-line jQuery in 600 ms (Sublime 98 ms, VS Code ~2 s, Atom
6 s); simple XML ~50,000 lines/sec; ~23 ms to load from binary dump; ~1.9 ms to
parse and highlight a 30-line file.

## Build order

1. **Marker locator**, standalone, property-tested. Gate on the byte-for-byte
   invariant.
2. **Windows text spike** — Latin + CJK + Arabic + colour emoji in one paragraph,
   typed into with a Japanese IME. Tests `fontique#644` and the IMM32 path.
3. Buffer + full-reparse pipeline + inline decoration. No incrementality.
4. Block widgets for tables and fences.
5. Tab model with deferred materialization + session/backup
   ([05](05-windows-platform.md)).
6. Profile. Add block-level dirty regions only if the numbers demand it.
