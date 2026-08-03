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
    fn ask(
        &mut self,
        _name: &str,
        _what: mdedit_app::prompt::Closing,
    ) -> mdedit_app::prompt::Unsaved {
        mdedit_app::prompt::Unsaved::Discard
    }
}

/// Answers Keep to everything. For the window close, which is the one place *that* is offered.
#[derive(Debug)]
struct AlwaysKeep;

impl mdedit_app::prompt::UnsavedPrompt for AlwaysKeep {
    /// Never reached — this one is only ever asked `ask_all`. Cancel rather than Discard so an
    /// accidental call cannot silently throw work away.
    fn ask(
        &mut self,
        _name: &str,
        _what: mdedit_app::prompt::Closing,
    ) -> mdedit_app::prompt::Unsaved {
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

// ---- scratch buffers (MDEDIT-66 slice 2) ----

/// Records what it was asked about, and answers as told.
#[derive(Debug)]
struct Watching(
    std::rc::Rc<
        std::cell::RefCell<(
            Vec<mdedit_app::prompt::Closing>,
            mdedit_app::prompt::Unsaved,
        )>,
    >,
);

impl Watching {
    fn saying(answer: mdedit_app::prompt::Unsaved) -> Self {
        Self(std::rc::Rc::new(std::cell::RefCell::new((
            Vec::new(),
            answer,
        ))))
    }

    fn share(&self) -> Self {
        Self(std::rc::Rc::clone(&self.0))
    }

    fn asked(&self) -> Vec<mdedit_app::prompt::Closing> {
        self.0.borrow().0.clone()
    }
}

impl mdedit_app::prompt::UnsavedPrompt for Watching {
    fn ask(
        &mut self,
        _name: &str,
        what: mdedit_app::prompt::Closing,
    ) -> mdedit_app::prompt::Unsaved {
        let mut state = self.0.borrow_mut();
        state.0.push(what);
        state.1
    }
}

/// An untitled tab nobody has typed into is not work, and must not fill the store with blanks.
#[test]
fn an_empty_scratch_buffer_is_not_stored() {
    let dir = TempDir::new().unwrap();
    let mut editor = Editor::new("", Grid::new());
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    editor.on_idle();

    let (kept, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(kept.is_empty(), "an empty scratch buffer is not work");
}

/// And typing into one makes it work, which is stored.
#[test]
fn a_scratch_buffer_with_anything_in_it_is_stored() {
    let dir = TempDir::new().unwrap();
    let mut editor = Editor::new("", Grid::new());
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    type_into(&mut editor, "a thought");
    editor.on_idle();

    let (kept, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].path.is_none(), "it belongs to no file");
    assert!(kept[0].text.contains("a thought"));
}

/// Emptying one again takes it back out — the store follows the buffer rather than accumulating
/// every state it has been in.
#[test]
fn emptying_a_scratch_buffer_removes_its_stored_copy() {
    let dir = TempDir::new().unwrap();
    let mut editor = Editor::new("", Grid::new());
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    type_into(&mut editor, "a thought");
    editor.on_idle();
    assert_eq!(
        Store::in_directory(dir.path().join("recovery"))
            .read_all()
            .0
            .len(),
        1
    );

    editor.select_all();
    editor.insert_text("");
    editor.on_idle();

    let (kept, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(kept.is_empty());
}

/// Closing a scratch tab asks a different question from closing a file, because Save means
/// something different: there is nowhere to write it yet.
#[test]
fn closing_a_scratch_tab_asks_as_a_scratch_tab() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");

    let prompt = Watching::saying(mdedit_app::prompt::Unsaved::Discard);
    editor.with_prompt(Box::new(prompt.share()));

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    type_into(&mut editor, "a thought");

    assert!(editor.apply_command(mdedit_menu_win::Command::Close));
    assert_eq!(prompt.asked(), vec![mdedit_app::prompt::Closing::Scratch]);
}

/// And closing a file tab still asks as a file.
#[test]
fn closing_a_file_tab_asks_as_a_file() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");
    type_into(&mut editor, "EDITED");

    let prompt = Watching::saying(mdedit_app::prompt::Unsaved::Discard);
    editor.with_prompt(Box::new(prompt.share()));

    assert!(editor.apply_command(mdedit_menu_win::Command::Close));
    assert_eq!(prompt.asked(), vec![mdedit_app::prompt::Closing::File]);
}

/// The wart this slice fixes. Choosing Save on a scratch tab used to call `save`, which has no path,
/// declined silently, and left the tab refusing to close with only a line in the log to say why.
#[test]
fn save_on_a_scratch_tab_asks_where_to_put_it() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("chosen.md");
    let (mut editor, _) = editor(&dir, "# Notes\n");
    editor.with_prompt(Box::new(Watching::saying(
        mdedit_app::prompt::Unsaved::Save,
    )));

    let chosen = target.clone();
    editor.with_file_picker(Box::new(move |_| Some(chosen.clone())));

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    type_into(&mut editor, "a thought");

    assert!(
        editor.apply_command(mdedit_menu_win::Command::Close),
        "with somewhere to put it, the tab should close"
    );
    assert!(
        std::fs::read_to_string(&target)
            .unwrap()
            .contains("a thought"),
        "and the text should be where the picker said"
    );
}

/// Declining the location leaves the tab open with its text, rather than closing over it.
#[test]
fn declining_the_location_keeps_the_scratch_tab() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");
    editor.with_prompt(Box::new(Watching::saying(
        mdedit_app::prompt::Unsaved::Save,
    )));
    // The default picker declines.

    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    type_into(&mut editor, "a thought");
    let before = editor.workspace().tabs().len();

    assert!(!editor.apply_command(mdedit_menu_win::Command::Close));
    assert_eq!(editor.workspace().tabs().len(), before);
}

/// Scratch buffers come back in the order they were first stored, so a run with several of them is
/// not a lucky dip.
#[test]
fn scratch_buffers_come_back_in_a_stable_order() {
    let dir = TempDir::new().unwrap();
    let mut editor = Editor::new("", Grid::new());
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));

    for text in ["first thought", "second thought", "third thought"] {
        let index = editor.workspace_mut().add_untitled();
        editor.workspace_mut().activate(index).unwrap();
        type_into(&mut editor, text);
        editor.on_idle();
    }
    drop(editor);

    let mut next = Editor::new("", Grid::new());
    next.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    next.restore_unsaved_work(true);

    let texts: Vec<String> = next
        .workspace()
        .tabs()
        .iter()
        .filter_map(mdedit_session::Tab::open)
        .map(|open| open.document.to_string())
        .collect();
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert!(texts[0].contains("first"), "{texts:?}");
    assert!(texts[1].contains("second"), "{texts:?}");
    assert!(texts[2].contains("third"), "{texts:?}");
}

// ---- restoring over a file that moved (MDEDIT-66 slice 3) ----

/// Answers the file-changed question as told, and counts the asking.
#[derive(Debug)]
struct Conflicted(std::rc::Rc<std::cell::RefCell<(usize, mdedit_app::prompt::Conflict)>>);

impl Conflicted {
    fn saying(answer: mdedit_app::prompt::Conflict) -> Self {
        Self(std::rc::Rc::new(std::cell::RefCell::new((0, answer))))
    }

    fn share(&self) -> Self {
        Self(std::rc::Rc::clone(&self.0))
    }

    fn asked(&self) -> usize {
        self.0.borrow().0
    }
}

impl mdedit_app::prompt::ConflictPrompt for Conflicted {
    fn ask(
        &mut self,
        _name: &str,
        _why: mdedit_app::prompt::Conflicting,
    ) -> mdedit_app::prompt::Conflict {
        let mut state = self.0.borrow_mut();
        state.0 += 1;
        state.1
    }
}

/// Stores an edit to `path`, then hands back a fresh editor ready to restore it.
fn stored_edit(dir: &TempDir, contents: &str) -> std::path::PathBuf {
    let path = dir.path().join("notes.md");
    std::fs::write(&path, contents).unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    editor.insert_text("MY EDIT");
    editor.on_idle();
    path
}

fn restoring(dir: &TempDir, conflict: &Conflicted) -> Editor<Grid> {
    let mut editor = Editor::new("", Grid::new());
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    editor.with_conflict_prompt(Box::new(conflict.share()));
    editor
}

/// The ordinary case: nothing else touched the file, so nothing is asked.
#[test]
fn an_unchanged_file_restores_without_asking() {
    let dir = TempDir::new().unwrap();
    stored_edit(&dir, "# Notes\n");

    let conflict = Conflicted::saying(mdedit_app::prompt::Conflict::Reload);
    let mut editor = restoring(&dir, &conflict);
    editor.restore_unsaved_work(true);

    assert_eq!(conflict.asked(), 0, "there was no conflict to ask about");
    assert!(editor.document().to_string().contains("MY EDIT"));
}

/// The case this slice exists for. Reinstating silently would put an edit back on top of whatever
/// else wrote the file.
#[test]
fn a_file_that_changed_underneath_is_asked_about() {
    let dir = TempDir::new().unwrap();
    let path = stored_edit(&dir, "# Notes\n");

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "# Notes\n\nsomething else wrote this\n").unwrap();

    let conflict = Conflicted::saying(mdedit_app::prompt::Conflict::Cancel);
    let mut editor = restoring(&dir, &conflict);
    editor.restore_unsaved_work(true);

    assert_eq!(conflict.asked(), 1);
    assert!(
        editor.document().to_string().contains("MY EDIT"),
        "Cancel means keep mine, which is what restoring is"
    );
}

/// Choosing the file drops the stored edit — and drops it *for good*, or the same question would be
/// asked on every launch from now on.
#[test]
fn choosing_the_file_discards_the_stored_edit_for_good() {
    let dir = TempDir::new().unwrap();
    let path = stored_edit(&dir, "# Notes\n");

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "# Notes\n\nsomething else wrote this\n").unwrap();

    let conflict = Conflicted::saying(mdedit_app::prompt::Conflict::Reload);
    let mut editor = restoring(&dir, &conflict);
    editor.restore_unsaved_work(true);

    assert!(
        !editor.document().to_string().contains("MY EDIT"),
        "the stored edit should not have been put back"
    );

    let (left, _) = Store::in_directory(dir.path().join("recovery")).read_all();
    assert!(
        left.is_empty(),
        "and it should be gone, or every launch asks again"
    );
}

/// A scratch buffer has no file to conflict with, so it is never asked about however long it has
/// been sitting there.
#[test]
fn a_scratch_buffer_is_never_asked_about() {
    let dir = TempDir::new().unwrap();
    let mut editor = Editor::new("", Grid::new());
    editor.with_snapshot_store(Store::in_directory(dir.path().join("recovery")));
    let index = editor.workspace_mut().add_untitled();
    editor.workspace_mut().activate(index).unwrap();
    type_into(&mut editor, "a thought");
    editor.on_idle();
    drop(editor);

    let conflict = Conflicted::saying(mdedit_app::prompt::Conflict::Reload);
    let mut next = restoring(&dir, &conflict);
    next.restore_unsaved_work(true);

    assert_eq!(conflict.asked(), 0);
    assert!(next.document().to_string().contains("a thought"));
}

/// A file deleted since is not a conflict: the stored text is then the only copy of itself, and it
/// comes back as a scratch buffer rather than being argued about.
#[test]
fn a_deleted_file_is_not_a_conflict() {
    let dir = TempDir::new().unwrap();
    let path = stored_edit(&dir, "# Notes\n");
    std::fs::remove_file(&path).unwrap();

    let conflict = Conflicted::saying(mdedit_app::prompt::Conflict::Reload);
    let mut editor = restoring(&dir, &conflict);
    editor.restore_unsaved_work(true);

    assert_eq!(conflict.asked(), 0, "there is nothing to conflict with");
    assert!(editor.document().to_string().contains("MY EDIT"));
}

/// A copy stored by a build older than the stamp has nothing to compare against. Restoring it is
/// right: refusing would lose work over a missing field.
#[test]
fn a_copy_with_no_stamp_is_restored() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Notes\n").unwrap();
    std::fs::create_dir_all(&recovery).unwrap();
    std::fs::write(recovery.join("1.snapshot"), "AN OLD EDIT\n").unwrap();
    std::fs::write(
        recovery.join("manifest.json"),
        format!(
            r#"{{"entries":[{{"id":1,"path":"{}"}}]}}"#,
            // Windows separators need escaping for JSON.
            path.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();

    let conflict = Conflicted::saying(mdedit_app::prompt::Conflict::Reload);
    let mut editor = restoring(&dir, &conflict);
    editor.restore_unsaved_work(true);

    assert_eq!(conflict.asked(), 0);
    assert!(editor.document().to_string().contains("AN OLD EDIT"));
}

// ---------------------------------------------------------------------------
// The default, which is nowhere (MDEDIT-68).
// ---------------------------------------------------------------------------

/// **The bug this whole file could not see.** Every test above installs a store, so none of them
/// noticed that an editor *given* no store used to reach for the real one — and then filled the
/// developer's own recovery directory with test fixtures, which MDEDIT-66 dutifully restored into
/// their editor on the next launch. It was found by launching the editor and reading a tab.
///
/// Asserted on the default rather than by inspecting the user's directory, because a test that reads
/// `%LOCALAPPDATA%` passes on a machine which has never run the editor, and is then no test at all.
#[test]
fn an_editor_nobody_gave_a_store_writes_nowhere() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Notes\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    // Deliberately no `with_snapshot_store`. This is the shape of every test in every other file.

    editor.insert_text("unsaved work nobody asked to keep");
    editor.on_idle();

    assert!(
        !editor.stores_unsaved_work(),
        "an editor built without a store must not have gone and found one"
    );
}

/// And the seam still works, which is what makes the above a fix rather than a removal.
#[test]
fn an_editor_given_a_store_uses_it() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor(&dir, "# Notes\n");

    assert!(editor.stores_unsaved_work());
    editor.insert_text("x");
    editor.on_idle();

    assert!(
        dir.path().join("recovery").join("manifest.json").exists(),
        "the store it was given should have been written to"
    );
}
