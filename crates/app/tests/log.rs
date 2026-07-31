//! The log file, the rotation, and the policy about what may go in it.
//!
//! The panic hook is deliberately not tested here: installing one is process-global and taking it
//! down again is not, so a test that installed it would change how every other test in this binary
//! reports a failure. It is verified by hand, and the PR says so.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::log;

/// The log lives beside the session file, not beside the config — it is state, not settings.
#[test]
fn the_log_sits_with_the_session_not_the_config() {
    let (Some(log), Some(session), Some(config)) = (
        log::default_log_path(),
        mdedit_session::default_session_path(),
        mdedit_app::config::default_config_path(),
    ) else {
        // None resolve without the environment variables they read; nothing to compare.
        return;
    };

    assert_eq!(
        log.parent(),
        session.parent(),
        "state belongs with state: a crash log is not a setting"
    );
    assert_ne!(log.parent(), config.parent());
    assert!(log.ends_with("mdedit.log"));
}

/// A log that cannot be opened must not be a reason to fail to start, and `write` must survive
/// being called before `start` — which is exactly what happens for anything that goes wrong while
/// resolving the state directory.
#[test]
fn writing_without_a_log_file_is_harmless() {
    log::write("this test never calls start(), so there is no sink");
    log::write("and a second line still does nothing untoward");
}

/// The `log_error!` macro exists so twenty-one call sites could be renamed rather than rewritten;
/// it must take exactly what `eprintln!` took.
#[test]
fn the_macro_takes_format_arguments() {
    let path = std::path::Path::new("some/file.md");
    mdedit_app::log_error!("could not open {}: {}", path.display(), "a reason");
    mdedit_app::log_error!("no arguments at all");
}
