//! Where the caret goes in a chunk whose earlier lines have wrapped (MDEDIT-183).
//!
//! A list item or a quoted line is one chunk holding several source lines, and wrapping adds visual
//! rows **in the middle** of it. `hit` used to find a row's bytes by counting newlines, which is
//! only right for rows before the first wrap — so the caret for every later line was drawn as many
//! rows too high as the wrapping had added.
//!
//! # What these assert, and why it is the drawn caret rather than the mapping
//!
//! The row the caret is *drawn* on, compared against the row that line's own glyphs are drawn on.
//! Both come out of one frame, so neither side is a number this file owns: a test that asserted
//! "row 142" would be restating the stub's metrics, and would pass just as happily if both the
//! caret and the text moved. What is wrong here is the *relationship* between them.
//!
//! Every y is a multiple of [`stub::LINE_HEIGHT`] away from its neighbour, never a literal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::view::Theme;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::{Grid, LINE_HEIGHT};

/// Narrow enough that a sixty-word line wraps, wide enough that a short one does not.
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

fn caret_at(editor: &mut Editor<Grid>, offset: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(offset)));
}

/// The top of the caret drawn this frame.
///
/// The caret is the one quad in the caret colour that is taller than it is wide — the modified-tab
/// dot shares the colour in the default theme, and a row of chrome in it would otherwise answer for
/// a caret that had stopped being drawn.
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

/// Which row the caret is drawn on for a caret at `offset`, counting from the first row.
///
/// **Caret measured against caret, never against a glyph.** A caret quad is taller than a glyph and
/// starts above it, so the two differ by a constant that has nothing to do with rows. The difference
/// between two caret positions cancels it, and dividing by [`LINE_HEIGHT`] is then a row count
/// rather than a pixel assertion. The reference is offset 0, which is on row 0 of any document —
/// including a broken one, since nothing about row 0 depends on the mapping under test.
fn caret_row(editor: &mut Editor<Grid>, offset: usize) -> i64 {
    caret_at(editor, 0);
    let first = caret_top(editor);
    caret_at(editor, offset);
    let top = caret_top(editor);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a row index in a 600 px window"
    )]
    {
        ((top - first) / LINE_HEIGHT).round() as i64
    }
}

/// Which row the caret is on now, without moving it — for after an Up or a Down.
fn caret_row_now(editor: &mut Editor<Grid>) -> i64 {
    let top = caret_top(editor);
    let head = editor.document().selection().head().get();
    let row = caret_row(editor, head);
    // Put it back where the caller left it, and check the two agree: `caret_row` moves the caret to
    // measure, so a disagreement would mean the move itself changed where it was drawn.
    caret_at(editor, head);
    assert_eq!(caret_top(editor), top, "measuring moved the caret");
    row
}

/// The top of every row of the **document** that has text on it, in order.
///
/// Bounded below by the content rect, because the formatting bar's labels are drawn in the text
/// colour too — and counting one of those as a row of the document is exactly the mistake that made
/// the first version of these tests expect a row that was not there.
fn glyph_rows(editor: &mut Editor<Grid>) -> Vec<f32> {
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
    tops
}

/// How many rows the document draws text on.
fn drawn_rows(editor: &mut Editor<Grid>) -> usize {
    glyph_rows(editor).len()
}

/// A row index as [`caret_row`] reports one.
///
/// Signed, because a caret drawn *above* row 0 is a failure worth seeing as a negative number
/// rather than as a vast unsigned one — which is how the first run of these read.
fn row(index: usize) -> i64 {
    i64::try_from(index).expect("a row index in a 600 px window fits in an i64")
}

/// The middle of document row `row`, in window coordinates, for a click.
///
/// **Measured from the content rect, not from where a glyph was drawn.** A glyph sits below its
/// row's top by the font's ascent — fifteen of the stub's twenty pixels — so `glyph_top +
/// LINE_HEIGHT / 2` is in the row *below* the glyph, and the first version of this test clicked a
/// row lower than it meant to everywhere. The rows are uniform here, so the row's own top is the
/// content's top plus whole rows.
fn row_middle(editor: &Editor<Grid>, row: usize) -> f32 {
    #[allow(clippy::cast_precision_loss, reason = "a row index under a hundred")]
    let rows = row as f32;
    LINE_HEIGHT.mul_add(rows, editor.content_rect().top_edge()) + LINE_HEIGHT / 2.0
}

/// **The defect.** The items after a wrapped first one: each caret must be drawn on the row that
/// item's own text is on, not on one of the rows the wrap added above it.
///
/// **Three items, not two, and that is what makes this test worth having.** With two, the line after
/// the wrap is the *last* line — and a mapping that merely clamps a row to the last source line
/// lands on it by accident, which a hand-mutation proved: the two-item version passed with the row
/// mapping replaced by the row's own index. A middle item has a wrong answer available on both
/// sides.
#[test]
fn the_caret_sits_on_the_list_items_after_the_first_wrapped() {
    let source = format!("- {}\n- middle\n- last\n", long());
    let mut editor = editor(&source);
    let rows = drawn_rows(&mut editor);
    assert!(
        rows >= 4,
        "the first item has to wrap for this to mean anything: {rows} rows"
    );

    // The two short items are the last two rows; the long one wrapped into every row above them.
    for (needle, expected) in [("middle", rows - 2), ("last", rows - 1)] {
        let offset = source.find(needle).expect("the item is there");
        assert_eq!(
            caret_row(&mut editor, offset),
            row(expected),
            "the caret for `{needle}` is not on the row that item is drawn on",
        );
    }
}

/// The same for a block quote, which is the other chunk that holds several source lines — and with
/// a middle line for the reason the list's version gives.
#[test]
fn the_caret_sits_on_the_quoted_lines_after_the_first_wrapped() {
    let source = format!("> {}\n> middle\n> last\n", long());
    let mut editor = editor(&source);
    let rows = drawn_rows(&mut editor);
    assert!(rows >= 4, "the first line has to wrap: {rows} rows");

    for (needle, expected) in [("middle", rows - 2), ("last", rows - 1)] {
        let offset = source.find(needle).expect("the line is there");
        assert_eq!(
            caret_row(&mut editor, offset),
            row(expected),
            "the caret for `{needle}` is not on the row that quoted line is drawn on",
        );
    }
}

/// A wrapped paragraph is one source line and several rows, so nothing about newline counting was
/// ever right for it either: the caret in its second half belongs on its second row.
#[test]
fn the_caret_sits_on_the_row_it_is_on_inside_a_wrapped_paragraph() {
    let source = long();
    let mut editor = editor(&source);
    let rows = drawn_rows(&mut editor);
    assert!(rows >= 3, "the paragraph has to wrap: {rows} rows");

    // The last byte of the paragraph is on its last row.
    assert_eq!(
        caret_row(&mut editor, source.trim_end().len()),
        row(rows - 1),
        "the caret should be on the paragraph's last row",
    );
}

/// Up from the line after a wrap goes to the wrapped line's **last** row, not to its first.
#[test]
fn up_from_after_a_wrap_lands_on_the_last_wrapped_row() {
    let source = format!("> {}\n> next\n", long());
    let mut editor = editor(&source);
    let rows = drawn_rows(&mut editor);
    let offset = source.rfind("next").expect("the line is there");
    caret_at(&mut editor, offset);
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.move_caret(false, false);
    assert_eq!(
        caret_row_now(&mut editor),
        row(rows - 2),
        "up should land on the row directly above, which is the wrap's last row",
    );
}

/// A click on a blank quoted line after a wrap puts the caret on that line, rather than on whatever
/// byte counting newlines happened to reach.
#[test]
fn a_click_on_a_blank_quoted_row_after_a_wrap_lands_on_that_line() {
    let source = format!("> {}\n>\n> next\n", long());
    let mut editor = editor(&source);
    // **The blank quoted row draws no glyphs**, so it is not one of the rows with text on them — it
    // shows up as the one gap in them, between the wrap's last row and `next`'s. Found that way
    // rather than counted, because how many rows the wrap took is the stub's business.
    let rows = glyph_rows(&mut editor);
    let gap = rows
        .windows(2)
        .position(|pair| pair[1] - pair[0] > LINE_HEIGHT * 1.5)
        .expect("a blank quoted row leaves a gap between two rows of text");

    let x = editor.content_rect().left_edge() + 40.0;
    editor.click(x, row_middle(&editor, gap + 1));

    let head = editor.document().selection().head().get();
    let blank_line = source.find("\n>\n").expect("the blank line is there") + 1;
    assert!(
        (blank_line..blank_line + 2).contains(&head),
        "the caret landed at {head}, not on the blank quoted line at {blank_line}",
    );
}

/// A hard line break makes one chunk out of several source lines without a list or a quote being
/// involved, which the ticket flagged as "probably also" affected. It is, for the same reason, and
/// the rule is the same: the line after the wrap gets the row its own text is drawn on.
#[test]
fn the_caret_sits_on_the_line_after_a_hard_break_in_a_wrapped_paragraph() {
    let source = format!("{}  \nnext line\n", long());
    let mut editor = editor(&source);
    let rows = drawn_rows(&mut editor);
    assert!(rows >= 3, "the first line has to wrap: {rows} rows");

    let offset = source.rfind("next").expect("the second line is there");
    assert_eq!(
        caret_row(&mut editor, offset),
        row(rows - 1),
        "the caret is not on the row the line after the break is drawn on",
    );
}
