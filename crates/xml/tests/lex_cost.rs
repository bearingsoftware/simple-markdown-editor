//! What lexing XML costs. A measurement, not an assertion.
//!
//! Ignored, so it never runs in CI — run it by hand:
//!
//! ```text
//! cargo test --release -p mdedit-xml --test lex_cost -- --ignored --nocapture
//! ```
//!
//! # Why a ladder rather than one number
//!
//! The ratio between the rows is the finding, and it survives being run on a machine that reclocks —
//! which this one does, by up to 3× between runs (MDEDIT-131). Linear work doubles when the input
//! doubles. Quadratic work quadruples, which is how MDEDIT-99 was found in the YAML lexer *after* it
//! shipped, because every fixture in its unit tests was a few lines long.
//!
//! The two shapes here are the two ways XML goes wrong. `nested` is deep, which is what the
//! depth counter exists for. `attributes` is wide and quote-heavy, which is the arm that scans.
//!
//! # Segment density is the number that matters downstream
//!
//! MDEDIT-36 found it was density rather than parse time that cost: JSON produced 140,009 segments
//! against Markdown's 19,410 for the same document, and everything downstream pays per segment. So
//! this reports segments per KiB beside the time, because that is the figure a renderer inherits.
//!
//! No assertion on the numbers, deliberately, for the reason `lex_cost.rs` in `mdedit-yaml` gives: a
//! threshold measured on one machine is not a threshold.

// Printing *is* the output of this file — there is nothing to assert, and a number nobody can read
// is not a measurement.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::cast_precision_loss
)]

use std::fmt::Write as _;
use std::time::Instant;

use mdedit_language::Language as _;
use mdedit_xml::Xml;

/// Deeply nested elements — the shape the depth counter exists for.
fn nested(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 64);
    let mut depth = 0_usize;
    // Half, because every opening tag is paid for again on the way out.
    while text.len() < bytes / 2 {
        write!(text, "<level{depth}>").expect("a String never fails to write");
        depth += 1;
    }
    for level in (0..depth).rev() {
        write!(text, "</level{level}>").expect("a String never fails to write");
    }
    text
}

/// Wide, attribute-heavy elements — the shape a configuration file actually is.
fn attributes(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 128);
    text.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<settings>\n");
    let mut index = 0_u32;
    while text.len() < bytes {
        writeln!(
            text,
            "  <entry key=\"item{index}\" type=\"string\" scope=\"user\">value &amp; {index}</entry>"
        )
        .expect("a String never fails to write");
        index += 1;
    }
    text.push_str("</settings>\n");
    text
}

fn cost(label: &str, text: &str) {
    let mut samples = Vec::new();
    let mut segments = 0;
    for _ in 0..5 {
        let started = Instant::now();
        let analysis = Xml.analyze(text);
        samples.push(started.elapsed());
        segments = analysis.segments.len();
    }
    samples.sort_unstable();
    let kib = text.len() as f64 / 1024.0;
    println!(
        "{label}  {:>9} bytes   p50 {:>10?}   min {:>10?}   {segments:>8} segments   {:>6.0}/KiB",
        text.len(),
        samples[samples.len() / 2],
        samples[0],
        segments as f64 / kib
    );
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn lexing_scales_with_the_input() {
    for bytes in [32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576] {
        cost("nested      ", &nested(bytes));
    }
    for bytes in [32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576] {
        cost("attributes  ", &attributes(bytes));
    }
}

/// The keystroke budget is stated for 60 KiB, which is the size `docs/research/04` measures
/// Markdown at — so the comparable XML number belongs beside it rather than at a megabyte.
#[test]
#[ignore = "a measurement, not an assertion"]
fn lexing_sixty_kib_against_the_keystroke_budget() {
    cost("nested   60K", &nested(60 * 1024));
    cost("attrs    60K", &attributes(60 * 1024));
}
