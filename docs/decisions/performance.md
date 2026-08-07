# Performance — what a keystroke costs and what is never materialised

Startup attribution, the scoped re-parse, chunking and virtualization, the two scroll axes, and what is measured rather than asserted.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

- **Startup costs ~250 ms before anything is drawn, and none of it is ours to fix.** MDEDIT-20
  attributed it with `spikes/startup-attribution`: creating the window is 60–79 ms, `request_adapter`
  70–100, `request_device` 62–72, `surface.configure` 39–54. Shader compilation is **0.3 ms** and
  pipeline creation under 11, so the obvious suspects are not it, and `PowerPreference` makes no
  difference. Creating a window alone exceeds the ≤50 ms target before any GPU work starts, so that
  target has been **split**: cold start is recorded rather than budgeted, and `file open → first
  paint ≤50 ms` now applies to a process that is already running — which is the case that matters,
  because opening N files from Explorer is required to reuse one process. See
  `docs/research/06-performance-targets.md`. Still measured on every run: `mdedit` prints
  `launch -> first paint` on exit.
- **A keystroke re-parses a region, not the document.** MDEDIT-18: an edit re-parses the block it is
  in plus a block of margin either side, and splices that into the existing analysis. A region
  cannot see what the rest of the document does to it — an unclosed fence above, a link reference
  definition below — so the whole document is re-parsed 150 ms after the typing stops. Documents
  under 16 KB skip all of this and parse whole, exactly as they used to.
- **A chunk is a whole top-level block.** A document that is one enormous list or one enormous
  paragraph is one chunk, and no cheaper than before — only its *drawing* is culled, line by line.
  Splitting inside a block needs the layout engine to lay out part of one, which none of them does.
- **Off-screen heights are estimated**, at a line count times a nominal line height, so a long
  wrapped paragraph nobody has scrolled to yet is under-estimated. The estimate is corrected the
  moment a chunk is measured, and the viewport is anchored so the correction does not move what is
  on screen — but the document's total height, and therefore how far it can scroll, is approximate
  until it has all been looked at. This is what a scrollbar would make visible, and there is none.
- **Scrolling has two axes now, and the second one only exists when word wrap is off.** MDEDIT-49.
  View → Word Wrap, per-run like the drawing mode; with it off, chunks lay out at `UNWRAPPED` (a
  large *finite* width, because it goes into `ChunkKey` and is hashed there) and **Shift+wheel** pans.
  No horizontal scrollbar, the same position as the vertical one. Scrolling is still instant — no
  smooth or momentum scrolling.

  **`ContentRect::to_window` takes an `Offset`, not an `f32`,** and that is the whole safety argument
  of the ticket: the conversions used to take a bare vertical scroll, so every caller that had never
  considered a second axis kept compiling and would have drawn in the wrong place. Making it a pair
  made the compiler ask each of the ten call sites once. The gutter and the file pane are unaffected
  because they draw in window coordinates and never ask.

  **The horizontal clamp is measured, not estimated**, so it only knows about chunks that have been
  laid out — the same position the vertical axis is in with its estimated heights, and it grows as
  more of the document is looked at. `widest` is cleared with the layout caches by both `set_wrap`
  and `set_zoom`.

  **The caret's horizontal reveal happens after `warm`, not beside the vertical one**, and that is
  load-bearing: before `warm` the chunk the caret is in may not be measured — which is *always* true
  of the chunk just edited — so `widest` was zero there and the reveal clamped to nothing. Typing at
  the end of a long line left the caret off the right edge.

  **And a pan clears `follow_caret`, or it is undone by the frame it asks for.** This is the one the
  window found and the tests could not: Shift+wheel is the only way to pan, **Shift is a key press**,
  so holding it re-armed the caret reveal — the pan landed, the next frame revealed the caret at the
  left edge, and the offset went straight back to zero. Panning did nothing at all, on any document,
  while `pan_by` itself was provably correct in isolation. The vertical axis has the same hazard and
  hides it, because a plain wheel is not preceded by a keystroke. **Anything that moves the viewport
  deliberately has to say so** — `drag_thumb` already did, which is what it should have been read
  against.

  Measured at 333 KB of prose: **wrap on p50 0.98–1.36 ms, wrap off 1.23–1.48 ms** across three runs,
  both well inside the 3 ms budget and overlapping run to run. `crates/app/tests/wrap_cost.rs`.
- **No perf assertions.** The bench in `crates/markers/benches` measures; nothing gates a regression.
- **CI does not run the app**, only the headless crates. There is no way to open a window on a CI
  runner, so rendering is verified by hand on Windows.
