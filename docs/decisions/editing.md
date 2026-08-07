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
- **Undo groups break on caret movement, not on a timer.** Deliberate — see `crates/document/src/history.rs`.
  It means a long pause mid-word still undoes as one step, which is the one place the behaviour
  differs from editors that use a clock.
