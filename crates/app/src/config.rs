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
//!
//! # It is written back now, and the rule that makes that safe
//!
//! This module used to say "read once, at startup, and never written back — the editor does not
//! rewrite a file somebody hand-edited". Four features stalled on that: the strip/pane choice, zoom,
//! word wrap, and the line-numbers toggle, each recorded as per-run "until the settings story grows
//! a write path". MDEDIT-60 grew it, and kept the spirit of the old rule with three:
//!
//! - **Unknown keys survive a round trip.** The file is read into a `serde_json::Map` *as well as*
//!   the typed [`Config`]; a write merges the changed fields into that map and writes the map. So a
//!   key from a newer build, or one this build never heard of, passes through untouched. This is the
//!   load-bearing rule — a typed-struct rewrite silently deletes the future, which is precisely the
//!   thing that would make somebody who hand-edits this file angry.
//! - **Writes are atomic**, temp then rename, as the session and snapshot files are. A crash
//!   mid-write leaves the old settings rather than half of the new ones.
//! - **Written when something changes, debounced onto the idle timer** — not on exit. MDEDIT-54 is
//!   the argument: a run that ends in a crash never reaches its exit path, and a setting the user
//!   changed an hour ago should not be among the things lost.
//!
//! The one case where unknown keys *cannot* be preserved is a malformed file: there was nothing
//! valid to preserve, so a settings change after one writes a file containing only what this build
//! knows. Stated rather than hidden, because it is the one way this can lose something.
//!
//! # Why JSON
//!
//! `serde_json` is already in the workspace for the session file, so this costs no new dependency.
//! The honest cost is that JSON has no comments, and a file meant to be hand-edited is exactly where
//! comments earn their keep. TOML would be nicer to live with and costs a dependency; the trade is
//! worth revisiting the moment this grows past a handful of keys, not before.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::view::Chrome;

/// Largest config file that will be read, in bytes. Generous for something with one key in it.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Everything the editor lets you choose.
///
/// Every field has a default, so an absent file, an empty one, and one that sets nothing all behave
/// identically — which is what makes "no config" the ordinary case rather than a special one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Show source-line numbers in a gutter down the left.
    ///
    /// Off by default: this is a WYSIWYG Markdown editor, and the reference implementations —
    /// Typora, Obsidian and `MarkText` — show none, because the rendered view deliberately is not the
    /// source. On for people who talk about their documents by line number, which is most people
    /// whose Markdown lives in git.
    pub line_numbers: bool,

    /// Whether the open files show as a strip of tabs or a pane down the left.
    ///
    /// Ctrl+B swaps them. Before MDEDIT-60 the editor started on the strip every time, whatever you
    /// had chosen.
    pub chrome: Chrome,

    /// How much larger or smaller than normal to draw the text.
    ///
    /// A multiplier rather than a point size, so it scales headings with the body — the sizes are a
    /// ramp off one number (see [`crate::view::Theme::heading_size`]) and a zoom that only moved the
    /// body would flatten the ramp. Clamped on read: a file saying `0` or `-3` must not produce an
    /// editor that draws nothing.
    pub zoom: f32,
}

/// Narrowest and widest the text may be scaled.
///
/// Not preferences — bounds on a number read from a file. Below the floor the text is unreadable and
/// above the ceiling a single glyph fills the window, and either would look like the editor is
/// broken rather than like a setting is wrong.
const MIN_ZOOM: f32 = 0.5;
/// See [`MIN_ZOOM`].
const MAX_ZOOM: f32 = 4.0;

impl Default for Config {
    fn default() -> Self {
        Self {
            line_numbers: false,
            chrome: Chrome::default(),
            zoom: 1.0,
        }
    }
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

        match serde_json::from_slice::<Self>(&bytes) {
            Ok(config) => config.clamped(),
            Err(error) => {
                crate::log_error!("{} is malformed, using defaults: {error}", path.display());
                Self::default()
            }
        }
    }

    /// Brings anything out of range back into it.
    ///
    /// The file is untrusted input, and a zoom of zero is not a setting — it is a window with
    /// nothing in it, which reads as a broken editor rather than a wrong preference.
    #[must_use]
    fn clamped(mut self) -> Self {
        if !self.zoom.is_finite() {
            self.zoom = 1.0;
        }
        self.zoom = self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self
    }
}

/// The settings, and the file they came from.
///
/// Holds the file's *whole* contents as well as the typed [`Config`], which is what lets a write
/// preserve keys this build does not know about. See the module docs.
#[derive(Debug, Clone)]
pub struct Settings {
    path: Option<PathBuf>,
    config: Config,
    /// Every key the file had, including ones [`Config`] has no field for.
    ///
    /// Empty when the file was absent or malformed — in the second case that is the one way a write
    /// can lose something, and the module docs say so.
    raw: serde_json::Map<String, serde_json::Value>,
    /// Whether [`Config`] has changed since the file was last in step with it.
    dirty: bool,
}

impl Settings {
    /// Reads the settings from the usual place.
    #[must_use]
    pub fn load() -> Self {
        match default_config_path() {
            Some(path) => Self::at(path),
            // Nowhere to read from and nowhere to write to. The editor runs on defaults, and a
            // change to a setting simply does not persist — the same position the session file is in.
            None => Self::nowhere(),
        }
    }

    /// Settings with no file behind them: defaults, and changes that do not persist.
    ///
    /// What an editor gets before anything tells it where its settings live — every test, and the
    /// real one for the moment between construction and [`with_settings`]. Deliberately not the same
    /// as [`load`](Self::load), which would read the user's file.
    ///
    /// [`with_settings`]: crate::editor::Editor::with_settings
    #[must_use]
    pub fn nowhere() -> Self {
        Self {
            path: None,
            config: Config::default(),
            raw: serde_json::Map::new(),
            dirty: false,
        }
    }

    /// Reads the settings from a given file. For tests, which must not touch the user's.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        let config = Config::load_from(&path);
        // Read a second time as a bare map. Deliberately separate from the typed read: they answer
        // different questions, and a file this one rejects still gives a usable `Config` from the
        // defaults.
        let raw = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();

        Self {
            path: Some(path),
            config,
            raw,
            dirty: false,
        }
    }

    /// What the user chose.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config
    }

    /// Changes a setting, and remembers to write it.
    ///
    /// Nothing is written here — [`flush`](Self::flush) does that, on the idle timer, so a run of
    /// rapid changes costs one write rather than one each.
    pub fn update(&mut self, change: impl FnOnce(&mut Config)) {
        let before = self.config;
        change(&mut self.config);
        self.config = self.config.clamped();
        if self.config != before {
            self.dirty = true;
        }
    }

    /// Whether there is a change waiting to be written.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Writes the settings back, if anything changed. Returns whether it wrote.
    ///
    /// Merges the typed fields into the map the file was read as, so unknown keys survive. A failure
    /// is reported once and forgotten: settings are not worth failing anything for, and retrying
    /// every idle tick would fill the log rather than fix the disk.
    pub fn flush(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;

        let Some(path) = self.path.as_deref() else {
            return false;
        };

        // Serializing the typed config gives exactly the keys this build owns; everything else in
        // `raw` is left as it was found. That is the whole of the unknown-key guarantee.
        let Ok(serde_json::Value::Object(known)) = serde_json::to_value(self.config) else {
            return false;
        };
        let mut merged = self.raw.clone();
        for (key, value) in known {
            merged.insert(key, value);
        }

        let Ok(json) = serde_json::to_vec_pretty(&serde_json::Value::Object(merged.clone())) else {
            return false;
        };
        if let Some(directory) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(directory) {
                crate::log_error!("could not create {}: {error}", directory.display());
                return false;
            }
        }

        // Temp then rename, as the session and snapshot files are: a crash mid-write must leave the
        // old settings rather than half of the new ones.
        let mut temporary = path.to_path_buf();
        temporary.set_extension("tmp");
        if let Err(error) = std::fs::write(&temporary, &json) {
            crate::log_error!("could not write {}: {error}", temporary.display());
            return false;
        }
        if let Err(error) = std::fs::rename(&temporary, path) {
            crate::log_error!("could not replace {}: {error}", path.display());
            return false;
        }

        // Kept so a second change merges onto what was actually written rather than onto what was
        // read at startup.
        self.raw = merged;
        true
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
