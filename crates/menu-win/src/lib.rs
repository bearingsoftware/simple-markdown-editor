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
    /// Edit → Select All. Only in the context menu today — the Edit menu has no item for it, which
    /// is its own small gap.
    SelectAll,
    /// Edit → Paste.
    Paste,
    /// Edit → Find.
    Find,
    /// Edit → Find Next.
    FindNext,
    /// Edit → Find Previous.
    FindPrevious,
    /// View → Plain Markup.
    PlainMarkup,
    /// View → File Pane.
    FilePane,
    /// View → Line Numbers.
    LineNumbers,
    /// View → Formatting Bar.
    Toolbar,
    /// View → Zoom In.
    ZoomIn,
    /// View → Zoom Out.
    ZoomOut,
    /// View → Reset Zoom.
    ResetZoom,
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

    /// Shows nothing, there being no popup to show.
    pub fn show_context_menu(&mut self, _hwnd: isize, _x: f64, _y: f64, _enabled: ContextState) {}

    /// Ticks nothing, there being no menu to tick.
    pub fn set_view(&self, _state: ViewState) {}

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

/// How long Windows says a double-click may take.
///
/// # Why this is asked rather than assumed
///
/// It is a system setting, reachable from Ease of Access, and somebody who has lengthened it has
/// done so because the default does not work for them. A hardcoded 500 ms in the editor quietly
/// overrides that — the kind of accessibility failure that is invisible to everyone it does not
/// affect. See `mdedit_app::clicks`, which takes this as a parameter for exactly that reason.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "the platform layer; see this function's docs for why it is here"
)]
#[must_use]
pub fn double_click_interval() -> std::time::Duration {
    // SAFETY: `GetDoubleClickTime` takes no arguments, touches no memory, and cannot fail — it
    // returns the current setting in milliseconds. It is in this crate rather than in `mdedit-app`
    // because `AGENTS.md` makes `unsafe` outside a platform crate a review block, and one Win32 call
    // is not a reason to mint a crate.
    let milliseconds =
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
    std::time::Duration::from_millis(u64::from(milliseconds))
}

/// Off Windows there is no such setting, so the conventional default stands.
#[cfg(not(windows))]
#[must_use]
pub fn double_click_interval() -> std::time::Duration {
    std::time::Duration::from_millis(500)
}

/// Which of the View menu's toggles are currently on.
///
/// The menu shows these as ticks, which is the whole reason they are in a menu rather than merely
/// being chords: a chord can toggle something but cannot tell you which way it is now.
///
/// It follows that the ticks have to be updated when the *chord* is used, not only when the item is
/// clicked — a menu that only knew about its own clicks would be wrong the moment anybody pressed
/// Ctrl+/.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field per check item, and a check item is a bool by definition. The state \
              machine the lint suggests would be a product of four independent toggles, which is \
              this struct with a worse name."
)]
pub struct ViewState {
    /// Whether the Markdown is shown as plain text rather than concealed.
    pub plain_markup: bool,
    /// Whether the open files are shown as a pane rather than a strip.
    pub file_pane: bool,
    /// Whether source-line numbers are shown.
    pub line_numbers: bool,
    /// Whether the formatting bar is showing.
    pub toolbar: bool,
}

/// Which context-menu entries are available.
///
/// Asked of the editor rather than worked out here: whether there is a selection is a fact about the
/// document, and this crate deliberately knows nothing about documents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextState {
    /// Whether anything is selected — Cut and Copy are greyed when nothing is.
    pub has_selection: bool,
    /// Whether the clipboard holds text worth pasting.
    pub can_paste: bool,
}
