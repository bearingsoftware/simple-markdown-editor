//! Noticing that a file has been written since it was read.
//!
//! `disk_state` is a cheap approximation on purpose — a modification time and a length, never a hash
//! — so these pin the direction it is allowed to be wrong in. It may say "changed" about a file that
//! has not; it must not say "unchanged" about one that has.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_file::{DiskState, load};
use tempfile::TempDir;

/// A pause before rewriting, because filesystem timestamps are coarse. See `Stamp`.
fn externally_write(path: &std::path::Path, contents: &str) {
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(path, contents).unwrap();
}

#[test]
fn an_untouched_file_is_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "hello\n").unwrap();

    let file = load(&path).unwrap();
    assert_eq!(mdedit_file::disk_state(&path, &file), DiskState::Unchanged);
}

#[test]
fn a_rewritten_file_has_changed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "hello\n").unwrap();

    let file = load(&path).unwrap();
    externally_write(&path, "hello, and more\n");
    assert_eq!(mdedit_file::disk_state(&path, &file), DiskState::Changed);
}

/// The case a length alone would miss.
#[test]
fn a_same_length_rewrite_is_caught_by_the_time() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "aaaaa\n").unwrap();

    let file = load(&path).unwrap();
    // A whole second, because that is the granularity a filesystem is allowed to have. It costs the
    // suite a second and it is the only honest way to assert this: a shorter pause would be
    // asserting that this machine's filesystem is fine-grained, not that the check works.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&path, "bbbbb\n").unwrap();

    assert_eq!(mdedit_file::disk_state(&path, &file), DiskState::Changed);
}

#[test]
fn a_deleted_file_is_gone() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "hello\n").unwrap();

    let file = load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(mdedit_file::disk_state(&path, &file), DiskState::Gone);
}

/// Re-reading takes a fresh stamp, which is what makes a save not look like somebody else's write.
#[test]
fn reloading_takes_a_new_stamp() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "hello\n").unwrap();

    let file = load(&path).unwrap();
    externally_write(&path, "hello, and more\n");
    assert_eq!(mdedit_file::disk_state(&path, &file), DiskState::Changed);

    let file = mdedit_file::reload(&path, &file).unwrap();
    assert_eq!(mdedit_file::disk_state(&path, &file), DiskState::Unchanged);
}

#[test]
fn a_stamp_is_taken_on_load() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "hello\n").unwrap();

    assert!(
        load(&path).unwrap().stamp().is_some(),
        "without a stamp every later comparison says unchanged, and the feature does nothing"
    );
}
