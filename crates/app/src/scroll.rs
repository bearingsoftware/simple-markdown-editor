//! Where the viewport sits over the document.
//!
//! Arithmetic only — no GPU, text or window types, for the same reason the tab strip's geometry is
//! separate: a scroll offset that clamps wrongly strands the last line off screen or lets the
//! document drift into empty space, and neither is obvious from a screenshot until someone hits it.
//!
//! Everything is in **device-independent pixels**, measured from the top of the laid-out document.
//! Zero means the first line is at the top of the viewport.

/// How far the viewport has moved down the document.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Scroll {
    offset: f32,
}

/// How far one wheel notch moves the document.
///
/// Three lines is the Windows default, and matching it matters more than picking a "nicer" number:
/// a wheel that moves a different amount from every other application feels broken rather than
/// distinctive.
pub const LINES_PER_NOTCH: f32 = 3.0;

impl Scroll {
    /// At the top.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Distance from the top of the document to the top of the viewport.
    #[must_use]
    pub fn offset(self) -> f32 {
        self.offset
    }

    /// The furthest down the viewport may go.
    ///
    /// Zero when the document is shorter than the viewport: there is nothing below to reveal, and
    /// allowing a positive offset there would let the text scroll up off the top into blank space.
    #[must_use]
    pub fn max_offset(content_height: f32, viewport_height: f32) -> f32 {
        (content_height - viewport_height).max(0.0)
    }

    /// Moves by `delta` device-independent pixels, positive being further down the document.
    pub fn by(&mut self, delta: f32, content_height: f32, viewport_height: f32) {
        self.set(self.offset + delta, content_height, viewport_height);
    }

    /// Moves a whole viewport, less one line of overlap so the reader keeps their place.
    pub fn page(
        &mut self,
        pages: f32,
        line_height: f32,
        content_height: f32,
        viewport_height: f32,
    ) {
        let stride = (viewport_height - line_height).max(line_height);
        self.by(pages * stride, content_height, viewport_height);
    }

    /// Jumps to the top.
    pub fn to_top(&mut self) {
        self.offset = 0.0;
    }

    /// Jumps to the bottom.
    pub fn to_bottom(&mut self, content_height: f32, viewport_height: f32) {
        self.offset = Self::max_offset(content_height, viewport_height);
    }

    /// Sets the offset, clamped to the document.
    pub fn set(&mut self, offset: f32, content_height: f32, viewport_height: f32) {
        // `clamp` would panic if max were NaN or below zero; `max_offset` guarantees neither, and
        // ordering the operations this way keeps a NaN input from surviving as an offset.
        let max = Self::max_offset(content_height, viewport_height);
        self.offset = offset.max(0.0).min(max);
    }

    /// Scrolls the least amount that brings `top..bottom` fully into view.
    ///
    /// The least amount, deliberately: a caret that is already visible must not move the document
    /// at all, or every keystroke would jerk the page. Only when the caret has left the viewport
    /// does it pull the minimum distance needed to bring it back.
    ///
    /// A region taller than the viewport aligns to its top, because the caret sits at the top of
    /// the line it is on and losing sight of it is worse than losing sight of the line's descender.
    pub fn reveal(&mut self, top: f32, bottom: f32, content_height: f32, viewport_height: f32) {
        let target = if top < self.offset {
            top
        } else if bottom > self.offset + viewport_height {
            bottom - viewport_height
        } else {
            return;
        };
        self.set(target, content_height, viewport_height);
    }
}
