//! What lexing a single-line YAML document costs. A measurement, not an assertion.
//!
//! Ignored, so it never runs in CI — run it by hand:
//!
//! ```text
//! cargo test --release -p mdedit-yaml --test lex_cost -- --ignored --nocapture
//! ```
//!
//! It exists because MDEDIT-99 was quadratic in a way no multi-line fixture could show. `token_end`
//! scanned for a trailing `` #`` comment to the end of the *line* for every plain scalar on it, and
//! a minified flow document — JSON renamed `.yaml`, which is valid YAML and a realistic thing to
//! open — is one line with tens of thousands of scalars on it. Every fixture in `lex.rs` is a few
//! lines long, so all of them were fast.
//!
//! The doubling ladder is the point rather than any single number: what says "quadratic" is the
//! ratio between the rows, and it holds on whatever machine and power state this is given. Linear
//! work doubles when the input doubles; this used to quadruple. Re-run it after touching
//! `token_end`, `flow` or `analyze`, and put the ladder in the PR body.
//!
//! No assertion on the numbers, deliberately, for the reason `find_cost.rs` gives: a threshold
//! measured on one machine is not a threshold.

// Printing *is* the output of this file — there is nothing to assert, and a number nobody can read
// is not a measurement.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::fmt::Write as _;
use std::time::Instant;

use mdedit_language::Language as _;
use mdedit_yaml::Yaml;

/// Minified flow content on **one line**, to about `bytes`.
///
/// This is what a `.json` file renamed `.yaml` looks like, and it is the shape that hurt: a great
/// many scalars with no newline anywhere to bound the scan.
fn one_line(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 64);
    text.push('{');
    let mut index = 0_u32;
    while text.len() < bytes {
        if index > 0 {
            text.push(',');
        }
        write!(text, "\"key{index}\":\"value{index}\"").expect("a String never fails to write");
        index += 1;
    }
    text.push('}');
    text
}

/// Plain scalars rather than quoted ones, which is the arm of `token_end` that was quadratic.
fn one_line_plain(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 64);
    text.push('[');
    let mut index = 0_u32;
    while text.len() < bytes {
        if index > 0 {
            text.push(',');
        }
        write!(text, "item{index} of some length").expect("a String never fails to write");
        index += 1;
    }
    text.push(']');
    text
}

fn cost(label: &str, text: &str) {
    let mut samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let analysis = Yaml.analyze(text);
        samples.push(started.elapsed());
        assert!(!analysis.segments.is_empty());
    }
    samples.sort_unstable();
    println!(
        "{label}  {:>9} bytes   p50 {:?}   min {:?}",
        text.len(),
        samples[samples.len() / 2],
        samples[0]
    );
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn lexing_one_long_line() {
    // A ladder rather than one size: linear work doubles with the input, and quadratic work
    // quadruples. That ratio is the finding, and it survives being run on a different machine.
    for bytes in [32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576] {
        cost("quoted flow ", &one_line(bytes));
    }
    for bytes in [32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576] {
        cost("plain flow  ", &one_line_plain(bytes));
    }
}

/// The same bytes with newlines in them, which is the shape that was always fast — kept beside the
/// slow one so the comparison is in the same run rather than in somebody's memory.
#[test]
#[ignore = "a measurement, not an assertion"]
fn lexing_the_same_bytes_over_many_lines() {
    for bytes in [262_144, 1_048_576] {
        let text = one_line_plain(bytes).replace(',', ",\n");
        cost("many lines  ", &text);
    }
}
