//! What a keystroke in a large JSON document costs. A measurement, not an assertion.
//!
//! ```text
//! cargo test --release -p mdedit-app --test json_typing_cost -- --ignored --nocapture
//! ```
//!
//! **It deliberately uses nothing but the API that has always been there** — an `Editor`, a
//! keystroke and a frame — so the same file can be run against an older commit and the two numbers
//! compared. That is the whole reason it is separate from `validate_cost.rs`, which uses MDEDIT-47's
//! API and therefore cannot be run on a build that predates it.
//!
//! It was written for MDEDIT-47, whose acceptance criterion is that typing at this size is
//! *unchanged*, and it stays because MDEDIT-37 is still open: the ~8 ms a JSON keystroke costs over
//! a Markdown one has not been attributed, and nothing else in the repo measures it.
//!
//! No assertion on the numbers. This runs on whatever machine and power state it is given.

// Printing *is* the output of this file.
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
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 800.0;

/// Roughly the size MDEDIT-36 and MDEDIT-37 were measured against.
fn body() -> String {
    let mut text = String::from("{\n  \"records\": [\n");
    for index in 0..4_000 {
        let _ = writeln!(
            text,
            "    {{\"id\": {index}, \"name\": \"record number {index}\", \"tags\": [\"alpha\", \
             \"beta\"], \"ok\": true, \"score\": {index}.5}},"
        );
    }
    text.push_str("    null\n  ]\n}\n");
    text
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn a_keystroke_in_a_large_json_document() {
    let text = body();
    println!("document: {} bytes", text.len());

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("big.json");
    std::fs::write(&path, &text).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

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
        "keystroke: p50 {:?}  p90 {:?}  p99 {:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 90 / 100],
        samples[samples.len() * 99 / 100],
    );
}
