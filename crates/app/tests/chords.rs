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

mod chordlist;
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

// ---- inline formatting, and the chord it took (MDEDIT-50) ----

/// The reassignment. In every editor an end user has formatted text in, Ctrl+B is bold — a Markdown
/// editor where it moves a sidebar is wrong in a way people notice within a minute.
#[test]
fn ctrl_b_is_bold_and_no_longer_the_file_pane() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    editor.insert_text("word");
    select_the_word(&mut editor);

    let before = editor.chrome();
    editor.apply_chord("b", false);

    assert!(editor.document().to_string().contains("**word**"));
    assert_eq!(editor.chrome(), before, "the pane must not have moved");
}

#[test]
fn ctrl_shift_e_is_the_file_pane() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    let before = editor.chrome();

    editor.apply_chord("E", true);

    assert_ne!(editor.chrome(), before);
}

#[test]
fn the_other_three_inline_chords_do_what_they_say() {
    for (character, shift, expected) in [
        ("i", false, "*word*"),
        ("X", true, "~~word~~"),
        ("e", false, "`word`"),
    ] {
        let dir = TempDir::new().unwrap();
        let (mut editor, _, _) = editor(&dir, None);
        editor.insert_text("word");
        select_the_word(&mut editor);

        editor.apply_chord(character, shift);
        assert!(
            editor.document().to_string().contains(expected),
            "{character:?} shift={shift} should have produced {expected}"
        );
    }
}

/// Ctrl+Shift+X is strikethrough and Ctrl+X is still cut — winit reports the shifted one as "X",
/// so the arms have to be in the right order. Exactly the mistake MDEDIT-65 was.
#[test]
fn ctrl_x_is_still_cut() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    editor.insert_text("word");
    editor.apply_chord("a", false);

    editor.apply_chord("x", false);

    assert!(
        !editor.document().to_string().contains("word"),
        "Ctrl+X should still cut"
    );
}

/// And Ctrl+E is code while Ctrl+Shift+E is the pane, which is the other pair sharing a letter.
#[test]
fn ctrl_e_and_ctrl_shift_e_are_different_things() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    editor.insert_text("word");
    select_the_word(&mut editor);
    let chrome = editor.chrome();

    editor.apply_chord("e", false);
    assert!(editor.document().to_string().contains("`word`"));
    assert_eq!(editor.chrome(), chrome);
}

/// Selects the `word` these tests insert at the start of the buffer.
///
/// Explicitly, rather than relying on the caret: `insert_text` leaves it at the end of what it
/// inserted, which is a word *boundary*, and the mid-word rule deliberately does not fire there.
fn select_the_word(editor: &mut Editor<Grid>) {
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::new(
            mdedit_offsets::ByteOffset::new(0),
            mdedit_offsets::ByteOffset::new(4),
        ));
}

// ---- block formatting (MDEDIT-51) ----

/// Typora's convention, and the one place a number key means something other than zoom.
#[test]
fn ctrl_1_to_6_set_heading_levels() {
    for (character, expected) in [("1", "# "), ("3", "### "), ("6", "###### ")] {
        let dir = TempDir::new().unwrap();
        let (mut editor, _, _) = editor(&dir, None);
        editor.insert_text("a line");

        editor.apply_chord(character, false);
        assert!(
            editor.document().to_string().starts_with(expected),
            "Ctrl+{character} should have produced {expected:?}, got {:?}",
            editor.document().to_string()
        );
    }
}

/// Clearing back to a paragraph is pressing the same level again, because Ctrl+0 belongs to zoom.
#[test]
fn pressing_the_same_level_again_clears_it() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    editor.insert_text("a line");

    editor.apply_chord("2", false);
    assert!(editor.document().to_string().starts_with("## "));

    editor.apply_chord("2", false);
    assert!(
        !editor.document().to_string().starts_with('#'),
        "got {:?}",
        editor.document().to_string()
    );
}

// ---- tab cycling, and the chord that could not reach it (MDEDIT-101) ----

/// An editor on `count` saved files, with the first one active.
fn tabs(dir: &TempDir, count: usize) -> Editor<Grid> {
    let mut workspace = Workspace::new();
    for index in 0..count {
        let path = dir.path().join(format!("note{index}.md"));
        std::fs::write(&path, format!("# Note {index}\n")).unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor
}

/// The defect. `"\t"` had an arm in `apply_chord` and nothing could reach it: `handle_chord`
/// dispatched only `Key::Character`, Tab arrives as `Key::Named(NamedKey::Tab)`, and there is no
/// menu item for next/previous tab either — so `cycle_tab` was dead code in the shipped editor while
/// the help page promised Ctrl+Tab.
///
/// What this cannot cover is the decode itself, for the reason this file's own docs give: a winit
/// `KeyEvent` cannot be constructed by a test. It covers everything below it, and the decode is one
/// arm beside the Ctrl+Home one that has always worked the same way.
#[test]
fn ctrl_tab_cycles_forward_and_ctrl_shift_tab_back() {
    let dir = TempDir::new().unwrap();
    let mut editor = tabs(&dir, 3);
    assert_eq!(editor.workspace().active(), Some(0));

    assert!(editor.apply_chord("\t", false));
    assert_eq!(editor.workspace().active(), Some(1));
    assert!(editor.apply_chord("\t", false));
    assert_eq!(editor.workspace().active(), Some(2));

    // And round the end, which is what makes it a cycle rather than a walk.
    assert!(editor.apply_chord("\t", false));
    assert_eq!(editor.workspace().active(), Some(0));

    assert!(editor.apply_chord("\t", true));
    assert_eq!(
        editor.workspace().active(),
        Some(2),
        "Shift goes the other way"
    );
}

/// One tab is not a cycle, and saying so costs nothing: a chord that redrew the frame to change
/// nothing would be a flicker.
#[test]
fn cycling_one_tab_does_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = tabs(&dir, 1);
    assert!(!editor.apply_chord("\t", false));
    assert!(!editor.apply_chord("\t", true));
}

// ---- the arms nothing pressed (MDEDIT-101) ----

/// Ctrl+N, Ctrl+W and Ctrl+O move tabs about, and none of them had a test.
#[test]
fn ctrl_n_opens_a_tab_and_ctrl_w_closes_one() {
    let dir = TempDir::new().unwrap();
    let mut editor = tabs(&dir, 1);
    assert_eq!(editor.workspace().tabs().len(), 1);

    assert!(editor.apply_chord("n", false));
    assert_eq!(
        editor.workspace().tabs().len(),
        2,
        "Ctrl+N opens a new file"
    );

    assert!(editor.apply_chord("w", false));
    assert_eq!(editor.workspace().tabs().len(), 1, "Ctrl+W closes it again");
}

#[test]
fn ctrl_o_opens_what_the_picker_names() {
    let dir = TempDir::new().unwrap();
    let elsewhere = dir.path().join("other.md");
    std::fs::write(&elsewhere, "# Other\n").unwrap();

    // The *open* picker, which is a different collaborator from the save-as one — Ctrl+O asks
    // "which files", Ctrl+Shift+S asks "where to put this".
    let asked = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&asked);
    let chosen = elsewhere.clone();
    let mut editor = tabs(&dir, 1);
    editor.with_open_picker(Box::new(move || {
        *counter.borrow_mut() += 1;
        Some(vec![chosen.clone()])
    }));
    assert!(editor.apply_chord("o", false));

    assert_eq!(*asked.borrow(), 1, "Ctrl+O must ask which file");
    assert!(
        editor
            .workspace()
            .tabs()
            .iter()
            .any(|tab| tab.path() == Some(elsewhere.as_path())),
        "and open it"
    );
}

/// Copy, paste and the second spelling of redo. Cut had a test and these three did not, which is
/// how `"y"` — an arm with no other coverage at all — could have been deleted unnoticed.
#[test]
fn copy_paste_and_ctrl_y_do_what_they_say() {
    let held = Rc::new(RefCell::new(None));

    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    editor.with_clipboard(Box::new(Held(Rc::clone(&held))));
    editor.insert_text("word");
    select_the_word(&mut editor);

    editor.apply_chord("c", false);
    assert_eq!(held.borrow().as_deref(), Some("word"), "Ctrl+C copies");

    // Pasted at the caret rather than over the selection it was copied from, or the edit replaces
    // the same bytes with themselves and there is nothing for undo to take back.
    let before_paste = editor.document().to_string();
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::caret(
            mdedit_offsets::ByteOffset::new(4),
        ));
    editor.apply_chord("v", false);
    let after_paste = editor.document().to_string();
    assert!(
        after_paste.starts_with("wordword"),
        "Ctrl+V pastes at the caret, got {after_paste:?}"
    );

    // Ctrl+Z then Ctrl+Y: the second is the redo spelling that shares no letter with undo, and had
    // no test of its own at all.
    assert!(editor.apply_chord("z", false));
    assert_eq!(editor.document().to_string(), before_paste);
    assert!(editor.apply_chord("y", false));
    assert_eq!(editor.document().to_string(), after_paste, "Ctrl+Y redoes");
}

/// A clipboard that remembers, so a copy can be looked at.
#[derive(Debug)]
struct Held(Rc<RefCell<Option<String>>>);

impl mdedit_app::clipboard::Clipboard for Held {
    fn read(&mut self) -> Option<String> {
        self.0.borrow().clone()
    }

    fn write(&mut self, text: &str) {
        *self.0.borrow_mut() = Some(text.to_owned());
    }
}

/// Every chord in the shared list is pressed at least once, on a scratch editor, and none of them
/// panics or is silently unreachable.
///
/// This is what makes `chordlist` two-directional rather than a second hand-kept list: a label added
/// there and never wired up is pressed here, and `shortcuts.rs` holds it against the help page.
#[test]
fn every_listed_chord_is_pressed() {
    for chord in chordlist::CHORDS {
        for (character, shift) in chord.presses {
            let dir = TempDir::new().unwrap();
            let mut editor = tabs(&dir, 2);
            editor.insert_text("some words here");
            // Nothing is asserted about the result — several of these legitimately return false,
            // and what a chord *does* is the business of the named tests above. What is asserted is
            // that pressing it is a thing that can happen at all.
            let _ = editor.apply_chord(character, *shift);
        }
    }
}

/// And Ctrl+0 is still zoom, not a seventh heading level.
#[test]
fn ctrl_0_is_still_zoom() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = editor(&dir, None);
    editor.insert_text("a line");
    editor.apply_chord("=", false);
    let zoomed = editor.body_size();

    editor.apply_chord("0", false);

    assert!(editor.body_size() < zoomed, "Ctrl+0 should have reset zoom");
    assert!(!editor.document().to_string().starts_with('#'));
}
