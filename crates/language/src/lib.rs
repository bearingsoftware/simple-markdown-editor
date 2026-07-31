//! What the editor needs from a language, and the vocabulary it needs it in.
//!
//! # Why this crate exists
//!
//! Everything below the window used to assume one language: `mdedit-markers` called
//! `pulldown_cmark::Parser` directly, and `mdedit-document` called its `analyze` on every edit. So
//! opening a `.json` file was not a feature that plugged in — the boundary did not exist.
//!
//! This is that boundary. It holds the types a renderer reads and the [`Language`] trait a parser
//! implements, and it depends on nothing but `mdedit-offsets`. That last part is the point: a
//! dependency here is one every language inherits, so a Markdown parser named here would be one
//! that JSON had to compile in order to be JSON. Cargo's dependency graph is the allow-list.
//!
//! # The decision that shapes it: two rendering models, not N parsers
//!
//! Markdown conceals its syntax because the syntax is scaffolding for prose — nobody wants to look
//! at `**` while reading a sentence. JSON has no such scaffolding: the braces, brackets and quotes
//! **are** the document, and YAML's indentation is load-bearing. Concealing either produces
//! something that cannot be edited back into itself.
//!
//! So the boundary is not "which parser" but **which of two rendering models** — conceal-and-style,
//! or colour-only. That is what [`Language::conceals`] answers, and it is a far smaller and safer
//! abstraction than a general one would have been.
//!
//! # The three things a language provides
//!
//! - **Spans with a role**, tiling the buffer. The invariant is the same for every language and it
//!   is the gate, not a nicety: concatenating the segments must reproduce the source byte for byte.
//! - **Block extents**, with a kind and a depth, so the per-block layout and scoped re-parse built
//!   in MDEDIT-17 and MDEDIT-18 keep working unchanged.
//! - **Whether syntax is ever concealed**, which decides how it is drawn.

use mdedit_offsets::{BlockId, ByteSpan};

/// A run of bytes with a single role.
///
/// Produced by a [`Language`]; the spans of one analysis tile the whole input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    span: ByteSpan,
    kind: SegmentKind,
}

impl Segment {
    /// Builds a segment.
    ///
    /// Public for the same reason [`Block::new`] is: a language implementation is in another crate.
    #[must_use]
    pub fn new(span: ByteSpan, kind: SegmentKind) -> Self {
        Self { span, kind }
    }

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

    /// The same run, covering a different span.
    ///
    /// For a caller splicing two analyses together: an edit moves everything after it, and a run
    /// straddling the edge of a re-parsed region is trimmed rather than dropped, because segments
    /// tile and dropping one would leave its bytes covered by nothing.
    #[must_use]
    pub fn moved_to(mut self, span: ByteSpan) -> Self {
        self.span = span;
        self
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
    /// Builds a block.
    ///
    /// Public because a [`Language`] implementation lives in another crate and has to be able to
    /// produce one — that is what the trait is for.
    #[must_use]
    pub fn new(id: BlockId, kind: BlockKind, span: ByteSpan, depth: u16) -> Self {
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

    /// The same block, covering a different span. As [`Segment::moved_to`].
    #[must_use]
    pub fn moved_to(mut self, span: ByteSpan) -> Self {
        self.span = span;
        self
    }

    /// The same block under a different id.
    ///
    /// A [`BlockId`] numbers a block within one analysis, so a list spliced from two of them has to
    /// be renumbered or two blocks would answer to the same id.
    #[must_use]
    pub fn with_id(mut self, id: BlockId) -> Self {
        self.id = id;
        self
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

/// A language the editor can render.
///
/// One implementation per language, and the renderer holds a `&'static dyn Language` rather than a
/// type parameter — the language is chosen by a file's extension at runtime, so it cannot be a
/// generic without pushing that choice up through every type that touches a document.
///
/// The dispatch is one vtable call per parse. A parse is milliseconds; the call is nanoseconds.
pub trait Language: std::fmt::Debug + Send + Sync {
    /// What this language is called, for a status bar or a menu.
    fn name(&self) -> &'static str;

    /// Partitions `source` and reports its block structure, in one parse.
    ///
    /// The segments **must** tile `source` exactly: concatenating them reproduces it byte for byte.
    /// That is the invariant the whole decorate-the-source architecture rests on, and it is a gate
    /// rather than a nicety — a language whose segments do not tile is not renderable safely.
    fn analyze(&self, source: &str) -> Analysis;

    /// Analyses only `region`, as if the rest of `source` were not there, in `source`'s coordinates.
    ///
    /// `None` means **this language has no safe restart boundary here**, and the caller owes it a
    /// whole-document [`analyze`](Self::analyze) instead. That is not a failure: JSON nests without
    /// limit, so an edit deep inside an object may have no boundary short of the top, and a language
    /// that says so gets a correct parse that is merely slower. A language that pretended otherwise
    /// would get a fast wrong one.
    fn analyze_region(&self, source: &str, region: ByteSpan) -> Option<Analysis>;

    /// Whether this language's syntax is ever hidden.
    ///
    /// The whole rendering model in one answer. `true` conceals markers unless the caret is near
    /// them, which is what makes Markdown read as prose. `false` draws every byte always and colours
    /// them instead — the only honest choice for JSON, whose braces and quotes *are* the document,
    /// and for YAML, whose indentation is load-bearing.
    fn conceals(&self) -> bool;
}
