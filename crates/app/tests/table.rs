//! A table drawn as a grid (MDEDIT-126).
//!
//! Two halves, and both are testable without a window because `table.rs` takes laid-out cells rather
//! than a text engine:
//!
//! - **the model** — which bytes are cells, taken from the parse;
//! - **the placement** — where a cell's glyphs end up once the columns are sized.
//!
//! What no test here can see is whether the result looks like a table on Windows. That is the gap
//! `AGENTS.md` names, and it is why the drawing needs an eye as MDEDIT-73's rules did.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::{hit, table};
use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan, LayoutPos};
use mdedit_text::{FontId, FontSize, GlyphId, GlyphKey, Layout, LayoutLine, PositionedGlyph};

const TABLE: &str = "| Keys | Does |\n| --- | --- |\n| Ctrl+B | Bold |\n";

fn span(start: usize, end: usize) -> ByteSpan {
    ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end))
}

/// One cell's layout: a single glyph `width` wide, so a column's width is predictable.
fn cell(width: f32) -> Layout {
    cell_sized(width, 16.0, 12.0)
}

fn cell_sized(width: f32, height: f32, baseline: f32) -> Layout {
    Layout {
        lines: vec![LayoutLine {
            baseline,
            height,
            glyphs: vec![PositionedGlyph {
                key: GlyphKey {
                    font: FontId::new(0),
                    glyph: GlyphId::new(1),
                    size: FontSize::new(16.0),
                },
                pos: LayoutPos {
                    x: 0.0,
                    y: baseline,
                },
                advance: width,
                source: span(0, 1),
            }],
            rules: Vec::new(),
        }],
    }
}

/// Where each glyph on `row` sits, left to right.
fn xs(layout: &Layout, row: usize) -> Vec<f32> {
    layout.lines[row].glyphs.iter().map(|g| g.pos.x).collect()
}

// ---- the model ----

/// The cells come from the parse, not from splitting on pipes — which is the whole reason
/// `ENABLE_TABLES` had to come with this ticket rather than before it.
#[test]
fn a_pipe_table_yields_its_cells_row_by_row() {
    let doc = Document::from(TABLE);
    let rows = table::cells(&doc, span(0, TABLE.len()));

    assert_eq!(rows.len(), 2, "a header row and one body row: {rows:?}");
    assert_eq!(rows[0].len(), 2);
    assert_eq!(rows[1].len(), 2);

    let text = |s: ByteSpan| s.slice(TABLE).trim().to_owned();
    assert_eq!(text(rows[0][0]), "Keys");
    assert_eq!(text(rows[0][1]), "Does");
    assert_eq!(text(rows[1][0]), "Ctrl+B");
    assert_eq!(text(rows[1][1]), "Bold");
}

/// **The delimiter row is not a row of cells.** It is syntax describing the columns, and counting it
/// would put an empty row between the header and the body of every table in the editor.
#[test]
fn the_delimiter_row_is_not_one_of_the_rows() {
    let doc = Document::from(TABLE);
    let rows = table::cells(&doc, span(0, TABLE.len()));

    for row in &rows {
        for cell in row {
            assert!(
                !cell.slice(TABLE).contains("---"),
                "the delimiter row leaked into the cells: {rows:?}"
            );
        }
    }
}

/// Something that is not a table yields nothing, and the caller then draws it as ordinary lines.
/// A malformed table has to degrade rather than come out as an empty grid.
#[test]
fn prose_yields_no_cells() {
    let source = "just a paragraph with a | pipe in it\n";
    let doc = Document::from(source);
    assert!(table::cells(&doc, span(0, source.len())).is_empty());
}

// ---- the rows a click resolves against ----

/// One cell's layout whose glyph carries `source`, so `hit` sees the bytes it would really see.
fn cell_from(source: ByteSpan, width: f32) -> Layout {
    let mut layout = cell(width);
    layout.lines[0].glyphs[0].source = source;
    layout
}

/// The layout and the boxes for `source`, with its cells.
fn grid_of(source: &str) -> (Vec<Vec<ByteSpan>>, table::Grid, Layout) {
    let doc = Document::from(source);
    let cells = table::cells(&doc, span(0, source.len()));
    // A cell with no text lays out to a line and no glyphs, which is what a text engine really
    // returns for one — and the whole reason a grid cannot hit-test by walking glyphs.
    let rows: Vec<Vec<Layout>> = cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    if c.slice(source).trim().is_empty() {
                        Layout {
                            lines: vec![LayoutLine {
                                baseline: 12.0,
                                height: 16.0,
                                glyphs: Vec::new(),
                                rules: Vec::new(),
                            }],
                        }
                    } else {
                        cell_from(*c, 40.0)
                    }
                })
                .collect()
        })
        .collect();
    let (layout, boxes) = table::assemble(&rows, &cells, source.ends_with('\n'));
    (cells, boxes, layout)
}

/// A row covers its first cell to its last. The delimiter row has no span because it is not a row:
/// its bytes are syntax, and MDEDIT-118 has the caret step over concealed syntax, not into it.
#[test]
fn a_rows_span_runs_from_its_first_cell_to_its_last() {
    let (cells, boxes, _) = grid_of(TABLE);

    assert_eq!(boxes.rows.len(), 2, "the delimiter row is not one of them");
    assert_eq!(boxes.rows[0].start(), cells[0][0].start());
    assert_eq!(boxes.rows[0].end(), cells[0][1].end());
    assert_eq!(boxes.rows[1].start(), cells[1][0].start());
    assert_eq!(boxes.rows[1].end(), cells[1][1].end());
    assert_eq!(boxes.cells, cells, "and every cell is carried too");
}

/// **The second bug the first manual look found.** A grid is the one layout that **drops** a source
/// line — the delimiter row draws as nothing — so visual row 1 is source line 2, and the caret could
/// not be placed anywhere below the header.
///
/// **Its control is gone, and deliberately** (MDEDIT-183). This used to assert that the same chunk
/// with `grid: None` placed the offset on *no* row, which was the bug it was written against: `hit`
/// counted newlines, and a layout that drops a row defeats a count. `hit` now maps rows to source
/// lines through the glyphs, which gets a dropped row right for the same reason it gets a wrapped
/// one right — so the no-mapping chunk answers `(1, 10.0)` too, and an assertion that it answers
/// nothing would be asserting the old defect rather than this ticket's rule.
///
/// What the grid's spans are still needed for is the case the glyphs cannot speak to: a cell with
/// **no glyphs in it at all**. That is `an_empty_cell_can_be_clicked_into`, and it fails without
/// them.
#[test]
fn a_click_below_the_header_lands_in_the_row_it_was_aimed_at() {
    let (cells, boxes, grid) = grid_of(TABLE);
    let body = cells[1][0].start();

    let chunk = hit::Chunk {
        layout: &grid,
        text: TABLE,
        start: ByteOffset::ZERO,
        base: ByteOffset::ZERO,
        grid: Some(&boxes),
    };
    let (row, _) = hit::place_of(&chunk, 2, body).expect("the body row owns its own bytes");
    assert_eq!(row, 1, "the body row is visual row 1, not source line 2");
}

/// Clicking past the last column lands at the end of *that* row's text. Counting newlines would put
/// it at the end of the delimiter row, which is bytes the caret may not occupy.
#[test]
fn clicking_past_the_last_column_lands_at_the_end_of_that_row() {
    let (cells, boxes, grid) = grid_of(TABLE);

    let chunk = hit::Chunk {
        layout: &grid,
        text: TABLE,
        start: ByteOffset::ZERO,
        base: ByteOffset::ZERO,
        grid: Some(&boxes),
    };
    assert_eq!(hit::offset_in_row(&chunk, 1, 10_000.0), cells[1][1].end());
}

/// **The third bug a manual look found, and the one a glyph-walking hit test cannot fix.** An empty
/// cell has no glyphs, so there is nothing to land on: a click in a freshly inserted column carried
/// past it to whichever column had text next. A grid resolves the column from its boxes, so a cell
/// is clickable before anyone has typed in it.
#[test]
fn an_empty_cell_can_be_clicked_into() {
    let source = "| a |  | c |\n| --- | --- | --- |\n| 1 |  | 3 |\n";
    let (cells, boxes, grid) = grid_of(source);

    let chunk = hit::Chunk {
        layout: &grid,
        text: source,
        start: ByteOffset::ZERO,
        base: ByteOffset::ZERO,
        grid: Some(&boxes),
    };

    // Aimed at the middle column's box, which holds nothing.
    let middle = boxes.text_left(1);
    assert_eq!(
        hit::offset_in_row(&chunk, 0, middle),
        cells[0][1].end(),
        "the click lands in the empty cell, not in the one after it"
    );

    // And the caret it puts there has somewhere to be drawn.
    let (row, x) = hit::place_of(&chunk, 2, cells[0][1].end()).expect("the empty cell is a place");
    assert_eq!(row, 0);
    assert_eq!(x, middle, "at the empty cell's own left edge");
}

/// Every pixel of the table belongs to a column, so there is no gap between two boxes where a click
/// does nothing. MDEDIT-116 shipped that bug once in one dimension; this is the two-dimensional one.
#[test]
fn every_pixel_across_the_table_belongs_to_a_column() {
    let rows = vec![vec![cell(30.0), cell(50.0)]];
    let (_, boxes) = table::assemble(&rows, &[], false);

    let width = boxes.width();
    for step in 0_u8..64 {
        let x = width * f32::from(step) / 64.0;
        assert!(
            boxes.column_at(x) < 2,
            "x {x} fell outside both columns of a {width}-wide table"
        );
    }
    assert_eq!(
        boxes.column_at(width + 100.0),
        1,
        "past the end is the last"
    );
}

// ---- the placement ----

/// A column is as wide as its widest cell, over every row including the header. A ragged column is
/// the thing a reader's eye cannot follow.
#[test]
fn a_column_is_as_wide_as_its_widest_cell() {
    let rows = vec![vec![cell(30.0), cell(10.0)], vec![cell(80.0), cell(10.0)]];
    let (grid, boxes) = table::assemble(&rows, &[], false);

    // Column zero's box is the 80-wide cell from the second row, plus a padding either side.
    assert_eq!(boxes.columns, vec![100.0, 30.0]);

    // So column one's text starts past that box, in *both* rows, and each sits in from its edge.
    let first = table::PADDING_X;
    let second = 100.0 + table::PADDING_X;
    assert_eq!(xs(&grid, 0), vec![first, second]);
    assert_eq!(xs(&grid, 1), vec![first, second]);
}

/// The rules a table is drawn with: one edge per column boundary, and the outer two.
#[test]
fn a_column_edge_is_where_a_vertical_rule_goes() {
    let rows = vec![vec![cell(30.0), cell(50.0)]];
    let (_, boxes) = table::assemble(&rows, &[], false);

    assert_eq!(boxes.columns, vec![50.0, 70.0]);
    assert_eq!(
        boxes.column_edges(),
        vec![0.0, 50.0, 120.0],
        "two columns have three vertical rules, the outer two included"
    );
    assert_eq!(boxes.width(), 120.0);
}

/// **A cell's text sits inside its box.** Without the padding the border is drawn against the
/// glyphs, which is the difference between a table and a box round some words.
#[test]
fn text_sits_inside_the_cell_rather_than_on_its_border() {
    let rows = vec![vec![cell_sized(20.0, 16.0, 12.0)]];
    let (grid, boxes) = table::assemble(&rows, &[], false);

    assert_eq!(xs(&grid, 0), vec![table::PADDING_X]);
    assert_eq!(boxes.columns, vec![20.0 + table::PADDING_X * 2.0]);
    assert_eq!(
        grid.lines[0].height,
        16.0 + table::PADDING_Y * 2.0,
        "the row is its text plus a padding above and below"
    );
    assert_eq!(
        grid.lines[0].baseline,
        table::PADDING_Y + 12.0 - table::optical_lift(12.0),
        "the padding, then the ascent, less the lift that centres the ink (MDEDIT-154)"
    );
}

/// Cells of different heights sit on one baseline, or the row reads as a stagger.
#[test]
fn a_row_shares_one_baseline() {
    let rows = vec![vec![
        cell_sized(20.0, 16.0, 12.0),
        cell_sized(20.0, 24.0, 19.0),
    ]];
    let (grid, _) = table::assemble(&rows, &[], false);

    let line = &grid.lines[0];
    let baseline = table::PADDING_Y + 19.0 - table::optical_lift(19.0);
    assert_eq!(line.baseline, baseline, "the tallest cell sets it");
    assert_eq!(line.height, 24.0 + table::PADDING_Y * 2.0);
    assert!(
        line.glyphs.iter().all(|g| g.pos.y == baseline),
        "every glyph in the row sits on the row's baseline"
    );
}

/// **Rows stack down the block, and this shipped wrong to the first manual look.**
/// [`mdedit_text::LayoutLine::baseline`] is measured from the top of the *block*, and each cell is
/// laid out as a block of its own — so a row that takes its baseline straight from its cells gets
/// that cell's ascent, every row gets the same number, and the whole table draws as one overstruck
/// line. Every other test here used a single row and saw nothing.
#[test]
fn each_row_sits_below_the_one_before_it() {
    let rows = vec![
        vec![cell_sized(20.0, 16.0, 12.0)],
        vec![cell_sized(20.0, 24.0, 19.0)],
        vec![cell_sized(20.0, 16.0, 12.0)],
    ];
    let (grid, _) = table::assemble(&rows, &[], false);

    // Row boxes are 22, 30 and 22 tall — the text plus a padding either side — so each baseline is
    // the row's own ascent below everything above it.
    let pad = table::PADDING_Y;
    let baselines: Vec<f32> = grid.lines.iter().map(|line| line.baseline).collect();
    assert_eq!(
        baselines,
        vec![
            pad + 12.0 - table::optical_lift(12.0),
            22.0 + pad + 19.0 - table::optical_lift(19.0),
            52.0 + pad + 12.0 - table::optical_lift(12.0),
        ],
        "each row's own ascent, below the heights of the rows above it"
    );

    for (line, baseline) in grid.lines.iter().zip(&baselines) {
        assert!(
            line.glyphs.iter().all(|glyph| glyph.pos.y == *baseline),
            "a row's glyphs sit on that row's baseline, not the first row's"
        );
    }
}

/// **The bug this would otherwise ship with, silently.** A chunk whose bytes end in a newline gets a
/// trailing empty line from the text engine, and `Cached` drops the layout's last line to match. An
/// assembled layout without one therefore loses its **last table row** — from the height and from
/// the drawing both, with nothing to indicate it.
#[test]
fn a_table_ending_in_a_newline_keeps_a_line_for_it() {
    let rows = vec![vec![cell(20.0)], vec![cell(20.0)]];

    let (with, _) = table::assemble(&rows, &[], true);
    assert_eq!(
        with.lines.len(),
        3,
        "two rows and the empty line after them"
    );
    assert!(
        with.lines[2].glyphs.is_empty(),
        "the trailing line carries no glyphs"
    );

    let (without, _) = table::assemble(&rows, &[], false);
    assert_eq!(without.lines.len(), 2);
}

/// One line per table row, so a source line is still a visual row — which is what lets hit-testing,
/// caret movement and `row_selection` go on working unchanged over a grid.
#[test]
fn one_row_of_the_table_is_one_visual_line() {
    let rows = vec![vec![cell(20.0), cell(20.0)]; 4];
    let (grid, _) = table::assemble(&rows, &[], false);

    assert_eq!(grid.lines.len(), 4);
    for line in &grid.lines {
        assert_eq!(line.glyphs.len(), 2, "both cells are on the one line");
    }
}

/// Glyphs are pushed column by column, so the last one on a row is the rightmost. `Cached::new`
/// measures the pan clamp from exactly that assumption.
#[test]
fn the_last_glyph_on_a_row_is_the_rightmost() {
    let rows = vec![vec![cell(30.0), cell(50.0), cell(10.0)]];
    let (grid, _) = table::assemble(&rows, &[], false);

    let positions = xs(&grid, 0);
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "columns should be placed left to right: {positions:?}"
    );
}

/// A ragged table — a row with fewer cells than the header — places what it has and stops, rather
/// than panicking on the missing column.
#[test]
fn a_row_with_missing_cells_is_placed_and_not_a_panic() {
    let rows = vec![vec![cell(30.0), cell(30.0)], vec![cell(30.0)]];
    let (grid, _) = table::assemble(&rows, &[], false);

    assert_eq!(grid.lines.len(), 2);
    assert_eq!(xs(&grid, 1), vec![table::PADDING_X]);
}

#[test]
fn cell_width_is_the_right_edge_of_its_last_glyph() {
    assert_eq!(table::cell_width(&cell(42.0)), 42.0);
    assert_eq!(table::cell_width(&Layout::default()), 0.0);
}
