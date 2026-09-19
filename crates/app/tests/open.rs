//! File → Open, and the recent list it feeds.
//!
//! The dialog itself is `rfd` and cannot be linked into a test binary, so it is behind a seam here
//! exactly as the save picker is — these drive the seam. That the native dialog appears at all is a
//! manual check, as Save As's is.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mdedit_app::editor::Editor;
use mdedit_session::recent::Recent;
use tempfile::TempDir;

use crate::stub;
use stub::Grid;

/// An editor with nothing open and its recent list in a temporary directory.
fn editor(dir: &TempDir) -> Editor<Grid> {
    let mut editor = Editor::new("", Grid::new());
    editor.with_recent(Recent::at(dir.path().join("recent.json")));
    editor
}

fn a_file(dir: &TempDir, name: &str, contents: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

/// Hands back a fixed answer, and counts how many times it was asked.
fn picker(paths: Vec<PathBuf>, asked: &Rc<RefCell<usize>>) -> mdedit_app::editor::OpenPathPicker {
    let asked = Rc::clone(asked);
    Box::new(move || {
        *asked.borrow_mut() += 1;
        Some(paths.clone())
    })
}

#[test]
fn opening_a_file_shows_it() {
    let dir = TempDir::new().unwrap();
    let path = a_file(&dir, "notes.md", "# Opened\n\nfrom a dialog\n");
    let mut editor = editor(&dir);

    assert!(editor.open_paths(std::slice::from_ref(&path)));
    assert!(editor.document().to_string().contains("from a dialog"));
    assert_eq!(editor.workspace().active_path(), Some(path.as_path()));
}

/// The case a multi-select picker exists for.
#[test]
fn several_files_each_get_a_tab() {
    let dir = TempDir::new().unwrap();
    let first = a_file(&dir, "one.md", "# One\n");
    let second = a_file(&dir, "two.md", "# Two\n");
    let third = a_file(&dir, "three.md", "# Three\n");
    let mut editor = editor(&dir);

    assert!(editor.open_paths(&[first, second, third.clone()]));
    assert_eq!(editor.workspace().tabs().len(), 3);
    assert_eq!(
        editor.workspace().active_path(),
        Some(third.as_path()),
        "the last one opened is the one to show"
    );
}

/// **Twenty files, in one call, in selection order** — the editor half of MDEDIT-61's acceptance
/// case, and the reason the number is twenty rather than three. A command-line verb defaults to
/// `MultiSelectModel=Document`, which stops invoking the verb at all past fifteen items; the
/// `DropTarget` verb hands the whole selection over as one call, and this is what that call does
/// when it lands. The COM half cannot be tested here — see `mdedit-shell-win` — so what is pinned is
/// that nothing in the editor caps, reorders or drops a selection of that size.
#[test]
fn a_whole_explorer_selection_opens_in_order() {
    let dir = TempDir::new().unwrap();
    let paths: Vec<PathBuf> = (0..20)
        .map(|index| {
            a_file(
                &dir,
                &format!("note-{index:02}.md"),
                &format!("# {index}\n"),
            )
        })
        .collect();
    let mut editor = editor(&dir);

    assert!(editor.open_paths(&paths));
    assert_eq!(editor.workspace().tabs().len(), 20);

    let opened: Vec<PathBuf> = editor
        .workspace()
        .tabs()
        .iter()
        .filter_map(|tab| tab.path().map(Path::to_path_buf))
        .collect();
    assert_eq!(opened, paths, "the tabs are not in the order selected");
}

#[test]
fn opening_something_already_open_activates_its_tab() {
    let dir = TempDir::new().unwrap();
    let path = a_file(&dir, "notes.md", "# Notes\n");
    let other = a_file(&dir, "other.md", "# Other\n");
    let mut editor = editor(&dir);

    editor.open_paths(&[path.clone(), other]);
    assert_eq!(editor.workspace().tabs().len(), 2);

    editor.open_paths(std::slice::from_ref(&path));
    assert_eq!(editor.workspace().tabs().len(), 2, "no duplicate tab");
    assert_eq!(editor.workspace().active_path(), Some(path.as_path()));
}

#[test]
fn the_picker_is_asked_and_what_it_says_is_opened() {
    let dir = TempDir::new().unwrap();
    let path = a_file(&dir, "notes.md", "# Chosen\n");
    let asked = Rc::new(RefCell::new(0));

    let mut editor = editor(&dir);
    editor.with_open_picker(picker(vec![path.clone()], &asked));

    assert!(editor.open_prompted());
    assert_eq!(*asked.borrow(), 1);
    assert!(editor.document().to_string().contains("Chosen"));
}

/// An editor with no dialog opens nothing rather than guessing — the same default as saving.
#[test]
fn an_editor_with_no_picker_opens_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor(&dir);
    assert!(!editor.open_prompted());
    assert!(editor.workspace().tabs().is_empty());
}

/// A dismissed dialog is not a failure and not an empty file.
#[test]
fn a_dismissed_dialog_opens_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor(&dir);
    editor.with_open_picker(Box::new(|| None));

    assert!(!editor.open_prompted());
    assert!(editor.workspace().tabs().is_empty());
}

#[test]
fn opening_records_it_as_recent_most_recent_first() {
    let dir = TempDir::new().unwrap();
    let first = a_file(&dir, "one.md", "# One\n");
    let second = a_file(&dir, "two.md", "# Two\n");
    let mut editor = editor(&dir);

    editor.open_paths(&[first.clone(), second.clone()]);
    assert_eq!(editor.recent().paths(), [second, first]);
}

#[test]
fn a_recent_entry_can_be_opened_by_its_position() {
    let dir = TempDir::new().unwrap();
    let first = a_file(&dir, "one.md", "# One\n");
    let second = a_file(&dir, "two.md", "# Two\n");
    let mut editor = editor(&dir);
    editor.open_paths(&[first.clone(), second]);

    // Position 1 is the older of the two — position 0 is what is already showing.
    assert!(editor.open_recent(1));
    assert_eq!(editor.workspace().active_path(), Some(first.as_path()));
}

#[test]
fn opening_a_recent_position_that_is_not_there_does_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor(&dir);
    assert!(!editor.open_recent(0));
    assert!(!editor.open_recent(99));
}

/// A file deleted since it was last opened must fail politely and stop being offered, rather than
/// failing again every time the menu is opened.
#[test]
fn a_recent_file_that_has_gone_is_dropped_rather_than_opened() {
    let dir = TempDir::new().unwrap();
    let path = a_file(&dir, "notes.md", "# Notes\n");
    let mut editor = editor(&dir);
    editor.open_paths(std::slice::from_ref(&path));
    assert_eq!(editor.recent().paths().len(), 1);

    std::fs::remove_file(&path).unwrap();
    assert!(!editor.open_paths(&[path]));
    assert!(editor.recent().paths().is_empty());
}

/// A path a person chose is not a path this process wrote, and the two go through different
/// functions on purpose. Neither accepts this one.
#[test]
fn a_hostile_path_is_not_opened() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor(&dir);

    assert!(!editor.open_paths(&[PathBuf::from(r"\\evil.example\share\x.md")]));
    assert!(editor.workspace().tabs().is_empty());
    assert!(editor.recent().paths().is_empty());
}

/// One bad path in a multi-select must not take the good ones with it.
#[test]
fn one_bad_path_does_not_stop_the_rest() {
    let dir = TempDir::new().unwrap();
    let good = a_file(&dir, "good.md", "# Good\n");
    let mut editor = editor(&dir);

    assert!(editor.open_paths(&[dir.path().join("missing.md"), good.clone()]));
    assert_eq!(editor.workspace().tabs().len(), 1);
    assert_eq!(editor.workspace().active_path(), Some(good.as_path()));
}

/// The list is what survives a restart, so an editor built fresh against the same file sees it.
#[test]
fn the_recent_list_survives_the_editor() {
    let dir = TempDir::new().unwrap();
    let path = a_file(&dir, "notes.md", "# Notes\n");

    let mut first_run = editor(&dir);
    first_run.open_paths(std::slice::from_ref(&path));
    drop(first_run);

    assert_eq!(editor(&dir).recent().paths(), [path]);
}
