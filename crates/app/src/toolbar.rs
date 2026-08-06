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
//! # Two sets, and the first is why the bar never disappears
//!
//! [`STANDARD`] is what every document gets; a language appends its own after a gap. Before
//! MDEDIT-84 the bar was Markdown's alone and took no height on JSON or YAML, which was fine while
//! it sat *below* the tab strip and untenable once it sits above one — a bar that comes and goes
//! would move the strip on every tab switch.
//!
//! Whether a button can act right now is the editor's question, not this module's: see
//! `Editor::is_enabled`. Geometry does not change with state, which is deliberate — a dimmed button
//! keeps its rect, so the row does not reflow as you select and deselect.
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
//! **No accessible names**; those wait for the accesskit pass. Recorded rather than quietly skipped,
//! because a toolbar is exactly the chrome a screen-reader user would expect to be able to reach —
//! and MDEDIT-85's tooltips are **not** that. A tooltip is mouse-only and a screen reader will not
//! read it.
//!
//! There were no tooltips either until MDEDIT-85, "because there is no hover affordance in this
//! editor to hang one on". That sentence was the ticket rather than an excuse; see
//! [`crate::tooltip`] for what building the affordance actually cost.

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
    /// A new, empty document.
    New,
    /// The file picker.
    Open,
    /// Write the buffer back where it came from.
    Save,
    /// Reverse the last group of edits.
    Undo,
    /// Put back what [`Undo`](Self::Undo) took away.
    Redo,
    /// Copy the selection and delete it.
    Cut,
    /// Copy the selection.
    Copy,
    /// Insert whatever is on the clipboard.
    Paste,
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
    ///
    /// The standard set is **words**, where the Markdown set is symbols. That is not an
    /// inconsistency to tidy up: `B` and `I` are a forty-year convention and there is no equivalent
    /// glyph for Open that anybody would read without a legend. MDEDIT-92 replaces both sets with
    /// icons, and MDEDIT-85's tooltips come first because a row of unlabelled glyphs is unusable
    /// without them.
    #[must_use]
    pub fn glyph(self) -> String {
        match self {
            Self::New => "New".to_owned(),
            Self::Open => "Open".to_owned(),
            Self::Save => "Save".to_owned(),
            Self::Undo => "Undo".to_owned(),
            Self::Redo => "Redo".to_owned(),
            Self::Cut => "Cut".to_owned(),
            Self::Copy => "Copy".to_owned(),
            Self::Paste => "Paste".to_owned(),
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

    /// What the button does, in the words a tooltip shows.
    ///
    /// MDEDIT-85's plan asked for these to be looked up out of [`crate::shortcuts::ALL`], so the
    /// phrase lives in one place. **They are here instead, and the drift is caught by a test rather
    /// than by a lookup** — because the list is a *reference document* and reads like one where a
    /// tooltip has to read like a label. Its entry for a heading is one row covering all six levels
    /// ("Heading level — press the same level again to clear it"), which is right on a help page and
    /// wrong on a button that means H2 specifically; its entry for a link carries a clause about the
    /// clipboard.
    ///
    /// The drift that actually matters is **advertising a chord that does not exist**, which is
    /// MDEDIT-65 exactly, and `tests/tooltip.rs` asserts every chord named by [`chord`](Self::chord)
    /// appears in the shortcut list. A wording that reads differently in two places is not a defect;
    /// a button promising Ctrl+B when nothing answers to it is.
    #[must_use]
    pub fn describes(self) -> String {
        match self {
            Self::New => "New file".to_owned(),
            Self::Open => "Open a file".to_owned(),
            Self::Save => "Save".to_owned(),
            Self::Undo => "Undo".to_owned(),
            Self::Redo => "Redo".to_owned(),
            Self::Cut => "Cut".to_owned(),
            Self::Copy => "Copy".to_owned(),
            Self::Paste => "Paste".to_owned(),
            Self::Bold => "Bold".to_owned(),
            Self::Italic => "Italic".to_owned(),
            Self::Strikethrough => "Strikethrough".to_owned(),
            Self::Code => "Inline code".to_owned(),
            Self::Heading(level) => {
                let mut label = String::from("Heading ");
                let _ = write!(label, "{level}");
                label
            }
            Self::Bullet => "Bullet list".to_owned(),
            Self::Ordered => "Numbered list".to_owned(),
            Self::Task => "Task list".to_owned(),
            Self::Quote => "Block quote".to_owned(),
            Self::Fence => "Code block".to_owned(),
            Self::Rule => "Horizontal rule".to_owned(),
            Self::Link => "Link".to_owned(),
        }
    }

    /// The chord that does the same thing, if there is one.
    ///
    /// `None` for the block actions, which have never had one — the bar is the only way to reach a
    /// bullet list or a fence, which is a good part of why the bar exists. Spelled the way
    /// [`crate::shortcuts::ALL`] spells a chord, because a test compares the two.
    #[must_use]
    pub fn chord(self) -> Option<String> {
        let chord = match self {
            Self::New => "Ctrl+N",
            Self::Open => "Ctrl+O",
            Self::Save => "Ctrl+S",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Y",
            Self::Cut => "Ctrl+X",
            Self::Copy => "Ctrl+C",
            Self::Paste => "Ctrl+V",
            Self::Bold => "Ctrl+B",
            Self::Italic => "Ctrl+I",
            Self::Strikethrough => "Ctrl+Shift+X",
            Self::Code => "Ctrl+E",
            Self::Link => "Ctrl+K",
            Self::Heading(level) => {
                let mut chord = String::from("Ctrl+");
                let _ = write!(chord, "{level}");
                return Some(chord);
            }
            Self::Bullet | Self::Ordered | Self::Task | Self::Quote | Self::Fence | Self::Rule => {
                return None;
            }
        };
        Some(chord.to_owned())
    }

    /// The whole line a tooltip shows: what it does, and the chord that does the same.
    #[must_use]
    pub fn tooltip(self) -> String {
        let mut line = self.describes();
        if let Some(chord) = self.chord() {
            let _ = write!(line, " — {chord}");
        }
        line
    }

    /// Whether a gap goes *before* this button, separating it from the group before.
    ///
    /// Grouping is the only thing standing in for labels: file, history, clipboard, then the
    /// language's own — inline styling, headings, lists, blocks. Without it the bar reads as an
    /// undifferentiated row of symbols.
    ///
    /// [`Bold`](Self::Bold) is in the list because it is the first of the language's set, and the
    /// gap before it is the one that says where the standard set ends. [`layout`] suppresses a gap
    /// before the *first* button, so the floating bar — which starts at Bold — is unaffected.
    #[must_use]
    fn starts_a_group(self) -> bool {
        matches!(
            self,
            Self::Undo | Self::Cut | Self::Bold | Self::Heading(1) | Self::Bullet | Self::Quote
        )
    }
}

/// The buttons **every** document gets, whatever its language.
///
/// MDEDIT-84. Before this the bar was Markdown's alone, so a JSON tab had no bar at all — and once
/// the bar sits above the tab strip, a bar that comes and goes moves the *strip* every time the user
/// switches tab. A set every document has is what makes the bar's row a fixed thing, which is why
/// this constant exists before the swap rather than after it.
///
/// Leftmost, deliberately. [`layout`] stops at the window's edge, so a narrow window loses the
/// language's buttons before it loses Save — and the standard actions are the ones somebody who does
/// not know the chords is looking for, while every formatting button has both a chord and a menu
/// item.
///
/// **Close is not here.** A one-click Close beside Save is a misclick that shuts a document, and the
/// bar is meant to be the safe surface. It is on the File menu, on Ctrl+W, and on each tab's `×`.
pub const STANDARD: &[Action] = &[
    Action::New,
    Action::Open,
    Action::Save,
    Action::Undo,
    Action::Redo,
    Action::Cut,
    Action::Copy,
    Action::Paste,
];

/// The buttons a Markdown document gets, *after* [`STANDARD`].
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
///
/// [`STANDARD`] is not here either, and for a stronger version of the same argument: New and Save
/// have no business floating over a paragraph the user has just highlighted.
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
