//! MDEDIT-67: which clipboard operation, at what concurrency, corrupts the heap.
//!
//! Run it as `cargo run -- <mode> <threads> <rounds>`, or with no arguments to run the whole matrix
//! as separate child processes and report how many of each aborted — which is the only way to count
//! them, because a heap corruption takes the process with it rather than returning an error.
//!
//! Modes:
//!
//! - `new`   — construct an `arboard::Clipboard` and drop it. Opens nothing.
//! - `get`   — construct one and read the text.
//! - `set`   — construct one and write text.
//! - `mixed` — thread 0 reads; every other thread writes. If this aborts, a reader is being
//!   invalidated by *another* thread's clipboard open/close rather than by a second reader.
//!
//! Re-run this whenever `arboard` is bumped. The editor's answer to it — the real clipboard is
//! `!Send`, and an editor is built with one that holds nothing until `main` says otherwise — is a
//! constraint imposed by this measurement, so a release that fixed it upstream would be worth knowing
//! about.

use std::process::Command;
use std::thread;

const MODES: [(&str, usize, usize); 5] = [
    ("new", 9, 200),
    ("get", 1, 2000),
    ("get", 2, 200),
    ("get", 9, 200),
    ("mixed", 9, 200),
];

const RUNS: usize = 8;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(mode) = args.next() else {
        matrix();
        return;
    };
    let threads = args.next().and_then(|a| a.parse().ok()).unwrap_or(9);
    let rounds = args.next().and_then(|a| a.parse().ok()).unwrap_or(200);
    hammer(&mode, threads, rounds);
    println!("survived {mode}: {threads} threads x {rounds} rounds");
}

/// Runs each case `RUNS` times as a child process and reports how many died.
fn matrix() {
    let me = std::env::current_exe().expect("the probe should know where it is");
    println!("{RUNS} runs of each case; a death is a heap corruption taking the process with it\n");
    for (mode, threads, rounds) in MODES {
        let died = (0..RUNS)
            .filter(|_| {
                !Command::new(&me)
                    .args([mode, &threads.to_string(), &rounds.to_string()])
                    .status()
                    .expect("the probe should be able to run itself")
                    .success()
            })
            .count();
        println!("{mode:<6} threads={threads:<2} rounds={rounds:<5} : {died}/{RUNS} aborted");
    }
}

fn hammer(mode: &str, threads: usize, rounds: usize) {
    let handles: Vec<_> = (0..threads)
        .map(|index| {
            let mode = mode.to_owned();
            thread::spawn(move || {
                for _ in 0..rounds {
                    let Ok(mut clipboard) = arboard::Clipboard::new() else {
                        continue;
                    };
                    match mode.as_str() {
                        "new" => {}
                        "set" => drop(clipboard.set_text("probe")),
                        "mixed" if index > 0 => drop(clipboard.set_text("probe")),
                        _ => drop(clipboard.get_text()),
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("no thread here panics");
    }
}
