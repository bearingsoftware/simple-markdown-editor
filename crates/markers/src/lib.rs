//! Partitions Markdown source into syntax-marker and content spans.
//!
//! The editor renders Markdown by *decorating the source* rather than converting it to a document
//! model: the buffer holds literal Markdown and syntax is concealed when the cursor is elsewhere.
//! That needs the byte range of each individual delimiter — the `**`, the `#`, the `>` — which no
//! Rust CommonMark parser reports. [`pulldown_cmark::Tag::Emphasis`] is a fieldless variant;
//! comrak's `Emph` carries no delimiter; mdast has no mark nodes.
//!
//! # The guarantee
//!
//! [`segment`] returns a **total partition** of the input. Segments are ordered, contiguous and
//! non-overlapping, and together cover exactly `0..source.len()`. Concatenating them reproduces the
//! source byte for byte:
//!
//! ```
//! # use mdedit_markers::segment;
//! let src = "## Heading\n\nSome **bold** text.\n";
//! let rebuilt: String = segment(src).iter().map(|s| s.span().slice(src)).collect();
//! assert_eq!(rebuilt, src);
//! ```
//!
//! That property is what makes the decorate-the-source architecture safe: a renderer can conceal
//! every [`SegmentKind::Marker`] span and still be holding the user's exact bytes.
//!
//! # How markers are found
//!
//! Not by scanning for delimiters per construct. `pulldown-cmark` reports both the range of a
//! construct and the ranges of its content, so **the markers are the difference**:
//!
//! ```text
//! 0..8  Start(Strong)  "**bold**"
//! 2..6  Text           "bold"
//!       => markers 0..2 and 6..8
//! ```
//!
//! Subtraction handles nested emphasis, backslash escapes, and link/image titles without needing the
//! offsets that [pulldown-cmark#441] does not expose.
//!
//! [pulldown-cmark#441]: https://github.com/pulldown-cmark/pulldown-cmark/issues/441

use mdedit_language::Language;
use mdedit_offsets::{ByteOffset, ByteSpan};

pub use mdedit_language::{Analysis, Block, BlockKind, Marker, Segment, SegmentKind};

mod classify;

/// Markdown, as this editor renders it.
///
/// The implementation of [`Language`] that every other part of the editor used to assume — see the
/// [`mdedit_language`] crate docs for why that assumption had to be given a name before a second
/// language could exist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Markdown;

/// The one Markdown there is, for a caller holding a `&'static dyn Language`.
pub static MARKDOWN: Markdown = Markdown;

impl Language for Markdown {
    fn name(&self) -> &'static str {
        "Markdown"
    }

    fn analyze(&self, source: &str) -> Analysis {
        analyze(source)
    }

    fn analyze_region(&self, source: &str, region: ByteSpan) -> Option<Analysis> {
        // Markdown always has one: a top-level block boundary is a safe place to restart, which is
        // what MDEDIT-18 is built on. The caller still owes a whole-document parse when the typing
        // stops, because a region cannot see an unclosed fence above it — see `analyze_region`.
        Some(analyze_region(source, region))
    }

    fn conceals(&self) -> bool {
        // The whole reason the editor exists. Syntax is scaffolding for prose, so it is hidden
        // unless the caret is near it.
        true
    }
}
/// Partitions `source` and reports its block structure, in one parse.
#[must_use]
pub fn analyze(source: &str) -> Analysis {
    classify::analyze(source)
}

/// Analyses only `region`, as if the rest of `source` were not there.
///
/// The spans come back in `source`'s coordinates, so the result splices into a whole-document
/// analysis. Segments tile `region` exactly, as they tile the whole document in [`analyze`].
///
/// # What this can and cannot know
///
/// The region is parsed **on its own**. That is exactly right for a region that starts and ends on a
/// top-level block boundary and contains no construct left open at either edge, which is what a
/// caller should pass — and it is what makes an edit cost the size of a paragraph rather than the
/// size of the file.
///
/// It is *not* right when the surrounding text changes what the region means: an unclosed fence
/// above it makes the whole region code, and a link reference definition anywhere in the document
/// decides whether `[a]` in the region is a link. A caller that uses this on the keystroke path
/// therefore owes the document a whole-document [`analyze`] when the typing stops — which is what
/// `mdedit_document`'s `Document::reconcile` is for.
///
/// A region outside the source, or one whose ends are not character boundaries, is clamped and
/// snapped rather than rejected: a stale viewport must not be able to panic a render.
#[must_use]
pub fn analyze_region(source: &str, region: ByteSpan) -> Analysis {
    let start = floor_boundary(source, region.start().get().min(source.len()));
    let end = floor_boundary(source, region.end().get().clamp(start, source.len()));

    let mut analysis = classify::analyze(&source[start..end]);
    if start == 0 {
        return analysis;
    }
    for segment in &mut analysis.segments {
        *segment = segment.clone().moved_to(shift(segment.span(), start));
    }
    for block in &mut analysis.blocks {
        *block = block.moved_to(shift(block.span(), start));
    }
    analysis
}

fn shift(span: ByteSpan, by: usize) -> ByteSpan {
    ByteSpan::new(
        ByteOffset::new(span.start().get() + by),
        ByteOffset::new(span.end().get() + by),
    )
}

/// The character boundary at or before `at`.
fn floor_boundary(source: &str, at: usize) -> usize {
    let mut at = at;
    while at > 0 && !source.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Partitions `source` into ordered marker and content runs.
///
/// The returned segments tile the whole of `source` exactly — see the [crate docs](crate) for the
/// guarantee and an example. Never panics, and never returns an empty segment.
///
/// Constructs this crate does not yet classify (HTML blocks, entity references, link reference
/// definitions) are returned as [`SegmentKind::Content`]; their bytes are still present.
#[must_use]
pub fn segment(source: &str) -> Vec<Segment> {
    analyze(source).segments
}
