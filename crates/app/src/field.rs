//! A single line of editable text that is not the document.
//!
//! # Why this exists
//!
//! Until MDEDIT-44 the document was the only thing in this editor that accepted a keystroke. A find
//! bar cannot be built on it: it needs its own buffer, its own caret, its own selection, and a rule
//! about which of the two a key goes to. That is a component, not a field on [`Editor`], and it is
//! written here so that Replace, Go to Line and whatever dialog-ish thing comes next reuse it rather
//! than growing a second half-copy.
//!
//! [`Editor`]: crate::editor::Editor
//!
//! # What it deliberately is not
//!
//! A `String` and two byte offsets. Not a rope, and no undo history — a search box holds a few words
//! and is thrown away when the bar closes, and giving it the document's machinery would be building
//! the wrong thing carefully. If a field ever needs to hold a paragraph, that is the moment to
//! revisit this, and not before.
//!
//! It also holds **one line**. [`Field::insert`] drops line breaks rather than refusing the text
//! around them, because the thing that produces them is a paste of several lines into a search box,
//! and taking the first line of it is what every editor does.
//!
//! # Not yet: composition
//!
//! A field takes committed text only. `WindowEvent::Ime` goes to the document as it always has, so
//! composing Japanese into the search box does not work — the pre-edit would be drawn at the
//! document's caret, which is the wrong place, and the commit would land in the document. That is a
//! real gap for a CJK user searching CJK text and it is written down here rather than left to be
//! discovered.

/// A key press a field understands.
///
/// A second key vocabulary, and it earns its keep the same way [`Editor::apply_chord`] does: a winit
/// [`KeyEvent`](winit::event::KeyEvent) carries a private platform field and cannot be built by a
/// test, so a field driven straight from one would be a component nothing could press. MDEDIT-65 was
/// a chord that did the wrong thing with nothing able to catch it; this is the shape that stops the
/// same gap opening in the find bar.
///
/// [`Editor::apply_chord`]: crate::editor::Editor::apply_chord
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// Ordinary text, already committed. A field never sees a pre-edit — see the module docs.
    Text(String),
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Ctrl+A.
    SelectAll,
}

/// One line of editable text, with a caret and a selection.
///
/// Offsets are **bytes into [`text`](Self::text)**, and every one of them lands on a character
/// boundary because the only things that move the caret are the methods below. They are bare
/// `usize` rather than [`ByteOffset`](mdedit_offsets::ByteOffset) on purpose: that type indexes the
/// *document*, and a search box's third character is not a position in the file. Mixing the two is
/// exactly the bug class the newtype exists to stop, so the field does not borrow its name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Field {
    text: String,
    caret: usize,
    /// Where a shifted motion started. Equal to `caret` when nothing is selected.
    anchor: usize,
}

impl Field {
    /// An empty field.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What has been typed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether anything has been typed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Where the caret sits, as a byte offset into [`text`](Self::text).
    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected range, low end first. Empty when nothing is selected.
    #[must_use]
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// The selected text, or nothing when the selection is collapsed.
    #[must_use]
    pub fn selected_text(&self) -> Option<&str> {
        let (from, to) = self.selection();
        (from < to).then(|| &self.text[from..to])
    }

    /// Replaces the whole contents and selects it.
    ///
    /// Selected rather than left with a caret at the end, because the two callers — opening the bar
    /// over a selection, and reopening it on a previous search — both want the next keystroke to
    /// replace what is there. That is what every find bar does with a term it has pre-filled.
    pub fn set_text(&mut self, text: &str) {
        self.text = text.replace(['\n', '\r'], " ");
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Empties the field.
    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
    }

    /// Types `text` at the caret, replacing the selection.
    ///
    /// Line breaks are dropped rather than the text around them being refused: the way they arrive
    /// is a paste of several lines into a one-line box.
    pub fn insert(&mut self, text: &str) {
        let cleaned: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        let (from, to) = self.selection();
        self.text.replace_range(from..to, &cleaned);
        self.caret = from + cleaned.len();
        self.anchor = self.caret;
    }

    /// Removes the selection, or the character before the caret. Returns whether anything went.
    pub fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let Some(previous) = self.previous_boundary() else {
            return false;
        };
        self.text.replace_range(previous..self.caret, "");
        self.caret = previous;
        self.anchor = previous;
        true
    }

    /// Removes the selection, or the character after the caret. Returns whether anything went.
    pub fn delete(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let Some(next) = self.next_boundary() else {
            return false;
        };
        self.text.replace_range(self.caret..next, "");
        true
    }

    /// Moves the caret one character left, dragging the selection when `extend`.
    ///
    /// With a selection and no `extend` this collapses to its left end without moving further, which
    /// is what a left arrow does in every text field and is not the same as moving from the caret.
    pub fn left(&mut self, extend: bool) {
        if !extend && self.caret != self.anchor {
            self.caret = self.selection().0;
            self.anchor = self.caret;
            return;
        }
        if let Some(previous) = self.previous_boundary() {
            self.caret = previous;
        }
        self.settle(extend);
    }

    /// Moves the caret one character right, dragging the selection when `extend`.
    pub fn right(&mut self, extend: bool) {
        if !extend && self.caret != self.anchor {
            self.caret = self.selection().1;
            self.anchor = self.caret;
            return;
        }
        if let Some(next) = self.next_boundary() {
            self.caret = next;
        }
        self.settle(extend);
    }

    /// Moves the caret to the start.
    pub fn home(&mut self, extend: bool) {
        self.caret = 0;
        self.settle(extend);
    }

    /// Moves the caret to the end.
    pub fn end(&mut self, extend: bool) {
        self.caret = self.text.len();
        self.settle(extend);
    }

    /// Selects everything, with the caret at the end.
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Does what one key press means. Returns whether the *text* changed, which is what tells the
    /// bar to search again — a caret that moved needs a redraw but not a rescan.
    pub fn apply(&mut self, press: &Press, shift: bool) -> bool {
        match press {
            Press::Text(text) => {
                self.insert(text);
                true
            }
            Press::Backspace => self.backspace(),
            Press::Delete => self.delete(),
            Press::Left => {
                self.left(shift);
                false
            }
            Press::Right => {
                self.right(shift);
                false
            }
            Press::Home => {
                self.home(shift);
                false
            }
            Press::End => {
                self.end(shift);
                false
            }
            Press::SelectAll => {
                self.select_all();
                false
            }
        }
    }

    /// Drops the selection unless the motion was extending one.
    fn settle(&mut self, extend: bool) {
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Removes the selection if there is one. Returns whether there was.
    fn delete_selection(&mut self) -> bool {
        let (from, to) = self.selection();
        if from == to {
            return false;
        }
        self.text.replace_range(from..to, "");
        self.caret = from;
        self.anchor = from;
        true
    }

    /// The character boundary before the caret, or nothing at the start.
    fn previous_boundary(&self) -> Option<usize> {
        self.text[..self.caret]
            .chars()
            .next_back()
            .map(|c| self.caret - c.len_utf8())
    }

    /// The character boundary after the caret, or nothing at the end.
    fn next_boundary(&self) -> Option<usize> {
        self.text[self.caret..]
            .chars()
            .next()
            .map(|c| self.caret + c.len_utf8())
    }
}
