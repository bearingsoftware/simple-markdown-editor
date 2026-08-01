//! Crash recovery: what survives, what is refused, and what must not linger.
//!
//! The lifecycle claims matter more than the happy path. A snapshot that outlives the work it
//! protects offers to restore something already on disk, which teaches people to dismiss the offer —
//! so "deleted on save" and "cleared on clean exit" are as much the feature as "written at all".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_session::PathRejected;
use mdedit_session::snapshot::Store;
use tempfile::TempDir;

fn store(dir: &TempDir) -> Store {
    Store::in_directory(dir.path().join("recovery"))
}

/// A real file, because the manifest's paths go through the session boundary on the way back and
/// that boundary requires a path that exists.
fn a_file(dir: &TempDir, name: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, "on disk\n").unwrap();
    path
}

#[test]
fn a_snapshot_survives_and_comes_back() {
    let dir = TempDir::new().unwrap();
    let path = a_file(&dir, "notes.md");

    let mut writer = store(&dir);
    writer.write(1, Some(&path), "unsaved work\n").unwrap();

    // A separate store, as the next run's would be.
    let mut reader = store(&dir);
    let (recovered, skipped) = reader.read_all();

    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].text, "unsaved work\n");
    assert_eq!(recovered[0].path.as_deref(), Some(path.as_path()));
}

/// The case with the most to lose: a buffer that was never saved anywhere has no path to identify
/// it, which is why snapshots are addressed by their own id.
#[test]
fn an_untitled_buffer_comes_back_too() {
    let dir = TempDir::new().unwrap();
    let mut writer = store(&dir);
    writer.write(7, None, "never saved\n").unwrap();

    let (recovered, skipped) = store(&dir).read_all();
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].path.is_none());
    assert_eq!(recovered[0].text, "never saved\n");
}

#[test]
fn writing_twice_replaces_rather_than_accumulates() {
    let dir = TempDir::new().unwrap();
    let mut writer = store(&dir);
    writer.write(1, None, "first\n").unwrap();
    writer.write(1, None, "second\n").unwrap();

    let (recovered, _) = store(&dir).read_all();
    assert_eq!(recovered.len(), 1, "one buffer, one snapshot");
    assert_eq!(recovered[0].text, "second\n");
}

/// Saved work no longer needs protecting, and a snapshot that lingers would offer to restore what
/// is already on disk.
#[test]
fn forgetting_removes_it() {
    let dir = TempDir::new().unwrap();
    let mut writer = store(&dir);
    writer.write(1, None, "a\n").unwrap();
    writer.write(2, None, "b\n").unwrap();
    writer.forget(1).unwrap();

    let (recovered, _) = store(&dir).read_all();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].text, "b\n");
}

/// The signal the whole feature rests on: something left behind means the last run did not end here.
#[test]
fn a_clean_exit_leaves_nothing_to_recover() {
    let dir = TempDir::new().unwrap();
    let mut writer = store(&dir);
    writer.write(1, None, "a\n").unwrap();
    writer.write(2, None, "b\n").unwrap();
    writer.clear();

    let (recovered, skipped) = store(&dir).read_all();
    assert!(recovered.is_empty(), "a clean exit must leave nothing");
    assert!(skipped.is_empty(), "and nothing to complain about");
}

/// The manifest is a file this process wrote into a directory something else may reach, which is the
/// session file's position exactly — so it gets the session file's boundary.
#[test]
fn a_hostile_path_in_the_manifest_is_refused() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    std::fs::create_dir_all(&recovery).unwrap();
    std::fs::write(recovery.join("1.snapshot"), "content\n").unwrap();
    std::fs::write(
        recovery.join("manifest.json"),
        r#"{"entries":[{"id":1,"path":"\\\\evil.example\\share\\x.md"}]}"#,
    )
    .unwrap();

    let (recovered, skipped) = store(&dir).read_all();
    assert!(recovered.is_empty(), "a UNC path must not be opened");
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].contains("refused"), "{skipped:?}");

    // And the boundary itself is unchanged — this test would be worthless if `validate` had been
    // loosened to make the feature work.
    assert!(matches!(
        mdedit_session::path::validate(std::path::Path::new(r"\\evil.example\share\x.md")),
        Err(PathRejected::NetworkOrDevice(_))
    ));
}

/// A recovery file must never be a reason the editor will not start.
#[test]
fn a_malformed_manifest_is_reported_not_fatal() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    std::fs::create_dir_all(&recovery).unwrap();
    std::fs::write(recovery.join("manifest.json"), "{not json at all").unwrap();

    let (recovered, skipped) = store(&dir).read_all();
    assert!(recovered.is_empty());
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].contains("malformed"), "{skipped:?}");
}

/// A manifest naming a snapshot whose file has gone — deleted by hand, or a half-finished clear.
#[test]
fn a_missing_snapshot_file_is_skipped() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    std::fs::create_dir_all(&recovery).unwrap();
    std::fs::write(recovery.join("manifest.json"), r#"{"entries":[{"id":9}]}"#).unwrap();

    let (recovered, skipped) = store(&dir).read_all();
    assert!(recovered.is_empty());
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].contains("missing"), "{skipped:?}");
}

#[test]
fn nothing_at_all_is_the_ordinary_case() {
    let dir = TempDir::new().unwrap();
    let (recovered, skipped) = store(&dir).read_all();
    assert!(recovered.is_empty());
    assert!(skipped.is_empty(), "a first run must not complain");
}

/// Written temp-then-rename, so an interrupted write leaves the previous snapshot intact rather
/// than a half-written one. Checked by proving no stray temporary is left behind.
#[test]
fn writing_leaves_no_temporary_behind() {
    let dir = TempDir::new().unwrap();
    let mut writer = store(&dir);
    writer.write(1, None, "content\n").unwrap();

    let strays: Vec<_> = std::fs::read_dir(dir.path().join("recovery"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(strays.is_empty(), "{strays:?}");
}

/// The store must survive a directory it cannot write to, because the alternative is an editor that
/// will not run when its state directory is read-only.
#[test]
fn an_unwritable_directory_is_an_error_not_a_panic() {
    let mut writer = Store::in_directory(std::path::PathBuf::from("\0invalid"));
    assert!(writer.write(1, None, "x").is_err());
}
