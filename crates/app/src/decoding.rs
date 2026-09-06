//! Where a picture's pixels come from, and the thread they come from (MDEDIT-156).
//!
//! Not to be confused with [`crate::image`], which is about *finding* an image in the document and
//! placing the rectangle it gets. This is about the bytes behind it.
//!
//! # The split, and why the layout cache survives it
//!
//! Decoding a 4K PNG costs 65 ms on the best run ever measured on this project's machine and about
//! 250 ms on a warm laptop, against a 50 ms first-paint target — the measurement is in
//! `docs/research/06-performance-targets.md`. It used to happen on the thread that draws, because
//! `AGENTS.md` claimed a worker existed and MDEDIT-146 built around it. MDEDIT-151 corrected the
//! claim; this is the worker.
//!
//! The move works because the two halves of a picture are wanted at different times and only one of
//! them is expensive. **A row's geometry comes from the header** — `mdedit_image::measure`, a few
//! dozen bytes — so the paragraph is the right height on the frame the image first appears in.
//! **Only the pixels are decoded off-thread**, and the draw already skips a picture whose id has no
//! texture yet, so a blank row that fills in a frame or two later needed no new branch anywhere.
//!
//! That is also what keeps this away from MDEDIT-118's trap. `ChunkKey` does not carry decode state,
//! and a cached layout that changed with something the key does not carry is exactly the
//! reveal-on-caret defect. Here the layout is a function of the header alone and does not change
//! when the pixels land, so there is nothing for the key to be missing.
//!
//! # The trust boundary does not move
//!
//! `mdedit_image::resolve` runs on the drawing thread, before anything is queued, and it is what
//! stands between a document you were sent and a UNC path that leaks an NTLM hash. The worker is
//! handed a path that has already been resolved, normalised and accepted, and only reads it. It
//! decides nothing about what a destination may name.
//!
//! Failures come back as values and are logged by the editor when it collects them, rather than by
//! the worker. One thread writes the log, and the lines stay in an order that matches the frames.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime};

use mdedit_image::{Picture, Undecodable};
use winit::event_loop::EventLoopProxy;

use crate::editor::Wake;

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

/// What the editor knows about one picture.
#[derive(Debug, Clone)]
pub(crate) enum PictureState {
    /// Refused, missing, or not a usable PNG.
    ///
    /// Cached like any other answer, so a document naming a file that is not there costs one
    /// attempt and one line in the log rather than one of each per layout.
    Unusable,

    /// The header has been read. `pixels` is `None` until the worker answers.
    ///
    /// **The size is the header's, and it stays the header's even after the pixels arrive.** The
    /// promise this whole change makes is that nothing moves when a picture appears, and adopting
    /// the decoded dimensions would break it for exactly the file that had been rewritten between
    /// the two reads — see MDEDIT-150 for noticing that at all.
    Known {
        width: u32,
        height: u32,
        pixels: Option<std::rc::Rc<Picture>>,
    },
}

impl PictureState {
    /// The size to lay a row out at, or `None` if there will never be a picture.
    pub(crate) fn size(&self) -> Option<(u32, u32)> {
        match *self {
            Self::Unusable => None,
            Self::Known { width, height, .. } => Some((width, height)),
        }
    }

    /// The pixels, if they have arrived.
    pub(crate) fn pixels(&self) -> Option<&std::rc::Rc<Picture>> {
        match self {
            Self::Unusable => None,
            Self::Known { pixels, .. } => pixels.as_ref(),
        }
    }
}

/// One thread that reads and decodes pictures, and the two channels to it.
///
/// Spawned on the first picture the editor lays out rather than at startup, so the great majority
/// of sessions — and every headless test that never places one — never start a thread at all.
///
/// **Nothing here is joined on shutdown.** Dropping the editor drops `requests`, the worker's `for`
/// loop ends on the closed channel, and the thread finishes on its own. A decode still running at
/// that point finishes into a channel nobody reads, which costs the milliseconds it was already
/// going to cost and nothing else.
#[derive(Debug)]
pub(crate) struct Decoder {
    requests: Sender<PictureKey>,
    answers: Receiver<(PictureKey, Result<Picture, Undecodable>)>,
}

impl Decoder {
    /// Starts the worker, waking the event loop through `wake` whenever an answer is ready.
    ///
    /// `wake` is `None` in a test, which has no event loop to wake; the answers still arrive and
    /// are collected by [`Decoder::wait`].
    pub(crate) fn spawn(wake: Option<EventLoopProxy<Wake>>) -> Self {
        let (requests, inbox) = mpsc::channel::<PictureKey>();
        let (outbox, answers) = mpsc::channel();

        let worker = move || {
            for key in inbox {
                let decoded = mdedit_image::read(&key.path);
                if outbox.send((key, decoded)).is_err() {
                    // The editor is gone. So is the reason to decode anything else.
                    return;
                }
                if let Some(wake) = wake.as_ref() {
                    // A closed event loop is a window on its way out, not an error to report.
                    let _ = wake.send_event(Wake::Pictures);
                }
            }
        };
        // Named, because a thread that shows up in a debugger or a crash report without one is a
        // small mystery at exactly the wrong moment.
        let spawned = std::thread::Builder::new()
            .name("mdedit-picture-decode".to_owned())
            .spawn(worker);
        if let Err(error) = spawned {
            // A machine that will not give us a thread still draws documents: every picture stays
            // at `pixels: None`, which is a blank row rather than a crash.
            crate::log_error!("could not start the picture decoder: {error}");
        }

        Self { requests, answers }
    }

    /// Asks for `key` to be decoded.
    ///
    /// Silent if the worker is gone, because the caller's alternative is to draw nothing — which is
    /// what happens anyway.
    pub(crate) fn request(&self, key: PictureKey) {
        let _ = self.requests.send(key);
    }

    /// Every answer that has arrived, without waiting for one.
    pub(crate) fn collect(&self) -> Vec<(PictureKey, Result<Picture, Undecodable>)> {
        self.answers.try_iter().collect()
    }

    /// Waits up to `patience` for the first answer, then takes any others that came with it.
    ///
    /// For a caller with no event loop to be woken by — a headless test, which blocks on the answer
    /// rather than on a clock and so is not the flaky kind.
    pub(crate) fn wait(
        &self,
        patience: Duration,
    ) -> Vec<(PictureKey, Result<Picture, Undecodable>)> {
        match self.answers.recv_timeout(patience) {
            Ok(first) => {
                let mut answers = vec![first];
                answers.extend(self.answers.try_iter());
                answers
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Vec::new(),
        }
    }
}
