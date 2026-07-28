//! Distinct types for the coordinate spaces this editor carries at once.
//!
//! Source bytes, rope characters, line indices and — later — laid-out glyph positions and block
//! indices are all `usize` underneath, and passing one where another is expected compiles silently.
//! `AGENTS.md` names that as the likeliest bug class in the project, so the spaces are separate
//! types and every conversion is written down.
//!
//! This crate has no dependencies and knows nothing about Markdown. The text layer needs
//! [`ByteOffset`] to back an `ITextStoreACP`; it has no business depending on a parser to say so.
//!
//! # Spans, not ranges
//!
//! [`ByteSpan`] is a struct rather than a `Range<ByteOffset>` because a range of a newtype cannot
//! index a `str` — `impl Index<Range<ByteOffset>> for str` is barred by the orphan rule. Slicing
//! goes through [`ByteSpan::slice`] instead:
//!
//! ```
//! # use mdedit_offsets::{ByteOffset, ByteSpan};
//! let span = ByteSpan::new(ByteOffset::new(5), ByteOffset::new(9));
//! assert_eq!(span.slice("some bold text"), "bold");
//! ```
//!
//! [`LineIndex`] needs no such treatment: nothing is ever indexed by a line range, so
//! `Range<LineIndex>` works as it stands.
//!
//! # Layout space is not pixel space
//!
//! [`LayoutPos`] is in **device-independent pixels**. The physical pixels a GPU draws to are a
//! different space, separated by the monitor's scale factor, and on Windows that factor changes
//! while the program runs — `WM_DPICHANGED` fires when a window is dragged between monitors. Mixing
//! the two is the same class of bug as mixing bytes with chars, so they are the same kind of type.

use std::fmt;
use std::ops::Range;

/// A byte position in the buffer.
///
/// Always on a UTF-8 character boundary when it comes from this workspace's APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteOffset(usize);

impl ByteOffset {
    /// The start of the buffer.
    pub const ZERO: Self = Self(0);

    /// Wraps a raw byte position.
    #[must_use]
    pub const fn new(offset: usize) -> Self {
        Self(offset)
    }

    /// The underlying byte position.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl fmt::Display for ByteOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A character position in the rope.
///
/// `ropey` addresses by character, this workspace addresses by byte. This type exists so the two
/// cannot be confused at the one place they meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CharOffset(usize);

impl CharOffset {
    /// Wraps a raw character position.
    #[must_use]
    pub const fn new(offset: usize) -> Self {
        Self(offset)
    }

    /// The underlying character position.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

/// A zero-based line number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LineIndex(usize);

impl LineIndex {
    /// Wraps a raw line number.
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The underlying line number.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

/// Identifies one block within a single analysis of a document.
///
/// Stable only for as long as the document is unedited: it is a position in the block list, not a
/// durable name. That is enough for what it is for — keying a cached layout for the duration of a
/// frame, and telling two blocks apart — and pretending otherwise would invite a stale id to
/// address the wrong block after an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(u32);

impl BlockId {
    /// Wraps a block index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The underlying index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block#{}", self.0)
    }
}

/// A position in laid-out text, in device-independent pixels.
///
/// The origin is the top-left of the laid-out block; `y` grows downward, matching every text layout
/// API and both graphics APIs this project targets.
///
/// **Not physical pixels.** Multiply by the monitor's scale factor at the point of drawing, and
/// nowhere else — see the [crate docs](crate).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutPos {
    /// Horizontal offset from the left edge of the laid-out block.
    pub x: f32,
    /// Vertical offset from the top edge of the laid-out block.
    pub y: f32,
}

impl LayoutPos {
    /// The top-left corner.
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    /// A position at `(x, y)`.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// This position converted to physical pixels for a monitor at `scale_factor`.
    ///
    /// The one place the two spaces are allowed to meet.
    #[must_use]
    pub fn to_physical(self, scale_factor: f32) -> (f32, f32) {
        (self.x * scale_factor, self.y * scale_factor)
    }
}

impl fmt::Display for LayoutPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// A half-open run of bytes, `start..end`.
///
/// Carries no reference to the text it addresses, exactly like a `Range` — a span outliving the
/// buffer it came from is still a caller error, just a typed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ByteSpan {
    start: ByteOffset,
    end: ByteOffset,
}

impl ByteSpan {
    /// A span from `start` to `end`.
    ///
    /// `start` should not exceed `end`. This is not enforced, matching `Range`: the buffer owns
    /// bounds checking, and rejecting an inverted span here would move that error to the wrong
    /// place. [`slice`](Self::slice) panics on one, as slicing with an inverted `Range` does.
    #[must_use]
    pub const fn new(start: ByteOffset, end: ByteOffset) -> Self {
        Self { start, end }
    }

    /// The empty span at `at`, as a caret occupies.
    #[must_use]
    pub const fn empty_at(at: ByteOffset) -> Self {
        Self { start: at, end: at }
    }

    /// First byte of the span.
    #[must_use]
    pub const fn start(self) -> ByteOffset {
        self.start
    }

    /// One past the last byte of the span.
    #[must_use]
    pub const fn end(self) -> ByteOffset {
        self.end
    }

    /// Length in bytes.
    #[must_use]
    pub const fn len(self) -> usize {
        self.end.get().saturating_sub(self.start.get())
    }

    /// Whether the span covers no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The bytes this span addresses within `text`.
    ///
    /// # Panics
    ///
    /// If the span is inverted, extends past the end of `text`, or splits a multi-byte character —
    /// the same conditions under which slicing with a `Range` panics.
    #[must_use]
    pub fn slice(self, text: &str) -> &str {
        &text[self.range()]
    }

    /// Whether the two spans share at least one byte.
    ///
    /// Empty spans never overlap anything, including each other — an empty span contains no bytes
    /// to share, even sitting inside another. The emptiness check is not redundant with the
    /// comparisons: without it, `2..2` "overlaps" `0..4`. Use [`touches`](Self::touches) where a
    /// caret should count as inside.
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.start.get() < other.end.get()
            && other.start.get() < self.end.get()
    }

    /// Whether the two spans share a byte *or* meet at an edge.
    ///
    /// The distinction is the whole of the reveal predicate: a caret resting immediately after `**`
    /// touches it without overlapping it, and still has to be able to delete it.
    #[must_use]
    pub const fn touches(self, other: Self) -> bool {
        self.start.get() <= other.end.get() && other.start.get() <= self.end.get()
    }

    /// The equivalent raw byte range.
    ///
    /// A deliberate exit from the type, for the two places that need one: slicing, and `ropey`'s
    /// character conversions.
    #[must_use]
    pub fn range(self) -> Range<usize> {
        self.start.get()..self.end.get()
    }
}

impl From<Range<usize>> for ByteSpan {
    /// A deliberate entry into the type, for boundaries where offsets arrive raw — `pulldown-cmark`
    /// event ranges, and test fixtures.
    fn from(range: Range<usize>) -> Self {
        Self::new(ByteOffset::new(range.start), ByteOffset::new(range.end))
    }
}

impl fmt::Display for ByteSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}
