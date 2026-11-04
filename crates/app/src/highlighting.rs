//! Where a fence's colours come from, and the thread they come from (MDEDIT-190).
//!
//! Highlighting is the second thing in this editor that is too slow for the thread that draws.
//! `mdedit_highlight` measured, in a release build, **54 ms to load the syntax set** and about
//! **4 us a byte** to colour code: a one-character edit in a 300-line fence cost 169 ms at p50
//! against an 8.3 ms p99 target. `AGENTS.md` asks for a number from a cost harness before a second
//! worker, and this is that number. The shape deliberately mirrors [`crate::decoding`].
//!
//! # The split
//!
//! **The drawing thread never loads the syntax set and never parses a line of code.** It hashes a
//! fence's info string and body, asks the editor's store, and draws plain until an answer is there.
//! `mdedit_highlight::highlight` is called from the worker and from nowhere else, and a test in its
//! own binary (`tests/isolated/highlight_not_loaded.rs`) holds the drawing thread to that.
//!
//! # Stale work
//!
//! Typing in a big fence makes a new key on every keystroke. The worker blocks for one request,
//! drains whatever else is queued, and **highlights only the newest request per fence**. Each one it
//! skipped is answered [`Answer::Superseded`] rather than forgotten, so the editor can drop its
//! `Pending` entry — otherwise undoing back to those bytes would wait on an answer that was never
//! coming. No answer ever carries spans for a body other than the one its key hashes.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use mdedit_language::Token;
use mdedit_offsets::ByteSpan;
use winit::event_loop::EventLoopProxy;

use crate::editor::Wake;

/// What identifies one fence's text: its info string and its body.
///
/// 64 bits of `DefaultHasher`, as the layout cache's key is. A collision would colour a fence with
/// another's spans, which is a wrong colour and not a wrong byte.
pub(crate) fn key_of(info: &str, body: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    info.hash(&mut hasher);
    body.hash(&mut hasher);
    hasher.finish()
}

/// Spans relative to a fence's body.
pub(crate) type Spans = std::sync::Arc<[(ByteSpan, Token)]>;

/// What the editor knows about one fence's text.
#[derive(Debug, Clone)]
pub(crate) enum Highlight {
    /// Asked for; draws plain until the answer lands.
    Pending,
    Ready(Spans),
    /// Nothing to colour: no such language, or past the size bound.
    Plain,
}

/// A fence to colour. `fence` says which fence, so a newer request can replace an older one.
#[derive(Debug)]
pub(crate) struct Request {
    pub(crate) key: u64,
    pub(crate) fence: usize,
    pub(crate) info: String,
    pub(crate) body: String,
}

/// What the worker says back.
#[derive(Debug)]
pub(crate) enum Answer {
    /// The spans for `key`, or `None` if the info string names no language.
    Spans(u64, Option<Vec<(ByteSpan, Token)>>),
    /// A newer request for the same fence replaced `key` before it was started.
    Superseded(u64),
}

/// What colours a fence: `mdedit_highlight::highlight`, or a stand-in in a test.
pub(crate) type Job = crate::editor::HighlightJob;

struct Worker {
    job: Job,
    inbox: Receiver<Request>,
    outbox: Sender<Answer>,
    wake: Option<EventLoopProxy<Wake>>,
}

impl Worker {
    /// Runs one job, and **survives it panicking**: the fence is answered plain and the thread lives
    /// on, because a dead worker would leave every later fence plain for the rest of the session.
    /// The workspace unwinds (`panic = "abort"` is deliberately not set), so this can be caught.
    fn run_job(&self, request: &Request) -> Option<Vec<(ByteSpan, Token)>> {
        let job = self.job;
        std::panic::catch_unwind(|| job(&request.info, &request.body)).unwrap_or_else(|_| {
            crate::log_error!("the highlighter panicked on a fence; drawing it plain");
            None
        })
    }

    fn run(self) {
        while let Ok(first) = self.inbox.recv() {
            let mut batch = vec![first];
            batch.extend(self.inbox.try_iter());

            let mut newest: HashMap<usize, usize> = HashMap::new();
            for (index, request) in batch.iter().enumerate() {
                newest.insert(request.fence, index);
            }
            for (index, request) in batch.into_iter().enumerate() {
                let answer = if newest.get(&request.fence) == Some(&index) {
                    Answer::Spans(request.key, self.run_job(&request))
                } else {
                    Answer::Superseded(request.key)
                };
                if self.outbox.send(answer).is_err() {
                    return;
                }
            }
            if let Some(wake) = self.wake.as_ref() {
                // A closed event loop is a window on its way out, not an error to report.
                let _ = wake.send_event(Wake::Highlights);
            }
        }
    }
}

/// One thread that colours fences, and the two channels to it.
///
/// Spawned on the first tagged fence the editor lays out, so a session that never draws one never
/// starts a thread. Nothing is joined on shutdown: dropping the editor closes the request channel
/// and the thread ends on its own.
#[derive(Debug)]
pub(crate) struct Highlighter {
    requests: Sender<Request>,
    answers: Receiver<Answer>,
    held: Vec<Answer>,
    /// A worker that has been built but not started, for a test that must lay a frame out with no
    /// worker running. `None` once started.
    unstarted: Option<Worker>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker").finish_non_exhaustive()
    }
}

impl Highlighter {
    /// Builds the channels and the worker without starting it.
    pub(crate) fn unstarted(wake: Option<EventLoopProxy<Wake>>, job: Job) -> Self {
        let (requests, inbox) = mpsc::channel();
        let (outbox, answers) = mpsc::channel();
        Self {
            requests,
            answers,
            held: Vec::new(),
            unstarted: Some(Worker {
                job,
                inbox,
                outbox,
                wake,
            }),
        }
    }

    /// Builds the channels and starts the worker.
    pub(crate) fn spawn(wake: Option<EventLoopProxy<Wake>>, job: Job) -> Self {
        let mut highlighter = Self::unstarted(wake, job);
        highlighter.start();
        highlighter
    }

    /// Starts the worker if it has not been.
    pub(crate) fn start(&mut self) {
        let Some(worker) = self.unstarted.take() else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("mdedit-highlight".to_owned())
            .spawn(move || worker.run());
        if let Err(error) = spawned {
            // A machine that will not give us a thread still draws documents: fences stay plain.
            crate::log_error!("could not start the highlighter: {error}");
        }
    }

    /// Asks for a fence to be coloured. Silent if the worker is gone: plain is what is drawn anyway.
    pub(crate) fn request(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    /// Every answer that has arrived, without waiting. The only way anything leaves this type.
    pub(crate) fn collect(&mut self) -> Vec<Answer> {
        let mut answers = std::mem::take(&mut self.held);
        answers.extend(self.answers.try_iter());
        answers
    }

    /// Blocks up to `patience` for one answer and holds it for the next [`Highlighter::collect`].
    pub(crate) fn wait_for_one(&mut self, patience: Duration) {
        match self.answers.recv_timeout(patience) {
            Ok(answer) => self.held.push(answer),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        }
    }
}
