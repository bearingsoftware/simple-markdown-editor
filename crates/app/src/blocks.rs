//! The units the renderer lays out, and where each one sits down the page.
//!
//! The document is cut into **chunks**: every top-level block, plus the text between them — the
//! blank lines that belong to no block. Chunks tile the buffer, so concatenating them reproduces it
//! byte for byte, which is what lets each one be laid out on its own and stacked.
//!
//! Nothing here talks to a text engine. Heights arrive already measured (or estimated); this module
//! only stacks them and answers which chunks the viewport covers.

use std::ops::Range;

use mdedit_document::Document;
use mdedit_markers::BlockKind;
use mdedit_offsets::{ByteOffset, ByteSpan};

/// One unit of layout: a top-level block, or the gap between two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    /// The bytes it covers.
    pub span: ByteSpan,
    /// What kind of block it is, or `None` for the gap between blocks.
    pub kind: Option<BlockKind>,
}

/// Cuts `doc` into chunks, in document order, covering every byte exactly once.
///
/// Top-level blocks only: a list and everything in it is one chunk, because laying out a list item
/// without its list would lose the indentation the item is drawn with. That makes a document whose
/// whole body is one huge list a single chunk, and no cheaper to draw than it is today.
///
/// Cuts land on **line starts**, and only there. A boundary in the middle of a line would put two
/// chunks on the same line, and two layouts cannot share one: the second would start at the left
/// margin, on a line of its own, under the first. Blocks already begin at a line start, so this
/// costs nothing except the guarantee.
///
/// `text` is the buffer, which the caller has already materialized.
#[must_use]
pub fn split(doc: &Document, text: &str) -> Vec<Chunk> {
    let len = text.len();
    // Where each chunk begins, and what starts there. A block's trailing blank lines belong to it,
    // because the next cut is the next block's line.
    let mut bounds: Vec<(usize, Option<BlockKind>)> = Vec::new();
    for block in doc.blocks().iter().filter(|block| block.depth() == 0) {
        let start = line_start(text, block.span().start().get().min(len));
        if start >= len {
            continue;
        }
        match bounds.last() {
            // Two blocks starting on one line: the outer one already claimed it.
            Some((last, _)) if *last >= start => {}
            _ => bounds.push((start, Some(block.kind()))),
        }
    }

    if bounds.first().is_none_or(|(start, _)| *start > 0) {
        bounds.insert(0, (0, None));
    }

    let mut chunks: Vec<Chunk> = Vec::with_capacity(bounds.len());
    for (index, (start, kind)) in bounds.iter().enumerate() {
        let end = bounds.get(index + 1).map_or(len, |(next, _)| *next);
        chunks.push(Chunk {
            span: ByteSpan::new(ByteOffset::new(*start), ByteOffset::new(end)),
            kind: *kind,
        });
    }
    chunks
}

/// The start of the line `at` is on.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |index| index + 1)
}

/// How many lines a chunk contributes to the stacked document.
///
/// Laying a chunk out on its own gives one more line than it has newlines — text after the last
/// newline occupies a line even when it is empty. Stacked, that trailing empty line is not a line
/// of its own: it is where the next chunk starts. So every chunk that ends in a newline gives up
/// its last line, except the final one, where the empty line is real and the caret can sit on it.
#[must_use]
pub fn line_count(text: &str, is_last: bool) -> usize {
    let newlines = text.bytes().filter(|byte| *byte == b'\n').count();
    if drops_last_line(text, is_last) {
        newlines
    } else {
        newlines + 1
    }
}

/// Whether a laid-out chunk's last line belongs to the next chunk rather than to it.
#[must_use]
pub fn drops_last_line(text: &str, is_last: bool) -> bool {
    !is_last && text.ends_with('\n')
}

/// Running totals: `tops[i]` is where chunk `i` starts, measured from the top of the document.
#[must_use]
pub fn stack(heights: &[f32]) -> Vec<f32> {
    let mut tops = Vec::with_capacity(heights.len());
    let mut y = 0.0;
    for height in heights {
        tops.push(y);
        y += height;
    }
    tops
}

/// The chunks a viewport of `viewport` pixels starting at `offset` touches.
///
/// Half-open, and empty only when there are no chunks at all. A chunk that is partly on screen is
/// included — clipping happens in the renderer, which is where a partly-visible fence has to be
/// drawn from anyway.
#[must_use]
pub fn visible(tops: &[f32], heights: &[f32], offset: f32, viewport: f32) -> Range<usize> {
    if tops.is_empty() {
        return 0..0;
    }
    let bottom = offset + viewport;

    // The last chunk starting at or before the viewport's top edge, so one straddling it is
    // included rather than skipped.
    let mut first = tops.partition_point(|top| *top <= offset).saturating_sub(1);
    // Unless it ends above the edge, which happens when the offset lands in the gap a zero-height
    // chunk leaves behind.
    if first + 1 < tops.len() && tops[first] + heights[first] <= offset {
        first += 1;
    }

    // Forward from there by walking: heights vary too much for a second binary search to beat it,
    // and the walk only covers what is on screen.
    let mut last = first;
    while last + 1 < tops.len() && tops[last + 1] < bottom {
        last += 1;
    }
    first..last + 1
}
