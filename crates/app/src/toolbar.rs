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
//! # The button glyphs come from a font, which is how they became icons
//!
//! They were text until MDEDIT-92 — a bold `B`, an italic `I`, `H1` — on the reasoning that there is
//! no icon pipeline, no asset format and nothing to rasterise at two DPIs. **That reasoning was not
//! overturned; it was satisfied.** Windows ships two monochrome icon fonts, so an icon costs a code
//! point rather than a decoder and an atlas format, and the text engine draws it exactly as it drew
//! the letter. The label cache, the measurement, the hit-testing and `set_zoom`'s clearing all kept
//! working because to the renderer nothing changed.
//!
//! See [`Action::icon`] for the table, [`Action::label`] for what is drawn without it, and
//! [`mdedit_text::FontFamily::Icon`] for the resolution.
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
///
/// **Thirty until MDEDIT-92, and this is where that ticket also answers MDEDIT-86.** A 22-pixel icon
/// does not fit a 30-pixel row, so making the bar iconic and making it bigger are one change rather
/// than two — the build order says as much. The numbers are the mock-up's: a 48-pixel bar, 22-pixel
/// glyphs, and buttons that come out roughly square.
pub const HEIGHT: f32 = 48.0;

/// Space either side of a button's glyph.
///
/// **Six rather than the mock-up's eight, and the difference is the whole bar fitting.** Twenty-two
/// buttons at 22-pixel icons come to 930 logical pixels with eight, against a default window of 900
/// — so a fresh install lost Fence, Rule and Link to the overflow rule without ever saying so. The
/// icons are untouched; only the air around them is smaller, which is the cheapest of the three
/// levers and the one nobody looking at the bar can see.
const PADDING: f32 = 6.0;

/// Narrowest a button may be, so a short label is still a comfortable target.
///
/// Binds on the three lettered heading buttons and nothing else — an icon is wider than this on its
/// own. It is what stops `H1` being a sliver between two squares.
const MIN_WIDTH: f32 = 34.0;

/// Space between groups of related buttons.
const GROUP_GAP: f32 = 14.0;

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
    ///
    /// On the *bar* this is [`Headings`](Self::Headings) — one button opening a list. This variant
    /// is what that list's rows are, and what the floating selection bar and the chords still use
    /// directly.
    Heading(u8),
    /// Opens the list of heading levels. **Not an edit.**
    ///
    /// MDEDIT-116. Three buttons became one, which is what made room for MDEDIT-87's, and the list
    /// carries all six levels rather than the three the bar had space for — so H4 to H6 stop being
    /// chord-only.
    Headings,
    /// Opens the list of buttons that did not fit. **Not an edit.**
    ///
    /// Only ever on the bar when something was displaced, and it is why nothing is silently lost off
    /// the right-hand end any more.
    More,
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
    /// The icon this button draws, if the icon font has one for it.
    ///
    /// MDEDIT-92. Private-use code points from Segoe Fluent Icons, with Segoe MDL2 Assets behind it
    /// — see [`mdedit_text::FontFamily::Icon`]. **A code point here is a promise about a font, not a
    /// standard**, which is why [`label`](Self::label) still exists and why the renderer checks the
    /// laid-out glyph for `.notdef` rather than trusting this table.
    ///
    /// Every one of these was confirmed against the installed font rather than taken from a
    /// reference page, and two of them were *chosen* by rasterising the candidates and looking:
    /// `Fence` is `U+F0E3`, a bordered box with lines in it, and `Rule` is `U+E738`, which is
    /// literally a horizontal line. Both were on the plan as "weak — check".
    ///
    /// **The three headings have none, and that is the ticket's one accepted exception.** Neither
    /// font has a heading glyph because the convention does not exist — Word labels its heading
    /// styles with letterforms too, and so does every editor that has them. The alternatives were
    /// drawing three glyphs by hand as quads, which reintroduces the shape pipeline the font choice
    /// exists to avoid, and collapsing six levels into one stepping button, which needs a label to
    /// say which level it is on and so arrives back here. `H1` it is.
    #[must_use]
    pub fn icon(self) -> Option<char> {
        let code = match self {
            Self::New => '\u{E7C3}',
            Self::Open => '\u{E8E5}',
            Self::Save => '\u{E74E}',
            Self::Undo => '\u{E7A7}',
            Self::Redo => '\u{E7A6}',
            Self::Cut => '\u{E8C6}',
            Self::Copy => '\u{E8C8}',
            Self::Paste => '\u{E77F}',
            Self::Bold => '\u{E8DD}',
            Self::Italic => '\u{E8DB}',
            Self::Strikethrough => '\u{EDE0}',
            Self::Code => '\u{E943}',
            Self::Bullet => '\u{E8FD}',
            Self::Ordered => '\u{E8FE}',
            Self::Task => '\u{E9D5}',
            Self::Quote => '\u{E9D2}',
            Self::Fence => '\u{F0E3}',
            Self::Rule => '\u{E738}',
            Self::Link => '\u{E71B}',
            // Fluent's "More" — a horizontal ellipsis, which is the convention for an overflow.
            Self::More => '\u{E712}',
            // The documented exception. See above.
            //
            // `Headings` letters for a second reason on top of that one: it has to say *which* level
            // the caret is in, and a picture cannot.
            Self::Heading(_) | Self::Headings => return None,
        };
        Some(code)
    }

    /// The text drawn on the button when there is no icon for it.
    ///
    /// Two jobs, and they used to be the only one. It is what the three heading buttons draw
    /// **always** — `H1` is what a heading looks like everywhere — and it is what *every* button
    /// draws on a machine whose icon font is missing, which is the whole of MDEDIT-92's fallback:
    /// the old bar rather than a row of empty boxes.
    ///
    /// Short enough to read on a button and conventional enough not to need a legend: `B` and `I`
    /// are what every word processor has used for forty years. The standard set is words, because
    /// there is no letterform for Open that anybody would read without one.
    #[must_use]
    pub fn label(self) -> String {
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
            // What the heading button reads when the caret is not in one. **`H1`, not a bare `H`**:
            // a lone letter looks like a placeholder, and H1 is both the commonest level and what
            // the button did before it opened a list. When the caret *is* in a heading it reads that
            // level instead — see `Editor::face_key`, which is also the cache key.
            Self::Headings => "H1".to_owned(),
            // Only ever drawn if the icon font is missing, in which case an ellipsis is still the
            // right word for it.
            Self::More => "…".to_owned(),
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
            Self::Headings => "Heading".to_owned(),
            Self::More => "More".to_owned(),
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
            // The block actions have never had one. Neither menu-opener has one either, and could
            // not: a chord that opened a list would be a chord that does nothing on its own.
            Self::Bullet
            | Self::Ordered
            | Self::Task
            | Self::Quote
            | Self::Fence
            | Self::Rule
            | Self::Headings
            | Self::More => {
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
            Self::Undo
                | Self::Cut
                | Self::Bold
                | Self::Heading(1)
                | Self::Headings
                | Self::Bullet
                | Self::Quote
                | Self::More
        )
    }

    /// Whether this button's face is a **letterform by design** rather than a missing icon.
    ///
    /// Only the headings, and it is the distinction that decides how big the face is drawn. A
    /// letterform standing in for an icon has to hold its own beside one, so it is set near the icon
    /// size; the *fallback* words a machine with no icon font draws are set small, because "Paste"
    /// at that size would not fit a button. The two look identical from the outside — both are text
    /// where an icon might have been — and sizing them the same made `H1` read as a caption between
    /// two pictures.
    #[must_use]
    pub fn letters(self) -> bool {
        matches!(self, Self::Heading(_) | Self::Headings)
    }

    /// The list this button opens instead of editing anything, if it opens one.
    ///
    /// **Two buttons on this bar are not a second door to a chord**, which is the rule every other
    /// one follows. They open a panel; the panel's rows are the doors. Answering it here rather than
    /// with two special cases in the click path is what keeps that path one lookup, and what makes
    /// "did this button perform something?" a question with one answer.
    ///
    /// `More`'s rows are not known here — they are whatever [`overflowing`] displaced, which depends
    /// on the window — so this returns the *kind* and the caller supplies the contents.
    #[must_use]
    pub fn opens(self) -> Option<Menu> {
        match self {
            Self::Headings => Some(Menu::Headings),
            Self::More => Some(Menu::Overflow),
            _ => None,
        }
    }
}

/// Which list a button opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// Every heading level, one to six.
    Headings,
    /// Whatever did not fit on the bar.
    Overflow,
}

/// The heading levels the list offers.
///
/// **Six, where the bar only ever had room for three.** Collapsing them into one button is what paid
/// for the other three, and H4 to H6 stop being chord-only as a result — the MDEDIT-92 coverage audit
/// flagged that and no ticket had picked it up.
pub const HEADINGS: &[Action] = &[
    Action::Heading(1),
    Action::Heading(2),
    Action::Heading(3),
    Action::Heading(4),
    Action::Heading(5),
    Action::Heading(6),
];

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
    // One button, six levels, since MDEDIT-116. It was `Heading(1)`, `Heading(2)`, `Heading(3)`,
    // and dropping two of them is what made room for MDEDIT-87's Image and Table.
    Action::Headings,
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

/// A row of buttons, and whatever would not fit on it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    /// What is drawn, in order. Ends with [`Action::More`] when `dropped` is not empty.
    pub buttons: Vec<ButtonRect>,
    /// What `More` opens. Empty when everything fitted, and then there is no `More` either.
    pub dropped: Vec<Action>,
}

/// Lays the buttons out, and puts the ones that do not fit behind an overflow button.
///
/// MDEDIT-116, and the difference from [`layout`] is the whole ticket: that one **stops at the
/// edge**, which is correct for the floating bar and was silently losing controls on the fixed one.
/// A window narrow enough to displace Link used to just not have Link.
///
/// # Two things this has to get right
///
/// **The reserve is conditional.** Room for `More` is only taken when `More` is actually needed, so
/// a wide window is not one button short for the sake of an overflow that never appears. That means
/// laying out twice in the overflowing case — once to discover the shortfall, once with the reserve
/// — which is a few dozen comparisons and happens once per frame.
///
/// **What is drawn and what is behind the button come from one call.** They are two halves of one
/// partition, and computing them separately is how an action ends up in neither. `every_button_is_
/// either_drawn_or_in_the_overflow` is the test that says so.
#[must_use]
pub fn overflowing(actions: &[Action], glyph_widths: &[f32], available_width: f32) -> Row {
    let placed = layout(actions, glyph_widths, available_width);
    if placed.len() == actions.len() {
        return Row {
            buttons: placed,
            dropped: Vec::new(),
        };
    }

    // `More` is a group of its own at the right-hand end, so it costs a gap as well as a button.
    let more = (MORE_WIDTH + PADDING * 2.0).max(MIN_WIDTH);
    let reserve = more + GROUP_GAP + PADDING;
    let mut buttons = layout(actions, glyph_widths, available_width - reserve);
    let dropped = actions[buttons.len().min(actions.len())..].to_vec();

    let x = buttons.last().map_or(PADDING, ButtonRect::right) + GROUP_GAP;
    buttons.push(ButtonRect {
        action: Action::More,
        x,
        width: more,
    });
    Row { buttons, dropped }
}

/// What [`overflowing`] budgets for the overflow button's glyph.
///
/// A constant rather than a measurement because the reserve has to be known *before* the row is laid
/// out, and the row is what would supply the measurement. `More` is one icon like any other, so this
/// is the icon size — and [`MIN_WIDTH`] catches it if the fallback ellipsis measures narrower.
const MORE_WIDTH: f32 = 22.0;

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
