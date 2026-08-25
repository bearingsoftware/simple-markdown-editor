# Rendering — palette, concealment and what is drawn on the text

The two palettes and the roles in them, how Markdown syntax is concealed and coloured, and the marks the editor puts on or under a line.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **A table is drawn as a grid, and it needed no second architecture to do it.** MDEDIT-126.
  `AGENTS.md` had promised a *model-backed block widget* since before there was a renderer, on the
  reasoning that "pure decoration over a linear character stream cannot do 2D layout". The reasoning
  was sound and the premise was wrong: `mdedit_text::Layout` is a `Vec<LayoutLine>` whose glyphs each
  carry their own `pos` and `source`, and nothing in it requires those positions to march left to
  right. **The linearity is a property of what a text engine returns, not of the type.**

  So `crates/app/src/table.rs` lays each cell out on its own, sizes every column to its widest cell,
  translates the glyphs into place and assembles one layout whose lines are the table's rows.
  `draw_chunks`, `row_selection` and the layout cache are all untouched, because the glyphs still
  carry truthful source spans and one table row is still one visual row. The model half is real —
  `BlockKind::Table`, `TableRow` and `TableCell` come from the parse, not from splitting on pipes —
  and it is what MDEDIT-128 will insert rows and columns against.

  **It is ruled like a table, not gapped like one.** The first slice separated the columns with a
  plain gap standing in for the pipe; the second gave every cell a box — [`PADDING_X`] and
  [`PADDING_Y`] inside it, a rule at every row and column boundary, the outer border, and a wash
  behind the header row. `table::Grid` carries the boxes because the layout only carries glyphs, and
  `draw_table` rules them once per chunk *before* any row, so the caret line, a search match and the
  selection all land on top of the wash and the glyphs on top of those — painter's order, as with the
  rest of the chrome.

  `Rule` could not be reused for this. That type is horizontal, is measured from a baseline, and
  takes its colour from the run it underlines; a border is none of those. Both the rules and the wash
  draw in `Theme::line`, the one translucent hairline role, which lands correctly on either ground.
  `panel` cannot do the wash — it is pure white in light mode, which is the page. **`Ctrl+/` rules
  nothing**, for the same reason it gets no grid.

  **An empty cell has no glyphs, so it has no geometry** — and that turned out to be two defects
  rather than one, both found by eye on MDEDIT-128 and neither visible to a test that only ever laid
  out a table someone had already typed into. A text engine handed nothing returns a layout with *no
  rows*, the same answer that made a blank document draw no caret in MDEDIT-81, so a freshly inserted
  row came out six pixels tall. And hit-testing walked glyphs, so a freshly inserted column had
  nothing to land on and a click carried past it into whichever column had text next.

  A cell now lays out to a line whether or not anyone has typed in it, and `table::Grid` carries its
  **cell** spans as well as its row spans, so a point resolves to a *column box* first and the cell
  that column names answers. Every pixel across a table belongs to a column — the two-dimensional
  version of the rule MDEDIT-127's picker already follows, and a rule the caret needs as much as the
  pointer: `place_of` puts the caret at the cell's own left edge when no glyph in it can say where.

  **Hit-testing was the one consumer that did not survive untouched**, and the first manual look is
  what found it. `hit.rs` finds a row's bytes by counting newlines, which is sound everywhere else
  because wrapping only ever *adds* visual rows. A grid is the one layout that **drops** a source
  line — the delimiter row draws as nothing — so visual row 1 is source line 2, `place_of` matched no
  row, and the caret could not be placed anywhere below the header. A grid therefore hands `hit` its
  row spans (`table::row_spans`) and the newline count stays the fallback for everything else. That
  is a mapping rather than a second hit test: one `offset_in_row` and one `place_of` still answer for
  both. The delimiter row gets no span, so the caret steps over it as MDEDIT-118 has it step over any
  concealed syntax.

  **The module takes laid-out cells rather than a text engine**, as `toolbar.rs` and `popup.rs` take
  geometry rather than a window, and for the same reason: the placement is then testable without one.
  It is also what the borrow checker wants, since finding the cells needs `&` the document while
  laying one out needs `&mut` the engine.

  Three things this cost, each a deliberate limit rather than an oversight:

  - **A cell does not wrap.** Cells are laid out at an unbounded width, so a table wider than the
    window pans rather than reflows. A column width that depended on the viewport would re-lay the
    whole table on every resize, and the cache key does not carry one.
  - **A table inside a block quote stays as pipe rows.** `blocks::split` only makes a chunk from a
    depth-zero block, so a quoted table is part of the quote's chunk and never reaches the grid.
  - **`Ctrl+/` never gets a grid**, which is the point of the mode rather than a gap: plain markup
    exists to show the bytes, and the bytes of a table are pipe rows.

  **The trailing empty line is load-bearing and the failure is silent.** A chunk whose bytes end in a
  newline gets an empty last line from the text engine, and `Cached` drops it again via
  `blocks::drops_last_line`. An assembled layout without one loses its **last table row** instead —
  from the height and from the drawing both, with nothing on screen to say so. `assemble` emits it,
  so its output is shaped exactly like a linear layout, and a test names the trap.

  **`LayoutLine::baseline` is measured from the top of the block, not from the line above** — and a
  cell is laid out as a block of its own, so its baseline is its own ascent. Reading a row's baseline
  straight off its cells therefore gives every row the same number and the table draws as one
  overstruck line. `assemble` carries a running top down the rows. This shipped to the first manual
  look: nine tests passed over it because each used a single row, where the bug is invisible. A
  second row in any one of them would have caught it.

  Why this is here and not on MDEDIT-89: that ticket asked for the pipes to be *coloured*, which
  MDEDIT-118 had already made impossible — a marker is never drawn, so classifying them as syntax
  erases the columns rather than tinting them. The flag and the grid had to arrive together, and
  MDEDIT-89 was closed as superseded.
- **Preview shows no syntax at all, including under the caret.** MDEDIT-118, and it overturns the
  rule the editor was built on. `Document::is_revealed` used to be reveal-on-caret, modelled on
  Typora: a marker concealed until the selection came near it, then shown dimmed so it could be
  edited in place. Its own doc comment called that "the whole live-preview illusion".

  The illusion is better without it. A heading is large text, and what says *which* heading it is is
  the caret's own height — `caret_position` already takes that from the laid-out row, so the cue cost
  nothing to build. Syntax that appears as you approach and vanishes as you leave is a fourth thing
  on screen that moves on its own. `Ctrl+/` (plain markup) is the escape hatch and is unchanged; it
  is now the **only** mode that draws a marker, which is where a test asserting a marker's colour or
  slant has to go.

  **The predicate no longer takes a `Selection`, and neither does `view::build_runs`.** Leaving an
  unread parameter would have read as a dependency that is not there, and this one has a consequence:
  what a chunk draws is now a function of its bytes, its kind and the width — exactly the layout
  cache's key. So **the caret's own chunk is cached like every other**, and the one chunk that is
  laid out on every keystroke stopped being.

  **What it cost is that markers stopped being editable in place**, and that is why the ticket is
  three changes rather than one — shipping the concealment alone would have given an editor whose
  headings could not be un-headed:

  - `Document::concealed_at` answers "is the caret standing in something invisible", by binary search
    over the segments. Half-open, so the position *after* a marker — the one the caret rests at, and
    the one Backspace acts from — is not in it.
  - **A position inside a concealed marker is not a place the caret may be.** Bytes 0, 1 and 2 of
    `# Title` are all the same x, so Left and Right normalise through the whole marker before and
    after stepping. Right from a line's start lands after the first letter; Left from that letter
    lands at the end of the line above, because a marker's *start* is as unreachable as its middle.
  - **Backspace and Delete take a concealed marker whole**, in one transaction so one Ctrl+Z restores
    every byte. Half of an invisible `**` left behind is two asterisks appearing from nowhere.

  Two things were checked rather than assumed, and both held: the caret already grows with the
  heading, and concealed markers were already absent from the laid-out text, so hit-testing needed no
  change.

  Copy is unchanged and carries the **source** bytes — `# Title`, not `Title`. Byte preservation
  points that way and the alternative is a second serializer by the back door. It is the decision
  here most likely to surprise somebody, which is why it is written down.

- **A concealed marker may be *substituted* rather than dropped, and that is the third rendering
  model.** MDEDIT-141. The other two are conceal-and-style — hide `**`, embolden what it wrapped —
  and colour-only, where every byte stays and the tint carries the meaning. Neither works for a list:
  hide `- ` and a bulleted list is pixel-identical to consecutive paragraphs, and an ordered one loses
  a number that no styling replaces. So `view::substitute` gives the concealed run text of its own —
  `• ` for `- ` — and `build_runs_in` pushes that instead of pushing nothing.

  **The rule for when this applies** is the one already written on `Marker::Table`, generalised:
  concealment is safe for scaffolding *whose removal still reads*. Where the marker is the meaning,
  substitute it.

  **Nesting costs nothing extra, and that is the whole reason this shape was chosen.** `mdedit-markers`
  counts a list marker's leading whitespace as part of the marker, so `␣␣- ` is concealed indent and
  all; carrying the whitespace into the substitute makes a deeper item sit further in by construction.
  The rough ticket had assumed a per-chunk indent instead, which cannot work — `blocks::split` makes a
  whole list *one* chunk with one origin, so an offset would move every item equally.

  **An ordered marker substitutes to its own bytes**, which is not a no-op: revealing it would draw
  the same pixels and put those bytes back in the caret's path, which MDEDIT-118 cleared. CommonMark
  keeps the author's start number, so there is nothing to renumber and nothing to invent.

  **Two things follow for the glyphs, and both are load-bearing** — see `StyledRun::substituted`,
  which is where a backend reads them. Every glyph of a substituted run reports the run's source
  **start**, collapsed: collapsed, so the caret cannot be dropped into the middle of bytes that are
  not there; the start rather than an empty span elsewhere, because a glyph's colour is recovered by
  asking which run's span contains it, and a span containing nothing answers nobody.

  **The flag is declared, and the first version of it was derived — from `text.len() !=
  source.len()`, which is wrong in both directions.** Chrome passes an empty span with real text,
  exactly as `StyledRun` told it to, so the derivation collapsed its glyphs and put the find bar's
  caret at the end of the box for every position but the start. And a substituted *ordered* marker
  draws its own bytes, so its lengths match and the derivation missed it — letting a click land at
  byte 1 of `7. `, inside a concealed marker. Neither is visible from the list side of the change,
  which is the argument for declaring a thing the caller knows and the callee cannot work out.

  **Not a hanging indent.** Substitution indents an item's *first* row; a wrapped row still begins at
  the layout's left edge, so a long item wraps back under its own bullet. That needs a concept
  `TextLayout::layout` does not have — it takes `(&[StyledRun], max_width)` and nothing else — so it
  crosses `mdedit-text` and both backends. MDEDIT-147, with block quotes and definition lists wanting
  the same thing.

  Cost, on a 135 KB document of 6000 list items: `build_runs_in` **0.98 ms before, 1.55 ms after** —
  medians of five pairs run alternately against a worktree of the previous commit, after higher in
  all five. That is weaker than `benches/options.rs`'s shape, which times both halves microseconds
  apart in one process and pairs a control against itself; two processes seconds apart cannot do
  either, so read the direction rather than the figure. The extra is ~0.1 µs an item and it is mostly
  the 6000 runs that did not exist before rather than the allocation — each one is a `decorate` call,
  and MDEDIT-37 found `block_at` to be the cost in there. A frame decorates visible chunks only, so a
  screen of items is a few microseconds. `list_cost.rs`.

- **A wrapped list item hangs under its own text, and the text trait did not have to grow to do it.**
  MDEDIT-147. Substitution gives an item's *first* row its bullet and its nesting indent; a
  continuation row still began at the layout's left edge, so a long item wrapped back underneath its
  own bullet.

  **The ticket was raised assuming this needed a hanging indent on `TextLayout::layout`**, which takes
  `(&[StyledRun], max_width)` and nothing else — and that premise is true and stops one step short, in
  exactly the way MDEDIT-126's did. `Layout` is not what a text engine hands back; it is what this
  editor assembles. A table already lays its cells out apart and *places* them, so a list does the
  same with simpler arithmetic. `mdedit-text` and both backends are untouched, and there is no shape
  for parley to satisfy later. **The decision this ticket exists to take is "don't".**

  A per-layout indent could not have worked anyway: `blocks::split` makes a whole list, nested items
  and all, **one chunk**, so a single value on the layout call cannot express items at different
  depths.

  **The unit is an item and its indent is its own marker.** `list::units` cuts at each `ListItem`
  marker; nesting needs no recursion, because a nested item's marker is another `ListItem` carrying
  more leading whitespace. Each unit is laid out twice — the substituted marker alone to measure the
  indent, then the rest at the width that leaves — and `list::assemble` stacks them. Every row of an
  item then has the same usable text width, which is what a hanging indent *is*. One call at the
  reduced width would be a row narrower on the first line, because the marker would be inside that
  width as well as beside it.

  **Not every `ListItem` marker opens an item**, which a test found rather than a reviewer: a
  continuation line's leading indent is a marker too, since a marker carries its whitespace. Cutting
  there hangs `␣␣more of alpha` as though it were an item, so a marker that is only whitespace is not
  a boundary.

  **`hit` needs nothing from this.** A table hands it row spans because a grid *drops* a source line;
  this drops none — wrapping only ever adds rows — so counting newlines stays correct. Checked rather
  than argued: `a_click_on_a_wrapped_row_lands_on_the_byte_under_it`.

  The row-count invariant is the one to keep: a list that does not wrap must assemble to **the same
  number of rows as the linear layout it replaces**, asserted against that layout rather than against
  a number written by hand. Each unit ends in a newline and the engine gives it a trailing empty line,
  so a naive stack is `2N` rows for `N` items.

- **Whitespace and line endings can be drawn, and drawing is *all* it does.** MDEDIT-46. Spaces are a
  middot, tabs an arrow, the rest of C0 its Unicode Control Picture, and each line ending its own mark
  — `␊`, `␍␊`, or a lone `␍` **in the warning colour**, because a bare CR is nearly always damage
  where the other two are ordinary. This is the answer to "Mixed" in the status bar, which until now
  said a document had mixed endings without saying which lines.

  **Every mark is stamped into space the layout already decided on.** Nothing re-shapes, so no glyph
  moves, no row re-wraps, and no click lands anywhere new.

  That used to read *"into the cell the character already has"*, and the difference is the whole of
  MDEDIT-124: **a character the layout never shaped has no cell.** On DirectWrite a tab is an advance
  to a stop rather than a glyph, and a C0 control is dropped outright — so iterating the glyphs
  marked neither, and the feature's headline mark was invisible in the case people actually hit. The
  one place a tab *was* marked is the walk past the last glyph, which reads characters out of the
  text; that is why a trailing tab worked and an interior one did not.

  So the characters come from the **text** and their positions from the **layout**: an unshaped one
  sits in the gap between the glyph before it and the glyph after, and consecutive unshaped
  characters share that gap between them rather than stacking their marks at its middle. A **control
  is still not marked**, and that is a decision rather than the same bug — it takes no advance at
  all, so its picture would land on top of the character beside it, and giving it room is a different
  feature from one that changes nothing else.
  That is why **the toggle clears no layout cache**: MDEDIT-46's plan expected one by analogy with
  zoom, and the analogy does not hold — zoom changes what the text engine is *asked for*, and this
  changes nothing it is asked for. The marks themselves are a sixth cache of laid-out text, so they
  *are* cleared by `set_zoom`.

  Per-run and not remembered, like the drawing mode and for the same reason: it is a way of looking
  at a document for a moment rather than a preference.

  **Three of its four defects were only visible in the window, and all three came from the same
  place** — the headless stub shaped text more simply than DirectWrite does, giving every character a
  glyph. **A test on the stub could not see any of them.** MDEDIT-110 measured what DirectWrite really
  does, MDEDIT-113 acted on it, and MDEDIT-124 was the third: it corrected the stub to shape neither
  a tab nor a control, which is what finally put the failing case in front of a test. Correcting the
  stub had to happen *in the same change* as the fix — on its own it turns the marking tests red with
  no fix available, which is why it had been left alone.

  **What DirectWrite actually does** (Windows 11 26200, installed Segoe UI, pinned in
  `crates/text-dwrite/tests/layout.rs`):

  | Character | Shaped? | |
  | --- | --- | --- |
  | `\n`, `\r`, `\r\n` | **no glyph** | each still ends a line |
  | a space, trailing or not | **shaped** | ordinary glyph, ordinary advance |
  | a tab | **no glyph** | advances to a 64 px stop instead, and they stack |
  | a C0 control, `DEL` | **no glyph** | *and no advance* — dropped outright |
  | U+000B | **no glyph** | DirectWrite ends a line on it |

  Two consequences, and they pull in opposite directions.

  **`ending_before` is gone.** It existed to answer "is the offset past the last glyph the terminator
  or past it". No backend gives a terminator a glyph, so the offset is always *of* it and
  `view::ending_at` reads forward from there. The stub used to shape a `\r`, which was the only thing
  keeping the question alive; it now agrees with the measurement, which is what the stub is for.
  Watch for this in review: the old `>`-shaped mark-count test passed with the CRLF read as a plain
  `Lf`, because the stray CR glyph made the count go up anyway. **Counts, not comparisons.**

  **The whitespace walk stays, and is now for tabs and controls rather than for spaces.** It walks
  from past the last glyph, marking whatever `mark_for` names and the layout declined to shape. Its
  condition is that pair of properties rather than a list of characters, because which characters a
  backend skips is the backend's business.

  It only reaches the ones that **trail** a row, though, and a tab or control in the middle of a line
  has no cell either — so on Windows those cannot be marked at all. **That hole is MDEDIT-124**, and
  the stub deliberately still shapes tabs and controls so the marking tests have cells to assert
  against. That divergence is the same shape as the one that caused this ticket, and is written on
  MDEDIT-124 so it is a known cost rather than a repeat.
- **There are two palettes, and one of them is unreachable.** MDEDIT-71. `Theme::dark` is what the
  editor has always drawn with — **unchanged to the byte**, which was that story's whole acceptance
  criterion — and `Theme::light` is the MDEDIT-69 design pack's Nocturne, complete and
  contrast-checked and read by nothing until MDEDIT-78 adds the OS read and the config key. `Default`
  is dark, so no caller changed and no test was touched.

  **`Appearance`, not `Mode`.** `Mode` already means how the *document* is drawn — preview against
  plain markup — and the two are independent: plain markup is still dark.

  **Seven roles exist that nothing reads**: `span_error`, `warn`, `link`, `muted`, `gutter`,
  `caret_line`, `line`, `panel`. Each names the story that will consume it. They are here rather than
  added one at a time because every story after this would otherwise re-open the struct, and because
  the light values have to be chosen against one another to be checked against one another.

  **`span_error` is rose and `status_error` stays amber**, deliberately two roles where the pack had
  one. Red in a status bar reads as "the editor broke"; a squiggle under the offending bytes is
  exactly the case a warning colour is for. Both arguments are right about different things.

  **The contrast test names its pairs, and that is the point of it.** `crates/app/tests/contrast.rs`
  computes WCAG ratios from the palette rather than checking in a table of numbers, and it asserts
  only the foreground-on-background combinations the renderer actually produces — traced from the
  draw sites. Asserting everything against everything measures fictions: `token_comment` on
  `tab_active` is 4.13:1 and irrelevant, because a JSON comment is never drawn on a tab. The pack
  itself measures against the page alone and so misses every pair that is on chrome.

  It carried **one known shortfall** — the close `×` on the *active* tab, `marker` on `tab_active` at
  4.31:1 — which MDEDIT-72 cleared at 5.25:1, so the list is gone rather than shorter.
- **The dark palette is Nocturne, and that is the visible one.** MDEDIT-72. Four hue families over
  one neutral ramp: lavender for structure, sage for literals, sand for numerics, steel for
  constants, rose for errors alone. The editor stops looking like VS Code's default.

  **Six roles are not in the pack and are derived from its ramp**, each for a stated reason.
  `find_match` is sand and translucent, so that blended over the page it is within 1.2× of the
  selection's *brightness* — the two are told apart by hue, which survives peripheral vision and
  colour blindness, and not by weight, which is the failure mode. `strip`, `tab_active` and
  `tab_inactive` are three surfaces from the pack's two: the strip is `chrome`, the active tab is
  `panel` because it is the raised one, and the inactive tab is a ramp step between the page and the
  panel. `tab_modified` is the accent. `token_comment` is a dimmed sage, because the pack sends
  comments to `muted` and the editor already makes that distinction.

  **`gutter` is the one value deliberately not the pack's.** The pack measures every role against the
  page alone; against `panel` its `#82869a` is 4.22:1, under its own floor, on exactly the surface a
  file pane is. `#8b8fa4` clears it at 4.75:1 there and 5.50:1 on the page, and
  `the_gutter_clears_the_floor_on_a_panel_as_well_as_on_the_page` is what keeps that true.

  **The file pane draws on `panel` now**, not on the tab strip's colour — which it only ever borrowed
  because the strip's was the one dark chrome value there was.

  **The tints are real alpha and the pipeline is `ALPHA_BLENDING`,** so `contrast.rs` blends before it
  measures: what a reader compares is the tint over the page, not the entry in the palette. Text on
  the find highlight, and on the current match where the selection sits over it, are both asserted.

  **The status bar's count stays amber and a diagnostic span stays rose**, asserted rather than
  trusted, because collapsing them into the pack's single `error` role is the easy mistake here.

  Verified by eye on Windows against Markdown, JSON and YAML — the four families read distinctly, the
  current match is lavender against the other matches' sand, and the count is still amber. The
  scrollbar thumb is 2.01:1 against the page where the old palette was 2.07:1; it is not text.
- **Quoted prose and `true`/`false`/`null` are italic, which is half the contract's second cues.**
  MDEDIT-74. The pack's rule is "never colour alone": a role's colour always has a second signal, so
  a reader who cannot separate two greys or two blues still sees the distinction.

  **A quote is asked of the block list, not tracked in `DecorState`.** That state resets at every
  newline — inline delimiters pair within a line — and a quote is the opposite shape, spanning lines
  by definition, so a toggle would be cleared by the first one. `Document::in_block_quote` answers
  "is anything enclosing this byte a quote", which `block_at` cannot: it returns the *innermost*
  block, and a quoted paragraph is a paragraph. Same binary search and same bounded walk, because the
  renderer asks it once per line and a scan of the block list is MDEDIT-37 again.

  Markers stay upright — a slanted `>` reads as prose rather than syntax — and so does code inside a
  quote, because italic monospace is where alignment goes to die.

  **`Token::Literal` is the first token to reach `TextStyle` rather than only a colour.** A quoted
  `"true"` is `Token::String` and stays upright: the classification already knows the difference, and
  italicising on the text would be ignoring it. There is a test that says so.
- **The light palette is reachable, and the editor follows Windows by default.** MDEDIT-78. A `theme`
  config key — `"system" | "dark" | "light"`, never a path — three View → Appearance items that tick,
  and `Theme::for_appearance` doing the rest. Half the design pack was unused until this.

  **The setting is read from the registry, and that is not a style preference.** `AGENTS.md` and
  `docs/research/05-windows-platform.md` both say `ShouldAppsUseDarkMode` is unreliable from 1903, so
  `mdedit-menu-win::apps_use_light_theme` reads
  `HKCU\…\Themes\Personalize\AppsUseLightTheme` with one `RegGetValueW`. It lives in the menu crate
  because that is where the platform `unsafe` for appearance already is — **no fourth unsafe crate**.

  **winit's `ThemeChanged` is not a reliable notification, and this is the measurement.** winit raises
  it from `WM_SETTINGCHANGE`, but only when *its own* answer changed — and its answer comes from
  `ShouldAppsUseDarkMode`, the API this project does not trust. Flipping `AppsUseLightTheme` under a
  running editor on Windows 11 produced **no event at all**: the palette did not move for five
  seconds, while a direct read of the registry returned the new value immediately. So the event is
  wired up and welcome, and two event-driven checks stand behind it: **on focus**, beside
  `check_disk`, because coming back to the window is when somebody who just changed the setting is
  looking at it; and **on the idle beat**, which is one registry read of a few microseconds against a
  whole-document re-parse. Neither is a timer — `on_idle` runs when the loop is about to wait after
  handling input, so an idle editor asks nothing. Verified by eye: flip the setting, press a key, the
  document is light.

  **Nothing is cleared on a switch, and that is asserted rather than reasoned.** MDEDIT-78's ticket
  asked for exactly that: `theme_switch.rs` swaps palettes and checks that no quad still carries a
  colour from the old one, and that the frame after a switch lays out no more text than an ordinary
  frame. It holds because colour is in neither `TextStyle` nor `ChunkKey` — every cache holds
  laid-out glyphs, and a glyph is the same shape in both palettes.

  **The zoom is carried across by ratio**, taken from the theme being replaced rather than from the
  config or the settings: those two disagree for as long as it takes `with_config` to run, and a
  palette switch that quietly reset the text size would be a strange bug to hunt.

  **The menus followed on MDEDIT-79**, which closed the epic: the mode `mdedit-menu-win` forces is a
  parameter now, taken from the editor's resolved palette rather than from Windows'.
- **The menus are drawn in the editor's palette, not Windows'.** MDEDIT-79, which closes MDEDIT-69.
  `dark.rs` forced `PreferredAppMode::ForceDark` unconditionally and the bar attached as
  `MenuTheme::Dark`; both are parameters now, and `MenuBar::set_appearance` re-themes an *attached*
  bar so a switch needs no restart — `FlushMenuThemes` is the call that makes that true, and a menu
  bar that only picked the change up on the next launch would have been a bug rather than a limit.

  **Still not `MenuTheme::Auto`, and that is the whole point.** Auto follows Windows; the editor's
  palette may be pinned against Windows by `theme`. On a light-themed machine with `"theme": "dark"`,
  Auto would put a light bar over a dark document — MDEDIT-32 reintroduced in exactly the case the
  config key exists to serve.

  **Verified in light, not in dark.** A pinned-light editor draws a light bar and light dropdowns.
  The dark dropdown could not be confirmed on the machine this was built on (Windows 11 26200) — the
  popup photographed light with `attach` provably receiving `Dark`, which points at the uxtheme
  ordinals rather than at this wiring, since forcing dark popups is the undocumented best-effort
  trick `dark.rs` has always been. **If dark dropdowns are broken there, they were broken before this
  story**: nothing here changed how the call is made, only what it is passed. Worth an eye and a
  ticket rather than a claim in either direction.
- **The gutter has its own colour, and the caret's line is lit.** MDEDIT-77. Line numbers draw in
  `gutter` rather than borrowing `marker` — which was a good call while there was one dim colour and
  stopped being one when there were several — and the caret's own number steps up to `text`, which is
  the pack's rule for the role. The row itself takes the `caret_line` tint, a 7% lavender.

  **Two cues for one question.** The tint is deliberately near invisible, which is exactly why the
  brighter number exists: a reader who cannot see a 7% wash still sees which number is bright. The
  gutter is off by default, so on a default install the tint is the only cue — that is a real gap and
  it is written down rather than fixed by turning a setting on for people.

  **Three decisions the ticket asked to be made explicitly**, all in `draw_caret_line`'s docs: every
  row of a **wrapped** line is lit, because a line is a source line and that is what every editor
  does; it stays lit while the **find field** has focus, because the document's caret has not moved
  and Esc puts you back there; and it stays lit **under a selection**, so two tints stack.

  **`contrast.rs` asserts the stack**, which the pack does not model: it measures every role against
  `bg` alone and claims "selection tints preserve 4.5:1" — a claim about one tint, not two. Text on
  the caret line, and text on a selection over the caret line, are both asserted, as is the caret's
  number reading brighter than an ordinary one.

  **In the gutter, rose outranks bright outranks dim**: a line that is both flagged and current stays
  rose, because something wrong outranks where you are.

  The tint is drawn **first** on its row, so the selection, the search highlights and the glyphs all
  land on top. It spans the document's width and its margins but **not** the gutter, which has its own
  answer. It is placed in window coordinates rather than panned with the text: a highlight that slid
  sideways with a horizontal pan would leave the rest of its row untinted.
- **A diagnostic is drawn on the document now, not only counted.** MDEDIT-76. A rose squiggle under
  the offending span, and the line's gutter number in rose and bold with a `!` in front of it — the
  design pack's `error` role is "paired with a gutter marker", and adopting the colour without
  drawing one would have been claiming a cue that did not exist.

  **The squiggle is a row of quads**, because there is one pipeline and it draws rectangles: short
  dashes alternating between two heights, which reads as a wave at body size and needs no shader.
  Its geometry comes from `row_selection`, the same helper the selection and the find highlights use,
  so a span crossing rows is clipped per row by code that already knows how.

  **It follows `diagnostics()`, so an edit takes it away and the idle timer brings it back**, with no
  hook on any edit path — the same `(tab, revision)` staleness rule the count has, and for the
  stronger reason: a squiggle under bytes that have moved points at innocent text. **Nothing on the
  drawing path re-runs the check.** Measured at 412 KB of JSON: typing p50 **31.7 ms before, 29.6 and
  32.1 ms after** across two runs — the change is inside the run-to-run spread. `validate_cost.rs`.

  **The check's 200-diagnostic cap is what makes the per-chunk filter free**: the whole list is
  shorter than one screen of find matches, so it is scanned rather than binary-searched.

  **The squiggle draws *after* the row's glyphs.** Everything else under a row is a ground the text
  sits on; this is a mark *about* the text, and a descender must not be able to hide it.

  **A gutter number sits on its line's baseline, and did not before.** Found by putting a `!` there
  and looking: the digits were placed from the row's *top* and then offset by the glyph's own
  distance from the baseline, which is negative — so every number was drawn about an ascent above the
  line it belonged to. `gutter.rs` asserts where numbers sit *horizontally* (MDEDIT-34) and the
  headless stub gives every glyph a one-pixel offset, so the error was one pixel in a test and most
  of a row in the window. Now asserted vertically too.

  **With line numbers off there is no gutter, so the squiggle is the only cue.** Reserving gutter
  space when an error appears would reflow the document at the moment something went wrong, which is
  worse; the honest answer is that the second cue depends on a gutter being shown.
- **Markdown's syntax is coloured per kind, and `Role` carries the marker to do it.** MDEDIT-75. All
  thirteen `Marker` variants used to collapse to one colour, so a heading's `#`, a bullet, a fence and
  a link's `](` were the same green. Two kinds leave the `marker` colour, and both are landmarks
  rather than scaffolding: a **list bullet** takes `key`, and a **link destination** takes `link`,
  underlined — MDEDIT-73's first consumer.

  **A destination is not a `Marker` variant and does not become one.** `pulldown-cmark` reports a link
  as one construct and subtracts its *text*, so `[a](url)` arrives as the markers `[` and `](url)`,
  with the destination inside the closing marker's own segment. `view::push_line` splits that segment
  by a character class (`brackets_a_link`) rather than re-deriving where the URL starts: the
  alternative is a second link parser in the renderer, handling angle brackets, titles and reference
  labels. A reference label and a title take the destination's colour under that rule — the right
  answer for a label, and a knowingly loose one for a title. **Nothing about the locator or its tiling
  property test changes**: this is a renderer-side split of bytes the locator already classified.

  **A link's text is `Content`, so it is told from prose by where the walk is.** `DecorState` grew
  `link_text`, set from the marker's own bytes — `[` and `![` open one, everything else closes it —
  rather than by counting segments, which depends on the form the link is written in. Unlike
  MDEDIT-74's quote it *is* per-line state, because a link is inline and pairs within a line like the
  emphasis beside it.

  **Code content takes `string` in both forms.** A code span's content is `Content` too, and the
  `code_span` flag already tracked for the monospacing carries the colour with it; a fence's content
  comes from the block list.

  **`style_at` became `decorate`, returning the style and the colour together**, because both now need
  the same `block_at` answer and asking twice would double the per-run cost MDEDIT-37 went to find.

  **Concealment did not change**, which is the acceptance criterion that mattered: a marker still
  vanishes off the caret's line, and a test asserts the drawn text of a document full of syntax. Plain
  markup still shows every byte — and keeps the destination's underline, because "never colour alone"
  is not a preview-only contract.
- **The drawing mode is global and deliberately not remembered.** Toggling to plain markup switches
  every tab, and the editor opens previewed every time. Unlike the chrome choice, the zoom and the
  line numbers — all three of which persist since MDEDIT-60 — this one is left per-run on purpose:
  it is a way of looking at a document for a moment rather than a preference, and an editor that
  opened in plain markup because you glanced at the source last week would be wrong.
  `plain_markup_does_not_persist` pins it, so changing it is a decision rather than an accident.
