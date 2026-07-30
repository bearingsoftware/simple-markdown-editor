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
