//! The tab strip: where each tab sits, and what a click on it means.
//!
//! Geometry only — no GPU types, no text engine, no window. That is deliberate: hit-testing is the
//! part that is easy to get subtly wrong and impossible to check by looking at a screenshot, so it
//! lives somewhere a test can reach it.
//!
//! Everything here is in **device-independent pixels**. The renderer multiplies by the scale factor
//! when it draws, exactly once, as [`mdedit_offsets::LayoutPos`] requires.

/// Height of the strip.
pub const STRIP_HEIGHT: f32 = 30.0;

/// Padding either side of a tab's label.
const LABEL_PADDING: f32 = 12.0;

/// Width of the close region at the right of each tab.
const CLOSE_WIDTH: f32 = 20.0;

/// Widest a single tab may be, so one long file name cannot fill the strip.
const MAX_TAB_WIDTH: f32 = 200.0;

/// Narrowest a tab may be, so a very short name still has something to click.
const MIN_TAB_WIDTH: f32 = 70.0;

/// Where one tab sits in the strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabRect {
    /// Which tab this is.
    pub index: usize,
    /// Left edge.
    pub x: f32,
    /// Total width, including the close region.
    pub width: f32,
}

impl TabRect {
    /// Right edge.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Where the label may be drawn, before the close region.
    #[must_use]
    pub fn label_x(&self) -> f32 {
        self.x + LABEL_PADDING
    }

    /// Width available to the label.
    #[must_use]
    pub fn label_width(&self) -> f32 {
        (self.width - LABEL_PADDING - CLOSE_WIDTH).max(0.0)
    }

    /// The close region, as `(x, width)`.
    #[must_use]
    pub fn close_region(&self) -> (f32, f32) {
        (self.right() - CLOSE_WIDTH, CLOSE_WIDTH)
    }

    /// Whether `x` falls inside this tab at all.
    #[must_use]
    pub fn contains(&self, x: f32) -> bool {
        x >= self.x && x < self.right()
    }

    /// Whether `x` falls on the close region rather than the body.
    #[must_use]
    pub fn is_close(&self, x: f32) -> bool {
        let (close_x, close_width) = self.close_region();
        x >= close_x && x < close_x + close_width
    }
}

/// Lays out as many tabs as fit across `available_width`.
///
/// Deliberately stops at the edge rather than laying out all of them and drawing a subset. With two
/// hundred tabs open, laying out every label every frame would cost two hundred text-shaping calls
/// to draw perhaps eight of them — the same mistake the renderer makes with document blocks, and
/// not one worth repeating here.
///
/// `label_widths` supplies the measured width of each tab's name, in the same order as the tabs.
#[must_use]
pub fn layout(label_widths: &[f32], available_width: f32) -> Vec<TabRect> {
    let mut rects = Vec::new();
    let mut x = 0.0_f32;

    for (index, label_width) in label_widths.iter().enumerate() {
        let width =
            (label_width + LABEL_PADDING * 2.0 + CLOSE_WIDTH).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH);
        if x >= available_width {
            break;
        }
        rects.push(TabRect { index, x, width });
        x += width;
    }

    rects
}

/// What a click at `x` in the strip means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Show this tab.
    Select(usize),
    /// Close this tab.
    Close(usize),
}

/// Resolves a click in the strip.
///
/// The close region is checked before the body, because it sits inside the tab and the body would
/// otherwise swallow it.
#[must_use]
pub fn hit_test(rects: &[TabRect], x: f32, y: f32) -> Option<Hit> {
    if !(0.0..STRIP_HEIGHT).contains(&y) {
        return None;
    }
    let rect = rects.iter().find(|rect| rect.contains(x))?;
    if rect.is_close(x) {
        Some(Hit::Close(rect.index))
    } else {
        Some(Hit::Select(rect.index))
    }
}
