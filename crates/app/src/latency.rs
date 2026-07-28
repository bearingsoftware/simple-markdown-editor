//! The keystroke latency harness.
//!
//! `AGENTS.md` commits to keystroke→pixel p50 ≤ 3 ms and **p99 ≤ 8.3 ms**, and
//! `docs/research/06-performance-targets.md` is explicit that a mean on AC power — which is what
//! every published editor benchmark reports — is the least informative measurement available.
//! Emacs goes 5.3 ms to 50.5 ms on power-save. So this reports percentiles, and the numbers that
//! go in a PR body are taken on battery as well as plugged in.
//!
//! # What this actually measures, and what it does not
//!
//! The sample starts when winit hands us a `KeyboardInput` event and ends when `present()` returns.
//! That is everything this process controls: edit, re-segment, lay out, rasterize, pack, upload,
//! encode, submit, present.
//!
//! It excludes two real intervals on either side:
//!
//! - **Before**: USB polling, the driver, and the Windows input queue before winit sees anything.
//! - **After**: the compositor, and the panel's own response time. Under
//!   [`wgpu::PresentMode::Fifo`] `present()` may also return before the image is actually scanned
//!   out.
//!
//! So this is a *lower bound* on true keystroke-to-photon, and calling it "keystroke to pixel"
//! without that caveat would overstate it. Measuring the real thing needs a high-speed camera or a
//! hardware latency probe. What this number is good for is regression detection and comparing our
//! own frames against each other, which is what the budget is actually enforced with.

use std::fmt::Write as _;
use std::time::Duration;

/// Collected keystroke-to-present samples.
#[derive(Debug, Clone, Default)]
pub struct Latency {
    samples: Vec<Duration>,
}

impl Latency {
    /// An empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    /// Records one sample.
    pub fn record(&mut self, elapsed: Duration) {
        self.samples.push(elapsed);
    }

    /// How many samples have been taken.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The `p`th percentile, with `p` in 0..=100.
    ///
    /// Nearest-rank: the smallest sample at or above the rank, with no interpolation. Interpolating
    /// invents a number that was never measured, which for a tail statistic is exactly the wrong
    /// thing to do.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "p is clamped to 0..=100 and sample counts are in the thousands, so every value \n                  here is far inside what f64 and u128 represent exactly"
    )]
    pub fn percentile(&self, p: f64) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();

        // Integer arithmetic on purpose. `ceil(p/100 * n)` in floating point is exactly the sort of
        // rounding that puts a percentile one rank out, and a tail statistic that is quietly off by
        // one is worse than no statistic. Scaling by 100 first keeps it exact for whole percents.
        let count = sorted.len() as u128;
        let scaled = (p.clamp(0.0, 100.0) * 100.0).round().max(0.0) as u128;
        let rank = usize::try_from(scaled.saturating_mul(count).div_ceil(10_000))
            .unwrap_or(sorted.len())
            .clamp(1, sorted.len());

        sorted.get(rank - 1).copied()
    }

    /// The median.
    #[must_use]
    pub fn p50(&self) -> Option<Duration> {
        self.percentile(50.0)
    }

    /// The 99th percentile — the number the budget is actually about.
    #[must_use]
    pub fn p99(&self) -> Option<Duration> {
        self.percentile(99.0)
    }

    /// The worst sample.
    #[must_use]
    pub fn max(&self) -> Option<Duration> {
        self.samples.iter().copied().max()
    }

    /// A percentile table, ready to paste into a PR body.
    #[must_use]
    pub fn report(&self) -> String {
        if self.samples.is_empty() {
            return "no keystrokes recorded".to_owned();
        }

        let ms = |d: Option<Duration>| d.map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let mut out = String::new();
        // Writing into the buffer rather than building a `String` per line. Nothing here is hot,
        // but `format!` into `push_str` allocates twice to say the same thing.
        let _ = writeln!(out, "samples: {}", self.samples.len());
        for (label, value) in [
            ("p50", self.percentile(50.0)),
            ("p90", self.percentile(90.0)),
            ("p99", self.percentile(99.0)),
            ("max", self.max()),
        ] {
            let _ = writeln!(out, "{label}: {:.2} ms", ms(value));
        }
        let _ = writeln!(
            out,
            "budget: p50 <= 3.00 ms {}, p99 <= 8.30 ms {}",
            verdict(self.percentile(50.0), 3.0),
            verdict(self.percentile(99.0), 8.3),
        );
        out
    }
}

fn verdict(value: Option<Duration>, budget_ms: f64) -> &'static str {
    match value {
        Some(d) if d.as_secs_f64() * 1000.0 <= budget_ms => "PASS",
        Some(_) => "FAIL",
        None => "n/a",
    }
}
