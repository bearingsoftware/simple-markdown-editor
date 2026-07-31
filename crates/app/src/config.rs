//! The settings file, and the first setting in it.
//!
//! # It is untrusted input
//!
//! `AGENTS.md` is explicit that the editor's own config and session files are untrusted, and names
//! three consecutive Notepad++ bypasses in 2026 as what happens when that slips. The rules here
//! follow from that, and the first one is the strongest:
//!
//! - **No setting may name a path.** Not "validate paths carefully" — none at all. That removes the
//!   entire class the standard warns about rather than defending against it. When a setting genuinely
//!   needs a path, that is a decision for its ticket, with the normalisation written down.
//! - **Unknown keys are ignored**, so a file written by a newer build still starts an older one.
//! - **A malformed file is reported and the defaults are used.** Refusing to start because a setting
//!   is misspelled is worse than the setting being wrong.
//! - **A size cap**, as the session file has, so a pathological file cannot be read into memory
//!   unbounded.
//! - **Read once, at startup, and never written back.** The editor does not rewrite a file somebody
//!   hand-edited.
//!
//! # Why JSON
//!
//! `serde_json` is already in the workspace for the session file, so this costs no new dependency.
//! The honest cost is that JSON has no comments, and a file meant to be hand-edited is exactly where
//! comments earn their keep. TOML would be nicer to live with and costs a dependency; the trade is
//! worth revisiting the moment this grows past a handful of keys, not before.

use std::path::PathBuf;

use serde::Deserialize;

/// Largest config file that will be read, in bytes. Generous for something with one key in it.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Everything the editor lets you choose.
///
/// Every field has a default, so an absent file, an empty one, and one that sets nothing all behave
/// identically — which is what makes "no config" the ordinary case rather than a special one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Show source-line numbers in a gutter down the left.
    ///
    /// Off by default: this is a WYSIWYG Markdown editor, and the reference implementations —
    /// Typora, Obsidian and `MarkText` — show none, because the rendered view deliberately is not the
    /// source. On for people who talk about their documents by line number, which is most people
    /// whose Markdown lives in git.
    pub line_numbers: bool,
}

impl Config {
    /// Reads the config, or returns the defaults.
    ///
    /// Never fails. Anything that goes wrong is reported once and the defaults are used, because an
    /// editor that will not open because of a stray comma is worse than one with a setting off.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = default_config_path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Reads the config at a given path. Public so a test can point it somewhere temporary.
    #[must_use]
    pub fn load_from(path: &std::path::Path) -> Self {
        let Ok(metadata) = std::fs::metadata(path) else {
            // No file is the ordinary case, not a problem worth mentioning.
            return Self::default();
        };
        if metadata.len() > MAX_CONFIG_BYTES {
            crate::log_error!(
                "ignoring {}: {} bytes is larger than the {MAX_CONFIG_BYTES} byte limit",
                path.display(),
                metadata.len()
            );
            return Self::default();
        }

        let Ok(bytes) = std::fs::read(path) else {
            crate::log_error!("could not read {}; using defaults", path.display());
            return Self::default();
        };

        match serde_json::from_slice(&bytes) {
            Ok(config) => config,
            Err(error) => {
                crate::log_error!("{} is malformed, using defaults: {error}", path.display());
                Self::default()
            }
        }
    }
}

/// Where the config lives.
///
/// Deliberately not beside the session file. A session is *state* — what you had open, where the
/// caret was — and belongs in the state directory; this is *settings*, which you choose and might
/// want to copy between machines.
#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);

    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

    base.map(|base| base.join("SimpleMarkdownEditor").join("config.json"))
}
