//! What a keystroke costs with word wrap on and off. A measurement, not an assertion.
//!
//! ```text
//! cargo test --release -p mdedit-app --test app -- wrap_cost:: --ignored --nocapture
//! ```
//!
//! MDEDIT-49's acceptance criterion is that the keystroke budget holds in **both** modes, and the
//! two are not obviously alike: wrap-off lays every chunk out at an unbounded width, which is one
//! row per source line instead of several, so there are fewer rows to place and more glyphs on each.
//! Which of those wins is a question for a measurement rather than for reasoning.
//!
//! No assertion on the numbers. This runs on whatever machine and power state it is given, and the
//! research says a threshold measured on one of those is not a threshold.

// Printing *is* the output of this file.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::time::Instant;

use mdedit_app::editor::Editor;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 800.0;

/// Prose with long paragraphs, so wrapping actually has work to do.
fn body() -> String {
    let mut text = String::new();
    for index in 0..2_000 {
        text.push_str(
            "A paragraph of ordinary prose that runs on for long enough to wrap several times in \
             an eight-hundred pixel viewport, which is the case the two modes disagree about.\n\n",
        );
        if index % 20 == 0 {
            text.push_str("## A heading\n\n");
        }
    }
    text
}

fn typing_cost(editor: &mut Editor<Grid>, label: &str) {
    for _ in 0..3 {
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    }
    let mut samples = Vec::new();
    for _ in 0..60 {
        editor
            .document_mut()
            .set_selection(Selection::caret(ByteOffset::new(40)));
        let started = Instant::now();
        editor.insert_text("x");
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    println!(
        "{label}: p50 {:?}  p90 {:?}  p99 {:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 90 / 100],
        samples[samples.len() * 99 / 100],
    );
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn a_keystroke_in_both_modes() {
    let text = body();
    println!("document: {} bytes", text.len());

    let mut wrapped = Editor::new(&text, Grid::new());
    typing_cost(&mut wrapped, "wrap on ");

    let mut unwrapped = Editor::new(&text, Grid::new());
    unwrapped.set_wrap(false);
    typing_cost(&mut unwrapped, "wrap off");
}
