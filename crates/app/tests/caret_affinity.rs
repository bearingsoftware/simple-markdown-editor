//! Which of two visual rows the caret is drawn on at a soft-wrap boundary (MDEDIT-188).
//!
//! Where a line wraps, the end of one row and the start of its continuation are the **same byte**.
//! A byte offset alone therefore cannot say where to draw the caret, and the two gestures that
//! reach that byte want opposite answers: a click in the space right of the first row means the end
//! of *that* row, while Up from the row below aims at the continuation's left edge — which is the
//! same boundary — and means the start of the continuation.
//!
//! MDEDIT-183 had no affinity to consult and always took the later row. That was the better of two
//! wrong answers, because preferring the earlier one made Up skip a row, and its ticket said so.
//! These tests pin the third answer.
//!
//! # What they measure, and why not `wrapped_caret`'s helper
//!
//! The row the caret is **drawn** on, as a difference from the caret drawn at offset 0 — the same
//! caret-against-caret measurement `wrapped_caret.rs` explains, so the constant between a caret
//! quad and a glyph cancels and no test here states a pixel.
//!
//! But that file's `caret_row_now` cannot be used, and the reason is the whole subject of this one:
//! it re-reads the head and puts the caret back with `Selection::caret`, which is
//! [`Affinity::Downstream`]. Re-seating a caret that way *destroys* the thing under test. So the
//! reference is taken **before** the gesture and nothing touches the selection afterwards.
//!
//! # The byte is not supposed to move
//!
//! Every assertion about a row is paired with one about the offset, because the ticket's last
//! criterion is that none of this changes which byte the caret sits on. `the_same_byte_is_drawn_on_
//! two_different_rows` is that criterion stated directly: two gestures, one byte, two rows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::view::Theme;
use mdedit_document::Selection;
use mdedit_offsets::{Affinity, ByteOffset};

use crate::stub;
use stub::{Grid, LINE_HEIGHT};

/// Narrow enough that a sixty-word line wraps several times.
const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 600.0;

/// A line long enough to wrap several times at [`WIDTH`].
fn long() -> String {
    "word ".repeat(60)
}

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// The top of the caret drawn this frame.
///
/// The caret is the one quad in the caret colour taller than it is wide — the modified-tab dot
/// shares the colour, and a band of chrome in it would otherwise answer for a caret that had
/// stopped being drawn at all.
fn caret_top(editor: &mut Editor<Grid>) -> f32 {
    let caret = Theme::default().caret;
    let wanted = [caret.r, caret.g, caret.b, caret.a];
    let tops: Vec<f32> = editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted && quad.rect[3] > quad.rect[2])
        .map(|quad| quad.rect[1])
        .collect();
    assert_eq!(tops.len(), 1, "expected exactly one caret, got {tops:?}");
    tops[0]
}

/// The caret top for a caret on row 0, to measure the others against.
///
/// Offset 0 is on row 0 of any document, and [`Affinity::Downstream`] is unambiguous there: nothing
/// precedes row 0, so there is no earlier row for an affinity to choose instead.
fn reference(editor: &mut Editor<Grid>) -> f32 {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::ZERO));
    caret_top(editor)
}

/// Which row the caret is drawn on now, without touching the selection.
fn caret_row(editor: &mut Editor<Grid>, reference: f32) -> i64 {
    let top = caret_top(editor);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a row index in a 600 px window"
    )]
    {
        ((top - reference) / LINE_HEIGHT).round() as i64
    }
}

/// How many rows the document draws text on.
///
/// Bounded below by the content rect: the formatting bar's labels are drawn in the text colour too,
/// and counting one as a row of the document is the mistake `wrapped_caret.rs` records.
fn drawn_rows(editor: &mut Editor<Grid>) -> usize {
    let text = Theme::default().text;
    let wanted = [text.r, text.g, text.b, text.a];
    let top = editor.content_rect().top_edge();
    let mut tops: Vec<f32> = editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted && quad.rect[1] >= top)
        .map(|quad| quad.rect[1])
        .collect();
    tops.sort_by(f32::total_cmp);
    tops.dedup();
    tops.len()
}

/// The middle of document row `row`, in window coordinates.
///
/// **From the content rect, not from a glyph's top.** A glyph sits an ascent below its row's top, so
/// `glyph_top + LINE_HEIGHT / 2` is in the row *below* it — the error that made the first version of
/// `wrapped_caret.rs` click a row lower than it meant to everywhere.
fn row_middle(editor: &Editor<Grid>, row: usize) -> f32 {
    #[allow(clippy::cast_precision_loss, reason = "a row index under a hundred")]
    let rows = row as f32;
    LINE_HEIGHT.mul_add(rows, editor.content_rect().top_edge()) + LINE_HEIGHT / 2.0
}

/// A point in the empty space to the right of every row's text.
fn past_the_text(editor: &Editor<Grid>) -> f32 {
    let content = editor.content_rect();
    content.left_edge() + content.width() - 1.0
}

/// A point at the very left of the text.
fn at_the_left(editor: &Editor<Grid>) -> f32 {
    editor.content_rect().left_edge() + 1.0
}

fn head(editor: &Editor<Grid>) -> usize {
    editor.document().selection().head().get()
}

/// Asserts the fixture really wraps, so a test that depends on two rows sharing a byte cannot pass
/// by there being nothing to share.
fn assert_wraps(editor: &mut Editor<Grid>, source: &str) {
    let rows = drawn_rows(editor);
    let lines = source.lines().count();
    assert!(
        rows > lines,
        "the fixture must wrap: {rows} rows for {lines} source lines",
    );
}

/// **The defect.** A click in the whitespace right of a wrapped row leaves the caret at the end of
/// **that** row, not at the start of the next one.
#[test]
fn a_click_past_a_wrapped_rows_end_stays_on_that_row() {
    let source = long();
    let mut editor = editor(&source);
    assert_wraps(&mut editor, &source);
    let reference = reference(&mut editor);

    editor.click_at(past_the_text(&editor), row_middle(&editor, 0));

    assert_eq!(
        caret_row(&mut editor, reference),
        0,
        "the caret belongs to the row that was clicked",
    );
}

/// **The ticket, stated as one assertion.** The same byte, reached two ways, is drawn on two rows —
/// and it is the *same byte*, which is the criterion that none of this moves the caret's offset.
///
/// If the fixture stopped wrapping the two offsets would differ and this would fail saying so,
/// rather than passing on a document where the question cannot arise.
#[test]
fn the_same_byte_is_drawn_on_two_different_rows() {
    let source = long();
    let mut editor = editor(&source);
    assert_wraps(&mut editor, &source);
    let reference = reference(&mut editor);

    editor.click_at(past_the_text(&editor), row_middle(&editor, 0));
    let (ended, end_row) = (head(&editor), caret_row(&mut editor, reference));

    editor.click_at(at_the_left(&editor), row_middle(&editor, 1));
    let (started, start_row) = (head(&editor), caret_row(&mut editor, reference));

    assert_eq!(
        ended, started,
        "the end of row 0 and the start of row 1 are one byte",
    );
    assert_eq!(end_row, 0, "clicked past row 0, so drawn on row 0");
    assert_eq!(start_row, 1, "clicked on row 1, so drawn on row 1");
}

/// A click at the start of a continuation row stays on it — the other half of the tie-break, and
/// what reds if the affinity is applied to every click rather than to one past a row's end.
#[test]
fn a_click_at_the_start_of_a_continuation_row_stays_on_it() {
    let source = long();
    let mut editor = editor(&source);
    assert_wraps(&mut editor, &source);
    let reference = reference(&mut editor);

    editor.click_at(at_the_left(&editor), row_middle(&editor, 2));

    assert_eq!(caret_row(&mut editor, reference), 2);
}

/// **Down from a caret clicked past a wrapped row's end moves one row, not two.**
///
/// This is the half of the defect the ticket does not mention. Up and Down aim from where
/// `caret_place` says the caret is, so while the caret at a boundary was always placed on the later
/// row, Down out of it started a row lower than the caret looked and skipped one.
#[test]
fn down_from_the_end_of_a_wrapped_row_moves_one_row() {
    let source = long();
    let mut editor = editor(&source);
    assert_wraps(&mut editor, &source);
    let reference = reference(&mut editor);

    editor.click_at(past_the_text(&editor), row_middle(&editor, 0));
    assert_eq!(caret_row(&mut editor, reference), 0, "clicked row 0");

    editor.move_caret(true, false);

    assert_eq!(
        caret_row(&mut editor, reference),
        1,
        "one row down, not two"
    );
}

/// And Up from it reaches the row above rather than the row it was already on.
///
/// **Row 1, and both rows asserted absolutely.** The first version of this clicked a row of a
/// fixture it had miscounted and asserted only that Up moved *one* row from wherever the click had
/// landed — which is true whichever row the caret was misplaced on, so it passed with the affinity
/// tie-break reverted. A relative assertion cannot see a defect that moves both of its terms.
///
/// Up out of row 1 of one wrapped paragraph must reach row 0. With the caret misplaced on row 2 it
/// reached row 1 instead, and the caret appeared not to move at all.
#[test]
fn up_from_the_end_of_a_wrapped_row_reaches_the_row_above() {
    let source = long();
    let mut editor = editor(&source);
    assert_wraps(&mut editor, &source);
    let reference = reference(&mut editor);

    editor.click_at(past_the_text(&editor), row_middle(&editor, 1));
    assert_eq!(caret_row(&mut editor, reference), 1, "clicked row 1");

    editor.move_caret(false, false);

    assert_eq!(caret_row(&mut editor, reference), 0, "Up reached row 0");
}

/// **A hard line ending needs no affinity, and must not be given one.** The newline separates the
/// two rows' bytes, so only one row owns each and both answers agree.
///
/// Clicking past the end of a short line puts the caret on that line's row, and the byte is that
/// line's end rather than the start of the next.
#[test]
fn a_click_past_a_hard_line_ending_stays_on_its_own_row() {
    let source = "first\nsecond\nthird\n";
    let mut editor = editor(source);
    let reference = reference(&mut editor);

    editor.click_at(past_the_text(&editor), row_middle(&editor, 1));

    assert_eq!(caret_row(&mut editor, reference), 1);
    assert_eq!(
        head(&editor),
        source.find('\n').unwrap() + 1 + "second".len(),
        "the end of the second line's text, before its newline",
    );
}

/// Typing clears the affinity, because a keystroke cannot know which row it came from — and the
/// caret must not stay pinned to a row the edit may have moved.
#[test]
fn typing_clears_the_affinity() {
    let source = long();
    let mut editor = editor(&source);
    assert_wraps(&mut editor, &source);

    editor.click_at(past_the_text(&editor), row_middle(&editor, 0));
    assert_eq!(
        editor.document().selection().affinity(),
        Affinity::Upstream,
        "the click set it",
    );

    editor.insert_text("x");

    assert_eq!(
        editor.document().selection().affinity(),
        Affinity::Downstream,
    );
}

/// An affinity is part of a caret's identity, so moving between the two rows of a boundary ends the
/// undo group — the caret moved, visibly, even though the offset did not.
#[test]
fn crossing_the_boundary_ends_the_undo_group() {
    let a = Selection::caret(ByteOffset::new(7));
    let b = a.with_affinity(Affinity::Upstream);
    assert_ne!(a, b, "same byte, different row, different caret");
    assert_eq!(a.head(), b.head(), "and the byte really is the same");
    assert_eq!(a.affinity(), Affinity::Downstream, "the default");
}
