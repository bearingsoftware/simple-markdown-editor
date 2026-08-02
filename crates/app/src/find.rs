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

/// How tall the bar is. The formatting bar's height, because two bands of chrome that differ by
/// three pixels look like a mistake rather than like a distinction.
pub const HEIGHT: f32 = 30.0;

/// Space at the bar's edges and between its parts.
const PADDING: f32 = 6.0;

/// How wide a button is. Fixed rather than measured: all four glyphs are one or two characters, and
/// a row of buttons that are each a different width reads as ragged.
const BUTTON: f32 = 24.0;

/// The narrowest the text box is allowed to get before the buttons stop being given their room.
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
}

impl Button {
    /// What is drawn on it.
    ///
    /// Text, like the formatting bar's, and monochrome for the same reason MDEDIT-52 gave up on `☑`
    /// and `🔗`: an arrow taken from the emoji block resolves to the colour font and reads as a lit
    /// button sitting in a row of unlit ones.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Case => "Aa",
            Self::Previous => "<",
            Self::Next => ">",
            Self::Close => "X",
        }
    }
}

/// The buttons, left to right, as they are drawn.
pub const BUTTONS: [Button; 4] = [Button::Case, Button::Previous, Button::Next, Button::Close];

/// Where a button landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonRect {
    /// Which button.
    pub button: Button,
    /// Left edge, from the window's.
    pub x: f32,
    /// How wide.
    pub width: f32,
}

impl ButtonRect {
    /// Whether a window x is over this button.
    #[must_use]
    pub fn contains(self, x: f32) -> bool {
        x >= self.x && x < self.x + self.width
    }
}

/// Where every part of the bar goes, in device-independent pixels from the window's left edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The text box: left edge, then width.
    pub field: (f32, f32),
    /// Left edge of the "3 of 17" label.
    pub count: f32,
    /// The buttons.
    pub buttons: Vec<ButtonRect>,
}

impl Layout {
    /// Whether a window x is over the text box.
    #[must_use]
    pub fn over_field(&self, x: f32) -> bool {
        x >= self.field.0 && x < self.field.0 + self.field.1
    }

    /// Which button a window point is over, `y` measured from the bar's own top edge.
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Button> {
        if !(0.0..HEIGHT).contains(&y) {
            return None;
        }
        self.buttons
            .iter()
            .find(|rect| rect.contains(x))
            .map(|rect| rect.button)
    }
}

/// Works out where the bar's parts go in a window `width` wide.
///
/// The buttons are packed against the right edge and get their room first, because they are the
/// only fixed-size things here and a button that has been squeezed out cannot be pressed at all.
/// The count sits to their left. The field takes the rest, down to a floor, after which it
/// takes the count's space instead — a bar too narrow to say "3 of 17" is still a bar you can type
/// in.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "there are four buttons, and there is no count of anything else here"
)]
pub fn layout(width: f32, count_width: f32) -> Layout {
    let buttons_width = BUTTON * BUTTONS.len() as f32;
    let buttons_left = (width - PADDING - buttons_width).max(PADDING);
    let buttons = BUTTONS
        .iter()
        .enumerate()
        .map(|(index, &button)| ButtonRect {
            button,
            x: BUTTON.mul_add(index as f32, buttons_left),
            width: BUTTON,
        })
        .collect();

    let count = (buttons_left - PADDING - count_width).max(PADDING);
    let field_width = count - PADDING - PADDING;
    // Under the floor the count is not drawn at all — `count_label` is asked for nothing to show —
    // and the field runs up to the buttons instead.
    let field = if field_width < MIN_FIELD {
        (PADDING, (buttons_left - PADDING - PADDING).max(1.0))
    } else {
        (PADDING, field_width)
    };
    Layout {
        field,
        count,
        buttons,
    }
}

/// The find bar: what was typed, what it matched, and which match is current.
///
/// # It recomputes exactly when it must
///
/// [`refresh`](Self::refresh) is given the document's text and revision every frame the bar is
/// open, and rescans only when one of the three things a match list depends on has changed: the
/// needle, the case flag, or the document. So typing in the document with the bar open costs one
/// scan per edit, typing in the field costs one per keystroke, and a still frame costs a comparison.
///
/// The alternative was to put the scan on the idle timer, as the word count is. It is not, because
/// a stale highlight sits over the wrong bytes and looks like a bug, whereas a stale word count just
/// looks like an old number.
#[derive(Debug, Clone, Default)]
pub struct Bar {
    /// What is being searched for.
    pub field: Field,
    /// Whether case matters. Off by default, which is what people expect of a search box and the
    /// opposite of what a programmer's editor usually does.
    pub case_sensitive: bool,
    matches: Vec<ByteSpan>,
    /// The needle, case flag, document and revision `matches` was computed from.
    computed: Option<Stamp>,
}

/// Everything a match list depends on.
///
/// The document is in here because a revision counts a document's own edits and says nothing about
/// *which* document — two tabs opened and not yet typed in are both at revision zero, so without it
/// switching between them would keep the first one's offsets and light the wrong bytes.
type Stamp = (String, bool, usize, u64);

/// A key press the find bar answers to, decoded from a winit event.
///
/// The same reason [`field::Press`](crate::field::Press) exists, one level up: a winit
/// [`KeyEvent`](winit::event::KeyEvent) cannot be built by a test, so if Enter meant Find Next only
/// inside a `match` on one, nothing could check that it did. Decoding is the one untestable step and
/// it is three lines; everything after it is reachable from
/// [`Editor::apply_find_key`](crate::editor::Editor::apply_find_key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// Goes to the text field.
    Edit(crate::field::Press),
    /// Enter, or F3.
    Next,
    /// Shift+Enter, or Shift+F3.
    Previous,
    /// Escape.
    Close,
}

impl Bar {
    /// Every match, in order.
    #[must_use]
    pub fn matches(&self) -> &[ByteSpan] {
        &self.matches
    }

    /// Rescans if anything the match list depends on has moved. Returns whether it did.
    pub fn refresh(&mut self, source: &str, document: usize, revision: u64) -> bool {
        let key = (
            self.field.text().to_owned(),
            self.case_sensitive,
            document,
            revision,
        );
        if self.computed.as_ref() == Some(&key) {
            return false;
        }
        self.matches = matches(source, self.field.text(), self.case_sensitive);
        self.computed = Some(key);
        true
    }

    /// What the bar says about where you are: `"3 of 17"`, `"No results"`, or nothing.
    ///
    /// Nothing while the field is empty, because a bar that says "No results" before anything has
    /// been typed is telling the user off for not having started.
    #[must_use]
    pub fn count_label(&self, selection: ByteSpan) -> Option<String> {
        if self.field.is_empty() {
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
