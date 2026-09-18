# Chrome — the bar, the panels, the strip and the status line

Everything the editor draws that is not the document: the formatting bar and its lists, the tooltip, the find bar, the tab strip, the file pane, the gutter and the status bar.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **All chrome draws after the document.** There is no depth buffer and no scissor, so the painter's
  order is the whole of the answer: the formatting bar, tab strip, file pane and status bar are
  appended after `draw_chunks`. The row cull deliberately keeps up to a line-height of overshoot above the viewport
  — it is what makes a partly-scrolled row look cut off rather than popping in — so a piece of chrome
  drawn first is painted over by it. MDEDIT-39 was exactly that, and `frame.rs` guards it.

  The document is the bottom layer for clicks as well. Since MDEDIT-142 it has something pressable
  in it, a task's box, and that is asked only after every panel in `Editor::click` has declined the
  point. So a panel's rules still hold without change: nothing drawn over a box can pass a click
  through to it. See the MDEDIT-142 entry in [rendering](rendering.md).

- **A file-pane row shows what kind of file it is, in three shapes rather than seven.** MDEDIT-174.
  A pane of names told you nothing until you read the extension at the end of a name the pane may
  have truncated. The icon is asked of `detect::language_for` — the same answer the highlighting
  uses — so `.yml` and `.yaml` cannot drift apart, and a new language gets an icon by being added to
  `detect` and nowhere else.

  **The font decided the grouping, and only rasterising found that out.** MDEDIT-92's rule is that a
  private-use code point is a promise about a font rather than a standard, and this is the second
  time it paid: `U+E8A5` "Document", `U+E7C3` "Page" and `U+E130` "MusicInfo" rasterise to *the same
  glyph* in Segoe Fluent Icons — identical bitmaps, not merely similar — and there is no JSON glyph
  or YAML one at all. The usable silhouettes are a page, a page with lines, code brackets and three
  rules. So the seven languages are grouped by what they are for: Markdown a document, the five
  structured formats code brackets, plain text lined paper. Letter badges for all seven were the
  alternative and cost twice the width in a 200 px pane, to be *read* rather than glanced at.
  `tests/icons.rs` asserts the three exist, have ink, and rasterise to three different shapes.

  **The column comes off the name**, which is the whole of its cost, so `RowRect::label_x` and
  `label_width` move by it and MDEDIT-121's truncation is measured against what is left. The hit test
  needed nothing: everything left of the close region is already the row's body, so a click on the
  icon selects the row.

  **Whether the glyph resolves is asked once, not per row per frame.** A `.notdef` check is a
  text-engine call, and chrome is redrawn every frame — the trap the gutter, the status bar and the
  pane itself have each hit. The answer cannot change with the size it is drawn at, so it is cached
  and never invalidated; a machine without either Segoe icon font draws MD, {} or TXT instead.

- **A tab draws the same icon as its pane row, and the icon goes on top of the tab.** MDEDIT-178.
  Nothing about *which* icon was redecided: the strip asks the same `icon_for` and draws through the
  same `row_icon_face`, so the two presentations cannot disagree about a file. `Icon` moved out of
  `pane.rs` into `file_icon.rs` because it stopped being the pane's, and `tabs` depending on `pane`
  for it would have been the wrong way round.

  **Where the column comes from is the one thing that differs.** The pane is a fixed 200 px, so its
  icon had to come off the name. A tab is sized from its name, so `tabs::layout` adds `ICON_WIDTH`
  *after* the clamp: the 70–200 px bounds still describe the name's room, every name keeps exactly
  what it had, and the eighteen pixels are paid in how many tabs fit instead. That trade is the right
  way round because a tab that does not fit is simply not laid out, whereas a name cut short sooner
  sends the user to MDEDIT-121's tooltip more often.

- **The pointer says what is under it, and the shape follows the click.** MDEDIT-176. Nothing in the
  workspace called `Window::set_cursor`, so the arrow was drawn everywhere — including over the text,
  where an editor is expected to show an I-beam, and over MDEDIT-142's box, which is the one thing in
  a document you press. `Editor::pointer_at` answers a point with `pointer::Pointer` — this crate's
  own three shapes, not winit's `CursorIcon` — so a headless test asks it through the grid stub
  exactly as it asks a click, and the mapping to winit is the one line that talks to the window.

  **It asks the same predicates as `Editor::click`, in the same order**, because a panel that takes
  the click takes the shape. What it does *not* do is repeat every arm: the formatting bar, the tab
  strip, the file pane, the status bar, the gutter and — less obviously — the scrollbar all sit
  **outside `ContentRect`**, the bar because it overlays the window's right margin rather than the
  text. Every point outside the content is already the arrow, so an arm apiece would be branches no
  test could tell from their absence. Both facts were found by mutating the arms one at a time: four
  survived, which is what said they were redundant rather than untested. What is left is the dialog
  and the open list, drawn over the text, and the find bar, outside the content and still wanting an
  I-beam on its fields.

  **A gesture outranks the pixel underneath.** Dragging a selection keeps the I-beam wherever the
  pointer goes and a held scrollbar thumb keeps the arrow, because mid-drag what the pointer is doing
  has stopped being a question about what it is over.

  **Set on a move and on a frame.** A scroll, a panel opening, a tab switch or `Ctrl+/` all change
  what is under a pointer that has not moved, and `CursorMoved` does not fire for any of them. The
  shape is compared before it is set, so crossing a region costs a comparison per event rather than a
  call into the window per event.

  Not done: a hand over a link, until something follows one — a hand promising an action that does
  not happen is worse than the arrow. Nor a resize shape on the file pane's edge, which is a fixed
  width, nor a busy pointer, since nothing blocks the UI thread long enough to need one (MDEDIT-151).
- **There is a File, Edit, View and Help menu, a status bar, a scrollbar, a right-click context menu,
  a formatting bar, and the open files shown either as a tab strip or as a left pane.** Every view
  toggle now has a menu item as well as a chord.
- **The formatting bar is a second door, never a second implementation.** MDEDIT-52: every button
  calls the editor method its chord calls, through one `Editor::perform` match, so there is no
  behaviour that exists only when clicked. It is per-run — View → Formatting Bar, no chord, and
  nothing written back to the config. **Its buttons are icons from a font**, drawn by the same engine
  as everything else and laid out once into `toolbar_labels`, which is therefore a fifth cache
  `set_zoom` has to clear. No accessible names (those wait for the accesskit pass; a tooltip is not
  one). Lighting is lookups only: `is_inside` asks
  the segments, `carries` and `is_quoted` read the caret's line out of the rope, and headings ask the
  block list so a setext one still lights. Rule and Link never light, because neither is a state.

  **Every document gets a standard set, and that is what stopped the bar disappearing.** MDEDIT-84.
  `toolbar::STANDARD` is New, Open, Save, Undo, Redo, Cut, Copy and Paste — eight buttons a language
  appends its own after, so JSON and YAML have a bar where they used to have none. It used to be
  Markdown's alone, on the rule that a button which does nothing is worse than no button; that rule
  is intact and the *bar* is no longer the thing it applies to. Close is deliberately not in the set:
  a one-click Close beside Save is a misclick that shuts a document, and it is on the menu, on Ctrl+W
  and on each tab's `×`.

  **The standard set is leftmost, so overflow cuts the language's buttons first.** The standard
  actions are what somebody who does not know the chords is looking for, and every formatting button
  has a chord *and* a menu item. Words rather than symbols, because there is no glyph for Open that
  reads without a legend — MDEDIT-92 makes both sets icons, after MDEDIT-85's tooltips.

  **A button that cannot act right now is dimmed and ignores its click**, in `theme.gutter` over the
  bar's `strip`, asserted legible by `contrast.rs`. Save with nothing to save, Undo with no history,
  Cut and Copy with no selection. **Paste is never dimmed, against the ticket's plan**: deciding needs
  the system clipboard, and reading it per frame is what `crate::clipboard` says not to do while
  caching it means a stale dim costs the user a paste — where a live Paste that inserts nothing is
  exactly what Ctrl+V already is. The dimmed check is in `click_toolbar` rather than in `perform`,
  because `perform` is what a chord reaches too and Ctrl+S on an unmodified document is allowed.
  **Geometry does not change with state** — a dimmed button keeps its rect, or the row would reflow
  as the user selects and deselects.

  **It is the topmost chrome, above the tab strip.** MDEDIT-84's second item, and it is four y
  offsets and two hit tests in `editor.rs` — `tabs.rs` and `toolbar.rs` are untouched, because each
  lays out within a row whose origin the caller supplies. The document's top edge does not move:
  `Insets::top` is a sum and a sum does not care about order. The find bar, status bar and file pane
  keep their places.

  **The strip's conversion is `strip_hit`, and it exists because there are two callers.** A left
  click and a middle-click-to-close both turn a window position into a tab, and the first version of
  this moved only the left one — the same mistake `pane_hit` was written to prevent, made again one
  function away from it. **Both bands are thirty pixels of `strip`**, so a test cannot tell the two
  backgrounds apart; `the_bar_is_drawn_above_the_strip` reads what is unique to each instead — an
  inactive tab's quad, and a dimmed label's `gutter`. Without that, moving the drawing and not the
  clicks (or the reverse) passes every other test in the file, and so does leaving the tab *labels*
  at their old baseline while their backgrounds move.
- **Link and Image ask for their parts, and that reverses "no dialog at all".** MDEDIT-87 added the
  Image button — the only element of the Markdown Guide's basic syntax with no way in but typing —
  and MDEDIT-129 made both of them ask before they write.

  Two decisions were reversed, both deliberate and both written down. `make_link` called the
  clipboard case *"the case that needs no dialog at all"*; `insert_image` called caret-parking *"the
  whole of why there is no dialog"*. What they missed is that **a link has two parts and an image has
  three**: a skeleton to fill in only helps when you already know the shape, and it offers no way to
  see what is about to be written and no way out except undo.

  **The fast path survives as one keystroke.** The clipboard's URL and the selection both pre-fill,
  so copy-select-Ctrl+K becomes copy-select-Ctrl+K-Enter. The caret starts in the first *empty*
  field, so with both filled it sits on the URL — the one you would correct if the guess was wrong.
  Rejected: a dialog only when the clipboard has no URL (one key, two behaviours, invisible state),
  and button-opens-dialog-while-the-chord-stays-fast, which breaks the bar's own rule that it is *a
  second door, never a second implementation*.

  **The dim is the modality**, not decoration. This editor's only other keyboard-taking surface is
  the find bar, which announces itself by taking a strip of the window; a panel floating over
  unchanged text announces nothing. `dialog_key` is checked above F3 and above the find bar, and a
  chord that is not the field's own is *dropped* rather than performed — Ctrl+S with a dialog up
  should not save.

  **A click inside the panel is swallowed even where there is no control**, which is the other half
  of modal: letting one through would move the caret out from under the text about to be replaced. A
  click outside closes it, as every other panel here does.

  **The escaping is the part that is easy to get wrong.** A space ends a destination as far as the
  parser is concerned, so `](my chart.png)` is a *broken* link rather than a link to a file with a
  space in it — and a path with a space is the commonest thing anybody will paste in. The destination
  takes angle brackets when it needs them, a title's quotes are escaped, and the test **round-trips
  the result through the parser** rather than only comparing the string.

  An image gets a title field and a link does not: a link's title is a browser tooltip and is almost
  never written, and a third field nobody fills in on the dialog reached most often is worse than the
  inconsistency.
- **Resting on a toolbar button says what it does, and the panel is the least of it.** MDEDIT-85.
  `toolbar.rs` said there were no tooltips "because there is no hover affordance in this editor to
  hang one on", and that sentence was the ticket. The editor had always tracked the pointer and used
  it for dragging; nothing anywhere was *hover* state.

  **Two deadlines, one `ControlFlow`, and that is the whole risk.** The loop sleeps on `Wait`, so
  "the pointer stopped moving 400 ms ago" is not an event — it needs a `WaitUntil`, and the reconcile
  timer already owned the only one there is. A dwell set while a reconcile is pending would push the
  reconcile out; a reconcile set while a dwell is pending would swallow the tooltip. So both are
  fields and `Editor::next_wake` takes the earliest, `reschedule` is the only place `ControlFlow` is
  set after start-up, and the keystroke handler records `reconcile_at` rather than setting the loop
  directly. It has its own test because the failure is silent and intermittent — the same shape as
  MDEDIT-49's `follow_caret` bug, where two features quietly fought over one field.

  **And `reschedule` is called at the end of `window_event`, unconditionally, which is the line that
  was missing when this first went to the window.** Turning the keystroke's `set_control_flow` into a
  field left nothing to convert `next_wake` into a `ControlFlow`: `new_events` runs only on
  `ResumeTimeReached`, so the loop slept on `Wait` for ever. No tooltip ever appeared — and the idle
  beat was dead with it, so the whole-document re-parse, the word count, the recovery snapshots and
  the theme check all went quiet. **No test could see it.** The handler needs an `ActiveEventLoop`,
  which cannot be constructed, and the tooltip tests drive `on_dwell` directly, which is precisely
  what let them pass over a feature that never fired. Found by opening the editor and resting on a
  button. It is outside the match rather than in the two arms that arm a deadline, so a third cannot
  forget it.

  **The dwell restarts per button, not per pixel.** Moving inside the button you are already on
  leaves the deadline alone, or nobody without a very steady hand would ever see a tooltip.

  **The panel is never hit-tested**, and there is deliberately no `click_tooltip`: it hangs over the
  document and a click on it has to reach the text. Asserted by comparing the same click with and
  without a panel, which is a claim that cannot go vacuous when the geometry moves. Drawn genuinely
  last, after even the floating bar. `theme.line` gets its first consumer since MDEDIT-71 as its
  edge; `theme.panel` gets its second.

  **Its words are in `toolbar.rs`, not looked up from `shortcuts.rs`, and the drift is caught by a
  test instead.** The plan asked for the lookup; the reference document reads like a reference
  ("Heading level — press the same level again to clear it") where a button label has to read like a
  label. What actually matters is not advertising a chord that does not exist — MDEDIT-65 exactly —
  and `tests/tooltip.rs` asserts every chord `Action::chord` names appears in `shortcuts::ALL`.

  **A tooltip is not an accessible name.** Mouse-only, and a screen reader will not read it. The bar
  still owes accesskit its names and its keyboard reach, and this does not pay that debt.

  Dismissed by a click, a keystroke, a wheel notch, the pointer leaving the window, and switching
  tab — the last through the click that switched, rather than a hook on every path that activates
  one. Two keyboard sites, not one: `handle_key` for the keys that do not insert, and `insert_text`
  for every character, which is also the funnel an IME commit and a paste come through.
- **The bar is icons, and they come from a font rather than an asset pipeline.** MDEDIT-92.
  `FontFamily::Icon` is the third role at the text boundary; `text-dwrite` resolves it **once, at
  construction**, by asking the system collection for Segoe Fluent Icons and then Segoe MDL2 Assets.
  The probe is not optional: naming a family DirectWrite has not got does not fail, it *substitutes*,
  so a Windows 10 machine would have drawn tofu instead of falling back to MDL2. With neither it
  resolves to Segoe UI, where every private-use code point is `.notdef` — which is the signal, not an
  accident.

  **The fallback is part of the feature.** `Editor::button_face` lays the icon out, looks at the
  glyph, and draws the button's *word* instead if it came back `.notdef` — caching that under the
  same key, so there is one entry per action either way. A machine with no icon font gets the bar it
  had before this story; a row of empty boxes would be a bug report. It costs one extra layout per
  button on the first frame of such a machine, and nothing after that.

  **A private-use code point is a promise about a font, not a standard**, so every one of them is
  asserted against the installed font in `crates/app/tests/icons.rs` — the only test file in that
  crate that does not use the stub, because a stub answers yes to both questions by construction.
  Two were chosen by rasterising candidates and *looking*: `Fence` is `U+F0E3` and `Rule` is
  `U+E738`. The plan had both as "weak — check".

  **The three heading buttons stay lettered**, and that is the ticket's one accepted exception:
  neither font has a heading glyph because the convention does not exist — Word labels its heading
  styles with letterforms too.

  **This is also MDEDIT-86**, which the build order sanctions folding in. A 22-pixel icon does not
  fit a 30-pixel row, so the bar is 48 pixels. **And the padding is six rather than the mock-up's
  eight**, because at eight the row came to 930 logical pixels against a 900-pixel default window and
  `toolbar::layout` silently dropped Fence, Rule and Link on a fresh install. Every stub test passed
  through that, which is why `all_of_the_buttons_fit_a_default_window` measures real advances.
  MDEDIT-87's Image and Table buttons would have needed this conversation again — MDEDIT-116 is
  where it was had.
- **The bar never silently loses a button, and the headings are one button with six levels.**
  MDEDIT-116, which overturns MDEDIT-92's "no overflow menu" out-of-scope line on purpose. A window
  too narrow for everything ends with a `…` and the displaced buttons are behind it;
  `toolbar::overflowing` reserves room for that button **only when it is needed**, so a wide window
  is not one control short for the sake of an overflow that never appears.

  **What the bar drops and what the list offers come from one call**, because they are two halves of
  one partition and deriving them separately is how an action ends up in neither.
  `every_button_is_either_drawn_or_in_the_overflow` sweeps the widths and rejoins them.

  **Three heading buttons became one, and that is what paid for MDEDIT-87.** 844px → 776, and Image
  and Table land at 861 against a 900px window — measured in `icons.rs` rather than hoped for, so if
  it stops being true that ticket hears about it first. (861, not the 858 estimated here in advance
  and not the 861-without-Table that MDEDIT-127's plan quoted: the bar is 827px with twenty-one
  buttons, Table's own is 34, and 39px of headroom is left. That is room for one more button and no
  further.) H4 to H6 stop being chord-only as a result.
  The button reads `H1` by default and the caret's own level when it is in one; `Action::letters`
  separates a letterform that is one **on purpose** (set near the icon size) from the words a machine
  with no icon font falls back to (set small, because "Paste" at that size would not fit).

  **A list is a third z-order** — drawn after even the tooltip, hit-tested before the floating bar —
  and `Editor::click`'s order is the whole of it in both directions. **That is the bug this shipped
  with once:** `click` dismissed the tooltip first, and dismissal closed the menu with it, so every
  click on a row found nothing open and fell through to the document. `dismiss_tooltip` and
  `dismiss_panels` are separate functions now precisely so the click path cannot reach the wrong one.

  **A click anywhere while a list is open belongs to the list** — a row acts, anything else closes and
  is swallowed, including a tab. A click that both dismissed and landed is how closing a menu moves
  your caret.

  Two things found by their own tests rather than by eye: `popup::hit_test` gave the panel's top
  margin to row zero, because the subtraction went negative and the cast to `usize` saturated; and
  the icon column has to clear `PADDING` plus the icon or the words start on top of the picture.
- **The Table button opens a grid, and a grid is the one panel here that is not a list.** MDEDIT-127.
  MDEDIT-87 shipped that button writing a fixed 2×2 skeleton and it was rejected on sight — a button
  that always writes the same shape is barely better than typing the pipes — so the button came back
  pointing at a rows × columns picker, hover to size and click to insert, as Word and Google Docs do.

  Everything above lays out one dimension: `popup::hit_test` divides by `ROW_HEIGHT` and is done.
  `popup::grid` is a second geometry beside it rather than a parameter on it, and `toolbar::Menu`
  gained a third variant so the two places that differ — the click and the hover — say which they
  are by matching rather than by a panel that is secretly two things.

  **Every pixel of the grid belongs to a cell.** The gap between two squares is drawn, not
  hit-tested: a cell owns its whole pitch. The row version's one bug was at exactly this seam, and
  dead pixels between eighty cells would be the same defect with a hundred more places to hide —
  `every_part_of_every_cell_answers_with_that_cell` sweeps each cell's pitch rather than sampling its
  middle.

- **The same button opens a different menu inside a table**, and that is what kept a group of nine
  controls off a bar with room for one. MDEDIT-128. GFM has no nested table, so the size picker has
  nothing to offer while the caret is already in one — which leaves the button free to mean *what to
  do to this table* there, as a list of nine rows.

  The plan had this group either **replacing the formatting buttons** while the caret was in a table
  or **going to the overflow**, and had measured the bar to decide which: 827px for twenty-one
  buttons, 861px with Table's own, 39px of headroom in a 900px window. Both options were answers to a
  width problem, and reusing the button does not have one. It also needs no new geometry — a list is
  the shape `popup` already lays out, where the picker was the exception.

  **Which menu a button opens is not `Action::opens`'s question.** That is a pure function of the
  button and this depends on the caret, so `Editor::menu_for` is where the two meet, and it is the
  only place that knows. One button, two menus, one lookup.

  **The nine rows carry no icon**, which is the second accepted exception after the heading levels
  and the opposite one: a heading list is letterforms with no words; this is words with no
  letterform. Neither icon font has a glyph for "insert row above", and arrows assembled to mean one
  would fall to a font the rest of the row is not set in.

  **The lit block is the table.** Every cell up to and including the hovered one fills, so the shape
  is read rather than counted; the caption underneath names it ("3 × 4 table") and reads "Table size"
  when nothing is hovered, because the strip's height is reserved either way and an empty strip along
  the bottom of a panel looks like something that failed to draw.

  **`rows` counts the header**, in the picker, in the caption and in `Document::insert_table`. Three
  lit squares mean three rows of table, one of which is the header the delimiter sits under — a
  picker whose smallest offer is two rows lies about its own top-left square.
- **There is a find bar, and the text field is the part that did not exist before.** MDEDIT-44. Until
  it, the document was the only thing in this editor that took a keystroke; `crates/app/src/field.rs`
  is the other one, and Replace, Go to Line and every future dialog-ish thing are meant to reuse it
  rather than grow a second half-copy. It is a `String` and two byte offsets — no rope, no undo
  history, and one line, so a paste of several arrives flattened. **It takes committed text only:**
  `WindowEvent::Ime` still goes to the document, so composing CJK into the search box does not work.
  That is a real gap for the user most likely to want it, and it is written down rather than left to
  be found.

  **The focus model is one flag and it has to stay that way.** Exactly one of {document, field} takes
  a key, and `find_focused` is consulted in one place at the top of `handle_key`. Chords are
  deliberately *not* swallowed — Ctrl+S saves while the caret is in the box, because anything else
  would be a search box that quietly disables the editor. Only select-all, copy, cut and paste are
  taken by the field.

  **The search is over the source, so `**` is findable**, and the current match is simply the
  selection — which is why the reveal, the scroll and the highlight all follow with nothing new
  written for them. Matches are lit *under* the selection quad, so "the one you are on" needs no
  second colour and no index kept in step.

  **It rescans on the keystroke path, and the number is why that is allowed.** The first
  implementation cost **5.8 ms p50** per keystroke on a 224 KB document with 3,000 matches, against
  a 3 ms budget — so `find::matches` gained an ASCII path (a byte comparison for the first character,
  and `eq_ignore_ascii_case` for the rest) and it is now **1.13 ms p50 against 0.63 closed**. The
  ticket said to debounce; the measurement said it was not needed, and `crates/app/tests/find_cost.rs`
  is the harness that decides which of those is true after the next change. The non-ASCII path is
  unchanged and still correct: `İ` lowercases to two characters, so a match may not end half way
  through one character's expansion, and there is a test saying so.

  **The match list's key includes which document it is.** A revision counts one document's edits and
  says nothing about which — two tabs neither has typed in are both at revision zero — so without it
  switching tab kept the first one's offsets. Same shape, and same reason, as `floating_for`.
- **The same bar does Replace and Go to Line, and the mode is what makes it a different shape.**
  MDEDIT-45. Ctrl+H adds a second row with its own field and a Replace / All pair; Ctrl+G is one row,
  one field, digits only. `find::Mode` is in the match list's key too, so switching to Go to Line
  clears the highlights rather than leaving a search the user has moved on from lit up. **Three
  fields, not one reused**: Ctrl+G would otherwise throw away what you were looking for.

  **Replace All is one undo step, and that is what the ticket was waiting for.** `Document::replace_all`
  works **from the end backwards** inside MDEDIT-63's `transact`, so no site moves before it is used
  and no offset arithmetic is needed — undo restores the original bytes, property-tested. A call that
  replaces nothing records no history, so a Replace All that matched nothing does not cost a press of
  Ctrl+Z that appears to do nothing.

  **Replace with the caret away from a match moves to one rather than replacing what is nearby.** The
  second press is what edits. A Replace that acted on the nearest thing is how a find-and-replace
  loses work.

  The two word buttons are **measured**, not given the symbols' fixed width — "Replace" at 24 pixels
  would be a button with three letters on it. `Button::row` returns a row *number* rather than a y
  offset so grouping buttons by row is an integer comparison; comparing two `f32`s for equality would
  work today and fail the first time a height is scaled.
- **There was a bar that floated over a settled selection, and MDEDIT-119 removed it.** MDEDIT-53
  built it when the fixed bar was Markdown-only, per-run and *below* the tab strip — so a selection
  in the middle of a document had no formatting surface within reach. Three tickets took each of
  those reasons away: MDEDIT-84 gave every document a standard set and moved the bar to the top,
  MDEDIT-92 made it icons, and MDEDIT-116 collapsed the headings and gave it an overflow. Its whole
  set — the inline four plus three heading levels — ended up on the fixed bar, which now offers six
  levels where the floating one offered three.

  **The cost is real and was accepted rather than argued away:** a bar at the selection is zero
  pointer travel, and Typora, Notion and Medium all keep one. What settled it is that the second
  surface carried a maintenance cost invisible in the feature — a third z-order in `Editor::click`,
  a second place every inline action was wired, and a set of rules (settle on release, settle on the
  idle beat for a Shift-selection, never during a drag, not for a selection the search made) that
  existed only to stop it being annoying.

  **What went with it is the interesting part.** `selection_anchor`, recorded in `draw_chunks`
  because nothing else knew where a selection's first visible row was. `found_selection`, which
  existed *only* so a search match would not raise a bar. `toolbar::place`, `shifted` and `Anchor`,
  which had no other caller. Twenty-five tests. `tooltip::place` and `popup::place` still clamp the
  way `toolbar::place` did and say so in their own words, because the reasoning outlived the
  function.
- **The View menu's toggles are check items, and the ticks follow the *chord* as well as the click.**
  MDEDIT-40. That is the whole reason a toggle belongs in a menu rather than only on a chord: a chord
  can toggle something but cannot tell you which way it is now. The cost is that every path which
  changes one has to call `refresh_view_menu` — a menu that only knew about its own clicks would be
  wrong the moment anybody pressed Ctrl+/.
- **Zoom is a multiplier, and clearing the caches is the whole of the feature.** MDEDIT-43: Ctrl+= /
  Ctrl+- / Ctrl+0, a 1.1 ladder. Text sizes come from the theme at layout time and are **not** in
  `ChunkKey`, so changing one without clearing leaves every cached chunk drawing at the old size.
  Five things hold laid-out text; four are cleared on zoom — chunk layouts, measured heights, the
  gutter's digits, the status bar's labels, the pane's row labels — and the tab strip's are measured
  per frame so need nothing. The anchor's `into_chunk` is in *pixels*, so it and the scroll offset
  are scaled by the same ratio or zooming mid-document throws the reader somewhere else. Adding
  anything else that caches laid-out text means adding it to `set_zoom`.
- **The recent-files list is state, and is treated as untrusted input.** MDEDIT-41 put it beside the
  session file rather than in `config.json` — the config file is read-only by design and may not name
  a path at all, and nobody hand-writes a recent list. That puts it in the session file's threat
  model: every entry goes through `path::validate` **on the way out and on the way in**, the second
  direction so the list cannot launder a path past the boundary by storing it now and reading it back
  later. `validate`'s last rule is that the path names a regular file, so a deleted file falls out of
  the menu without anything having to prune it.
- **Help → Keyboard Shortcuts is a Markdown document in a tab, not a dialog.** The editor showing its
  own help as Markdown is the cheapest demonstration that it renders headings, tables and inline code,
  and it costs no UI. **The "tables" in that sentence was false from the day it was written until
  MDEDIT-126**, because `ENABLE_TABLES` was off and the page's own table came out as literal pipes.
  The MDEDIT-92 coverage audit caught it and MDEDIT-89 was raised to fix it; what actually fixed it
  was the grid, because turning the flag on alone would have concealed the pipes and left the columns
  merged. The claim is now true, which is worth stating precisely because a page that demonstrates
  the renderer is only useful while somebody checks that it still does. It is generated from one list in `crates/app/src/shortcuts.rs` so the page and
  the bindings cannot drift silently. That needed `Origin::Generated`: a tab that is untitled but
  *not modified*, carrying its own text as its record of disk, because a reference document asking to
  be saved on the way out is a nuisance rather than a safeguard. It is never written to the session —
  it can be regenerated, and a stale copy of a reference is worse than none.
- **A pane row truncates its name; it does not ellipsise it. It does now say the whole of it on
  hover.** MDEDIT-28's plan asked for eliding with the full name on hover, and got half: truncation
  is what the tab strip already does and what this matches, an ellipsis needs its own measurement
  pass, and a tooltip needed a hover affordance that did not exist. MDEDIT-85 built that affordance
  and **MDEDIT-121 is the other half**, for tabs as well as rows.

  **The rule that makes it different from the bar's tooltips**: a name that *fits* says nothing. Item
  1 of MDEDIT-85 was "explain a control", which is true of a button whatever its width; this is "show
  what did not fit", so whether to offer a panel depends on a measurement rather than on what is
  under the pointer. A panel repeating a name already fully on screen is noise.

  **That measurement is recorded as the frame draws, not computed when asked.** Whether a name fits
  depends on the width the strip gave that tab and on the glyphs the engine produced for it, neither
  of which exists before a frame — so `draw_label` returns whether it clipped, and the two loops keep
  a list. An editor that has not drawn a frame knows of no truncated tab, which is what
  `nothing_is_truncated_until_a_frame_has_drawn` pins.

  `Hovered` replaced the bare `Action` the hover state used to be: a button, a tab or a row. A tab's
  panel hangs below the strip and a row's below **that row** — a pane is as tall as the window, so a
  panel below the pane would point at nothing.
- **The pane lists what is open, not what is on disk.** A file *tree* is a different feature with
  filesystem-watching decisions attached, and deliberately not this one.
- **The status bar's word count lags, deliberately.** It is recomputed on the same idle timer that
  runs the whole-document re-parse 150 ms after typing stops, because counting words is a walk of the
  buffer and doing it per frame would undo MDEDIT-17 and MDEDIT-18. While you type, the number shown
  is the one from before you started. A document nobody has typed in yet is counted once, on its
  first frame.

  **Five things share that timer, and it is now scheduled after any edit.** It used to be set only
  when the parse had been scoped — i.e. only above 16 KB — so on an ordinary note it never fired at
  all, which froze the word count and, when MDEDIT-54 arrived, would have meant no crash-recovery
  snapshot for exactly the small unsaved buffers the feature exists to protect. `Editor::on_idle` is
  public so a test can run that work without an event loop.
- **The breadcrumb shows headings, and nothing else.** A caret above the document's first heading
  gets an empty breadcrumb rather than a root. Nothing in the bar is clickable.
- **The status bar is two containers that truncate, not one row that overflows.** The right-hand
  fields ask first, up to 60%% of the bar; the breadcrumb takes what is left, and both clip at their
  own edge. Because the fields read with the least important last, what a narrow window loses is the
  word count, then the line endings, then the encoding — line and column survive longest, which is
  the priority order without anything encoding it. Truncated, never ellipsised: an ellipsis needs its
  own measurement pass.
- **Line and column count characters.** Not bytes, and not display columns — a tab is one column
  here. `ByteOffset` and `CharOffset` exist as separate types so this cannot be fudged; see
  `Document::line_and_column`.
- **There is no menu to reach copy and paste from**, only the chords. A context menu is MDEDIT-58.
- **Four settings, all reachable from the View menu.** `%APPDATA%\SimpleMarkdownEditor\config.json`
  holds `line_numbers`, `chrome`, `zoom` and — since MDEDIT-78 — `theme`. Read at startup and written
  back on the idle timer when one changes — see the settings entry above for the unknown-key rule that makes writing safe.
  **No setting may name a path** — see `crates/app/src/config.rs` for why that is a rule rather than
  a caution. JSON because `serde_json` was already here; the lack of comments is the known cost, and
  TOML is the thing to revisit if this grows past a handful of keys — preserving unknown keys removed
  the strongest reason a hand-editor would object to JSON.
- **A line number counts source lines, not rows.** A wrapped paragraph is numbered once, at its first
  row, which is what every editor does. The gutter is sized for the largest number the document can
  show, with a two-digit floor, so it does not visibly resize as you pass line 9.

- **A line number clears the file pane, and did not.** MDEDIT-117. `draw_line_number` right-aligns
  its digits against the gutter's edge, and measured that from **window x = 0** — right while the
  gutter was the only thing on the left, wrong with the pane showing. `insets.left` is
  `pane_width() + gutter_width()`, so the document moved right by both and the numbers moved not at
  all, landing inside the pane's own band. `draw_pane` runs after `draw_chunks` — the painter-order
  rule, since there is no depth buffer — so they were **drawn and then painted over**.

  The give-away was the empty gutter-width strip between the pane and the text: the space was
  reserved and nothing was in it. Found by eye; no test could have caught it, because the quads are
  emitted either way and only their x is wrong.

  **The regression test asserts a gap, not a coordinate**: wherever the text starts, the number ends
  just before it, and that distance is the same with the strip as with the pane. A coordinate would
  have been a restatement of the fix. Getting there needed a narrower filter than `is_gutter` — with
  the pane showing, "left of the text and between the two bars" also describes the pane's
  background, its active row, and every glyph of the file name on it, and the first attempt failed
  against the *fixed* code for that reason.
