//! Reading and writing files without changing a byte that was not edited.
//!
//! The byte-preservation invariant is the rule that outranks everything in `AGENTS.md`: opening a
//! file and saving it must produce an identical byte sequence. Until this crate existed that was
//! proven only for the marker segmentation — never for a round trip through the filesystem.
//!
//! # Why metadata alone is not enough
//!
//! The obvious design is to record the source encoding, decode on open, and re-encode on save. That
//! is necessary and **not sufficient**, because decode-then-encode is not the identity function:
//!
//! - **Shift-JIS has duplicate mappings.** Two different byte sequences decode to the same
//!   character, so only one of them survives a round trip.
//! - **Unpaired UTF-16 surrogates cannot be represented** in a Rust `String` at all. They decode to
//!   U+FFFD and the original value is gone.
//! - **Malformed bytes in any encoding** decode to U+FFFD, which encodes back as U+FFFD.
//! - **`encoding_rs::Encoding::encode` substitutes HTML numeric character references** — a literal
//!   `&#1234;` — for anything the target encoding cannot represent. In a web form that is correct
//!   behaviour. In a text editor it is silent corruption.
//!
//! So the guarantee here does not come from re-deriving the bytes. It comes from **keeping them**:
//! [`LoadedFile`] holds the original, and [`save`] writes those exact bytes back when the text has
//! not been edited. Re-encoding happens only for text that actually changed, and only after
//! checking that this file can survive it — see [`LoadedFile::round_trips`].
//!
//! # No guessing
//!
//! [`load`] honours a byte-order mark and otherwise assumes UTF-8. It does **not** sniff for
//! windows-1252 or Shift-JIS, because charset guessing is heuristic by nature and guessing wrong
//! rewrites someone's document. A caller who knows better says so with [`load_as`].
//!
//! # Newlines are not normalised
//!
//! Most editors convert CRLF to LF on open and back on save. That destroys a file with mixed line
//! endings, which is a real thing to find in a repository, and it is a rewrite of bytes nobody
//! asked to change. The buffer here holds exactly what was decoded. [`Newline`] is recorded only so
//! the editor knows what to *insert* when someone presses Enter.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use encoding_rs::{Encoding, UTF_8};

/// Why a file could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FileError {
    /// The filesystem refused.
    #[error("{action} {path}: {source}")]
    Io {
        /// What was being attempted, for a message that says which file and why.
        action: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The underlying failure.
        source: io::Error,
    },

    /// The bytes are not valid in the encoding they were read as.
    ///
    /// Not a guess that some other encoding would work — that is the caller's decision, made with
    /// [`load_as`]. Reported rather than silently decoded to replacement characters, because a
    /// document full of U+FFFD that then gets saved has been destroyed.
    #[error("{path} is not valid {encoding}")]
    Malformed {
        /// The path involved.
        path: PathBuf,
        /// The encoding it was read as.
        encoding: &'static str,
    },

    /// The edited text cannot be written in this file's encoding without losing characters.
    ///
    /// Typing an emoji into a windows-1252 file reaches this. Refusing is the point: the
    /// alternative is writing `&#128512;` into the user's document and calling it saved.
    #[error("{path} cannot hold this text as {encoding} without losing characters")]
    Unrepresentable {
        /// The path involved.
        path: PathBuf,
        /// The encoding that cannot represent it.
        encoding: &'static str,
    },

    /// This file does not survive a decode/encode round trip, so its unedited bytes cannot be
    /// reproduced.
    ///
    /// Saving it would rewrite parts nobody touched. Reported instead, so the caller can offer to
    /// save a copy as UTF-8 rather than corrupting the original.
    #[error(
        "{path} does not round-trip through {encoding}; saving it would rewrite unedited bytes"
    )]
    NotRoundTrippable {
        /// The path involved.
        path: PathBuf,
        /// The encoding at fault.
        encoding: &'static str,
    },
}

impl FileError {
    fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

/// The dominant line ending in a file.
///
/// Advisory only. Nothing rewrites line endings; this exists so pressing Enter inserts what the
/// rest of the file already uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Newline {
    /// `\n`.
    #[default]
    Lf,
    /// `\r\n`.
    CrLf,
    /// Both are present. Enter inserts the more common one; neither is rewritten.
    Mixed,
}

impl Newline {
    /// What to insert for a new line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf | Self::Mixed => "\n",
            Self::CrLf => "\r\n",
        }
    }

    /// The dominant style in `text`.
    #[must_use]
    pub fn detect(text: &str) -> Self {
        let crlf = text.matches("\r\n").count();
        // Every `\r\n` also contains a `\n`, so lone newlines are the difference.
        let lone_lf = text.matches('\n').count() - crlf;

        match (crlf, lone_lf) {
            (0, _) => Self::Lf,
            (_, 0) => Self::CrLf,
            _ => Self::Mixed,
        }
    }
}

/// How a file is encoded on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFormat {
    encoding: &'static Encoding,
    bom: bool,
    newline: Newline,
}

impl FileFormat {
    /// The encoding's name, as the Encoding Standard spells it.
    #[must_use]
    pub fn encoding(&self) -> &'static str {
        self.encoding.name()
    }

    /// Whether the file began with a byte-order mark, which must be reproduced on save.
    #[must_use]
    pub fn has_bom(&self) -> bool {
        self.bom
    }

    /// The dominant line ending.
    #[must_use]
    pub fn newline(&self) -> Newline {
        self.newline
    }

    /// The byte-order mark for this format, if it had one.
    fn bom_bytes(&self) -> &'static [u8] {
        if !self.bom {
            return &[];
        }
        match self.encoding.name() {
            "UTF-8" => &[0xEF, 0xBB, 0xBF],
            "UTF-16LE" => &[0xFF, 0xFE],
            "UTF-16BE" => &[0xFE, 0xFF],
            // Only those three encodings have a BOM to sniff, so this is unreachable in practice.
            _ => &[],
        }
    }

    /// Encodes `text` for this format, BOM included.
    fn encode(&self, text: &str, path: &Path) -> Result<Vec<u8>, FileError> {
        let (encoded, _, had_errors) = self.encoding.encode(text);
        if had_errors {
            // `encode` substitutes HTML numeric character references for anything unmappable, so
            // "had_errors" means the output is already corrupt. It never reaches the disk.
            return Err(FileError::Unrepresentable {
                path: path.to_path_buf(),
                encoding: self.encoding.name(),
            });
        }

        let mut out = Vec::with_capacity(self.bom_bytes().len() + encoded.len());
        out.extend_from_slice(self.bom_bytes());
        out.extend_from_slice(&encoded);
        Ok(out)
    }
}

/// What a file looked like from the outside when it was read.
///
/// Modification time and length together, because neither is enough alone. Filesystem timestamp
/// granularity is coarse — FAT32 rounds to two seconds, and even NTFS is not fine enough to
/// distinguish two writes in the same instant — so a change can leave the time untouched. A length
/// alone misses any edit that happens to preserve it, which is most of them.
///
/// Deliberately *not* a hash. Comparing content means reading the whole file on every window focus,
/// which is the cost this exists to avoid. The residual false negative — same length, same coarse
/// timestamp, different bytes — needs a write inside the timestamp's granularity that keeps the
/// length exactly; rare enough to accept, where a false *positive* would mean a dialog on every
/// focus and would get the feature turned off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

impl Stamp {
    /// Reads the stamp of whatever is at `path`, or `None` if it cannot be looked at.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// What the file at a path is doing relative to the copy that was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskState {
    /// As it was read, as far as [`Stamp`] can tell.
    Unchanged,
    /// Something else has written it since.
    Changed,
    /// It is not there any more.
    Gone,
}

/// Asks what has happened to `path` since `file` was read from it.
///
/// [`DiskState::Unchanged`] is the answer whenever the question cannot be answered — a stamp that
/// could not be taken at load, or metadata that will not read now for a reason other than the file
/// being absent. The alternative is claiming a change on no evidence, and the cost of that lands on
/// the user as a dialog rather than on this function as a wrong return value.
#[must_use]
pub fn disk_state(path: &Path, file: &LoadedFile) -> DiskState {
    let Some(recorded) = file.stamp else {
        return DiskState::Unchanged;
    };
    match fs::metadata(path) {
        Ok(metadata) => {
            let now = Stamp {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            };
            if now == recorded {
                DiskState::Unchanged
            } else {
                DiskState::Changed
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => DiskState::Gone,
        Err(_) => DiskState::Unchanged,
    }
}

/// A file, decoded, with everything needed to write it back unchanged.
#[derive(Debug, Clone)]
pub struct LoadedFile {
    text: String,
    format: FileFormat,
    original: Vec<u8>,
    round_trips: bool,
    /// What the file looked like from outside when these bytes were read. `None` when the metadata
    /// could not be taken, which makes every later comparison say "unchanged".
    stamp: Option<Stamp>,
}

impl LoadedFile {
    /// The decoded text, exactly as it was on disk — line endings and all.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// How it is encoded.
    #[must_use]
    pub fn format(&self) -> FileFormat {
        self.format
    }

    /// The bytes as they were read.
    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        &self.original
    }

    /// Whether re-encoding this file reproduces it exactly.
    ///
    /// False means the encoding loses information on the way back — duplicate Shift-JIS mappings,
    /// unpaired surrogates, bytes that were already malformed. Such a file can be read and
    /// displayed, but saving *edited* text would rewrite parts nobody touched, so [`save`] refuses.
    #[must_use]
    pub fn round_trips(&self) -> bool {
        self.round_trips
    }

    /// What the file looked like from outside when these bytes were read.
    ///
    /// `None` when the metadata could not be taken, which makes [`disk_state`] answer "unchanged"
    /// for ever after — see there for why that is the right way to be wrong.
    #[must_use]
    pub fn stamp(&self) -> Option<Stamp> {
        self.stamp
    }
}

/// Reads a file, honouring a byte-order mark and otherwise assuming UTF-8.
///
/// # Errors
///
/// [`FileError::Io`] if the file cannot be read, or [`FileError::Malformed`] if the bytes are not
/// valid in the encoding the BOM implied. No charset guessing happens — see [`load_as`].
pub fn load(path: &Path) -> Result<LoadedFile, FileError> {
    // Stamped *before* the read, never after. A write landing between the two then leaves a stamp
    // older than the bytes, so the next comparison says "changed" about a file that has not changed
    // since — harmless. Stamping afterwards would leave a stamp *newer* than the bytes, and the
    // next comparison would say "unchanged" about a file that had been rewritten under us, which is
    // the failure this whole mechanism exists to prevent.
    let stamp = Stamp::of(path);
    let bytes = fs::read(path).map_err(|e| FileError::io("reading", path, e))?;
    let encoding = Encoding::for_bom(&bytes).map_or(UTF_8, |(encoding, _)| encoding);
    decode(path, bytes, encoding, stamp)
}

/// Reads a file as `encoding`, ignoring what a byte-order mark would have said.
///
/// For the case the editor cannot decide for itself: a windows-1252 or Shift-JIS file carries no
/// BOM, and guessing is what this crate exists not to do.
///
/// # Errors
///
/// As [`load`].
pub fn load_as(path: &Path, encoding: &'static Encoding) -> Result<LoadedFile, FileError> {
    let stamp = Stamp::of(path);
    let bytes = fs::read(path).map_err(|e| FileError::io("reading", path, e))?;
    decode(path, bytes, encoding, stamp)
}

/// Re-reads a file with the encoding it was originally read as.
///
/// What a caller wants after saving: the on-disk bytes have changed, so the copy held for the
/// byte-identical fast path is stale. Re-deriving the encoding from a BOM would be wrong for a file
/// opened with [`load_as`], which is why this exists rather than a second `load`.
///
/// # Errors
///
/// As [`load`].
pub fn reload(path: &Path, previous: &LoadedFile) -> Result<LoadedFile, FileError> {
    load_as(path, previous.format.encoding)
}

fn decode(
    path: &Path,
    bytes: Vec<u8>,
    encoding: &'static Encoding,
    stamp: Option<Stamp>,
) -> Result<LoadedFile, FileError> {
    // `decode` strips a BOM for the encoding it belongs to, which is what we want: the BOM is
    // format, not content, and is put back on save.
    let (text, actual, had_errors) = encoding.decode(&bytes);
    if had_errors {
        return Err(FileError::Malformed {
            path: path.to_path_buf(),
            encoding: encoding.name(),
        });
    }

    let bom = Encoding::for_bom(&bytes).is_some();
    let format = FileFormat {
        encoding: actual,
        bom,
        newline: Newline::detect(&text),
    };

    // Asked once, on open, rather than discovered at save time when the user is waiting and their
    // work is at stake.
    let round_trips = format
        .encode(&text, path)
        .is_ok_and(|re_encoded| re_encoded == bytes);

    Ok(LoadedFile {
        text: text.into_owned(),
        format,
        original: bytes,
        round_trips,
        stamp,
    })
}

/// Writes `text` back to `path`, preserving every byte that was not edited.
///
/// When `text` is unchanged this writes [`LoadedFile::original_bytes`] verbatim, so the file is
/// byte-identical by construction rather than by hoping the encoder is symmetric.
///
/// # Errors
///
/// - [`FileError::NotRoundTrippable`] if the text changed and this file's encoding cannot reproduce
///   its own unedited bytes. Saving would rewrite parts nobody touched.
/// - [`FileError::Unrepresentable`] if the edited text contains characters the encoding cannot
///   hold.
/// - [`FileError::Io`] if writing fails.
pub fn save(path: &Path, text: &str, file: &LoadedFile) -> Result<(), FileError> {
    let bytes = if text == file.text {
        // The guarantee, and the reason the original is kept at all.
        file.original.clone()
    } else {
        if !file.round_trips {
            return Err(FileError::NotRoundTrippable {
                path: path.to_path_buf(),
                encoding: file.format.encoding.name(),
            });
        }
        file.format.encode(text, path)?
    };

    write_atomically(path, &bytes)
}

/// Writes `text` to a path that has no bytes to preserve.
///
/// For a buffer that has never been on disk. [`save`] cannot serve this case: its whole job is to
/// reproduce bytes it was given, and there are none — so the choice of encoding has to be made here
/// rather than inherited. **UTF-8, no BOM**, and whatever line endings the text already contains,
/// which for a buffer typed into this editor means the ones [`Newline::as_str`] inserted.
///
/// Atomic, as [`save`] is: a crash part way through must not leave a half-written file where the user
/// asked for a whole one.
///
/// # Errors
///
/// [`FileError::Io`] if writing fails.
pub fn save_new(path: &Path, text: &str) -> Result<(), FileError> {
    write_atomically(path, text.as_bytes())
}

/// Writes to a temporary file in the same directory, then renames over the target.
///
/// A crash or a full disk half way through a plain write truncates the user's document. Rename is
/// atomic within a filesystem, so the file is either the old bytes or the new ones — never half of
/// each. Same directory because a rename across filesystems is not atomic and may not be permitted.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = directory.join(temp_name(path));

    fs::write(&temporary, bytes).map_err(|e| FileError::io("writing", &temporary, e))?;

    if let Err(error) = fs::rename(&temporary, path) {
        // Leaving a stray temporary behind would be its own small mess.
        let _ = fs::remove_file(&temporary);
        return Err(FileError::io("replacing", path, error));
    }
    Ok(())
}

/// A sibling name for the temporary file.
///
/// Derived from the target rather than random: two concurrent saves of *different* files must not
/// collide, and two concurrent saves of the *same* file are already a conflict the editor has to
/// resolve at a higher level.
fn temp_name(path: &Path) -> String {
    let stem = path
        .file_name()
        .map_or_else(|| "unnamed".into(), |name| name.to_string_lossy());
    format!(".{stem}.mdedit-tmp")
}
