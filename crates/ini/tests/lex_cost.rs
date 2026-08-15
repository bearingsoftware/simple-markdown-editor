//! What lexing INI costs. A measurement, not an assertion.
//!
//! Ignored, so it never runs in CI — run it by hand:
//!
//! ```text
//! cargo test --release -p mdedit-ini --test lex_cost -- --ignored --nocapture
//! ```
//!
//! The ladder is the finding rather than any single row: linear work doubles when the input doubles
//! and quadratic work quadruples, which is how MDEDIT-99 was found in the YAML lexer after it
//! shipped. Segment density is beside it because MDEDIT-36 found density rather than parse time was
//! the cost downstream — and INI's is the lowest of any language here, since a value is one run
//! however long it is.
//!
//! No assertion on the numbers: a threshold measured on one machine is not a threshold.

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

use mdedit_ini::Ini;
use mdedit_language::Language as _;

/// What a real `.ini` looks like: small sections of short entries.
fn sections(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 128);
    let mut index = 0_u32;
    while text.len() < bytes {
        write!(
            text,
            "; settings for entry {index}\n\
             [entry{index}]\n\
             name = item{index}\n\
             path = C:\\program files\\thing{index};C:\\other\n\
             port = {index}\n\
             enabled = yes\n\
             label = \"a quoted label\"\n\n"
        )
        .expect("a String never fails to write");
        index += 1;
    }
    text
}

/// Long continuation-line values, which is the shape a `setup.cfg` takes.
fn continuations(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 128);
    let mut index = 0_u32;
    while text.len() < bytes {
        write!(text, "[group{index}]\nrequires =\n").expect("a String never fails to write");
        for item in 0..8 {
            writeln!(text, "    package-{index}-{item} >= 1.0").expect("a String never fails");
        }
        text.push('\n');
        index += 1;
    }
    text
}

fn cost(label: &str, text: &str) {
    let mut samples = Vec::new();
    let mut segments = 0;
    for _ in 0..5 {
        let started = Instant::now();
        let analysis = Ini.analyze(text);
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
        cost("sections    ", &sections(bytes));
    }
    for bytes in [32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576] {
        cost("continuation", &continuations(bytes));
    }
}

/// The keystroke budget is stated for 60 KiB, so the comparable number belongs there.
#[test]
#[ignore = "a measurement, not an assertion"]
fn lexing_sixty_kib_against_the_keystroke_budget() {
    cost("sections 60K", &sections(60 * 1024));
    cost("continu. 60K", &continuations(60 * 1024));
}
