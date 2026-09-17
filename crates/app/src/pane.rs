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

/// The column a row's type icon sits in, its gap included (MDEDIT-174).
///
/// It comes off the name's width, which is the whole of its cost: the pane is a fixed 200 px, so an
/// icon is eighteen pixels a name no longer has. That is what makes MDEDIT-121's truncation the thing
/// to keep honest — the name must be measured against what is left, not against what used to be there.
pub const ICON_WIDTH: f32 = 18.0;

/// What a row's icon says its file is (MDEDIT-174).
///
/// **Three, not seven.** The icon font has no JSON glyph and no YAML one; its usable silhouettes are
/// a page, a page with lines, code brackets and three rules, and its `Document`, `Page` and
/// `MusicInfo` code points rasterise to one identical glyph. So the languages are grouped by what
/// they are *for* rather than given a shape apiece they cannot have. JSON and YAML look alike here,
/// and their names are what tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Prose to read: Markdown.
    Note,
    /// Structure to configure: JSON, YAML, XML, TOML, INI.
    Structured,
    /// Plain text, and anything with no language of its own.
    Text,
}

impl Icon {
    /// Which icon a language gets.
    ///
    /// **Keyed off the language rather than the extension**, so a new one gets an icon by being added
    /// to [`crate::detect`] and nowhere else — `.yml` and `.yaml` cannot drift apart, because they are
    /// one language by the time this is asked.
    #[must_use]
    pub fn of(language: &dyn mdedit_language::Language) -> Self {
        match language.name() {
            "Markdown" => Self::Note,
            "JSON" | "YAML" | "XML" | "TOML" | "INI" => Self::Structured,
            _ => Self::Text,
        }
    }

    /// The private-use code point this draws, from the icon font MDEDIT-92 already uses.
    ///
    /// **Rasterised and looked at, not taken from a reference page** — that is how the three
    /// identical "document" code points were found.
    #[must_use]
    pub fn glyph(self) -> char {
        match self {
            Self::Note => '\u{E9F9}',
            Self::Structured => '\u{E943}',
            Self::Text => '\u{E14C}',
        }
    }

    /// What to draw instead on a machine whose fonts have no such glyph.
    ///
    /// Short enough for the icon's column, because that column is all the room there is: a fallback
    /// that overran it would push into the name it is supposed to leave alone.
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Note => "MD",
            Self::Structured => "{}",
            Self::Text => "TXT",
        }
    }
}

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

    /// Where the type icon is drawn (MDEDIT-174).
    #[must_use]
    pub fn icon_x(&self) -> f32 {
        LABEL_PADDING
    }

    /// Where the label may be drawn: past the icon's column.
    #[must_use]
    pub fn label_x(&self) -> f32 {
        LABEL_PADDING + ICON_WIDTH
    }

    /// Width available to the label, before the close region.
    #[must_use]
    pub fn label_width(&self) -> f32 {
        (PANE_WIDTH - LABEL_PADDING - ICON_WIDTH - CLOSE_WIDTH).max(0.0)
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
