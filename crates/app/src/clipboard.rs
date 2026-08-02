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
//!
//! The default is [`HoldNothing`] and `main` installs [`System`] over it. That is not a preference:
//! reading the real clipboard from more than one thread of a process corrupts the heap, and while
//! [`System`] was the default every test binary did exactly that. See [`System`] for the mechanism.

use std::marker::PhantomData;

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

/// A clipboard with nothing on it, which takes whatever it is given and forgets it.
///
/// The default an [`Editor`](crate::editor::Editor) is built with, so an editor constructed by a
/// test touches nothing of the user's — the same rule the recent-files list and the dialogs already
/// follow, and `main` installs [`System`] over it exactly as it installs the real prompts. That was
/// not always the default, and MDEDIT-67 is what it cost: see [`System`].
#[derive(Debug, Default)]
pub struct HoldNothing;

impl Clipboard for HoldNothing {
    fn read(&mut self) -> Option<String> {
        None
    }

    fn write(&mut self, _text: &str) {}
}

/// The real one.
///
/// Constructed lazily, per operation, rather than held open: `arboard` keeps a handle to the
/// platform clipboard, and holding one for the life of the process is how an editor ends up blocking
/// another application's copy. It is cheap to open.
///
/// # It is deliberately `!Send`, and that is a memory-safety rule rather than a style choice
///
/// MDEDIT-67: reading the Windows clipboard from two threads of one process corrupts the heap.
/// `arboard`'s read path sizes a conversion buffer from a clipboard handle and then converts into
/// it, and the handle is only valid while *this process* has the clipboard open — but `CloseClipboard`
/// is process-wide, so any other thread finishing its own clipboard operation invalidates the handle
/// a reader is part way through using. Measured: one thread reading 2,000 times is clean; two threads
/// reading corrupt the heap 8 runs in 8; one reader against eight writers corrupts 6 in 8.
///
/// Nothing here can fix that, so the type refuses to leave the thread it was made on and the compiler
/// enforces it. The editor is a synchronous frame loop on the UI thread, which is where every
/// clipboard operation belongs anyway.
#[derive(Debug, Default)]
pub struct System(PhantomData<*const ()>);

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
