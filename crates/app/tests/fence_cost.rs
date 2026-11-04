//! What highlighting a fence costs (MDEDIT-190). A measurement, not an assertion.
//!
//! Ignored, so it never runs in CI. fancy-regex is very slow in a debug build, so run it release:
//!
//! ```text
//! cargo nextest run --release -p mdedit-app --all-features -E 'test(/^fence_cost::/)' --run-ignored only --no-capture
//! ```
//!
//! It times the cold syntax-set load, highlighting a Rust fence of 30, 300 and 3,000 lines, and a
//! one-character edit inside a 300-line fence through the same run builder the layout-cache miss
//! calls. No assertion on the numbers: they are for the decision record.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::fmt::Write as _;
use std::time::Instant;

use mdedit_app::view::{Decoration, Mode, Theme, build_runs_in};
use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};

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

fn fence(lines: usize) -> String {
    format!("```rust\nfn main() {{\n{}}}\n```\n", rust_lines(lines))
}

fn millis(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

fn build(source: &str) -> usize {
    let doc = Document::from(source);
    let theme = Theme::default();
    build_runs_in(
        &doc,
        Decoration {
            theme: &theme,
            mode: Mode::Preview,
        },
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
        source,
    )
    .len()
}

#[test]
#[ignore = "a measurement"]
fn fence_cost() {
    let started = Instant::now();
    let _ = mdedit_highlight::highlight("rust", "fn main() {}\n");
    println!(
        "cold syntax set load + first rust parse: {:.1} ms",
        millis(started)
    );

    for lines in [30, 300, 3_000] {
        let source = fence(lines);
        let _ = build(&source);
        let started = Instant::now();
        let runs = build(&source);
        println!(
            "{lines:>5} line rust fence ({} bytes): {:.2} ms, {runs} runs",
            source.len(),
            millis(started)
        );
    }

    let source = fence(300);
    let mut times = Vec::new();
    for run in 0..200 {
        let mut edited = source.clone();
        let at = edited.find("compute").unwrap() + (run % 5);
        edited.insert(at, 'x');
        let started = Instant::now();
        let _ = build(&edited);
        times.push(millis(started));
    }
    times.sort_by(f64::total_cmp);
    println!(
        "one-character edit in a 300 line fence: p50 {:.2} ms, p99 {:.2} ms (n={})",
        percentile(&times, 50),
        percentile(&times, 99),
        times.len()
    );
}
