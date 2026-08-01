//! The recently-opened list, and the boundary it shares with the session file.
//!
//! The ordering and the cap are the easy half. The half that matters is that a path read back out of
//! this file is checked before anything can act on it — it is a file this process wrote into a
//! directory something else may reach, which is exactly what `path::validate` exists for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use mdedit_session::PathRejected;
use mdedit_session::recent::{MAX_ENTRIES, Recent};
use tempfile::TempDir;

fn list(dir: &TempDir) -> Recent {
    Recent::at(dir.path().join("recent.json"))
}

/// A real file at an absolute path.
///
/// It has to exist: `validate` is the boundary this list goes through in both directions, and its
/// last rule is that the path names a regular file. That is not incidental — it is what makes a
/// deleted file fall out of the menu on its own, which
/// [`a_deleted_file_drops_out_of_the_list`] pins.
fn a_path(dir: &TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, "content\n").unwrap();
    path
}

#[test]
fn the_most_recent_is_first() {
    let dir = TempDir::new().unwrap();
    let mut recent = list(&dir);
    recent.record(&a_path(&dir, "one.md"));
    recent.record(&a_path(&dir, "two.md"));

    assert_eq!(
        recent.paths(),
        [a_path(&dir, "two.md"), a_path(&dir, "one.md")]
    );
}

/// A list that logged every open would say "what I have opened", which is not what anyone reads it
/// for.
#[test]
fn opening_something_again_moves_it_rather_than_repeating_it() {
    let dir = TempDir::new().unwrap();
    let mut recent = list(&dir);
    recent.record(&a_path(&dir, "one.md"));
    recent.record(&a_path(&dir, "two.md"));
    recent.record(&a_path(&dir, "one.md"));

    assert_eq!(
        recent.paths(),
        [a_path(&dir, "one.md"), a_path(&dir, "two.md")]
    );
}

#[test]
fn it_stops_at_the_cap() {
    let dir = TempDir::new().unwrap();
    let mut recent = list(&dir);
    for index in 0..MAX_ENTRIES + 5 {
        recent.record(&a_path(&dir, &format!("note-{index}.md")));
    }

    assert_eq!(recent.paths().len(), MAX_ENTRIES);
    assert_eq!(
        recent.paths()[0],
        a_path(&dir, &format!("note-{}.md", MAX_ENTRIES + 4)),
        "the newest must survive the truncation, not the oldest"
    );
}

/// Written through on every change rather than on exit, because the run this list is most useful
/// after is the one that did not reach an exit.
#[test]
fn it_survives_a_restart() {
    let dir = TempDir::new().unwrap();
    let mut recent = list(&dir);
    recent.record(&a_path(&dir, "one.md"));
    recent.record(&a_path(&dir, "two.md"));
    drop(recent);

    assert_eq!(
        list(&dir).paths(),
        [a_path(&dir, "two.md"), a_path(&dir, "one.md")]
    );
}

#[test]
fn forgetting_removes_one_and_keeps_the_rest() {
    let dir = TempDir::new().unwrap();
    let mut recent = list(&dir);
    recent.record(&a_path(&dir, "one.md"));
    recent.record(&a_path(&dir, "two.md"));
    recent.forget(&a_path(&dir, "two.md"));

    assert_eq!(recent.paths(), [a_path(&dir, "one.md")]);
    assert_eq!(list(&dir).paths(), [a_path(&dir, "one.md")]);
}

/// The boundary. A menu of paths that get opened on click is a good way to be handed a hostile one,
/// and this file is one the process wrote into a directory something else may be able to write.
#[test]
fn a_hostile_entry_is_refused_on_the_way_back_in() {
    let dir = TempDir::new().unwrap();
    let survivor = a_path(&dir, "fine.md");
    let file = dir.path().join("recent.json");
    std::fs::write(
        &file,
        format!(
            r#"{{"paths":["\\\\evil.example\\share\\x.md","relative/note.md","{}"]}}"#,
            survivor.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();

    let recent = Recent::at(file);
    assert_eq!(
        recent.paths(),
        [survivor],
        "only the absolute local path should survive"
    );

    // And the boundary itself is unchanged — this test is worthless if `validate` was loosened to
    // make the feature work.
    assert!(matches!(
        mdedit_session::path::validate(Path::new(r"\\evil.example\share\x.md")),
        Err(PathRejected::NetworkOrDevice(_))
    ));
    assert!(matches!(
        mdedit_session::path::validate(Path::new("relative/note.md")),
        Err(PathRejected::NotAbsolute(_))
    ));
}

/// Refused on the way in as well as on the way out, so the list cannot be used to launder a path
/// past the boundary by storing it now and reading it back later.
#[test]
fn a_hostile_path_is_not_recorded_either() {
    let dir = TempDir::new().unwrap();
    let mut recent = list(&dir);
    recent.record(Path::new(r"\\evil.example\share\x.md"));
    recent.record(Path::new("relative/note.md"));

    assert!(recent.paths().is_empty());
}

#[test]
fn a_malformed_file_is_an_empty_list_not_a_failure() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("recent.json");
    std::fs::write(&file, "{not json").unwrap();

    assert!(Recent::at(file).paths().is_empty());
}

#[test]
fn nothing_there_is_the_ordinary_case() {
    let dir = TempDir::new().unwrap();
    assert!(list(&dir).paths().is_empty());
}

/// A crafted file cannot make the menu unbounded, however many entries it claims.
#[test]
fn a_file_claiming_hundreds_of_entries_is_capped() {
    let dir = TempDir::new().unwrap();
    let entries = dir.path().join("entries");
    std::fs::create_dir(&entries).unwrap();

    let quoted: Vec<String> = (0..500)
        .map(|index| {
            let path = entries.join(format!("note-{index}.md"));
            std::fs::write(&path, "x").unwrap();
            // JSON needs the Windows separators escaped, and `to_string_lossy` gives them raw.
            format!("\"{}\"", path.display().to_string().replace('\\', "\\\\"))
        })
        .collect();

    let file = dir.path().join("recent.json");
    std::fs::write(&file, format!(r#"{{"paths":[{}]}}"#, quoted.join(","))).unwrap();

    assert_eq!(Recent::at(file).paths().len(), MAX_ENTRIES);
}

/// The menu never offers a file that is not there, and nothing had to remember to prune it: the
/// boundary's last rule is that the path names a regular file, so a deleted one simply fails it.
#[test]
fn a_deleted_file_drops_out_of_the_list() {
    let dir = TempDir::new().unwrap();
    let kept = a_path(&dir, "kept.md");
    let deleted = a_path(&dir, "deleted.md");

    let mut recent = list(&dir);
    recent.record(&kept);
    recent.record(&deleted);
    drop(recent);

    std::fs::remove_file(&deleted).unwrap();
    assert_eq!(list(&dir).paths(), [kept]);
}

/// A list with no file behind it still works in memory. It is what an editor gets when there is
/// nowhere to store state, which is the same condition under which there is no session file.
#[test]
fn a_list_with_nowhere_to_live_still_remembers_within_the_run() {
    let dir = TempDir::new().unwrap();
    let mut recent = Recent::default();
    recent.record(&a_path(&dir, "one.md"));
    assert_eq!(recent.paths().len(), 1);
}
