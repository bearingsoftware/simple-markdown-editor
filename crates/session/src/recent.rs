//! The files this editor opened last, so it can offer them again.
//!
//! # State, not settings
//!
//! This lives beside the session file rather than in `config.json`, and the distinction is
//! load-bearing. The config file is something a person edits and this process only ever reads;
//! `mdedit_app::config` documents why nothing in it may name a path. A recent-files list is the
//! opposite — nobody hand-writes one, and it is written on every open.
//!
//! # Which puts it in the session file's threat model exactly
//!
//! `AGENTS.md`: *treat our own config and session files as untrusted input.* A file this process
//! writes into a directory something else may reach is precisely the attacker-writable path source
//! [`crate::path::validate`] exists for, and a menu of paths that get opened on click is a good way
//! to be handed one. So every entry read back is validated before it can reach a menu, and an entry
//! that fails is dropped without a word — a recent list is a convenience, and no convenience is
//! worth a dialog on startup, let alone a refusal to start.
//!
//! Notepad++ shipped three consecutive path-handling bypasses in 2026 by patching symptoms rather
//! than the boundary. The boundary is `validate`, and this file calls it rather than reimplementing
//! any part of it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::path::validate;

/// How many files to remember.
///
/// Ten is what fits comfortably in a submenu and is roughly where a "recent" list stops being one.
pub const MAX_ENTRIES: usize = 10;

/// Largest recent-files file that will be read.
///
/// A bound on input this process does not fully control, like the session file's. Ten paths cannot
/// approach it; anything that does was not written by this editor.
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    paths: Vec<PathBuf>,
}

/// The recently-opened files, most recent first.
#[derive(Debug, Default)]
pub struct Recent {
    file: Option<PathBuf>,
    paths: Vec<PathBuf>,
}

impl Recent {
    /// Opens the list that lives beside the session file, reading whatever survived.
    ///
    /// A list that cannot be found or cannot be read is an empty one. There is no error to report
    /// because there is nothing the user could do about it and nothing they have lost.
    #[must_use]
    pub fn beside_session() -> Self {
        let Some(file) = crate::default_session_path()
            .and_then(|session| session.parent().map(|dir| dir.join("recent.json")))
        else {
            return Self::default();
        };
        Self::at(file)
    }

    /// Opens the list stored at `file`. For tests, which must not touch the real one.
    #[must_use]
    pub fn at(file: PathBuf) -> Self {
        let paths = read(&file);
        Self {
            file: Some(file),
            paths,
        }
    }

    /// The remembered files, most recent first. Every one has been through [`validate`].
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Records that `path` was opened, moving it to the front.
    ///
    /// A path already in the list is moved rather than duplicated, which is what makes the list
    /// read as "what I have been working on" rather than as a log.
    ///
    /// Written through immediately. The alternative is writing on exit, and the whole point of this
    /// list is to survive a run that did not get there.
    pub fn record(&mut self, path: &Path) {
        // Refused here as well as on the way in. A path this process would not accept from its own
        // file is one it should not be writing into it either — the list must not become a way to
        // launder something past the boundary by storing it now and reading it back later.
        if validate(path).is_err() {
            return;
        }
        self.paths.retain(|existing| existing != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_ENTRIES);
        self.write();
    }

    /// Forgets `path` — it has been opened and turned out not to be there.
    pub fn forget(&mut self, path: &Path) {
        let before = self.paths.len();
        self.paths.retain(|existing| existing != path);
        if self.paths.len() != before {
            self.write();
        }
    }

    fn write(&self) {
        let Some(file) = self.file.as_deref() else {
            return;
        };
        let stored = Stored {
            paths: self.paths.clone(),
        };
        let Ok(json) = serde_json::to_vec_pretty(&stored) else {
            return;
        };
        if let Some(directory) = file.parent() {
            if std::fs::create_dir_all(directory).is_err() {
                return;
            }
        }
        // Temp-then-rename, as the session and snapshot files are: a crash part way through must
        // leave the previous list rather than a truncated one.
        let mut temporary = file.to_path_buf();
        temporary.set_extension("tmp");
        if std::fs::write(&temporary, &json).is_ok() {
            let _ = std::fs::rename(&temporary, file);
        }
    }
}

/// Reads and validates the stored list, returning what survived in order.
fn read(file: &Path) -> Vec<PathBuf> {
    let Ok(metadata) = std::fs::metadata(file) else {
        return Vec::new();
    };
    if metadata.len() > MAX_BYTES {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(file) else {
        return Vec::new();
    };
    let Ok(stored) = serde_json::from_slice::<Stored>(&bytes) else {
        return Vec::new();
    };

    let mut paths = Vec::new();
    for path in stored.paths {
        // The boundary. Nothing below this line has decided a path is safe; this is where it is
        // decided, and it is the same function the session file's entries go through.
        if validate(&path).is_err() {
            continue;
        }
        if paths.contains(&path) {
            continue;
        }
        paths.push(path);
        if paths.len() == MAX_ENTRIES {
            break;
        }
    }
    paths
}
