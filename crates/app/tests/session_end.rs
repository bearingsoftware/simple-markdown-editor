//! A machine shutdown is not an ordinary exit (MDEDIT-120).
//!
//! These cover the work the event loop does when Windows says the session is ending: flush unsaved
//! buffers, mark the exit clean, and set the flag the window subclass waits on. They go through
//! [`wake_for`] and [`Editor::on_session_wake`], the seams the window subclass and the event loop
//! use. What they do **not** cover is the `WM_QUERYENDSESSION` and `WM_ENDSESSION` handling in
//! `mdedit-menu-win`, because no test can deliver a session-end message; that is checked by hand
//! with a real log-off.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use mdedit_app::editor::{Editor, wake_for};
use mdedit_menu_win::SessionEnd;
use mdedit_session::snapshot::Store;
use mdedit_session::{Workspace, last_run_ended_cleanly};
use tempfile::TempDir;

use crate::stub;
use stub::Grid;

fn editor(dir: &TempDir, with_session: bool) -> (Editor<Grid>, PathBuf) {
    let file = dir.path().join("notes.md");
    std::fs::write(&file, "# Notes\n").unwrap();
    let session = dir.path().join("session.json");

    let mut workspace = Workspace::new();
    workspace.add(file);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    if with_session {
        editor.with_session_path(session.clone());
    }
    (editor, session)
}

fn recovered(dir: &TempDir) -> Vec<String> {
    Store::in_directory(dir.path().join("recovery"))
        .read_all()
        .0
        .into_iter()
        .map(|snapshot| snapshot.text)
        .collect()
}

#[test]
fn ending_flushes_typing_and_marks_the_exit_clean() {
    let dir = TempDir::new().unwrap();
    let (mut editor, session) = editor(&dir, true);

    editor.insert_text("UNSAVED");
    assert!(recovered(&dir).is_empty(), "nothing flushed before the end");
    assert!(editor.on_session_wake(&wake_for(SessionEnd::Ending)));

    let texts = recovered(&dir);
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("UNSAVED"), "{texts:?}");
    assert!(last_run_ended_cleanly(&session));
}

#[test]
fn a_cancelled_shutdown_makes_the_session_unclean_again() {
    let dir = TempDir::new().unwrap();
    let (mut editor, session) = editor(&dir, true);

    editor.on_session_wake(&wake_for(SessionEnd::Ending));
    assert!(last_run_ended_cleanly(&session));
    editor.on_session_wake(&wake_for(SessionEnd::Cancelled));

    assert!(!last_run_ended_cleanly(&session));
}

#[test]
fn the_done_flag_follows_ending_and_cancelling() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, true);

    assert!(!editor.session_ended());
    editor.on_session_wake(&wake_for(SessionEnd::Ending));
    assert!(editor.session_ended());
    editor.on_session_wake(&wake_for(SessionEnd::Cancelled));
    assert!(!editor.session_ended());
}

#[test]
fn without_a_session_path_it_still_flushes_and_signals() {
    let dir = TempDir::new().unwrap();
    let (mut editor, session) = editor(&dir, false);

    editor.insert_text("UNSAVED");
    editor.on_session_wake(&wake_for(SessionEnd::Ending));

    assert!(recovered(&dir)[0].contains("UNSAVED"));
    assert!(editor.session_ended());
    assert!(!session.exists());
    assert!(
        !editor.end_session(),
        "nothing was written, so it must not say it was"
    );
}

#[test]
fn other_wakes_are_not_taken() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, true);

    assert!(!editor.on_session_wake(&mdedit_app::editor::Wake::Pictures));
    assert!(!editor.session_ended());
}

/// With no tabs there is nothing to restore, so nothing is written — the same rule the ordinary exit
/// path keeps. An empty session file would replace a real one with a list of nothing.
#[test]
fn with_no_tabs_no_session_is_written() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    let mut editor = Editor::new("", Grid::new());
    editor.with_session_path(session.clone());

    assert!(!editor.end_session());
    assert!(!session.exists());
    assert!(
        editor.session_ended(),
        "the subclass is still told to stop waiting"
    );
}
