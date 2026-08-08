# Text layout — the trait and its one implementation

Why DirectWrite is required on Windows today, and what the trait costs while it has no second implementation to check it against.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **No portable text implementation.** `mdedit-text-dwrite` is the only backend, so the trait has
  one implementation and therefore no correctness oracle yet. parley fills that in when
  [parley#689](https://github.com/linebender/parley/pull/689) ships released, and until then the
  app does not run at all off Windows.
- **A run can be underlined, and where the rule sits is the font's answer rather than ours.**
  MDEDIT-73. `TextStyle::underline` reaches DirectWrite as `SetUnderline`, and what comes back is a
  `mdedit_text::Underline` **per row** — x, width, top and thickness, the last two from the font's own
  metrics. That is what makes a wrapped underlined run underlined on both rows, and what makes the
  rule grow with a zoom, without the renderer knowing a rule from a rectangle: it draws the quad and
  decides only the colour, which is the colour of the run above it.

  **Nothing asks for one yet.** The theme's `link` role is what will, and colouring markers per kind
  is MDEDIT-75; this is the same shape as MDEDIT-71's unread roles, deliberately.

  **`DWRITE_UNDERLINE` carries geometry and no text position**, so a rule is matched to its text by
  *position* — the first glyph under it names the byte the colour is looked up from. A rule with no
  glyph under it is whitespace only, and is dropped rather than coloured by a guess.

  **Underline is not in `ChunkKey` and does not need to be**: like every other `TextStyle` field it is
  derived from the chunk's bytes and the drawing mode, and both of those are.

  **No test on the stub can see any of this** — a rule's position comes from a font and the stub has
  none, so it reports no underlines at all. `crates/text-dwrite/tests/layout.rs` is where the
  behaviour is held up, against the real system stack, which is the only oracle this layer has.
