//! Files changing on disk under an open buffer.
//!
//! Two checks with different jobs. The one on window focus keeps the buffer fresh and is a
//! convenience; the one before a save is what stands between somebody else's work and being written
//! over. The save check is where most of these tests are, for that reason.
//!
//! Everything here drives `Editor::check_disk` and `Editor::save` directly. The *focus event* that
//! calls `check_disk` comes from winit and so cannot be delivered here — same limit as
//! `tests/recovery.rs`, and stated for the same reason.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use mdedit_app::editor::Editor;
use mdedit_app::prompt::{Conflict, ConflictPrompt, Conflicting};
use mdedit_session::Workspace;
use tempfile::TempDir;

use crate::stub;
use stub::Grid;

/// Answers with whatever it was told to, and records what it was asked.
#[derive(Debug, Default)]
struct Scripted {
    answer: Option<Conflict>,
    asked: Vec<Conflicting>,
}

#[derive(Debug, Clone, Default)]
struct Answers(Rc<RefCell<Scripted>>);

impl Answers {
    fn saying(answer: Conflict) -> Self {
        let this = Self::default();
        this.0.borrow_mut().answer = Some(answer);
        this
    }

    fn asked(&self) -> Vec<Conflicting> {
        self.0.borrow().asked.clone()
    }
}

impl ConflictPrompt for Answers {
    fn ask(&mut self, _name: &str, why: Conflicting) -> Conflict {
        let mut scripted = self.0.borrow_mut();
        scripted.asked.push(why);
        scripted
            .answer
            .expect("asked a question the test did not script an answer to")
    }
}

/// An editor on one file, with a scripted answer to any conflict.
fn editor(dir: &TempDir, contents: &str, answers: &Answers) -> (Editor<Grid>, std::path::PathBuf) {
    let path = dir.path().join("notes.md");
    std::fs::write(&path, contents).unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.with_conflict_prompt(Box::new(answers.clone()));
    (editor, path)
}

/// Writes `contents` to `path` as another program would.
///
/// The sleep is not decoration. The stamp is a modification time and a length, and a filesystem
/// timestamp is coarse — on FAT32, two seconds. A rewrite in the same instant that happens to keep
/// the length is exactly the false negative documented on `mdedit_file::Stamp`, and without a pause
/// these tests would hit it at random. Every write here changes the length as well, which is what
/// makes them deterministic; the pause is belt and braces.
fn externally_write(path: &std::path::Path, contents: &str) {
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(path, contents).unwrap();
}

#[test]
fn an_unmodified_buffer_reloads_without_asking() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::default();
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);

    externally_write(&path, "# Notes\n\nwritten by something else\n");
    assert!(editor.check_disk());

    assert!(
        editor
            .document()
            .to_string()
            .contains("written by something else")
    );
    assert!(
        answers.asked().is_empty(),
        "there was nothing to lose, so there was nothing to ask"
    );
}

/// Line endings are format, not content: a CRLF file that reloads must still be a CRLF file, or the
/// next save rewrites every line in it.
#[test]
fn a_reload_keeps_the_format() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::default();
    let (mut editor, path) = editor(&dir, "# Notes\r\n\r\nline\r\n", &answers);

    externally_write(&path, "# Notes\r\n\r\nline\r\nand another\r\n");
    assert!(editor.check_disk());

    assert!(
        editor.document().to_string().contains("\r\n"),
        "the CRLF endings did not survive the reload"
    );
    // And a save writes it back unchanged, which is the guarantee that actually matters.
    editor.save();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"# Notes\r\n\r\nline\r\nand another\r\n"
    );
}

#[test]
fn an_unchanged_file_is_not_reloaded_and_not_asked_about() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::default();
    let (mut editor, _) = editor(&dir, "# Notes\n", &answers);

    assert!(!editor.check_disk());
    assert!(answers.asked().is_empty());
}

#[test]
fn a_modified_buffer_asks_and_keeping_mine_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::saying(Conflict::Cancel);
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);
    editor.insert_text("MINE");

    externally_write(&path, "# Notes\n\ntheirs\n");
    assert!(!editor.check_disk());

    assert_eq!(answers.asked(), vec![Conflicting::Found]);
    assert!(editor.document().to_string().contains("MINE"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Notes\n\ntheirs\n",
        "keeping mine must not write anything"
    );
}

#[test]
fn a_modified_buffer_can_be_reloaded_over() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::saying(Conflict::Reload);
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);
    editor.insert_text("MINE");

    externally_write(&path, "# Notes\n\ntheirs\n");
    assert!(editor.check_disk());

    assert_eq!(answers.asked(), vec![Conflicting::Found]);
    let text = editor.document().to_string();
    assert!(text.contains("theirs"));
    assert!(!text.contains("MINE"), "the local edits should be gone");
    assert!(
        !editor.is_modified(),
        "a buffer that matches the file it came from is not modified"
    );
}

/// The one that prevents data loss. Everything else here is convenience.
#[test]
fn saving_over_an_external_change_asks_first() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::saying(Conflict::Cancel);
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);
    editor.insert_text("MINE");

    externally_write(&path, "# Notes\n\ntheirs\n");
    editor.save();

    assert_eq!(answers.asked(), vec![Conflicting::Saving]);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Notes\n\ntheirs\n",
        "cancelling the save must leave the other change on disk"
    );
    assert!(
        editor.document().to_string().contains("MINE"),
        "and must leave the buffer alone"
    );
}

#[test]
fn overwrite_is_a_choice_and_it_writes() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::saying(Conflict::Overwrite);
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);
    editor.insert_text("MINE");

    externally_write(&path, "# Notes\n\ntheirs\n");
    editor.save();

    assert_eq!(answers.asked(), vec![Conflicting::Saving]);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("MINE"));
    assert!(!on_disk.contains("theirs"));
}

/// Choosing to reload at the moment of saving means abandoning the save. Writing afterwards would
/// put back the edits the user just chose to discard.
#[test]
fn reloading_instead_of_saving_does_not_then_write() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::saying(Conflict::Reload);
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);
    editor.insert_text("MINE");

    externally_write(&path, "# Notes\n\ntheirs\n");
    editor.save();

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Notes\n\ntheirs\n"
    );
    assert!(!editor.document().to_string().contains("MINE"));
}

/// The regression the ticket named: a save updates the stamp before anything can observe the file,
/// so the *next* save must not think its own write was somebody else's.
#[test]
fn our_own_save_is_not_an_external_change() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::default();
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);

    editor.insert_text("first");
    editor.save();
    editor.insert_text("second");
    editor.save();

    assert!(
        answers.asked().is_empty(),
        "our own saves asked about themselves: {:?}",
        answers.asked()
    );
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("first") && on_disk.contains("second"));
}

/// A save with no conflict must not ask, or the prompt becomes noise and gets dismissed unread.
#[test]
fn an_ordinary_save_asks_nothing() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::default();
    let (mut editor, _) = editor(&dir, "# Notes\n", &answers);

    editor.insert_text("edit");
    editor.save();

    assert!(answers.asked().is_empty());
}

/// A file deleted from under the editor. The buffer is now the only copy of itself, so it must
/// neither be lost nor reported as an error the user has to dismiss.
#[test]
fn a_deleted_file_leaves_the_buffer_alone() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::default();
    let (mut editor, path) = editor(&dir, "# Notes\n", &answers);
    editor.insert_text("MINE");

    std::fs::remove_file(&path).unwrap();
    editor.check_disk();

    assert!(editor.document().to_string().contains("MINE"));
    assert!(answers.asked().is_empty(), "there is nothing to ask about");
    assert!(
        editor.is_modified(),
        "with no file to compare against, everything in the buffer is unsaved"
    );

    // And saving puts it back, rather than refusing because the file is missing.
    editor.save_as(&path);
    assert!(std::fs::read_to_string(&path).unwrap().contains("MINE"));
}

/// Every open tab is checked, not only the visible one — a background tab holding stale bytes is how
/// the wrong thing gets written later.
#[test]
fn a_background_tab_is_checked_too() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "# First\n").unwrap();
    std::fs::write(&second, "# Second\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(first.clone());
    let index = workspace.add(second.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    // Open both, then leave the first one showing.
    editor.workspace_mut().activate(index).unwrap();
    editor.workspace_mut().activate(0).unwrap();

    externally_write(&second, "# Second\n\nchanged behind your back\n");
    assert!(editor.check_disk());

    let reloaded = editor
        .workspace()
        .tabs()
        .iter()
        .find(|tab| tab.path() == Some(second.as_path()))
        .and_then(mdedit_session::Tab::open)
        .map(|open| open.document.to_string())
        .unwrap();
    assert!(reloaded.contains("changed behind your back"));
}

/// A tab that has never been displayed has read nothing, so there is nothing of it to be stale — and
/// checking it would read the file, which is the whole thing deferred tabs exist to avoid.
#[test]
fn a_deferred_tab_is_not_read_to_check_it() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "# First\n").unwrap();
    std::fs::write(&second, "# Second\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(first);
    workspace.add(second.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    assert_eq!(editor.workspace().materialised_count(), 1);

    externally_write(&second, "# Second\n\nchanged\n");
    editor.check_disk();

    assert_eq!(
        editor.workspace().materialised_count(),
        1,
        "checking for external changes must not materialise a deferred tab"
    );
}

/// MDEDIT-109. Reloading clears every layout cache and, until now, not the word count — so after a
/// file was rewritten underneath and reloaded, the status bar showed the *old* file's number.
///
/// It lasted because nothing schedules an idle tick on a focus change: the count sat wrong until the
/// next keystroke. `reopen_with` clears both, for exactly this stated reason, and `reload` was its
/// inconsistent twin.
#[test]
fn reloading_recounts_the_words() {
    let dir = TempDir::new().unwrap();
    let answers = Answers::saying(Conflict::Reload);
    let (mut editor, path) = editor(
        &dir,
        "one two three four five six
",
        &answers,
    );

    editor.build_frame(900.0, 400.0, 1.0);
    assert_eq!(editor.words(), Some(6), "the count starts right");

    externally_write(&path, "just two\n");
    assert!(editor.check_disk(), "the file changed and was reloaded");
    editor.build_frame(900.0, 400.0, 1.0);

    assert_eq!(
        editor.words(),
        Some(2),
        "the bar is showing the word count of a file that is no longer there"
    );
}
