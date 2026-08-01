//! Unsaved work, stored and brought back — wired into the editor rather than tested in the store.
//!
//! Started as crash recovery (MDEDIT-54) and became the store for unsaved work generally
//! (MDEDIT-66). The difference is the lifecycle, not the storage: a stored copy now survives an
//! ordinary quit, is removed when its tab closes, and a *crash* is told apart by a marker in the
//! session file rather than by whether anything was left behind.
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

/// **Changed by MDEDIT-66.** A clean exit used to throw the stored copies away — that was how the
/// editor told a crash from an ordinary quit. It no longer does: unsaved work surviving a quit is
/// the whole point, and the clean-exit marker in the session file carries the signal instead.
#[test]
fn a_clean_exit_keeps_them() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");

    type_into(&mut editor, "UNSAVED");
    editor.on_idle();

    let (kept, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert_eq!(kept.len(), 1, "the edit should still be stored");
    assert!(kept[0].text.contains("UNSAVED"));
}

/// Closing a tab is where a document stops existing, so it is where its stored copy stops existing
/// too. Nothing did this before MDEDIT-66 — a leak that did not matter while every clean exit
/// cleared the lot, and matters a great deal now they are permanent.
#[test]
fn closing_a_tab_forgets_its_stored_copy() {
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
        "it has to be stored before removing it means anything"
    );

    editor.with_prompt(Box::new(AlwaysDiscard));
    assert!(editor.apply_command(mdedit_menu_win::Command::Close));

    let (kept, skipped) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(kept.is_empty(), "the closed tab's copy should have gone");
    assert!(skipped.is_empty(), "and left nothing dangling: {skipped:?}");
}

/// Discarding at the *window* level is not offered, and would be wrong if it were: it would close
/// the editor and leave the stored copies, which is Keep with worse words.
#[test]
fn keeping_on_the_way_out_leaves_the_work_stored() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path) = editor(&dir, "# Notes\n");
    type_into(&mut editor, "UNSAVED");
    editor.on_idle();

    editor.with_prompt(Box::new(AlwaysKeep));
    assert!(editor.may_close(), "Keep must let the editor close");

    let (kept, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Notes\n",
        "and must not have written anything"
    );
}

/// The whole point, end to end: what one editor loses, the next one gets back — modified, so the
/// ordinary unsaved-changes machinery protects it and the user decides whether to keep it.
#[test]
fn the_next_run_gets_the_unsaved_bytes_back() {
    let dir = TempDir::new().unwrap();
    let (mut crashed, path) = editor(&dir, "# Notes\n\nshort.\n");

    type_into(&mut crashed, "UNSAVED");
    crashed.on_idle();
    // No session written and no clean-exit marker: this is the crash.
    drop(crashed);

    let mut next = Editor::new("", Grid::new());
    next.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    next.restore_unsaved_work(false);

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
    next.restore_unsaved_work(false);

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
    next.restore_unsaved_work(false);

    assert!(next.document().to_string().contains("never saved anywhere"));
    assert!(
        next.workspace()
            .tabs()
            .iter()
            .any(|tab| tab.path().is_none() && tab.is_modified())
    );
}

/// Answers Discard to everything. For the per-tab close, which is the one place it is offered.
#[derive(Debug)]
struct AlwaysDiscard;

impl mdedit_app::prompt::UnsavedPrompt for AlwaysDiscard {
    fn ask(&mut self, _name: &str) -> mdedit_app::prompt::Unsaved {
        mdedit_app::prompt::Unsaved::Discard
    }
}

/// Answers Keep to everything. For the window close, which is the one place *that* is offered.
#[derive(Debug)]
struct AlwaysKeep;

impl mdedit_app::prompt::UnsavedPrompt for AlwaysKeep {
    /// Never reached — this one is only ever asked `ask_all`. Cancel rather than Discard so an
    /// accidental call cannot silently throw work away.
    fn ask(&mut self, _name: &str) -> mdedit_app::prompt::Unsaved {
        mdedit_app::prompt::Unsaved::Cancel
    }

    fn ask_all(&mut self, _names: &[String]) -> mdedit_app::prompt::Unsaved {
        mdedit_app::prompt::Unsaved::Keep
    }
}

/// Work that was saved is not unsaved work, so nothing keeps a copy of it — the store must not fill
/// up with documents that are safely on disk.
#[test]
fn saving_still_clears_the_stored_copy() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");

    type_into(&mut editor, "UNSAVED");
    editor.on_idle();
    editor.save();
    editor.on_idle();

    let (kept, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(kept.is_empty());
}
