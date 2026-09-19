//! What a keystroke costs with the find bar open. A measurement, not an assertion.
//!
//! Ignored, so it never runs in CI — run it by hand:
//!
//! ```text
//! cargo test --release -p mdedit-app --test app -- find_cost:: --ignored --nocapture
//! ```
//!
//! It exists because the find bar's rescan is the first thing this editor has put **on the document
//! keystroke path** that is proportional to the whole buffer, and MDEDIT-44's acceptance criterion
//! is that typing does not get slower with the bar open. It earned its keep immediately: the first
//! implementation measured **5.8 ms p50** here against a 3 ms budget, which is why `find::matches`
//! has an ASCII path. Re-run it after touching that function, and put the number in the PR body —
//! nothing here fails a build.
//!
//! No assertion on the numbers, deliberately. This runs on whatever machine and power state it is
//! given, and the research says a threshold measured on one of those is not a threshold.

// Printing *is* the output of this file — there is nothing to assert, and a number nobody can read
// is not a measurement.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::time::Instant;

use mdedit_app::editor::Editor;
use mdedit_app::field::Press;
use mdedit_app::find::Key;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 800.0;

fn body() -> String {
    let mut text = String::new();
    for index in 0..3_000 {
        text.push_str(
            "A paragraph of ordinary prose about the alpha channel and other matters.\n\n",
        );
        if index % 20 == 0 {
            text.push_str("## A heading\n\n");
        }
    }
    text
}

fn typing_cost(editor: &mut Editor<Grid>, label: &str) {
    // Warm every cache first, so what is measured is a keystroke rather than a first paint.
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
        "{label}: p50 {:?}  p99 {:?}  max {:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 99 / 100],
        samples.last().unwrap()
    );
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn keystroke_cost_with_and_without_the_bar() {
    let text = body();
    println!("document: {} bytes", text.len());

    let mut scans = Vec::new();
    for _ in 0..60 {
        let started = Instant::now();
        let found = mdedit_app::find::matches(&text, "alpha", false);
        scans.push(started.elapsed());
        assert_eq!(found.len(), 3_000);
    }
    scans.sort_unstable();
    println!("scan alone : p50 {:?}", scans[scans.len() / 2]);

    let mut closed = Editor::new(&text, Grid::new());
    typing_cost(&mut closed, "bar closed  ");

    let mut open = Editor::new(&text, Grid::new());
    open.apply_chord("f", false);
    open.apply_find_key(&Key::Edit(Press::Text("alpha".to_owned())), false);
    let _ = open.build_frame(WIDTH, HEIGHT, 1.0);
    println!("matches: {}", open.find_bar().unwrap().matches().len());
    typing_cost(&mut open, "bar open    ");
}
