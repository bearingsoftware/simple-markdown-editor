//! Counting clicks, because winit does not.
//!
//! winit reports presses and releases and leaves "was that a double-click?" to the application. So
//! this counts them: a press within the threshold of the last one, and close enough to it in space,
//! is the next in a run.
//!
//! # The threshold is injected
//!
//! Windows has a double-click time and it is a setting, reachable from Ease of Access. A hardcoded
//! 500 ms ignores somebody who has deliberately made it longer. So the app reads
//! `GetDoubleClickTime` and passes it in — and because it is a parameter rather than a call, a test
//! can drive this with explicit timestamps instead of sleeping.
//!
//! # And so is the position
//!
//! A double-click that drifts a couple of pixels between presses is still a double-click; one that
//! lands in a different word is two clicks that happened to be quick. Windows has
//! `SM_CXDOUBLECLK`/`SM_CYDOUBLECLK` for this, and it is 4 pixels by default, which is what
//! [`SLOP`] is.

use std::time::{Duration, Instant};

/// Default double-click time, for an editor that has not been told the system's.
///
/// Matches the Windows default. Used when nothing supplies the real one, which off Windows is
/// always.
pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(500);

/// How far a press may drift from the last one and still continue the run, in logical pixels.
///
/// The Windows default for `SM_CXDOUBLECLK`. Not read from the system: unlike the interval, this one
/// is not a setting anybody adjusts, and a wrong value here shows up as a double-click that did not
/// take rather than as an accessibility failure.
pub const SLOP: f32 = 4.0;

/// How much of the text one click selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Granularity {
    /// One position. A single click.
    #[default]
    Character,
    /// A whole word. A double-click.
    Word,
    /// A whole source line. A triple-click.
    Line,
}

impl Granularity {
    /// What a run of `count` clicks means.
    ///
    /// Wraps at three rather than growing: a quadruple-click starts again at a caret, which is what
    /// Windows and every editor on it do, and means a run of clicks cycles rather than sticking.
    #[must_use]
    pub fn of(count: u32) -> Self {
        match count % 3 {
            2 => Self::Word,
            0 => Self::Line,
            _ => Self::Character,
        }
    }
}

/// Counts presses into runs.
#[derive(Debug)]
pub struct Clicks {
    interval: Duration,
    last: Option<(Instant, f32, f32)>,
    count: u32,
}

impl Default for Clicks {
    fn default() -> Self {
        Self {
            interval: DEFAULT_INTERVAL,
            last: None,
            count: 0,
        }
    }
}

impl Clicks {
    /// A counter using the system's double-click time.
    #[must_use]
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            interval,
            ..Self::default()
        }
    }

    /// Records a press and says which of the run it is, counting from one.
    pub fn press(&mut self, at: Instant, x: f32, y: f32) -> u32 {
        let continues = self.last.is_some_and(|(when, last_x, last_y)| {
            // `checked_duration_since` rather than subtraction: a caller passing an earlier instant
            // than the last would panic on the plain form, and a clock that appears to go backwards
            // should end the run rather than take the process with it.
            at.checked_duration_since(when)
                .is_some_and(|since| since <= self.interval)
                && (x - last_x).abs() <= SLOP
                && (y - last_y).abs() <= SLOP
        });

        self.count = if continues { self.count + 1 } else { 1 };
        self.last = Some((at, x, y));
        self.count
    }

    /// Forgets the run, so the next press starts a new one.
    ///
    /// For anything that means the pointer has moved on to different business — activating another
    /// tab, or a click that landed on chrome rather than on the document.
    pub fn reset(&mut self) {
        self.last = None;
        self.count = 0;
    }
}
