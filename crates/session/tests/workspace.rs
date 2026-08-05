//! Deferred tabs, session round trips, and the path boundary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use mdedit_session::{PathRejected, Workspace, path};
use tempfile::TempDir;

/// Writes `count` small Markdown files and returns their paths.
fn files(dir: &TempDir, count: usize) -> Vec<PathBuf> {
    (0..count)
        .map(|index| {
            let path = dir.path().join(format!("note-{index}.md"));
            std::fs::write(&path, format!("# Note {index}\n\nbody\n")).unwrap();
            path
        })
        .collect()
}

/// The headline rule: a tab is a handle until something displays it.
#[test]
fn adding_a_tab_does_not_read_the_file() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 5);

    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }

    assert_eq!(workspace.tabs().len(), 5);
    assert_eq!(
        workspace.materialised_count(),
        0,
        "adding tabs must not open anything"
    );
    assert!(workspace.tabs().iter().all(|tab| !tab.is_materialised()));
}

/// The acceptance criterion, asserted rather than eyeballed.
#[test]
fn restoring_two_hundred_tabs_opens_at_most_the_visible_one() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 200);

    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }
    workspace.activate(7).unwrap();

    let session = dir.path().join("session.json");
    workspace.save_session(&session).unwrap();

    let (mut restored, skipped) = Workspace::restore_session(&session).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(restored.tabs().len(), 200);
    assert_eq!(
        restored.materialised_count(),
        0,
        "restoring a session must read the session file and nothing else"
    );

    restored.activate(7).unwrap();
    assert_eq!(
        restored.materialised_count(),
        1,
        "displaying one tab opens one document, not two hundred"
    );
}

#[test]
fn activating_a_tab_reads_it_once() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 3);
    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }

    workspace.activate(1).unwrap();
    assert_eq!(workspace.materialised_count(), 1);
    workspace.activate(1).unwrap();
    assert_eq!(workspace.materialised_count(), 1, "no second read");

    workspace.activate(2).unwrap();
    assert_eq!(workspace.materialised_count(), 2);
    assert_eq!(workspace.active(), Some(2));
}

#[test]
fn the_caret_survives_a_session_round_trip() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 2);
    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }

    {
        let open = workspace.activate(0).unwrap();
        open.document
            .set_selection(mdedit_document::Selection::caret(
                mdedit_offsets::ByteOffset::new(9),
            ));
    }

    let session = dir.path().join("session.json");
    workspace.save_session(&session).unwrap();

    let (mut restored, _) = Workspace::restore_session(&session).unwrap();
    let open = restored.activate(0).unwrap();
    assert_eq!(
        open.document.selection().head(),
        mdedit_offsets::ByteOffset::new(9)
    );
}

/// A caret past the end of a file that shrank since must clamp, not panic.
#[test]
fn a_stale_caret_is_clamped_rather_than_trusted() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("shrinks.md");
    std::fs::write(&path, "a long line of text\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    {
        let open = workspace.activate(0).unwrap();
        open.document
            .set_selection(mdedit_document::Selection::caret(
                mdedit_offsets::ByteOffset::new(18),
            ));
    }
    let session = dir.path().join("session.json");
    workspace.save_session(&session).unwrap();

    // The file shrinks behind our back.
    std::fs::write(&path, "hi\n").unwrap();

    let (mut restored, _) = Workspace::restore_session(&session).unwrap();
    let open = restored.activate(0).unwrap();
    assert!(open.document.selection().head().get() <= open.document.len_bytes());
}

#[test]
fn adding_a_file_twice_reuses_its_tab() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);
    let mut workspace = Workspace::new();

    let first = workspace.add(paths[0].clone());
    let second = workspace.add(paths[0].clone());
    assert_eq!(first, second);
    assert_eq!(workspace.tabs().len(), 1);
}

#[test]
fn closing_reports_unsaved_changes_rather_than_deciding() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 2);
    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }

    assert!(!workspace.has_unsaved_changes());
    {
        let open = workspace.activate(0).unwrap();
        open.document
            .insert(mdedit_offsets::ByteOffset::ZERO, "x")
            .unwrap();
    }
    assert!(workspace.has_unsaved_changes());

    assert!(
        workspace.close(0).unwrap(),
        "closing a modified tab reports that it was modified"
    );
    assert_eq!(workspace.tabs().len(), 1);
    assert!(!workspace.has_unsaved_changes());
}

/// A deferred tab has never been editable, so it cannot be dirty.
#[test]
fn a_deferred_tab_is_never_modified() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 3);
    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }
    assert!(!workspace.has_unsaved_changes());
    assert!(!workspace.close(1).unwrap());
}

#[test]
fn closing_keeps_a_sensible_tab_active() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 3);
    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }

    workspace.activate(2).unwrap();
    workspace.close(0).unwrap();
    assert_eq!(
        workspace.active(),
        Some(1),
        "closing a tab before the active one shifts the index down"
    );

    workspace.close(1).unwrap();
    assert_eq!(
        workspace.active(),
        Some(0),
        "closing the active tab shows its neighbour"
    );

    workspace.close(0).unwrap();
    assert_eq!(workspace.active(), None, "and nothing when none are left");
}

#[test]
fn display_name_is_the_file_name() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);
    let mut workspace = Workspace::new();
    workspace.add(paths[0].clone());
    assert_eq!(workspace.tabs()[0].display_name(), "note-0.md");
}

// ---------------------------------------------------------------------------
// The trust boundary.
// ---------------------------------------------------------------------------

/// The attack this boundary exists for.
///
/// A UNC path in a session file makes Windows open an SMB connection and attempt NTLM
/// authentication to an attacker's host the moment the tab is restored — a credential leak with
/// nothing on screen. Refused on shape, before the filesystem is touched.
#[test]
fn unc_and_device_paths_are_refused() {
    for hostile in [
        r"\\evil.example\share\note.md",
        r"\\.\pipe\anything",
        r"\\?\C:\Users\someone\note.md",
        "//evil.example/share/note.md",
    ] {
        let result = path::validate(std::path::Path::new(hostile));
        assert!(
            matches!(result, Err(PathRejected::NetworkOrDevice(_))),
            "{hostile} should be refused as a network or device path, got {result:?}"
        );
    }
}

#[test]
fn relative_and_traversing_paths_are_refused() {
    assert!(matches!(
        path::validate(std::path::Path::new("relative/note.md")),
        Err(PathRejected::NotAbsolute(_))
    ));

    let traversing = if cfg!(windows) {
        r"C:\Users\someone\..\..\Windows\System32\config\SAM"
    } else {
        "/home/someone/../../etc/shadow"
    };
    assert!(
        matches!(
            path::validate(std::path::Path::new(traversing)),
            Err(PathRejected::Traversal(_))
        ),
        "a `..` component must be refused before the filesystem is consulted"
    );
}

#[test]
fn a_directory_is_not_a_file() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        path::validate(dir.path()),
        Err(PathRejected::NotAFile(_))
    ));
}

#[test]
fn an_ordinary_file_is_accepted() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);
    assert_eq!(path::validate(&paths[0]), Ok(()));
}

/// One bad entry costs one tab, not the session.
#[test]
fn a_hostile_entry_is_skipped_and_reported() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 2);
    let session = dir.path().join("session.json");

    let hostile = format!(
        r#"{{"version":1,"active":0,"tabs":[
            {{"path":{good0},"caret":0}},
            {{"path":"\\\\evil.example\\share\\x.md","caret":0}},
            {{"path":"relative.md","caret":0}},
            {{"path":{good1},"caret":0}}
        ]}}"#,
        good0 = serde_json::to_string(&paths[0]).unwrap(),
        good1 = serde_json::to_string(&paths[1]).unwrap(),
    );
    std::fs::write(&session, hostile).unwrap();

    let (workspace, skipped) = Workspace::restore_session(&session).unwrap();
    assert_eq!(workspace.tabs().len(), 2, "the two real files survive");
    assert_eq!(skipped.len(), 2, "and the two bad entries are reported");
    assert_eq!(workspace.materialised_count(), 0);
}

#[test]
fn a_deleted_file_is_skipped_rather_than_fatal() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 2);
    let mut workspace = Workspace::new();
    for path in &paths {
        workspace.add(path.clone());
    }
    let session = dir.path().join("session.json");
    workspace.save_session(&session).unwrap();

    std::fs::remove_file(&paths[0]).unwrap();

    let (restored, skipped) = Workspace::restore_session(&session).unwrap();
    assert_eq!(restored.tabs().len(), 1);
    assert_eq!(skipped.len(), 1);
    assert!(matches!(skipped[0].reason, PathRejected::NotAFile(_)));
}

#[test]
fn a_crafted_active_index_cannot_address_past_the_end() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);
    let session = dir.path().join("session.json");
    std::fs::write(
        &session,
        format!(
            r#"{{"version":1,"active":9999,"tabs":[{{"path":{},"caret":0}}]}}"#,
            serde_json::to_string(&paths[0]).unwrap()
        ),
    )
    .unwrap();

    let (workspace, _) = Workspace::restore_session(&session).unwrap();
    assert_eq!(workspace.active(), Some(0), "clamped, not trusted");
}

#[test]
fn a_malformed_session_is_an_error_not_a_panic() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");

    for content in [
        "not json at all",
        "{}",
        r#"{"version":999,"tabs":[]}"#,
        "[]",
    ] {
        std::fs::write(&session, content).unwrap();
        assert!(
            Workspace::restore_session(&session).is_err(),
            "{content:?} should be rejected"
        );
    }
}

#[test]
fn a_missing_session_file_is_an_error_rather_than_an_empty_workspace() {
    let dir = TempDir::new().unwrap();
    // Distinguishable from "a session with no tabs", which is a different thing to report.
    assert!(Workspace::restore_session(&dir.path().join("absent.json")).is_err());
}

#[test]
fn an_empty_session_round_trips() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    Workspace::new().save_session(&session).unwrap();

    let (workspace, skipped) = Workspace::restore_session(&session).unwrap();
    assert!(workspace.tabs().is_empty());
    assert_eq!(workspace.active(), None);
    assert!(skipped.is_empty());
}

#[test]
fn saving_creates_the_directory_it_needs() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("a").join("b").join("session.json");
    Workspace::new().save_session(&nested).unwrap();
    assert!(nested.exists());
}

/// A tab that has been opened but not edited is not modified.
///
/// The title bar reads this to decide whether to show an unsaved marker, and it showed one on a
/// freshly restored session.
#[test]
fn a_freshly_opened_tab_is_not_modified() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);
    let mut workspace = Workspace::new();
    workspace.add(paths[0].clone());
    workspace.activate(0).unwrap();

    let open = workspace.active_open().unwrap();
    assert_eq!(
        open.document.to_string(),
        open.file.as_ref().expect("a file-backed tab").text(),
        "opening a file must not change its text"
    );
    assert!(!workspace.has_unsaved_changes());
}

// ---------------------------------------------------------------------------
// The other trust source: a path the user typed.
// ---------------------------------------------------------------------------

/// MDEDIT-33. The command line went through `validate`, so `mdedit README.md` exited 1.
///
/// A relative path is how anyone invokes a CLI. It is refused from a session file — where it would
/// mean whatever directory the editor happened to be launched from — and accepted here, because
/// here that directory is exactly what the user meant.
#[test]
fn a_relative_argument_resolves_against_the_working_directory() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);
    let name = paths[0].file_name().unwrap();

    // The resolution is against the process's directory, so the test has to stand in it. That is
    // process-wide state in a suite that runs threaded, and it is safe here only because every other
    // test in this binary addresses its `TempDir` absolutely — a test that used a relative path
    // would have to be told about this one. The directory is restored before anything is asserted,
    // so a failure does not leave the rest of the suite somewhere else.
    let restore = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let resolved = path::from_command_line(std::path::Path::new(name));
    std::env::set_current_dir(restore).unwrap();

    let resolved = resolved.expect("a relative path naming a real file should resolve");
    assert!(resolved.is_absolute(), "{resolved:?} has to be absolute");
    assert_eq!(
        std::fs::canonicalize(&resolved).unwrap(),
        std::fs::canonicalize(&paths[0]).unwrap(),
        "it should have resolved to the file that was actually there"
    );
}

/// A `..` component is a traversal in a session file and an ordinary thing to type at a shell.
#[test]
fn a_traversing_argument_is_accepted_and_flattened() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);
    let name = paths[0].file_name().unwrap();

    let nested = dir.path().join("down");
    std::fs::create_dir(&nested).unwrap();
    let up_and_over = nested.join("..").join(name);

    let resolved =
        path::from_command_line(&up_and_over).expect("`..` from a real directory should resolve");
    assert!(
        !resolved
            .components()
            .any(|c| c == std::path::Component::ParentDir),
        "the `..` should be gone, or the session file gets one: {resolved:?}"
    );
    assert_eq!(
        std::fs::canonicalize(&resolved).unwrap(),
        std::fs::canonicalize(&paths[0]).unwrap()
    );
}

/// Whatever comes out has to be something the session boundary accepts, because that is what it
/// will be handed on the next start.
///
/// The two ways of getting this wrong are opposite, which is why the resolution is per-platform:
/// canonicalising gives a verbatim `\\?\` path on Windows, which `validate` reads as the device
/// namespace, and *not* canonicalising leaves `..` on Unix, which it reads as traversal.
#[test]
fn a_resolved_argument_survives_a_round_trip_through_the_session_file() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 1);

    let name = paths[0].file_name().unwrap();
    let nested = dir.path().join("down");
    std::fs::create_dir(&nested).unwrap();

    // The second case is the one that matters. The first version of this test used only a plain
    // path, passed on both platforms, and would have shipped a Linux build whose every traversing
    // argument was dropped from the session on restart.
    for given in [paths[0].clone(), nested.join("..").join(name)] {
        let resolved = path::from_command_line(&given).expect("a real file");
        assert!(
            path::validate(&resolved).is_ok(),
            "{given:?} resolved to {resolved:?}, which the session boundary refuses"
        );
    }
}

/// A typo says so rather than opening an empty buffer named after it.
#[test]
fn an_argument_naming_nothing_is_refused() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("not-here.md");

    assert!(matches!(
        path::from_command_line(&missing),
        Err(PathRejected::NotAFile(_))
    ));
    assert!(matches!(
        path::from_command_line(std::path::Path::new("")),
        Err(PathRejected::NotAFile(_))
    ));
}

/// The point of keeping the two apart. Loosening the command line must not loosen the session file,
/// which is the boundary the whole module exists for.
#[test]
fn the_session_boundary_is_unchanged() {
    assert!(matches!(
        path::validate(std::path::Path::new("relative/note.md")),
        Err(PathRejected::NotAbsolute(_))
    ));
    assert!(matches!(
        path::validate(std::path::Path::new(r"\\evil.example\share\note.md")),
        Err(PathRejected::NetworkOrDevice(_))
    ));
}

/// Re-reading a tab replaces both halves of what `Tab::is_modified` compares, so the buffer cannot
/// come back from a reload still looking edited.
#[test]
fn reloading_a_tab_leaves_it_unmodified() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("note.md");
    std::fs::write(&path, "# Note\n").unwrap();

    let mut workspace = Workspace::new();
    let index = workspace.add(path.clone());
    workspace.activate(index).unwrap();

    let open = workspace.active_open_mut().unwrap();
    let end = mdedit_offsets::ByteOffset::new(open.document.len_bytes());
    open.document
        .replace(mdedit_offsets::ByteSpan::new(end, end), "edited")
        .unwrap();
    assert!(workspace.tabs()[index].is_modified());

    std::fs::write(&path, "# Note\n\nrewritten\n").unwrap();
    workspace.reload_tab(index).unwrap();

    assert!(!workspace.tabs()[index].is_modified());
    assert!(
        workspace
            .active_open()
            .unwrap()
            .document
            .to_string()
            .contains("rewritten")
    );
}

/// There is nothing on disk to re-read, so this is a caller error rather than a silent no-op.
#[test]
fn reloading_an_untitled_tab_is_refused() {
    let mut workspace = Workspace::new();
    let index = workspace.add_untitled();
    assert!(workspace.reload_tab(index).is_err());
    assert!(workspace.reload_tab(99).is_err());
}

/// A tab that has never been displayed has read nothing, so reloading it would be a first read
/// dressed up as a refresh — and deferred tabs exist precisely not to do that.
#[test]
fn reloading_a_deferred_tab_is_refused() {
    let dir = TempDir::new().unwrap();
    let paths = files(&dir, 2);

    let mut workspace = Workspace::new();
    workspace.add(paths[0].clone());
    let deferred = workspace.add(paths[1].clone());
    workspace.activate(0).unwrap();

    assert!(workspace.reload_tab(deferred).is_err());
    assert_eq!(workspace.materialised_count(), 1);
}

// ---- the clean-exit marker (MDEDIT-66) ----

/// The signal that replaced "a snapshot survived". Since unsaved work now outlives an ordinary quit,
/// its presence can no longer say the last run died — this can.
#[test]
fn an_exit_records_that_it_was_clean() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    let paths = files(&dir, 1);

    let mut workspace = Workspace::new();
    workspace.add(paths[0].clone());
    workspace.save_session_on_exit(&session).unwrap();

    assert!(mdedit_session::last_run_ended_cleanly(&session));
}

/// A session written mid-run and then lost to a crash must still read as an unclean end, which it
/// was — so only the exit path sets the marker.
#[test]
fn an_ordinary_save_does_not_claim_a_clean_exit() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    let paths = files(&dir, 1);

    let mut workspace = Workspace::new();
    workspace.add(paths[0].clone());
    workspace.save_session(&session).unwrap();

    assert!(!mdedit_session::last_run_ended_cleanly(&session));
}

/// Every way of not knowing answers "not cleanly". A file that cannot be read cannot vouch for
/// anything, and being wrong in this direction costs one sentence in a log rather than lost work.
#[test]
fn anything_unreadable_is_not_a_clean_exit() {
    let dir = TempDir::new().unwrap();

    assert!(
        !mdedit_session::last_run_ended_cleanly(&dir.path().join("nothing-here.json")),
        "a first run has no session at all"
    );

    let malformed = dir.path().join("malformed.json");
    std::fs::write(&malformed, "{not json").unwrap();
    assert!(!mdedit_session::last_run_ended_cleanly(&malformed));

    let empty = dir.path().join("empty.json");
    std::fs::write(&empty, "").unwrap();
    assert!(!mdedit_session::last_run_ended_cleanly(&empty));
}

/// A session written by an older build has no marker at all, and must read as unclean rather than
/// failing to parse.
#[test]
fn a_session_without_the_marker_reads_as_unclean() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    std::fs::write(&session, r#"{"version":1,"active":null,"tabs":[]}"#).unwrap();

    assert!(!mdedit_session::last_run_ended_cleanly(&session));
}

/// And the marker must not stop such a session restoring — it is one optional field, not a format
/// change.
#[test]
fn a_session_without_the_marker_still_restores() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    std::fs::write(&session, r#"{"version":1,"active":null,"tabs":[]}"#).unwrap();

    assert!(Workspace::restore_session(&session).is_ok());
}

/// The bug a by-hand check caught and no test did.
///
/// The marker is only written on exit, so a run that is *killed* never rewrites it — and the
/// previous clean exit's `true` would survive to tell the next launch that a crash recovery was an
/// ordinary restore. Clearing it at startup is what makes it describe the run in progress rather
/// than the one before.
#[test]
fn a_clean_exit_marker_does_not_outlive_the_next_launch() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    let paths = files(&dir, 1);

    let mut workspace = Workspace::new();
    workspace.add(paths[0].clone());

    // A run that ended properly.
    workspace.save_session_on_exit(&session).unwrap();
    assert!(mdedit_session::last_run_ended_cleanly(&session));

    // The next launch reads it and then clears it, which is what `main` does.
    workspace.save_session(&session).unwrap();

    // This run is now killed — nothing else is written. The marker must not still claim the last
    // ending was clean, because the last ending was this one.
    assert!(
        !mdedit_session::last_run_ended_cleanly(&session),
        "a killed run must not inherit the previous clean exit's marker"
    );
}

// ---- the session file is state, and a truncated one loses more than itself (MDEDIT-106) ----

/// Every state file this crate writes goes through a temporary file and a rename. The session file
/// was the one that did not, while `recent.rs` claimed in a comment that it did.
///
/// A truncated `session.json` reads as `Malformed`, which loses the tab list **and** the
/// `clean_exit` marker — so the next launch also reports the previous quit as a crash. What this
/// pins is the mechanism: a temporary file appears beside the target and is renamed over it, so
/// there is no window in which the target is a partial file.
#[test]
fn writing_a_session_leaves_no_temporary_behind_and_replaces_the_old_one_whole() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session.json");
    std::fs::write(&session, "{\"previous\": true}").unwrap();

    let mut workspace = Workspace::new();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Notes\n").unwrap();
    workspace.add(path.clone());
    workspace.save_session(&session).unwrap();

    let written = std::fs::read_to_string(&session).unwrap();
    assert!(
        written.contains("notes.md"),
        "the new session should be there whole: {written}"
    );
    assert!(
        !written.contains("previous"),
        "and the old one gone, not partly overwritten"
    );

    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e == "tmp")
        })
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

/// And the directory is created on the way, which is what a first run needs.
#[test]
fn a_session_can_be_written_into_a_directory_that_does_not_exist_yet() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("never").join("made").join("session.json");

    Workspace::new().save_session(&session).unwrap();

    assert!(session.exists());
}
