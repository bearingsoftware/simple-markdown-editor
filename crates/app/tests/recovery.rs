//! Crash recovery, wired into the editor rather than tested in the store.
//!
//! `mdedit-session`'s own tests cover what the store keeps and refuses. These cover the wiring, which
//! is where this feature was actually broken: the snapshot rides the idle timer, and that timer used
//! to be scheduled only for documents large enough to be parsed in regions — so the buffers most
//! worth protecting, the small unsaved ones, were the exact buffers never snapshotted.
//!
//! What these do **not** cover is that scheduling. They call [`Editor::on_idle`] directly, because
//! the timer is set through an `ActiveEventLoop` that only winit can hand out — so a test here would
//! have passed against the broken build too. The scheduling is verified by hand on Windows: type,
//! kill the process, relaunch, and see the bytes come back. Anything below this line is the work the
//! timer does once it fires; that it fires at all is a manual check.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_session::Workspace;
use mdedit_session::snapshot::Store;
use tempfile::TempDir;

mod stub;
use stub::Grid;

/// An editor on one saved file, with its snapshots pointed at a temporary directory.
fn editor(dir: &TempDir, contents: &str) -> (Editor<Grid>, std::path::PathBuf) {
    let path = dir.path().join("notes.md");
    std::fs::write(&path, contents).unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    (editor, path)
}

fn type_into(editor: &mut Editor<Grid>, text: &str) {
    editor.insert_text(text);
}

/// The regression this file exists for.
///
/// A five-line note is under every threshold in the editor: it parses whole, it lays out in one
/// chunk, and it used to schedule no idle work at all. Nothing about it should stop a snapshot.
#[test]
fn a_small_document_is_snapshotted_too() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path) = editor(&dir, "# Notes\n\nshort.\n");

    type_into(&mut editor, "UNSAVED");
    editor.on_idle();

    let (recovered, skipped) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].text.contains("UNSAVED"), "{:?}", recovered[0]);
    assert_eq!(recovered[0].path.as_deref(), Some(path.as_path()));

    // And the file on disk is untouched — a snapshot is not a save.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Notes\n\nshort.\n"
    );
}

/// An unmodified buffer has nothing to protect, and a snapshot of one would offer on the next start
/// to restore what is already on disk.
#[test]
fn an_unmodified_buffer_is_not_snapshotted() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");

    editor.on_idle();

    let (recovered, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(recovered.is_empty());
}

/// The lifecycle claim: work that reaches disk stops being recoverable work.
#[test]
fn saving_clears_the_snapshot() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");

    type_into(&mut editor, "UNSAVED");
    editor.on_idle();
    assert_eq!(
        Store::in_directory(dir.path().join("recovery"))
            .read_all()
            .0
            .len(),
        1,
        "the snapshot has to exist before clearing it means anything"
    );

    editor.save();
    editor.on_idle();

    let (recovered, skipped) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(recovered.is_empty(), "saved work needs no recovering");
    assert!(skipped.is_empty(), "{skipped:?}");
}

/// A run that ends on purpose leaves nothing behind, which is what makes anything left behind mean
/// "the last run did not end here".
#[test]
fn a_clean_exit_discards_them() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");

    type_into(&mut editor, "UNSAVED");
    editor.on_idle();
    editor.discard_snapshots();

    let (recovered, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(recovered.is_empty());
}

/// The whole point, end to end: what one editor loses, the next one gets back — modified, so the
/// ordinary unsaved-changes machinery protects it and the user decides whether to keep it.
#[test]
fn the_next_run_gets_the_unsaved_bytes_back() {
    let dir = TempDir::new().unwrap();
    let (mut crashed, path) = editor(&dir, "# Notes\n\nshort.\n");

    type_into(&mut crashed, "UNSAVED");
    crashed.on_idle();
    // No `discard_snapshots`, and no session written: this is the crash.
    drop(crashed);

    let mut next = Editor::new("", Grid::new());
    next.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    next.recover_snapshots();

    assert!(
        next.document().to_string().contains("UNSAVED"),
        "the unsaved bytes did not come back"
    );
    assert!(
        next.workspace()
            .tabs()
            .iter()
            .any(|tab| tab.path() == Some(path.as_path()) && tab.is_modified()),
        "the recovered tab must be marked modified, or a save would look unnecessary"
    );
}

/// Recovery runs after the session has been restored, so the file is normally already open. Two tabs
/// on one file — one holding the recovered text, one holding what is on disk — is the most confusing
/// thing this feature could do.
///
/// It does not happen, because `Workspace::add` returns the existing tab for a path it already
/// holds. This pins that, since recovery depends on it: a change to `add` that dropped the dedupe
/// would break recovery in a way nothing in `mdedit-session` would notice.
#[test]
fn recovering_a_file_already_open_reuses_its_tab() {
    let dir = TempDir::new().unwrap();
    let (mut crashed, path) = editor(&dir, "# Notes\n\nshort.\n");
    type_into(&mut crashed, "UNSAVED");
    crashed.on_idle();
    drop(crashed);

    // The next run, as `main` builds it: the session reopens the tab, then recovery runs.
    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut next = Editor::with_workspace(workspace, Grid::new());
    next.activate_saved_tab();
    next.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    next.recover_snapshots();

    assert_eq!(next.workspace().tabs().len(), 1, "one file, one tab");
    assert!(next.document().to_string().contains("UNSAVED"));
}

/// The case with the most to lose: a buffer that was never saved anywhere has no path to identify
/// it, and no copy of it exists anywhere else at all.
#[test]
fn an_untitled_buffer_comes_back() {
    let dir = TempDir::new().unwrap();

    let mut crashed = Editor::new("", Grid::new());
    crashed.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    crashed.workspace_mut().add_untitled();
    let index = crashed.workspace().tabs().len() - 1;
    crashed.workspace_mut().activate(index).unwrap();
    type_into(&mut crashed, "never saved anywhere");
    crashed.on_idle();
    drop(crashed);

    let mut next = Editor::new("", Grid::new());
    next.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    next.recover_snapshots();

    assert!(next.document().to_string().contains("never saved anywhere"));
    assert!(
        next.workspace()
            .tabs()
            .iter()
            .any(|tab| tab.path().is_none() && tab.is_modified())
    );
}
