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

use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::Layout;

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
    /// Which bytes each visual row covers, when counting newlines would not find out (MDEDIT-126).
    ///
    /// `None` for the ordinary case, where a row is a source line or part of a wrapped one. A table
    /// laid out as a grid needs this because it is the one layout that *drops* a source line — the
    /// delimiter row is drawn as nothing — so row 1 is source line 2 and every row below the header
    /// resolves against the wrong bytes without it.
    ///
    /// In the same space as a glyph's `source`, so [`base`](Self::base) rebases it.
    pub rows: Option<&'a [ByteSpan]>,
}

impl Chunk<'_> {
    /// A glyph's source offset, moved to where this chunk actually is.
    fn rebase(&self, offset: ByteOffset) -> ByteOffset {
        ByteOffset::new(self.start.get() + offset.get().saturating_sub(self.base.get()))
    }

    /// The bytes of visual line `row` when the layout carries its own mapping, as offsets into
    /// [`text`](Self::text).
    fn row_span(&self, row: usize) -> Option<(usize, usize)> {
        let span = self.rows?.get(row)?;
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

    for glyph in &line.glyphs {
        if x < glyph.pos.x + glyph.advance / 2.0 {
            return chunk.rebase(glyph.source.start());
        }
    }

    // Past the last glyph, or a line with no glyphs at all — a blank one. Either way the answer is
    // the end of that line's text, which the layout cannot give us and the text can.
    ByteOffset::new(chunk.start.get() + chunk.line_end(row))
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
