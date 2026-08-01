//! The vertical scrollbar: where the thumb sits, and what a click on the bar means.
//!
//! Geometry only — no GPU types, no text engine, no window — for the reason `tabs.rs` gives: this is
//! arithmetic that is easy to get subtly wrong and impossible to check by looking at a screenshot.
//!
//! Everything here is in **device-independent pixels**.
//!
//! # The thumb wobbles, and that is the design working
//!
//! `AGENTS.md`: off-screen heights are *estimated*, at a line count times a nominal line height, and
//! corrected the moment a chunk is actually measured. A scrollbar is precisely the UI that makes
//! that approximation visible — as you drag into a document nobody has scrolled to yet, chunks get
//! measured, the total corrects, and the thumb shifts or resizes slightly under the pointer.
//!
//! **That must not be "fixed".** The obvious fix is to re-anchor the view so the thumb stays put,
//! and it is the wrong trade: the viewport is already anchored so a height correction does not move
//! what is *on screen*, and content stability beats thumb stability every time. A user watching the
//! text jump because the scrollbar wanted to hold still would rightly call that the bug.
//!
//! The mapping is therefore recomputed from the current total every frame, and nothing about the
//! total is cached across frames. A cached total and a corrected one disagree, and the disagreement
//! shows up as the mapping tearing mid-drag.
//!
//! # Overlay, not a column
//!
//! The bar draws over the right-hand edge of the text rather than taking width from
//! [`crate::content::ContentRect`]. Two reasons: the document does not reflow when a document grows
//! past a screenful, and the bar can be absent entirely — with no width to give back — when
//! everything fits.
//!
//! The cost is that the bar covers a few pixels of text, so the click chain has to try the bar
//! *before* the document. See `Editor::click`.

/// Width of the bar.
pub const WIDTH: f32 = 12.0;

/// Shortest the thumb may be drawn, so a very long document still leaves something to grab.
///
/// Without it, a 10 MB document in a 600-pixel window gives a thumb under a pixel high.
pub const MIN_THUMB: f32 = 24.0;

/// Where the bar and its thumb are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// Left edge of the bar.
    pub x: f32,
    /// Top of the track.
    pub top: f32,
    /// Height of the track.
    pub height: f32,
    /// Top of the thumb, in window coordinates.
    pub thumb_top: f32,
    /// Height of the thumb.
    pub thumb_height: f32,
}

impl Bar {
    /// Bottom of the thumb.
    #[must_use]
    pub fn thumb_bottom(&self) -> f32 {
        self.thumb_top + self.thumb_height
    }

    /// Whether `x` falls on the bar.
    #[must_use]
    pub fn contains_x(&self, x: f32) -> bool {
        x >= self.x && x < self.x + WIDTH
    }

    /// How far the thumb may travel: the track less the thumb.
    #[must_use]
    pub fn travel(&self) -> f32 {
        (self.height - self.thumb_height).max(0.0)
    }
}

/// Works out the bar for a document of `content` height in a viewport `height` tall.
///
/// `top` is where the track starts and `width` is the window's, so the bar sits against the right
/// edge below whatever chrome is above it.
///
/// Returns `None` when the document fits, which is what makes the bar absent rather than full-height
/// — a scrollbar that is always there and always full is a decoration that costs a click target.
#[must_use]
pub fn layout(width: f32, top: f32, height: f32, content: f32, offset: f32) -> Option<Bar> {
    if content <= height || height <= 0.0 {
        return None;
    }

    // The visible fraction, which is what a thumb's height means — clamped so the minimum cannot
    // produce a thumb taller than its own track.
    let thumb_height = (height * (height / content)).clamp(MIN_THUMB.min(height), height);
    let travel = (height - thumb_height).max(0.0);

    // Against `content - height`, the furthest the view can go, so a thumb at the bottom of its
    // travel means the last line is on screen — not that the offset reached the content height,
    // which it never does.
    let max_offset = (content - height).max(1.0);
    let fraction = (offset / max_offset).clamp(0.0, 1.0);

    Some(Bar {
        x: width - WIDTH,
        top,
        height,
        thumb_top: fraction.mul_add(travel, top),
        thumb_height,
    })
}

/// What a press on the bar means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// On the thumb: start dragging it.
    Thumb,
    /// On the track above the thumb: a page back.
    PageUp,
    /// On the track below the thumb: a page on.
    PageDown,
}

/// Resolves a press at `(x, y)` against the bar.
#[must_use]
pub fn hit_test(bar: Option<Bar>, x: f32, y: f32) -> Option<Hit> {
    let bar = bar?;
    if !bar.contains_x(x) || y < bar.top || y >= bar.top + bar.height {
        return None;
    }
    if y < bar.thumb_top {
        Some(Hit::PageUp)
    } else if y >= bar.thumb_bottom() {
        Some(Hit::PageDown)
    } else {
        Some(Hit::Thumb)
    }
}

/// Where the view should go for a thumb whose top is dragged to `thumb_top`.
///
/// The inverse of what [`layout`] did, against the *current* total — which during a drag into
/// unmeasured territory is not the total the drag started with. See the module docs: the mapping
/// follows the correction rather than the drag holding a stale one.
#[must_use]
pub fn offset_for_thumb(bar: &Bar, content: f32, thumb_top: f32) -> f32 {
    let travel = bar.travel();
    if travel <= 0.0 {
        return 0.0;
    }
    let fraction = ((thumb_top - bar.top) / travel).clamp(0.0, 1.0);
    fraction * (content - bar.height).max(0.0)
}
