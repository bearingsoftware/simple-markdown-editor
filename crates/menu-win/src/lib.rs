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
    /// File → Open.
    Open,
    /// File → Open Recent → one of the entries, by its position in the list.
    ///
    /// The index rather than the path, because the menu is rebuilt from the list and the two would
    /// otherwise have to be kept in step by hand. The app reads it back against the same list.
    OpenRecent(usize),
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
    /// Help → Keyboard Shortcuts.
    Shortcuts,
    /// Help → About.
    About,
}

#[cfg(windows)]
mod dark;

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

    /// Lists nothing, there being no submenu to list it in.
    pub fn set_recent(&mut self, _labels: &[String]) {}

    /// Forwards nothing, because nothing can be chosen.
    pub fn forward_to(&self, _deliver: impl Fn(Command) + Send + Sync + 'static) {}
}

/// Reattaches a windows-subsystem process to the console that launched it, if there was one.
///
/// # Why this lives in this crate
///
/// It is a raw Win32 call, and `AGENTS.md` makes `unsafe` outside a platform crate a review block.
/// This crate is the platform layer for the shell-adjacent parts of the window — the menu bar today,
/// the `DropTarget` verb eventually — and console attachment is the same kind of thing. When
/// MDEDIT-61 adds `mdedit-shell-win` this should move there; the name is imperfect and the rule is
/// not, so it lives here rather than in `mdedit-app`, and rather than minting a crate for one call.
///
/// # Why it is needed at all
///
/// The binary is windows-subsystem, so double-clicking it from Explorer no longer flashes a console.
/// The cost is that a windows-subsystem process gets no standard handles even when it *was* started
/// from a terminal — so `stderr` would go nowhere and the latency harness's `launch -> first paint`
/// line, which `AGENTS.md` says is measured on every run, would vanish. Attaching to the parent's
/// console when there is one gives that back and does nothing when there is not.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "the platform layer; see this function's docs for why it is here"
)]
pub fn attach_parent_console() {
    // SAFETY: `AttachConsole` takes a process id and attaches this process to that console if it has
    // one. `ATTACH_PARENT_PROCESS` is its documented sentinel for "my parent". It fails harmlessly
    // when the parent has no console — which is the Explorer case — and the failure is ignored
    // because having no console is not an error here.
    unsafe {
        let _ = windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

/// Off Windows there is no console to reattach to.
#[cfg(not(windows))]
pub fn attach_parent_console() {}
