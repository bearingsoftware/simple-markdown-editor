//! Clicking inside a table drawn as a grid (MDEDIT-126).
//!
//! `tests/table.rs` covers the assembler and the row mapping in isolation. This drives the whole
//! editor — parse, chunk, lay out, hit-test, draw — because both bugs the first manual look found
//! were in the joins between those, where a unit test on either side of a join saw nothing.
//!
//! **The invariant is that the two agree**: the byte a click resolves to, and the place the caret is
//! then drawn. Either alone can be right while the pair is useless, which is exactly what shipped —
//! clicks landed on the correct byte and the caret was drawn on the trailing empty row.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::view::Theme;

use crate::stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

/// The narrow table from the manual fixture: two columns, a one-character first column.
const DOC: &str = "| a | bb |\n| --- | --- |\n| 1 | 2 |\n";

/// Where the text of column zero starts: in from its box's left border.
fn first_column() -> f32 {
    mdedit_app::table::PADDING_X
}

/// Where the text of column one starts: past column zero's whole box, then in from its own border.
/// Column zero holds a single character in every row.
fn second_column() -> f32 {
    ADVANCE + mdedit_app::table::PADDING_X * 3.0
}

/// How tall a ruled row is: a line of text with a padding above and below.
fn row_height() -> f32 {
    LINE_HEIGHT + mdedit_app::table::PADDING_Y * 2.0
}

fn editor() -> Editor<Grid> {
    let mut editor = Editor::new(DOC, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// The top-left of the text, in window coordinates.
fn origin() -> (f32, f32) {
    (MARGIN, MARGIN + mdedit_app::toolbar::HEIGHT)
}

/// Middle of ruled row `row`, `x` pixels from the table's left edge.
fn at(x: f32, row: usize) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    (
        origin().0 + x,
        row_height().mul_add(row as f32, origin().1) + row_height() / 2.0,
    )
}

/// Where the caret is drawn, as an offset from the top-left of the text.
fn caret_at(editor: &mut Editor<Grid>) -> (f32, f32) {
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let caret = Theme::default().caret;
    let rect = quads
        .iter()
        .find(|quad| {
            quad.colour[0] == caret.r
                && quad.colour[1] == caret.g
                && quad.colour[2] == caret.b
                && quad.colour[3] == caret.a
        })
        .map(|quad| quad.rect)
        .expect("the caret is drawn");
    (rect[0] - origin().0, rect[1] - origin().1)
}

/// Clicks at `x` on visual `row` and reports the byte it landed on and where the caret went.
fn click(editor: &mut Editor<Grid>, x: f32, row: usize) -> (usize, (f32, f32)) {
    let (px, py) = at(x, row);
    editor.click(px, py);
    (editor.document().selection().head().get(), caret_at(editor))
}

/// **The bug the first manual look found, in the join.** The click resolved to the right byte all
/// along; the caret was drawn by a *second* newline count, which a grid breaks because it drops the
/// delimiter row. Every cell below the header put the caret on the trailing empty line instead.
#[test]
fn the_caret_is_drawn_in_the_cell_that_was_clicked() {
    let mut editor = editor();
    let (first, second) = (first_column(), second_column());

    let (head, caret) = click(&mut editor, first + 2.0, 0);
    assert_eq!(&DOC[head..=head], "a");
    assert_eq!(caret, (first, 0.0), "header, first column");

    let (head, caret) = click(&mut editor, second + 2.0, 0);
    assert_eq!(&DOC[head..head + 2], "bb");
    assert_eq!(caret, (second, 0.0), "header, second column");

    let (head, caret) = click(&mut editor, first + 2.0, 1);
    assert_eq!(&DOC[head..=head], "1");
    assert_eq!(caret, (first, row_height()), "body, first column");

    let (head, caret) = click(&mut editor, second + 2.0, 1);
    assert_eq!(&DOC[head..=head], "2");
    assert_eq!(caret, (second, row_height()), "body, second column");
}

/// The quads drawn in [`Theme::line`] — the table's rules and the wash behind its header.
fn rules(editor: &mut Editor<Grid>) -> Vec<[f32; 4]> {
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let line = Theme::default().line;
    quads
        .iter()
        .filter(|quad| {
            quad.colour[0] == line.r
                && quad.colour[1] == line.g
                && quad.colour[2] == line.b
                && quad.colour[3] == line.a
        })
        .map(|quad| quad.rect)
        .collect()
}

/// A table is ruled: a line at every row boundary, one at every column boundary, and a wash behind
/// the header. Two columns and two rows means three vertical rules and three horizontal ones.
#[test]
fn a_table_is_ruled_around_every_cell() {
    let mut editor = editor();
    let drawn = rules(&mut editor);

    let width = second_column() + ADVANCE * 2.0 + mdedit_app::table::PADDING_X;
    let (left, top) = origin();

    let wash = drawn
        .iter()
        .find(|rect| rect[3] == row_height())
        .expect("the header row has a wash behind it");
    assert_eq!(
        (wash[0], wash[1], wash[2]),
        (left, top, width),
        "the wash covers the header row, the full width of the table"
    );

    let horizontal: Vec<f32> = drawn
        .iter()
        .filter(|rect| rect[2] == width && rect[3] != row_height())
        .map(|rect| rect[1] - top)
        .collect();
    assert_eq!(
        horizontal,
        vec![0.0, row_height(), row_height() * 2.0],
        "above the header, between the rows, and under the last"
    );

    let vertical: Vec<f32> = drawn
        .iter()
        .filter(|rect| rect[3] == row_height() * 2.0)
        .map(|rect| rect[0] - left)
        .collect();
    assert_eq!(
        vertical,
        vec![0.0, ADVANCE + mdedit_app::table::PADDING_X * 2.0, width],
        "both outer edges and the one between the columns"
    );
}

/// **`Ctrl+/` rules nothing.** Plain mode exists to show the bytes, and the bytes of a table are
/// pipe rows — a border round them would be the mode failing at the one thing it is for.
#[test]
fn plain_mode_draws_no_rules() {
    let mut editor = editor();
    assert!(!rules(&mut editor).is_empty(), "ruled in preview");

    editor.toggle_mode();
    assert!(rules(&mut editor).is_empty(), "and not in plain");
}

/// The delimiter row is not a row, so the row under the header is the body — not `| --- |`.
#[test]
fn the_row_below_the_header_is_the_body_row() {
    let mut editor = editor();
    let (head, _) = click(&mut editor, first_column() + 2.0, 1);

    assert!(
        !DOC[head..].starts_with("---"),
        "the delimiter row is drawn as nothing and cannot be clicked into"
    );
}
