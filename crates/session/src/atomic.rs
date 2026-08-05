//! Writing a state file so an interrupted write cannot destroy what was there.
//!
//! Every file this crate owns is state the editor rebuilds its world from at startup: the session,
//! the recovery manifest and its snapshots, the recent-files list. A truncated one is worse than an
//! absent one — `restore_session` reads a half-written session as `Malformed`, which loses the tab
//! list *and* the clean-exit marker, so the next launch also calls the previous quit a crash.
//!
//! Two of the three did temp-then-rename and the session file did not, while `recent.rs` claimed in
//! a comment that all three did (MDEDIT-106). One copy now, so the claim and the code cannot part
//! company again.

use std::io;
use std::path::Path;

/// Writes `bytes` to `path` via a temporary file and a rename.
///
/// The rename is the atomic step: it either happens or does not, so a crash or a full disk part way
/// through leaves the previous contents rather than a truncated file.
///
/// The parent directory is created if it is missing, because every caller here writes into a
/// directory that may not exist yet on a first run.
///
/// # Errors
///
/// Any I/O failure creating the directory, writing the temporary file, or renaming it.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = path.to_path_buf();
    temporary.set_extension("tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}
