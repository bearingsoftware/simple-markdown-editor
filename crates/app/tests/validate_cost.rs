//! What the strict check costs, and what a keystroke costs beside it. A measurement, not an
//! assertion.
//!
//! Ignored, so it never runs in CI — run it by hand:
//!
//! ```text
//! cargo test --release -p mdedit-app --test validate_cost -- --ignored --nocapture
//! ```
//!
//! MDEDIT-47's acceptance criterion is that typing at 357 KB of JSON is **unchanged**, and the whole
//! design rests on that being true by construction: the check runs on the idle timer, so a keystroke
//! should not pay for it at all. This is the number that says whether that is what actually happens
//! rather than what was intended — the find bar's rescan proved the difference matters, at 5.8 ms a
//! keystroke before it was measured.
//!
//! It also times the check itself, because that is what the idle timer has to absorb alongside the
//! whole-document re-parse and the word count.
//!
//! No assertion on the numbers, deliberately: this runs on whatever machine and power state it is
//! given, and the research says a threshold measured on one of those is not a threshold.

// Printing *is* the output of this file.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::fmt::Write as _;
use std::time::Instant;

/// How many times the per-frame ask is timed. One call is below the clock's resolution.
const CALLS: u32 = 100_000;

use mdedit_app::editor::Editor;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 800.0;

/// Roughly the 357 KB JSON MDEDIT-36 and MDEDIT-37 were measured against.
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

fn open(dir: &TempDir, name: &str, contents: &str) -> Editor<Grid> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn the_check_costs_the_idle_timer_and_not_the_keystroke() {
    let text = body();
    println!("document: {} bytes", text.len());

    let mut checks = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        let found = mdedit_json::validate(&text);
        checks.push(started.elapsed());
        assert!(found.is_empty(), "the fixture should be valid JSON");
    }
    checks.sort_unstable();
    println!("check alone   : p50 {:?}", checks[checks.len() / 2]);

    // Typing in the checked language. Comparing this against a Markdown document of the same bytes
    // would measure the wrong thing — `AGENTS.md` already records that a JSON keystroke costs more
    // than a Markdown one for reasons that predate this ticket, and MDEDIT-37 carries finding them.
    // The comparison that answers MDEDIT-47 is this number against the same number on `main`, which
    // is why it uses nothing this branch added.
    let dir = TempDir::new().unwrap();
    let mut json = open(&dir, "big.json", &text);
    typing_cost(&mut json, "typing in JSON");

    // What a *frame* pays, which is the only part of this feature a keystroke can reach: the status
    // bar asks for the count once, and the count is a stamp comparison. Measured over many calls
    // because one is below the clock's resolution — and measured at all because the keystroke
    // numbers above have a noise floor on any ordinary machine far wider than this could ever be.
    let started = Instant::now();
    let mut total = 0;
    for _ in 0..CALLS {
        total += json.diagnostic_count();
    }
    let each = started.elapsed() / CALLS;
    println!("per frame     : {each:?} ({total} counted)");

    // And what the idle timer now absorbs, which is where the cost was deliberately put.
    let mut idles = Vec::new();
    for _ in 0..10 {
        json.insert_text("x");
        let started = Instant::now();
        json.on_idle();
        idles.push(started.elapsed());
    }
    idles.sort_unstable();
    println!("one idle tick : p50 {:?}", idles[idles.len() / 2]);
}
