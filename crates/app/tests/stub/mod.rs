//! A text backend with no font behind it, shared by the tests that need a laid-out document.
//!
//! One line per `\n`, one fixed-width glyph per character, at whole-pixel metrics. Enough for the
//! frame to have geometry to place, and predictable enough that a wrong position is arithmetic
//! rather than a rendering judgement call — which is what makes the renderer assertable on a CI
//! runner with no GPU and no fonts worth relying on.
//!
//! It reports no underlines. Where a rule sits and how thick it is come from a font's own metrics,
//! and there is no font here — so MDEDIT-73's rendering is verified against DirectWrite and by eye,
//! which is what its ticket says.

// Each integration test binary compiles this separately, so one of them not using a given helper is
// expected rather than a mistake.
#![allow(dead_code)]

use mdedit_offsets::{ByteOffset, ByteSpan, LayoutPos};
use mdedit_text::{
    FontId, GlyphId, GlyphImage, GlyphKey, Layout, LayoutLine, PositionedGlyph, StyledRun,
    TextError, TextLayout,
};

pub const LINE_HEIGHT: f32 = 20.0;
pub const ASCENT: f32 = 16.0;
pub const ADVANCE: f32 = 8.0;
/// Matches `MARGIN` in the editor. A test that hardcodes it would silently pass if it changed.
pub const MARGIN: f32 = 24.0;

/// A text backend with no font behind it: one line per `\n`, one fixed-width glyph per character.
///
/// Enough for the frame to have geometry to place, and predictable enough that a wrong offset is
/// arithmetic rather than a rendering judgement call.
#[derive(Default)]
pub struct Grid {
    /// How many chunks have been through the engine. The whole point of MDEDIT-17 is that this
    /// stays small however long the document is, so the tests assert on it directly.
    layouts: usize,
}

impl Grid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn layouts(&self) -> usize {
        self.layouts
    }
}

impl TextLayout for Grid {
    fn layout(&mut self, runs: &[StyledRun<'_>], max_width: f32) -> Result<Layout, TextError> {
        self.layouts += 1;
        let mut lines: Vec<LayoutLine> = Vec::new();
        let mut pen_x = 0.0;
        let mut row = 0usize;

        for run in runs {
            for (index, character) in run.text.char_indices() {
                if character == '\n' {
                    // An empty line still occupies one, so push it before moving on.
                    while lines.len() <= row {
                        lines.push(new_line(lines.len()));
                    }
                    row += 1;
                    pen_x = 0.0;
                    continue;
                }
                // Wrapping, crudely: at the character rather than at a word boundary, which is
                // wrong for prose and exactly right for a test — a row break happens at a position
                // arithmetic can predict. Without it nothing that only happens on a wrapped line
                // could be tested at all.
                if pen_x + ADVANCE > max_width && pen_x > 0.0 {
                    row += 1;
                    pen_x = 0.0;
                }
                while lines.len() <= row {
                    lines.push(new_line(lines.len()));
                }
                let start = run.source.start().get() + index;
                let baseline = lines[row].baseline;
                lines[row].glyphs.push(PositionedGlyph {
                    key: GlyphKey {
                        font: FontId::new(0),
                        // **A private-use code point is `.notdef` here, and that is a simulation
                        // rather than a shortcut.** This grid has no icon font — nothing does off
                        // Windows — so MDEDIT-92's `U+E8DD` has to come back the way it would on a
                        // machine without Segoe Fluent Icons, or every test in this crate would
                        // exercise the icon path and none would exercise the fallback that exists
                        // for exactly that machine. The real font is asked in `tests/icons.rs`.
                        glyph: if ('\u{E000}'..='\u{F8FF}').contains(&character) {
                            GlyphId::new(0)
                        } else {
                            GlyphId::new(1)
                        },
                        size: run.style.size,
                    },
                    pos: LayoutPos::new(pen_x, baseline),
                    advance: ADVANCE,
                    source: ByteSpan::new(
                        ByteOffset::new(start),
                        ByteOffset::new(start + character.len_utf8()),
                    ),
                });
                pen_x += ADVANCE;
            }
        }
        // **Empty input lays out to nothing**, as DirectWrite does — `Layout::default()`, no lines
        // at all. This used to fabricate line 0 for it, and MDEDIT-81 is what that hid: a new
        // document drew no caret, because `caret_position` gave up on `lines.get(0)` and the stub
        // never let a test see it. `crates/text-dwrite/tests/layout.rs` pins the real behaviour;
        // this is the stub agreeing with it.
        if runs.iter().all(|run| run.text.is_empty()) {
            return Ok(Layout::default());
        }
        while lines.len() <= row {
            lines.push(new_line(lines.len()));
        }
        Ok(Layout { lines })
    }

    fn rasterize(
        &mut self,
        _key: GlyphKey,
        _scale_factor: f32,
    ) -> Result<Option<GlyphImage>, TextError> {
        Ok(Some(GlyphImage {
            width: 1,
            height: 1,
            left: 0,
            top: -1,
            coverage: vec![255],
        }))
    }

    fn font_name(&self, _font: FontId) -> Option<String> {
        Some("grid".to_owned())
    }
}

fn new_line(row: usize) -> LayoutLine {
    #[allow(
        clippy::cast_precision_loss,
        reason = "row counts in a test document are small"
    )]
    let row = row as f32;
    LayoutLine {
        baseline: row * LINE_HEIGHT + ASCENT,
        height: LINE_HEIGHT,
        glyphs: Vec::new(),
        rules: Vec::new(),
    }
}
