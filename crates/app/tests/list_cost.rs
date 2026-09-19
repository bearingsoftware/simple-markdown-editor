//! What decorating a list-heavy document costs. A measurement, not an assertion.
//!
//! ```text
//! cargo test --release -p mdedit-app --test app -- list_cost:: --ignored --nocapture
//! ```
//!
//! MDEDIT-141 put a branch and, for a bulleted item, a small allocation on `build_runs_in` — which is
//! on the keystroke path. **It uses nothing but the API that has always been there**, following
//! `json_typing_cost.rs`, so the same file runs against a commit from before the substitution and the
//! two numbers can be compared.
//!
//! Compare in **pairs**, alternating the two builds: this project's own measurements drift by more
//! than the difference being looked for when a run is left to sit on its own.
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

use mdedit_app::view::{Decoration, Mode, Theme, build_runs_in};
use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};

/// Nested two deep, so the deeper bullet is exercised as well as the shallow one, and ordered items
/// interleaved, because those substitute without allocating.
fn body() -> String {
    let mut text = String::new();
    for index in 0..2_000 {
        let _ = writeln!(text, "- item number {index}");
        let _ = writeln!(text, "  - a child of item {index}");
        let _ = writeln!(text, "{}. an ordered sibling", index + 1);
    }
    text
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn decorating_a_list_heavy_document() {
    let text = body();
    let document = Document::from(text.as_str());
    let theme = Theme::default();
    let decoration = Decoration {
        theme: &theme,
        mode: Mode::Preview,
    };
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(text.len()));
    println!("document: {} bytes, 6000 list items", text.len());

    for _ in 0..3 {
        let _ = build_runs_in(&document, decoration, whole, &text);
    }

    let mut best = f64::MAX;
    for _ in 0..20 {
        let started = Instant::now();
        let runs = build_runs_in(&document, decoration, whole, &text);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        assert!(!runs.is_empty());
        best = best.min(elapsed);
    }
    println!("build_runs_in over the whole document: {best:.3} ms (best of 20)");
}
