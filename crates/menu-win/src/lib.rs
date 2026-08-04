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
//! It is **not** where the shell integration lives, though this file once said it would be: the
//! `DropTarget` verb went into `mdedit-shell-win` on MDEDIT-61, because COM registration and
//! `IDropTarget` have nothing to do with menus and the ticket carries the reason for a third
//! platform crate.
//!
//! # What it does not do
//!
//! It does not decide anything — including which palette to draw in. It is *told*, by
//! [`MenuBar::attach`] and [`MenuBar::set_appearance`], because the editor's palette may be pinned
//! against Windows' by config and only the editor knows that. The one thing this crate reads for
//! itself is [`apps_use_light_theme`], which answers what Windows is set to and decides nothing.
//!
//! The menu reports which item was chosen as a [`Command`], and the
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
    /// File → Reopen with Encoding → one of them, by its position in the list the app supplied.
    ///
    /// The index rather than the encoding, for the reason [`Command::OpenRecent`] gives and one
    /// more: this crate depends on nothing that knows what a document is, and an encoding is
    /// `mdedit-file`'s vocabulary.
    ReopenWith(usize),
    /// File → Save with Encoding → one of them, by the same index.
    SaveWith(usize),
    /// File → Line Endings → LF.
    ///
    /// Two variants rather than one carrying a line-ending type, because that type belongs to
    /// `mdedit-file` and this crate deliberately depends on nothing that knows what a document is.
    /// The editor maps them, as it maps every other command.
    ConvertToLf,
    /// File → Line Endings → CRLF.
    ConvertToCrLf,
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
    /// Edit → Replace.
    Replace,
    /// Edit → Go to Line.
    GoToLine,
    /// Tools → Validate Document.
    Validate,
    /// Tools → Format Document.
    Format,
    /// View → Plain Markup.
    PlainMarkup,
    /// View → File Pane.
    FilePane,
    /// View → Line Numbers.
    LineNumbers,
    /// View → Formatting Bar.
    Toolbar,
    /// View → Whitespace and Line Endings.
    Whitespace,
    /// View → Word Wrap.
    WordWrap,
    /// View → Appearance → one of the three (MDEDIT-78).
    ///
    /// One variant carrying the choice rather than three, because they are one radio group: the
    /// editor sets a value, it does not toggle three things that happen to be exclusive.
    SetAppearance(AppearanceChoice),
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

/// Which palette the user has asked for, as a menu sees it (MDEDIT-78).
///
/// Three items in one radio group, so a tri-state rather than bools: "follow Windows" is a choice
/// in its own right and not the absence of the other two. The editor owns what each *means* — this
/// crate knows only which item is ticked, which is the same boundary that keeps `Newline` and
/// encodings out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppearanceChoice {
    /// Whatever Windows is set to, and change with it.
    #[default]
    System,
    /// Dark, whatever Windows is set to.
    Dark,
    /// Light, whatever Windows is set to.
    Light,
}

#[cfg(windows)]
mod appearance;

#[cfg(windows)]
pub use appearance::apps_use_light_theme;

/// Whether Windows is drawing apps light, off Windows: it does not say, because there is no Windows
/// to ask. The Windows build reads the registry — see this crate's `appearance` module there, which
/// is deliberately not linked, since it does not exist in this build to link to.
#[cfg(not(windows))]
#[must_use]
pub fn apps_use_light_theme() -> Option<bool> {
    None
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

    /// Attaches to nothing, in no particular palette.
    pub fn attach(&self, _hwnd: isize, _dark: bool) {}

    /// Re-themes nothing, there being no menu to re-theme.
    pub fn set_appearance(&self, _dark: bool) {}

    /// Shows nothing, there being no popup to show.
    pub fn show_context_menu(&mut self, _hwnd: isize, _x: f64, _y: f64, _enabled: ContextState) {}

    /// Ticks nothing, there being no menu to tick.
    pub fn set_view(&self, _state: ViewState) {}

    /// Greys nothing, there being no item to grey.
    pub fn set_tools(&self, _state: ToolsState) {}

    /// Ticks nothing, there being no submenu to tick.
    pub fn set_endings(&self, _state: EndingsState) {}

    /// Lists nothing, there being no submenu to list it in.
    pub fn set_encodings(&mut self, _labels: &[&str]) {}

    /// Ticks nothing, there being no submenu to tick.
    pub fn set_encoding_state(&self, _state: EncodingState) {}

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

/// Puts the executable's own icon on a window.
///
/// # Why this is needed at all
///
/// Embedding an icon resource gives Explorer, the taskbar's pinned entry and Alt-Tab something to
/// draw for the *file*. It does **not** give a window one: a window takes its icon from its class or
/// from `WM_SETICON`, and winit registers its class with neither. MDEDIT-70 assumed the resource
/// would be enough and the window disagreed — `WM_GETICON` returned null for all three sizes and
/// `GCLP_HICON` was zero, so the title bar drew a system fallback while the executable itself had a
/// perfectly good icon inside it.
///
/// # Why here
///
/// Same reason as [`double_click_interval`]: `AGENTS.md` makes `unsafe` outside a platform crate a
/// review block, and one Win32 call is not a reason to mint a fourth. This is the same `hwnd`, at the
/// same moment, as [`MenuBar::attach`].
///
/// Nothing is loaded from disk and there is no second copy of the pixels — `LoadImageW` reads the
/// resource `build.rs` already embedded, at the size Windows asks for.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "the platform layer; see this function's docs for why it is here"
)]
pub fn set_window_icon(hwnd: isize) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, LoadImageW, SM_CXICON,
        SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
    };

    // The id `mdedit.rc` gives the icon group. `MAKEINTRESOURCEW` is not a call — an ordinal is
    // passed where a name pointer goes, and the low word being the id is the whole convention.
    //
    // `without_provenance` rather than a cast, which clippy reads as a dangling pointer and is
    // right to: this is an integer wearing a pointer's type, and Windows never dereferences it.
    let name: *const u16 = std::ptr::without_provenance(1);

    // SAFETY: a null module handle means this executable, which is always loaded while it is
    // running, so the handle is valid and the call cannot fail in a way that matters.
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };

    for (which, cx, cy) in [
        (ICON_BIG, SM_CXICON, SM_CYICON),
        (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
    ] {
        // SAFETY: `GetSystemMetrics` takes an index and returns a number; it reads no memory of
        // ours. `LoadImageW` is asked for an icon from this module by ordinal, at exactly the
        // system metric size — which is the documented precondition for `LR_SHARED`, and what makes
        // the returned handle one Windows owns and destroys rather than one we must free. A missing
        // resource returns null, which is checked.
        let icon = unsafe {
            let (width, height) = (GetSystemMetrics(cx), GetSystemMetrics(cy));
            LoadImageW(module, name, IMAGE_ICON, width, height, LR_SHARED)
        };
        if icon.is_null() {
            continue;
        }

        // SAFETY: `hwnd` comes from winit's `RawWindowHandle` for a window this process created and
        // still holds. `WM_SETICON` takes the size in the wparam and the icon in the lparam, and
        // returns the previous icon — which is null here and deliberately dropped, because a window
        // that had no icon has nothing to restore.
        unsafe {
            SendMessageW(hwnd as _, WM_SETICON, which as usize, icon as isize);
        }
    }
}

/// Off Windows there is no window class to put an icon on.
#[cfg(not(windows))]
pub fn set_window_icon(_hwnd: isize) {}

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
    /// Whether spaces, tabs and line endings are drawn.
    pub whitespace: bool,
    /// Whether long lines wrap rather than running off to the right.
    pub word_wrap: bool,
    /// Which of the three Appearance items is ticked (MDEDIT-78).
    ///
    /// Not a bool, and so not covered by the `struct_excessive_bools` note above: it is a radio
    /// group, and exactly one of its items is ticked at a time.
    pub appearance: AppearanceChoice,
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

/// Which encoding the document being shown uses, and whether the question applies at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EncodingState {
    /// Whether the submenus apply — false for an untitled buffer, which has nothing on disk to
    /// reopen and settles its encoding at Save As.
    pub enabled: bool,
    /// Which entry to tick, by position in the list the app supplied. `None` ticks nothing, which is
    /// the honest answer for a file in an encoding the menu does not offer.
    pub current: Option<usize>,
}

/// Which line ending the document being shown uses.
///
/// Two bools rather than a `Newline`, because the menu's question is "which item is ticked" and the
/// answer for a Mixed document — or one made entirely of lone carriage returns — is **neither**.
/// A single value would have to invent a tick for a state that has no item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EndingsState {
    /// Whether the document is entirely `\n`.
    pub lf: bool,
    /// Whether the document is entirely `\r\n`.
    pub crlf: bool,
}

/// Which Tools entries apply to the document being shown.
///
/// Asked of the editor for the same reason [`ContextState`] is: whether a language reformats is a
/// fact about the document, and this crate deliberately knows nothing about documents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolsState {
    /// Whether Format Document would do anything — false for Markdown and YAML, which do not
    /// reformat, and greyed rather than left to fail when pressed.
    pub can_format: bool,
}
