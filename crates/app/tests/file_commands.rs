//! The File menu's commands, without the menu.
//!
//! `muda` is native and cannot be driven headlessly, as the IME could not — so the menu is a thin
//! layer over these, and these are what get asserted. New, Save As, Save All and Close All are all
//! testable with a real temporary directory and no window at all.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use mdedit_app::editor::Editor;
use mdedit_app::prompt::{Unsaved, UnsavedPrompt};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::Grid;

fn open_on(paths: &[&Path]) -> Editor<Grid> {
    let mut workspace = Workspace::new();
    for path in paths {
        workspace.add((*path).to_path_buf());
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor
}

fn write(dir: &TempDir, name: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

// ---------------------------------------------------------------------------
// File → New, and the untitled buffer it forces into existence.
// ---------------------------------------------------------------------------

#[test]
fn a_new_file_is_untitled_empty_and_showing() {
    let mut editor = Editor::new("", Grid::new());
    editor.new_file();

    let tabs = editor.workspace().tabs();
    assert_eq!(tabs.len(), 1);
    assert!(tabs[0].is_untitled());
    assert_eq!(tabs[0].display_name(), "Untitled 1");
    assert_eq!(tabs[0].path(), None, "there is nowhere to save it yet");
    assert!(editor.document().is_empty());
}

/// The numbers count up rather than filling gaps, so a tab does not get renamed out from under
/// somebody while it is still on screen.
#[test]
fn untitled_numbers_are_not_reused() {
    let mut editor = Editor::new("", Grid::new());
    editor.new_file();
    editor.new_file();
    editor.new_file();

    let names: Vec<String> = editor
        .workspace()
        .tabs()
        .iter()
        .map(mdedit_session::Tab::display_name)
        .collect();
    assert_eq!(names, ["Untitled 1", "Untitled 2", "Untitled 3"]);
}

/// An untitled buffer has no disk to differ from, so "modified" means "has anything in it".
#[test]
fn an_untitled_buffer_becomes_modified_as_soon_as_it_has_content() {
    let mut editor = Editor::new("", Grid::new());
    editor.new_file();
    assert!(
        !editor.is_modified(),
        "an empty new file has nothing to lose"
    );

    editor.insert_text("a");
    assert!(
        editor.is_modified(),
        "there is now something that closing would lose"
    );
}

#[test]
fn the_title_shows_an_untitled_buffer_by_name() {
    let mut editor = Editor::new("", Grid::new());
    editor.new_file();
    assert!(
        editor.title().starts_with("Untitled 1"),
        "got {:?}",
        editor.title()
    );
}

/// Save on an untitled buffer has nowhere to go, and must not pretend otherwise.
#[test]
fn saving_an_untitled_buffer_without_a_path_does_nothing() {
    let mut editor = Editor::new("", Grid::new());
    editor.new_file();
    editor.insert_text("unsaved");

    editor.save();
    assert!(
        editor.is_modified(),
        "nothing was written, so it is still unsaved"
    );
}

// ---------------------------------------------------------------------------
// Save As.
// ---------------------------------------------------------------------------

#[test]
fn save_as_writes_an_untitled_buffer_and_binds_the_tab_to_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("new.md");

    let mut editor = Editor::new("", Grid::new());
    editor.new_file();
    editor.insert_text("# Written\n");

    assert!(editor.save_as(&path));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Written\n");

    let tabs = editor.workspace().tabs();
    assert!(!tabs[0].is_untitled(), "it has a home now");
    assert_eq!(tabs[0].path(), Some(path.as_path()));
    assert_eq!(tabs[0].display_name(), "new.md");
    assert!(!editor.is_modified(), "what is in the buffer is on disk");
}

/// The byte-preservation invariant, at the one place Save As could break it: copying a file that
/// nobody has edited must not re-encode it.
#[test]
fn save_as_on_an_unedited_file_writes_its_original_bytes() {
    let dir = TempDir::new().unwrap();
    // A BOM and CRLF endings: both are format rather than content, and both would be lost by a
    // careless re-encode.
    let original: Vec<u8> = b"\xEF\xBB\xBF# Title\r\n\r\nBody\r\n".to_vec();
    let source = dir.path().join("source.md");
    std::fs::write(&source, &original).unwrap();
    let target = dir.path().join("copy.md");

    let mut editor = open_on(&[source.as_path()]);
    assert!(editor.save_as(&target));

    assert_eq!(
        std::fs::read(&target).unwrap(),
        original,
        "Save As re-encoded a file nobody had edited"
    );
}

#[test]
fn save_as_then_save_writes_to_the_new_path() {
    let dir = TempDir::new().unwrap();
    let source = write(&dir, "source.md", "first\n");
    let target = dir.path().join("target.md");

    let mut editor = open_on(&[source.as_path()]);
    assert!(editor.save_as(&target));

    let end = ByteOffset::new(editor.document().len_bytes());
    editor.document_mut().set_selection(Selection::caret(end));
    editor.insert_text("second\n");
    editor.save();

    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "first\nsecond\n",
        "the later save followed the tab to its new path"
    );
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "first\n",
        "and left the file it came from alone"
    );
}

// ---------------------------------------------------------------------------
// Save All.
// ---------------------------------------------------------------------------

#[test]
fn save_all_writes_every_modified_tab() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");
    let two = write(&dir, "two.md", "two\n");
    let three = write(&dir, "three.md", "three\n");

    let mut editor = open_on(&[one.as_path(), two.as_path(), three.as_path()]);

    // Edit the first and the third, leaving the second alone.
    for index in [0, 2] {
        editor.workspace_mut().activate(index).unwrap();
        let end = ByteOffset::new(editor.document().len_bytes());
        editor.document_mut().set_selection(Selection::caret(end));
        editor.insert_text("edited\n");
    }

    assert_eq!(editor.save_all(), 2, "two were modified");
    assert_eq!(std::fs::read_to_string(&one).unwrap(), "one\nedited\n");
    assert_eq!(std::fs::read_to_string(&two).unwrap(), "two\n");
    assert_eq!(std::fs::read_to_string(&three).unwrap(), "three\nedited\n");
}

#[test]
fn save_all_leaves_the_same_tab_showing() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");
    let two = write(&dir, "two.md", "two\n");

    let mut editor = open_on(&[one.as_path(), two.as_path()]);
    editor.workspace_mut().activate(1).unwrap();
    editor.insert_text("x");

    editor.save_all();
    assert_eq!(
        editor.workspace().active(),
        Some(1),
        "saving should not move the user to another file"
    );
}

/// An untitled tab has no path, so Save All has nothing to do with it — and must not report a
/// failure for something it was never asked to decide.
#[test]
fn save_all_skips_an_untitled_buffer() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");

    let mut editor = open_on(&[one.as_path()]);
    editor.insert_text("x");
    editor.new_file();
    editor.insert_text("untitled content");

    assert_eq!(editor.save_all(), 1, "only the one with a path");
    assert!(
        editor.workspace().tabs()[1].is_modified(),
        "the untitled one is still waiting for a name"
    );
}

#[test]
fn save_all_with_nothing_modified_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");
    let mut editor = open_on(&[one.as_path()]);
    assert_eq!(editor.save_all(), 0);
}

// ---------------------------------------------------------------------------
// Close All.
// ---------------------------------------------------------------------------

#[test]
fn close_all_closes_everything_when_nothing_is_unsaved() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");
    let two = write(&dir, "two.md", "two\n");

    let mut editor = open_on(&[one.as_path(), two.as_path()]);
    assert!(editor.close_all());
    assert!(editor.workspace().tabs().is_empty());
    assert_eq!(editor.workspace().active(), None);
}

// ---------------------------------------------------------------------------
// Closing, and the question it has to ask.
// ---------------------------------------------------------------------------

/// Answers the unsaved-changes question without a dialog, and counts how often it was asked.
#[derive(Debug, Clone)]
struct Answers(Rc<RefCell<(Unsaved, usize)>>);

impl Answers {
    fn always(answer: Unsaved) -> Self {
        Self(Rc::new(RefCell::new((answer, 0))))
    }

    fn times_asked(&self) -> usize {
        self.0.borrow().1
    }
}

impl UnsavedPrompt for Answers {
    fn ask(&mut self, _name: &str) -> Unsaved {
        let mut state = self.0.borrow_mut();
        state.1 += 1;
        state.0
    }
}

#[test]
fn close_all_asks_about_each_modified_tab_and_discarding_closes_them() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");
    let two = write(&dir, "two.md", "two\n");

    let answers = Answers::always(Unsaved::Discard);
    let mut editor = open_on(&[one.as_path(), two.as_path()]);
    editor.with_prompt(Box::new(answers.clone()));

    // Modify both.
    for index in [0, 1] {
        editor.workspace_mut().activate(index).unwrap();
        editor.insert_text("x");
    }

    assert!(editor.close_all());
    assert!(editor.workspace().tabs().is_empty());
    assert_eq!(answers.times_asked(), 2, "once per modified tab");
    assert_eq!(
        std::fs::read_to_string(&one).unwrap(),
        "one\n",
        "discard means discard: the file on disk is untouched"
    );
}

/// A cancelled prompt means "I did not mean this", not "skip that one".
#[test]
fn close_all_stops_on_cancel_and_keeps_everything() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");
    let two = write(&dir, "two.md", "two\n");

    let answers = Answers::always(Unsaved::Cancel);
    let mut editor = open_on(&[one.as_path(), two.as_path()]);
    editor.with_prompt(Box::new(answers.clone()));
    editor.workspace_mut().activate(0).unwrap();
    editor.insert_text("x");

    assert!(!editor.close_all(), "it did not finish");
    assert_eq!(
        editor.workspace().tabs().len(),
        2,
        "cancelling the first must not close the second"
    );
    assert_eq!(answers.times_asked(), 1, "and must not keep asking");
}

#[test]
fn choosing_save_writes_the_tab_before_closing_it() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one.md", "one\n");

    let mut editor = open_on(&[one.as_path()]);
    editor.with_prompt(Box::new(Answers::always(Unsaved::Save)));
    let end = ByteOffset::new(editor.document().len_bytes());
    editor.document_mut().set_selection(Selection::caret(end));
    editor.insert_text("kept\n");

    assert!(editor.close_all());
    assert!(editor.workspace().tabs().is_empty());
    assert_eq!(std::fs::read_to_string(&one).unwrap(), "one\nkept\n");
}

/// An untitled buffer with content has nowhere to be saved to, so choosing Save cannot succeed —
/// and the tab must stay rather than being closed on the strength of a save that did not happen.
#[test]
fn an_untitled_buffer_is_not_closed_by_a_save_that_cannot_happen() {
    let mut editor = Editor::new("", Grid::new());
    editor.with_prompt(Box::new(Answers::always(Unsaved::Save)));
    editor.new_file();
    editor.insert_text("would be lost");

    assert!(!editor.close_all(), "it could not finish");
    assert_eq!(
        editor.workspace().tabs().len(),
        1,
        "the buffer is still here, with its contents"
    );
    assert_eq!(editor.document().to_string(), "would be lost");
}
