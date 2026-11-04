//! What highlighting a fence costs, on each thread (MDEDIT-190). A measurement, not an assertion.
//!
//! Ignored, so it never runs in CI. fancy-regex is very slow in a debug build, so run it release:
//!
//! ```text
//! cargo nextest run --release -p mdedit-app --all-features -E 'test(/^fence_cost::/)' --run-ignored only --no-capture
//! ```
//!
//! Three numbers matter. The **worker** time per job is what the user waits for colours by. The
//! **UI** cost of a frame holding a pending 300-line fence should be about the plain path's, since
//! the drawing thread only hashes and looks up. And the UI cost of a keystroke inside that fence
//! includes deriving the previous version's spans for the prefix and suffix the edit left alone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::fmt::Write as _;
use std::time::Instant;

use mdedit_app::editor::Editor;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub::Grid;

fn rust_lines(count: usize) -> String {
    let mut code = String::new();
    for index in 0..count {
        let _ = writeln!(
            code,
            "    let value_{index}: u32 = compute(\"name {index}\", {index}); // note {index}"
        );
    }
    code
}

fn fence(info: &str, lines: usize) -> String {
    format!("```{info}\nfn main() {{\n{}}}\n```\n", rust_lines(lines))
}

fn millis(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

fn frame_ms(editor: &mut Editor<Grid>) -> f64 {
    let started = Instant::now();
    let _ = editor.build_frame(900.0, 600.0, 1.0);
    millis(started)
}

#[test]
#[ignore = "a measurement"]
fn fence_cost() {
    let started = Instant::now();
    let _ = mdedit_highlight::highlight("rust", "fn main() {}\n");
    println!(
        "worker: cold syntax set load + first parse: {:.1} ms",
        millis(started)
    );
    for lines in [30, 300, 3_000] {
        let source = rust_lines(lines);
        let started = Instant::now();
        let _ = mdedit_highlight::highlight("rust", &source);
        println!(
            "worker: {lines:>5} line job ({} bytes): {:.1} ms",
            source.len(),
            millis(started)
        );
    }

    // UI: a first frame holding a pending fence, against the same fence with no info string.
    for (label, info) in [("pending, tagged", "rust"), ("plain, untagged", "")] {
        let mut times = Vec::new();
        for _ in 0..30 {
            let mut editor = Editor::new(&fence(info, 300), Grid::new());
            editor.hold_highlighting();
            times.push(frame_ms(&mut editor));
        }
        times.sort_by(f64::total_cmp);
        println!(
            "ui: first frame, 300 line fence, {label}: p50 {:.2} ms, p99 {:.2} ms",
            percentile(&times, 50),
            percentile(&times, 99)
        );
    }

    // UI: a keystroke inside a coloured 300-line fence, the worker held back so every one is
    // answered from the previous version's spans.
    let source = fence("rust", 300);
    let mut editor = Editor::new(&source, Grid::new());
    let _ = editor.build_frame(900.0, 600.0, 1.0);
    assert!(editor.await_highlights(std::time::Duration::from_secs(60)));
    let _ = editor.build_frame(900.0, 600.0, 1.0);
    editor.hold_highlighting();
    let middle = source.find("value_150").unwrap();
    let mut times = Vec::new();
    for offset in 0..200 {
        editor
            .document_mut()
            .set_selection(Selection::caret(ByteOffset::new(middle + offset % 5)));
        editor.insert_text("x");
        times.push(frame_ms(&mut editor));
    }
    times.sort_by(f64::total_cmp);
    println!(
        "ui: keystroke in a 300 line fence, reusing spans: p50 {:.2} ms, p99 {:.2} ms (n={})",
        percentile(&times, 50),
        percentile(&times, 99),
        times.len()
    );
}
