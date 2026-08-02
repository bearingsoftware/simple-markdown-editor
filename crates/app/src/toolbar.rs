//! The formatting bar: which buttons a language gets, where they sit, and what a click means.
//!
//! Geometry and vocabulary only — no GPU types, no text engine, no window — as `tabs.rs` and
//! `pane.rs` are, and for the reason they give: hit-testing is easy to get subtly wrong and
//! impossible to check by looking at a screenshot.
//!
//! # A second door, never a second implementation
//!
//! Every button names an [`Action`] that is already an editor method with a chord. The bar performs
//! nothing itself; it is the discoverability surface for people arriving from Word, and trivially
//! hidden for everyone who would rather have the chords.
//!
//! # The button glyphs are text
//!
//! A bold `B`, an italic `I`, `H1`, `</>`. Drawn by the text engine the rest of the editor uses, so
//! there is no icon pipeline, no asset format and nothing to rasterise at two DPIs. It costs some
//! elegance and buys the whole feature at the price of a label cache.
//!
//! **Laid out once and cached.** The per-frame-layout trap has been paid for three times already —
//! the gutter, the status bar and the file pane each hit it — and a fourth would be indefensible.
//!
//! # What it does not have
//!
//! **No tooltips**, because there is no hover affordance in this editor to hang one on. **No
//! accessible names** either; those wait for the accesskit pass. Both are recorded rather than
//! quietly skipped, because a toolbar is exactly the chrome a screen-reader user would expect to be
//! able to reach.

use std::fmt::Write as _;

/// Height of the bar when it is showing.
pub const HEIGHT: f32 = 30.0;

/// Space either side of a button's glyph.
const PADDING: f32 = 5.0;

/// Narrowest a button may be, so a one-character glyph still has something to click.
const MIN_WIDTH: f32 = 24.0;

/// Space between groups of related buttons.
const GROUP_GAP: f32 = 10.0;

/// Something a button does, which is always something a chord already does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `**bold**`.
    Bold,
    /// `*italic*`.
    Italic,
    /// `~~struck through~~`.
    Strikethrough,
    /// `` `code` ``.
    Code,
    /// A heading of this level.
    Heading(u8),
    /// `- ` list.
    Bullet,
    /// `1. ` list.
    Ordered,
    /// `- [ ] ` list.
    Task,
    /// `> ` quote.
    Quote,
    /// A fenced code block.
    Fence,
    /// A thematic break.
    Rule,
    /// A link, using a URL from the clipboard if there is one.
    Link,
}

impl Action {
    /// The text drawn on the button.
    ///
    /// Short enough to read at 30 pixels and conventional enough not to need a legend: `B` and `I`
    /// are what every word processor has used for forty years.
    #[must_use]
    pub fn glyph(self) -> String {
        match self {
            Self::Bold => "B".to_owned(),
            Self::Italic => "I".to_owned(),
            Self::Strikethrough => "S".to_owned(),
            Self::Code => "<>".to_owned(),
            Self::Heading(level) => {
                let mut label = String::from("H");
                let _ = write!(label, "{level}");
                label
            }
            Self::Bullet => "•".to_owned(),
            Self::Ordered => "1.".to_owned(),
            // `[x]` and `[]()` rather than ☑ and 🔗: both of those resolve to the emoji font and
            // render as colour glyphs, which look like lit buttons sitting in a monochrome row.
            // These are also the Markdown the buttons write, which is its own kind of legend.
            Self::Task => "[x]".to_owned(),
            Self::Quote => "❝".to_owned(),
            Self::Fence => "```".to_owned(),
            Self::Rule => "—".to_owned(),
            Self::Link => "[]()".to_owned(),
        }
    }

    /// Whether a gap goes *before* this button, separating it from the group before.
    ///
    /// Grouping is the only thing standing in for labels: inline styling, then headings, then lists,
    /// then blocks. Without it the bar reads as an undifferentiated row of symbols.
    #[must_use]
    fn starts_a_group(self) -> bool {
        matches!(self, Self::Heading(1) | Self::Bullet | Self::Quote)
    }
}

/// The buttons a Markdown document gets.
///
/// Ordered as the plan asked: inline styling, headings, lists, blocks. Three heading levels rather
/// than six — H4 and below are rare enough that the chords carry them, and a bar of sixteen buttons
/// stops being scannable.
pub const MARKDOWN: &[Action] = &[
    Action::Bold,
    Action::Italic,
    Action::Strikethrough,
    Action::Code,
    Action::Heading(1),
    Action::Heading(2),
    Action::Heading(3),
    Action::Bullet,
    Action::Ordered,
    Action::Task,
    Action::Quote,
    Action::Fence,
    Action::Rule,
    Action::Link,
];

/// The buttons the bar that follows a selection gets.
///
/// The inline set plus three heading levels — what somebody who has just selected a run of text
/// plausibly wants done *to that text*. The list and block actions are on the fixed bar: they act on
/// whole lines rather than on a selection, so offering them here would be a different operation
/// wearing the same clothes. A selection bar full of document-level buttons is a category error, and
/// the whole reason the two sets are separate constants rather than one.
pub const SELECTION: &[Action] = &[
    Action::Bold,
    Action::Italic,
    Action::Strikethrough,
    Action::Code,
    Action::Link,
    Action::Heading(1),
    Action::Heading(2),
    Action::Heading(3),
];

/// Gap between the selection and the bar that floats over it.
pub const FLOAT_GAP: f32 = 6.0;

/// Where one button sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonRect {
    /// What it does.
    pub action: Action,
    /// Left edge, in device-independent pixels from the window's left.
    pub x: f32,
    /// Width.
    pub width: f32,
}

impl ButtonRect {
    /// Right edge.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Whether `x` falls on this button.
    #[must_use]
    pub fn contains(&self, x: f32) -> bool {
        x >= self.x && x < self.right()
    }
}

/// Lays the buttons out across `available_width`.
///
/// `glyph_widths` supplies the measured width of each button's glyph, in the same order as
/// `actions`. Stops at the edge rather than laying out buttons nobody can see — the same rule the
/// tab strip follows, for the same reason.
#[must_use]
pub fn layout(actions: &[Action], glyph_widths: &[f32], available_width: f32) -> Vec<ButtonRect> {
    let mut rects = Vec::new();
    let mut x = PADDING;

    for (action, glyph_width) in actions.iter().zip(glyph_widths) {
        if action.starts_a_group() && !rects.is_empty() {
            x += GROUP_GAP;
        }
        let width = (glyph_width + PADDING * 2.0).max(MIN_WIDTH);
        if x + width > available_width {
            break;
        }
        rects.push(ButtonRect {
            action: *action,
            x,
            width,
        });
        x += width;
    }

    rects
}

/// How wide a bar holding these buttons is, including the padding at its right end.
#[must_use]
pub fn width_of(buttons: &[ButtonRect]) -> f32 {
    buttons
        .last()
        .map_or(0.0, |button| button.right() + PADDING)
}

/// The same buttons, moved right by `dx`.
///
/// [`layout`] measures from the bar's own left edge, which for the fixed bar is the window's. The
/// floating one sits wherever the selection is, and shifting the rects once here is what lets
/// [`hit_test`] stay a plain comparison against window coordinates rather than learn about origins.
#[must_use]
pub fn shifted(buttons: &[ButtonRect], dx: f32) -> Vec<ButtonRect> {
    buttons
        .iter()
        .map(|button| ButtonRect {
            x: button.x + dx,
            ..*button
        })
        .collect()
}

/// The first row of a selection, in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// Left edge of the selected run on that row.
    pub x: f32,
    /// Top of the row.
    pub y: f32,
    /// Width of the selected run on that row.
    pub width: f32,
    /// The row's height.
    pub height: f32,
}

/// Where a bar `bar_width` wide goes, given the selection it belongs to and the room available.
///
/// Above the selection's **first row**, centred on it. Not on the bounding box of the whole
/// selection: the centre of a tall selection is over the middle of the document and the bar would
/// appear nowhere near what the user just dragged over.
///
/// Flipped below when there is no room above, so it never leaves the content rect at the top; and
/// clamped horizontally, so it never leaves it at either side. It must never cover the selection it
/// acts on, which is why the flip is below the row rather than over it.
#[must_use]
pub fn place(bar_width: f32, anchor: Anchor, left: f32, right: f32, top: f32) -> (f32, f32) {
    let centred = anchor.x + anchor.width / 2.0 - bar_width / 2.0;
    // `max` last: in a window too narrow for the bar the left edge wins, because a bar clipped at
    // the right is readable from its start and one clipped at the left is not.
    let x = centred.min(right - bar_width).max(left);

    let above = anchor.y - HEIGHT - FLOAT_GAP;
    let y = if above < top {
        anchor.y + anchor.height + FLOAT_GAP
    } else {
        above
    };
    (x, y)
}

/// Which button a click at `(x, y)` landed on, if any.
///
/// `y` is measured from the top of the bar. A click anywhere in the bar's height counts, including
/// the gaps between buttons — which return `None` and are still swallowed by the caller, because a
/// click on chrome must never reach the document.
#[must_use]
pub fn hit_test(buttons: &[ButtonRect], x: f32, y: f32) -> Option<Action> {
    if !(0.0..HEIGHT).contains(&y) {
        return None;
    }
    buttons
        .iter()
        .find(|button| button.contains(x))
        .map(|button| button.action)
}
