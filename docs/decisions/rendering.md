# Rendering — palette, concealment and what is drawn on the text

The two palettes and the roles in them, how Markdown syntax is concealed and coloured, and the marks the editor puts on or under a line.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **Whitespace and line endings can be drawn, and drawing is *all* it does.** MDEDIT-46. Spaces are a
  middot, tabs an arrow, the rest of C0 its Unicode Control Picture, and each line ending its own mark
  — `␊`, `␍␊`, or a lone `␍` **in the warning colour**, because a bare CR is nearly always damage
  where the other two are ordinary. This is the answer to "Mixed" in the status bar, which until now
  said a document had mixed endings without saying which lines.

  **Every mark is stamped into the cell the character already has, at the advance the layout already
  gave it.** Nothing re-shapes, so no glyph moves, no row re-wraps, and no click lands anywhere new.
  That is why **the toggle clears no layout cache**: MDEDIT-46's plan expected one by analogy with
  zoom, and the analogy does not hold — zoom changes what the text engine is *asked for*, and this
  changes nothing it is asked for. The marks themselves are a sixth cache of laid-out text, so they
  *are* cleared by `set_zoom`.

  Per-run and not remembered, like the drawing mode and for the same reason: it is a way of looking
  at a document for a moment rather than a preference.

  **Two of its three defects were only visible in the window, and both came from the same place** —
  the headless stub shapes text more simply than DirectWrite does. `view::ending_before` exists
  because the offset past a row's last glyph may be the terminator or past it, and the whitespace
  marks walk trailing spaces out of the *text* rather than off the glyphs. **A test on the stub
  cannot see either.**

  **The DirectWrite behaviour those two were written for is not what DirectWrite does**, which
  MDEDIT-110 measured and MDEDIT-113 carries. This entry used to say a `\n` gets a glyph of its own
  and trailing whitespace is not shaped; on Windows 11 26200 against the installed Segoe UI, both are
  the other way round — `"ab\ncd"` produces glyphs for bytes 0, 1, 3 and 4, and `"ab   "` produces
  one for every trailing space. `crates/text-dwrite/tests/layout.rs` pins that as a characterisation
  test, so a change is noticed rather than assumed; whether `ending_before` and the whitespace walk
  should be simplified to match is MDEDIT-113, and wants an eye on Windows rather than a test.
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
