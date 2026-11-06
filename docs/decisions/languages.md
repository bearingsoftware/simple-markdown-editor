# Languages — Markdown, JSON, YAML, XML, TOML, INI and the boundary between them

The language trait and its implementations, what each of them conceals, validates and reformats, and how a keystroke re-parses a region rather than a document.

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
  `syntect` **failed MSRV 1.87** (`plist` and `time` need 1.88) — **that ground has gone, the floor
  is 1.90 as of MDEDIT-189, and MDEDIT-190 adopted syntect for fenced code, on fancy-regex (see its entry below)**;
  `tree-sitter` passes but is a C
  parser at a file-parsing trust boundary, which is the CVE class this project's language choice
  exists to remove; and a real YAML parser (`saphyr-parser`) **failed 3 of 4** mid-edit documents —
  a half-typed flow sequence, inconsistent indentation, and a tab — which is what a file being
  written looks like.
- **The YAML lexer highlights; it does not parse.** It understands comments, document markers, keys,
  sequences, quoted scalars, anchors, aliases, tags, flow collections and block scalars including
  their indentation. It approximates a key as text before `:` at end-of-line or `: ` before a
  comment, which is wrong for the explicit `? key` form and for a colon-space inside a flow scalar.
  Both mis-*colour* rather than mis-*cover*, and tiling is property-tested either way.
- **`.xml` opens highlighted, and its nesting is a counter rather than a stack.** MDEDIT-94. Same
  hand-written, error-tolerant shape as JSON's and YAML's, for the reason recorded on those: a real
  parser refuses half-typed input, and `<person id=` is what an attribute looks like a keystroke
  before it is finished. `quick-xml` and `roxmltree` both stop at the first ill-formed construct and
  hand back no spans for the bytes after it.

  XML nests without limit, so recursive descent overflows on `<a><a><a>…`. JSON's validator answered
  that with an explicit heap stack; this goes one step further and keeps **a `usize` counter**,
  because nothing reads the element names — highlighting `</b>` does not depend on which element it
  closes, and this crate has no `validate` to say the wrong one was typed. So a depth bomb costs one
  integer that never grows and needs no cap to be safe. It saturates rather than wrapping, so
  `overflow-checks` cannot turn a pathological file into a panic. Property-tested at 200,000 levels.

  **It is the first language to answer `analyze_region` with `None`,** which is the case that
  `Option` was put there for and which nothing had exercised. Every other language recovers its state
  from a column-zero boundary; XML has none, because a line beginning `</item>` says nothing about
  how deep it is and depth is the whole of the state — an element at depth one is a block and the
  same element at depth two is not. So every keystroke re-parses the document, and the measurement
  below is what that costs.

  **A block is a child of the root**, which is JSON's rule rather than a new one. A whole document is
  one element, so blocks-per-element at every depth would nest blocks the renderer has no use for and
  a block per document would defeat the per-block cache entirely.

  **Cost, at the 60 KiB `docs/research/04` states the keystroke budget for**: 0.61 ms p50 for deeply
  nested elements, 1.15 ms p50 for an attribute-heavy configuration file, against a 3 ms budget —
  and since there is no region parse, that *is* the keystroke cost rather than a ceiling on it. The
  doubling ladder in `crates/xml/tests/lex_cost.rs` is linear over 32 KiB → 1 MiB, which is the part
  worth re-running: MDEDIT-99 was quadratic and every unit fixture was too short to show it. Segment
  density is 184/KiB nested and 306/KiB attribute-heavy, against the JSON figure MDEDIT-37 is chasing.
- **An element name is `Token::Tag`, and that variant was added rather than defaulted.** The plan left
  it open between `Key` and a new variant. In `<a href="x">` the element name and the attribute name
  are different things and every highlighter colours them differently, so collapsing both into `Key`
  would draw the landmark and the detail identically. The theme value is **a step up the same
  lavender ramp** as `token_key` rather than a new hue: the pack's four hue families are unchanged,
  and rose stays reserved for errors.
- **`.toml` opens highlighted, from its own lexer rather than a shared one with INI.** MDEDIT-95. No
  `toml` crate: it deserializes, and this needs spans and error tolerance, which is the argument that
  rejected `serde_json` and `saphyr` unchanged.

  **TOML and INI look alike and are not alike**, which is why sharing was rejected up front rather
  than tried. `key = "value"` is *one token* in INI — everything after the separator is the value,
  quotes included — and in TOML it is a quoted string with escapes, inside a syntax that also has
  arrays, inline tables, dates and three other string forms. A shared lexer would have to be told
  which language it was, which is two lexers wearing one name.

  It understands tables and array tables, bare and dotted and quoted keys, all four string forms
  including both multi-line ones, integers in four bases, floats, booleans, dates and times, arrays,
  inline tables and `#` comments. **Anything starting with a digit or a sign is a number**, loosely
  and on purpose: that makes `1979-05-27` a number and `1.2.3` one too, and the second is what `1.23`
  looks like a keystroke before it is finished.

  **A table header at column zero is a safe restart point, so it answers `analyze_region`.** The
  state a TOML lexer cannot recover from a slice is whether it is inside a multi-line string or an
  unclosed array, and a block here is *defined* as a column-zero line that is inside neither — so the
  contract and the block list are the same statement. That is checked rather than argued: a property
  test restarts at **every** block of a generated document and compares against the whole-document
  parse, and `scoped_data.rs` gained the TOML half of MDEDIT-36's gate.

  **Cost at 60 KiB**: 0.70 ms p50 for a document of small tables at 383 segments/KiB, 0.20 ms for one
  of long multi-line strings at 61/KiB, against the 3 ms budget — and unlike XML those are ceilings
  rather than the keystroke cost, because a keystroke re-lexes a region. Linear over 32 KiB → 1 MiB.
- **`.ini` and `.cfg` open highlighted, and the crate states its dialect because there is no
  specification to be right against.** MDEDIT-96. INI is a family of conventions that agree on
  `[section]` and `key = value` and disagree on nearly everything else, so the four choices are
  written down in `crates/ini/src/lib.rs` and pinned by name in its tests:

  - **A comment is `;` or `#` at the start of a line, never mid-line** — `configparser`'s default,
    and the one that matters: `path = C:\a;C:\b` is a value, not a value with a comment after it, and
    guessing the other way takes bytes off the end of a value on screen.
  - **`=` or `:` separates, whichever comes first**, so `query = a=b&c=d` has one separator.
  - **A value is the whole rest of the line, one run**, classified as a unit.
  - **A line with no separator continues the value above it.** `configparser` decides that by
    indentation; deciding it by the missing separator reads the same to a person and also copes with
    the files that indent their keys under a header.

  **`.cfg` is not a format, it is a file extension**, and mapping it here is a guess of the same kind
  as the `data.txt` note `detect.rs` already carries. Python's `setup.cfg` is INI and plenty of other
  `.cfg` files are not. It is a better guess than the plain text it would otherwise get and it is
  still a guess, which is why it is a comment in `detect.rs` and an assertion in `crates/app/tests/ini.rs`
  rather than one line of code nobody would notice removing.

  A column-zero line carries no state from the line above, so `analyze_region` is the easiest claim
  of the four languages that make it — held to the same property test and the same fuzz target
  anyway. **Cost at 60 KiB**: 0.61 ms p50 either shape, at ~225 segments/KiB, and a keystroke pays a
  region rather than that. Linear over 32 KiB → 1 MiB.
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

## MDEDIT-190: fenced code is highlighted by `syntect`, chosen by the info string alone

- **Backend: `syntect` 5.3.0 on `fancy-regex`, never Oniguruma.** A fence's contents are untrusted
  file contents and Oniguruma is C — the same ground `tree-sitter` was rejected on above. `syntect`
  is taken with `default-features = false` and `default-syntaxes` + `regex-fancy`; `deny.toml` bans
  `onig` and `onig_sys` so a feature flip cannot bring them back (`cargo tree -i onig` finds
  nothing). The MSRV objection recorded under YAML has gone with the 1.90 floor. The hand-written
  lexers stay for whole-file languages: they guarantee their spans tile the source, which syntect's
  styles do not.
- **`two-face` was tried and dropped.** It bundles about 200 of `bat`'s syntaxes under mixed
  licences, and no definition with an unknown or copyleft licence may ship inside `mdedit.exe`.
  `syntect`'s own bundled set is the sublimehq/Packages dump.
- **Licence of the bundled set, audited.** The `syntect` crate itself is MIT, but it ships no
  licence file for the syntax dump. The 5.3.0 dump is built from `sublimehq/Packages` at commit
  `fa6b8629c95041bf262d4c1dab95c456a0530122` (2017-11-13), and **all 74 `.sublime-syntax` files are
  in it; none are excluded.** The top-level `LICENSE` is a permissive Sublime grant (copy, use,
  modify, sell and distribute, as is). `Rust/LICENSE.txt` is MIT with no copyright line;
  `YAML.sublime-syntax` carries an MIT header, (c) 2015 FichteFoll, and `C#.sublime-syntax` an MIT
  header, (c) 2016 Sublime HQ. **No copyleft licence was found.** Shipping the notices is tracked
  by MDEDIT-200. **The syntaxes date from 2017**, so a newer language feature may draw plain.
- **The language comes from the info string and nothing else.** First word, lower-cased. No info
  string means no highlighting, and nothing reads the code to guess; a JSON body under ```` ```text ````
  is plain text. The set loads lazily, on the first fence that names a language; `is_loaded()`
  exists so a test can prove it (`crates/highlight/tests/lazy.rs`, a binary of its own: that is cheap in the highlight crate).
- **Colours are the existing token colours**, by a scope-prefix table in one place
  (`crates/highlight/src/lib.rs`), longest prefix wins, and a comment or string anywhere in the
  scope stack governs what is inside it. No syntect theme is imported.
- **Coverage: 75 syntaxes.** Plain Text, ASP, ActionScript, AppleScript, Batch File, C, C#, C++,
  Clojure, CSS, D, Diff, Erlang, Go, Graphviz, Groovy, Haskell, HTML, Java, JavaDoc, JSON,
  JavaScript, LaTeX, TeX, BibTeX, Lisp, Lua, Makefile, Markdown, MATLAB, Objective-C(++), OCaml,
  Pascal, Perl, PHP, Python, R, reStructuredText, Ruby (and Rails/Haml variants), Rust, Scala, SQL,
  Bash/shell, Tcl, Textile, XML, YAML, and a few regex and output grammars. **Absent here: TOML,
  TypeScript, Dockerfile, PowerShell, Kotlin, Swift, JSX/TSX, Vue** — see MDEDIT-201 below for what
  was added; Swift, Vue and a real TypeScript syntax are still absent.
- **Dropped syntaxes: none found.** `every_bundled_syntax_parses` (`#[ignore]`) runs ten sample
  lines through all 75 and reports 0 that error under `fancy-regex`. Syntax regexes compile lazily,
  so this proves the contexts those lines reach, not every rule.
- **Highlighting runs on a worker thread, and that is the one second thread** (`crates/app/src/
  highlighting.rs`, mirroring `decoding.rs`). `AGENTS.md` asked for a number from a cost harness
  before a second worker, and the first version, which highlighted on the UI thread, supplied it
  (release build): a 54 ms cold load, about 0.22 ms a line (4 us a byte), and a p50 of 169 ms / p99
  of 303 ms for a one-character edit in a 300-line fence, against an 8.3 ms p99. A size cap on the
  UI thread (1,536 bytes) would have made ordinary fences plain, so the work moved instead.
- **The drawing thread never loads the syntax set and never parses code.** It hashes a fence's info
  string and body, looks in a store (`Pending`, `Ready` or `Plain`, capped at 256 entries), and
  draws plain until an answer is there. The test for that is in `crates/app/tests/fence_colours.rs`
  (not a binary of its own, which would cost a full app link, MDEDIT-182): it needs a process to
  itself, which nextest gives every test and CI runs, and it panics rather than skips if the set is
  already loaded. The worker is held back, so a loaded set could only have come from the drawing
  thread.
  The view takes ready spans as a parameter (`build_runs_highlighted`) and stays a pure function.
- **Stale work: the worker highlights only the newest request per fence** (identified by where its
  body starts) out of everything queued, and answers each skipped one `Superseded`. The editor then
  drops that key's `Pending` entry, so undoing back to those bytes asks again instead of waiting for
  an answer that was never coming. No answer carries spans for a body other than its key's.
- **The layout-cache key carries the highlight state.** Colours are read from the run list at draw
  time, but the layout holds the glyphs and a literal (`true`, `None`) draws italic, so a plain
  layout cannot be served once colours exist. The state is a separate field of `ChunkKey`, not part
  of `content`, which the scroll anchor matches on: colours landing must not lose the reader's
  place. `an_arriving_answer_rebuilds_its_own_chunk_and_no_other` pins it.
- **No flash to plain while typing.** The editor remembers the last answered version of each fence
  by where its body starts. While a new version is pending, spans ending before the first differing
  byte are kept (right, because the highlighter's state is causal), spans starting after the common
  suffix are kept shifted by the length delta (close enough until the answer lands), and spans
  overlapping the edit are dropped. A changed info string reuses nothing.
- **Caps, on the worker:** a fence body over **128 KiB** is `Plain` (about half a second of work at
  4 us a byte), and a line over **4,096 bytes** ends the highlighting there, the rest drawing plain
  (a 3,000-character line took 121 ms). Both are `pub const` in `mdedit-highlight`.
- **Numbers, release build.** Worker, per job: 30 lines 6.9 ms, 300 lines 74 ms, 1,000 lines 224 ms
  (`highlight_cost`, quiet machine); the same sizes inside the app harness on a busier run took 25,
  263 and 811 ms, and the cold load 54 to 130 ms. UI, first frame holding a pending 300-line fence:
  p50 2.65 ms against 2.51 ms for the same fence untagged, so the cost of asking is noise. UI, a
  keystroke in a 300-line fence reusing the previous spans: p50 3.9 ms, p99 9.6 ms (the untagged
  baseline in the same run was noisy at 10 / 52 ms, so only the first frame is a clean comparison).
- **Error handling.** A line `parse_line` refuses ends highlighting there and the rest draws plain.
  No test input reaches that arm — no sample made `fancy-regex` error — so it is unverified by test;
  a mutation that unwraps it survived.
- **Remembered versions, and the worker's survival.** The last answered version of a fence is kept
  only for fences that are laid out (visible, or holding the caret), at most 64, evicting one when
  full and never copying a body twice. A reused layout's key includes the hash of the version its
  spans came from, so two previous versions cannot share one. The worker runs each job under
  `catch_unwind` (the workspace unwinds) and answers a panicking fence plain, so one bad fence does
  not leave every later one plain; the test uses a stand-in job that panics.

## MDEDIT-201: TOML, Dockerfile, Kotlin and PowerShell fences, and TypeScript as JavaScript

- **Four syntaxes are vendored** under `crates/highlight/syntaxes/<Language>/`, each with its upstream
  licence and a `SOURCE` file giving the repo, pinned commit, path and every edit: TOML
  (`jasonwilliams/sublime_toml_highlighting`, MIT), Dockerfile (`asbjornenge/Docker.tmbundle`, MIT),
  Kotlin (`guille/sublime-kotlin`, Unlicense) and PowerShell (`PowerShell/EditorSyntax`, MIT, via
  `bat`'s converted file). PowerShell needed one edit: a `\g<1>` subroutine call, which `fancy-regex`
  rejects, became the character class it called. The notices are MDEDIT-200's.
- **TypeScript has no syntax of its own.** `ts`, `tsx`, `typescript`, `jsx`, `mts` and `cts` map to the
  bundled JavaScript (`ALIASES` in `lib.rs`, with `pwsh`, `ps` and `docker`). The maintained TypeScript
  syntaxes are sublimehq's `version: 2`, which `extends` another and uses `branch_point`, `fail` and
  `pop: N`; syntect 5.3.0 reads none of those and drops them silently. Interim, until syntect 6.0.
- **Newer Sublime syntaxes wait for syntect 6.0.** Its support is unreleased, and open #707 hangs the
  parser on a one-line TypeScript ternary, which a fence's untrusted text could trigger.
  `vendored_syntaxes_use_no_features_syntect_5_ignores` fails on a vendored file that grows any of them.
- **The set is built at compile time.** `crates/highlight/build.rs` loads syntect's defaults, adds the
  vendored files and dumps an uncompressed 385 KB set to `OUT_DIR`; `lib.rs` loads it with
  `from_uncompressed_data`. The YAML loader is a build-dependency only, so the binary has none.
  A bad vendored file fails the build, which is how the PowerShell edit was found.
- **Numbers (release).** First `highlight` call, 10-run median: 46 ms before (quiet machine) and 103
  ms after, but an interleaved run on a busy machine gave 103 against 113 ms, so no measurable change.
  `mdedit.exe`: 12,550,144 to 12,579,328 bytes (+29 KB). The set has 79 syntaxes.
