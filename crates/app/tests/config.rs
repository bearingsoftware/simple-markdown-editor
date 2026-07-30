//! The settings file, treated as what it is: untrusted input somebody hand-edited.
//!
//! The behaviour that matters is what happens when it is *wrong*. A misspelled key, a stray comma, a
//! file from a newer build, a file that is not JSON at all — none of them may stop the editor
//! opening, because a setting being wrong is a smaller problem than an editor that will not start.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::config::Config;
use tempfile::TempDir;

fn written(dir: &TempDir, contents: &str) -> std::path::PathBuf {
    let path = dir.path().join("config.json");
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn no_file_at_all_is_the_ordinary_case() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("nothing-here.json");
    assert_eq!(Config::load_from(&missing), Config::default());
    assert!(
        !Config::default().line_numbers,
        "line numbers are off by default"
    );
}

#[test]
fn a_setting_is_read() {
    let dir = TempDir::new().unwrap();
    let path = written(&dir, r#"{"line_numbers": true}"#);
    assert!(Config::load_from(&path).line_numbers);
}

#[test]
fn an_empty_object_is_the_defaults() {
    let dir = TempDir::new().unwrap();
    let path = written(&dir, "{}");
    assert_eq!(Config::load_from(&path), Config::default());
}

/// A file written by a newer build has to start an older one.
#[test]
fn unknown_keys_are_ignored() {
    let dir = TempDir::new().unwrap();
    let path = written(
        &dir,
        r#"{"line_numbers": true, "something_from_the_future": 42, "nested": {"a": [1, 2]}}"#,
    );
    assert!(
        Config::load_from(&path).line_numbers,
        "the key it did understand should still have been read"
    );
}

/// The important one: a stray comma must not stop the editor opening.
#[test]
fn a_malformed_file_falls_back_to_the_defaults() {
    let dir = TempDir::new().unwrap();
    for broken in [
        r#"{"line_numbers": true,}"#,
        "{not json at all",
        "",
        "null",
        "[1, 2, 3]",
        r#"{"line_numbers": "yes please"}"#,
    ] {
        let path = written(&dir, broken);
        assert_eq!(
            Config::load_from(&path),
            Config::default(),
            "{broken:?} should have fallen back rather than failed"
        );
    }
}

/// A bound on attacker-controlled input, not a judgement about how big a config anyone needs.
#[test]
fn an_enormous_file_is_refused_rather_than_read() {
    let dir = TempDir::new().unwrap();
    let mut huge = String::from(r#"{"line_numbers": true, "padding": ""#);
    huge.push_str(&"x".repeat(128 * 1024));
    huge.push_str(r#""}"#);

    let path = written(&dir, &huge);
    assert_eq!(
        Config::load_from(&path),
        Config::default(),
        "a file past the cap should not be read at all"
    );
}

/// Settings and state are different things and live in different places: a session is what you had
/// open, and belongs in the state directory.
#[test]
fn the_config_path_is_not_the_session_path() {
    let (Some(config), Some(session)) = (
        mdedit_app::config::default_config_path(),
        mdedit_session::default_session_path(),
    ) else {
        // Neither resolves without the environment variables they read; nothing to compare.
        return;
    };
    assert_ne!(config, session);
    assert!(config.ends_with("SimpleMarkdownEditor/config.json"));
}
