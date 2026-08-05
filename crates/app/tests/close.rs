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
use mdedit_app::prompt::{Closing, Unsaved, UnsavedPrompt};
use mdedit_session::Workspace;
use mdedit_session::snapshot::Store;
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
    fn ask(&mut self, name: &str, _what: Closing) -> Unsaved {
        let mut asked = self.0.borrow_mut();
        asked.dialogs.push(vec![name.to_owned()]);
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
/// Points an editor's recovery store at a scratch directory, as `main` does.
fn with_store(editor: &mut Editor<Grid>, dir: &TempDir) {
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
}

/// What the store holds, as text.
fn stored(dir: &TempDir) -> Vec<String> {
    let (recovered, skipped) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(skipped.is_empty(), "{skipped:?}");
    recovered.into_iter().map(|entry| entry.text).collect()
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

/// **The feature.** Closing the program with unsaved work closes it. MDEDIT-64 put a dialog here
/// because a close used to lose the work; MDEDIT-54 and MDEDIT-66 then made the work survive, and
/// the dialog was left asking the user to decide something already handled.
#[test]
fn unsaved_work_closes_without_asking() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::default();
    let (mut editor, _) = editor(&dir, 3, &prompt);
    with_store(&mut editor, &dir);
    for index in 0..3 {
        dirty(&mut editor, index);
    }

    assert!(editor.may_close(), "closing the program closes it");
    assert!(
        prompt.dialogs().is_empty(),
        "and asks nothing: {:?}",
        prompt.dialogs()
    );
}

/// And the work is there to come back to, which is the whole reason nothing is asked.
#[test]
fn the_unsaved_work_is_in_the_store_afterwards() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::default();
    let (mut editor, _) = editor(&dir, 2, &prompt);
    with_store(&mut editor, &dir);
    dirty(&mut editor, 0);
    dirty(&mut editor, 1);

    editor.may_close();

    let kept = stored(&dir);
    assert_eq!(kept.len(), 2, "both buffers should be kept: {kept:?}");
    assert!(kept.iter().all(|text| text.contains("EDITED")));
}

/// **The flush is synchronous, and that is not a detail.** The copy on disk is whatever the idle
/// timer last wrote, so an edit made and the window closed inside that interval would come back a
/// few keystrokes short — which is worse than the dialog was, because it looks like the feature
/// working.
///
/// Nothing calls `on_idle` here, deliberately: this is the interval.
#[test]
fn closing_flushes_what_the_idle_timer_has_not_written_yet() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::default();
    let (mut editor, _) = editor(&dir, 1, &prompt);
    with_store(&mut editor, &dir);

    dirty(&mut editor, 0);
    editor.on_idle();
    // And then more typing, with no tick after it.
    editor.insert_text("LAST WORDS");

    editor.may_close();

    let kept = stored(&dir);
    assert_eq!(kept.len(), 1);
    assert!(
        kept[0].contains("LAST WORDS"),
        "the last keystrokes before the close were lost: {:?}",
        kept[0]
    );
}

/// An untitled buffer has nowhere to be saved and the most to lose, which is why the old flow asked
/// where to put it. It is kept instead, and asked about not at all.
#[test]
fn an_untitled_buffer_is_kept_rather_than_asked_about() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::default();
    let (mut editor, _) = editor(&dir, 1, &prompt);
    with_store(&mut editor, &dir);

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    editor.insert_text("never saved anywhere");

    assert!(editor.may_close());
    assert!(prompt.dialogs().is_empty());
    assert!(
        stored(&dir).iter().any(|text| text.contains("never saved")),
        "the buffer with the most to lose was not kept"
    );
}

/// **Seamless caching is not auto-save.** The file on disk is not written on the way out; the copy
/// beside the session file is. That distinction is deliberate and is what keeps this from being a
/// feature that edits people's files behind them.
#[test]
fn nothing_is_written_to_the_files_themselves() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::default();
    let (mut editor, paths) = editor(&dir, 2, &prompt);
    with_store(&mut editor, &dir);
    let before: Vec<String> = paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();

    dirty(&mut editor, 0);
    dirty(&mut editor, 1);
    editor.may_close();

    for (path, was) in paths.iter().zip(before) {
        assert_eq!(std::fs::read_to_string(path).unwrap(), was);
    }
}

/// **Closing a *tab* still asks.** That is where a document is deliberately discarded, and the user
/// is looking at it. The unprompted flow is for leaving the program, not for every close.
#[test]
fn closing_a_tab_still_asks() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::saying(Unsaved::Cancel);
    let (mut editor, _) = editor(&dir, 1, &prompt);
    dirty(&mut editor, 0);

    assert!(!editor.apply_chord("w", false), "cancel keeps the tab");
    assert_eq!(prompt.dialogs().len(), 1, "and it asked");
    assert_eq!(editor.workspace().tabs().len(), 1);
}

/// An editor with nobody to ask still closes, which is the other half of asking nothing: the answer
/// does not depend on a dialog being available.
#[test]
fn an_editor_with_no_prompt_closes_over_unsaved_work() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Notes\n").unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);

    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    with_store(&mut editor, &dir);
    editor.insert_text("EDITED");

    assert!(editor.may_close());
    assert!(stored(&dir).iter().any(|text| text.contains("EDITED")));
}

/// And an editor with no store still closes rather than refusing. Losing the copy is a smaller
/// problem than an editor that will not shut down, and `Editor::stores_unsaved_work` is the seam
/// `main` uses to make sure that case is only ever a test's.
#[test]
fn an_editor_with_no_store_still_closes() {
    let dir = TempDir::new().unwrap();
    let prompt = Scripted::default();
    let (mut editor, _) = editor(&dir, 1, &prompt);
    dirty(&mut editor, 0);

    assert!(!editor.stores_unsaved_work());
    assert!(editor.may_close());
}
