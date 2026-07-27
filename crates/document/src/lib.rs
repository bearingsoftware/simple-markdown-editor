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
//! let mut doc = Document::from("# Title\n\nSome **bold** text.\n");
//! doc.replace(9..13, "Other")?;
//!
//! let text = doc.to_string();
//! let rebuilt: String = doc.segments().iter().map(|s| &text[s.range()]).collect();
//! assert_eq!(rebuilt, text);
//! # Ok::<(), mdedit_document::EditError>(())
//! ```
//!
//! # Design notes
//!
//! **Offsets are bytes.** Every public range is a byte range into the buffer, matching
//! [`mdedit_markers`]. `ropey` indexes by character, so the two spaces coexist here — the conversion
//! is confined to [`Document::replace`] and appears nowhere else.
//!
//! **Segmentation is lazy.** [`Document::replace`] marks the segmentation stale;
//! [`Document::segments`] recomputes it if needed. A transaction of several edits therefore
//! re-segments once rather than once per edit.
//!
//! **Re-segmentation is a full re-parse**, per the measurement in
//! `docs/research/04-parsing-benchmarks.md`: at 60 KB it costs less than a frame, and incremental
//! parsing was measured to be *slower* than re-parsing for this workload.

use std::ops::Range;

use mdedit_markers::Segment;
use ropey::Rope;

/// Why an edit was rejected.
///
/// Both variants mean the caller computed a range that does not address this buffer, which is a
/// programmer error rather than a user one — but a text editor should not abort over it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    /// The range extended past the end of the buffer, or started after it ended.
    #[error("byte range {range:?} is outside the buffer of {len} bytes")]
    OutOfBounds {
        /// The offending range.
        range: Range<usize>,
        /// Buffer length in bytes at the time of the edit.
        len: usize,
    },
    /// An endpoint landed inside a multi-byte character.
    #[error("byte offset {offset} is not a character boundary")]
    NotCharBoundary {
        /// The offending endpoint.
        offset: usize,
    },
}

/// Markdown source, plus the marker/content segmentation derived from it.
#[derive(Debug, Clone)]
pub struct Document {
    text: Rope,
    /// `None` once an edit invalidates it; recomputed on the next read.
    segments: Option<Vec<Segment>>,
}

impl Document {
    /// An empty document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: Rope::new(),
            segments: None,
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

    /// The marker/content partition of the current buffer, recomputing it if an edit invalidated it.
    ///
    /// Takes `&mut self` because the segmentation is a cache: reading it may have to fill it. That is
    /// preferred over interior mutability, which would hide the cost.
    pub fn segments(&mut self) -> &[Segment] {
        if self.segments.is_none() {
            self.segments = Some(mdedit_markers::segment(&self.text.to_string()));
        }
        // Just populated above if it was absent.
        self.segments.as_deref().unwrap_or_default()
    }

    /// Replaces the bytes in `range` with `replacement`.
    ///
    /// The single editing primitive — [`insert`](Self::insert) and [`delete`](Self::delete) are
    /// spellings of it, so the bounds checking and cache invalidation live in one place.
    ///
    /// # Errors
    ///
    /// [`EditError::OutOfBounds`] if the range does not lie within the buffer, or
    /// [`EditError::NotCharBoundary`] if an endpoint splits a multi-byte character. The buffer is
    /// left untouched in both cases.
    pub fn replace(&mut self, range: Range<usize>, replacement: &str) -> Result<(), EditError> {
        self.check(&range)?;

        // The only place character indices exist. ropey addresses by character; every offset in this
        // crate's API is a byte offset, and mixing the two silently is the bug this containment
        // exists to prevent.
        let char_start = self.text.byte_to_char(range.start);
        let char_end = self.text.byte_to_char(range.end);

        if char_start != char_end {
            self.text.remove(char_start..char_end);
        }
        if !replacement.is_empty() {
            self.text.insert(char_start, replacement);
        }

        self.segments = None;
        Ok(())
    }

    /// Inserts `text` at a byte offset.
    ///
    /// # Errors
    ///
    /// As [`replace`](Self::replace).
    pub fn insert(&mut self, at: usize, text: &str) -> Result<(), EditError> {
        self.replace(at..at, text)
    }

    /// Removes the bytes in `range`.
    ///
    /// # Errors
    ///
    /// As [`replace`](Self::replace).
    pub fn delete(&mut self, range: Range<usize>) -> Result<(), EditError> {
        self.replace(range, "")
    }

    /// Rejects a range that does not address this buffer.
    fn check(&self, range: &Range<usize>) -> Result<(), EditError> {
        let len = self.len_bytes();
        if range.start > range.end || range.end > len {
            return Err(EditError::OutOfBounds {
                range: range.clone(),
                len,
            });
        }
        for offset in [range.start, range.end] {
            // `Rope::byte_to_char` saturates rather than reporting a split character, so the check
            // has to be explicit or a mid-character edit silently moves the edit point.
            if !self.is_char_boundary(offset) {
                return Err(EditError::NotCharBoundary { offset });
            }
        }
        Ok(())
    }

    /// Whether a byte offset sits between characters.
    fn is_char_boundary(&self, offset: usize) -> bool {
        if offset == 0 || offset == self.len_bytes() {
            return true;
        }
        let char_idx = self.text.byte_to_char(offset);
        self.text.char_to_byte(char_idx) == offset
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for Document {
    fn from(text: &str) -> Self {
        Self {
            text: Rope::from_str(text),
            segments: None,
        }
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
