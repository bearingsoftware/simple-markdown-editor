//! What one parser option costs, measured so the answer survives a machine that drifts (MDEDIT-131).
//!
//! # Why this is not a criterion bench
//!
//! Criterion measures one configuration at a time. Comparing a flag with it means comparing two
//! *measurements* — either two runs of two builds, or two benches seconds apart in one run — and on
//! a laptop that reclocks, the gap between the measurements is where the answer goes to die.
//!
//! MDEDIT-126 could not get a number this way. `segment/parse-only` builds a `Parser` with no
//! options and so cannot observe any flag — an accidental control group — and across six interleaved
//! runs pinned to two CPUs its floor drifted −40% to +21%. Within-run intervals were tight at ±5%,
//! which is what made the bad runs look credible. Putting both variants in one process helped and
//! was still not enough: measured that way the same comparison read +3% in one run and +76% in the
//! next, because the two benches are still seconds apart.
//!
//! # Pairs, and the ratio between them
//!
//! So the two configurations are timed **microseconds apart, alternating**, and what is reported is
//! the *ratio* of one to the other. Whatever the clock is doing, it is doing the same thing to both
//! halves of a pair — so drift cancels instead of accumulating. The order within a pair alternates
//! too, so being second is not systematically worse.
//!
//! # The control is the method, not a third configuration
//!
//! `control` pairs a configuration **against itself**. Identical work, so the honest answer is
//! exactly 1.00 and anything else is the harness measuring its own noise rather than the flag. Read
//! it first: if the control is not tight around 1, no other line here means anything. That is the
//! check MDEDIT-126 was missing until it was too late.
//!
//! # Two corpora, and the second is the interesting one
//!
//! `prose` has no table in it, so it measures the flag's **standing overhead** — what pulldown-cmark
//! spends checking whether a paragraph's second line is a delimiter row on documents that never have
//! one. That is the number MDEDIT-126 was trying to produce.
//!
//! `tables` has them, so it measures what parsing a table actually costs. That is the number anybody
//! would want, and no bench in this repo could produce it before.
//!
//! ```text
//! cargo bench -p mdedit-markers --features bench --bench options
//! ```

// This bench supplies its own `main`; criterion is deliberately not involved.
#![allow(missing_docs, clippy::print_stdout)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use mdedit_markers::bench;
use pulldown_cmark::Options;

/// How many pairs to time. Each pair is two analyses, so the whole run is this many of each.
const PAIRS: usize = 250;

/// Pairs discarded before measuring, so the first is not paying for a cold cache.
const WARMUP: usize = 20;

fn main() {
    let without = bench::options() - Options::ENABLE_TABLES;
    let with = bench::options();

    println!("MDEDIT-131 — what ENABLE_TABLES costs, measured in pairs\n");
    println!("{PAIRS} pairs, alternating order within each pair; the ratio is what drift cancels");
    println!("out of. Read `control` first: it pairs a configuration against itself, so anything");
    println!("but 1.00 is this harness measuring its own noise.\n");

    for (name, source) in [("prose", prose(60 * 1024)), ("tables", tables(60 * 1024))] {
        println!("== {name}, {} KiB ==", source.len() / 1024);
        report("control (off vs off)", &source, without, without);
        report("ENABLE_TABLES", &source, without, with);
        println!();
    }
}

/// Times `a` against `b` in pairs and prints the ratio of b to a.
fn report(what: &str, source: &str, a: Options, b: Options) {
    let mut ratios = Vec::with_capacity(PAIRS);
    let mut first = Vec::with_capacity(PAIRS);
    let mut second = Vec::with_capacity(PAIRS);

    for pair in 0..(PAIRS + WARMUP) {
        // **The order alternates**, so whichever runs second is not systematically the slower.
        let (one, two) = if pair % 2 == 0 {
            (time(source, a), time(source, b))
        } else {
            let two = time(source, b);
            (time(source, a), two)
        };
        if pair < WARMUP {
            continue;
        }
        first.push(one);
        second.push(two);
        ratios.push(two.as_secs_f64() / one.as_secs_f64());
    }

    ratios.sort_by(f64::total_cmp);
    first.sort_unstable();
    second.sort_unstable();

    let median = ratios[ratios.len() / 2];
    let low = ratios[ratios.len() / 20];
    let high = ratios[ratios.len() - 1 - ratios.len() / 20];
    println!(
        "  {what:<22} {:>8.3?} -> {:>8.3?}   x{median:.3}  [{low:.3} .. {high:.3}]  {:+.1}%",
        first[first.len() / 2],
        second[second.len() / 2],
        (median - 1.0) * 100.0
    );
}

/// One analysis, timed.
fn time(source: &str, options: Options) -> Duration {
    let at = Instant::now();
    let analysis = bench::analyze_with(black_box(source), options);
    let elapsed = at.elapsed();
    drop(black_box(analysis));
    elapsed
}

/// Prose with the delimiter density of real notes, and not a table in it.
fn prose(target_bytes: usize) -> String {
    repeated(
        "\
## Section heading

Some prose with **strong**, *emphasis*, `code` and a [link](http://example.com \"title\").

- a list item
- another with `code`

> a quoted line
> and its continuation

",
        target_bytes,
    )
}

/// The same shape of document with tables through it.
///
/// Eight columns and four rows: wide enough that the per-cell work shows, short enough that the
/// surrounding prose is still most of the bytes — which is what a document containing tables looks
/// like, rather than a file that is nothing else.
fn tables(target_bytes: usize) -> String {
    repeated(
        "\
## Section heading

Some prose before the table.

| Key | Type | Default | Since | Notes | Owner | Status | Tag |
| --- | --- | --- | --- | --- | --- | --- | --- |
| alpha | string | none | 1.0 | the first | ann | live | a |
| beta | number | 0 | 1.1 | the second | bob | live | b |
| gamma | bool | false | 1.2 | the third | cal | draft | c |
| delta | string | `x` | 1.3 | the fourth | dee | live | d |

Some prose after it.

",
        target_bytes,
    )
}

fn repeated(unit: &str, target_bytes: usize) -> String {
    let mut out = String::with_capacity(target_bytes + unit.len());
    while out.len() < target_bytes {
        out.push_str(unit);
    }
    out
}
