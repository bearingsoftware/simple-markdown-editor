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

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

/// The narrow table from the manual fixture: two columns, a one-character first column.
const DOC: &str = "| a | bb |\n| --- | --- |\n| 1 | 2 |\n";

/// Where the second column starts: the widest cell in column zero, then the gap.
fn second_column() -> f32 {
    ADVANCE + mdedit_app::table::COLUMN_GAP
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

/// Middle of visual row `row`, `x` pixels from the text's left edge.
fn at(x: f32, row: usize) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    (
        origin().0 + x,
        LINE_HEIGHT.mul_add(row as f32, origin().1) + LINE_HEIGHT / 2.0,
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
    let second = second_column();

    let (head, caret) = click(&mut editor, 2.0, 0);
    assert_eq!(&DOC[head..=head], "a");
    assert_eq!(caret, (0.0, 0.0), "header, first column");

    let (head, caret) = click(&mut editor, second + 2.0, 0);
    assert_eq!(&DOC[head..head + 2], "bb");
    assert_eq!(caret, (second, 0.0), "header, second column");

    let (head, caret) = click(&mut editor, 2.0, 1);
    assert_eq!(&DOC[head..=head], "1");
    assert_eq!(caret, (0.0, LINE_HEIGHT), "body, first column");

    let (head, caret) = click(&mut editor, second + 2.0, 1);
    assert_eq!(&DOC[head..=head], "2");
    assert_eq!(caret, (second, LINE_HEIGHT), "body, second column");
}

/// The delimiter row is not a row, so the row under the header is the body — not `| --- |`.
#[test]
fn the_row_below_the_header_is_the_body_row() {
    let mut editor = editor();
    let (head, _) = click(&mut editor, 2.0, 1);

    assert!(
        !DOC[head..].starts_with("---"),
        "the delimiter row is drawn as nothing and cannot be clicked into"
    );
}
