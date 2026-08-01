//! What the Ctrl chords actually do, as opposed to what the menu says they do.
//!
//! Until MDEDIT-65 nothing here was testable at all: a chord arrives as a winit `KeyEvent`, which
//! carries a private platform-specific field and so cannot be constructed by a test. That is how
//! Ctrl+Shift+S came to be labelled "Save As" in the menu and to plain-save over the file.
//!
//! `Editor::apply_chord` takes the character and Shift directly, which is the whole of what the
//! character arms read, so these exercise the real dispatch rather than a copy of it. What they do
//! *not* cover is the step above — that winit reports Shift+S as `"S"` and that `handle_chord`
//! passes `modifiers.shift_key()` — which stays a manual check.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mdedit_app::editor::Editor;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::Grid;

/// An editor on one saved file, and a picker that answers with `save_to`.
fn editor(dir: &TempDir, save_to: Option<PathBuf>) -> (Editor<Grid>, PathBuf, Rc<RefCell<usize>>) {
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Notes\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

    let asked = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&asked);
    editor.with_file_picker(Box::new(move |_| {
        *counter.borrow_mut() += 1;
        save_to.clone()
    }));
    (editor, path, asked)
}

/// The bug. Ctrl+Shift+S must ask where to put it, not overwrite what is already there.
#[test]
fn ctrl_shift_s_is_save_as_not_save() {
    let dir = TempDir::new().unwrap();
    let elsewhere = dir.path().join("copy.md");
    let (mut editor, original, asked) = editor(&dir, Some(elsewhere.clone()));
    editor.insert_text("EDITED");

    editor.apply_chord("S", true);

    assert_eq!(*asked.borrow(), 1, "Save As must ask where to put it");
    assert!(
        std::fs::read_to_string(&elsewhere)
            .unwrap()
            .contains("EDITED")
    );
    assert_eq!(
        std::fs::read_to_string(&original).unwrap(),
        "# Notes\n",
        "the file it came from must not have been written"
    );
}

/// And the unshifted chord must still be a plain save, asking nothing.
#[test]
fn ctrl_s_saves_without_asking() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path, asked) = editor(&dir, None);
    editor.insert_text("EDITED");

    editor.apply_chord("s", false);

    assert_eq!(*asked.borrow(), 0);
    assert!(std::fs::read_to_string(&path).unwrap().contains("EDITED"));
}

/// A dismissed Save As writes nowhere at all — not to the new path, and not to the old one as a
/// consolation.
#[test]
fn a_dismissed_save_as_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path, asked) = editor(&dir, None);
    editor.insert_text("EDITED");

    assert!(!editor.apply_chord("S", true));

    assert_eq!(*asked.borrow(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Notes\n");
}

/// Ctrl+Z and Ctrl+Shift+Z are the other pair that reads Shift, and the one this fix was modelled
/// on. Pinned so the two stay consistent.
#[test]
fn shift_turns_undo_into_redo() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    editor.insert_text("EDITED");

    editor.apply_chord("z", false);
    assert!(!editor.document().to_string().contains("EDITED"));

    editor.apply_chord("Z", true);
    assert!(editor.document().to_string().contains("EDITED"));
}

/// A chord nothing answers to must do nothing, rather than falling into a neighbouring arm.
#[test]
fn an_unbound_chord_does_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path, asked) = editor(&dir, None);
    editor.insert_text("EDITED");

    assert!(!editor.apply_chord("q", false));
    assert!(!editor.apply_chord("q", true));

    assert_eq!(*asked.borrow(), 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Notes\n");
    assert!(editor.document().to_string().contains("EDITED"));
}

/// Ctrl+A, which has no shifted meaning: both must select everything rather than one doing nothing.
#[test]
fn select_all_ignores_shift() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);

    assert!(editor.apply_chord("a", false));
    assert!(editor.apply_chord("A", true));
}
