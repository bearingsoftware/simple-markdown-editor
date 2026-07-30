//! Asking the user about work they are about to lose.
//!
//! Behind a trait, and **the real dialog lives in the binary rather than here** — the same
//! arrangement as the text backend, where only `main.rs` picks a platform implementation.
//!
//! Two reasons. The first is the same as [`crate::clipboard`]'s: closing tabs has consequences worth
//! asserting, and a native dialog cannot be driven by a test.
//!
//! The second is harder-edged, and is why the implementation cannot merely be *behind* the trait but
//! has to be outside this crate's library altogether. `rfd`'s Windows dialog imports from the
//! `ComCtl32` v6 side-by-side assembly, which the editor gets from the manifest `build.rs` embeds into
//! the executable. **A test binary has no manifest.** So a test that so much as links the dialog dies
//! at load with `STATUS_ENTRYPOINT_NOT_FOUND`, before a single assertion runs — and a trait does not
//! help, because the library still references `rfd` and the test still links the library. Moving the
//! implementation into `main.rs` is what actually keeps it out.

/// What to do about a tab with unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsaved {
    /// Write it, then close.
    Save,
    /// Close without writing.
    Discard,
    /// Keep the tab.
    Cancel,
}

/// Something that can ask about unsaved work and get an answer.
pub trait UnsavedPrompt {
    /// Asks about the tab called `name`.
    fn ask(&mut self, name: &str) -> Unsaved;
}

/// What an editor with nobody to ask does: nothing.
///
/// The default, and the answer whenever there is no dialog to put the question to. Cancel rather
/// than Discard because losing work is the one outcome that must never happen by default — an
/// editor that cannot ask keeps the tab.
#[derive(Debug, Default)]
pub struct KeepEverything;

impl UnsavedPrompt for KeepEverything {
    fn ask(&mut self, _name: &str) -> Unsaved {
        Unsaved::Cancel
    }
}
