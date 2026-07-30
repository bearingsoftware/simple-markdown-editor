//! The system clipboard, behind a trait.
//!
//! Not for portability — the editor is Windows-only for now. For **testability**: copy, cut and
//! paste are editing operations with undo consequences, and asserting them against a real clipboard
//! would mean a test that depends on whatever else the machine has copied, fails on a headless CI
//! runner, and races any other test doing the same thing. The same argument as
//! [`mdedit_text::TextLayout`], for the same reason.
//!
//! Text only. An image on the clipboard is not something a Markdown buffer can hold, and pretending
//! otherwise would mean deciding where to write the file — a feature, not a paste.

/// Somewhere text can be put and taken from.
pub trait Clipboard {
    /// The text on the clipboard, or `None` if there is none or it is not text.
    ///
    /// Never an error: a clipboard that cannot be read is indistinguishable, from the editor's point
    /// of view, from a clipboard with nothing on it, and neither is worth interrupting a keystroke
    /// over.
    fn read(&mut self) -> Option<String>;

    /// Puts `text` on the clipboard. Failure is silent, for the same reason.
    fn write(&mut self, text: &str);
}

/// The real one.
///
/// Constructed lazily, per operation, rather than held open: `arboard` keeps a handle to the
/// platform clipboard, and holding one for the life of the process is how an editor ends up blocking
/// another application's copy. It is cheap to open.
#[derive(Debug, Default)]
pub struct System;

impl Clipboard for System {
    fn read(&mut self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    fn write(&mut self, text: &str) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            // Nothing to do about a failure here that the user would thank us for. The text is still
            // in the buffer; the clipboard simply did not take it.
            let _ = clipboard.set_text(text);
        }
    }
}
