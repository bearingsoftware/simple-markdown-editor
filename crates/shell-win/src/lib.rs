//! Windows shell integration: one process for a whole Explorer selection.
//!
//! # Why this is a crate, and the third one
//!
//! `AGENTS.md` permits `unsafe` in a platform crate and makes it a review block anywhere else, and
//! says a third such crate needs a reason on the ticket rather than a `SAFETY` comment. The reason
//! is MDEDIT-61's: COM class registration and `IDropTarget` are unsafe Win32 by nature, and neither
//! menus (`mdedit-menu-win`) nor text (`mdedit-text-dwrite`) is where shell integration belongs.
//!
//! # The thing this exists to avoid
//!
//! A file type can name its handler two ways. The obvious one is a **command-line verb**:
//! `shell\open\command` holding `"mdedit.exe" "%1"`. That defaults to `MultiSelectModel=Document`,
//! which means Explorer runs the command **once per selected file** — fifteen files, fifteen
//! processes, each with its own window, its own GPU device and its own session file — and past
//! fifteen it stops invoking the verb at all, silently. Notepad++ ships that bug and has since 2023.
//!
//! The other is a **`DropTarget` verb**: the type points at a CLSID, the CLSID points at a
//! `LocalServer32`, and Explorer activates it through COM. Because a running instance has published
//! its class object, the activation binds to **the process already running** and arrives as one
//! `IDropTarget::Drop` carrying the entire selection. One process, one window, twenty tabs, in
//! selection order.
//!
//! `AGENTS.md` states the requirement and names the failure mode; this is the implementation of it.
//!
//! # Single instance is not what this is
//!
//! **There is no mutex here, and there must not be one.** Launching `mdedit` twice by hand still
//! opens two windows, exactly as it does today. The reuse comes from the verb: Explorer asks COM for
//! the class object, and COM hands it the one that is already registered. Anything that made a second
//! hand-launch impossible would be a different feature, and a worse one.
//!
//! # Registration is manual, per-user, and reversible
//!
//! `mdedit --register` and `mdedit --unregister`. Nothing happens at startup and nothing happens at
//! install, because there is no installer — the gap register parks the installer and updater
//! deliberately, and the updater's signature requirements are the highest-consequence security
//! decision in the project. Keeping this manual is what stops half of one growing here by accident.
//!
//! Everything is written under `HKEY_CURRENT_USER`, which needs no elevation by construction.
//! `AGENTS.md`: **per-user install, never elevate** — elevating breaks drag-and-drop from a
//! non-elevated Explorer, and the documented mitigation is one Microsoft says is not safe.
//!
//! # Where paths from here sit in the trust model
//!
//! A path arriving through `Drop` is Explorer handing over the selection the user just made. That is
//! the same source as a command-line argument, not the same as a session file this process wrote —
//! so it goes through `mdedit_session::path::from_command_line`, which resolves and checks the
//! result is a file, rather than through `validate`, whose rules exist to defend against a file the
//! editor itself wrote being edited by something else. The distinction is `mdedit-session`'s and is
//! written down there.

pub mod registry;

#[cfg(windows)]
mod com;

use std::path::PathBuf;

/// Why shell integration could not do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A registry key could not be written or removed.
    #[error("registry: {0}")]
    Registry(String),
    /// COM refused: the class object could not be published, or the runtime is unavailable.
    #[error("COM: {0}")]
    Com(String),
    /// The running executable's own path could not be determined, so there is nothing to register.
    #[error("could not find this executable: {0}")]
    Executable(std::io::Error),
    /// Not Windows. There is no shell to integrate with.
    #[error("shell integration is Windows-only")]
    Unsupported,
}

/// What Explorer's selection turned into.
pub type Deliver = Box<dyn Fn(Vec<PathBuf>) + Send + Sync + 'static>;

/// The argument COM adds when it launches a server rather than a person doing it.
///
/// It must never be mistaken for a file to open — it is not a path, and treating it as one would
/// make every Explorer-launched instance report that it could not open a file called `-Embedding`.
pub const EMBEDDING: &str = "-Embedding";

/// Whether an argument is COM's rather than the user's.
///
/// Case-insensitive because the shell's own documentation writes it both ways, and a leading `/` is
/// accepted for the same reason: this is a switch the system supplies, and being strict about its
/// spelling gains nothing and risks opening a tab named after it.
#[must_use]
pub fn is_embedding(argument: &str) -> bool {
    let argument = argument.strip_prefix(['-', '/']).unwrap_or(argument);
    argument.eq_ignore_ascii_case(EMBEDDING.trim_start_matches('-'))
}

/// Writes the footprint in [`registry::plan`], pointing at `exe`.
///
/// Idempotent: every step is a write of a known value, so running it twice leaves what running it
/// once did.
///
/// # Errors
///
/// [`Error::Registry`] if a key cannot be created or written, which under `HKEY_CURRENT_USER` means
/// something is wrong with the profile rather than with permissions.
#[cfg(windows)]
pub fn register(exe: &std::path::Path) -> Result<(), Error> {
    com::apply(&registry::plan(exe))
}

/// Removes everything [`register`] wrote, and nothing else.
///
/// Owned keys go with their subtrees; a value written into a key the system owns is removed on its
/// own, leaving the key. See [`registry::Step`].
///
/// # Errors
///
/// [`Error::Registry`] if a key exists and cannot be removed. A key that is already absent is not an
/// error — unregistering twice is not a failure, and neither is unregistering something that was
/// never registered.
#[cfg(windows)]
pub fn unregister() -> Result<(), Error> {
    // The path only ever reaches values, never key names, so any path produces the same footprint to
    // remove. Using the running executable would make unregistering from a moved copy miss.
    com::remove(&registry::plan(std::path::Path::new("")))
}

/// Publishes the class object, so Explorer activations land in this process.
///
/// Hold the returned [`Server`] for as long as the process should answer them; dropping it revokes
/// the class and any further activation starts a new process instead.
///
/// # Errors
///
/// [`Error::Com`] if COM cannot be initialised on this thread, or the class object is refused.
#[cfg(windows)]
pub fn serve(deliver: Deliver) -> Result<Server, Error> {
    com::serve(deliver)
}

/// A published class object. Revokes on drop.
#[cfg(windows)]
pub use com::Server;

#[cfg(not(windows))]
mod elsewhere {
    use super::{Deliver, Error};

    /// There is no Windows registry to write to.
    ///
    /// # Errors
    ///
    /// Always [`Error::Unsupported`].
    pub fn register(_exe: &std::path::Path) -> Result<(), Error> {
        Err(Error::Unsupported)
    }

    /// There is nothing registered to remove.
    ///
    /// # Errors
    ///
    /// Always [`Error::Unsupported`].
    pub fn unregister() -> Result<(), Error> {
        Err(Error::Unsupported)
    }

    /// There is no shell to publish a class object to.
    ///
    /// # Errors
    ///
    /// Always [`Error::Unsupported`].
    pub fn serve(_deliver: Deliver) -> Result<Server, Error> {
        Err(Error::Unsupported)
    }

    /// Nothing, held so the caller compiles.
    #[derive(Debug)]
    pub struct Server;
}

#[cfg(not(windows))]
pub use elsewhere::{Server, register, serve, unregister};

#[cfg(test)]
mod tests {
    use super::is_embedding;

    /// COM's own argument, in the spellings the shell uses, must never reach the path parser.
    #[test]
    fn com_s_argument_is_recognised_however_it_is_spelled() {
        assert!(is_embedding("-Embedding"));
        assert!(is_embedding("/Embedding"));
        assert!(is_embedding("-embedding"));
        assert!(is_embedding("Embedding"));
    }

    /// And a real file is not mistaken for it. `embedding.md` is a document somebody might have.
    #[test]
    fn a_file_named_like_it_is_still_a_file() {
        assert!(!is_embedding("embedding.md"));
        assert!(!is_embedding("C:\\notes\\Embedding.md"));
        assert!(!is_embedding("--embedding"));
    }
}
