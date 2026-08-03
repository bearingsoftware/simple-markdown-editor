//! Where the document is allowed to draw, decided once.
//!
//! # Why this exists
//!
//! Chrome takes space out of the window. The tab strip takes a strip off the top; a menu bar, a
//! status bar, a file pane and a line-number gutter each want their own. Everything left over is
//! where the text goes.
//!
//! That sounds like arithmetic too small to name, and it was, until four features wanted to do it at
//! once. The document's position was computed independently in nine places — the wrapping width, the
//! glyph positions, the caret, the selection highlight, the pre-edit, and the reverse mapping that
//! turns a click back into a byte. Each new region meant nine edits, and the failure mode when one
//! is missed is silent: the text draws in one place and the clicks land in another.
//!
//! So the offset is computed here, once, and callers cannot apply it themselves — [`to_window`] and
//! [`to_document`] are the only way across, and they are inverses. A click and a glyph disagreeing
//! is then not a bug that review has to catch; it is a thing the types do not let you write.
//!
//! [`to_window`]: ContentRect::to_window
//! [`to_document`]: ContentRect::to_document

/// Space chrome has taken out of the window, in device-independent pixels.
///
/// Deliberately not named per feature — this is "how much is gone from each edge", so a status bar
/// and a horizontal scrollbar both just add to `bottom` without this type learning about either.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Insets {
    /// Taken off the top: the tab strip today, a menu bar later.
    pub top: f32,
    /// Taken off the bottom: a status bar.
    pub bottom: f32,
    /// Taken off the left: a file pane, a line-number gutter.
    pub left: f32,
    /// Taken off the right.
    pub right: f32,
}

/// How far the viewport has moved over the document, on both axes.
///
/// One type rather than two `f32` parameters, and that is the whole of MDEDIT-49's safety argument:
/// the conversions used to take a bare vertical scroll, so **every caller that had not thought about
/// horizontal scrolling silently kept working and drew in the wrong place**. Making it a pair means
/// the compiler asks each of them the question once.
///
/// `x` is zero whenever the text wraps, which is most of the time — there is nothing off to the side
/// to reach.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Offset {
    /// How far right the viewport has panned.
    pub x: f32,
    /// How far down the document the viewport has moved.
    pub y: f32,
}

impl Offset {
    /// A vertical-only offset, which is every offset while the text wraps.
    #[must_use]
    pub fn down(y: f32) -> Self {
        Self { x: 0.0, y }
    }
}

/// The document's drawable area, and the only thing that knows where it is.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ContentRect {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl ContentRect {
    /// Works out what is left of a window once chrome and the margin have taken their share.
    ///
    /// `width` and `height` are the window's, in device-independent pixels. The margin is inside the
    /// insets: chrome sits against the window edge, and the text is inset from whatever chrome
    /// leaves.
    #[must_use]
    pub fn new(width: f32, height: f32, margin: f32, insets: Insets) -> Self {
        let left = insets.left + margin;
        let top = insets.top + margin;
        Self {
            left,
            top,
            // At least a pixel each way. A window dragged to nothing, or chrome that claims more
            // than there is, must not produce a zero or negative wrapping width — the text engine
            // is entitled to reject that, and a render is not the place to find out.
            width: (width - left - margin - insets.right).max(1.0),
            height: (height - top - insets.bottom).max(1.0),
        }
    }

    /// How wide text may be before it wraps.
    #[must_use]
    pub fn width(self) -> f32 {
        self.width
    }

    /// How much of the document is on screen at once.
    #[must_use]
    pub fn height(self) -> f32 {
        self.height
    }

    /// Where a point in the document lands in the window, at a given scroll offset.
    #[must_use]
    pub fn to_window(self, x: f32, y: f32, at: Offset) -> (f32, f32) {
        (x + self.left - at.x, y + self.top - at.y)
    }

    /// Where a point in the window lands in the document. The inverse of [`Self::to_window`].
    #[must_use]
    pub fn to_document(self, x: f32, y: f32, at: Offset) -> (f32, f32) {
        (x - self.left + at.x, y - self.top + at.y)
    }

    /// Where the text starts, measured from the window's left edge.
    ///
    /// The one thing outside this type that needs the raw number: a gutter draws to the left of it,
    /// which is the one thing that is not in document coordinates at all.
    #[must_use]
    pub fn left_edge(self) -> f32 {
        self.left
    }

    /// Where the document starts down the window, chrome and margin included.
    ///
    /// The scrollbar wants this: its track runs beside the document rather than beside the window,
    /// so a tab strip above the text pushes the top of the bar down with it.
    #[must_use]
    pub fn top_edge(self) -> f32 {
        self.top
    }

    /// Whether a window point is inside the document's area rather than over chrome.
    #[must_use]
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.left && y >= self.top && x < self.left + self.width && y < self.top + self.height
    }

    /// The band of the document that is on screen, in document coordinates.
    ///
    /// What culling compares against, so that the check happens in the same space the layout is in
    /// rather than being converted to the window and back.
    ///
    /// Vertical only, deliberately. Rows are culled because a document is far taller than a window
    /// and drawing all of it is the large-file freeze; a row is never wider than a screen by enough
    /// for per-glyph culling to pay for the test, and MDEDIT-49 did not change that.
    #[must_use]
    pub fn visible(self, at: Offset) -> (f32, f32) {
        (at.y, at.y + self.height)
    }
}
