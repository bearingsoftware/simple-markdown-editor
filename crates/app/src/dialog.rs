//! The modal panel that asks for a link's or an image's parts.
//!
//! Geometry and vocabulary only — no GPU types, no text engine, no window — as `toolbar.rs`,
//! `popup.rs` and `tabs.rs` are, and for the reason they give.
//!
//! # This reverses a decision that was written down
//!
//! `make_link` said the clipboard case was *"the case that needs no dialog at all — copy a link,
//! select some words, press Ctrl+K, done"*, and `insert_image` said parking the caret in the empty
//! parentheses was *"the whole of why there is no dialog"*. Both were deliberate.
//!
//! What they missed is that a link has **two** parts and an image has **three**, and inserting a
//! skeleton for you to fill in is only convenient when you already know the shape. The dialog asks
//! for the parts by name.
//!
//! **The fast path survives as one extra keystroke.** A URL on the clipboard pre-fills the URL
//! field and a selection pre-fills the text field, so copy-select-`Ctrl+K` becomes
//! copy-select-`Ctrl+K`-`Enter` — and you can see what it is about to write before it writes it.
//!
//! # Modal, and it says so
//!
//! The document dims behind it. A panel that takes the keyboard while the text behind it looks
//! ordinary is the thing a non-modal panel gets wrong, and this editor has exactly one other
//! keyboard-taking surface — the find bar — which announces itself by taking a strip of the window.
//!
//! `Esc` cancels, `Enter` accepts, `Tab` and `Shift+Tab` move between fields.

use crate::field::Field;

/// Which dialog this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `[text](url)`.
    Link,
    /// `![alt](location "title")`.
    Image,
}

impl Kind {
    /// What the panel is called.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Link => "Insert link",
            Self::Image => "Insert image",
        }
    }

    /// The fields, in tab order.
    ///
    /// **A link has two and an image has three**, which is the asymmetry a reader would ask about.
    /// Markdown allows a title on a link as well, and it is left out: a link's title shows as a
    /// browser tooltip and is almost never written, where an image's is genuinely used. A third
    /// field nobody fills in, on the dialog reached most often, is worse than the inconsistency.
    #[must_use]
    pub fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Link => &["URL", "Text"],
            Self::Image => &["Location", "Alt text", "Title"],
        }
    }

    /// Which field the destination is, so a caller knows where to put a clipboard URL.
    ///
    /// Zero for both, and named rather than written as `0` at each call site — the two dialogs
    /// having the same answer is a coincidence of their layouts rather than a rule.
    #[must_use]
    pub fn destination(self) -> usize {
        match self {
            Self::Link | Self::Image => 0,
        }
    }

    /// Which field is the visible text, so a caller knows where to put the selection.
    #[must_use]
    pub fn text(self) -> usize {
        match self {
            Self::Link | Self::Image => 1,
        }
    }
}

/// Width of the panel.
///
/// Wide enough for a URL that is not a toy. Fixed rather than proportional for the reason the file
/// pane's width is fixed: a panel that grew with the window would re-lay-out its fields on a resize
/// for no gain.
pub const WIDTH: f32 = 460.0;

/// Height of one field's row.
pub const ROW_HEIGHT: f32 = 30.0;

/// Space between rows.
pub const ROW_GAP: f32 = 6.0;

/// Space inside the panel's edge.
pub const PADDING: f32 = 16.0;

/// Width of a field's label column.
///
/// Set by the longest of them — "Location" — so the boxes line up between the two dialogs and the
/// panel does not appear to jump when one replaces the other.
pub const LABEL_WIDTH: f32 = 76.0;

/// Height of the title row.
pub const TITLE_HEIGHT: f32 = 30.0;

/// Size of the two buttons at the foot.
pub const BUTTON_WIDTH: f32 = 84.0;
/// Height of the two buttons at the foot.
pub const BUTTON_HEIGHT: f32 = 28.0;

/// How tall a panel with `fields` rows is.
#[must_use]
pub fn height_of(fields: usize) -> f32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "three fields at most, which is exact in an f32"
    )]
    let rows = fields as f32;
    PADDING
        + TITLE_HEIGHT
        + rows * ROW_HEIGHT
        + (rows - 1.0).max(0.0) * ROW_GAP
        + ROW_GAP
        + BUTTON_HEIGHT
        + PADDING
}

/// Where a panel of this height goes in a window this size.
///
/// **Centred horizontally, and a third of the way down rather than halfway.** A panel at the exact
/// centre covers the line the caret is on more often than not, and the thing being described is
/// usually the text behind it.
///
/// Clamped to zero on both axes, so a window smaller than the panel shows its top-left rather than
/// its middle — text clipped at the right and bottom is readable from its start.
#[must_use]
pub fn place(window_width: f32, window_height: f32, panel_height: f32) -> (f32, f32) {
    let x = ((window_width - WIDTH) / 2.0).max(0.0);
    let y = ((window_height - panel_height) / 3.0).max(0.0);
    (x, y)
}

/// What a click in the panel means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Put the caret in this field.
    Field(usize),
    /// Write the construct.
    Accept,
    /// Leave the document alone.
    Cancel,
}

/// Where one field's box sits, as `(x, y, width, height)`.
#[must_use]
pub fn field_rect(index: usize, at: (f32, f32)) -> (f32, f32, f32, f32) {
    #[allow(
        clippy::cast_precision_loss,
        reason = "three fields at most, which is exact in an f32"
    )]
    let row = index as f32;
    (
        at.0 + PADDING + LABEL_WIDTH,
        at.1 + PADDING + TITLE_HEIGHT + row * (ROW_HEIGHT + ROW_GAP),
        WIDTH - PADDING * 2.0 - LABEL_WIDTH,
        ROW_HEIGHT,
    )
}

/// Where the Cancel and Accept buttons sit, in that order.
///
/// Right-aligned with Accept last, which is the Windows order — the opposite of the Mac one, and
/// this is a Windows-first editor.
#[must_use]
pub fn button_rects(fields: usize, at: (f32, f32)) -> [(f32, f32, f32, f32); 2] {
    let y = at.1 + height_of(fields) - PADDING - BUTTON_HEIGHT;
    let accept_x = at.0 + WIDTH - PADDING - BUTTON_WIDTH;
    let cancel_x = accept_x - BUTTON_WIDTH - ROW_GAP;
    [
        (cancel_x, y, BUTTON_WIDTH, BUTTON_HEIGHT),
        (accept_x, y, BUTTON_WIDTH, BUTTON_HEIGHT),
    ]
}

/// What is under `(x, y)`, if anything is.
///
/// **Returns `None` for a click inside the panel but on nothing**, and the caller must still swallow
/// it — a modal panel that let a click on its own background reach the document would move the caret
/// out from under the thing it is about to edit.
#[must_use]
pub fn hit_test(fields: usize, x: f32, y: f32, at: (f32, f32)) -> Option<Hit> {
    let inside =
        |(bx, by, bw, bh): (f32, f32, f32, f32)| x >= bx && x < bx + bw && y >= by && y < by + bh;

    for index in 0..fields {
        if inside(field_rect(index, at)) {
            return Some(Hit::Field(index));
        }
    }
    let [cancel, accept] = button_rects(fields, at);
    if inside(cancel) {
        return Some(Hit::Cancel);
    }
    if inside(accept) {
        return Some(Hit::Accept);
    }
    None
}

/// Whether `(x, y)` is anywhere in the panel.
#[must_use]
pub fn contains(fields: usize, x: f32, y: f32, at: (f32, f32)) -> bool {
    x >= at.0 && x < at.0 + WIDTH && y >= at.1 && y < at.1 + height_of(fields)
}

/// An open dialog: which one, its fields, and which of them has the caret.
#[derive(Debug, Clone)]
pub struct Dialog {
    kind: Kind,
    fields: Vec<Field>,
    focused: usize,
}

impl Dialog {
    /// An empty dialog of this kind.
    #[must_use]
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            fields: kind.labels().iter().map(|_| Field::new()).collect(),
            focused: 0,
        }
    }

    /// Which dialog this is.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The fields, in tab order.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Which field has the caret.
    #[must_use]
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// The field with the caret, for a key to act on.
    pub fn focused_field(&mut self) -> &mut Field {
        &mut self.fields[self.focused]
    }

    /// Puts `text` in a field and leaves the caret at its end.
    ///
    /// Out-of-range is ignored rather than panicking: the index comes from [`Kind::text`], and a
    /// dialog that grew a field would otherwise crash on the pre-fill rather than skip it.
    pub fn prefill(&mut self, index: usize, text: &str) {
        if let Some(field) = self.fields.get_mut(index) {
            field.set_text(text);
        }
    }

    /// Moves the caret to the next field, or the previous one.
    ///
    /// Wraps, because a three-field form is a ring and Tab at the end of one is how everybody
    /// discovers that.
    pub fn focus_next(&mut self, forwards: bool) {
        let count = self.fields.len();
        self.focused = if forwards {
            (self.focused + 1) % count
        } else {
            (self.focused + count - 1) % count
        };
    }

    /// Puts the caret in a field, if that field exists.
    pub fn focus(&mut self, index: usize) {
        if index < self.fields.len() {
            self.focused = index;
        }
    }

    /// The Markdown this dialog would write, or `None` when there is not enough to write any.
    ///
    /// **`None` for an empty destination**, which is the whole of the validation: a link with no URL
    /// and an image with no location are the two cases where accepting would put a construct in the
    /// document that says nothing. Everything else is optional — an image with no alt text is poor
    /// practice and legal Markdown, and it is not this dialog's business to refuse it.
    #[must_use]
    pub fn markdown(&self) -> Option<String> {
        let destination = self.fields[self.kind.destination()].text().trim();
        if destination.is_empty() {
            return None;
        }
        let text = self.fields[self.kind.text()].text();
        let title = self.fields.get(2).map(Field::text).unwrap_or_default();

        let bang = match self.kind {
            Kind::Link => "",
            Kind::Image => "!",
        };
        let title = if title.trim().is_empty() {
            String::new()
        } else {
            format!(" \"{}\"", escape_title(title.trim()))
        };
        Some(format!(
            "{bang}[{text}]({}{title})",
            escape_destination(destination)
        ))
    }
}

/// A destination that would not survive being written bare, wrapped in angle brackets.
///
/// A space or a parenthesis ends the destination as far as the parser is concerned, so
/// `](my file.png)` is not a link to `my file.png` — it is a broken one. CommonMark's answer is
/// `<my file.png>`, and this is the one piece of escaping the dialog cannot skip: a path with a
/// space in it is the commonest thing anybody will paste into it.
fn escape_destination(destination: &str) -> String {
    if destination.contains([' ', '\t', '(', ')']) && !destination.starts_with('<') {
        format!("<{}>", destination.replace('>', "\\>"))
    } else {
        destination.to_owned()
    }
}

/// A title with its quotes escaped, since the title is written inside quotes.
fn escape_title(title: &str) -> String {
    title.replace('\\', "\\\\").replace('"', "\\\"")
}
