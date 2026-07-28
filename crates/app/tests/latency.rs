//! The percentile arithmetic behind the latency numbers.
//!
//! Worth testing precisely because the numbers it produces are the evidence in a PR body. A
//! percentile that is quietly wrong by one rank would let a regression through while looking fine.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use mdedit_app::latency::Latency;

fn from_millis(values: &[u64]) -> Latency {
    let mut latency = Latency::new();
    for value in values {
        latency.record(Duration::from_millis(*value));
    }
    latency
}

#[test]
fn an_empty_collection_reports_nothing_rather_than_zero() {
    let latency = Latency::new();
    assert!(latency.is_empty());
    assert_eq!(latency.p50(), None);
    assert_eq!(latency.p99(), None);
    assert_eq!(latency.max(), None);
    assert_eq!(latency.report(), "no keystrokes recorded");
}

#[test]
fn percentiles_use_nearest_rank_without_interpolating() {
    // 1..=10 ms. Nearest-rank p50 is the 5th value, not the mean of the 5th and 6th: interpolating
    // invents a number that was never measured, which for a tail statistic is the wrong answer.
    let latency = from_millis(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert_eq!(latency.p50(), Some(Duration::from_millis(5)));
    assert_eq!(latency.percentile(90.0), Some(Duration::from_millis(9)));
    assert_eq!(latency.p99(), Some(Duration::from_millis(10)));
    assert_eq!(latency.max(), Some(Duration::from_millis(10)));
}

#[test]
fn order_of_arrival_does_not_change_the_answer() {
    let ascending = from_millis(&[1, 2, 3, 4, 5]);
    let shuffled = from_millis(&[4, 1, 5, 3, 2]);
    assert_eq!(ascending.p50(), shuffled.p50());
    assert_eq!(ascending.p99(), shuffled.p99());
}

#[test]
fn a_single_sample_is_every_percentile() {
    let latency = from_millis(&[7]);
    assert_eq!(latency.percentile(0.0), Some(Duration::from_millis(7)));
    assert_eq!(latency.p50(), Some(Duration::from_millis(7)));
    assert_eq!(latency.p99(), Some(Duration::from_millis(7)));
}

/// The tail is the point. A p99 that got swallowed by the bulk of fast samples would report
/// success while one keystroke in a hundred stuttered.
#[test]
fn a_rare_slow_sample_shows_up_in_p99_and_not_in_p50() {
    let mut values = vec![1u64; 99];
    values.push(500);
    let latency = from_millis(&values);

    assert_eq!(latency.p50(), Some(Duration::from_millis(1)));
    assert_eq!(
        latency.p99(),
        Some(Duration::from_millis(1)),
        "the 99th of 100 samples is still fast"
    );
    assert_eq!(latency.max(), Some(Duration::from_millis(500)));

    // One more slow sample and the tail crosses into p99.
    let mut values = vec![1u64; 98];
    values.extend([500, 500]);
    assert_eq!(from_millis(&values).p99(), Some(Duration::from_millis(500)));
}

#[test]
fn the_report_names_the_budget_and_whether_it_was_met() {
    let fast = from_millis(&[1, 1, 1, 2]);
    let report = fast.report();
    assert!(report.contains("samples: 4"), "{report}");
    assert!(report.contains("p50"), "{report}");
    assert!(report.contains("p99"), "{report}");
    assert!(
        report.contains("p50 <= 3.00 ms PASS"),
        "a 1-2 ms run should pass the p50 budget: {report}"
    );

    let slow = from_millis(&[50; 100]);
    assert!(
        slow.report().contains("p99 <= 8.30 ms FAIL"),
        "a 50 ms run must report FAIL rather than a number the reader has to check"
    );
}
