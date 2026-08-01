//! The Win32 half: building the menu, attaching it, and reading what was chosen.

// The only crate other than the text backend permitted to lower this, and for one call. See the
// crate docs.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuId, MenuItem, MenuTheme, Submenu};

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
        let help = Submenu::new("&Help", true);

        // Accelerator text only — see the crate docs. The key handler owns the chords.
        let items = [
            (
                &file,
                "&New",
                Command::New,
                Some((Modifiers::CONTROL, Code::KeyN)),
            ),
            (
                &file,
                "&Open…",
                Command::Open,
                Some((Modifiers::CONTROL, Code::KeyO)),
            ),
            (
                &file,
                "&Save",
                Command::Save,
                Some((Modifiers::CONTROL, Code::KeyS)),
            ),
            (
                &file,
                "Save &As…",
                Command::SaveAs,
                Some((Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyS)),
            ),
            (&file, "Save A&ll", Command::SaveAll, None),
            (
                &file,
                "&Close",
                Command::Close,
                Some((Modifiers::CONTROL, Code::KeyW)),
            ),
            (&file, "Close A&ll", Command::CloseAll, None),
            (
                &edit,
                "&Undo",
                Command::Undo,
                Some((Modifiers::CONTROL, Code::KeyZ)),
            ),
            (
                &edit,
                "&Redo",
                Command::Redo,
                Some((Modifiers::CONTROL, Code::KeyY)),
            ),
            (
                &edit,
                "Cu&t",
                Command::Cut,
                Some((Modifiers::CONTROL, Code::KeyX)),
            ),
            (
                &edit,
                "&Copy",
                Command::Copy,
                Some((Modifiers::CONTROL, Code::KeyC)),
            ),
            (
                &edit,
                "&Paste",
                Command::Paste,
                Some((Modifiers::CONTROL, Code::KeyV)),
            ),
            (&help, "&Keyboard Shortcuts", Command::Shortcuts, None),
            (&help, "&About", Command::About, None),
        ];

        for (menu, label, command, chord) in items {
            let accelerator =
                chord.map(|(modifiers, code)| Accelerator::new(Some(modifiers), code));
            let item = MenuItem::new(label, true, accelerator);
            commands.insert(item.id().clone(), command);
            menu.append(&item).ok()?;
        }

        // Empty and disabled until the app fills it in — see `set_recent`. Placed after Open, where
        // every editor puts it.
        let recent = Submenu::new("Open &Recent", false);
        file.insert(&recent, 2).ok()?;

        // A separator between saving and closing, because they are different kinds of decision.
        // Index 6 rather than 4: Open and Open Recent went in above it.
        file.insert(&muda::PredefinedMenuItem::separator(), 6)
            .ok()?;

        menu.append(&file).ok()?;
        menu.append(&edit).ok()?;
        menu.append(&help).ok()?;
        Some(Self {
            menu,
            commands,
            recent,
            recent_ids: Arc::new(Mutex::new(HashMap::new())),
        })
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
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = commands.get(&event.id).copied() {
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
