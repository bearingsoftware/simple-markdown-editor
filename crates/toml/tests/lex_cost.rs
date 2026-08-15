//! What lexing TOML costs. A measurement, not an assertion.
//!
//! Ignored, so it never runs in CI — run it by hand:
//!
//! ```text
//! cargo test --release -p mdedit-toml --test lex_cost -- --ignored --nocapture
//! ```
//!
//! The ladder is the finding rather than any single row: linear work doubles when the input doubles,
//! and quadratic work quadruples. That is how MDEDIT-99 was found in the YAML lexer *after* it
//! shipped, because every unit fixture was a few lines long. Segment density is reported beside it
//! because MDEDIT-36 found density rather than parse time was what cost downstream.
//!
//! No assertion on the numbers, for the reason `mdedit-yaml`'s equivalent gives: a threshold
//! measured on one machine is not a threshold.

// Printing *is* the output of this file.
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
use mdedit_toml::Toml;

/// What a `Cargo.toml` or a configuration file looks like: many small tables.
fn tables(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 128);
    let mut index = 0_u32;
    while text.len() < bytes {
        write!(
            text,
            "[package.entry{index}]\n\
             name = \"item{index}\"\n\
             version = \"1.{index}.0\"\n\
             enabled = true\n\
             ratio = 3.14159\n\
             updated = 1979-05-27T07:32:00Z\n\
             tags = [\"one\", \"two\", \"three\"]\n\
             meta = {{ owner = \"ann\", scope = \"user\" }}\n\n"
        )
        .expect("a String never fails to write");
        index += 1;
    }
    text
}

/// Long multi-line strings, which is the one construct that carries state across a line.
fn multiline(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 128);
    let mut index = 0_u32;
    while text.len() < bytes {
        write!(
            text,
            "doc{index} = \"\"\"\n\
             A paragraph of documentation with a # that is not a comment\n\
             and a [bracket] that is not a table, over several lines.\n\
             \"\"\"\n\n"
        )
        .expect("a String never fails to write");
        index += 1;
    }
    text
}

fn cost(label: &str, text: &str) {
    let mut samples = Vec::new();
    let mut segments = 0;
    for _ in 0..5 {
        let started = Instant::now();
        let analysis = Toml.analyze(text);
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
        cost("tables      ", &tables(bytes));
    }
    for bytes in [32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576] {
        cost("multiline   ", &multiline(bytes));
    }
}

/// The keystroke budget is stated for 60 KiB, so the comparable number belongs there.
#[test]
#[ignore = "a measurement, not an assertion"]
fn lexing_sixty_kib_against_the_keystroke_budget() {
    cost("tables   60K", &tables(60 * 1024));
    cost("multi    60K", &multiline(60 * 1024));
}
