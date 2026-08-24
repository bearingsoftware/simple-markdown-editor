//! A list, assembled out of the layouts of its items so a wrapped one hangs (MDEDIT-147).
//!
//! Geometry and vocabulary only — no GPU types, no text engine, no window — as [`crate::table`] is,
//! and for the same two reasons: it makes the placement testable without a window, and laying an item
//! out needs `&mut` the engine while finding the items needs `&` the document.
//!
//! # Why the text trait did not have to grow a hanging indent
//!
//! MDEDIT-141 gives an item's **first** row its bullet and its nesting indent, by substituting text
//! for the concealed marker. A wrapped continuation row still began at the layout's left edge, so a
//! long item wrapped back underneath its own bullet.
//!
//! MDEDIT-147 was raised believing the fix had to be a hanging indent on `TextLayout::layout`, which
//! takes `(&[StyledRun], max_width)` and nothing else. That premise is true and it stops one step
//! short, in exactly the way MDEDIT-126's did: [`Layout`] is not what a text engine hands back, it is
//! what this editor assembles. A table already lays its cells out apart and *places* them; this does
//! the same with simpler arithmetic, and `mdedit-text` and both backends are untouched.
//!
//! **A per-layout indent could not have worked anyway.** `blocks::split` makes a whole list — nested
//! items and all — one chunk, so a single value on the layout call cannot express items at different
//! depths. The widened trait would have needed the value per item regardless.
//!
//! # The unit is an item, and its indent is its own marker
//!
//! A unit runs from one `Marker::ListItem` to the next ([`units`]). **Nesting needs no recursion and
//! no walk of the block tree**: a nested item's marker is another `ListItem` carrying more leading
//! whitespace, so it opens its own unit and brings its own deeper indent with it. A continuation
//! paragraph, or the blank line before the next block, attaches to the unit above it.
//!
//! The caller lays each unit out in two pieces — the substituted marker alone, to measure the indent,
//! and the rest at the width that leaves — and [`assemble`] stacks them. Every row of an item then has
//! the same usable text width, which is what a hanging indent *is*. Laying a whole unit out at the
//! reduced width in **one** call would be a row narrower on the first line, because the marker would
//! be inside that width as well as beside it.
//!
//! # What `hit.rs` needs from this, which is nothing
//!
//! A table hands `hit` its row spans because a grid **drops** a source line. This drops none —
//! wrapping only ever *adds* rows — so counting newlines stays correct and a list needs no mapping.

use mdedit_document::Document;
use mdedit_language::{Marker, SegmentKind};
use mdedit_offsets::{ByteSpan, LayoutPos};
use mdedit_text::{Layout, LayoutLine};

/// One item's two laid-out halves, and where its text hangs.
#[derive(Debug)]
pub struct Item {
    /// The substituted marker — the bullet or the number — laid out alone.
    ///
    /// Its width is the indent, which is why it is measured rather than computed: the marker carries
    /// the item's leading whitespace (MDEDIT-141), so a nested item's is wider by exactly the nesting.
    pub marker: Layout,
    /// Everything after the marker, laid out at the width the marker leaves.
    pub text: Layout,
}

/// Where each list item begins within `span`, as the spans to lay out.
///
/// Taken from the `ListItem` markers rather than from the block tree, which reports a parent item's
/// span as *containing* its nested list and would need unpicking. Every item's marker is a segment of
/// its own at the depth it sits, nested or not, so the markers already are the boundaries.
///
/// **Not every `ListItem` marker opens an item.** A continuation line's leading indent is one too —
/// `mdedit-markers` counts a list marker's whitespace as part of it (MDEDIT-141), and a lazy
/// continuation has the whitespace and nothing else. Cutting there would give `  more of alpha` a
/// unit of its own and hang it as though it were an item, so a marker that is *only* whitespace is
/// not a boundary. `text` is the whole buffer, as segment spans are absolute.
///
/// Empty when there is no list marker here at all, and the caller then lays the chunk out as ordinary
/// lines — the honest answer for a chunk that did not turn out to be a list.
#[must_use]
pub fn units(doc: &Document, span: ByteSpan, text: &str) -> Vec<ByteSpan> {
    let mut starts: Vec<_> = doc
        .segments()
        .iter()
        .filter(|segment| segment.kind() == SegmentKind::Marker(Marker::ListItem))
        .filter(|segment| !segment.span().slice(text).trim_start().is_empty())
        .map(|segment| segment.span().start())
        .filter(|start| *start >= span.start() && *start < span.end())
        .collect();
    starts.dedup();

    // Anything before the first marker belongs to the first unit rather than to no unit: a list
    // chunk can open with the blank lines `blocks::split` gave it.
    if let Some(first) = starts.first_mut() {
        *first = span.start();
    }

    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(span.end());
            ByteSpan::new(*start, end)
        })
        .collect()
}

/// Stacks laid-out items into one layout, each item's text hanging under its own marker.
///
/// # Two things this has to get right
///
/// **A unit's own trailing empty line is not a row of the list.** Every unit ends in a newline, so the
/// engine gives it a trailing line with no glyphs — and stacked naively that is a blank row between
/// every pair of items, which is `2N` rows for `N` items. Only the **last** unit keeps its trailing
/// line, because that one is the blank line after the list, which the linear layout had too and which
/// `blocks::drops_last_line` expects to find. The same bargain `table::assemble` strikes with
/// `ends_with_newline`.
///
/// **A marker sits on its item's first baseline, not on its own.** The marker is laid out alone, so
/// its baseline is measured from *its* block's top; putting it on the row unchanged drops it onto the
/// text's ascent and the bullet floats.
#[must_use]
pub fn assemble(items: &[Item]) -> Layout {
    let mut lines: Vec<LayoutLine> = Vec::new();
    let mut top = 0.0_f32;

    for (index, item) in items.iter().enumerate() {
        let last = index + 1 == items.len();
        let indent = width_of(&item.marker);
        let rows = rows_of(&item.text, last);

        for (row, line) in item.text.lines.iter().take(rows).enumerate() {
            let baseline = top + line.baseline;
            let mut glyphs = Vec::with_capacity(line.glyphs.len() + item.marker.glyphs().count());

            // The marker belongs to the item's first row and to no other. That is the whole of the
            // hanging indent: every row after this one gets the offset without the bullet.
            if row == 0 {
                let first = item.marker.lines.first().map_or(0.0, |line| line.baseline);
                for glyph in item.marker.glyphs() {
                    let mut moved = *glyph;
                    moved.pos = LayoutPos::new(glyph.pos.x, baseline + (glyph.pos.y - first));
                    glyphs.push(moved);
                }
            }

            for glyph in &line.glyphs {
                let mut moved = *glyph;
                moved.pos = LayoutPos::new(glyph.pos.x + indent, baseline);
                glyphs.push(moved);
            }

            // Rules move with the text they are under. A link's underline on a wrapped row would
            // otherwise stay at the left edge while its glyphs hung, which is the sort of thing only
            // a document with an underlined link in a long list item would ever show.
            let rules = line
                .rules
                .iter()
                .map(|rule| mdedit_text::Rule {
                    x: rule.x + indent,
                    ..*rule
                })
                .collect();

            lines.push(LayoutLine {
                baseline,
                height: line.height,
                glyphs,
                rules,
            });
            top += line.height;
        }
    }

    Layout { lines }
}

/// How many of a unit's rows are rows of the list.
///
/// All of them for the last unit. For any other, its trailing empty line is the newline that ends it
/// rather than a row — see [`assemble`].
fn rows_of(text: &Layout, last: bool) -> usize {
    if last {
        return text.lines.len();
    }
    let trailing = text
        .lines
        .last()
        .is_some_and(|line| line.glyphs.is_empty() && text.lines.len() > 1);
    text.lines.len() - usize::from(trailing)
}

/// How wide a laid-out marker is, which is the indent its item's text hangs at.
fn width_of(marker: &Layout) -> f32 {
    marker
        .glyphs()
        .map(|glyph| glyph.pos.x + glyph.advance)
        .fold(0.0_f32, f32::max)
}
