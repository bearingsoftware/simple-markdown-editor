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

    /// Every source line in [`text`](Self::text), as `(start, end)` with the newline excluded.
    fn line_spans(&self) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut start = 0;
        for (index, byte) in self.text.bytes().enumerate() {
            if byte == b'\n' {
                spans.push((start, index));
                start = index + 1;
            }
        }
        spans.push((start, self.text.len()));
        spans
    }

    /// Where a row's first glyph came from, as an offset into [`text`](Self::text).
    fn glyph_start(&self, row: usize) -> Option<usize> {
        let glyph = self.layout.lines.get(row)?.glyphs.first()?;
        Some(
            glyph
                .source
                .start()
                .get()
                .saturating_sub(self.base.get())
                .min(self.text.len()),
        )
    }

    /// The bytes each visual row covers, as offsets into [`text`](Self::text) (MDEDIT-183).
    ///
    /// # Why this is not newline counting
    ///
    /// It was, and the argument was that wrapping only ever *adds* rows. It does — but it adds them
    /// **in the middle**. In `- long…\n- next\n` where the first item wraps to nine rows, `next` is
    /// row 9, and counting newlines made row 1 its answer: the caret for every line after a wrap was
    /// drawn as many rows too high as the wrap had added, and in a wrapped paragraph — one source
    /// line, several rows — row 0 answered for all of it.
    ///
    /// So the mapping comes from the layout, which is the only thing that knows where the breaks
    /// fell. **A row with glyphs knows its line from its first glyph's source**; a row with none is
    /// the line after the row above it, which is the rule `Editor::line_of_row` already uses to
    /// decide what a glyph-less row is for. MDEDIT-126 solved the mirror image of this — a grid
    /// *drops* a row — by handing over its own spans, and [`row_span`](Self::row_span) stays the
    /// override it was.
    ///
    /// # Where a row begins and ends
    ///
    /// The **first** row of a source line starts at that line's first byte, so a caret in a
    /// concealed marker still belongs to the row the marker was dropped from. The **last** row of a
    /// source line ends at that line's last byte, so a caret at the end of the line belongs there
    /// rather than nowhere. The boundaries in between are where the wrap put them: a continuation
    /// row starts at its own first glyph.
    pub(crate) fn row_bytes(&self) -> Vec<(usize, usize)> {
        let rows = self.layout.lines.len();
        let lines = self.line_spans();
        let last_line = lines.len().saturating_sub(1);

        // Each row's first glyph, taken once. Asked twice per row otherwise — a row needs its own
        // and its successor's — and each ask walks into the layout.
        let starts: Vec<Option<usize>> = (0..rows).map(|row| self.glyph_start(row)).collect();

        // Which source line each row belongs to.
        let mut line_of: Vec<usize> = Vec::with_capacity(rows);
        for start in &starts {
            let line = match *start {
                Some(at) => lines.partition_point(|(_, end)| *end < at),
                // A row with no glyphs is a blank source line, and blank lines do not wrap — so it
                // is the line after the one above it. Row 0 is the chunk's own first line.
                None => line_of.last().map_or(0, |previous| previous + 1),
            };
            line_of.push(line.min(last_line));
        }

        (0..rows)
            .map(|row| {
                if let Some(span) = self.row_span(row) {
                    return span;
                }
                let line = line_of[row];
                let (line_start, line_end) = lines[line];
                let first_of_line = row == 0 || line_of[row - 1] != line;
                let last_of_line = row + 1 >= rows || line_of[row + 1] != line;
                let start = if first_of_line {
                    line_start
                } else {
                    starts[row].unwrap_or(line_start)
                };
                let end = if last_of_line {
                    line_end
                } else {
                    starts[row + 1].unwrap_or(line_end)
                };
                (start, end.max(start))
            })
            .collect()
    }

    /// Where visual line `row` ends, not counting its newline.
    ///
    /// Builds the whole row map to read one row of it, which is deliberate: the alternative is a
    /// second implementation of the rule for one row, and three copies of this rule is what
    /// MDEDIT-183 was. It is reached once per click that lands past a row's last glyph, where a map
    /// of the one chunk under the pointer is not worth avoiding. **The draw path does not come
    /// through here** — `draw_chunks` builds the map once per chunk and hands it down on `Row`.
    fn line_end(&self, row: usize) -> usize {
        self.row_bytes()
            .get(row)
            .map_or(self.text.len(), |&(_, end)| end)
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
    let bytes = chunk.row_bytes();
    // **The last row that holds the offset, not the first** (MDEDIT-183). One byte lies on two rows
    // wherever a line wrapped: the end of a row and the start of its continuation are the same
    // offset, and there is no affinity here to tell which the caret arrived from. The continuation
    // is the right answer for the one gesture that can tell the difference — Up onto a wrapped row
    // aims at that row's left edge, which *is* the boundary, and preferring the earlier row sent it
    // a row further than it asked and made Up skip a row. Hard line ends are unaffected: a line's
    // last byte and the next line's first are separated by the newline, so only one row holds each.
    let row = (0..rows).rev().find(|&row| {
        let (from, to) = bytes.get(row).copied().unwrap_or((0, chunk.text.len()));
        let start = chunk.start.get() + from;
        let end = chunk.start.get() + to;
        (start..=end).contains(&offset.get())
    })?;

    {
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
        Some((row, x))
    }
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
