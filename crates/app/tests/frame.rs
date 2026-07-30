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
use mdedit_app::status::BAR_HEIGHT;
use mdedit_app::tabs::STRIP_HEIGHT;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{ADVANCE, ASCENT, Grid, LINE_HEIGHT, MARGIN};

/// One line, as a whole number of pixels: the tests compare rounded rows, so the step has to be
/// expressible as one too.
const LINE_STEP: i32 = 20;

/// The status bar's two strings — the breadcrumb, and the right-hand fields.
///
/// They are laid out when what they say changes and not otherwise, so this is a per-*edit* cost and
/// never a per-frame one. Named here so a test's budget says which part of it is chrome.
const CHROME_LABELS: usize = 2;

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

/// The vertical position of every quad the *document* drew, rounded so float noise does not defeat
/// comparison.
///
/// The status bar draws its own glyphs across the foot of every window, and they are not what any of
/// these tests are about — so anything at or below where it starts is left out. `bar_top` is
/// [`status_bar_top`] for whatever height the frame was built at.
fn rows_above(quads: &[QuadInstance], bar_top: i32) -> Vec<i32> {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "quad positions are small pixel coordinates"
    )]
    let mut rows: Vec<i32> = quads
        .iter()
        .map(|quad| quad.rect[1].round() as i32)
        .filter(|y| *y < bar_top)
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// Where the status bar starts, for a window `height` tall.
#[allow(
    clippy::cast_possible_truncation,
    reason = "window heights in these tests are small whole pixels"
)]
fn status_bar_top(height: f32) -> i32 {
    (height - BAR_HEIGHT).round() as i32
}

/// [`rows_above`] for the standard window these tests use.
fn rows(quads: &[QuadInstance]) -> Vec<i32> {
    rows_above(quads, status_bar_top(HEIGHT))
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
    // — plus a bottom margin, less the visible document: the 200 px below the top margin, less what
    // the status bar takes off the foot.
    let content = 101.0f32.mul_add(LINE_HEIGHT, MARGIN);
    assert_eq!(editor.scroll_offset(), content - (200.0 - BAR_HEIGHT));
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
    // Two for the chunk, and up to two more for the status bar: this edit moves the caret, so the
    // "Ln, Col" fields really do say something new, and it lands in the first heading, so the
    // breadcrumb does too. Those are the chrome's whole per-edit cost — see `Editor::status_label`,
    // which is what keeps them from being laid out per *frame* rather than per change.
    assert!(
        again <= 2 + CHROME_LABELS,
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
    let drawn = rows_above(&quads, status_bar_top(2000.0));
    assert_eq!(drawn[0], 24, "the caret, at the top of line 0");
    assert_eq!(
        &drawn[1..],
        [glyph_row(0.0), glyph_row(2.0), glyph_row(4.0)],
        "three lines of text, with a blank line between each pair"
    );
}

// ---------------------------------------------------------------------------
// Putting the caret where you point (MDEDIT-21).
// ---------------------------------------------------------------------------

/// The x of the `n`th character on a line, in window coordinates.
fn column(n: f32) -> f32 {
    MARGIN + n * ADVANCE
}

/// The y of visual line `n`, in window coordinates — the middle of it, as a click would land.
fn line_y(n: f32) -> f32 {
    MARGIN + n.mul_add(LINE_HEIGHT, LINE_HEIGHT / 2.0)
}

#[test]
fn clicking_puts_the_caret_where_you_clicked() {
    let source = "hello world\n\nsecond line\n";
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(3.0), line_y(0.0));
    assert_eq!(editor.document().selection().head(), ByteOffset::new(3));

    // The third line starts at byte 13.
    editor.click_at(column(1.0), line_y(2.0));
    assert_eq!(editor.document().selection().head(), ByteOffset::new(14));
}

/// The half-character rule: the left half of a character puts the caret before it, the right half
/// after. Without it there is no way to click *between* two letters.
#[test]
fn clicking_snaps_to_the_nearer_edge_of_a_character() {
    let mut editor = Editor::new("abc\n", Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(1.0) + 1.0, line_y(0.0));
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(1),
        "just past the start of the second character is still before it"
    );

    editor.click_at(column(1.0) + ADVANCE - 1.0, line_y(0.0));
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(2),
        "nearly at its end is after it"
    );
}

#[test]
fn clicking_past_the_end_of_a_line_lands_at_its_end() {
    let source = "short\n\nlonger line here\n";
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(40.0), line_y(0.0));
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(5),
        "the end of the first line, not the start of the next"
    );
}

#[test]
fn clicking_a_blank_line_lands_on_it() {
    let source = "first\n\nthird\n";
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(2.0), line_y(1.0));
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(6),
        "the blank line between them, not the line after"
    );
}

#[test]
fn clicking_below_the_last_line_lands_at_the_end() {
    let mut editor = Editor::new("one\n\ntwo\n", Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(0.0), HEIGHT - 1.0);
    let caret = editor.document().selection().head();
    assert!(
        caret.get() >= 5,
        "expected somewhere in the last line, got {caret}"
    );
}

#[test]
fn clicking_before_the_first_frame_does_nothing() {
    let mut editor = Editor::new("text\n", Grid::new());
    editor.click_at(column(2.0), line_y(0.0));
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::ZERO,
        "nothing has been laid out, so there is nothing to point at"
    );
}

// ---------------------------------------------------------------------------
// Up and down.
// ---------------------------------------------------------------------------

#[test]
fn down_moves_to_the_next_visual_line() {
    let source = "first line\n\nsecond line\n";
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(3.0), line_y(0.0));
    editor.move_caret(true, false);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(11),
        "the blank line, which has no column 3 to land on"
    );

    editor.move_caret(true, false);
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(15),
        "column 3 of the third line, which starts at 12"
    );
}

#[test]
fn up_from_the_first_line_goes_to_the_start() {
    let mut editor = Editor::new("only line\n", Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(4.0), line_y(0.0));
    editor.move_caret(false, false);
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::ZERO,
        "there is nowhere above to go"
    );
}

/// The column is remembered: passing through a short line must not drag the caret to its end and
/// leave it there.
#[test]
fn the_column_survives_a_shorter_line_in_between() {
    let source = "aaaaaaaaaa\n\nbb\n\ncccccccccc\n";
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(8.0), line_y(0.0));
    assert_eq!(editor.document().selection().head(), ByteOffset::new(8));

    // Down through the blank line, the short line, and the blank line, to the long one.
    for _ in 0..4 {
        editor.move_caret(true, false);
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    }

    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(24),
        "column 8 of the last line, which starts at 16 — not its start, not its end"
    );
}

// ---------------------------------------------------------------------------
// Selection.
// ---------------------------------------------------------------------------

#[test]
fn dragging_selects_between_the_two_points() {
    let mut editor = Editor::new("hello world\n", Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(2.0), line_y(0.0));
    editor.drag_to(column(7.0), line_y(0.0));

    let selection = editor.document().selection();
    assert_eq!(selection.anchor(), ByteOffset::new(2));
    assert_eq!(selection.head(), ByteOffset::new(7));
    assert!(!selection.is_collapsed());
}

#[test]
fn a_selection_is_drawn_and_an_empty_one_is_not() {
    let mut editor = Editor::new("hello world\n", Grid::new());
    let plain = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    editor.click_at(column(2.0), line_y(0.0));
    editor.drag_to(column(7.0), line_y(0.0));
    let highlighted = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    assert!(
        highlighted > plain,
        "the highlight should add quads: {plain} then {highlighted}"
    );

    editor.click_at(column(2.0), line_y(0.0));
    assert_eq!(
        editor.build_frame(WIDTH, HEIGHT, 1.0).len(),
        plain,
        "collapsing it should draw exactly what it drew before"
    );
}

#[test]
fn a_selection_spanning_lines_is_drawn_on_each_of_them() {
    let source = "first\n\nsecond\n\nthird\n";
    let mut editor = Editor::new(source, Grid::new());
    let plain = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    editor.click_at(column(1.0), line_y(0.0));
    editor.drag_to(column(3.0), line_y(4.0));
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(
        quads.len() >= plain + 3,
        "expected a highlight per row: {plain} then {}",
        quads.len()
    );
}

/// Selecting is not editing. The byte-preservation invariant does not care what is highlighted.
#[test]
fn selecting_does_not_touch_the_buffer() {
    let source = "hello world\n\nsecond line\n";
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(column(1.0), line_y(0.0));
    editor.drag_to(column(4.0), line_y(2.0));
    editor.move_caret(true, true);
    editor.move_caret(false, true);

    assert_eq!(editor.document().to_string(), source);
}

/// The failure MDEDIT-23 exists to prevent: chrome shifts where the text is drawn, and the hit test
/// does not follow. With a tab strip open the document starts 30 pixels lower, so a click 30 pixels
/// lower has to reach the same byte it would have without one.
#[test]
fn a_click_follows_the_document_when_chrome_pushes_it_down() {
    let source = "hello world\n\nsecond line\n";

    let mut bare = Editor::new(source, Grid::new());
    let _ = bare.build_frame(WIDTH, HEIGHT, 1.0);
    bare.click_at(column(4.0), line_y(2.0));
    let without_chrome = bare.document().selection().head();

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("open.md");
    std::fs::write(&path, source).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut tabbed = Editor::with_workspace(workspace, Grid::new());
    tabbed.activate_saved_tab();
    let _ = tabbed.build_frame(WIDTH, HEIGHT, 1.0);

    // Same point in the text, which is STRIP_HEIGHT further down the window.
    tabbed.click_at(column(4.0), line_y(2.0) + STRIP_HEIGHT);
    assert_eq!(
        tabbed.document().selection().head(),
        without_chrome,
        "the click did not follow the document down"
    );
}

/// And the drawing moved by the same amount, which is the other half of the same claim.
#[test]
fn chrome_moves_the_glyphs_by_exactly_its_own_height() {
    let source = "hello world\n\nsecond line\n";

    let mut bare = Editor::new(source, Grid::new());
    let bare_quads = bare.build_frame(WIDTH, HEIGHT, 1.0);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("open.md");
    std::fs::write(&path, source).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut tabbed = Editor::with_workspace(workspace, Grid::new());
    tabbed.activate_saved_tab();
    let tabbed_quads = tabbed.build_frame(WIDTH, HEIGHT, 1.0);

    // The strip draws quads of its own at the top; the document's rows are the ones below it.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the strip is thirty pixels tall"
    )]
    let step = STRIP_HEIGHT.round() as i32;
    let bar_top = status_bar_top(HEIGHT);
    // A row near the foot moves down past the status bar and stops being drawn, which is the strip
    // taking room rather than the arithmetic going wrong — so the comparison is over the rows that
    // are still on screen afterwards.
    let moved: Vec<i32> = rows(&bare_quads)
        .into_iter()
        .map(|y| y + step)
        .filter(|y| *y < bar_top)
        .collect();
    let after = rows(&tabbed_quads);
    assert!(
        moved.iter().all(|y| after.contains(y)),
        "expected every row to move down by {step}: {moved:?} not all in {after:?}"
    );
}
