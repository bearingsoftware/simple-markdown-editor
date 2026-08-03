//! The Win32 half: building the menu, attaching it, and reading what was chosen.

// The only crate other than the text backend permitted to lower this, and for one call. See the
// crate docs.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, MenuTheme, Submenu};

use crate::Command;

/// A menu bar, and which item means what.
pub struct MenuBar {
    menu: Menu,
    /// muda identifies items by an opaque id, so the mapping back to a command is kept here rather
    /// than inferred from the label — a label is what the user reads, not an identifier.
    commands: HashMap<MenuId, Command>,
    /// File → Open Recent, kept so its contents can be replaced as files are opened.
    recent: Submenu,
    /// The ids currently in `recent`, and which position each stands for.
    ///
    /// Separate from `commands` because it is thrown away and rebuilt every time the list changes,
    /// and muda hands out fresh ids each time. Shared rather than owned because [`Self::forward_to`]
    /// registers a handler that outlives the call and has to see rebuilds that happen after it —
    /// a clone taken at registration would answer for a menu that no longer exists.
    recent_ids: Arc<Mutex<HashMap<MenuId, usize>>>,
    /// The ids in the context menu currently being shown, and what each means. Shared and rebuilt
    /// for the same reasons `recent_ids` is — see `show_context_menu`.
    context_ids: Arc<Mutex<HashMap<MenuId, Command>>>,
    /// The View menu's three toggles, kept so their ticks can be made to match the editor. See
    /// [`Self::set_view`].
    view_toggles: [CheckMenuItem; 6],
    /// Tools → Format Document, kept for the same reason: its enabled state has to follow whichever
    /// document is showing. See [`Self::set_tools`].
    format_item: MenuItem,
    /// File → Line Endings, kept so its ticks can follow the document. See [`Self::set_endings`].
    ending_items: [CheckMenuItem; 2],
    /// The two encoding submenus, and the items in them. Filled by [`Self::set_encodings`], which
    /// the app calls once with the list it owns — this crate deliberately does not know what an
    /// encoding is, only that there are some and they have names.
    reopen_with: Submenu,
    save_with: Submenu,
    encoding_items: Vec<[CheckMenuItem; 2]>,
    /// The encoding items' ids, **shared rather than cloned**, for the reason `recent_ids` gives:
    /// these are minted by `set_encodings` after `forward_to` has taken its snapshot of `commands`,
    /// so an id put in that map would never be seen. Found by clicking one and watching nothing
    /// happen — no test can press a native menu item.
    encoding_ids: Arc<Mutex<HashMap<MenuId, Command>>>,
}

impl std::fmt::Debug for MenuBar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // muda's `Menu` is a handle to a native object with no useful representation, and requiring
        // `Debug` of it is not worth a wrapper.
        f.debug_struct("MenuBar")
            .field("items", &self.commands.len())
            .finish_non_exhaustive()
    }
}

impl MenuBar {
    /// Builds the File and Edit menus.
    ///
    /// Returns `None` if the platform refuses, which is not worth failing to start over: an editor
    /// with no menu bar still edits, and every command it would have offered has a chord.
    #[must_use]
    pub fn new() -> Option<Self> {
        let mut commands = HashMap::new();
        let menu = Menu::new();

        let file = Submenu::new("&File", true);
        let edit = Submenu::new("&Edit", true);
        let view = Submenu::new("&View", true);
        let tools = Submenu::new("&Tools", true);
        let help = Submenu::new("&Help", true);

        add_items(
            &Menus {
                file: &file,
                edit: &edit,
                view: &view,
                tools: &tools,
                help: &help,
            },
            &mut commands,
        )?;

        let view_toggles = add_view_toggles(&view, &mut commands)?;

        // Built here rather than in the table because it is the one item whose enabled state
        // changes, so something has to keep hold of it. Disabled to begin with: the first frame
        // turns it on if the document showing is one that formats, and an item that starts enabled
        // would be wrong for exactly as long as it takes to draw that frame.
        let format_item = MenuItem::new("&Format Document", false, None);
        commands.insert(format_item.id().clone(), Command::Format);
        tools.insert(&format_item, 0).ok()?;

        // File → Line Endings. A submenu beside the encoding ones eventually, because which endings
        // a file has is a property of the file in the same way its encoding is. Check items rather
        // than plain ones: the point is as much to say what the document *is* as to change it, and a
        // Mixed document ticks neither.
        let endings = Submenu::new("Line &Endings", true);
        let lf = CheckMenuItem::new("&LF", true, false, None);
        let crlf = CheckMenuItem::new("&CRLF", true, false, None);
        commands.insert(lf.id().clone(), Command::ConvertToLf);
        commands.insert(crlf.id().clone(), Command::ConvertToCrLf);
        endings.append(&lf).ok()?;
        endings.append(&crlf).ok()?;
        let ending_items = [lf, crlf];

        // Empty and disabled until the app fills it in — see `set_recent`. Placed after Open, where
        // every editor puts it.
        let recent = Submenu::new("Open &Recent", false);
        file.insert(&recent, 2).ok()?;

        // A separator between saving and closing, because they are different kinds of decision.
        // Index 6 rather than 4: Open and Open Recent went in above it.
        file.insert(&muda::PredefinedMenuItem::separator(), 6)
            .ok()?;

        // **After the separator exists**, so this lands between Save All and it. Inserting earlier
        // panics: the File menu is only seven items long until Open Recent and the separator are in,
        // and muda's insert is an index into a `Vec`. That is a startup crash, and no test can see
        // it — a menu bar cannot be built without a window, which is why `AGENTS.md` says menus are
        // checked by eye.
        file.insert(&endings, 6).ok()?;

        // Empty and disabled until the app fills them in — see `set_encodings`, and `set_recent`,
        // which does the same dance for the same reason. Above Line Endings, because which encoding
        // a file is in is the larger fact about it.
        let reopen_with = Submenu::new("&Reopen with Encoding", false);
        let save_with = Submenu::new("Sa&ve with Encoding", false);
        file.insert(&reopen_with, 6).ok()?;
        file.insert(&save_with, 7).ok()?;

        menu.append(&file).ok()?;
        menu.append(&edit).ok()?;
        menu.append(&view).ok()?;
        menu.append(&tools).ok()?;
        menu.append(&help).ok()?;
        Some(Self {
            menu,
            commands,
            recent,
            recent_ids: Arc::new(Mutex::new(HashMap::new())),
            context_ids: Arc::new(Mutex::new(HashMap::new())),
            view_toggles,
            format_item,
            ending_items,
            reopen_with,
            save_with,
            encoding_items: Vec::new(),
            encoding_ids: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Fills both encoding submenus with `labels`, in order.
    ///
    /// Called once, by the app, with the list it owns. This crate does not know what an encoding is
    /// — only that there are some, that they have names, and that a click should report **which one
    /// by position**, which is the same contract [`Self::set_recent`] uses and the reason neither
    /// submenu needs a type from `mdedit-file`.
    ///
    /// Check items in both, because the tick is as much about saying what the file *is* as about
    /// choosing.
    pub fn set_encodings(&mut self, labels: &[&str]) {
        self.encoding_items.clear();
        // Poisoning would mean a panic while the menu was being built, which has nothing to hold
        // half-written: the map is filled from empty either way.
        let mut ids = match self.encoding_ids.lock() {
            Ok(ids) => ids,
            Err(poisoned) => poisoned.into_inner(),
        };
        ids.clear();

        for (index, label) in labels.iter().enumerate() {
            let reopen = CheckMenuItem::new(*label, true, false, None);
            let save = CheckMenuItem::new(*label, true, false, None);
            ids.insert(reopen.id().clone(), Command::ReopenWith(index));
            ids.insert(save.id().clone(), Command::SaveWith(index));
            if self.reopen_with.append(&reopen).is_err() || self.save_with.append(&save).is_err() {
                break;
            }
            self.encoding_items.push([reopen, save]);
        }
    }

    /// Ticks the encoding the document is in, and greys the submenus when there is no document to
    /// ask about.
    pub fn set_encoding_state(&self, state: crate::EncodingState) {
        self.reopen_with.set_enabled(state.enabled);
        self.save_with.set_enabled(state.enabled);
        for (index, [reopen, save]) in self.encoding_items.iter().enumerate() {
            let ticked = state.enabled && state.current == Some(index);
            reopen.set_checked(ticked);
            save.set_checked(ticked);
        }
    }

    /// Ticks whichever line ending the document being shown uses.
    ///
    /// **Neither, for a document that is Mixed** — or one made entirely of lone carriage returns.
    /// Ticking one anyway would be the menu claiming a file is something it is not, which is exactly
    /// what the status bar's "Mixed" exists to avoid.
    pub fn set_endings(&self, state: crate::EndingsState) {
        let [lf, crlf] = &self.ending_items;
        lf.set_checked(state.lf);
        crlf.set_checked(state.crlf);
    }

    /// Greys the Tools items that do not apply to the document being shown.
    ///
    /// Called once per frame rather than from each of the many paths that change which document is
    /// active. The `counted_for` lesson, in push form: a list of call sites is a list somebody
    /// eventually forgets to add to, and the caller compares before it calls so this costs a Win32
    /// call only when the answer actually changes.
    pub fn set_tools(&self, state: crate::ToolsState) {
        self.format_item.set_enabled(state.can_format);
    }

    /// Makes the View menu's ticks match the editor.
    ///
    /// Called after anything that could have changed one — including a *chord*, which is the case a
    /// menu that only watched its own clicks would get wrong. Cheap enough to call every time rather
    /// than working out whether it was needed.
    pub fn set_view(&self, state: crate::ViewState) {
        let [
            plain_markup,
            file_pane,
            line_numbers,
            toolbar,
            whitespace,
            word_wrap,
        ] = &self.view_toggles;
        plain_markup.set_checked(state.plain_markup);
        file_pane.set_checked(state.file_pane);
        line_numbers.set_checked(state.line_numbers);
        toolbar.set_checked(state.toolbar);
        whitespace.set_checked(state.whitespace);
        word_wrap.set_checked(state.word_wrap);
    }

    /// Replaces the Open Recent submenu with `labels`, most recent first.
    ///
    /// Rebuilt rather than updated in place: muda has no way to shorten a submenu except by removing
    /// items, so keeping N items and hiding the unused ones would be the same work with a worse
    /// failure mode. The submenu is disabled outright when the list is empty, which is what stops it
    /// being a menu that opens onto nothing.
    ///
    /// The index a click reports is the position in `labels`, so the caller must read it back
    /// against the same list it passed in.
    pub fn set_recent(&mut self, labels: &[String]) {
        while self.recent.remove_at(0).is_some() {}
        // Poisoning would mean a panic while a menu was being rebuilt, which has nothing to hold
        // half-written: the map is replaced wholesale below either way.
        let mut ids = match self.recent_ids.lock() {
            Ok(ids) => ids,
            Err(poisoned) => poisoned.into_inner(),
        };
        ids.clear();

        for (index, label) in labels.iter().enumerate() {
            // `&` opens a mnemonic in a Win32 menu, and these labels are file paths — a directory
            // called `R&D` would silently eat its own ampersand and put the underline in the wrong
            // place. Doubling is how Win32 spells a literal one.
            let escaped = label.replace('&', "&&");
            // The leading digit is the accelerator people actually use in a recent menu, and it is
            // positional, so it is prepended here rather than stored in the label.
            let item = MenuItem::new(format!("&{} {escaped}", index + 1), true, None);
            ids.insert(item.id().clone(), index);
            if self.recent.append(&item).is_err() {
                break;
            }
        }
        drop(ids);
        self.recent.set_enabled(!labels.is_empty());
    }

    /// Puts the menu bar on a window.
    ///
    /// `hwnd` must be a live window handle, which is why this is the one place in the editor outside
    /// the text backend that needs `unsafe`.
    ///
    /// Attached **dark**, to match the editor. Deliberately not `MenuTheme::Auto`: Auto follows
    /// Windows, and the editor does not — its theme is one hardcoded dark palette with no settings
    /// surface to choose another. On a light-themed Windows, Auto would put a light menu bar over a
    /// dark document, which is the bug this replaced (MDEDIT-32). When the editor learns to follow
    /// the system, the menu follows with it and this becomes a variable.
    ///
    /// `MenuTheme::Dark` covers the bar and nothing else, because that is all muda draws — the
    /// dropdowns are Windows' own popups, and [`crate::dark`] is what makes those dark (MDEDIT-35).
    /// It runs first: the theme is a property of the process, and setting it after a menu exists
    /// means asking Windows to re-theme one rather than to build it right.
    pub fn attach(&self, hwnd: isize) {
        crate::dark::use_dark_menus();

        // SAFETY: `hwnd` comes from winit's `RawWindowHandle` for a window this process created and
        // still holds, so it is a valid `HWND` for the duration of the call. It is only read, to set
        // the window's menu.
        let attached = unsafe { self.menu.init_for_hwnd_with_theme(hwnd, MenuTheme::Dark) };
        if let Err(error) = attached {
            // Nothing to do about it that the user would thank us for: the chords still work.
            eprintln!("could not attach the menu bar: {error}");
        }
    }

    /// Pops the document's context menu at `(x, y)`, in physical pixels from the window's corner.
    ///
    /// `enabled` says which entries are available — see [`crate::ContextState`]. Built fresh each time
    /// rather than kept and re-enabled, because the state changes with every selection and a menu
    /// that exists between shows is one more thing to keep in step with the document.
    ///
    /// Its items carry the same [`Command`]s the menu bar's do, so they arrive through
    /// [`Self::forward_to`] like any other: **a second door, never a second implementation.** That
    /// also means the ids have to be registered before the menu is shown, and unregistered after —
    /// muda's ids are unique per item, so a menu shown twice would otherwise leak an entry per show.
    pub fn show_context_menu(&mut self, hwnd: isize, x: f64, y: f64, enabled: crate::ContextState) {
        use muda::ContextMenu as _;

        let menu = Submenu::new("", true);
        let items = [
            ("Cu&t", Command::Cut, enabled.has_selection),
            ("&Copy", Command::Copy, enabled.has_selection),
            ("&Paste", Command::Paste, enabled.can_paste),
        ];

        // Shared with the event handler rather than owned, for the reason `recent_ids` documents:
        // `forward_to` takes a *clone* of `commands` when it registers, so an id minted after that
        // — which every context item is — would never resolve through it.
        let mut ids = match self.context_ids.lock() {
            Ok(ids) => ids,
            Err(poisoned) => poisoned.into_inner(),
        };
        // Cleared rather than added to: muda mints a fresh id per item, so a menu shown a hundred
        // times would otherwise leave four hundred dead entries behind.
        ids.clear();

        for (label, command, is_enabled) in items {
            let item = MenuItem::new(label, is_enabled, None);
            ids.insert(item.id().clone(), command);
            if menu.append(&item).is_err() {
                return;
            }
        }
        if menu.append(&muda::PredefinedMenuItem::separator()).is_err() {
            return;
        }
        let select_all = MenuItem::new("Select &All", true, None);
        ids.insert(select_all.id().clone(), Command::SelectAll);
        if menu.append(&select_all).is_err() {
            return;
        }
        // Released before the menu is shown: `show_context_menu_for_hwnd` runs a modal message loop,
        // and the event handler it dispatches into needs this same lock to resolve what was chosen.
        drop(ids);

        // The dropdowns are Windows' own popups, so this is what makes them dark — the same call
        // `attach` makes, for the same reason (MDEDIT-35). Cheap, and idempotent.
        crate::dark::use_dark_menus();

        // SAFETY: `hwnd` comes from winit's `RawWindowHandle` for a window this process created and
        // still holds, so it is a valid `HWND` for the duration of the call. `show_context_menu_for_hwnd`
        // runs its own modal message loop and returns when the menu closes — which is why the ids
        // are removed on the line after rather than needing to outlive anything.
        unsafe {
            let _ = menu.show_context_menu_for_hwnd(
                hwnd,
                Some(muda::dpi::PhysicalPosition::new(x, y).into()),
            );
        }
    }

    /// Sends every chosen command to `deliver`.
    ///
    /// Pushed rather than polled. muda delivers menu events on a channel of its own, and winit's
    /// loop is asleep until something wakes it — so polling would mean either waking the loop on a
    /// timer, which costs battery for nothing, or a menu that responds only when something else
    /// happens to have woken it. This is the arrangement muda's own documentation describes: forward
    /// the event into the loop that owns the editor.
    ///
    /// The closure runs on whichever thread muda raises the event on, so it must only hand the
    /// command over — anything that touches the document belongs on the far side.
    pub fn forward_to(&self, deliver: impl Fn(Command) + Send + Sync + 'static) {
        let commands = self.commands.clone();
        // Shared, not cloned: the recent submenu is rebuilt after this handler is registered, and a
        // snapshot taken here would keep answering for the menu as it was at startup.
        let recent_ids = Arc::clone(&self.recent_ids);
        let context_ids = Arc::clone(&self.context_ids);
        let encoding_ids = Arc::clone(&self.encoding_ids);
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = commands.get(&event.id).copied() {
                deliver(command);
                return;
            }
            let from_context = match context_ids.lock() {
                Ok(ids) => ids.get(&event.id).copied(),
                Err(poisoned) => poisoned.into_inner().get(&event.id).copied(),
            };
            if let Some(command) = from_context {
                deliver(command);
                return;
            }
            let from_encodings = match encoding_ids.lock() {
                Ok(ids) => ids.get(&event.id).copied(),
                Err(poisoned) => poisoned.into_inner().get(&event.id).copied(),
            };
            if let Some(command) = from_encodings {
                deliver(command);
                return;
            }
            let index = match recent_ids.lock() {
                Ok(ids) => ids.get(&event.id).copied(),
                Err(poisoned) => poisoned.into_inner().get(&event.id).copied(),
            };
            if let Some(index) = index {
                deliver(Command::OpenRecent(index));
            }
        }));
    }
}

/// Puts the four toggles at the top of the View menu, and says what each means.
///
/// **Check items, not plain ones**, and that is the reason those three are in a menu at all: a chord
/// can toggle something but cannot tell you which way it is now. [`MenuBar::set_view`] keeps the
/// ticks true — including when the *chord* was used, which is the case a menu that only knew about
/// its own clicks would get wrong.
///
/// Inserted at the top rather than appended, because the zoom items went in with the rest of the
/// menu bar and what the view *is* belongs above how big it is drawn.
fn add_view_toggles(
    view: &Submenu,
    commands: &mut HashMap<MenuId, Command>,
) -> Option<[CheckMenuItem; 6]> {
    let plain_markup = CheckMenuItem::new(
        "&Plain Markup",
        true,
        false,
        Some(Accelerator::new(Some(Modifiers::CONTROL), Code::Slash)),
    );
    let file_pane = CheckMenuItem::new(
        "&File Pane",
        true,
        false,
        Some(Accelerator::new(
            Some(Modifiers::CONTROL | Modifiers::SHIFT),
            Code::KeyE,
        )),
    );
    // No accelerator, because there is no chord: this one has never had one and does not need one.
    let line_numbers = CheckMenuItem::new("&Line Numbers", true, false, None);
    // Nor this one. A bar that is visible is a bar you can find in the menu; a chord for it would be
    // one more thing to remember for something switched on twice a year.
    let toolbar = CheckMenuItem::new("Formatting &Bar", true, false, None);
    // Nor this one, and for a third reason: it is a way of looking at a document for a moment, so
    // it is reached for deliberately rather than by muscle memory.
    let whitespace = CheckMenuItem::new("&Whitespace and Line Endings", true, false, None);
    // MDEDIT-49 is what let this item exist at all: until horizontal scrolling shipped, turning wrap
    // off would have truncated long lines invisibly, and a menu item that has never worked is a
    // broken promise. No chord — it is a way of looking at a document, reached for deliberately.
    let word_wrap = CheckMenuItem::new("&Word Wrap", true, true, None);

    for (index, (item, command)) in [
        (&plain_markup, Command::PlainMarkup),
        (&file_pane, Command::FilePane),
        (&line_numbers, Command::LineNumbers),
        (&toolbar, Command::Toolbar),
        (&whitespace, Command::Whitespace),
        (&word_wrap, Command::WordWrap),
    ]
    .into_iter()
    .enumerate()
    {
        commands.insert(item.id().clone(), command);
        view.insert(item, index).ok()?;
    }
    view.insert(&muda::PredefinedMenuItem::separator(), 6)
        .ok()?;

    Some([
        plain_markup,
        file_pane,
        line_numbers,
        toolbar,
        whitespace,
        word_wrap,
    ])
}

/// Puts every ordinary item in its submenu, and says what each means.
///
/// One table rather than four builders: the shape is identical everywhere and a list reads better
/// than a sequence of calls. The accelerators are **labels only** — see the crate docs for why that
/// is deliberate and must stay that way.
struct Menus<'a> {
    file: &'a Submenu,
    edit: &'a Submenu,
    view: &'a Submenu,
    tools: &'a Submenu,
    help: &'a Submenu,
}

fn add_items(menus: &Menus<'_>, commands: &mut HashMap<MenuId, Command>) -> Option<()> {
    let items = file_items(menus.file)
        .into_iter()
        .chain(edit_items(menus.edit))
        .chain(view_and_help_items(menus.view, menus.tools, menus.help));
    for (menu, label, command, chord) in items {
        let accelerator = chord.map(|(modifiers, code)| Accelerator::new(Some(modifiers), code));
        let item = MenuItem::new(label, true, accelerator);
        commands.insert(item.id().clone(), command);
        menu.append(&item).ok()?;
    }
    Some(())
}

/// One row of the table: which submenu, what it says, what it means, and what it claims to answer to.
type Item<'a> = (
    &'a Submenu,
    &'static str,
    Command,
    Option<(Modifiers, Code)>,
);

/// The File menu. Split from the rest only so no one function is longer than a screen.
fn file_items(file: &Submenu) -> Vec<Item<'_>> {
    vec![
        (
            file,
            "&New",
            Command::New,
            Some((Modifiers::CONTROL, Code::KeyN)),
        ),
        (
            file,
            "&Open…",
            Command::Open,
            Some((Modifiers::CONTROL, Code::KeyO)),
        ),
        (
            file,
            "&Save",
            Command::Save,
            Some((Modifiers::CONTROL, Code::KeyS)),
        ),
        (
            file,
            "Save &As…",
            Command::SaveAs,
            Some((Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyS)),
        ),
        (file, "Save A&ll", Command::SaveAll, None),
        (
            file,
            "&Close",
            Command::Close,
            Some((Modifiers::CONTROL, Code::KeyW)),
        ),
        (file, "Close A&ll", Command::CloseAll, None),
    ]
}

/// The Edit menu.
fn edit_items(edit: &Submenu) -> Vec<Item<'_>> {
    vec![
        (
            edit,
            "&Undo",
            Command::Undo,
            Some((Modifiers::CONTROL, Code::KeyZ)),
        ),
        (
            edit,
            "&Redo",
            Command::Redo,
            Some((Modifiers::CONTROL, Code::KeyY)),
        ),
        (
            edit,
            "Cu&t",
            Command::Cut,
            Some((Modifiers::CONTROL, Code::KeyX)),
        ),
        (
            edit,
            "&Copy",
            Command::Copy,
            Some((Modifiers::CONTROL, Code::KeyC)),
        ),
        (
            edit,
            "&Paste",
            Command::Paste,
            Some((Modifiers::CONTROL, Code::KeyV)),
        ),
        (
            edit,
            "Select &All",
            Command::SelectAll,
            Some((Modifiers::CONTROL, Code::KeyA)),
        ),
        (
            edit,
            "&Find…",
            Command::Find,
            Some((Modifiers::CONTROL, Code::KeyF)),
        ),
        (
            edit,
            "Find &Next",
            Command::FindNext,
            Some((Modifiers::empty(), Code::F3)),
        ),
        (
            edit,
            "Find Pre&vious",
            Command::FindPrevious,
            Some((Modifiers::SHIFT, Code::F3)),
        ),
        (
            edit,
            "&Replace…",
            Command::Replace,
            Some((Modifiers::CONTROL, Code::KeyH)),
        ),
        (
            edit,
            "&Go to Line…",
            Command::GoToLine,
            Some((Modifiers::CONTROL, Code::KeyG)),
        ),
    ]
}

/// The View and Help menus. Together because neither is more than a handful.
fn view_and_help_items<'a>(
    view: &'a Submenu,
    tools: &'a Submenu,
    help: &'a Submenu,
) -> Vec<Item<'a>> {
    vec![
        (
            view,
            "Zoom &In",
            Command::ZoomIn,
            Some((Modifiers::CONTROL, Code::Equal)),
        ),
        (
            view,
            "Zoom &Out",
            Command::ZoomOut,
            Some((Modifiers::CONTROL, Code::Minus)),
        ),
        (
            view,
            "&Reset Zoom",
            Command::ResetZoom,
            Some((Modifiers::CONTROL, Code::Digit0)),
        ),
        // No accelerator, and none is wanted: the check runs on its own on the idle timer, so this
        // item is "take me to the first problem" rather than "check now".
        (tools, "&Validate Document", Command::Validate, None),
        (help, "&Keyboard Shortcuts", Command::Shortcuts, None),
        (help, "&About", Command::About, None),
    ]
}
