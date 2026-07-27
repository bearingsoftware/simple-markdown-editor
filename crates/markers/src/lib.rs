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
//! let rebuilt: String = segment(src).iter().map(|s| &src[s.range()]).collect();
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

use std::ops::Range;

mod classify;

/// A run of bytes with a single role.
///
/// Produced only by [`segment`]; the ranges of a returned slice tile the whole input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    range: Range<usize>,
    kind: SegmentKind,
}

impl Segment {
    /// Byte range within the source this segment was produced from.
    ///
    /// Always non-empty and always on UTF-8 character boundaries, so it is safe to slice with.
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Whether this run is syntax to conceal, or content to show.
    #[must_use]
    pub fn kind(&self) -> SegmentKind {
        self.kind
    }

    /// Whether this run is syntax the renderer may conceal.
    #[must_use]
    pub fn is_marker(&self) -> bool {
        matches!(self.kind, SegmentKind::Marker(_))
    }
}

/// The role a [`Segment`] plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SegmentKind {
    /// Markdown syntax. The renderer conceals this unless the cursor is inside it.
    Marker(Marker),
    /// Everything else — rendered text, and any construct not yet classified.
    Content,
}

/// Which construct a marker belongs to.
///
/// Non-exhaustive: constructs are added as the editor learns to render them, and an unrecognised
/// construct degrades to [`SegmentKind::Content`] rather than being dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Marker {
    /// `*` or `_` around emphasis.
    Emphasis,
    /// `**` or `__` around strong emphasis.
    Strong,
    /// `~~` around struck-through text.
    Strikethrough,
    /// Backtick run around an inline code span.
    CodeSpan,
    /// Fence and info string of a fenced code block, or the indent of an indented one.
    CodeBlock,
    /// Leading `#` run of an ATX heading, its trailing run, or a setext underline.
    Heading,
    /// `>` at the start of a block-quote line.
    BlockQuote,
    /// `-`, `*`, `+` or `1.` starting a list item.
    ListItem,
    /// A thematic break — `---`, `***`, `___`.
    ThematicBreak,
    /// Link syntax: `[`, `](`, the destination, the title, `)`.
    Link,
    /// Image syntax, as [`Marker::Link`] but including the leading `!`.
    Image,
    /// The backslash of a backslash escape.
    Escape,
    /// The trailing spaces or backslash forming a hard line break.
    HardBreak,
}

impl Marker {
    /// Whether this marker belongs to a block-level construct rather than an inline one.
    ///
    /// The distinction drives concealment. Inline syntax is revealed when the caret touches the
    /// marker itself; block syntax is revealed while the caret is anywhere in the block, which is
    /// how Typora behaves and why `## ` stays visible while you are editing the heading text.
    ///
    /// An unrecognised marker counts as inline: revealing too little is a cosmetic fault, whereas
    /// revealing a whole block's syntax unexpectedly is a distracting one.
    #[must_use]
    pub fn is_block(self) -> bool {
        matches!(
            self,
            Self::CodeBlock
                | Self::Heading
                | Self::BlockQuote
                | Self::ListItem
                | Self::ThematicBreak
        )
    }
}

/// Partitions `source` into ordered marker and content runs.
///
/// The returned segments tile `0..source.len()` exactly — see the [crate docs](crate) for the
/// guarantee and an example. Never panics, and never returns an empty segment.
///
/// Constructs this crate does not yet classify (HTML blocks, entity references, link reference
/// definitions) are returned as [`SegmentKind::Content`]; their bytes are still present.
#[must_use]
pub fn segment(source: &str) -> Vec<Segment> {
    classify::segment(source)
}
