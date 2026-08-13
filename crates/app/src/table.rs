//! A pipe table, assembled into a grid out of the layouts of its cells (MDEDIT-126).
//!
//! Geometry and vocabulary only — no GPU types, no text engine, no window — as `toolbar.rs`,
//! `popup.rs`, `tabs.rs` and `pane.rs` are. The caller lays each cell out and hands the results
//! here; this module decides where they go. That split is what makes a grid testable without a
//! window, and it is also what the borrow checker wants: laying a cell out needs `&mut` the engine
//! while finding the cells needs `&` the document, and they are the same `Editor`.
//!
//! # Why this is not the widget architecture `AGENTS.md` describes
//!
//! It says tables are *model-backed block widgets* because "pure decoration over a linear character
//! stream cannot do 2D layout". The conclusion is right and the premise turned out not to be:
//! [`Layout`] is a `Vec<LayoutLine>` of glyphs that each carry their own position and source span,
//! and nothing in it requires those positions to march left to right. The linearity is a property of
//! what a text engine hands back, not of the type.
//!
//! So a table does not need a second layout type, a second draw path or a second caret model. It
//! needs its cells laid out apart and **placed**, which is what this does. `draw_chunks`, the layout
//! cache and `row_selection` keep working unchanged, because the glyphs they read still carry
//! truthful source spans and one table row is still one visual row.
//!
//! **One thing did have to give.** `hit.rs` finds a row's bytes by counting newlines, which is sound
//! everywhere else because wrapping only ever *adds* rows. A grid **drops** one — the delimiter row
//! draws as nothing — so it hands `hit` the row spans instead ([`row_spans`]). That is a mapping,
//! not a second hit test: the same `offset_in_row` and `place_of` answer for both.
//!
//! **What this does not buy** is a cell that wraps. A column is as wide as its widest cell, and a
//! table wider than the window pans rather than reflows — see [`assemble`].

use mdedit_document::Document;
use mdedit_markers::BlockKind;
use mdedit_offsets::{ByteSpan, LayoutPos};
use mdedit_text::{Layout, LayoutLine};

/// Space between one column's text and the next column's.
///
/// Stands in for the pipe that is no longer drawn. Without it the columns touch and the grid reads
/// as one run of words again, which is the failure the whole ticket exists to fix.
pub const COLUMN_GAP: f32 = 20.0;

/// The cells of the table at `table`, row by row, in document order.
///
/// Empty when there is nothing here that parses as a table — the caller then lays the chunk out as
/// ordinary lines, which is what a malformed table should look like.
#[must_use]
pub fn cells(doc: &Document, table: ByteSpan) -> Vec<Vec<ByteSpan>> {
    let inside = |span: ByteSpan| span.start() >= table.start() && span.end() <= table.end();

    let mut rows: Vec<Vec<ByteSpan>> = Vec::new();
    // Blocks are outermost-first in document order, so a row is opened by its own block and every
    // cell block after it belongs to it until the next row opens. No nesting stack needed.
    for block in doc.blocks() {
        if !inside(block.span()) {
            continue;
        }
        match block.kind() {
            BlockKind::TableRow => rows.push(Vec::new()),
            BlockKind::TableCell => {
                if let Some(row) = rows.last_mut() {
                    row.push(block.span());
                }
            }
            _ => {}
        }
    }
    rows.retain(|row| !row.is_empty());
    rows
}

/// Which bytes each row covers: its first cell's start to its last cell's end.
///
/// A grid is the one layout that **drops** a source line — the delimiter row draws as nothing — so
/// `hit.rs` cannot find a row's bytes by counting newlines the way it does everywhere else. Without
/// this, every row below the header resolves against the line above it and the caret cannot be
/// placed in one.
///
/// The delimiter row's own bytes are deliberately not covered: they are syntax, and MDEDIT-118 has
/// the caret step over concealed syntax rather than into it.
#[must_use]
pub fn row_spans(cells: &[Vec<ByteSpan>]) -> Vec<ByteSpan> {
    cells
        .iter()
        .filter_map(|row| Some(ByteSpan::new(row.first()?.start(), row.last()?.end())))
        .collect()
}

/// How wide the text of one laid-out cell is.
///
/// The right edge of its last glyph, for the reason `Cached` measures a chunk the same way: a layout
/// asked for an unbounded width may report the width it was *asked* for.
#[must_use]
pub fn cell_width(cell: &Layout) -> f32 {
    cell.lines
        .iter()
        .filter_map(|line| line.glyphs.last())
        .map(|glyph| glyph.pos.x + glyph.advance)
        .fold(0.0_f32, f32::max)
}

/// Assembles laid-out cells into one layout whose lines are the table's rows.
///
/// `rows` holds each cell's own layout, laid out at an unbounded width so nothing wrapped.
///
/// # Two things this has to get right
///
/// **A column is as wide as its widest cell**, over every row including the header. A ragged column
/// is the thing a reader's eye cannot follow, and it is the whole difference between this and the
/// pipe rows it replaces.
///
/// **The trailing empty line is not optional.** A chunk whose bytes end in a newline gets one from
/// the text engine, and `Cached` drops it again through `blocks::drops_last_line` — so a layout
/// assembled without one has its **last row** dropped instead, silently, from both the height and
/// the drawing. `ends_with_newline` puts it back, and the assembled layout is then shaped exactly
/// like a linear one.
#[must_use]
pub fn assemble(rows: &[Vec<Layout>], ends_with_newline: bool) -> Layout {
    let columns = column_widths(rows);
    let mut lines = Vec::with_capacity(rows.len() + usize::from(ends_with_newline));
    let mut top = 0.0_f32;

    for row in rows {
        let height = row.iter().map(cell_height).fold(0.0_f32, f32::max);
        let baseline = top + row.iter().map(first_baseline).fold(0.0_f32, f32::max);

        // Pushed column by column, so the last glyph on the assembled line is the rightmost one —
        // which is what `Cached::new` assumes when it measures the widest row for the pan clamp.
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        for (index, cell) in row.iter().enumerate() {
            let first = first_baseline(cell);
            for glyph in cell.lines.iter().flat_map(|line| line.glyphs.iter()) {
                let mut moved = *glyph;
                moved.pos = LayoutPos {
                    x: glyph.pos.x + x,
                    // **The row's baseline, not the cell's.** Cells of different heights would
                    // otherwise sit on their own baselines and the row would read as a stagger.
                    // A cell that did break into more lines keeps its own spacing below that.
                    y: baseline + (glyph.pos.y - first),
                };
                glyphs.push(moved);
            }
            x += columns.get(index).copied().unwrap_or(0.0) + COLUMN_GAP;
        }

        lines.push(LayoutLine {
            baseline,
            height,
            glyphs,
            rules: Vec::new(),
        });
        top += height;
    }

    if ends_with_newline {
        let height = lines.last().map_or(0.0, |line| line.height);
        lines.push(LayoutLine {
            baseline: top + height,
            height,
            glyphs: Vec::new(),
            rules: Vec::new(),
        });
    }

    Layout { lines }
}

/// How tall one laid-out cell is. A cell that did not wrap is one line.
fn cell_height(cell: &Layout) -> f32 {
    cell.lines.iter().map(|line| line.height).sum()
}

/// Where a cell's own first baseline sits below its top.
///
/// A cell is laid out as its own block, so this is measured from *that* block's top; the assembled
/// row adds its own offset down the table. Getting this wrong stacks every row on one baseline.
fn first_baseline(cell: &Layout) -> f32 {
    cell.lines.first().map_or(0.0, |line| line.baseline)
}

/// The width of each column: the widest cell in it, over every row.
fn column_widths(rows: &[Vec<Layout>]) -> Vec<f32> {
    let count = rows.iter().map(Vec::len).max().unwrap_or(0);
    (0..count)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(cell_width)
                .fold(0.0_f32, f32::max)
        })
        .collect()
}
