//! The Win32 half: building the menu, attaching it, and reading what was chosen.

// The only crate other than the text backend permitted to lower this, and for one call. See the
// crate docs.
#![allow(unsafe_code)]

use std::collections::HashMap;

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuId, MenuItem, MenuTheme, Submenu};

use crate::Command;

/// A menu bar, and which item means what.
pub struct MenuBar {
    menu: Menu,
    /// muda identifies items by an opaque id, so the mapping back to a command is kept here rather
    /// than inferred from the label — a label is what the user reads, not an identifier.
    commands: HashMap<MenuId, Command>,
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
        ];

        for (menu, label, command, chord) in items {
            let accelerator =
                chord.map(|(modifiers, code)| Accelerator::new(Some(modifiers), code));
            let item = MenuItem::new(label, true, accelerator);
            commands.insert(item.id().clone(), command);
            menu.append(&item).ok()?;
        }

        // A separator between saving and closing, because they are different kinds of decision.
        file.insert(&muda::PredefinedMenuItem::separator(), 4)
            .ok()?;

        menu.append(&file).ok()?;
        menu.append(&edit).ok()?;
        Some(Self { menu, commands })
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
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = commands.get(&event.id).copied() {
                deliver(command);
            }
        }));
    }
}
