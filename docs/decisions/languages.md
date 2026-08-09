# Languages — Markdown, JSON, YAML and the boundary between them

The language trait and its three implementations, what each of them conceals, validates and reformats, and how a keystroke re-parses a region rather than a document.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **Four parser extensions are on, and each one is on for a stated reason.** Strikethrough since day
  one; **task lists** from MDEDIT-88; **footnotes** and **definition lists** from MDEDIT-90. GFM
  tables remain off and degrade to content, which is the right answer until a renderer handles them.

  The formatting bar has inserted `- [ ] ` since MDEDIT-52 while `ENABLE_TASKLISTS` was off in the
  locator, so **the write path and the read path disagreed about the language**: the editor rendered
  its own output as a bullet followed by a literal pair of brackets. Found by auditing the editor
  against the Markdown Guide's extended syntax rather than by using it.

  **The checkbox gets a `Marker` of its own**, not the bullet's. `Tag::Item` covers the bullet; the
  box arrives as a leaf `Event::TaskListMarker` with its own range and would otherwise fall to the
  item's content by subtraction. And they are different things on screen: a bullet is *scaffolding*
  and a box is **state**, which is why it takes the pack's constant colour where the bullet takes
  key. `- [x] done` is four spans — bullet, box, the item's space, then the text.

  **The extension is free, and on a document that uses it, cheaper.** Interleaved A/B in one process
  over the same source, `parse-only`:

  | Document | Off | On | |
  | --- | --- | --- | --- |
  | no task lists, 6 / 60 / 310 KiB | 75.0 µs / 778 µs / 4.86 ms | 75.6 µs / 782 µs / 4.91 ms | **+0.5 to +1.0%** |
  | task lists, 6 / 60 / 310 KiB | 89.4 µs / 1.18 ms / 8.34 ms | 77.1 µs / 1.02 ms / 7.35 ms | **−12 to −14%** |

  Faster on the documents that use it because `[ ]` read as literal text costs *more* events than one
  `TaskListMarker` — the bracket opens a link scan that goes nowhere. `criterion` could not see any
  of this: the runner's run-to-run spread is ±25% at 310 KiB, which is twenty times the effect.

  **Footnotes and definition lists cost +0.7% on a document that has neither and +4% on one that has
  both** (MDEDIT-90, same method). A footnote's `[^1]` is `link`-coloured, because it is a landmark
  a reader follows rather than scaffolding — the same argument that sends a destination to `link`.

  **Three of MDEDIT-90's four candidates were rejected on measurement, and one of them was rejected
  for losing a line.** The ticket assumed `ENABLE_GFM` gives bare-URL autolinks; in 0.13 it gives
  blockquote alert tags and nothing else, which this editor does not render and which changes no
  classification at all. `ENABLE_SUBSCRIPT` reinterprets `~x~` from strikethrough to subscript, which
  silently changes what existing files mean. And **`ENABLE_HEADING_ATTRIBUTES` makes an ATX heading
  swallow the following line when the heading ends with a lone `\r`** — reported as
  [pulldown-cmark#1132](https://github.com/pulldown-cmark/pulldown-cmark/issues/1132), minimised to
  `"# a\r$"`, and disqualifying here because this editor preserves lone CRs on purpose.

  **The property test's alphabet grew with the flags**, which is the part worth copying: `[^`, `]:`,
  `{#`, `}`, `:` and `^` were added so the generator could spell the new constructs. It found four
  defects in the first minute — the heading one above, and three in this crate. Two of those three
  were *pre-existing* and simply unreachable before: a `>` inside a footnote label being relabelled
  as a quote prefix, and a `)` on a quote's lazy continuation line passing a plausibility check that
  was shared between quotes and lists. **An extension that adds no fixture but widens the generator
  is still earning its keep.**
- **A document can be told it is wrong, and that is a second strict pass rather than a stricter
  lexer.** MDEDIT-47. `Language::validate` returns `Diagnostic`s — a `ByteSpan` and a `&'static str`
  — and defaults to none, so **Markdown opts out by existing**: there is no such thing as invalid
  Markdown. Only JSON implements it. The lexers stay error-tolerant, which is not a detail: `{"a":`
  is what every JSON file looks like halfway through being typed, and a parse that stopped there
  would unhighlight the rest of the document at the moment the user is looking at it. That is why
  `serde_json` and `saphyr` were rejected, and adding failure to `analyze` would give it back.

  **It is the fourth thing on the idle timer, and a keystroke pays 3 ns for it.** Measured at 412 KB
  of JSON: the check alone is 1.3 ms, and what a frame pays is one comparison of `(tab, revision)`
  against the pair the last check was computed for. The keystroke A/B was inconclusive at ±50% run
  to run on a loaded machine, which is why the per-frame number is the one that answers it — see
  `crates/app/tests/validate_cost.rs`, and `json_typing_cost.rs`, which uses only long-standing API
  so it can be run against an older commit.

  **An edit clears the count rather than ageing it.** The word count shows a stale number because a
  stale number is nearly right and costs nothing; a stale *diagnostic* points at bytes that have
  moved, and Tools → Validate Document puts the caret on one. Same `(tab, revision)` key as the find
  bar's match list, and for the same reason — two documents nobody has typed in are both at revision
  zero. The menu item **recomputes before it jumps**, because the count on the bar may be an idle
  interval old and "take me to it" may not be.

  **The checker has an explicit stack, and that is a security decision.** JSON nests without limit,
  so recursive descent overflows on `[[[[[…` — a file, from the internet, opened in an editor. Safe
  Rust does not save you from a stack overflow, and `AGENTS.md` names hand-rolled file parsing as the
  CVE class this project must not reintroduce. It also stops at 200 diagnostics: nobody is fixing the
  201st. Absence of errors is **silence, not "0 errors"**, and an empty document is silent too — a
  new file is empty, and diagnosing that is diagnosing the user's existence.
- **A JSON document can be reformatted, and that is the only whole-document rewrite there is.**
  MDEDIT-48 — the decision itself is in the byte-preservation rules above, because that is the rule
  it is an exception to. `Language::format` returns `Option<Result<String, Diagnostic>>`, and the two
  layers answer different questions: `None` is **"this language does not reformat"** (Markdown has no
  canonical form; YAML cannot round-trip its comments and anchors), which greys the menu item, and
  `Err` is "it does, but not this document", which shows a message naming the line. Nothing compares
  `Language::name()` against `"JSON"`.

  **It emits the original bytes of every token**, which is the whole of how `1.50` stays `1.50`,
  `1e3` stays `1e3` and `-0` stays `-0`. Re-serializing through a number type loses all three, and
  through a map loses key order. Nothing here parses a value — it walks MDEDIT-47's token stream and
  decides where the line breaks go, which is also why it needed no second tokenizer. Property-tested
  both ways: the result is still valid JSON, and every non-whitespace byte is unchanged.

  **MDEDIT-47's validator is the gate**, so a half-typed document is refused rather than mangled — a
  format that "fixed" malformed input would be inventing structure the user has not finished typing.
  An already-formatted document is left completely alone, so there is no undo press that appears to
  do nothing.

  **The menu item's enabled state is compared per frame, not hooked.** Which document is active
  changes on a great many paths, and a list of call sites is a list somebody forgets to add to — the
  same lesson as `counted_for` and `floating_for`, in push form, with `tools_state` holding what the
  menu was last told so the native call happens only when the answer changes.
- **An unknown extension is plain text, and that reverses a written-down decision.** MDEDIT-93.
  `detect.rs` said, deliberately, that an unknown extension or none is Markdown — *"this is a
  Markdown editor; that is the right default and not an admission of defeat"*.

  The reversal is that **guessing Markdown means styling a file that is not Markdown**, and
  concealment makes a wrong guess worse than a wrong colour: a `notes.txt` of shell commands gets its
  `#` comments drawn as headings and *loses the asterisks* in `*.log`. So the cue that a file is
  Markdown is the same cue that makes it Markdown when you save it — its name. `.md` is now spelled
  out like every other extension rather than being what is left over.

  **`Plain` is the trait's identity element rather than a parser**: one content span, no markers, no
  blocks, conceals false, no validation, no formatting. It still tiles, because tiling is not
  optional for any language.

  **It also fixed a defect found while planning it.** `Workspace::bind_to_path` set the origin and
  the file and left the document's language alone, so an untitled buffer saved as `data.json` stayed
  Markdown until the tab was closed and reopened. "Plain until you save it as something" cannot work
  without that, and it was worth having either way.

  **A language change is a cache change**, which the plan named as the thing to get wrong. The layout
  cache is keyed by content, and its key now carries the language — following the precedent the
  drawing mode set, in the key rather than clearing on a change, so switching back is free. Worth
  knowing: **no pair of today's languages can actually collide on the rest of that key**, because a
  plain document has no blocks and every other language produces some, so removing the language from
  it breaks no test. It is there because relying on a second field to happen to differ is relying on
  an accident, and that is said in the comment rather than dressed up as a tested property.

  Release note: anything without a recognised extension stops being Markdown, so a `notes.txt` full
  of Markdown will look different.
- **`.json` files open highlighted and never concealed**, by extension only — a JSON file named
  `data.txt` is plain text, deliberately. Content sniffing is the trust boundary the 2023
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
- **JSON and YAML re-parse a region, like Markdown — but a keystroke in them still costs more than
  one in Markdown, and the parse is not why.** MDEDIT-36 made both languages restart at a block
  boundary: JSON is one container deep there and YAML is provably outside every block scalar, and
  `Document::scope_for` already snapped every region to one. That halved the document-level cost of
  an edit on a 357 KB JSON, from ~3.7 ms of lexing to 1.96 ms end to end — **and moved the app's
  keystroke latency not at all**, p50 10.03 ms whole against 10.24 scoped in a same-session A/B.
  The remaining ~8 ms is elsewhere; MDEDIT-37 carries finding it, with segment density as the
  suspect (JSON 140,009 segments to Markdown's 19,410).
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
  Nothing walks parent or child links because nothing has needed to. `block_at` **binary-searches**
  rather than scanning — MDEDIT-37 found it was the largest single cost in drawing a frame, because
  the renderer asks it once per styled run: 4,001 blocks times thirty-six runs times twenty visible
  chunks is 2.9 million comparisons a frame for a 357 KB JSON. Fixing it took `build_runs_in` on one
  JSON chunk from 0.064 ms to 0.005.

- **A delimiter run is never whitespace, and it used to be.** MDEDIT-112. `pulldown-cmark` strips a
  soft break's surrounding whitespace from its `Text` events, so the gap between two children fell
  through the locator's subtraction and was emitted as the *enclosing* construct's marker: an inline
  construct spanning a soft break reported an `Emphasis` marker holding a space.

  **It never mis-covered.** The tiling gate held throughout, which is exactly why it survived — the
  property that guards this crate is that the segments reproduce the source, and a mis-*labelled*
  span does that perfectly. What it did was mis-colour, and worse, **conceal**: a marker is hidden in
  preview, so the space vanished from the rendered line.

  The rule is now unconditional in `normalise_line_syntax`: a whitespace byte carrying `Emphasis`,
  `Strong`, `Strikethrough` or `CodeSpan` becomes content. Those four are the constructs whose
  markers are `*`, `_`, `~` and a backtick and nothing else.

  **Three things narrowed it, each found by a test rather than by thinking.** `Link` and `Image` are
  *not* in that set — `[a](/url\n  "title")` puts a line's opening whitespace **inside** the closing
  marker, between the destination and the title, and concealing it is right; CommonMark spec example
  510 failed when the rule was drawn around "inline" instead. The first fix only cleared a line's
  *leading* whitespace and the property test found a trailing tab in seconds, because the strip
  happens at both ends of the break. And `u8::is_ascii_whitespace` **excludes the vertical tab**,
  which CommonMark counts — so `is_markdown_space` spells the set out.

  `assert_marker_shapes` lost the tolerance it carried for this, which is the real test that the fix
  landed. Removing it surfaced one more thing: a `HardBreak`'s reported range covers whatever
  trailing whitespace preceded the newline, not just the two spaces or backslash that were written.
  That is the format being loose rather than the locator being wrong.
