//! A Markdown buffer that keeps its marker/content segmentation current across edits.
//!
//! [`mdedit_markers`] partitions a *fixed* string. This crate holds the text that is being edited and
//! keeps that partition true as it changes — the difference between an architecture that works at
//! rest and one that works while someone is typing into it.
//!
//! # The guarantee
//!
//! Everything [`mdedit_markers::segment`] promises still holds after any sequence of edits: the
//! segments tile the buffer exactly, and concatenating them reproduces it byte for byte.
//!
//! ```
//! # use mdedit_document::Document;
//! # use mdedit_offsets::{ByteOffset, ByteSpan};
//! let mut doc = Document::from("# Title\n\nSome **bold** text.\n");
//! doc.replace(
//!     ByteSpan::new(ByteOffset::new(9), ByteOffset::new(13)),
//!     "Other",
//! )?;
//!
//! let text = doc.to_string();
//! let rebuilt: String = doc.segments().iter().map(|s| s.span().slice(&text)).collect();
//! assert_eq!(rebuilt, text);
//! # Ok::<(), mdedit_document::EditError>(())
//! ```
//!
//! # Design notes
//!
//! **Offsets are bytes.** Every public position is a [`ByteOffset`] into the buffer, matching
//! [`mdedit_markers`]. `ropey` indexes by character, so [`CharOffset`] coexists here — the
//! conversion is confined to [`Document::replace`] and appears nowhere else, which is the whole
//! reason the two are different types.
//!
//! **Segmentation is recomputed on edit.** [`Document::segments`] is therefore a plain `&self`
//! read, which matters: a renderer needs the segments and the selection in the same loop, and a
//! lazily-filled cache would borrow the document mutably just to read it.
//!
//! A lazy cache would let a multi-edit transaction re-parse once instead of once per edit. That
//! optimisation is deliberately absent until a benchmark shows a transaction shape that needs it —
//! the same discipline MDEDIT-4 applied to segmentation itself.
//!
//! **Re-segmentation is a full re-parse**, per the measurement in
//! `docs/research/04-parsing-benchmarks.md`: at 60 KB it costs less than a frame, and incremental
//! parsing was measured to be *slower* than re-parsing for this workload.

use std::ops::Range;

use mdedit_markers::{Segment, SegmentKind};
use mdedit_offsets::{ByteOffset, ByteSpan, CharOffset, LineIndex};
use ropey::Rope;

/// Why an edit was rejected.
///
/// Both variants mean the caller computed a range that does not address this buffer, which is a
/// programmer error rather than a user one — but a text editor should not abort over it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    /// The span extended past the end of the buffer, or started after it ended.
    #[error("byte span {span} is outside the buffer of {len} bytes")]
    OutOfBounds {
        /// The offending span.
        span: ByteSpan,
        /// Buffer length in bytes at the time of the edit.
        len: usize,
    },
    /// An endpoint landed inside a multi-byte character.
    #[error("byte offset {offset} is not a character boundary")]
    NotCharBoundary {
        /// The offending endpoint.
        offset: ByteOffset,
    },
}

/// Markdown source, plus the marker/content segmentation derived from it.
#[derive(Debug, Clone)]
pub struct Document {
    text: Rope,
    /// Always current: recomputed by every edit.
    segments: Vec<Segment>,
}

impl Document {
    /// An empty document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: Rope::new(),
            segments: Vec::new(),
        }
    }

    /// Length of the buffer in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }

    /// Whether the buffer holds no text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len_bytes() == 0
    }

    /// The underlying rope, for callers that need line indexing or cheap slicing.
    #[must_use]
    pub fn rope(&self) -> &Rope {
        &self.text
    }

    /// The marker/content partition of the current buffer.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Replaces the bytes in `span` with `replacement`.
    ///
    /// The single editing primitive — [`insert`](Self::insert) and [`delete`](Self::delete) are
    /// spellings of it, so the bounds checking and cache invalidation live in one place.
    ///
    /// # Errors
    ///
    /// [`EditError::OutOfBounds`] if the span does not lie within the buffer, or
    /// [`EditError::NotCharBoundary`] if an endpoint splits a multi-byte character. The buffer is
    /// left untouched in both cases.
    pub fn replace(&mut self, span: ByteSpan, replacement: &str) -> Result<(), EditError> {
        self.check(span)?;

        // The only place [`CharOffset`] exists. ropey addresses by character, this crate's API
        // addresses by byte, and mixing the two silently is the bug this containment exists to
        // prevent — now with the type system enforcing that the conversion is written down.
        let char_start = self.byte_to_char(span.start());
        let char_end = self.byte_to_char(span.end());

        if char_start != char_end {
            self.text.remove(char_start.get()..char_end.get());
        }
        if !replacement.is_empty() {
            self.text.insert(char_start.get(), replacement);
        }

        self.resegment();
        Ok(())
    }

    /// Inserts `text` at a byte offset.
    ///
    /// # Errors
    ///
    /// As [`replace`](Self::replace).
    pub fn insert(&mut self, at: ByteOffset, text: &str) -> Result<(), EditError> {
        self.replace(ByteSpan::empty_at(at), text)
    }

    /// Removes the bytes in `span`.
    ///
    /// # Errors
    ///
    /// As [`replace`](Self::replace).
    pub fn delete(&mut self, span: ByteSpan) -> Result<(), EditError> {
        self.replace(span, "")
    }

    /// Recomputes the partition from the current buffer.
    fn resegment(&mut self) {
        self.segments = mdedit_markers::segment(&self.text.to_string());
    }

    /// Rejects a span that does not address this buffer.
    fn check(&self, span: ByteSpan) -> Result<(), EditError> {
        let len = self.len_bytes();
        if span.start() > span.end() || span.end().get() > len {
            return Err(EditError::OutOfBounds { span, len });
        }
        for offset in [span.start(), span.end()] {
            // `Rope::byte_to_char` saturates rather than reporting a split character, so the check
            // has to be explicit or a mid-character edit silently moves the edit point.
            if !self.is_char_boundary(offset) {
                return Err(EditError::NotCharBoundary { offset });
            }
        }
        Ok(())
    }

    /// Whether a byte offset sits between characters.
    fn is_char_boundary(&self, offset: ByteOffset) -> bool {
        if offset == ByteOffset::ZERO || offset.get() == self.len_bytes() {
            return true;
        }
        let char_idx = self.byte_to_char(offset);
        self.text.char_to_byte(char_idx.get()) == offset.get()
    }

    /// The rope character index at a byte offset.
    fn byte_to_char(&self, offset: ByteOffset) -> CharOffset {
        CharOffset::new(self.text.byte_to_char(offset.get()))
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for Document {
    fn from(text: &str) -> Self {
        let mut doc = Self {
            text: Rope::from_str(text),
            segments: Vec::new(),
        };
        doc.resegment();
        doc
    }
}

impl std::str::FromStr for Document {
    type Err = std::convert::Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(text))
    }
}

/// Writes the buffer's text.
///
/// This is what gives [`Document`] a `to_string()`. Segment ranges index into that string, so a
/// caller rendering the document wants to hold one rather than build it per segment.
impl std::fmt::Display for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for chunk in self.text.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

/// Where the caret is, and what is selected.
///
/// Offsets are bytes, as everywhere else in this crate. A caret is a selection whose ends coincide;
/// `anchor` is where the selection started and `head` is where it currently is, so `head` is the end
/// that moves and the two can be in either order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    anchor: ByteOffset,
    head: ByteOffset,
}

impl Selection {
    /// A collapsed selection — a caret at `at`.
    #[must_use]
    pub fn caret(at: ByteOffset) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    /// A selection from `anchor` to `head`. Either order is valid.
    #[must_use]
    pub fn new(anchor: ByteOffset, head: ByteOffset) -> Self {
        Self { anchor, head }
    }

    /// Where the selection started.
    #[must_use]
    pub fn anchor(self) -> ByteOffset {
        self.anchor
    }

    /// The moving end of the selection.
    #[must_use]
    pub fn head(self) -> ByteOffset {
        self.head
    }

    /// The selection as an ordered byte span.
    #[must_use]
    pub fn span(self) -> ByteSpan {
        if self.anchor <= self.head {
            ByteSpan::new(self.anchor, self.head)
        } else {
            ByteSpan::new(self.head, self.anchor)
        }
    }

    /// Whether this is a caret rather than a span.
    #[must_use]
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.head
    }
}

impl Document {
    /// Whether a segment's syntax should be shown, given where the caret is.
    ///
    /// This predicate is the whole live-preview illusion: markers are concealed until the caret is
    /// near them, then revealed so they can be edited. Content is always visible.
    ///
    /// Two rules, because Typora has two and conflating them is the usual mistake:
    ///
    /// - **Inline** markers reveal when the selection meets the marker's own span. A *caret* has no
    ///   width, so adjacency counts: resting immediately either side of `**` reveals it, and you can
    ///   delete a delimiter you have just typed past. A *range* selection must genuinely overlap —
    ///   selecting the word inside `**bold**` should not light up delimiters it does not contain.
    /// - **Block** markers reveal when the selection is on any line the marker's block occupies, so
    ///   `## ` stays visible while you edit the heading text rather than vanishing under the cursor.
    ///
    /// Total: out-of-range selections are clamped rather than rejected, because a stale caret should
    /// not be able to crash a render.
    ///
    /// # Known approximation
    ///
    /// [`mdedit_markers`] reports marker spans but not block extents, so "the block" is approximated
    /// by "the line". That is exact for headings, block quotes and list items, whose markers sit on
    /// every line they affect. It is wrong for a multi-line fenced code block: its fences reveal only
    /// when the caret is on a fence line, where Typora would show them from anywhere inside.
    #[must_use]
    pub fn is_revealed(&self, segment: &Segment, selection: Selection) -> bool {
        let SegmentKind::Marker(marker) = segment.kind() else {
            return true;
        };

        let seg = segment.span();
        let sel = selection.span();

        if marker.is_block() {
            let marker_lines = self.line_span(seg);
            let caret_lines = self.line_span(sel);
            marker_lines.start <= caret_lines.end && caret_lines.start <= marker_lines.end
        } else if selection.is_collapsed() {
            // Inclusive, so a caret touching either edge counts as inside.
            sel.touches(seg)
        } else {
            sel.overlaps(seg)
        }
    }

    /// The inclusive range of line indices a byte span covers.
    ///
    /// A `Range<LineIndex>` needs no span type of its own: nothing is ever indexed by a line range,
    /// so the orphan-rule problem that [`ByteSpan`] exists to solve does not arise.
    fn line_span(&self, span: ByteSpan) -> Range<LineIndex> {
        let len = self.len_bytes();
        let start = self.text.byte_to_line(span.start().get().min(len));
        let end = self.text.byte_to_line(span.end().get().min(len));
        LineIndex::new(start)..LineIndex::new(end)
    }
}
