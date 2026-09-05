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
use mdedit_offsets::{ByteOffset, ByteSpan, LayoutPos};
use mdedit_text::{Layout, LayoutLine};

/// Space between a cell's text and its border, left and right.
pub const PADDING_X: f32 = 10.0;

/// Space between a cell's text and its border, above and below.
pub const PADDING_Y: f32 = 3.0;

/// How far a Latin font's ascent overshoots the capitals that ordinary text actually reaches, as a
/// fraction of the ascent (MDEDIT-154).
///
/// Measured rather than assumed: Consolas at 16 px has an ascent of 17.27 and puts the top of a
/// capital 12 px above the baseline, so 5.27 px — 30.5% — is reserved for accents and tall glyphs
/// that Latin prose never uses. Its descent, by contrast, is spent almost exactly: 0.02 px to spare
/// under a descender.
///
/// **Which is why a table row looked wrong before this.** `assemble` centred the *font* box in the
/// row, symmetric about ascent and descent, which is both what it says and what CSS does — but the
/// font box is not symmetric about the ink inside it, so the text landed half the overshoot low and
/// a `g` came close to the bottom rule.
///
/// 29% is the middle of the range this was checked across rather than one font's exact figure; the
/// same measurement at 28 px gives 30.5% again, and Arial and Times sit nearer 21% and 26%. Being a
/// *fraction* is the part that matters — the overshoot scales with the font size, so a fixed
/// nudge would centre one size and miss every other.
const ASCENT_OVERSHOOT: f32 = 0.29;

/// How thick a rule is, before the device scale.
///
/// One device-independent pixel, which the renderer floors at one physical pixel — a rule that
/// rounds to nothing is a line that is not there, the same reason `draw_rules` clamps.
pub const RULE: f32 = 1.0;

/// The boxes a table is ruled with (MDEDIT-126).
///
/// Everything a grid knows that a run of lines does not. The layout carries the glyphs; this carries
/// what the glyphs sit inside, which is what the renderer needs to draw a border and what `hit`
/// needs to find a row's bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// Which bytes each visual row covers.
    pub rows: Vec<ByteSpan>,
    /// Which bytes each cell covers, by row and then column.
    ///
    /// **An empty cell has no glyphs, so it has no geometry** — which is what made a new column
    /// impossible to click into and a new row six pixels tall. Hit-testing a grid therefore resolves
    /// a column from the boxes and takes the answer from here, rather than looking for a glyph that
    /// may not exist.
    pub cells: Vec<Vec<ByteSpan>>,
    /// The width of each column's box, padding included.
    pub columns: Vec<f32>,
}

impl Grid {
    /// The left edge of each column's box, and one more for the right edge of the last.
    ///
    /// `columns.len() + 1` values, which is exactly the number of vertical rules a table has.
    #[must_use]
    pub fn column_edges(&self) -> Vec<f32> {
        let mut edges = Vec::with_capacity(self.columns.len() + 1);
        let mut x = 0.0;
        edges.push(x);
        for width in &self.columns {
            x += width;
            edges.push(x);
        }
        edges
    }

    /// How wide the whole table is.
    #[must_use]
    pub fn width(&self) -> f32 {
        self.columns.iter().sum()
    }

    /// Which column `x` falls in, measured from the table's left edge.
    ///
    /// Every pixel of the table belongs to a column: past the right-hand edge is the last column,
    /// which is what makes clicking in the space after a short row land somewhere sensible. The same
    /// rule MDEDIT-127's picker follows for its cells, and for the same reason — a dead pixel
    /// between two boxes is a click that does nothing.
    #[must_use]
    pub fn column_at(&self, x: f32) -> usize {
        let mut edge = 0.0;
        for (index, width) in self.columns.iter().enumerate() {
            edge += width;
            if x < edge {
                return index;
            }
        }
        self.columns.len().saturating_sub(1)
    }

    /// Where the text of column `index` starts, measured from the table's left edge.
    #[must_use]
    pub fn text_left(&self, index: usize) -> f32 {
        self.columns.iter().take(index).sum::<f32>() + PADDING_X
    }

    /// The cell holding `at`, as a row and column.
    #[must_use]
    pub fn cell_of(&self, at: ByteOffset) -> Option<(usize, usize)> {
        self.cells.iter().enumerate().find_map(|(row, cells)| {
            cells
                .iter()
                .position(|cell| cell.start().get() <= at.get() && at.get() <= cell.end().get())
                .map(|column| (row, column))
        })
    }
}

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
/// # Three things this has to get right
///
/// **A column is as wide as its widest cell**, over every row including the header. A ragged column
/// is the thing a reader's eye cannot follow, and it is the whole difference between this and the
/// pipe rows it replaces.
///
/// **A cell's text sits inside its box, not on the border.** Every column carries [`PADDING_X`] on
/// each side and every row [`PADDING_Y`], so the returned [`Grid`] describes boxes the renderer can
/// rule around without the rule touching a glyph.
///
/// **The trailing empty line is not optional.** A chunk whose bytes end in a newline gets one from
/// the text engine, and `Cached` drops it again through `blocks::drops_last_line` — so a layout
/// assembled without one has its **last row** dropped instead, silently, from both the height and
/// the drawing. `ends_with_newline` puts it back, and the assembled layout is then shaped exactly
/// like a linear one. It is *not* part of the grid: it is the blank line after the table, and ruling
/// it would draw a box around nothing.
#[must_use]
pub fn assemble(
    rows: &[Vec<Layout>],
    cells: &[Vec<ByteSpan>],
    ends_with_newline: bool,
) -> (Layout, Grid) {
    let columns: Vec<f32> = column_widths(rows)
        .into_iter()
        .map(|width| width + PADDING_X * 2.0)
        .collect();
    let mut lines = Vec::with_capacity(rows.len() + usize::from(ends_with_newline));
    let mut top = 0.0_f32;
    let mut last_text = 0.0_f32;

    for row in rows {
        let text = row.iter().map(cell_height).fold(0.0_f32, f32::max);
        let height = text + PADDING_Y * 2.0;
        let ascent = row.iter().map(first_baseline).fold(0.0_f32, f32::max);
        // Lifted off the font box and onto the ink — see `optical_lift`. The row's *height* is
        // untouched: the space the lift takes from above the text is given straight back below it,
        // so columns, rules and row spans are all where they were.
        let baseline = top + PADDING_Y + ascent - optical_lift(ascent);

        // Pushed column by column, so the last glyph on the assembled line is the rightmost one —
        // which is what `Cached::new` assumes when it measures the widest row for the pan clamp.
        let mut glyphs = Vec::new();
        let mut edge = 0.0;
        for (index, cell) in row.iter().enumerate() {
            let left = edge + PADDING_X;
            let first = first_baseline(cell);
            for glyph in cell.lines.iter().flat_map(|line| line.glyphs.iter()) {
                let mut moved = *glyph;
                moved.pos = LayoutPos {
                    x: glyph.pos.x + left,
                    // **The row's baseline, not the cell's.** Cells of different heights would
                    // otherwise sit on their own baselines and the row would read as a stagger.
                    // A cell that did break into more lines keeps its own spacing below that.
                    y: baseline + (glyph.pos.y - first),
                };
                glyphs.push(moved);
            }
            edge += columns.get(index).copied().unwrap_or(0.0);
        }

        lines.push(LayoutLine {
            baseline,
            height,
            glyphs,
            rules: Vec::new(),
        });
        top += height;
        last_text = text;
    }

    if ends_with_newline {
        // The line *after* the table, so it gets an ordinary line's height rather than a ruled row's
        // padded one — the padding belongs to a box, and this one has none.
        lines.push(LayoutLine {
            baseline: top + last_text,
            height: last_text,
            glyphs: Vec::new(),
            rules: Vec::new(),
        });
    }

    (
        Layout { lines },
        Grid {
            rows: row_spans(cells),
            cells: cells.to_vec(),
            columns,
        },
    )
}

/// How far to raise a row's baseline so the text is centred on its ink (MDEDIT-154).
///
/// **Half** the overshoot, because moving the baseline up by one pixel takes a pixel from above the
/// ink and gives it below: the gap closes at twice the rate the text moves.
///
/// This is a correction to a *boxed* row, not to text in general. Prose stacks flush with no visible
/// edge to be off-centre in, so nothing there wants it — which is why the constant lives here rather
/// than in the text layer, and why the text trait did not have to learn about cap heights.
///
/// Public so a test can state the rule rather than restate a number: three of them assert where a
/// row's baseline lands, and a literal there would have to be recomputed by hand every time this
/// changed — which is how a test stops describing anything.
#[must_use]
pub fn optical_lift(ascent: f32) -> f32 {
    ascent * ASCENT_OVERSHOOT / 2.0
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
