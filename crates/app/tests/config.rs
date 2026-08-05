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

// ---- the write path (MDEDIT-60) ----

use mdedit_app::config::Settings;
use mdedit_app::view::Chrome;

fn open(dir: &TempDir) -> Settings {
    Settings::at(dir.path().join("config.json"))
}

fn read_back(dir: &TempDir) -> serde_json::Value {
    let bytes = std::fs::read(dir.path().join("config.json")).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn a_changed_setting_is_written_and_read_back() {
    let dir = TempDir::new().unwrap();

    let mut settings = open(&dir);
    settings.update(|config| config.chrome = Chrome::Pane);
    assert!(settings.flush());

    assert_eq!(open(&dir).config().chrome, Chrome::Pane);
}

/// The load-bearing rule. A typed-struct rewrite silently deletes the future — a key from a newer
/// build, or one somebody added by hand, must come back out untouched.
#[test]
fn unknown_keys_survive_a_write() {
    let dir = TempDir::new().unwrap();
    written(
        &dir,
        r#"{"line_numbers": true, "something_from_the_future": {"nested": [1, 2, 3]}}"#,
    );

    let mut settings = open(&dir);
    settings.update(|config| config.chrome = Chrome::Pane);
    settings.flush();

    let json = read_back(&dir);
    assert_eq!(
        json["something_from_the_future"]["nested"],
        serde_json::json!([1, 2, 3]),
        "the unknown key should have passed straight through"
    );
    assert_eq!(json["line_numbers"], serde_json::json!(true));
    assert_eq!(json["chrome"], serde_json::json!("pane"));
}

/// Two changes in a row must merge onto what was actually written, not onto what was read at
/// startup — otherwise the second write undoes the first.
#[test]
fn a_second_write_keeps_the_first() {
    let dir = TempDir::new().unwrap();

    let mut settings = open(&dir);
    settings.update(|config| config.chrome = Chrome::Pane);
    settings.flush();
    settings.update(|config| config.line_numbers = true);
    settings.flush();

    let reloaded = open(&dir);
    assert_eq!(reloaded.config().chrome, Chrome::Pane);
    assert!(reloaded.config().line_numbers);
}

/// Debounced: a run of rapid toggles is one write, and a flush with nothing to say writes nothing.
#[test]
fn nothing_is_written_without_a_change() {
    let dir = TempDir::new().unwrap();
    let mut settings = open(&dir);

    assert!(
        !settings.flush(),
        "a flush with no change must write nothing"
    );
    assert!(!dir.path().join("config.json").exists());

    settings.update(|config| config.chrome = Chrome::Pane);
    settings.update(|config| config.chrome = Chrome::Strip);
    assert!(
        settings.is_dirty(),
        "changed and changed back is still a change as far as the file is concerned"
    );
    assert!(settings.flush());
    assert!(!settings.flush(), "and the second flush has nothing to do");
}

/// Setting something to what it already was is not a change, so it does not schedule a write.
#[test]
fn setting_a_value_to_itself_is_not_a_change() {
    let dir = TempDir::new().unwrap();
    let mut settings = open(&dir);

    settings.update(|config| config.chrome = Chrome::Strip);
    assert!(!settings.is_dirty());
}

/// The one case where unknown keys cannot be preserved, stated in the module docs and pinned here:
/// there was nothing valid to preserve.
#[test]
fn a_malformed_file_is_replaced_with_what_this_build_knows() {
    let dir = TempDir::new().unwrap();
    written(&dir, "{not json at all");

    let mut settings = open(&dir);
    assert_eq!(settings.config(), mdedit_app::config::Config::default());

    settings.update(|config| config.chrome = Chrome::Pane);
    settings.flush();

    let json = read_back(&dir);
    assert_eq!(json["chrome"], serde_json::json!("pane"));
    assert!(json.get("line_numbers").is_some());
}

/// Temp-then-rename, checked by proving no stray temporary is left behind.
#[test]
fn writing_leaves_no_temporary_behind() {
    let dir = TempDir::new().unwrap();
    let mut settings = open(&dir);
    settings.update(|config| config.chrome = Chrome::Pane);
    settings.flush();

    let strays: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(strays.is_empty(), "{strays:?}");
}

/// Settings with nowhere to live still work in memory — the same position the editor is in when
/// there is no config directory at all.
#[test]
fn settings_with_no_file_still_hold_a_choice() {
    let mut settings = Settings::nowhere();
    settings.update(|config| config.chrome = Chrome::Pane);

    assert_eq!(settings.config().chrome, Chrome::Pane);
    assert!(
        !settings.flush(),
        "with nowhere to write, it writes nothing"
    );
}

/// The file is untrusted input, and a zoom of zero is a window with nothing in it rather than a
/// preference.
#[test]
fn a_zoom_out_of_range_is_clamped() {
    let dir = TempDir::new().unwrap();

    written(&dir, r#"{"zoom": 0}"#);
    assert!(Config::load_from(&dir.path().join("config.json")).zoom >= 0.5);

    written(&dir, r#"{"zoom": 1000}"#);
    assert!(Config::load_from(&dir.path().join("config.json")).zoom <= 4.0);

    written(&dir, r#"{"zoom": -3}"#);
    assert!(Config::load_from(&dir.path().join("config.json")).zoom >= 0.5);
}

/// And it cannot be set out of range from inside the editor either.
#[test]
fn a_zoom_set_out_of_range_is_clamped_too() {
    let mut settings = Settings::nowhere();
    settings.update(|config| config.zoom = 100.0);
    assert!(settings.config().zoom <= 4.0);
}

/// The rule that has not changed, and must not: no setting may name a path.
///
/// **This test used to prove nothing** (MDEDIT-110). Its fixture contained invalid JSON escapes —
/// `\s` and `\x` — so `load_from` took the malformed-file fallback and returned defaults whether or
/// not a path-typed field existed. A named security rule with an empty guard behind it.
///
/// The fixture is valid JSON now, and it sets a real key as well, so the file provably parsed: if it
/// did not, `zoom` would not have come back and the unknown-key half would be asserting the fallback
/// again.
#[test]
fn a_config_naming_a_path_sets_nothing() {
    let dir = TempDir::new().unwrap();
    written(
        &dir,
        r#"{"session_path": "\\\\evil.example\\share", "startup_file": "C:\\tmp\\x.md", "zoom": 2.0}"#,
    );

    let config = Config::load_from(&dir.path().join("config.json"));
    assert!(
        (config.zoom - 2.0).abs() < f32::EPSILON,
        "the file has to have parsed, or the rest of this test is about the fallback"
    );
    // The two path keys are unknown, so they set nothing — which is exactly the protection. The rule
    // is that no *field* exists for them, and the only way to break it is to add one.
    assert_eq!(
        config,
        Config {
            zoom: 2.0,
            ..Config::default()
        }
    );
}

/// And the rule stated structurally, which is what actually holds it: these are every field there
/// is, so adding a path-typed one is a change to this list rather than something nobody notices.
#[test]
fn the_config_has_exactly_these_fields_and_none_of_them_is_a_path() {
    let value = serde_json::to_value(Config::default()).expect("a config serialises");
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("a config is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();

    assert_eq!(
        keys,
        ["chrome", "line_numbers", "theme", "zoom"],
        "a field was added or renamed. If it names a path, see the module docs: the config file is \
         read-only by design and untrusted by rule, and a path in it is a path this process would \
         open on somebody else's say-so"
    );
}
