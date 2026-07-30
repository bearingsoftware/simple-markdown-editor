//! The native Windows menu bar.
//!
//! # Why this is a crate of its own
//!
//! Attaching a menu bar to a window is a raw Win32 call: `muda`'s `Menu::init_for_hwnd` is `unsafe`.
//! `AGENTS.md` makes `unsafe` outside the platform layer a review block rather than a discussion, so
//! it cannot live in `mdedit-app` — this crate is the platform layer for menus, as
//! `mdedit-text-dwrite` is for text. One `unsafe` block, with its `SAFETY` note, and a safe API over
//! it.
//!
//! It is also where the Windows shell integration `AGENTS.md` anticipates — the `DropTarget` verb, so
//! opening fifteen files from Explorer reuses one process — will belong.
//!
//! # What it does not do
//!
//! It does not decide anything. The menu reports which item was chosen as a [`Command`], and the
//! editor decides what that means. That keeps every command testable without a menu, which matters
//! because a native menu cannot be driven by a test at all.
//!
//! # Accelerators are labels here, not bindings
//!
//! The items carry accelerator text so the menu reads correctly — "Ctrl+S" beside Save. They do not
//! *fire*: muda's own documentation is explicit that on Windows accelerators need the message loop to
//! call `TranslateAcceleratorW`, and winit's loop does not. That is deliberate and load-bearing. The
//! editor's key handler already owns those chords, and two things acting on one Ctrl+Z would undo
//! twice.

#![cfg_attr(
    not(windows),
    allow(unused_imports, reason = "the whole crate is Windows-only")
)]

/// Something the user asked for from the menu.
///
/// Deliberately not "the item that was clicked": the editor should not care which submenu something
/// came from, and a command can arrive from a chord instead.
///
/// Deliberately **not** `#[non_exhaustive]` either, though it is a cross-crate enum. These two crates
/// ship together, and a new command that compiles but does nothing is worse than one that fails the
/// build until somebody says what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// File → New.
    New,
    /// File → Save.
    Save,
    /// File → Save As.
    SaveAs,
    /// File → Save All.
    SaveAll,
    /// File → Close.
    Close,
    /// File → Close All.
    CloseAll,
    /// Edit → Undo.
    Undo,
    /// Edit → Redo.
    Redo,
    /// Edit → Cut.
    Cut,
    /// Edit → Copy.
    Copy,
    /// Edit → Paste.
    Paste,
}

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::MenuBar;

/// The menu bar, off Windows: a type that exists so the app compiles, and does nothing.
///
/// The editor does not run off Windows at all — there is no text backend — so this is a placeholder
/// rather than a degraded experience. It is here so `mdedit-app` needs no `cfg` of its own.
#[cfg(not(windows))]
#[derive(Debug, Default)]
pub struct MenuBar;

#[cfg(not(windows))]
impl MenuBar {
    /// Builds nothing.
    #[must_use]
    pub fn new() -> Option<Self> {
        None
    }

    /// Attaches to nothing.
    pub fn attach(&self, _hwnd: isize) {}

    /// Forwards nothing, because nothing can be chosen.
    pub fn forward_to(&self, _deliver: impl Fn(Command) + Send + Sync + 'static) {}
}
