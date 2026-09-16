//! What shape the mouse pointer takes, and nothing about how it is set (MDEDIT-176).
//!
//! The editor drew the arrow everywhere until this existed, including over the text, because nothing
//! in the workspace called `Window::set_cursor` at all. An I-beam over the document is the first
//! thing anybody expects of an editor, and MDEDIT-142's task box — the one thing in a document you
//! *press* — is the first place a hand belongs.
//!
//! **This crate's own vocabulary, not winit's `CursorIcon`.** Which shape a point deserves is decided
//! by [`crate::editor::Editor::pointer_at`], which is asked by headless tests through the grid stub
//! exactly as clicks are; the mapping to winit lives at the one line that talks to the window.

/// The shape the pointer takes over a point in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    /// Over a control, or anywhere with nothing to point at. Windows' convention for a button is the
    /// arrow, not the hand.
    Arrow,
    /// Over text that a caret can be placed in: the document, the find bar's fields, a dialog's.
    Text,
    /// Over something a click *presses*. A task's box is the only one (MDEDIT-142); a link is
    /// deliberately not, because nothing follows a link yet and a hand would promise that it did.
    Hand,
}
