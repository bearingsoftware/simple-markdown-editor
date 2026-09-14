//! Going the other way: from a place on screen to a byte in the buffer.
//!
//! Everything else in the renderer maps bytes to pixels. Clicking, and moving the caret up or down,
//! need the inverse — and it is not simply the reverse lookup, because a point can land where there
//! is no text at all: past the end of a short line, on a blank line, below the last paragraph. Every
//! one of those has to resolve to a byte anyway, or the click does nothing and the editor feels
//! broken.
//!
//! Works on **visual** lines, not source lines. A wrapped paragraph is one source line and several
//! visual ones, and pressing Down in the middle of it must move to the next row of the same
//! paragraph rather than skipping to the next paragraph.
//!
//! No GPU or window types. The caller supplies a laid-out chunk and a point inside it; this decides
//! which byte that is.

use mdedit_offsets::ByteOffset;
use mdedit_text::Layout;

use crate::table::Grid;

/// A laid-out chunk, and where its text sits in the buffer.
#[derive(Debug, Clone, Copy)]
pub struct Chunk<'a> {
    /// The chunk's layout.
    pub layout: &'a Layout,
    /// The chunk's text, as it is in the buffer now.
    pub text: &'a str,
    /// Where the chunk starts in the buffer.
    pub start: ByteOffset,
    /// Where the chunk started when it was laid out, which is not the same thing: an identical
    /// chunk elsewhere in the document reuses the layout, and the glyphs still carry the offsets
    /// they were laid out with.
    pub base: ByteOffset,
    /// The boxes this chunk was laid out as, when it is a table (MDEDIT-126, MDEDIT-128).
    ///
    /// `None` for the ordinary case, where a row is a source line or part of a wrapped one, and a
    /// glyph is the only thing a point can address. A grid needs it twice over:
    ///
    /// - it is the one layout that **drops** a source line — the delimiter row draws as nothing — so
    ///   row 1 is source line 2 and counting newlines finds the wrong bytes;
    /// - **an empty cell has no glyphs**, so walking them cannot find it at all. A column comes from
    ///   the boxes, and the answer comes from the cell that column names.
    ///
    /// Spans are in the same space as a glyph's `source`, so [`base`](Self::base) rebases them.
    pub grid: Option<&'a Grid>,
}

impl Chunk<'_> {
    /// A glyph's source offset, moved to where this chunk actually is.
    fn rebase(&self, offset: ByteOffset) -> ByteOffset {
        ByteOffset::new(self.start.get() + offset.get().saturating_sub(self.base.get()))
    }

    /// The bytes of visual line `row` when the layout carries its own mapping, as offsets into
    /// [`text`](Self::text).
    fn row_span(&self, row: usize) -> Option<(usize, usize)> {
        let span = self.grid?.rows.get(row)?;
        let at = |offset: ByteOffset| {
            offset
                .get()
                .saturating_sub(self.base.get())
                .min(self.text.len())
        };
        Some((at(span.start()), at(span.end())))
    }

    /// Where visual line `row` starts, as an offset into [`text`](Self::text).
    ///
    /// Visual, so a wrapped line has no newline of its own to count. Wrapping only ever *adds*
    /// rows, so counting newlines finds the right byte for every row up to the number of newlines
    /// and the end of the text past that — which is what a caller clicking below the last line
    /// should get anyway. A layout that *drops* a row instead says so through
    /// [`rows`](Self::rows), which then answers ahead of the count.
    fn line_start(&self, row: usize) -> usize {
        if let Some((start, _)) = self.row_span(row) {
            return start;
        }
        if row == 0 {
            return 0;
        }
        let mut seen = 0;
        for (index, byte) in self.text.bytes().enumerate() {
            if byte == b'\n' {
                seen += 1;
                if seen == row {
                    return index + 1;
                }
            }
        }
        self.text.len()
    }

    /// Where visual line `row` ends, not counting its newline.
    fn line_end(&self, row: usize) -> usize {
        if let Some((_, end)) = self.row_span(row) {
            return end;
        }
        let from = self.line_start(row);
        self.text[from..]
            .find('\n')
            .map_or(self.text.len(), |at| from + at)
    }
}

/// How many visual lines a laid-out chunk has.
#[must_use]
pub fn rows(chunk: &Chunk<'_>, drawn: usize) -> usize {
    chunk.layout.lines.len().min(drawn)
}

/// Which visual line a point falls on, given the chunk's own coordinates.
///
/// Clamped: above the first line is the first line, below the last is the last. A click in the
/// margin below the text still has to put the caret somewhere.
#[must_use]
pub fn row_at(chunk: &Chunk<'_>, drawn: usize, y: f32) -> usize {
    let rows = rows(chunk, drawn);
    let mut top = 0.0;
    for (index, line) in chunk.layout.lines.iter().take(rows).enumerate() {
        if y < top + line.height {
            return index;
        }
        top += line.height;
    }
    rows.saturating_sub(1)
}

/// The byte a point addresses, in buffer coordinates.
///
/// `y` is measured from the top of the chunk and `x` from its left edge, both in
/// device-independent pixels.
#[must_use]
pub fn offset_at(chunk: &Chunk<'_>, drawn: usize, y: f32, x: f32) -> ByteOffset {
    let row = row_at(chunk, drawn, y);
    offset_in_row(chunk, row, x)
}

/// The byte at `x` on visual line `row`.
///
/// Nearest boundary rather than the glyph under the pointer: clicking the left half of a character
/// puts the caret before it and the right half after it, which is what every editor does and what
/// makes clicking between two letters possible at all.
#[must_use]
pub fn offset_in_row(chunk: &Chunk<'_>, row: usize, x: f32) -> ByteOffset {
    let Some(line) = chunk.layout.lines.get(row) else {
        return chunk.start;
    };

    // **A grid resolves the column first.** Walking glyphs cannot find an empty cell — it has none —
    // and would carry a click past it into whichever column has text next. Every pixel of the table
    // belongs to a column, so the column is what a point addresses and the glyphs within it decide
    // only where in the cell the caret goes.
    if let Some(grid) = chunk.grid {
        if let Some(cell) = grid.cells.get(row).and_then(|cells| {
            let column = grid.column_at(x);
            cells.get(column.min(cells.len().saturating_sub(1)))
        }) {
            let inside = |glyph: &&mdedit_text::PositionedGlyph| {
                glyph.source.start().get() >= cell.start().get()
                    && glyph.source.start().get() < cell.end().get()
            };
            for glyph in line.glyphs.iter().filter(inside) {
                if x < glyph.pos.x + glyph.advance / 2.0 {
                    return chunk.rebase(glyph.source.start());
                }
            }
            // Past this cell's last glyph, or a cell with none at all — a freshly inserted one. The
            // caret goes to the end of the cell's own text, which is where typing appends.
            return chunk.rebase(cell.end());
        }
    }

    for glyph in &line.glyphs {
        if x < glyph.pos.x + glyph.advance / 2.0 {
            return chunk.rebase(glyph.source.start());
        }
    }

    // Past the last glyph, or a line with no glyphs at all — a blank one. Either way the answer is
    // the end of that line's text, which the layout cannot give us and the text can.
    ByteOffset::new(chunk.start.get() + chunk.line_end(row))
}

/// Where the drawn stand-in under a point came from: the start of the marker it substitutes for.
///
/// A substitute's glyphs carry an empty source at the start of what they stand for
/// (`StyledRun::substituted`), so this is how a click finds the construct a task's box belongs to
/// (MDEDIT-142). **Under the glyph, not the nearest boundary** — a box is pressed, not placed between —
/// and on a row the point is really on: [`row_at`] clamps a point below the text to the last row,
/// which would tick a box at the end of a file from a click in the space beneath it.
#[must_use]
pub fn substitute_at(chunk: &Chunk<'_>, drawn: usize, y: f32, x: f32) -> Option<ByteOffset> {
    let mut top = 0.0;
    let line = chunk
        .layout
        .lines
        .iter()
        .take(rows(chunk, drawn))
        .find(|line| {
            let on = (top..top + line.height).contains(&y);
            top += line.height;
            on
        })?;
    line.glyphs
        .iter()
        .find(|glyph| {
            glyph.source.is_empty() && glyph.pos.x <= x && x < glyph.pos.x + glyph.advance
        })
        .map(|glyph| chunk.rebase(glyph.source.start()))
}

/// Where `offset` sits: which visual line, and how far along it.
///
/// The `x` is what a caller moving up or down should aim for on the next line.
#[must_use]
pub fn place_of(chunk: &Chunk<'_>, drawn: usize, offset: ByteOffset) -> Option<(usize, f32)> {
    let rows = rows(chunk, drawn);
    for row in 0..rows {
        let start = chunk.start.get() + chunk.line_start(row);
        let end = chunk.start.get() + chunk.line_end(row);
        if offset.get() < start || offset.get() > end {
            continue;
        }

        let line = chunk.layout.lines.get(row)?;
        let x = line
            .glyphs
            .iter()
            .find(|glyph| chunk.rebase(glyph.source.start()) >= offset)
            .map_or_else(
                || {
                    line.glyphs
                        .last()
                        .map_or(0.0, |glyph| glyph.pos.x + glyph.advance)
                },
                |glyph| glyph.pos.x,
            );
        // **An empty cell has no glyph to hang the caret on**, and the fallbacks above would put it
        // at the end of the row or against the table's left edge — both of them somewhere the caret
        // is not. The cell's own box says where it goes.
        if let Some(grid) = chunk.grid {
            if let Some((cell_row, column)) = grid.cell_of(ByteOffset::new(
                chunk.base.get() + offset.get().saturating_sub(chunk.start.get()),
            )) {
                let cell = grid.cells[cell_row][column];
                let empty = !line.glyphs.iter().any(|glyph| {
                    glyph.source.start().get() >= cell.start().get()
                        && glyph.source.start().get() < cell.end().get()
                });
                if cell_row == row && empty {
                    return Some((row, grid.text_left(column)));
                }
            }
        }
        return Some((row, x));
    }
    None
}

/// Where the top of visual line `row` is, measured from the top of the chunk.
#[must_use]
pub fn row_top(chunk: &Chunk<'_>, row: usize) -> f32 {
    chunk
        .layout
        .lines
        .iter()
        .take(row)
        .map(|line| line.height)
        .sum()
}
