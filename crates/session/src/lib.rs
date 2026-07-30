//! Open tabs as lightweight handles, and the session file that restores them.
//!
//! # A tab is a handle until it becomes visible
//!
//! `AGENTS.md` states the rule and names the reason: every large-session failure in the competition
//! comes from materialising every open document at once, and the ≤50 MB baseline does not survive
//! it. Obsidian's `DeferredView` is identified in `docs/research/01-editor-landscape.md` as the only
//! implementation in this category that gets it right.
//!
//! So a [`Tab`] is a path, a name and a remembered caret. The [`Document`] behind it does not exist
//! until something displays it, and restoring a session of two hundred tabs reads one file — the one
//! that was active.
//!
//! # The session file is untrusted input
//!
//! Every path out of it goes through [`path::validate`] before anything opens it, and an entry that
//! fails is skipped rather than taking the whole session down with it. See that module for the
//! threat model, which on Windows is mostly about UNC paths rather than traversal.

use std::path::{Path, PathBuf};

use mdedit_document::{Document, Selection};
use mdedit_file::LoadedFile;
use mdedit_offsets::ByteOffset;
use serde::{Deserialize, Serialize};

pub mod path;

pub use path::PathRejected;

/// The session format this build writes.
const FORMAT_VERSION: u32 = 1;

/// Most tabs a session file may restore.
///
/// A bound on attacker-controlled input, not a judgement about how many tabs anyone needs. Without
/// it a crafted file could ask for millions of handles before a single path is checked.
const MAX_TABS: usize = 1_000;

/// Largest session file that will be read, in bytes.
const MAX_SESSION_BYTES: u64 = 4 * 1024 * 1024;

/// Why a session could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The session file could not be read or written.
    #[error("{action} session at {path}: {source}")]
    Io {
        /// What was being attempted.
        action: &'static str,
        /// The session file.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },

    /// The session file is not valid JSON, or not the shape this build understands.
    #[error("session at {path} is malformed: {detail}")]
    Malformed {
        /// The session file.
        path: PathBuf,
        /// What was wrong.
        detail: String,
    },

    /// The session file is larger than [`MAX_SESSION_BYTES`].
    #[error("session at {path} is {size} bytes, larger than the {MAX_SESSION_BYTES} byte limit")]
    TooLarge {
        /// The session file.
        path: PathBuf,
        /// Its size.
        size: u64,
    },

    /// A tab could not be opened.
    #[error("{0}")]
    File(#[from] mdedit_file::FileError),

    /// A tab index that does not exist.
    #[error("no tab at index {0}")]
    NoSuchTab(usize),
}

/// Where a tab's contents came from.
///
/// An enum rather than an `Option<PathBuf>` because the two cases behave differently everywhere that
/// matters — what the tab is called, whether it counts as modified, whether it can be saved without
/// asking, and whether the session file mentions it at all. Making that a nullable path would leave
/// each of those decisions to be remembered at the call site.
#[derive(Debug)]
enum Origin {
    /// Backed by a file, which may not have been read yet.
    File(PathBuf),
    /// Never saved anywhere. Carries the number that names it, so "Untitled 2" stays "Untitled 2"
    /// when "Untitled 1" is closed.
    Untitled(u32),
}

/// A file that is open in the editor.
///
/// Holds no [`Document`] until [`Workspace::activate`] materialises one — except for an untitled
/// tab, which has nothing to defer and is open from the moment it exists.
#[derive(Debug)]
pub struct Tab {
    origin: Origin,
    /// Where the caret was when the session was saved, restored on materialisation.
    caret: ByteOffset,
    /// `None` while this tab is deferred. The whole point of the type.
    open: Option<OpenTab>,
}

/// A tab that has been displayed, and therefore read.
#[derive(Debug)]
pub struct OpenTab {
    /// The buffer, with its own undo history and caret.
    pub document: Document,
    /// The bytes as they were on disk, for the byte-identical save path. `None` for a buffer that
    /// has never been on disk — there is nothing to be identical to.
    pub file: Option<LoadedFile>,
}

impl Tab {
    /// The file this tab shows, or `None` for a buffer that has never been saved.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.origin {
            Origin::File(path) => Some(path),
            Origin::Untitled(_) => None,
        }
    }

    /// Whether this tab has never been saved anywhere.
    #[must_use]
    pub fn is_untitled(&self) -> bool {
        matches!(self.origin, Origin::Untitled(_))
    }

    /// The name to put on the tab strip.
    #[must_use]
    pub fn display_name(&self) -> String {
        match &self.origin {
            Origin::File(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            Origin::Untitled(number) => format!("Untitled {number}"),
        }
    }

    /// Whether this tab has been read from disk yet.
    #[must_use]
    pub fn is_materialised(&self) -> bool {
        self.open.is_some()
    }

    /// The open document, if this tab has been displayed.
    #[must_use]
    pub fn open(&self) -> Option<&OpenTab> {
        self.open.as_ref()
    }

    /// Whether the buffer differs from the file on disk.
    ///
    /// A deferred tab is never modified: nothing has been able to edit it. An untitled one is
    /// modified as soon as it has anything in it, because there is nothing on disk to compare
    /// against and closing it would lose whatever is there.
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.open.as_ref().is_some_and(|open| match &open.file {
            Some(file) => open.document.to_string() != file.text(),
            None => !open.document.is_empty(),
        })
    }
}

/// Every open tab, and which one is showing.
#[derive(Debug, Default)]
pub struct Workspace {
    tabs: Vec<Tab>,
    active: Option<usize>,
    /// How many untitled buffers have been made this run, so the next one gets a fresh number.
    next_untitled: u32,
}

/// A session entry that was refused, so the caller can say so rather than silently losing a tab.
#[derive(Debug)]
pub struct SkippedTab {
    /// Why it was refused.
    pub reason: PathRejected,
}

impl Workspace {
    /// An empty workspace.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The open tabs, in strip order.
    #[must_use]
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// Which tab is showing.
    #[must_use]
    pub fn active(&self) -> Option<usize> {
        self.active
    }

    /// The open document of the visible tab, if there is one and it has been displayed.
    #[must_use]
    pub fn active_open(&self) -> Option<&OpenTab> {
        self.active
            .and_then(|index| self.tabs.get(index))
            .and_then(Tab::open)
    }

    /// As [`active_open`](Self::active_open), mutably.
    pub fn active_open_mut(&mut self) -> Option<&mut OpenTab> {
        let index = self.active?;
        self.tabs.get_mut(index)?.open.as_mut()
    }

    /// The path of the visible tab.
    #[must_use]
    pub fn active_path(&self) -> Option<&Path> {
        self.active
            .and_then(|index| self.tabs.get(index))
            .and_then(Tab::path)
    }

    /// Replaces the visible tab's record of what is on disk, after a save.
    pub fn set_active_file(&mut self, file: LoadedFile) {
        if let Some(open) = self.active_open_mut() {
            open.file = Some(file);
        }
    }

    /// How many tabs have actually been read from disk.
    ///
    /// Exists to be asserted on. "Restoring two hundred tabs opens at most the visible one" is only
    /// a real guarantee if a test can check it, rather than someone watching a memory graph.
    #[must_use]
    pub fn materialised_count(&self) -> usize {
        self.tabs.iter().filter(|tab| tab.is_materialised()).count()
    }

    /// Adds a tab for `path` without reading it.
    ///
    /// Returns the index of the new tab, or of the existing one if the file is already open.
    pub fn add(&mut self, path: PathBuf) -> usize {
        if let Some(existing) = self
            .tabs
            .iter()
            .position(|tab| tab.path() == Some(path.as_path()))
        {
            return existing;
        }
        self.tabs.push(Tab {
            origin: Origin::File(path),
            caret: ByteOffset::ZERO,
            open: None,
        });
        self.tabs.len() - 1
    }

    /// Adds an empty buffer that has never been saved, and returns its index.
    ///
    /// Open from the moment it exists: there is nothing to defer, since there is no file to read.
    /// The number counts up and is never reused while the editor runs, so closing "Untitled 1" does
    /// not make the next one "Untitled 1" again while it is still on screen.
    pub fn add_untitled(&mut self) -> usize {
        self.next_untitled = self.next_untitled.saturating_add(1);
        self.tabs.push(Tab {
            origin: Origin::Untitled(self.next_untitled),
            caret: ByteOffset::ZERO,
            open: Some(OpenTab {
                document: Document::new(),
                file: None,
            }),
        });
        self.tabs.len() - 1
    }

    /// Points a tab at a file, after its contents have been written there.
    ///
    /// What makes Save As different from Save: the tab stops being untitled, or stops being the file
    /// it was, and everything downstream — its name, whether it is modified, whether the session
    /// records it — follows from that.
    ///
    /// `file` is what is now on disk, so the modified comparison is against the new bytes rather
    /// than the old ones.
    ///
    /// # Errors
    ///
    /// [`SessionError::NoSuchTab`] for an index that does not exist.
    pub fn bind_to_path(
        &mut self,
        index: usize,
        path: PathBuf,
        file: LoadedFile,
    ) -> Result<(), SessionError> {
        let tab = self
            .tabs
            .get_mut(index)
            .ok_or(SessionError::NoSuchTab(index))?;
        tab.origin = Origin::File(path);
        if let Some(open) = tab.open.as_mut() {
            open.file = Some(file);
        }
        Ok(())
    }

    /// Makes `index` the visible tab, reading it from disk if this is the first time.
    ///
    /// # Errors
    ///
    /// [`SessionError::NoSuchTab`] for an index that does not exist, or [`SessionError::File`] if
    /// the file cannot be read — it may have been deleted since the session was written.
    pub fn activate(&mut self, index: usize) -> Result<&mut OpenTab, SessionError> {
        let tab = self
            .tabs
            .get_mut(index)
            .ok_or(SessionError::NoSuchTab(index))?;

        if tab.open.is_none() {
            let Some(path) = tab.path() else {
                // An untitled tab has no file to read; it was open from the moment it existed.
                self.active = Some(index);
                return tab.open.as_mut().ok_or(SessionError::NoSuchTab(index));
            };
            let file = mdedit_file::load(path)?;
            let mut document = Document::from(file.text());
            // Put the caret back where the session left it, clamped: the file may have changed on
            // disk since, and a stale offset must not be able to panic a restore.
            let caret = ByteOffset::new(tab.caret.get().min(document.len_bytes()));
            document.set_selection(Selection::caret(caret));
            tab.open = Some(OpenTab {
                document,
                file: Some(file),
            });
        }

        self.active = Some(index);
        // Checked immediately above.
        tab.open.as_mut().ok_or(SessionError::NoSuchTab(index))
    }

    /// Closes a tab, returning whether it had unsaved changes.
    ///
    /// Reports rather than prompts: what to do about unsaved work is a decision for whatever has a
    /// user in front of it, not for the model.
    ///
    /// # Errors
    ///
    /// [`SessionError::NoSuchTab`] for an index that does not exist.
    pub fn close(&mut self, index: usize) -> Result<bool, SessionError> {
        if index >= self.tabs.len() {
            return Err(SessionError::NoSuchTab(index));
        }
        let was_modified = self.tabs[index].is_modified();
        self.tabs.remove(index);

        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            // Closing the active tab shows its neighbour rather than nothing.
            Some(active) if active == index => Some(active.min(self.tabs.len() - 1)),
            other => other,
        };
        Ok(was_modified)
    }

    /// Whether any open tab has unsaved changes.
    #[must_use]
    pub fn has_unsaved_changes(&self) -> bool {
        self.tabs.iter().any(Tab::is_modified)
    }
}

/// The on-disk shape. Deliberately small and boring.
#[derive(Debug, Serialize, Deserialize)]
struct SessionFile {
    version: u32,
    active: Option<usize>,
    tabs: Vec<SessionTab>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionTab {
    path: PathBuf,
    #[serde(default)]
    caret: usize,
}

impl Workspace {
    /// Writes the tab list and the active tab.
    ///
    /// Only handles are written — never content. A session file is a list of places to look, which
    /// is what makes restoring one cheap.
    ///
    /// # Errors
    ///
    /// [`SessionError::Io`] if the file cannot be written.
    pub fn save_session(&self, path: &Path) -> Result<(), SessionError> {
        let session = SessionFile {
            version: FORMAT_VERSION,
            active: self.active,
            // Untitled tabs are left out. The session persists handles, not content — that is what
            // makes restoring one cheap — and an untitled buffer is nothing but content. Writing it
            // here would turn the session file into a place documents live, with its own data-loss
            // questions to answer.
            tabs: self
                .tabs
                .iter()
                .filter_map(|tab| {
                    Some(SessionTab {
                        path: tab.path()?.to_path_buf(),
                        // The live caret if the tab is open, otherwise the one it was restored with.
                        caret: tab
                            .open
                            .as_ref()
                            .map_or(tab.caret, |open| open.document.selection().head())
                            .get(),
                    })
                })
                .collect(),
        };

        let json = serde_json::to_vec_pretty(&session).map_err(|e| SessionError::Malformed {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SessionError::Io {
                action: "creating directory for",
                path: path.to_path_buf(),
                source: e,
            })?;
        }
        std::fs::write(path, json).map_err(|e| SessionError::Io {
            action: "writing",
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Reads a session, skipping any entry whose path is refused.
    ///
    /// **No tab is materialised.** The returned workspace has read the session file and nothing
    /// else; the first document is read when something activates it.
    ///
    /// Entries are skipped rather than fatal, and reported rather than dropped silently: a file
    /// that has since been deleted should cost one tab, not the whole session.
    ///
    /// # Errors
    ///
    /// [`SessionError::Io`] if the file cannot be read, [`SessionError::TooLarge`] if it exceeds
    /// the size limit, or [`SessionError::Malformed`] if it is not the expected shape.
    pub fn restore_session(path: &Path) -> Result<(Self, Vec<SkippedTab>), SessionError> {
        let size = std::fs::metadata(path)
            .map_err(|e| SessionError::Io {
                action: "reading",
                path: path.to_path_buf(),
                source: e,
            })?
            .len();
        if size > MAX_SESSION_BYTES {
            return Err(SessionError::TooLarge {
                path: path.to_path_buf(),
                size,
            });
        }

        let bytes = std::fs::read(path).map_err(|e| SessionError::Io {
            action: "reading",
            path: path.to_path_buf(),
            source: e,
        })?;
        let session: SessionFile =
            serde_json::from_slice(&bytes).map_err(|e| SessionError::Malformed {
                path: path.to_path_buf(),
                detail: e.to_string(),
            })?;

        if session.version != FORMAT_VERSION {
            return Err(SessionError::Malformed {
                path: path.to_path_buf(),
                detail: format!(
                    "version {} is not the {FORMAT_VERSION} this build writes",
                    session.version
                ),
            });
        }

        let mut workspace = Self::new();
        let mut skipped = Vec::new();

        for entry in session.tabs.into_iter().take(MAX_TABS) {
            if let Err(reason) = path::validate(&entry.path) {
                skipped.push(SkippedTab { reason });
                continue;
            }
            let index = workspace.add(entry.path);
            workspace.tabs[index].caret = ByteOffset::new(entry.caret);
        }

        // Clamped rather than trusted: a crafted index must not be able to address past the end,
        // and skipped entries shift everything after them anyway.
        workspace.active = session
            .active
            .filter(|_| !workspace.tabs.is_empty())
            .map(|active| active.min(workspace.tabs.len() - 1));

        Ok((workspace, skipped))
    }
}

/// Where the session file lives for this user.
///
/// Per-user, never machine-wide: `AGENTS.md` requires a per-user install that never elevates, and a
/// session file somewhere writable by other accounts would be an obvious way to hand this process a
/// path it should not open.
#[must_use]
pub fn default_session_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);

    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")));

    base.map(|base| base.join("SimpleMarkdownEditor").join("session.json"))
}
