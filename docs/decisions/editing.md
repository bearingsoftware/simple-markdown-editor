# Editing — words, inline formatting and undo

What a word is, how a delimiter is taken off without a regular expression, and where an undo group breaks.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **A word is a Unicode word, and that is decided once for the whole editor.** MDEDIT-57 took
  `unicode-segmentation` (UAX #29) rather than an alphanumeric-run classifier, which splits `café` at
  a decomposed accent, takes a whole CJK run as one word, and cuts `don't` in half. Ctrl+arrows and
  MDEDIT-50's wrap-the-word want the same definition. One deviation, deliberate: UAX #29 breaks a run
  of punctuation into one segment per character, so runs of *non-word* segments are joined — word
  segments are not, because joining those is the CJK failure again. See `Document::word_at`.
- **Inline formatting removes delimiters it can point at, not ones it guessed.** MDEDIT-50:
  `Document::toggle_inline` uses the marker spans, so `**a _b_ c**` with the caret in `b` takes off
  the italic pair and nothing else. That is the differentiator — every editor that bolts Markdown
  buttons onto a text widget does un-bolding with a regular expression and gets nested emphasis
  wrong. Toggling twice is byte-identical, property-tested.

  **Ctrl+B is bold, and the file pane moved to Ctrl+Shift+E.** Non-negotiable: a Markdown editor
  where Ctrl+B moves a sidebar is wrong in a way people notice within a minute. Ctrl+I is italic,
  Ctrl+Shift+X strikethrough, Ctrl+E inline code — the last is this project's choice rather than the
  ticket's, and the weakest of the four.

  **Inside a code span the inline toggles do nothing.** `**` there is not markup, so a second press
  could not find it to take away, and a toggle that cannot be un-toggled is not one. The empty pair a
  press with no selection leaves behind *is* matched, textually and narrowly, for the same reason —
  otherwise pressing the chord twice by mistake leaves `****` with no way off but backspace.
- **Home and End move within the caret's visual row.** MDEDIT-192. Until then both plain keys
  went to the ends of the document, the same as Ctrl+Home and Ctrl+End, so two of the four bindings
  were one command twice. No test pressed any of them, and the code was duplicated between
  `handle_chord` and the plain key handler, which is how they stayed identical without anyone
  noticing.

  **The visual row, not the source line**, because Up and Down already move by visual row and a
  wrapped paragraph's continuations each have a start of their own. **Home and End are point
  gestures**: `press_home` and `press_end` aim beyond the left or right edge of the caret's row,
  at its vertical middle, through the same hit test a click uses. So End inherits MDEDIT-188's
  affinity for free. At a soft wrap the row's end is the same byte as the next row's start, and End
  leaves the caret drawn on the row it ended, which is exactly what a click past that row's end
  already did.

  **In preview Home stops after a concealed marker, and that is not a style choice.** A caret
  before `# ` or `- ` draws at the same x as the marker's far edge, and `past_concealed` is the
  invariant every motion keeps. **Plain markup conceals nothing, so Home reaches column zero
  there**, and it asks the *mode* to know that, because `past_concealed` does not: the document
  conceals by language. That asymmetry is what found MDEDIT-194, where the arrow keys in plain
  markup still treat a marker as one uncrossable step. **No two-press smart Home**, which toggles
  between the text and the margin. It would be a second definition of a row's start, and nobody had
  asked for one.

  Ctrl+Home and Ctrl+End are `press_document_edge`, so a test can tell all four apart from one
  caret. What no test can reach is the line mapping a key to its method, because a winit `KeyEvent`
  cannot be built outside winit. That line is a check by hand, as Ctrl+Enter's is (MDEDIT-175). One
  hand-mutation survives as equivalent: Home's aim of `f32::MIN` behaves the same as `0.0`, because
  document coordinates start at the text's left edge even when the view is panned.
- **Undo groups break on caret movement, not on a timer.** Deliberate — see `crates/document/src/history.rs`.
  It means a long pause mid-word still undoes as one step, which is the one place the behaviour
  differs from editors that use a clock.
- **Ctrl+Enter ticks the task on the caret's line.** MDEDIT-175. MDEDIT-142 made a box pressable,
  and in preview that left the mouse as the only way to tick one: the caret steps over the bullet
  and the box as one concealed run, so it cannot stand in `[ ]` to type the `x`. Obsidian's binding
  for the same thing. `Document::toggle_selected_tasks` writes the byte between the brackets, as a
  press on the box does, and puts the selection back, so the caret never moves.

  **A selection applies uniformly, as `set_block` does**: any unticked box means tick them all, and
  only when every one is ticked does a press untick them. Toggling each turns "three done, one not"
  into the reverse, and leaves no press that reliably finishes a list. A box already in the state
  asked for is not written, so `[X]` keeps its capital. One press is one undo step, however many
  boxes.

  **A line with no box is left alone.** Obsidian makes it a task; this does not, so the chord only
  ever changes a box's state and never what a line is. The Task button already makes a task. The
  boxes are the parse's, so `[ ]` in a code block is not ticked, and plain markup works the same,
  because concealment is how a box is drawn, not whether there is one.

  **No menu item.** Bold, italic and the heading levels have none either. The Formatting group of
  the shortcuts reference is where this is written down, and `tests/shortcuts.rs` holds the two
  together. **Ctrl+Shift+Enter is left unbound** on purpose: it is "insert a line above" in VS Code
  and Word, and a chord nobody has claimed is cheaper to keep free than to take back.

  `handle_chord` hands Enter on to `apply_chord` as `"\n"`, as it hands Tab on as `"\t"`, so a test
  can press it. That one mapping stays a manual check, because a test cannot construct the winit
  `KeyEvent` it reads.
