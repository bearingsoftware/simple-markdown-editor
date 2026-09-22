//! Finding literal text in a document: the search itself, and the bar it is driven from.
//!
//! Two halves, in that order. The top of this file is functions over a `&str` — no editor, no
//! window, nothing to build to test them. The bottom is [`Bar`], which holds what the user typed
//! and where the parts of it are drawn, and still knows nothing about a GPU or a text engine. What
//! is left for [`Editor`](crate::editor::Editor) is the wiring, which is the half that has to be
//! looked at rather than asserted.
//!
//! # Why it is here and not in `mdedit-document`
//!
//! Nothing about this needs the rope, the segmentation or the block list — it needs the source
//! bytes and a needle. Keeping it in the app crate keeps the document crate about the document, and
//! keeps these testable without building one.
//!
//! # It searches the source, never the decoration
//!
//! A search for `**` finds the emphasis markers, because what is searched is the buffer rather than
//! what is drawn. That falls out of taking a `&str`, and it is the behaviour a Markdown editor
//! should have: the file is the thing, and the concealment is a way of looking at it. A match under
//! concealed syntax is reached the same way the caret reaches it — landing there reveals it, which
//! is [`Document::is_revealed`](mdedit_document::Document::is_revealed) doing its existing job.
//!
//! # No regex
//!
//! A dependency and a footgun UI, for a feature nobody has asked for yet. Literal text only, and if
//! it is missed the place to add it is [`matches()`].

use mdedit_offsets::{ByteOffset, ByteSpan};

use crate::field::Field;

/// Every non-overlapping occurrence of `needle` in `haystack`, in order.
///
/// An empty needle matches nothing rather than matching everywhere, which is what makes "type into
/// the box and watch the highlights appear" start from a blank bar rather than from the whole
/// document lit up.
#[must_use]
pub fn matches(haystack: &str, needle: &str, case_sensitive: bool) -> Vec<ByteSpan> {
    if needle.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return haystack
            .match_indices(needle)
            .map(|(at, found)| ByteSpan::from(at..at + found.len()))
            .collect();
    }

    // The first character the needle folds to. Every candidate position has to begin with it, and
    // checking that first is what makes this affordable: without it a 224 KB document paid a full
    // folded comparison at every one of its characters, which measured 5.8 ms per keystroke against
    // a 3 ms budget. With it, all but the real candidates cost one byte comparison.
    let Some(first) = needle.chars().flat_map(char::to_lowercase).next() else {
        return Vec::new();
    };

    // ASCII is compared as a byte, without decoding a `char` at all. That is not a micro-optimisation
    // for its own sake: this runs over every byte of the buffer on the keystroke path, and decoding
    // each one cost 2 ms on a 224 KB document against a 3 ms budget for the whole frame.
    let ascii_first = first.is_ascii().then_some(first as u8);
    let bytes = haystack.as_bytes();

    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        let (candidate, step) = if byte.is_ascii() {
            // A needle folding to something non-ASCII can never begin at an ASCII byte, so the
            // `None` case is a skip rather than a fallback.
            (ascii_first == Some(byte.to_ascii_lowercase()), 1)
        } else {
            // Not the exotic case it looks like: `İ` folds to `i` plus a combining dot, so an ASCII
            // needle can begin at a multi-byte character and the decode is not skippable here.
            let character = haystack[at..]
                .chars()
                .next()
                .unwrap_or(char::REPLACEMENT_CHARACTER);
            (fold_first(character) == first, character.len_utf8())
        };

        if candidate {
            // An ASCII needle against ASCII source is a byte comparison, and that is the whole of
            // the remaining cost: the outer loop finds a candidate at every `a` in the buffer, and
            // rejecting each of those by building two folding iterators is what a rescan was
            // actually spending its time on. `ascii_match` answers `None` when it cannot be sure.
            let length = match ascii_match(&bytes[at..], needle) {
                Some(true) => Some(needle.len()),
                Some(false) => None,
                None => folded_match(&haystack[at..], needle),
            };
            if let Some(length) = length {
                found.push(ByteSpan::from(at..at + length));
                // A non-empty needle folds to at least one character and so consumes at least one,
                // which is what stops this being a loop that does not advance.
                at += length;
                continue;
            }
        }
        at += step;
    }
    found
}

/// Whether an ASCII `needle` matches the start of `source`, or `None` when it cannot say.
///
/// It cannot say when either side has a byte outside ASCII, because folding is then not a per-byte
/// operation and can change a length — `İ` is one character that lowercases to two, and an ASCII
/// needle of `i` must neither match it nor be told it does not. Both of those go to
/// [`folded_match`], which is right and slower.
fn ascii_match(source: &[u8], needle: &str) -> Option<bool> {
    if !needle.is_ascii() {
        return None;
    }
    let head = source.get(..needle.len())?;
    if !head.is_ascii() {
        return None;
    }
    Some(head.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The first character something lowercases to.
///
/// ASCII takes a byte operation rather than building a [`char::to_lowercase`] iterator, because in
/// this editor's documents almost every character is ASCII and this runs once per character of the
/// buffer per rescan.
fn fold_first(character: char) -> char {
    if character.is_ascii() {
        character.to_ascii_lowercase()
    } else {
        character.to_lowercase().next().unwrap_or(character)
    }
}

/// How many bytes of `haystack` `needle` matches at offset zero, case-folded, or nothing.
///
/// # Why not lowercase both and use `find`
///
/// Because the answer has to be a length in the *original* string, and folding does not preserve
/// lengths: `İ` is one character and lowercases to two, `SS` matches `ß` in neither direction
/// consistently. A lowercased copy would give offsets into the copy, and mapping those back is the
/// same walk as this one with a second allocation in front of it.
///
/// A source character that folds to several is matched only if the needle contains all of them, so
/// a match never ends half way through one character's expansion.
fn folded_match(haystack: &str, needle: &str) -> Option<usize> {
    let mut want = needle.chars().flat_map(char::to_lowercase);
    let mut source = haystack.char_indices();
    let mut consumed = 0;
    loop {
        let Some(expected) = want.next() else {
            return Some(consumed);
        };
        let (index, character) = source.next()?;
        let mut folded = character.to_lowercase();
        if folded.next() != Some(expected) {
            return None;
        }
        for extra in folded {
            if want.next() != Some(extra) {
                return None;
            }
        }
        consumed = index + character.len_utf8();
    }
}

/// The first match starting at or after `from`, wrapping to the first.
///
/// `from` is the caller's business: Find Next passes the end of the current match, so pressing it
/// twice does not land on the same one.
#[must_use]
pub fn next(spans: &[ByteSpan], from: ByteOffset) -> Option<usize> {
    if spans.is_empty() {
        return None;
    }
    let at = spans.partition_point(|span| span.start() < from);
    Some(if at == spans.len() { 0 } else { at })
}

/// The last match ending at or before `from`, wrapping to the last.
#[must_use]
pub fn previous(spans: &[ByteSpan], from: ByteOffset) -> Option<usize> {
    if spans.is_empty() {
        return None;
    }
    let at = spans.partition_point(|span| span.end() <= from);
    Some(if at == 0 { spans.len() - 1 } else { at - 1 })
}

/// Which match `span` is, if it is one exactly.
///
/// What the bar's "3 of 17" counts from. Exact rather than containing, because the selection is set
/// *to* a match when one is navigated to, and anything else means the user has moved on from it.
#[must_use]
pub fn index_of(spans: &[ByteSpan], span: ByteSpan) -> Option<usize> {
    spans
        .binary_search_by_key(&span.start(), |candidate| candidate.start())
        .ok()
        .filter(|&at| spans[at] == span)
}

/// The matches overlapping `within`, as a slice of the whole list.
///
/// Binary-searched rather than filtered, and that is the difference between a document with ten
/// thousand matches costing nothing to draw and costing ten thousand comparisons per visual row.
/// The same lesson as `Document::block_at` in MDEDIT-37.
#[must_use]
pub fn within(spans: &[ByteSpan], within: ByteSpan) -> &[ByteSpan] {
    let from = spans.partition_point(|span| span.end() <= within.start());
    let to = spans.partition_point(|span| span.start() < within.end());
    &spans[from..to]
}

/// What the bar is being used for.
///
/// Three, because they are three things a user is doing rather than three settings — and because the
/// bar is a different shape for each: Replace is two rows, and Go to Line has no matches to count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Find only.
    #[default]
    Find,
    /// Find, with a second field to replace with.
    Replace,
    /// A line number.
    GoTo,
}

/// Which of a bar's fields is taking keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Focus {
    /// The search term, or the line number — whichever the mode has on its first row.
    #[default]
    Primary,
    /// The replacement. Only reachable in [`Mode::Replace`].
    Secondary,
}

/// How tall one row of the bar is. The formatting bar's height, because two bands of chrome that
/// differ by three pixels look like a mistake rather than like a distinction.
pub const HEIGHT: f32 = 30.0;

/// How tall the whole bar is in `mode`.
#[must_use]
pub fn height(mode: Mode) -> f32 {
    if mode == Mode::Replace {
        HEIGHT * 2.0
    } else {
        HEIGHT
    }
}

/// How wide the panel wants to be in `mode`, before the room it has is taken into account
/// (MDEDIT-177).
///
/// A number rather than a measurement, and that is the point of it: the panel floats over the text,
/// so a width computed from its contents would change as the count went from "3 of 17" to "10 of
/// 17" and the panel would breathe while you navigated. The layout below already gives way
/// gracefully when the room is less than this — the count goes first, then the field takes its
/// space — so a fixed want plus a clamp is the whole of the sizing.
///
/// Go to Line is narrower because it is a line number and a close button, and a panel three times
/// the width of what it holds reads as an unfinished one.
#[must_use]
pub fn width(mode: Mode) -> f32 {
    if mode == Mode::GoTo { 220.0 } else { 380.0 }
}

/// Space at the bar's edges and between its parts.
const PADDING: f32 = 6.0;

/// The narrowest panel `mode` can be drawn in without a button falling off it (MDEDIT-177).
///
/// **A floor, not a preference.** [`layout`] packs each row's buttons against the right edge and
/// floors their left at [`PADDING`], so in a panel narrower than this the last cells run past the
/// panel's own width — and since [`Layout::local`] rejects a point outside that width, they would be
/// *drawn* over the document and answer no click. Close being unpressable is the case that matters:
/// Esc still shuts the bar, but a button you can see and cannot press is a defect either way.
///
/// It was not a question while the bar spanned the window: the width it was laid out in was the
/// window's, and a window that narrow has no room for the document either.
///
/// `widths` is what [`layout`] takes, and measured for the same reason.
#[must_use]
pub fn least_width(mode: Mode, widths: &[f32]) -> f32 {
    let widest_row = (0..2)
        .map(|row| {
            buttons(mode)
                .iter()
                .enumerate()
                .filter(|(_, button)| button.row() == row)
                .map(|(at, _)| cell(widths, at))
                .sum::<f32>()
        })
        .fold(0.0_f32, f32::max);
    widest_row + PADDING * 2.0
}

/// How wide the cell for the button at `index` is.
///
/// Never narrower than the symbols' width, so the four on the first row line up as a row of squares,
/// and never narrower than what the glyph measured — two of them are words.
fn cell(widths: &[f32], index: usize) -> f32 {
    widths
        .get(index)
        .map_or(BUTTON, |measured| (measured + PADDING * 2.0).max(BUTTON))
}

/// The narrowest a button may be. Wider ones are measured — see [`layout`].
const BUTTON: f32 = 24.0;

/// The narrowest the text box is allowed to get before the count stops being given its room.
///
/// Below this the field is unusable, and a window that narrow has bigger problems than the count
/// label — so the count is what goes, and the field keeps its floor.
const MIN_FIELD: f32 = 60.0;

/// A thing on the bar that can be clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Match case, on or off. The one button here that is a state and therefore lights.
    Case,
    /// Previous match.
    Previous,
    /// Next match.
    Next,
    /// Close the bar.
    Close,
    /// Replace the current match and move on.
    Replace,
    /// Replace every match, as one undo step.
    ReplaceAll,
}

impl Button {
    /// What is drawn on it.
    ///
    /// Text, like the formatting bar's, and monochrome for the same reason MDEDIT-52 gave up on the
    /// ballot box and the link emoji: a glyph taken from the emoji block resolves to the colour font
    /// and reads as a lit button sitting in a row of unlit ones.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Case => "Aa",
            Self::Previous => "<",
            Self::Next => ">",
            Self::Close => "X",
            Self::Replace => "Replace",
            Self::ReplaceAll => "All",
        }
    }

    /// Which row of the bar it sits on, counting from the top.
    ///
    /// A row *number* rather than a y offset, so deciding which buttons share a row is an integer
    /// comparison. Comparing two `f32`s for equality to group them would work and would still be
    /// the kind of thing that fails once somebody scales a height.
    #[must_use]
    pub fn row(self) -> usize {
        match self {
            Self::Replace | Self::ReplaceAll => 1,
            _ => 0,
        }
    }
}

const FIND_BUTTONS: [Button; 4] = [Button::Case, Button::Previous, Button::Next, Button::Close];

const REPLACE_BUTTONS: [Button; 6] = [
    Button::Case,
    Button::Previous,
    Button::Next,
    Button::Close,
    Button::Replace,
    Button::ReplaceAll,
];

/// Go to Line has no matches to walk and nothing to match case against, so it gets the one button
/// that still means something. Enter is what performs it.
const GOTO_BUTTONS: [Button; 1] = [Button::Close];

/// The buttons `mode` shows, in the order they are drawn.
#[must_use]
pub fn buttons(mode: Mode) -> &'static [Button] {
    match mode {
        Mode::Find => &FIND_BUTTONS,
        Mode::Replace => &REPLACE_BUTTONS,
        Mode::GoTo => &GOTO_BUTTONS,
    }
}

/// Where a button landed, **in the panel's coordinates** — both edges are measured from
/// [`Layout::origin`], not from the window's corner. See [`Layout`] for why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonRect {
    /// Which button.
    pub button: Button,
    /// Left edge, from the panel's.
    pub x: f32,
    /// Top edge, from the panel's.
    pub y: f32,
    /// How wide.
    pub width: f32,
}

impl ButtonRect {
    /// Whether a point in the bar is over this button, `y` measured from the bar's top edge.
    #[must_use]
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + HEIGHT
    }
}

/// Where every part of the panel goes, in device-independent pixels **from the panel's top-left
/// corner** — [`origin`](Self::origin) is where that corner is in the window.
///
/// # Why the parts are not in window coordinates
///
/// They were, until MDEDIT-177: the bar spanned the window and its left edge was the window's, so
/// the only conversion anyone needed was the vertical one, and every caller did that subtraction
/// itself. A panel that floats has an origin on both axes and it moves — so the subtraction is done
/// here, once, by [`over_field`](Self::over_field) and [`hit_test`](Self::hit_test), which take
/// window coordinates. The same argument `ContentRect` makes for the document.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// What it was laid out for.
    pub mode: Mode,
    /// Where the panel's top-left corner is in the window.
    pub origin: (f32, f32),
    /// How wide the panel is. Its height is [`height`]`(mode)`.
    pub width: f32,
    /// The first row's text box: left edge, then width.
    pub field: (f32, f32),
    /// The second row's, in [`Mode::Replace`] only.
    pub replace: Option<(f32, f32)>,
    /// Left edge of the "3 of 17" label.
    pub count: f32,
    /// The buttons.
    pub buttons: Vec<ButtonRect>,
}

impl Layout {
    /// How tall the panel is.
    #[must_use]
    pub fn height(&self) -> f32 {
        height(self.mode)
    }

    /// Whether a window point is on the panel at all.
    ///
    /// What makes the panel opaque to clicks (MDEDIT-177). It floats over the document now, so a
    /// point between two of its controls is still the panel's — and a caller that asked only
    /// `hit_test` would let that click put the caret in the text underneath, which is the mistake
    /// every piece of chrome in this editor has made once.
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.local(x, y).is_some()
    }

    /// A window point in the panel's own coordinates, or `None` when it is not on the panel.
    fn local(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (left, top) = self.origin;
        let (x, y) = (x - left, y - top);
        ((0.0..self.width).contains(&x) && (0.0..self.height()).contains(&y)).then_some((x, y))
    }

    /// Which field a window point is over.
    #[must_use]
    pub fn over_field(&self, x: f32, y: f32) -> Option<Focus> {
        let (x, y) = self.local(x, y)?;
        let inside = |box_: (f32, f32)| x >= box_.0 && x < box_.0 + box_.1;
        if (0.0..HEIGHT).contains(&y) && inside(self.field) {
            return Some(Focus::Primary);
        }
        match self.replace {
            Some(box_) if (HEIGHT..HEIGHT * 2.0).contains(&y) && inside(box_) => {
                Some(Focus::Secondary)
            }
            _ => None,
        }
    }

    /// Which button a window point is over.
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Button> {
        let (x, y) = self.local(x, y)?;
        self.buttons
            .iter()
            .find(|rect| rect.contains(x, y))
            .map(|rect| rect.button)
    }
}

/// Works out where the panel's parts go, given where it sits and how wide it is.
///
/// `origin` is the panel's top-left corner in the window; everything computed here is measured from
/// it, and `width` is the panel's rather than the window's (MDEDIT-177).
///
/// `widths` is how wide each of [`buttons`]`(mode)`'s glyphs measured, in the same order. Measured
/// rather than assumed because two of them are words: "Replace" at a fixed 24 pixels would be a
/// button with three letters on it. A cell is never narrower than the symbols' width, so the four
/// on the first row still line up as a row of squares.
///
/// Each row's buttons are packed against the right edge and get their room first — they are the only
/// fixed-size things here, and a button squeezed out cannot be pressed at all. The count sits to
/// their left on the first row. Each field takes what is left of its own row, down to a floor, after
/// which it takes the count's space too: a bar too narrow to say "3 of 17" is still a bar you can
/// type in.
#[must_use]
pub fn layout(
    mode: Mode,
    origin: (f32, f32),
    width: f32,
    count_width: f32,
    widths: &[f32],
) -> Layout {
    let all = buttons(mode);
    let cell = |index: usize| cell(widths, index);

    let mut rects = Vec::with_capacity(all.len());
    // Where each row's leftmost button starts, which is also where that row's field has to stop.
    let mut row_left = [width - PADDING, width - PADDING];
    for (row, left) in row_left.iter_mut().enumerate() {
        let on_this_row: Vec<usize> = (0..all.len()).filter(|&at| all[at].row() == row).collect();
        if on_this_row.is_empty() {
            continue;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "two rows, and the index is which one"
        )]
        let top = row as f32 * HEIGHT;
        let total: f32 = on_this_row.iter().map(|&at| cell(at)).sum();
        let mut x = (width - PADDING - total).max(PADDING);
        *left = x;
        for at in on_this_row {
            rects.push(ButtonRect {
                button: all[at],
                x,
                y: top,
                width: cell(at),
            });
            x += cell(at);
        }
    }

    let count = (row_left[0] - PADDING - count_width).max(PADDING);
    let room = count - PADDING - PADDING;
    // Under the floor the count is not drawn at all and the field runs up to the buttons instead.
    let field = if room < MIN_FIELD {
        (PADDING, (row_left[0] - PADDING - PADDING).max(1.0))
    } else {
        (PADDING, room)
    };
    let replace =
        (mode == Mode::Replace).then(|| (PADDING, (row_left[1] - PADDING - PADDING).max(1.0)));

    Layout {
        mode,
        origin,
        width,
        field,
        replace,
        count,
        buttons: rects,
    }
}

/// The find bar: what was typed, what it matched, and which match is current.
///
/// # It recomputes exactly when it must
///
/// [`refresh`](Self::refresh) is given the document's text and revision every frame the bar is
/// open, and rescans only when one of the things a match list depends on has changed: the needle,
/// the case flag, the mode, or the document. So typing in the document with the bar open costs one
/// scan per edit, typing in the field costs one per keystroke, and a still frame costs a comparison.
///
/// The alternative was to put the scan on the idle timer, as the word count is. It is not, because
/// a stale highlight sits over the wrong bytes and looks like a bug where a stale word count just
/// looks like an old number — and because it measured cheap enough not to need it. See
/// `crates/app/tests/find_cost.rs`, which is the thing that decides that again.
#[derive(Debug, Clone, Default)]
pub struct Bar {
    /// What is being searched for.
    pub field: Field,
    /// What a replacement puts in its place.
    pub replace: Field,
    /// The line number Go to Line will jump to.
    ///
    /// Its own field rather than the search term's, so Ctrl+G does not throw away what you were
    /// looking for and Esc then Ctrl+F still has it.
    pub line: Field,
    /// Whether case matters. Off by default, which is what people expect of a search box and the
    /// opposite of what a programmer's editor usually does.
    pub case_sensitive: bool,
    /// What the bar is for.
    pub mode: Mode,
    /// Which field is taking keys.
    pub focus: Focus,
    matches: Vec<ByteSpan>,
    /// The needle, case flag, mode, document and revision `matches` was computed from.
    computed: Option<Stamp>,
}

/// Everything a match list depends on.
///
/// The document is in here because a revision counts a document's own edits and says nothing about
/// *which* document — two tabs opened and not yet typed in are both at revision zero, so without it
/// switching between them would keep the first one's offsets and light the wrong bytes.
type Stamp = (String, bool, Mode, u64, u64);

/// A key press the find bar answers to, decoded from a winit event.
///
/// The same reason [`field::Press`](crate::field::Press) exists, one level up: a winit
/// [`KeyEvent`](winit::event::KeyEvent) cannot be built by a test, so if Enter meant Find Next only
/// inside a `match` on one, nothing could check that it did. Decoding is the one untestable step and
/// it is a dozen lines; everything after it is reachable from
/// [`Editor::apply_find_key`](crate::editor::Editor::apply_find_key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// Goes to whichever field has focus.
    Edit(crate::field::Press),
    /// Enter, or F3. In [`Mode::GoTo`] this is what performs the jump.
    Next,
    /// Shift+Enter, or Shift+F3.
    Previous,
    /// Escape.
    Close,
    /// Tab, which moves between the two fields of [`Mode::Replace`] and does nothing elsewhere.
    NextField,
}

impl Bar {
    /// Every match, in order. Always empty in [`Mode::GoTo`], which searches for nothing.
    #[must_use]
    pub fn matches(&self) -> &[ByteSpan] {
        &self.matches
    }

    /// The field keys are going to.
    #[must_use]
    pub fn active(&self) -> &Field {
        match (self.mode, self.focus) {
            (Mode::GoTo, _) => &self.line,
            (Mode::Replace, Focus::Secondary) => &self.replace,
            _ => &self.field,
        }
    }

    /// The field keys are going to, to be written into.
    pub fn active_mut(&mut self) -> &mut Field {
        match (self.mode, self.focus) {
            (Mode::GoTo, _) => &mut self.line,
            (Mode::Replace, Focus::Secondary) => &mut self.replace,
            _ => &mut self.field,
        }
    }

    /// Sends one press to whichever field has focus. Returns whether the text changed.
    ///
    /// **Go to Line takes digits only**, filtered here rather than at the call site so a paste is
    /// treated the same as a keystroke. Nothing else about the field changes, so Backspace, the
    /// arrows and a selection all still work.
    pub fn press(&mut self, press: &crate::field::Press, shift: bool) -> bool {
        let digits_only = self.mode == Mode::GoTo;
        let field = self.active_mut();
        if digits_only {
            if let crate::field::Press::Text(text) = press {
                let kept: String = text.chars().filter(char::is_ascii_digit).collect();
                if kept.is_empty() {
                    return false;
                }
                field.insert(&kept);
                return true;
            }
        }
        field.apply(press, shift)
    }

    /// Moves between the two fields of [`Mode::Replace`]. Returns whether anything moved.
    pub fn next_field(&mut self) -> bool {
        if self.mode != Mode::Replace {
            return false;
        }
        self.focus = match self.focus {
            Focus::Primary => Focus::Secondary,
            Focus::Secondary => Focus::Primary,
        };
        true
    }

    /// The line the Go to Line field names, counting from 1 as the status bar does.
    ///
    /// `None` while the field is empty, holds only zeroes, or holds a number too large for a
    /// `usize` — which is the only way something that is not a number gets here, since the field
    /// takes digits only.
    #[must_use]
    pub fn line_number(&self) -> Option<usize> {
        self.line.text().parse().ok().filter(|&line| line >= 1)
    }

    /// Rescans if anything the match list depends on has moved. Returns whether it did.
    pub fn refresh(&mut self, source: &str, document: u64, revision: u64) -> bool {
        let key = (
            self.field.text().to_owned(),
            self.case_sensitive,
            self.mode,
            document,
            revision,
        );
        if self.computed.as_ref() == Some(&key) {
            return false;
        }
        // Go to Line searches for nothing. Keeping the old list would light matches from a search
        // the user has moved on from, and computing a new one would be work for a mode with no use
        // for it.
        self.matches = if self.mode == Mode::GoTo {
            Vec::new()
        } else {
            matches(source, self.field.text(), self.case_sensitive)
        };
        self.computed = Some(key);
        true
    }

    /// What the bar says about where you are: `"3 of 17"`, `"No results"`, or nothing.
    ///
    /// Nothing while the field is empty, because a bar that says "No results" before anything has
    /// been typed is telling the user off for not having started. Nothing in [`Mode::GoTo`] either,
    /// which is not counting anything.
    #[must_use]
    pub fn count_label(&self, selection: ByteSpan) -> Option<String> {
        if self.mode == Mode::GoTo || self.field.is_empty() {
            return None;
        }
        if self.matches.is_empty() {
            return Some("No results".to_owned());
        }
        Some(match index_of(&self.matches, selection) {
            Some(at) => format!("{} of {}", at + 1, self.matches.len()),
            // Matches exist but the caret is not on one — the bar has just been opened, or the user
            // has clicked away. Saying how many there are is still useful; claiming to be on one
            // would not be.
            None => format!("{} matches", self.matches.len()),
        })
    }
}
