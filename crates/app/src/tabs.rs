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

/// The column a tab's type icon sits in, its gap included (MDEDIT-178).
///
/// **Added to the tab, not taken from its name**, which is where the strip parts company with the
/// pane (MDEDIT-174). The pane is a fixed 200 px, so its icon had to come off the name; a tab is
/// sized from its name, so the icon can go on top and every name keeps the room it had. The cost
/// moves to the strip instead: eighteen pixels a tab, so fewer tabs fit before [`layout`] stops.
pub const ICON_WIDTH: f32 = 18.0;

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

    /// Where the type icon is drawn (MDEDIT-178).
    #[must_use]
    pub fn icon_x(&self) -> f32 {
        self.x + LABEL_PADDING
    }

    /// Where the label may be drawn: past the icon, before the close region.
    #[must_use]
    pub fn label_x(&self) -> f32 {
        self.x + LABEL_PADDING + ICON_WIDTH
    }

    /// Width available to the label.
    #[must_use]
    pub fn label_width(&self) -> f32 {
        (self.width - LABEL_PADDING - ICON_WIDTH - CLOSE_WIDTH).max(0.0)
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
        let width = tab_width(*label_width);
        if x >= available_width {
            break;
        }
        rects.push(TabRect { index, x, width });
        x += width;
    }

    rects
}

/// How wide a tab is whose name measures `label_width`.
fn tab_width(label_width: f32) -> f32 {
    // The icon goes on after the clamp, so the bounds still describe the name's room and a name
    // clamped before MDEDIT-178 is clamped at exactly the same width now.
    (label_width + LABEL_PADDING * 2.0 + CLOSE_WIDTH).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH)
        + ICON_WIDTH
}

/// Width of the overflow button at the end of an overflowing strip (MDEDIT-179).
///
/// The formatting bar's `More` is the model, and so is its reserve: a constant rather than a
/// measurement, because the room has to be known before the tabs are laid out.
pub const MORE_WIDTH: f32 = 32.0;

/// The strip as drawn: the tabs that fit, and the ones behind the overflow button (MDEDIT-179).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Strip {
    /// What is drawn, left to right.
    pub tabs: Vec<TabRect>,
    /// What the overflow button lists, in tab order. Empty when everything fitted.
    pub hidden: Vec<usize>,
    /// Where the overflow button starts. `Some` exactly when `hidden` is not empty.
    pub more: Option<f32>,
}

impl Strip {
    /// What a click at `(x, y)`, measured from the strip's top-left, means.
    ///
    /// The overflow button is asked first, though nothing overlaps it: it sits after the last tab.
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Hit> {
        if !(0.0..STRIP_HEIGHT).contains(&y) {
            return None;
        }
        if self
            .more
            .is_some_and(|more| (more..more + MORE_WIDTH).contains(&x))
        {
            return Some(Hit::More);
        }
        hit_test(&self.tabs, x, y)
    }
}

/// Lays out the strip so every tab can be reached: the ones that fit, and an overflow button
/// listing the rest (MDEDIT-179).
///
/// [`layout`] stops at the edge and says nothing about what it left off, which made a tab past the
/// edge unreachable from the strip — and, because it always started at tab zero, could leave the
/// **active** tab off it too, so switching to a file lit nothing. This is the formatting bar's
/// answer (`toolbar::overflowing`, MDEDIT-116), with one addition the bar does not need: the window
/// of tabs shown is moved so it always includes `active`.
///
/// Only whole tabs are drawn once the strip overflows — a tab cut off at the edge is a tab that is
/// both on the strip and in the list. The single exception is a strip too narrow for even one tab,
/// which still draws one, because an empty strip looks broken.
///
/// **`label_width` is asked only for tabs this might draw**, which is how the rule [`layout`]
/// documents survives: with two hundred open, it is called for about as many as fit, twice over at
/// worst, rather than for two hundred.
#[must_use]
pub fn overflowing(
    count: usize,
    active: Option<usize>,
    available_width: f32,
    mut label_width: impl FnMut(usize) -> f32,
) -> Strip {
    let mut widths: Vec<Option<f32>> = vec![None; count];
    let mut width_of =
        |index: usize| *widths[index].get_or_insert_with(|| tab_width(label_width(index)));

    // Everything fits: no button, and nothing to reserve room for.
    let mut x = 0.0_f32;
    let mut fitted = 0;
    while fitted < count {
        let width = width_of(fitted);
        if x + width > available_width {
            break;
        }
        x += width;
        fitted += 1;
    }
    if fitted == count {
        return Strip {
            tabs: place(0, count, &mut width_of),
            hidden: Vec::new(),
            more: None,
        };
    }

    let budget = (available_width - MORE_WIDTH).max(0.0);

    // Walk back from the active tab for as long as the tabs before it still fit alongside it. That
    // puts the active tab at the right-hand end only when it has to be; if it fits counting from
    // zero, the start stays at zero and the strip does not move.
    let mut start = 0;
    if let Some(active) = active.filter(|&active| active < count) {
        let mut used = width_of(active);
        start = active;
        while start > 0 && used + width_of(start - 1) <= budget {
            start -= 1;
            used += width_of(start);
        }
    }

    // Then forward from there for as far as the budget goes, always keeping at least one tab.
    let mut end = start;
    let mut used = 0.0_f32;
    while end < count {
        let width = width_of(end);
        if end > start && used + width > budget {
            break;
        }
        used += width;
        end += 1;
    }

    let tabs = place(start, end, &mut width_of);
    // One tab wider than the whole strip lands here with nothing hidden. A button over an empty
    // list would be a control that does nothing, so it is only drawn when there is something in it.
    let hidden: Vec<usize> = (0..start).chain(end..count).collect();
    let more = (!hidden.is_empty()).then(|| tabs.last().map_or(0.0, TabRect::right));
    Strip { tabs, hidden, more }
}

/// Tabs `start..end`, abutting from the strip's left edge.
fn place(start: usize, end: usize, width_of: &mut impl FnMut(usize) -> f32) -> Vec<TabRect> {
    let mut x = 0.0_f32;
    (start..end)
        .map(|index| {
            let width = width_of(index);
            let rect = TabRect { index, x, width };
            x += width;
            rect
        })
        .collect()
}

/// What a click at `x` in the strip means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Show this tab.
    Select(usize),
    /// Close this tab.
    Close(usize),
    /// Open the list of tabs that did not fit (MDEDIT-179).
    More,
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
