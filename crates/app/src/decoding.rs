//! Where a picture's pixels come from.
//!
//! [`PictureKey`] identifies a decoded picture, and that is all this module holds today. It exists
//! as a module rather than as three types in `editor.rs` because MDEDIT-156 is about to add the
//! thread that fills the cache, and moving the key first keeps that diff to the change it makes.
//!
//! Not to be confused with [`crate::image`], which is about *finding* an image in the document and
//! placing the rectangle it gets. This is about the bytes behind it.

use std::path::PathBuf;
use std::time::SystemTime;

/// What identifies a decoded picture (MDEDIT-146).
///
/// The resolved path, plus the stamp of the bytes read from it. The stamp is what makes replacing a
/// file behind the editor's back safe rather than merely unlikely: a cache on the path alone would
/// hand back the old picture for the new bytes, and it costs one `metadata` call that
/// `mdedit_image::read` has to make anyway.
///
/// **The same length-and-mtime pair `mdedit-file` stamps a loaded buffer with**, and it is weak for
/// the same reason and to the same degree: a filesystem with a coarse clock can miss a rewrite
/// within its tick. That is a stale picture, not a wrong document, so it is a considerably cheaper
/// failure here than there.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PictureKey {
    pub(crate) path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

impl PictureKey {
    /// The key for `path` as it is on disk right now, or `None` if it cannot be stated.
    pub(crate) fn of(path: PathBuf) -> Option<Self> {
        let metadata = std::fs::metadata(&path).ok()?;
        Some(Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            path,
        })
    }
}
