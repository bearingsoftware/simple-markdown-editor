//! What the frame actually draws once the view has moved.
//!
//! Runs against a stub text backend rather than DirectWrite, so it runs on CI as well as here and
//! pins the arithmetic rather than the font: the quads' positions are the thing under test.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::fmt::Write as _;

use mdedit_app::editor::Editor;
use mdedit_app::gpu::QuadInstance;
use mdedit_document::Selection;
use mdedit_offsets::{ByteOffset, ByteSpan, LayoutPos};
use mdedit_text::{
    FontId, GlyphId, GlyphImage, GlyphKey, Layout, LayoutLine, PositionedGlyph, StyledRun,
    TextError, TextLayout,
};

/// One line, as a whole number of pixels: the tests compare rounded rows, so the step has to be
/// expressible as one too.
const LINE_STEP: i32 = 20;
const LINE_HEIGHT: f32 = 20.0;
const ASCENT: f32 = 16.0;
const ADVANCE: f32 = 8.0;
/// Matches `MARGIN` in the editor. A test that hardcodes it would silently pass if it changed.
const MARGIN: f32 = 24.0;

/// A text backend with no font behind it: one line per `\n`, one fixed-width glyph per character.
///
/// Enough for the frame to have geometry to place, and predictable enough that a wrong offset is
/// arithmetic rather than a rendering judgement call.
#[derive(Default)]
struct Grid {
    /// How many chunks have been through the engine. The whole point of MDEDIT-17 is that this
    /// stays small however long the document is, so the tests assert on it directly.
    layouts: usize,
}

impl Grid {
    fn new() -> Self {
        Self::default()
    }

    fn layouts(&self) -> usize {
        self.layouts
    }
}

impl TextLayout for Grid {
    fn layout(&mut self, runs: &[StyledRun<'_>], _max_width: f32) -> Result<Layout, TextError> {
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
                while lines.len() <= row {
                    lines.push(new_line(lines.len()));
                }
                let start = run.source.start().get() + index;
                let baseline = lines[row].baseline;
                lines[row].glyphs.push(PositionedGlyph {
                    key: GlyphKey {
                        font: FontId::new(0),
                        glyph: GlyphId::new(1),
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
    }
}

/// A window 200 device-independent pixels tall below the margin, at scale 1.
const WIDTH: f32 = 400.0;
const HEIGHT: f32 = MARGIN + 200.0;

fn document_of(lines: usize) -> String {
    let mut text = String::new();
    for n in 0..lines {
        writeln!(text, "line {n}").expect("a String never fails to write");
    }
    text
}

fn editor(lines: usize) -> Editor<Grid> {
    let mut editor = Editor::new(&document_of(lines), Grid::new());
    // One frame so the viewport has been measured; scrolling before the first paint has nothing to
    // clamp against, which is correct but not what these tests are about.
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// The vertical position of every quad drawn, rounded so float noise does not defeat comparison.
fn rows(quads: &[QuadInstance]) -> Vec<i32> {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "quad positions are small pixel coordinates"
    )]
    let mut rows: Vec<i32> = quads
        .iter()
        .map(|quad| quad.rect[1].round() as i32)
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// Where the glyphs of line `n` land on screen when the view has not moved.
///
/// The stub's glyph bitmap sits one pixel above the baseline, which is the `- 1.0`.
fn glyph_row(line: f32) -> i32 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "quad positions are small pixel coordinates"
    )]
    let row = (line.mul_add(LINE_HEIGHT, ASCENT) + MARGIN - 1.0).round() as i32;
    row
}

#[test]
fn a_document_taller_than_the_window_is_only_drawn_as_far_as_the_window_goes() {
    let mut editor = editor(100);
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let drawn = rows(&quads);
    assert!(
        // A line that starts inside the window is drawn even if it runs past the bottom edge; one
        // that starts below it is not.
        drawn
            .iter()
            .all(|y| f32::from(i16::try_from(*y).unwrap()) < HEIGHT + LINE_HEIGHT),
        "nothing should start below the window: {drawn:?}"
    );
    assert!(
        drawn.len() < 20,
        "a 100-line document cannot fit in 200 pixels, so most of it is off screen: {drawn:?}"
    );
}

/// The acceptance criterion: what is drawn changes when the view moves.
#[test]
fn scrolling_draws_what_was_below_the_fold() {
    let mut editor = editor(100);
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);

    // Not a whole number of lines, deliberately: a multiple would put a different line at exactly
    // the same pixel row and the assertions below could not tell the two frames apart.
    editor.scroll_by(510.0);
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.scroll_offset(), 510.0);
    assert_ne!(rows(&before), rows(&after), "the drawn set did not change");

    // Specifically: the first line has gone, and lines that were below the window are now on it.
    assert!(rows(&before).contains(&glyph_row(0.0)));
    assert!(!rows(&after).contains(&glyph_row(0.0)));
    // Line 26 sits just past 510 pixels down, so it is now near the top of the window.
    assert!(rows(&after).contains(&(glyph_row(26.0) - 510)));
    assert!(after.len() > 1, "there is content down there to draw");
}

#[test]
fn everything_drawn_moves_by_exactly_the_offset() {
    let mut editor = editor(100);
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.scroll_by(LINE_HEIGHT);
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0);

    // One line down means the second row of the old frame is the first row of the new one.
    let shifted: Vec<i32> = rows(&before)
        .into_iter()
        .skip(1)
        .map(|y| y - LINE_STEP)
        .collect();
    let now = rows(&after);
    assert!(
        shifted.iter().all(|y| now.contains(y)),
        "expected {shifted:?} to survive in {now:?}"
    );
}

#[test]
fn a_document_shorter_than_the_window_does_not_scroll() {
    let mut editor = editor(3);
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.scroll_by(1000.0);
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.scroll_offset(), 0.0);
    assert_eq!(rows(&before), rows(&after));
}

#[test]
fn the_view_stops_at_the_end_of_the_document() {
    let mut editor = editor(100);
    editor.scroll_by(1_000_000.0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    // 101 lines at 20 px — the trailing newline opens a line of its own, as it does in any editor
    // — plus a bottom margin, less the 200 px of visible document.
    let content = 101.0f32.mul_add(LINE_HEIGHT, MARGIN);
    assert_eq!(editor.scroll_offset(), content - 200.0);
}

#[test]
fn paging_moves_the_view_without_moving_the_caret() {
    let mut editor = editor(100);
    let caret = editor.document().selection();

    editor.page(1.0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(editor.scroll_offset() > 0.0);
    assert_eq!(
        editor.document().selection(),
        caret,
        "paging is a view movement, not an edit"
    );
}

/// Byte preservation, at the one place scrolling could plausibly touch it.
#[test]
fn scrolling_to_the_end_and_back_leaves_the_buffer_alone() {
    let source = document_of(100);
    let mut editor = editor(100);

    editor.scroll_by(100_000.0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.scroll_by(-100_000.0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.scroll_offset(), 0.0);
    assert_eq!(editor.document().to_string(), source);
}

// ---------------------------------------------------------------------------
// Scroll-follows-caret, through the frame rather than through the arithmetic alone.
// ---------------------------------------------------------------------------

#[test]
fn typing_at_the_bottom_of_a_long_document_brings_the_caret_into_view() {
    let mut editor = editor(100);
    let end = ByteOffset::new(editor.document().len_bytes());
    editor.document_mut().set_selection(Selection::caret(end));
    editor.document_mut().insert(end, "x").unwrap();
    editor.follow_caret();

    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        editor.scroll_offset() > 0.0,
        "the caret is 100 lines down; the view has to move"
    );

    // The caret is the last quad the frame emits.
    let caret = quads.last().expect("a caret quad");
    let top = caret.rect[1];
    assert!(
        top >= 0.0 && top + caret.rect[3] <= HEIGHT,
        "caret at {top} is outside the window"
    );
}

#[test]
fn a_caret_already_on_screen_does_not_move_the_view() {
    let mut editor = editor(100);
    editor.scroll_by(200.0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    // A caret on a line that is currently visible: 200 px down is line 10, and the window shows
    // ten lines from there.
    let target = ByteOffset::new(document_of(12).len());
    editor
        .document_mut()
        .set_selection(Selection::caret(target));
    editor.follow_caret();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.scroll_offset(), 200.0);
}

// ---------------------------------------------------------------------------
// Virtualization: what a frame is allowed to look at (MDEDIT-17).
// ---------------------------------------------------------------------------

/// A document of separate blocks, so it is cut into many chunks rather than one paragraph.
fn blocks_document(count: usize) -> String {
    let mut text = String::new();
    for n in 0..count {
        writeln!(text, "## Heading {n}\n\nParagraph {n}.\n")
            .expect("a String never fails to write");
    }
    text
}

/// The point of the whole ticket: what is off screen is not laid out.
#[test]
fn only_the_chunks_on_screen_are_laid_out() {
    let mut editor = Editor::new(&blocks_document(400), Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let laid_out = editor.text_backend().layouts();
    assert!(
        laid_out < 40,
        "a 400-block document should not lay out 400 blocks to fill 200 pixels: {laid_out}"
    );
    assert!(laid_out > 0, "something has to be drawn");
}

/// The cache is what makes the second frame cheap, and it survives an edit elsewhere.
#[test]
fn a_second_frame_lays_nothing_out_again() {
    let mut editor = Editor::new(&blocks_document(400), Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let first = editor.text_backend().layouts();

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let second = editor.text_backend().layouts() - first;
    assert!(
        second <= 2,
        "only the caret's chunk should be laid out again, not {second} of them"
    );
}

/// Typing shifts every byte after the caret, so a cache keyed on position would miss everywhere.
#[test]
fn editing_the_first_block_does_not_invalidate_the_rest() {
    let mut editor = Editor::new(&blocks_document(400), Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let before = editor.text_backend().layouts();

    editor
        .document_mut()
        .insert(ByteOffset::new(3), "x")
        .unwrap();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let again = editor.text_backend().layouts() - before;
    assert!(
        again <= 2,
        "the edited chunk should be the only one laid out again, not {again}"
    );
}

/// Scrolling somewhere new does lay out what arrives — the cost follows the viewport, not the file.
#[test]
fn scrolling_lays_out_what_comes_into_view() {
    let mut editor = Editor::new(&blocks_document(400), Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let before = editor.text_backend().layouts();

    editor.scroll_by(4000.0);
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let arrived = editor.text_backend().layouts() - before;
    assert!(arrived > 0, "new content has to be measured");
    assert!(
        arrived < 40,
        "and only what is on screen: {arrived} chunks laid out"
    );
    assert!(quads.len() > 1, "there is something down there");
}

/// The acceptance criterion that guards against a cache that returns the wrong layout: what a
/// chunk draws must not depend on where an identical chunk was drawn before.
#[test]
fn identical_blocks_in_different_places_draw_in_different_places() {
    let source = "same\n\nsame\n\nsame\n";
    let mut editor = Editor::new(source, Grid::new());
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let drawn = rows(&quads);
    assert!(
        drawn.len() >= 3,
        "three identical paragraphs occupy three different rows: {drawn:?}"
    );
}

/// Concealment is the document's answer, not the renderer's, and a chunk boundary must not change
/// it. A fence reveals its `` ``` `` from anywhere inside the block — including when the caret's line is
/// off screen, since the fence is one chunk.
#[test]
fn a_fence_reveals_its_markers_from_anywhere_inside_it() {
    let source = "before\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n\nafter\n";
    let caret = source.find("let b").expect("the second line of the fence");

    let mut editor = Editor::new(source, Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(caret)));
    let quads = editor.build_frame(WIDTH, 2000.0, 1.0);

    // The stub draws one glyph per character, so counting quads counts drawn characters. With the
    // fences revealed there are six more than with them concealed.
    let revealed = quads.len();

    let mut away = Editor::new(source, Grid::new());
    away.document_mut()
        .set_selection(Selection::caret(ByteOffset::new(0)));
    let concealed = away.build_frame(WIDTH, 2000.0, 1.0).len();

    assert!(
        revealed > concealed,
        "the fence markers should be drawn when the caret is inside the block: {revealed} vs {concealed}"
    );
}

/// Chunks are laid out separately and stacked, so the arithmetic that joins them has to produce
/// exactly what one layout of the whole document would have: no lost blank line, no doubled one.
#[test]
fn stacked_chunks_land_where_one_layout_would_have_put_them() {
    let source = "# Title\n\npara\n\nlast\n";
    let mut editor = Editor::new(source, Grid::new());
    let quads = editor.build_frame(WIDTH, 2000.0, 1.0);

    // Lines: "# Title", blank, "para", blank, "last". The first row is the caret, which sits at the
    // top of line 0 rather than on its baseline.
    let drawn = rows(&quads);
    assert_eq!(drawn[0], 24, "the caret, at the top of line 0");
    assert_eq!(
        &drawn[1..],
        [glyph_row(0.0), glyph_row(2.0), glyph_row(4.0)],
        "three lines of text, with a blank line between each pair"
    );
}
