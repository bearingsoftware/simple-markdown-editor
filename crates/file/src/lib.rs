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
    /// A lone `\r` — a classic-Mac file, or a document somebody has damaged.
    ///
    /// Never a *target* for conversion, only ever something a document is found to be. MDEDIT-59
    /// converts to LF or CRLF; nothing offers to make a file this.
    Cr,
    /// Both are present. Enter inserts the more common one; neither is rewritten.
    Mixed,
}

impl Newline {
    /// What to insert for a new line.
    ///
    /// `Mixed` inserts `\n`, because a document that is already both has no answer to give and the
    /// one that does not add a byte is the better guess.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf | Self::Mixed => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }

    /// The dominant style in `text`.
    ///
    /// **A lone `\r` counts.** It used to be invisible here — a classic-Mac file has no `\n` at all,
    /// so it was reported as `Lf`, and the status bar said so about a document containing not one
    /// of them. MDEDIT-46 draws those endings in the warning colour, which made the bar's answer
    /// visibly wrong beside them.
    #[must_use]
    pub fn detect(text: &str) -> Self {
        let crlf = text.matches("\r\n").count();
        // Every `\r\n` contains one of each, so the lone ones are the difference.
        let lone_lf = text.matches('\n').count() - crlf;
        let lone_cr = text.matches('\r').count() - crlf;

        match (crlf, lone_lf, lone_cr) {
            (_, 0, 0) if crlf > 0 => Self::CrLf,
            // Also the answer for a document with no line endings at all, which is what `default`
            // says and what such a document gets written with.
            (0, _, 0) => Self::Lf,
            (0, 0, _) => Self::Cr,
            _ => Self::Mixed,
        }
    }
}

/// Rewrites every line terminator in `text` to `to`, and says whether anything changed.
///
/// **Every terminator**, which includes the lone `\r` that nothing else in this codebase produces:
/// a classic-Mac file, or a stray carriage return left behind by a bad paste. Leaving those alone
/// would make "convert to LF" a conversion that demonstrably did not convert the document, and they
/// are the ones a user is most likely to be trying to get rid of.
///
/// Returns `None` when the text already uses `to` throughout — the caller then makes no edit at all,
/// so there is no undo step that appears to do nothing.
///
/// `Mixed` is not a target: it is a description of a document rather than a style to impose.
#[must_use]
pub fn normalise_newlines(text: &str, to: Newline) -> Option<String> {
    let ending = to.as_str();
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut rest = text;
    let mut changed = false;

    while let Some(at) = rest.find(['\r', '\n']) {
        out.push_str(&rest[..at]);
        let found = if rest[at..].starts_with("\r\n") {
            "\r\n"
        } else if rest[at..].starts_with('\r') {
            "\r"
        } else {
            "\n"
        };
        changed |= found != ending;
        out.push_str(ending);
        rest = &rest[at + found.len()..];
    }
    out.push_str(rest);

    changed.then_some(out)
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
        // **`encoding_rs` cannot encode to UTF-16, and does not say so by failing.** The Encoding
        // Standard defines the two UTF-16 encodings as decode-only and maps their *output* encoding
        // to UTF-8, so `UTF_16LE.encode(text)` returns UTF-8 bytes with `had_errors` false. Writing
        // that behind a UTF-16 byte-order mark produces a file no reader can make sense of, reported
        // as a successful save — the same class of quiet corruption this ticket exists to refuse,
        // wearing a different hat. Found by a test asserting the bytes rather than the absence of an
        // error.
        //
        // Doing it by hand is safe in a way the legacy encodings are not: UTF-16 can represent every
        // `char`, and a `&str` cannot contain an unpaired surrogate, so there is nothing to map badly
        // and no `had_errors` case to consider.
        if let Some(units) = utf16_order(self.encoding) {
            let mut out = Vec::with_capacity(self.bom_bytes().len() + text.len() * 2);
            out.extend_from_slice(self.bom_bytes());
            for unit in text.encode_utf16() {
                out.extend_from_slice(&match units {
                    Utf16Order::Little => unit.to_le_bytes(),
                    Utf16Order::Big => unit.to_be_bytes(),
                });
            }
            return Ok(out);
        }

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

/// Which way round a UTF-16 encoding puts its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Utf16Order {
    Little,
    Big,
}

/// Whether `encoding` is one of the two UTF-16s, and which way round.
///
/// By name rather than by pointer comparison, because that is how the rest of this module already
/// asks the question — see `bom_bytes`.
fn utf16_order(encoding: &'static Encoding) -> Option<Utf16Order> {
    match encoding.name() {
        "UTF-16LE" => Some(Utf16Order::Little),
        "UTF-16BE" => Some(Utf16Order::Big),
        _ => None,
    }
}

/// One encoding a user may pick, and what to call it.
///
/// A `FileFormat` in miniature: an encoding and whether it carries a byte-order mark, which are two
/// separate facts that together make "UTF-8" and "UTF-8 with BOM" different answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// What the menu calls it.
    pub label: &'static str,
    encoding: &'static Encoding,
    bom: bool,
}

impl Choice {
    /// Whether this is the encoding `format` already uses.
    #[must_use]
    pub fn matches(&self, format: FileFormat) -> bool {
        std::ptr::eq(self.encoding, format.encoding) && self.bom == format.bom
    }
}

/// The encodings the editor offers, in the order the menu shows them.
///
/// **A short practical list rather than everything `encoding_rs` knows.** The Encoding Standard has
/// around forty labels, most of which nobody has seen a file in this century; a menu of forty is a
/// menu nobody reads. These are UTF-8 with and without a BOM, both UTF-16 byte orders, and the three
/// legacy encodings a Windows user is most likely to meet a file in.
///
/// **The order is the contract.** A caller identifies a choice by its position — that is what a menu
/// click reports — so reordering this is an API change, and `crates/file/tests/encodings.rs` says so.
#[must_use]
pub fn choices() -> [Choice; 7] {
    [
        Choice {
            label: "UTF-8",
            encoding: UTF_8,
            bom: false,
        },
        Choice {
            label: "UTF-8 with BOM",
            encoding: UTF_8,
            bom: true,
        },
        Choice {
            label: "UTF-16 LE",
            encoding: encoding_rs::UTF_16LE,
            bom: true,
        },
        Choice {
            label: "UTF-16 BE",
            encoding: encoding_rs::UTF_16BE,
            bom: true,
        },
        Choice {
            label: "windows-1252",
            encoding: encoding_rs::WINDOWS_1252,
            bom: false,
        },
        Choice {
            label: "GBK",
            encoding: encoding_rs::GBK,
            bom: false,
        },
        Choice {
            label: "Shift_JIS",
            encoding: encoding_rs::SHIFT_JIS,
            bom: false,
        },
    ]
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
    /// Milliseconds since the Unix epoch, or `None` when the filesystem would not say.
    ///
    /// Milliseconds rather than a `SystemTime` so this survives being written to a file and read
    /// back **exactly** — MDEDIT-66 stores it alongside a buffer's stored copy, and a stamp that
    /// changed slightly on the way through JSON would report every restored file as modified.
    ///
    /// No loss worth naming: the comparison already tolerates timestamps far coarser than this, as
    /// this type's own documentation explains, and a length is the other half of the answer.
    modified_ms: Option<u64>,
    len: u64,
}

impl Stamp {
    /// Reads the stamp of whatever is at `path`, or `None` if it cannot be looked at.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        Some(Self::from_metadata(&fs::metadata(path).ok()?))
    }

    /// The stamp `metadata` describes.
    ///
    /// Always answers: a filesystem that will not report a modification time gives `None` for that
    /// half and the length still stands, which is a weaker check rather than no check.
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            modified_ms: metadata.modified().ok().and_then(|at| {
                at.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .and_then(|since| u64::try_from(since.as_millis()).ok())
            }),
            len: metadata.len(),
        }
    }

    /// The stamp as two plain numbers, for storing somewhere that has no notion of a time.
    #[must_use]
    pub fn parts(self) -> (Option<u64>, u64) {
        (self.modified_ms, self.len)
    }

    /// Rebuilds a stamp from [`parts`](Self::parts).
    ///
    /// Exact: the parts *are* the stamp, which is why the modification time is kept as a number
    /// rather than as a `SystemTime`.
    #[must_use]
    pub fn from_parts(modified_ms: Option<u64>, len: u64) -> Self {
        Self { modified_ms, len }
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
    file.stamp.map_or(DiskState::Unchanged, |recorded| {
        matches_stamp(path, recorded)
    })
}

/// Whether the file at `path` still looks as it did when `stamp` was taken.
///
/// What [`disk_state`] is, for a caller holding a stamp on its own rather than a whole
/// [`LoadedFile`] — which is the position anything restoring a *stored* buffer is in: it has the
/// text and the stamp it was written with, and no loaded file at all.
#[must_use]
pub fn matches_stamp(path: &Path, stamp: Stamp) -> DiskState {
    match fs::metadata(path) {
        Ok(metadata) if Stamp::from_metadata(&metadata) != stamp => DiskState::Changed,
        Err(error) if error.kind() == io::ErrorKind::NotFound => DiskState::Gone,
        // Either it matches, or the metadata will not read for a reason other than absence. The
        // second is "unchanged" for the reason `disk_state` documents: claiming a change on no
        // evidence costs the user a dialog, and being wrong this way costs nothing.
        _ => DiskState::Unchanged,
    }
}

/// A file, decoded, with everything needed to write it back unchanged.
#[derive(Debug, Clone)]
pub struct LoadedFile {
    text: String,
    format: FileFormat,
    original: Vec<u8>,
    round_trips: bool,
    /// Whether the encoding has been changed deliberately, so the original bytes are no longer what
    /// this file should be written as. See [`LoadedFile::set_encoding`].
    re_encode: bool,
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

    /// Changes the encoding the next save will write, and says whether anything changed (MDEDIT-42).
    ///
    /// # Why this has to disarm the byte-identical fast path
    ///
    /// [`save`] writes [`original_bytes`](Self::original_bytes) verbatim when the text is unedited,
    /// which is what makes an ordinary save byte-identical by construction. After a deliberate
    /// encoding change that shortcut is exactly wrong: the user has asked for *different* bytes and
    /// would get the old ones back with no indication that nothing happened. So this arms
    /// `re_encode`, and the fast path checks it.
    ///
    /// `round_trips` is set true for the same reason. It records whether this encoding can reproduce
    /// its own unedited bytes, which is a guard against rewriting parts nobody touched — and
    /// rewriting them is now the whole point. What protects the user instead is
    /// [`FileError::Unrepresentable`], which refuses the save if the new encoding cannot hold the
    /// text at all.
    pub fn set_encoding(&mut self, choice: Choice) -> bool {
        if choice.matches(self.format) {
            return false;
        }
        self.format.encoding = choice.encoding;
        self.format.bom = choice.bom;
        self.re_encode = true;
        self.round_trips = true;
        true
    }

    /// Records that this file's line endings are now `newline` (MDEDIT-59).
    ///
    /// The buffer is converted by the editor, which owns the undoable edit; this is the other half,
    /// so what the status bar says and what Enter inserts agree with the bytes. Called only after a
    /// conversion that actually happened.
    pub fn set_newline(&mut self, newline: Newline) {
        self.format.newline = newline;
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

/// A [`LoadedFile`] for text that has never been on disk and is not going there.
///
/// For a buffer the editor generates rather than reads — the keyboard-shortcuts reference is the
/// only one today. Such a buffer needs *something* to compare against or it counts as unsaved work
/// the moment it has anything in it, and would ask to be saved on the way out; giving it its own
/// text as its record of disk makes it unmodified, which is what it is.
///
/// UTF-8, no BOM, and no [`Stamp`] — there is no file to have one, so [`disk_state`] answers
/// `Unchanged` for ever, which is true.
#[must_use]
pub fn in_memory(text: &str) -> LoadedFile {
    LoadedFile {
        format: FileFormat {
            encoding: UTF_8,
            bom: false,
            newline: Newline::detect(text),
        },
        original: text.as_bytes().to_vec(),
        text: text.to_owned(),
        round_trips: true,
        re_encode: false,
        stamp: None,
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

/// Reads a file as `choice` says, for a caller that has one rather than an [`Encoding`].
///
/// The whole point of [`Choice`] is that a caller can offer a menu of encodings without depending on
/// `encoding_rs` — so it would be self-defeating to make them unwrap one to read a file.
///
/// # Errors
///
/// As [`load_as`].
pub fn load_with(path: &Path, choice: Choice) -> Result<LoadedFile, FileError> {
    load_as(path, choice.encoding)
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
        re_encode: false,
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
    // `re_encode` disarms the fast path: after a deliberate encoding change the original bytes are
    // the one thing that must *not* be written, and without this an unedited buffer would come back
    // in its old encoding with nothing to say the choice had been ignored.
    let bytes = if text == file.text && !file.re_encode {
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
