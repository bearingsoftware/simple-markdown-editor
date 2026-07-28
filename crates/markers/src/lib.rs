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

use mdedit_offsets::{BlockId, ByteSpan};

mod classify;

/// A run of bytes with a single role.
///
/// Produced only by [`segment`]; the spans of a returned slice tile the whole input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    span: ByteSpan,
    kind: SegmentKind,
}

impl Segment {
    /// Byte span within the source this segment was produced from.
    ///
    /// Always non-empty and always on UTF-8 character boundaries, so it is safe to slice with.
    #[must_use]
    pub fn span(&self) -> ByteSpan {
        self.span
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

/// What kind of block a [`Block`] is.
///
/// Only the distinctions the editor acts on. Anything else is [`BlockKind::Other`] rather than a
/// variant nobody reads — the list grows when a renderer needs a case, not before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BlockKind {
    /// A run of prose.
    Paragraph,
    /// An ATX or setext heading, carrying its level, 1–6.
    Heading(u8),
    /// A fenced or indented code block.
    ///
    /// The one kind whose markers do **not** repeat on every line, which is why block extents had
    /// to exist at all — see [`Block`].
    CodeBlock,
    /// A block quote. Contains other blocks.
    BlockQuote,
    /// A bullet or ordered list. Contains items.
    List,
    /// One item of a list. Contains other blocks.
    ListItem,
    /// A thematic break — `---`, `***`, `___`.
    ThematicBreak,
    /// A raw HTML block.
    Html,
    /// Anything not distinguished yet. Its bytes are still covered.
    Other,
}

/// One block-level construct, and the bytes it spans.
///
/// # Why this exists
///
/// Most block markers repeat on every line they affect: a block quote writes `>` on each line, a
/// list item indents each of its continuation lines. For those, the line a marker sits on *is* its
/// block for every purpose the editor has, and treating the whole construct as one unit would
/// reveal twenty `>` because the caret is on the first line.
///
/// Fenced code is the exception, and the reason for this type: `` ``` `` appears twice, at the top
/// and the bottom, and everything between belongs to the block without carrying a marker. Deciding
/// anything about the middle of a fence from its own line is impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    id: BlockId,
    kind: BlockKind,
    span: ByteSpan,
    depth: u16,
}

impl Block {
    pub(crate) fn new(id: BlockId, kind: BlockKind, span: ByteSpan, depth: u16) -> Self {
        Self {
            id,
            kind,
            span,
            depth,
        }
    }

    /// This block's identity within the analysis that produced it.
    #[must_use]
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// What kind of block this is.
    #[must_use]
    pub fn kind(&self) -> BlockKind {
        self.kind
    }

    /// The bytes this block covers, as `pulldown-cmark` reports them.
    #[must_use]
    pub fn span(&self) -> ByteSpan {
        self.span
    }

    /// How deeply nested this block is. Zero is top level.
    #[must_use]
    pub fn depth(&self) -> u16 {
        self.depth
    }
}

/// Everything one parse of a document yields.
///
/// Segments and blocks come from a **single** walk of the parser. Producing them separately would
/// double the parse, and the parse happens on every keystroke — `docs/research/04` measures it at
/// 2.99 ms for 60 KB against a 3 ms budget, so there is no room for a second one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Analysis {
    /// The marker/content partition. Tiles the source exactly.
    pub segments: Vec<Segment>,
    /// Block-level structure, outermost first, in document order.
    ///
    /// Unlike segments these do **not** tile the source: the blank line between two paragraphs
    /// belongs to neither. Gaps are expected, overlaps are not, except where a block nests inside
    /// another and both cover the same bytes at different depths.
    pub blocks: Vec<Block>,
}

/// Partitions `source` and reports its block structure, in one parse.
#[must_use]
pub fn analyze(source: &str) -> Analysis {
    classify::analyze(source)
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
