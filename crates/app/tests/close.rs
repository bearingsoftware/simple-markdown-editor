//! Closing the window with unsaved work.
//!
//! Until MDEDIT-64 this lost it: `CloseRequested` called `event_loop.exit()` and nothing else, and
//! the clean-exit path then cleared the crash-recovery snapshots that would have got it back. A
//! *crash* was survivable and a *close* was not.
//!
//! `Editor::may_close` is the decision, and it is what these drive. The `CloseRequested` arm itself
//! is one `if` over it, delivered by winit — the same limit `tests/recovery.rs` and
//! `tests/external.rs` state, for the same reason.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mdedit_app::editor::Editor;
use mdedit_app::prompt::{Unsaved, UnsavedPrompt};
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::Grid;

/// Answers with whatever it was told to, and records what it was shown.
#[derive(Debug, Default)]
struct Asked {
    answer: Option<Unsaved>,
    /// One entry per dialog. The *length* is the assertion that matters: three modified files must
    /// produce one dialog, not three.
    dialogs: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
struct Scripted(Rc<RefCell<Asked>>);

impl Scripted {
    fn saying(answer: Unsaved) -> Self {
        let this = Self::default();
        this.0.borrow_mut().answer = Some(answer);
        this
    }

    fn dialogs(&self) -> Vec<Vec<String>> {
        self.0.borrow().dialogs.clone()
    }
}

impl UnsavedPrompt for Scripted {
    fn ask(&mut self, name: &str) -> Unsaved {
        let mut asked = self.0.borrow_mut();
        asked.dialogs.push(vec![name.to_owned()]);
        asked.answer.expect("asked without a scripted answer")
    }

    fn ask_all(&mut self, names: &[String]) -> Unsaved {
        let mut asked = self.0.borrow_mut();
        asked.dialogs.push(names.to_vec());
        asked.answer.expect("asked without a scripted answer")
    }
}

/// An editor on `count` saved files, with a scripted answer.
fn editor(dir: &TempDir, count: usize, prompt: &Scripted) -> (Editor<Grid>, Vec<PathBuf>) {
    let mut workspace = Workspace::new();
    let mut paths = Vec::new();
    for index in 0..count {
        let path = dir.path().join(format!("note-{index}.md"));
        std::fs::write(&path, format!("# Note {index}\n")).unwrap();
        workspace.add(path.clone());
        paths.push(path);
    }

    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.with_prompt(Box::new(prompt.clone()));
    (editor, paths)
}

/// Edits tab `index`, leaving it modified.
fn dirty(editor: &mut Editor<Grid>, index: usize) {
    editor.workspace_mut().activate(index).unwrap();
    editor.insert_text("EDITED");
}

/// Nothing to lose, so nothing to ask. A prompt on every close is one people learn to dismiss.
#[test]
fn a_clean_editor_closes_without_asking() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::default();
    let (mut editor, _) = editor(&dir, 2, &prompt);

    assert!(editor.may_close());
    assert!(prompt.dialogs().is_empty());
}

/// The bug. Closing with unsaved work must ask.
#[test]
fn unsaved_work_is_asked_about() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Cancel);
    let (mut editor, _) = editor(&dir, 1, &prompt);
    dirty(&mut editor, 0);

    assert!(!editor.may_close(), "cancel must keep the editor open");
    assert_eq!(prompt.dialogs().len(), 1);
}

/// One dialog, not one per file — three consecutive dialogs is a thing people dismiss without
/// reading, which turns a safeguard into a formality.
#[test]
fn three_modified_files_produce_one_dialog_naming_all_three() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Cancel);
    let (mut editor, _) = editor(&dir, 3, &prompt);
    for index in 0..3 {
        dirty(&mut editor, index);
    }

    editor.may_close();

    let dialogs = prompt.dialogs();
    assert_eq!(dialogs.len(), 1, "one dialog: {dialogs:?}");
    assert_eq!(dialogs[0].len(), 3, "naming all three: {dialogs:?}");
}

/// And only the modified ones are named — listing files with nothing wrong with them makes the list
/// useless.
#[test]
fn only_the_modified_files_are_named() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Cancel);
    let (mut editor, _) = editor(&dir, 3, &prompt);
    dirty(&mut editor, 1);

    editor.may_close();

    let dialogs = prompt.dialogs();
    assert_eq!(dialogs[0], vec!["note-1.md".to_owned()]);
}

#[test]
fn cancel_keeps_everything_where_it_was() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Cancel);
    let (mut editor, paths) = editor(&dir, 1, &prompt);
    dirty(&mut editor, 0);

    assert!(!editor.may_close());
    assert!(editor.workspace().has_unsaved_changes());
    assert_eq!(
        std::fs::read_to_string(&paths[0]).unwrap(),
        "# Note 0\n",
        "cancelling must not have written anything"
    );
}

#[test]
fn discard_closes_without_writing() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Discard);
    let (mut editor, paths) = editor(&dir, 1, &prompt);
    dirty(&mut editor, 0);

    assert!(editor.may_close());
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "# Note 0\n");
}

#[test]
fn save_writes_every_modified_file_and_then_closes() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Save);
    let (mut editor, paths) = editor(&dir, 3, &prompt);
    dirty(&mut editor, 0);
    dirty(&mut editor, 2);

    assert!(editor.may_close());

    assert!(
        std::fs::read_to_string(&paths[0])
            .unwrap()
            .contains("EDITED")
    );
    assert!(
        std::fs::read_to_string(&paths[2])
            .unwrap()
            .contains("EDITED")
    );
    assert_eq!(
        std::fs::read_to_string(&paths[1]).unwrap(),
        "# Note 1\n",
        "the untouched file must be untouched"
    );
    assert!(!editor.workspace().has_unsaved_changes());
}

/// An untitled buffer has nowhere to go, so Save has to ask. Declining leaves it modified, and the
/// close is cancelled — exiting then would lose exactly the work the user asked to keep.
#[test]
fn declining_where_to_put_an_untitled_buffer_cancels_the_close() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Save);
    let (mut editor, _) = editor(&dir, 1, &prompt);

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    editor.insert_text("never saved");
    // The default picker declines, which is what a dismissed Save As dialog is.

    assert!(
        !editor.may_close(),
        "with work still unsaved, the editor must stay open"
    );
    assert!(editor.workspace().has_unsaved_changes());
}

/// And choosing a location for it writes it and lets the close through.
#[test]
fn saving_an_untitled_buffer_somewhere_lets_the_close_through() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("chosen.md");
    let prompt = Scripted::saying(Unsaved::Save);
    let (mut editor, _) = editor(&dir, 1, &prompt);

    let chosen = target.clone();
    editor.with_file_picker(Box::new(move |_| Some(chosen.clone())));

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    editor.insert_text("never saved");

    assert!(editor.may_close());
    assert!(
        std::fs::read_to_string(&target)
            .unwrap()
            .contains("never saved")
    );
}

/// A save that cannot happen must not let the editor close over the top of it. Here the file's
/// directory has gone, so the write fails.
#[test]
fn a_failed_save_cancels_the_close() {
    let dir = TempDir::new().unwrap();
    let inner = dir.path().join("inner");
    std::fs::create_dir(&inner).unwrap();
    let path = inner.join("note.md");
    std::fs::write(&path, "# Note\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.with_prompt(Box::new(Scripted::saying(Unsaved::Save)));
    editor.insert_text("EDITED");

    // The directory goes away between the edit and the save.
    std::fs::remove_dir_all(&inner).unwrap();

    assert!(
        !editor.may_close(),
        "a save that did not happen must not let the editor close"
    );
}

/// The editor with no dialog answers Cancel, which for a close means staying open. Safe, and the
/// same default every other prompt seam has.
#[test]
fn an_editor_with_no_prompt_does_not_close_over_unsaved_work() {
    let dir = TempDir::new().unwrap();
    let mut workspace = Workspace::new();
    let path = dir.path().join("note.md");
    std::fs::write(&path, "# Note\n").unwrap();
    workspace.add(path);

    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.insert_text("EDITED");

    assert!(!editor.may_close());
}
