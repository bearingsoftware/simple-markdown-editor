# Text layout — the trait and its one implementation

Why DirectWrite is required on Windows today, and what the trait costs while it has no second implementation to check it against.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **No portable text implementation.** `mdedit-text-dwrite` is the only backend, so the trait has
  one implementation and therefore no correctness oracle yet. parley fills that in when
  [parley#689](https://github.com/linebender/parley/pull/689) ships released, and until then the
  app does not run at all off Windows.
- **A run can be underlined or struck, and where the rule sits is the font's answer rather than
  ours.** MDEDIT-73, then MDEDIT-91. `TextStyle::underline` and `::strikethrough` reach DirectWrite as
  `SetUnderline` and `SetStrikethrough`, and what comes back is a `mdedit_text::Rule` **per row** —
  kind, x, width, top and thickness, the last two from the font's own metrics. That is what makes a
  wrapped ruled run ruled on both rows, and what makes the rule grow with a zoom, without the
  renderer knowing a rule from a rectangle: it draws the quad and decides only the colour, which is
  the colour of the run it belongs to.

  **One type for both, distinguished by a `kind` field**, because everything else about them is
  identical — the font decides the offset and the thickness, wrapping splits them per row, and the
  caller draws a quad. Adding the second cost a field on `Rule` and **no branch in the renderer**:
  DirectWrite measures both offsets downwards from the baseline, so a strikethrough's is simply
  negative and the same arithmetic places it.

  Two flags rather than one enum on `TextStyle`, because a run can be both: `~~[a](/url)~~` is a
  struck-through link whose destination is still underlined.

  **`DWRITE_UNDERLINE` and `DWRITE_STRIKETHROUGH` carry geometry and no text position**, so a rule is
  matched to its text by *position* — the first glyph under it names the byte the colour is looked up
  from. A rule with no glyph under it is whitespace only, and is dropped rather than coloured by a
  guess.

  **Neither is in `ChunkKey` and neither needs to be**: like every other `TextStyle` field they are
  derived from the chunk's bytes and the drawing mode, and both of those are.

  **No test on the stub can see any of this** — a rule's position comes from a font and the stub has
  none, so it reports no rules at all. `crates/text-dwrite/tests/layout.rs` is where the behaviour is
  held up, against the real system stack, which is the only oracle this layer has. What *can* be
  tested headlessly is the other half: whether the decoration step asked for a rule at all, which is
  `crates/app/tests/strikethrough.rs`.

  **Strikethrough is applied to content only, and only in preview.** A struck run's own `~~` is
  concealed, so there is nothing to strike; and plain markup is what you switch to in order to read
  the bytes, so a line through them would hide what is being read. `DrawStrikethrough` sat next to
  `DrawUnderline` as a no-op for four tickets, with a comment saying the renderer would decorate
  `~~struck~~` itself from the marker spans. It could have — but the position and the thickness are
  the font's answers, not the marker's, and doing it the other way would have been the same job in
  two places.
