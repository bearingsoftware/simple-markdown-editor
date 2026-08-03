//! Word wrap off, and the horizontal scrolling that had to exist first (MDEDIT-49).
//!
//! The toggle is the small half. The real content of this ticket is that **a second axis exists**,
//! and every place that converted between document and window coordinates had quietly assumed there
//! was only one — so the tests that matter are the round-trips at a nonzero pan, not the toggle.
//!
//! Wrap-on behaviour must be unchanged, byte for byte of rendering, and that is asserted here too:
//! this feature is only worth having if it costs the default nothing.

// `float_cmp`: the pan offsets compared here are values these tests set themselves — zero, or a
// multiple of the stub's fixed advance — so an exact comparison is the assertion, not an accident.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_possible_truncation
)]

use mdedit_app::content::Offset;
use mdedit_app::editor::Editor;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// A line far wider than the viewport, so there is something off to the right to reach.
fn long_line() -> String {
    format!("{}\n", "abcdefghij".repeat(40))
}

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// The window position of character `column` on row `row`.
fn at(editor: &Editor<Grid>, column: usize, row: usize) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    (
        ADVANCE.mul_add(column as f32, editor.content_rect().left_edge()) + ADVANCE / 2.0,
        editor
            .content_rect()
            .to_window(0.0, 0.0, Offset::down(0.0))
            .1
            + LINE_HEIGHT.mul_add(row as f32, LINE_HEIGHT / 2.0),
    )
}

// ---- the toggle ----

#[test]
fn wrap_is_on_to_begin_with() {
    assert!(editor("hello\n").wrap());
}

#[test]
fn toggling_reports_whether_anything_changed() {
    let mut editor = editor("hello\n");
    assert!(!editor.set_wrap(true));
    assert!(editor.set_wrap(false));
    assert!(!editor.set_wrap(false));
    assert!(editor.toggle_wrap());
    assert!(editor.wrap());
}

/// **Wrap on is what it always was.** A long line occupies several rows, and turning wrap off
/// collapses it to one — asserted by where a click on the *second* row lands, because the quad count
/// does not change: the same glyphs are drawn either way, just arranged differently. That was my
/// first guess and it was wrong.
#[test]
fn wrap_off_puts_a_long_line_on_one_row() {
    let text = long_line();

    let mut wrapped = editor(&text);
    let (x, y) = at(&wrapped, 2, 1);
    wrapped.click(x, y);
    let on_second_row = wrapped.document().selection().head();
    assert!(
        on_second_row.get() > 0 && on_second_row.get() < text.len() - 1,
        "with wrap on, the second row is the middle of the line"
    );

    let mut unwrapped = editor(&text);
    assert!(unwrapped.set_wrap(false));
    unwrapped.build_frame(WIDTH, HEIGHT, 1.0);
    unwrapped.click(x, y);

    assert_eq!(
        unwrapped.document().selection().head().get(),
        text.len(),
        "with wrap off there is no second row, so the click falls past the end"
    );
}

/// Toggling does not touch the buffer, which is the one thing a view toggle must never do.
#[test]
fn toggling_does_not_touch_the_document() {
    let text = long_line();
    let mut editor = editor(&text);

    assert!(editor.set_wrap(false));
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(editor.set_wrap(true));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.document().to_string(), text);
    assert!(!editor.document().can_undo());
}

// ---- panning ----

#[test]
fn there_is_nothing_to_pan_while_wrapping() {
    let mut editor = editor(&long_line());
    assert!(!editor.pan_by(200.0), "wrapping has no sideways to go");
    assert_eq!(editor.pan(), 0.0);
}

#[test]
fn panning_moves_right_and_clamps_at_both_ends() {
    let mut editor = editor(&long_line());
    assert!(editor.set_wrap(false));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(editor.pan_by(120.0));
    assert_eq!(editor.pan(), 120.0);

    // Back past the start stops at the start rather than going negative.
    assert!(editor.pan_by(-500.0));
    assert_eq!(editor.pan(), 0.0);

    // And forward past the end stops at the widest row less a viewport.
    editor.pan_by(100_000.0);
    let far = editor.pan();
    assert!(far > 0.0, "there should have been somewhere to go");
    editor.pan_by(100_000.0);
    assert_eq!(editor.pan(), far, "already at the end");
}

/// A document with nothing wider than the viewport has nowhere to pan, even with wrap off — the
/// same rule the vertical axis uses, and for the same reason: an offset with nothing to reveal
/// slides the text off into blank space.
#[test]
fn a_short_document_does_not_pan() {
    let mut editor = editor("short\n");
    assert!(editor.set_wrap(false));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.pan_by(500.0);
    assert_eq!(editor.pan(), 0.0);
}

/// Turning wrap back on returns the pan to zero. Leaving it would show a document scrolled past its
/// own right edge, with no way to scroll back.
#[test]
fn turning_wrap_back_on_returns_to_the_left() {
    let mut editor = editor(&long_line());
    editor.set_wrap(false);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.pan_by(200.0);
    assert!(editor.pan() > 0.0);

    editor.set_wrap(true);
    assert_eq!(editor.pan(), 0.0);
}

// ---- the round-trips, which are the point ----

/// **The drift test, on the axis that did not have one.** A click must land on the same byte whether
/// or not the viewport has panned — which is only true if every conversion goes through
/// `ContentRect`, and is exactly what the old bare-`f32` scroll parameter let callers get wrong.
#[test]
fn clicks_land_correctly_at_a_nonzero_pan() {
    let text = long_line();
    let mut editor = editor(&text);
    editor.set_wrap(false);
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    // Where character 10 is with no pan.
    let (x, y) = at(&editor, 10, 0);
    editor.click(x, y);
    let unpanned = editor.document().selection().head();
    assert_eq!(unpanned, ByteOffset::new(11));

    // Pan by exactly five characters, then click five characters further left: the same byte.
    editor.pan_by(ADVANCE * 5.0);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let (x, y) = at(&editor, 5, 0);
    editor.click(x, y);

    assert_eq!(
        editor.document().selection().head(),
        unpanned,
        "the same character is under a point five columns to the left"
    );
}

/// And the inverse: a click at the same screen point lands five characters further along.
/// **Two editors, not two clicks.** Clicking the same point twice in a row is a *double* click, and
/// the second one selects a word rather than placing a caret — which is what this test measured the
/// first time it was written, and it reported the end of the line.
#[test]
fn a_pan_shifts_which_byte_is_under_a_point() {
    let text = long_line();

    let mut still = editor(&text);
    still.set_wrap(false);
    still.build_frame(WIDTH, HEIGHT, 1.0);
    let (x, y) = at(&still, 10, 0);
    still.click(x, y);

    let mut panned = editor(&text);
    panned.set_wrap(false);
    panned.build_frame(WIDTH, HEIGHT, 1.0);
    panned.pan_by(ADVANCE * 5.0);
    panned.build_frame(WIDTH, HEIGHT, 1.0);
    panned.click(x, y);

    assert_eq!(
        panned.document().selection().head().get(),
        still.document().selection().head().get() + 5,
        "the document moved left under a stationary pointer"
    );
}

/// Following the caret reveals sideways as well as down. Typing at the end of a long line with wrap
/// off would otherwise put the caret off the right edge, which is the failure the vertical reveal
/// has always existed to prevent.
#[test]
fn the_caret_is_revealed_sideways() {
    let mut editor = editor(&long_line());
    editor.set_wrap(false);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(editor.pan(), 0.0);

    // The end of the line, which is far off the right of a 600-pixel viewport.
    let end = ByteOffset::new(editor.document().len_bytes() - 1);
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::caret(end));
    editor.insert_text("!");
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(
        editor.pan() > 0.0,
        "the caret was off the right edge and nothing followed it"
    );
}

/// **A pan survives the frame it asks for, even with the caret reveal armed.** This is the bug the
/// window found and no test had: Shift+wheel is the only way to pan, Shift is a key press, and a key
/// press re-arms `follow_caret` — so every pan was immediately revealed back to the caret at the left
/// edge. Nothing moved, on any document, ever, while `pan_by` itself was provably correct.
#[test]
fn a_pan_is_not_undone_by_the_caret_reveal() {
    let mut editor = editor(&long_line());
    editor.set_wrap(false);
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    // What holding Shift does before the wheel notch arrives.
    editor.follow_caret();
    assert!(editor.pan_by(200.0));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(
        editor.pan() > 0.0,
        "the frame revealed the caret and threw the pan away"
    );
}

/// The gutter does not pan with the text. It is chrome to the left of the document, not part of it —
/// and a line number sliding off the left edge is the sort of thing that is obvious in a screenshot
/// and invisible to every other test.
#[test]
fn the_gutter_stays_put_while_the_text_pans() {
    let mut editor = editor(&long_line());
    editor.toggle_line_numbers();
    editor.set_wrap(false);
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let gutter_before = leftmost_quad(&before);

    editor.pan_by(200.0);
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(
        leftmost_quad(&after),
        gutter_before,
        "the line number moved with the document"
    );
}

/// The x of the leftmost quad on screen, which with line numbers on is the gutter's digit.
fn leftmost_quad(quads: &[mdedit_app::gpu::QuadInstance]) -> Option<i32> {
    quads
        .iter()
        .map(|quad| quad.rect[0])
        .filter(|x| *x >= 0.0)
        .map(|x| (x * 100.0) as i32)
        .min()
}
