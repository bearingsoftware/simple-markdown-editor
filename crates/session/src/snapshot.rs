//! Unsaved work, kept where a crash cannot take it.
//!
//! # What this is for
//!
//! The session file records which tabs were open and is written on a clean exit. Nothing recorded
//! their *contents*, so a crash — or a power cut, or a kill — lost every unsaved buffer, and an
//! untitled one entirely, because there was not even a path to say what had been lost.
//!
//! # The shape, and why each part is the way it is
//!
//! **Snapshots are content, addressed by an id.** A file-backed tab could be keyed by its path, but
//! an untitled buffer has none and is exactly the case with most to lose. So every snapshot carries
//! its own id and the manifest says what it belonged to.
//!
//! **Written atomically**, temp-then-rename, as the session file is. A crash during a snapshot must
//! leave the *previous* snapshot intact rather than a half-written one; a recovery file that cannot
//! be trusted is worse than none, because the user will trust it.
//!
//! **Deleted on save, and when its tab closes.** A stored copy outliving the work it holds would
//! offer to restore something already on disk, which teaches people to dismiss the offer.
//!
//! It is **not** deleted on a clean exit any more (MDEDIT-66). Surviving an ordinary quit is the
//! point: unsaved work comes back next time. A crash is told apart by a marker in the session file
//! instead — see `last_run_ended_cleanly`.
//!
//! # The trust boundary is the manifest, not the content
//!
//! The manifest names paths, and it is a file this process wrote into the user's state directory —
//! the same position the session file is in, and `path` documents the threat. So every path read
//! back goes through [`crate::path::validate`] before anything opens it. The snapshot *content* is
//! arbitrary text that is shown to the user as unsaved work, never executed and never a path, so it
//! needs only a size cap.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::path::{PathRejected, validate};

/// Largest snapshot manifest that will be read.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Largest single snapshot that will be read back, matching what a session may hold.
const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

/// One recovered buffer.
#[derive(Debug)]
pub struct Recovered {
    /// Which stored copy this is, so a caller that decides against it can remove it.
    ///
    /// Without this a stored edit the user declined would be offered again on every launch, which
    /// turns a safeguard into a nuisance and then into something people click through.
    pub id: u64,
    /// The file it belonged to, or `None` if it was never saved anywhere.
    pub path: Option<PathBuf>,
    /// What was in the buffer when it was last snapshotted.
    pub text: String,
    /// What the file looked like when this copy was stored.
    ///
    /// `None` for a buffer with no file, and for one stored by a build before MDEDIT-66 added this.
    /// A caller with no stamp cannot tell whether the file has moved on, and should say so rather
    /// than assume either way.
    pub stamp: Option<mdedit_file::Stamp>,
}

/// What the manifest records about one snapshot.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    /// The snapshot file's name within the directory. Never a path — see [`Store::read_all`].
    id: u64,
    /// The file the buffer came from, absent for an untitled one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    /// The file's modification time in milliseconds when this copy was stored.
    ///
    /// Two plain numbers rather than a [`mdedit_file::Stamp`] because this file is JSON and that
    /// type is not `serde`'s business — see [`mdedit_file::Stamp::parts`], which exists for exactly
    /// this and round-trips exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modified_ms: Option<u64>,
    /// The file's length when this copy was stored. Absent together with `modified_ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    len: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    entries: Vec<Entry>,
}

/// The snapshot directory, and what is currently in it.
#[derive(Debug)]
pub struct Store {
    directory: PathBuf,
    manifest: Manifest,
}

impl Store {
    /// Opens the store beside the session file.
    ///
    /// Returns `None` when there is nowhere to put it, which is the same condition under which the
    /// session file does not exist either. Crash recovery is a courtesy; it never stops a start.
    #[must_use]
    pub fn beside_session() -> Option<Self> {
        let session = crate::default_session_path()?;
        let directory = session.parent()?.join("recovery");
        Some(Self {
            directory,
            manifest: Manifest::default(),
        })
    }

    /// Opens a store in a given directory. For tests, which must not touch the real one.
    #[must_use]
    pub fn in_directory(directory: PathBuf) -> Self {
        Self {
            directory,
            manifest: Manifest::default(),
        }
    }

    fn manifest_path(&self) -> PathBuf {
        self.directory.join("manifest.json")
    }

    fn snapshot_path(&self, id: u64) -> PathBuf {
        self.directory.join(format!("{id}.snapshot"))
    }

    /// Records one buffer's contents, replacing whatever was there for that id.
    ///
    /// `stamp` describes the bytes `text` was built on — the caller's [`mdedit_file::LoadedFile`]
    /// stamp, not the file as it is now. It is a parameter rather than something read here for the
    /// reason MDEDIT-100 was: this used to take its own `Stamp::of(path)`, so a file rewritten
    /// between the load and the idle tick was recorded as though the buffer had been built on the
    /// *new* bytes. The next launch then compared the manifest against a matching disk state, called
    /// it unchanged, and put a stale edit back over somebody else's write without asking. `None` for
    /// a buffer that has no file.
    ///
    /// # Errors
    ///
    /// Any I/O failure. The caller logs it and carries on: failing to snapshot is a smaller problem
    /// than refusing to keep editing.
    pub fn write(
        &mut self,
        id: u64,
        path: Option<&Path>,
        stamp: Option<mdedit_file::Stamp>,
        text: &str,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        write_atomically(&self.snapshot_path(id), text.as_bytes())?;

        let (modified_ms, len) = stamp.map_or((None, None), |stamp| {
            let (modified_ms, len) = stamp.parts();
            (modified_ms, Some(len))
        });

        if let Some(entry) = self
            .manifest
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
        {
            entry.path = path.map(Path::to_path_buf);
            entry.modified_ms = modified_ms;
            entry.len = len;
        } else {
            self.manifest.entries.push(Entry {
                id,
                path: path.map(Path::to_path_buf),
                modified_ms,
                len,
            });
        }
        self.write_manifest()
    }

    /// Forgets one buffer — it has been saved, or closed, and no longer needs protecting.
    ///
    /// # Errors
    ///
    /// Any I/O failure writing the manifest back.
    pub fn forget(&mut self, id: u64) -> std::io::Result<()> {
        let _ = std::fs::remove_file(self.snapshot_path(id));
        self.manifest.entries.retain(|entry| entry.id != id);
        if self.manifest.entries.is_empty() {
            let _ = std::fs::remove_file(self.manifest_path());
            return Ok(());
        }
        self.write_manifest()
    }

    /// Reads whatever survived, and reports anything refused.
    ///
    /// Nothing here is fatal: a manifest that will not parse, an entry whose path is hostile, a
    /// snapshot that has gone missing — each is skipped and named, because the alternative is an
    /// editor that will not start because of a recovery file.
    #[must_use]
    pub fn read_all(&mut self) -> (Vec<Recovered>, Vec<String>) {
        let mut recovered = Vec::new();
        let mut skipped = Vec::new();

        let manifest_path = self.manifest_path();
        let Ok(metadata) = std::fs::metadata(&manifest_path) else {
            return (recovered, skipped);
        };
        if metadata.len() > MAX_MANIFEST_BYTES {
            skipped.push(format!(
                "{} is {} bytes, past the {MAX_MANIFEST_BYTES} byte limit",
                manifest_path.display(),
                metadata.len()
            ));
            return (recovered, skipped);
        }

        let Ok(bytes) = std::fs::read(&manifest_path) else {
            skipped.push(format!("could not read {}", manifest_path.display()));
            return (recovered, skipped);
        };
        let manifest: Manifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(error) => {
                skipped.push(format!("{} is malformed: {error}", manifest_path.display()));
                return (recovered, skipped);
            }
        };

        for entry in &manifest.entries {
            // The same boundary the session file has, for the same reason: this is a file this
            // process wrote into a directory something else may be able to write too.
            //
            // **One rejection is not a refusal.** `validate`'s last rule is that the path names an
            // existing regular file, and a stored edit whose file has since been *deleted* fails it
            // — which used to drop the entry and with it the text, in precisely the case where the
            // stored copy is the only copy left in the world. So a missing file downgrades the entry
            // to one with no path, and the text survives as a scratch buffer.
            //
            // The other three rules are the security ones — UNC and device paths, relative paths,
            // traversal — and they still refuse outright. Nothing here loosens them, and the path is
            // dropped rather than carried forward, so nothing downstream can open it.
            let mut path = entry.path.clone();
            if let Some(named) = &entry.path {
                match validate(named) {
                    Ok(()) => {}
                    Err(PathRejected::NotAFile(_)) => {
                        skipped.push(format!(
                            "{} has gone; its unsaved text comes back with no file",
                            named.display()
                        ));
                        path = None;
                    }
                    Err(reason) => {
                        skipped.push(format!("recovery entry refused: {reason}"));
                        continue;
                    }
                }
            }

            let snapshot = self.snapshot_path(entry.id);
            let readable =
                std::fs::metadata(&snapshot).is_ok_and(|data| data.len() <= MAX_SNAPSHOT_BYTES);
            if !readable {
                skipped.push(format!("{} is missing or too large", snapshot.display()));
                continue;
            }
            match std::fs::read_to_string(&snapshot) {
                Ok(text) => recovered.push(Recovered {
                    id: entry.id,
                    path,
                    text,
                    // Both halves or neither: a length with no time, or the other way round, is not
                    // a stamp and must not be treated as one.
                    stamp: entry
                        .len
                        .map(|len| mdedit_file::Stamp::from_parts(entry.modified_ms, len)),
                }),
                Err(error) => skipped.push(format!("{}: {error}", snapshot.display())),
            }
        }

        self.manifest = manifest;
        (recovered, skipped)
    }

    fn write_manifest(&self) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(&self.manifest)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        write_atomically(&self.manifest_path(), &json)
    }
}

use crate::atomic::write as write_atomically;
