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

/// What to do when the file on disk has changed under a buffer that also has edits.
///
/// There is deliberately no "merge". This is an editor, not a version control system, and the two
/// versions are a buffer and a file rather than two commits with a common ancestor. Offering a merge
/// would mean writing one, badly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Take what is on disk, discarding the edits in the buffer.
    Reload,
    /// Write the buffer over the newer file. Only ever offered when saving — a check that was not
    /// going to write anything has nothing to overwrite.
    Overwrite,
    /// Change nothing: keep the buffer as it is and leave the file as it is.
    Cancel,
}

/// Why the question is being asked, which is what decides the wording and the choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflicting {
    /// Noticed on returning to the window. The buffer has edits and so does the file; nothing is
    /// being written yet. [`Conflict::Overwrite`] is not among the answers.
    Found,
    /// Noticed at the moment of saving. This is the one that prevents data loss:
    /// [`Conflict::Overwrite`] is a choice the user makes rather than one saving makes for them.
    Saving,
}

/// Something that can ask what to do about a file that changed underneath.
pub trait ConflictPrompt {
    /// Asks about the file called `name`.
    fn ask(&mut self, name: &str, why: Conflicting) -> Conflict;
}

/// What an editor with nobody to ask does about a conflict: nothing.
///
/// [`Conflict::Cancel`] in both cases, and it is the safe answer in both for different reasons. On
/// [`Conflicting::Found`] it keeps the buffer, so no edits are lost. On [`Conflicting::Saving`] it
/// abandons the save, so the other tool's write is not overwritten — the buffer stays in memory,
/// where crash recovery is now holding a copy of it anyway.
#[derive(Debug, Default)]
pub struct ChangeNothing;

impl ConflictPrompt for ChangeNothing {
    fn ask(&mut self, _name: &str, _why: Conflicting) -> Conflict {
        Conflict::Cancel
    }
}
