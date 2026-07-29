//! What the input method is composing, before it becomes text.
//!
//! # Why the pre-edit is not in the buffer
//!
//! Composing `nihongo` into 日本語 takes seven keystrokes and produces one word. If each of those
//! keystrokes edited the buffer, the document would take seven edits for one insertion, undo would
//! have seven steps to walk back through, and cancelling the composition would need an eighth edit
//! to remove what the other seven put there — which is exactly how an editor ends up splitting a
//! composition into fragments (step 6 of the script in `spikes/font-fallback/README.md`) or losing a
//! character when focus moves away mid-composition (step 4, the defect open against Slint as
//! [slint#10861](https://github.com/slint-ui/slint/issues/10861)).
//!
//! So the pre-edit lives here, beside the document rather than inside it. The buffer changes exactly
//! once, when the input method commits, and cancelling costs no edit at all because there was never
//! anything to undo.
//!
//! No platform types: this is the state machine, and it is testable without a window, a Japanese
//! keyboard or a human — which is most of what can be tested about an input method at all.

/// The text the input method is composing, if it is composing any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composition {
    text: String,
    /// The input method's own cursor within [`text`](Self::text), as a byte range. `None` means it
    /// wants no cursor shown.
    cursor: Option<(usize, usize)>,
}

/// What the editor should do about an input-method event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing changed; do not spend a frame on it.
    Nothing,
    /// The pre-edit changed and has to be redrawn.
    Redraw,
    /// The input method committed. Insert this into the buffer, as one edit.
    Insert(String),
}

impl Composition {
    /// Nothing being composed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an input method is mid-composition.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.text.is_empty()
    }

    /// The text being composed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the input method wants its own cursor, as a byte range into [`text`](Self::text).
    ///
    /// Clamped to the text and to a character boundary, because it arrives from the platform and a
    /// stale or malformed range must not be able to panic a render.
    #[must_use]
    pub fn cursor(&self) -> Option<(usize, usize)> {
        let (start, end) = self.cursor?;
        let clamp = |at: usize| {
            let mut at = at.min(self.text.len());
            while at > 0 && !self.text.is_char_boundary(at) {
                at -= 1;
            }
            at
        };
        let (start, end) = (clamp(start), clamp(end));
        Some((start.min(end), end))
    }

    /// The input method replaced what it is composing.
    ///
    /// An empty string means it cleared the composition, which winit also sends immediately before
    /// every commit.
    pub fn preedit(&mut self, text: String, cursor: Option<(usize, usize)>) -> Action {
        if self.text == text && self.cursor == cursor {
            return Action::Nothing;
        }
        self.text = text;
        self.cursor = cursor;
        Action::Redraw
    }

    /// The input method committed. The composition ends and the text becomes an edit.
    ///
    /// One edit, whatever the composition cost to build: that is what makes a committed word one
    /// step of undo rather than seven.
    pub fn commit(&mut self, text: String) -> Action {
        let was_active = self.is_active();
        self.clear();
        if text.is_empty() {
            // Nothing to insert, but the pre-edit may still have been on screen.
            return if was_active {
                Action::Redraw
            } else {
                Action::Nothing
            };
        }
        Action::Insert(text)
    }

    /// The input method was switched off — focus moved away, or the user pressed Escape.
    ///
    /// The buffer is untouched, because the pre-edit was never in it. Anything the input method
    /// meant to keep it has already sent as a commit.
    pub fn cancel(&mut self) -> Action {
        if !self.is_active() {
            self.clear();
            return Action::Nothing;
        }
        self.clear();
        Action::Redraw
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = None;
    }
}
