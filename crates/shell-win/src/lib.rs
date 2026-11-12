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
//! **There is no mutex here, and there must not be one.** Launching `mdedit` with no arguments
//! twice by hand still opens two windows. Explorer reuses a process because it asks COM for the class
//! object and COM hands it the one that is already registered.
//!
//! **A launch that names files is different** (MDEDIT-204): `mdedit a.md b.md` while an editor is
//! running hands the paths to it ([`hand_off`]) and exits, instead of opening a second window. Two
//! processes would write one session file and one `recent.json`, last writer wins, and a tab opened
//! in the second would vanish from the first's session; and the VS Code pop-out (MDEDIT-205) wants
//! "open this file in the editor" to mean the editor that is already there. Liveness is a Running
//! Object Table entry the server publishes, which is a signal and not a lock: nothing it says can
//! stop a second process starting. See `docs/decisions/platform.md`.
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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

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
    /// The running editor did not answer within [`HAND_OFF_TIMEOUT`].
    #[error("the running editor did not answer in time")]
    TimedOut,
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

/// How long a launch waits for a running editor to take its files before starting normally.
///
/// A local COM round trip to a responsive UI thread is milliseconds, and an editor in the middle of
/// loading a large file can be busy for a second or so. Three seconds covers that while still being
/// a wait a person at a terminal will not mistake for a hang. The cost of being too short is a
/// second window; of being too long, a launcher that looks frozen.
pub const HAND_OFF_TIMEOUT: Duration = Duration::from_secs(3);

/// What came of offering files to a running editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandedOff {
    /// A running editor took them. This process has nothing left to do.
    Delivered,
    /// A call into the running editor was made and its outcome is not known (it failed after
    /// the files may have been delivered, or it had not answered when the wait ran out). The text
    /// says why, for the log. **Treated as delivered**: opening twice is the worse failure.
    Uncertain(String),
    /// No editor is running for this user, so the files are this process's to open.
    NobodyListening,
}

/// Which side of a hand-off gets to decide what happened to the files.
///
/// Exactly one of the sender's wait expiring and the worker starting its call wins, because the
/// alternative (the sender gives up and opens the files itself while the worker's call lands
/// later) opens them twice.
#[derive(Debug)]
pub(crate) struct Claim(AtomicU8);

impl Claim {
    const IDLE: u8 = 0;
    const DELIVERING: u8 = 1;
    const ABANDONED: u8 = 2;

    fn new() -> Self {
        Self(AtomicU8::new(Self::IDLE))
    }

    /// The worker's: true if it may make the call that carries the files.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn begin_delivery(&self) -> bool {
        self.0
            .compare_exchange(
                Self::IDLE,
                Self::DELIVERING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// The sender's, on expiry: true if the worker never started, so nothing is in flight.
    fn abandon(&self) -> bool {
        self.0
            .compare_exchange(
                Self::IDLE,
                Self::ABANDONED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

/// Runs `work` on its own thread and waits at most `limit` for its answer.
///
/// COM calls into another process block until it replies, and there is no timeout on them. The
/// thread is abandoned on expiry rather than cancelled; the process is about to either exit or
/// become the editor, and a stuck call cannot outlive it.
///
/// On expiry, if `work` never claimed delivery this is [`Error::TimedOut`] and the worker will
/// skip its call. If it had, the files are in flight to an editor that has not answered, and the
/// answer is [`HandedOff::Uncertain`]: **no fallback**, because that editor has them or will. The
/// case left is an editor that then hangs forever: the person sees nothing, runs the command
/// again, and gets a window after the next wait.
#[cfg_attr(not(windows), allow(dead_code))]
fn bounded(
    limit: Duration,
    work: impl FnOnce(&Claim) -> Result<HandedOff, Error> + Send + 'static,
) -> Result<HandedOff, Error> {
    let claim = Arc::new(Claim::new());
    let worker = Arc::clone(&claim);
    let (send, receive) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("hand-off".into())
        .spawn(move || {
            let _ = send.send(work(&worker));
        });
    if spawned.is_err() {
        return Err(Error::Com("could not start the hand-off thread".into()));
    }
    match receive.recv_timeout(limit) {
        Ok(answer) => answer,
        Err(_) if claim.abandon() => Err(Error::TimedOut),
        Err(_) => Ok(HandedOff::Uncertain(
            "the running editor had not answered when the wait ran out".into(),
        )),
    }
}

/// A command line, sorted into what this process should treat as what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// COM started this process rather than a person.
    pub embedding: bool,
    /// Everything else, as given. `--register` and `--unregister` are handled before this exists.
    pub paths: Vec<PathBuf>,
}

/// Sorts arguments after the program name into a [`Launch`].
pub fn classify(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Launch {
    let mut launch = Launch {
        embedding: false,
        paths: Vec::new(),
    };
    for argument in arguments {
        if argument.to_str().is_some_and(is_embedding) {
            launch.embedding = true;
        } else {
            launch.paths.push(argument.into());
        }
    }
    launch
}

/// What a launch ended up doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A running editor has the files; exit successfully.
    HandedOver,
    /// Become the editor.
    StartHere,
}

/// Decides between handing files to a running editor and starting one.
///
/// Only a launch that names files, and that a person made, tries. `resolve` turns a path as typed
/// into the absolute one the other process can use, or `None` to leave it out; if none survive there
/// is nothing to hand over and this process starts normally, which reports the typo as it always did.
/// **Any failure to hand over is a start**: the user always gets their file.
pub fn decide(
    launch: &Launch,
    resolve: impl Fn(&Path) -> Option<PathBuf>,
    hand_off: impl FnOnce(&[PathBuf]) -> Result<HandedOff, Error>,
) -> Outcome {
    if launch.embedding || launch.paths.is_empty() {
        return Outcome::StartHere;
    }
    let sendable: Vec<PathBuf> = launch.paths.iter().filter_map(|p| resolve(p)).collect();
    if sendable.is_empty() {
        return Outcome::StartHere;
    }
    match hand_off(&sendable) {
        Ok(HandedOff::Delivered | HandedOff::Uncertain(_)) => Outcome::HandedOver,
        Ok(HandedOff::NobodyListening) | Err(_) => Outcome::StartHere,
    }
}

/// Offers `paths` (absolute) to the editor already running for this user, if there is one.
///
/// Bounded by [`HAND_OFF_TIMEOUT`]. If the wait expires before the call into the editor started,
/// nothing is sent and the answer is [`Error::TimedOut`]; if the call had started, the answer is
/// [`HandedOff::Uncertain`] and the caller must not open the files itself.
///
/// # Errors
///
/// [`Error::TimedOut`] if the editor did not answer in time, [`Error::Com`] if it answered with a
/// failure. Either way the caller should open the files itself.
#[cfg(windows)]
pub fn hand_off(paths: &[PathBuf]) -> Result<HandedOff, Error> {
    hand_off_within(paths, HAND_OFF_TIMEOUT, &com::CLSID_DROP_TARGET)
}

#[cfg(windows)]
fn hand_off_within(
    paths: &[PathBuf],
    limit: Duration,
    class: &windows_core::GUID,
) -> Result<HandedOff, Error> {
    let paths = paths.to_vec();
    let class = *class;
    bounded(limit, move |claim| {
        com::hand_off_class(&paths, claim, &class)
    })
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

    /// There is no running Windows editor to hand to.
    ///
    /// # Errors
    ///
    /// Never; the signature matches the Windows one.
    pub fn hand_off(_paths: &[std::path::PathBuf]) -> Result<super::HandedOff, Error> {
        Ok(super::HandedOff::NobodyListening)
    }

    /// Nothing, held so the caller compiles.
    #[derive(Debug)]
    pub struct Server;

    impl Server {
        /// Why a second launch cannot find this process, if it cannot.
        #[must_use]
        pub fn hand_off_unavailable(&self) -> Option<&str> {
            None
        }
    }
}

#[cfg(not(windows))]
pub use elsewhere::{Server, hand_off, register, serve, unregister};

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{Error, HandedOff, Outcome, bounded, classify, decide, is_embedding};

    fn launch(args: &[&str]) -> super::Launch {
        classify(args.iter().map(OsString::from))
    }

    #[test]
    fn classify_separates_com_s_switch_from_paths() {
        let found = launch(&["a.md", "-Embedding", "b.md"]);
        assert!(found.embedding);
        assert_eq!(
            found.paths,
            vec![PathBuf::from("a.md"), PathBuf::from("b.md")]
        );
        assert!(!launch(&["a.md"]).embedding);
        assert!(launch(&[]).paths.is_empty());
    }

    /// Hand-off decisions: (arguments, what the hand-off answers, outcome, whether it was asked).
    #[test]
    fn decide_table() {
        type Answer = fn() -> Result<HandedOff, Error>;
        let delivered: Answer = || Ok(HandedOff::Delivered);
        let nobody: Answer = || Ok(HandedOff::NobodyListening);
        let failed: Answer = || Err(Error::TimedOut);
        let cases: [(&[&str], Answer, Outcome, bool); 7] = [
            (&["a.md"], delivered, Outcome::HandedOver, true),
            (&["a.md", "b.md"], delivered, Outcome::HandedOver, true),
            (&["a.md"], nobody, Outcome::StartHere, true),
            (&["a.md"], failed, Outcome::StartHere, true),
            (&[], delivered, Outcome::StartHere, false),
            (&["-Embedding"], delivered, Outcome::StartHere, false),
            (
                &["-Embedding", "a.md"],
                delivered,
                Outcome::StartHere,
                false,
            ),
        ];
        for (args, answer, expected, asked) in cases {
            let mut was_asked = false;
            let got = decide(
                &launch(args),
                |path| Some(path.to_path_buf()),
                |_| {
                    was_asked = true;
                    answer()
                },
            );
            assert_eq!(got, expected, "{args:?}");
            assert_eq!(was_asked, asked, "{args:?}");
        }
    }

    #[test]
    fn rejected_paths_are_not_sent_and_none_left_means_no_attempt() {
        let only_good = |path: &Path| (path == Path::new("good.md")).then(|| path.to_path_buf());
        let mut sent = Vec::new();
        let got = decide(&launch(&["bad.md", "good.md"]), only_good, |paths| {
            sent = paths.to_vec();
            Ok(HandedOff::Delivered)
        });
        assert_eq!(got, Outcome::HandedOver);
        assert_eq!(sent, vec![PathBuf::from("good.md")]);

        let mut asked = false;
        let got = decide(&launch(&["bad.md"]), only_good, |_| {
            asked = true;
            Ok(HandedOff::Delivered)
        });
        assert_eq!(got, Outcome::StartHere);
        assert!(!asked, "nothing is left to send");
    }

    #[test]
    fn a_prompt_answer_is_returned() {
        let answer = bounded(Duration::from_secs(5), |_| Ok(HandedOff::Delivered));
        assert_eq!(answer.ok(), Some(HandedOff::Delivered));
    }

    /// The sender's wait expires before the worker has started its call: the sender wins, the worker
    /// is refused its call, and the caller starts normally.
    #[test]
    fn when_the_sender_wins_the_worker_skips_delivery() {
        let (tell, hear) = std::sync::mpsc::channel();
        let answer = bounded(Duration::from_millis(50), move |claim| {
            std::thread::sleep(Duration::from_millis(500));
            let _ = tell.send(claim.begin_delivery());
            Ok(HandedOff::Delivered)
        });
        assert!(matches!(answer, Err(Error::TimedOut)), "{answer:?}");
        assert_eq!(hear.recv_timeout(Duration::from_secs(5)), Ok(false));
    }

    /// The worker has claimed delivery and the editor has not answered: no fallback.
    #[test]
    fn when_the_worker_wins_the_sender_does_not_fall_back() {
        let answer = bounded(Duration::from_millis(50), |claim| {
            let claimed = claim.begin_delivery();
            std::thread::sleep(Duration::from_millis(500));
            if claimed {
                Ok(HandedOff::Delivered)
            } else {
                Err(Error::TimedOut)
            }
        });
        assert!(matches!(answer, Ok(HandedOff::Uncertain(_))), "{answer:?}");
        let outcome = decide(
            &launch(&["a.md"]),
            |p| Some(p.to_path_buf()),
            |_| Ok(HandedOff::Uncertain(String::new())),
        );
        assert_eq!(outcome, Outcome::HandedOver);
    }

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
