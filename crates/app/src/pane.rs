//! The file pane: where each open file's row sits, and what a click on one means.
//!
//! Geometry only — no GPU types, no text engine, no window, exactly as [`crate::tabs`] is. Everything
//! here is in **device-independent pixels**; the renderer multiplies by the scale factor once.
//!
//! # The first chrome with internal state
//!
//! The tab strip needed no scrolling and no hit-testing beyond "which x", because it is one row that
//! always fits — a tab that does not fit is simply not laid out. A pane cannot do that: a workspace
//! with forty files open has to reach the fortieth, so the pane scrolls, and scrolling means a click
//! at a given y refers to a different row depending on state the pane owns.
//!
//! That is the whole of the difficulty in this feature, and it is why the offset is a parameter to
//! both [`layout`] and [`hit_test`] rather than something either of them remembers. The two cannot
//! disagree about where a row is if neither of them knows.

/// Width of the pane.
///
/// Fixed rather than proportional. A pane that grew with the window would make the wrapping width
/// depend on it, and every cached layout is keyed by that width — so resizing the window would
/// invalidate the whole layout cache twice over instead of once.
pub const PANE_WIDTH: f32 = 200.0;

/// Height of one row.
pub const ROW_HEIGHT: f32 = 24.0;

/// Padding at the left of a row's label.
const LABEL_PADDING: f32 = 10.0;

/// Width of the close region at the right of each row.
const CLOSE_WIDTH: f32 = 24.0;

/// Where one row sits in the pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowRect {
    /// Which tab this is.
    pub index: usize,
    /// Top edge, relative to the top of the pane and already scrolled.
    pub y: f32,
}

impl RowRect {
    /// Bottom edge.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + ROW_HEIGHT
    }

    /// Where the label may be drawn.
    #[must_use]
    pub fn label_x(&self) -> f32 {
        LABEL_PADDING
    }

    /// Width available to the label, before the close region.
    #[must_use]
    pub fn label_width(&self) -> f32 {
        (PANE_WIDTH - LABEL_PADDING - CLOSE_WIDTH).max(0.0)
    }

    /// The close region, as `(x, width)`.
    #[must_use]
    pub fn close_region(&self) -> (f32, f32) {
        (PANE_WIDTH - CLOSE_WIDTH, CLOSE_WIDTH)
    }

    /// Whether `x` falls on the close region rather than the body.
    #[must_use]
    pub fn is_close(&self, x: f32) -> bool {
        let (close_x, close_width) = self.close_region();
        x >= close_x && x < close_x + close_width
    }

    /// Whether `y` falls inside this row.
    #[must_use]
    pub fn contains(&self, y: f32) -> bool {
        y >= self.y && y < self.bottom()
    }
}

/// How tall the whole list is, scrolled or not.
#[must_use]
pub fn content_height(count: usize) -> f32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a workspace with more tabs than an f32 can count is not reachable"
    )]
    let count = count as f32;
    count * ROW_HEIGHT
}

/// The furthest the pane may be scrolled, which is nothing when everything fits.
#[must_use]
pub fn max_scroll(count: usize, visible_height: f32) -> f32 {
    (content_height(count) - visible_height).max(0.0)
}

/// Lays out the rows that are visible at `scroll`.
///
/// Only the visible ones, for the reason the strip stops at the edge: laying out forty labels to
/// draw eight is the mistake the renderer already avoids with document blocks.
///
/// `scroll` is how far down the list has been moved, and is clamped by the caller — a stale offset
/// produces an empty list rather than a panic.
#[must_use]
pub fn layout(count: usize, visible_height: f32, scroll: f32) -> Vec<RowRect> {
    let mut rects = Vec::new();
    if visible_height <= 0.0 {
        return rects;
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a scroll offset is clamped to a non-negative height that fits an f32"
    )]
    let first = (scroll / ROW_HEIGHT).floor().max(0.0) as usize;

    for index in first..count {
        #[allow(
            clippy::cast_precision_loss,
            reason = "a workspace with more tabs than an f32 can count is not reachable"
        )]
        let top = index as f32 * ROW_HEIGHT - scroll;
        if top >= visible_height {
            break;
        }
        rects.push(RowRect { index, y: top });
    }

    rects
}

/// What a click in the pane means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Show this file.
    Select(usize),
    /// Close this file.
    Close(usize),
}

/// Resolves a click in the pane.
///
/// `x` and `y` are relative to the pane's top-left. The close region is checked before the body,
/// because it sits inside the row and the body would otherwise swallow it — as in the strip.
#[must_use]
pub fn hit_test(rects: &[RowRect], x: f32, y: f32) -> Option<Hit> {
    if !(0.0..PANE_WIDTH).contains(&x) {
        return None;
    }
    let rect = rects.iter().find(|rect| rect.contains(y))?;
    if rect.is_close(x) {
        Some(Hit::Close(rect.index))
    } else {
        Some(Hit::Select(rect.index))
    }
}
