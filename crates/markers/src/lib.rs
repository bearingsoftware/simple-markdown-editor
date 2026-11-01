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
///
/// # A panic in the parser is contained here
///
/// `pulldown-cmark` 0.13.4 panics on some documents — `"+\t[\0]:d\n\u{b}"` is nine characters that
/// unwrap a `None` in its `parse.rs`, found by the fuzz job on MDEDIT-114 and reproduced on Windows.
/// It is upstream's defect and not this crate's, and it arrives the way every defect at this boundary
/// arrives: in a **file somebody opened**. `AGENTS.md` names file parsing as the trust boundary and
/// hand-rolled parsing as the CVE class this project exists not to reintroduce; taking a third-party
/// parser's word for it and crashing the editor is the same failure with somebody else's name on it.
///
/// So the parse is contained, and a document that panics it comes back as **one content segment
/// covering the whole source**. That tiles — which is the invariant everything above depends on — and
/// it renders as plain text, which is a Markdown file drawn without its syntax highlighted rather
/// than an editor that will not open it. The bytes are untouched either way, because nothing here
/// ever rewrites them.
///
/// It is not free and it is not on the keystroke path for a large document: `catch_unwind` costs
/// nothing when nothing unwinds, and `analyze_region` is what a keystroke reaches.
///
/// # It stays when upstream fixes it
///
/// Reported as [pulldown-cmark#1129]. Two things were worth establishing before filing and are worth
/// keeping here: **only `into_offset_iter()` panics** — plain iteration over the same document is
/// fine, which is why nothing but this crate sees it — and **no options are required**, so it is not
/// something a configuration change avoids.
///
/// When it is fixed, this containment does not come out. Upstream fixed #1129 in 0.13.5, and the
/// containment stayed. It is not a workaround for one defect; it is what a third-party parser at a
/// trust boundary gets, and the next panic will not announce itself either. **It depends on
/// unwinding**: a `panic = "abort"` profile turns the crash back on silently, which is why
/// `Cargo.toml` says so beside the release profile.
///
/// [pulldown-cmark#1129]: https://github.com/pulldown-cmark/pulldown-cmark/issues/1129
#[must_use]
pub fn analyze(source: &str) -> Analysis {
    contained(source)
}

/// [`classify::analyze`], with a parser panic turned into "it is all content".
///
/// See [`analyze`] for why this exists at all. `AssertUnwindSafe` is sound here because the closure
/// borrows `source` immutably and this crate keeps no state across the call — a panicking parse
/// leaves nothing of ours half-written, and the fallback reads only `source.len()`.
fn contained(source: &str) -> Analysis {
    contain(source, classify::analyze)
}

fn contain(source: &str, parse: impl FnOnce(&str) -> Analysis) -> Analysis {
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(source)));
    match parsed {
        Ok(analysis) => analysis,
        Err(_) => all_content(source),
    }
}

/// The whole source as one content segment, and no blocks.
///
/// The safe answer for a document the parser could not handle: it tiles exactly, so every caller's
/// invariant holds, and the renderer draws plain text.
fn all_content(source: &str) -> Analysis {
    let segments = if source.is_empty() {
        Vec::new()
    } else {
        vec![Segment::new(
            ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
            SegmentKind::Content,
        )]
    };
    Analysis {
        segments,
        blocks: Vec::new(),
    }
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

    // Contained, like [`analyze`] — a region of a document that panics the parser is exactly as
    // likely to panic it, and this is the one a keystroke reaches.
    let mut analysis = contained(&source[start..end]);
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

/// The parser options this crate analyses with, and an analysis that takes them (MDEDIT-131).
///
/// **Not API. It exists so a benchmark can turn one parser option on and off inside a single
/// process**, which is the only honest way to compare the two on a machine whose clock speed drifts
/// between runs — the failure that left MDEDIT-126 unable to report a parse cost at all. Behind a
/// feature rather than `#[doc(hidden)]` so it cannot be reached by accident from the editor.
///
/// **Production has exactly one option set** and [`options`](bench::options) is it. Nothing here
/// makes the set configurable, and a diff that reads an option from a setting is a different
/// decision on a different ticket.
#[cfg(feature = "bench")]
pub mod bench {
    use mdedit_language::Analysis;
    use pulldown_cmark::Options;

    /// What [`crate::analyze`] parses with.
    #[must_use]
    pub fn options() -> Options {
        crate::classify::options()
    }

    /// [`crate::analyze`], with the parser options given rather than the ones production uses.
    ///
    /// Unlike `analyze` this is **not** wrapped against a parser panic: the containment exists for
    /// documents a user opened, and a benchmark's corpus is one this repo wrote.
    #[must_use]
    pub fn analyze_with(source: &str, options: Options) -> Analysis {
        crate::classify::analyze_with(source, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANICKING_SOURCE: &str = "+\t[\0]:d\n\u{b}";

    #[test]
    #[allow(clippy::panic, reason = "the parser under test panics on purpose")]
    fn a_parse_that_panics_comes_back_as_one_content_segment() {
        let analysis = contain(PANICKING_SOURCE, |_| panic!("upstream"));

        assert_eq!(analysis.segments.len(), 1, "{:?}", analysis.segments);
        assert_eq!(
            analysis.segments[0].span(),
            ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(PANICKING_SOURCE.len()))
        );
        assert_eq!(analysis.segments[0].kind(), SegmentKind::Content);
        assert!(analysis.blocks.is_empty(), "{:?}", analysis.blocks);
    }

    #[test]
    #[allow(clippy::panic, reason = "the parser under test panics on purpose")]
    fn an_empty_document_that_panics_comes_back_with_no_segments() {
        let analysis = contain("", |_| panic!("upstream"));

        assert!(analysis.segments.is_empty(), "{:?}", analysis.segments);
        assert!(analysis.blocks.is_empty(), "{:?}", analysis.blocks);
    }

    #[test]
    fn a_parse_that_returns_is_passed_through() {
        let source = "# Title\n\nSome **bold** text.\n";

        assert_eq!(
            contain(source, classify::analyze),
            classify::analyze(source)
        );
    }
}
