//! Where errors go, now that they cannot go to a console.
//!
//! # Why this exists at all
//!
//! Every soft error in the editor — a skipped session entry, a save that failed, a malformed config,
//! a menu that would not attach — went to `stderr` and nowhere else. That was survivable only while
//! the binary was console-subsystem, which it no longer is: a windows-subsystem binary launched from
//! Explorer has no console to print to, so without this file those messages would simply cease to
//! exist. Logging is not the polish that accompanies that change; it is its precondition.
//!
//! # Deliberately hand-rolled
//!
//! No `log`, no `tracing`. Those facades earn their keep by letting *other crates'* diagnostics flow
//! into one place, and nothing in this dependency tree emits any. What is left is a timestamped line
//! appended to a file, which is this module — and the project's standing preference is the standard
//! library over a dependency when the dependency buys nothing.
//!
//! # What may be written
//!
//! Messages and paths. **Never document bytes.** An error about a file names the file; it does not
//! quote it. A log the user might attach to a bug report must not carry the contents of whatever
//! they had open.
//!
//! # What is not here
//!
//! The Windows Event Log. It is the convention for services, needs a registered message source in
//! the registry, and is read by administrators rather than by the person whose editor just
//! misbehaved. A desktop editor serves that person with a file they can find and a dialog that
//! names it, which is what every comparable editor does.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Roll the log over at this size, keeping one previous generation.
const MAX_BYTES: u64 = 1024 * 1024;

/// The open log file, once [`start`] has found one.
static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();

struct Sink {
    file: File,
    path: PathBuf,
}

/// Where the log lives: beside the session file, because it is state rather than settings.
#[must_use]
pub fn default_log_path() -> Option<PathBuf> {
    mdedit_session::default_session_path().map(|session| {
        let mut path = session;
        path.set_file_name("mdedit.log");
        path
    })
}

/// Opens the log, rotating it first if it has grown past the cap.
///
/// Best-effort throughout: a log that cannot be opened is a reason to carry on without one, never a
/// reason to fail to start. Everything still reaches `stderr` regardless, so a terminal run behaves
/// exactly as it did before this module existed.
pub fn start() {
    let Some(path) = default_log_path() else {
        return;
    };
    if let Some(directory) = path.parent() {
        let _ = std::fs::create_dir_all(directory);
    }
    rotate_if_large(&path);

    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let opened = path.display().to_string();
    let _ = SINK.set(Mutex::new(Sink { file, path }));

    write(&format!(
        "started, version {}, log at {opened}",
        env!("CARGO_PKG_VERSION"),
    ));
}

/// The log's path, for a dialog that needs to name it.
#[must_use]
pub fn path() -> Option<PathBuf> {
    SINK.get()
        .and_then(|sink| sink.lock().ok().map(|sink| sink.path.clone()))
}

/// Records one line, and echoes it to `stderr`.
///
/// The echo is what keeps a terminal run readable and the latency harness legible; the file is what
/// keeps an Explorer launch diagnosable. Both, always — they cost a line each.
pub fn write(message: &str) {
    eprintln!("{message}");
    let line = format!("{} {message}\n", stamp());

    if let Some(sink) = SINK.get()
        && let Ok(mut sink) = sink.lock()
    {
        let _ = sink.file.write_all(line.as_bytes());
        let _ = sink.file.flush();
    }
}

/// Renames the log aside when it is too big, keeping exactly one previous generation.
///
/// One, not many: this is a diagnostic aid, and a user who needs more history than the last megabyte
/// has a different problem than a log rotation policy solves.
fn rotate_if_large(path: &Path) {
    let too_big = std::fs::metadata(path).is_ok_and(|data| data.len() > MAX_BYTES);
    if !too_big {
        return;
    }
    let mut previous = path.to_path_buf();
    previous.set_extension("log.old");
    let _ = std::fs::rename(path, previous);
}

/// Seconds since the epoch, as a fixed-width field.
///
/// Not a calendar date: formatting one needs `chrono` or `time`, and a monotonic-looking number is
/// enough to order events and to see that two runs are different. If a human-readable timestamp is
/// ever wanted, that is when the dependency earns its place.
fn stamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut out = String::new();
    let _ = write!(out, "[{seconds}]");
    out
}

/// Installs the panic hook: a panic lands in the log with a backtrace, and says where it landed.
///
/// The hook must not be able to panic itself — a panic inside a panic hook aborts without the
/// message that would explain either — so every step is best-effort and nothing here unwraps.
///
/// # Why `notify` is a parameter rather than a dialog
///
/// Telling the user needs `rfd`, and `main.rs` explains at length why `rfd` is used from the binary
/// and not from this library: it imports from the `ComCtl32` v6 side-by-side assembly, which comes
/// from the manifest `build.rs` embeds into the *executable*. Anything else that links this crate —
/// a test, an example, a future tool — has no manifest and would die at load. So the library logs,
/// and the binary is handed the job of saying so. `notify` receives the message and the log's path.
pub fn install_panic_hook(notify: impl Fn(&str, Option<&Path>) + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "unknown".to_owned(), ToString::to_string);

        // A panic payload is `&str` for a literal message and `String` for a formatted one, and
        // downcasting covers both. `payload_as_str` would say this in one line and is stable only
        // from 1.91, above this project's floor.
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("(no message)");
        let backtrace = std::backtrace::Backtrace::force_capture();

        write(&format!("PANIC at {location}: {message}"));
        write(&format!("backtrace:\n{backtrace}"));

        let where_to_look = path();
        notify(message, where_to_look.as_deref());

        previous(info);
    }));
}

/// Records a formatted line, with `eprintln!`'s shape.
///
/// A macro rather than a function so the call sites read as they did — this replaced twenty-one
/// `eprintln!`s and a macro made that a rename rather than a rewrite.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::log::write(&format!($($arg)*))
    };
}
