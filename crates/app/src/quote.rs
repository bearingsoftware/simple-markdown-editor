//! A block quote, assembled so its depth shows and its wrapped rows stay clear of its bars
//! (MDEDIT-145).
//!
//! Before this, `>` was concealed and quoted text drew muted and italic at any depth, so `>` and
//! `>>` looked identical: a reply inside a reply read as one quote. Now each level draws a bar at the
//! left, and the text is indented past them.
//!
//! # Why rows are placed rather than the marker substituted
//!
//! MDEDIT-141's substitution would give the concealed `> ` text of its own and indent a line's
//! **first** row. A long quoted line wraps, and every row after the first would begin at the left
//! edge — under the bars. The same defect MDEDIT-147 fixed for lists, with the same answer: lay the
//! text out at the width its indent leaves, and place it ([`crate::place`]).
//!
//! # The unit is a run of lines at one depth
//!
//! Depth changes line by line — `> outer` then `> > inner` — so a quote is cut wherever it does
//! ([`units`]), and each run is laid out once. Depth comes from the block tree via
//! `Document::quote_depth`, **not** from the markers: `> > b` and `>> b` are one coalesced marker
//! each, and a lazy continuation line has none while still belonging to its quote.
//!
//! # What `hit.rs` needs from this, which is nothing
//!
//! As for a list: placing rows only ever moves them, and wrapping only ever *adds* them, so counting
//! newlines stays correct and no row mapping is needed.

use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{Layout, LayoutLine};

use crate::place;

/// How wide one bar is, in device-independent pixels.
///
/// Wider than a table's one-pixel rule on purpose: a rule separates, and a quote bar *marks*, so it
/// has to read as a stroke at a glance rather than as a hairline you find.
pub const BAR: f32 = 3.0;

/// How far each level of quoting moves the text right, and so the distance between two bars.
pub const STEP: f32 = 16.0;

/// A run of consecutive lines quoted to the same depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unit {
    /// Whole lines, newline included.
    pub span: ByteSpan,
    /// How many quotes enclose them; 0 for the blank lines a chunk carries after its quote.
    pub depth: usize,
}

/// A stretch of the assembled layout that is quoted, and how deeply: where its bars go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// From the top of the chunk.
    pub top: f32,
    /// How far down the rows it covers run, and so how long each of its bars is.
    pub height: f32,
    /// How many bars: one per enclosing quote.
    pub depth: usize,
    /// Where the quote's left edge is, from the chunk's (MDEDIT-169). Zero for a quote that is a
    /// chunk of its own; for one inside a list item, the item's text indent, so its bars stand under
    /// the item rather than at the margin.
    pub x: f32,
}

/// How far text at `depth` is indented.
#[must_use]
pub fn indent(depth: usize) -> f32 {
    // Depth is a nesting count, and nobody writes 65,536 `>`s; saturating is the honest answer for
    // a document that did, rather than a cast that says nothing.
    f32::from(u16::try_from(depth).unwrap_or(u16::MAX)) * STEP
}

/// Where the bar for nesting `level` goes, counting from 0 for the outermost.
#[must_use]
pub fn bar_x(level: usize) -> f32 {
    indent(level)
}

/// `span`, cut into runs of lines quoted to one depth. `text` is the whole buffer.
///
/// A line's depth is asked at its **last** byte before the newline, not its first. The inner quote
/// of `> > b` begins at the second `>`, so the line's first byte is only in the outer one; its last
/// is inside both. A lazy continuation line has no `>` at all and its last byte is still inside its
/// quote, and a blank line after the quote has nothing but its newline, which is inside nothing.
#[must_use]
pub fn units(doc: &Document, span: ByteSpan, text: &str) -> Vec<Unit> {
    let end = span.end().get();
    let mut at = span.start().get();
    let mut units: Vec<Unit> = Vec::new();

    while at < end {
        let line_end = text[at..end]
            .find('\n')
            .map_or(end, |newline| at + newline + 1);
        let content_end = if text[at..line_end].ends_with('\n') {
            line_end - 1
        } else {
            line_end
        };
        let probe = if content_end > at {
            content_end - 1
        } else {
            at
        };
        let depth = doc.quote_depth(ByteOffset::new(probe));

        match units.last_mut() {
            Some(unit) if unit.depth == depth => {
                unit.span = ByteSpan::new(unit.span.start(), ByteOffset::new(line_end));
            }
            _ => units.push(Unit {
                span: ByteSpan::new(ByteOffset::new(at), ByteOffset::new(line_end)),
                depth,
            }),
        }
        at = line_end;
    }
    units
}

/// Stacks laid-out runs into one layout, each indented by its depth, and says where the bars go.
///
/// Each run arrives laid out on its own at the width its indent leaves. Its rows move right by that
/// indent and down to where the run before ended; a run's own trailing empty row is dropped unless
/// it is the last, for the reason [`place::rows_kept`] gives.
#[must_use]
pub fn assemble(runs: &[(usize, Layout)]) -> (Layout, Vec<Band>) {
    let mut lines: Vec<LayoutLine> = Vec::new();
    let mut bands: Vec<Band> = Vec::new();
    let mut top = 0.0_f32;

    for (index, (depth, layout)) in runs.iter().enumerate() {
        let last = index + 1 == runs.len();
        let start = top;
        for line in layout.lines.iter().take(place::rows_kept(layout, last)) {
            // From the run's start, not the row's: a row's baseline already counts the rows above
            // it in the run — see `place::row`.
            lines.push(place::row(line, indent(*depth), start));
            top += line.height;
        }
        if *depth > 0 && top > start {
            bands.push(Band {
                top: start,
                height: top - start,
                depth: *depth,
                x: 0.0,
            });
        }
    }

    (Layout { lines }, bands)
}
