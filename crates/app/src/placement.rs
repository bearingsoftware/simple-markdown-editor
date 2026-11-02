//! Whether a saved window placement is safe to open at (MDEDIT-198).
//!
//! The session file is untrusted input, so this is the only way a position or size from it reaches
//! the window. A placement that could leave the title bar unreachable, as when a monitor has been
//! unplugged since, is refused and the window opens at its default.

use mdedit_session::WindowPlacement;

/// The smallest window worth restoring, in logical pixels.
const MIN_WIDTH: f64 = 320.0;
const MIN_HEIGHT: f64 = 240.0;
/// The part of the window the user grabs to move it, in logical pixels.
const TITLE_STRIP: f64 = 32.0;

/// A monitor's usable rectangle in physical pixels, and its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorRect {
    /// Physical x of the left edge.
    pub x: i32,
    /// Physical y of the top edge.
    pub y: i32,
    /// Physical width.
    pub width: u32,
    /// Physical height.
    pub height: u32,
    /// Physical pixels per logical pixel.
    pub scale: f64,
}

/// The placement to open at, or `None` for the default size and position.
#[must_use]
pub fn restorable(saved: &WindowPlacement, monitors: &[MonitorRect]) -> Option<WindowPlacement> {
    if !(saved.width.is_finite() && saved.height.is_finite())
        || saved.width <= 0.0
        || saved.height <= 0.0
    {
        return None;
    }

    let mut best: Option<(f64, WindowPlacement)> = None;
    for monitor in monitors {
        if !(monitor.scale.is_finite() && monitor.scale > 0.0) {
            continue;
        }
        let logical_width = f64::from(monitor.width) / monitor.scale;
        let logical_height = f64::from(monitor.height) / monitor.scale;
        let width = saved.width.max(MIN_WIDTH).min(logical_width);
        let height = saved.height.max(MIN_HEIGHT).min(logical_height);

        let strip_left = f64::from(saved.x);
        let strip_top = f64::from(saved.y);
        let strip_right = strip_left + width * monitor.scale;
        let strip_bottom = strip_top + TITLE_STRIP * monitor.scale;
        let overlap_width = (strip_right.min(f64::from(monitor.x) + f64::from(monitor.width))
            - strip_left.max(f64::from(monitor.x)))
        .max(0.0);
        let overlap_height = (strip_bottom.min(f64::from(monitor.y) + f64::from(monitor.height))
            - strip_top.max(f64::from(monitor.y)))
        .max(0.0);
        let overlap = overlap_width * overlap_height;
        let strip = width * monitor.scale * TITLE_STRIP * monitor.scale;

        if overlap * 2.0 >= strip && best.as_ref().is_none_or(|(most, _)| overlap > *most) {
            best = Some((
                overlap,
                WindowPlacement {
                    width,
                    height,
                    ..*saved
                },
            ));
        }
    }
    best.map(|(_, placement)| placement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: i32, y: i32, width: u32, height: u32, scale: f64) -> MonitorRect {
        MonitorRect {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    fn at(x: i32, y: i32, width: f64, height: f64) -> WindowPlacement {
        WindowPlacement {
            x,
            y,
            width,
            height,
            maximized: false,
        }
    }

    const MAIN: MonitorRect = MonitorRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
    };

    #[test]
    fn fully_on_a_monitor_is_kept() {
        let saved = at(100, 100, 900.0, 700.0);
        assert_eq!(restorable(&saved, &[MAIN]), Some(saved));
    }

    #[test]
    fn maximised_flag_is_carried() {
        let saved = WindowPlacement {
            maximized: true,
            ..at(100, 100, 900.0, 700.0)
        };
        assert_eq!(restorable(&saved, &[MAIN]), Some(saved));
    }

    #[test]
    fn partly_on_with_the_title_strip_reachable_is_kept() {
        let saved = at(1300, 900, 900.0, 700.0);
        assert_eq!(restorable(&saved, &[MAIN]), Some(saved));
    }

    #[test]
    fn title_strip_off_the_top_is_refused() {
        assert_eq!(restorable(&at(100, -200, 900.0, 700.0), &[MAIN]), None);
    }

    #[test]
    fn title_strip_off_the_side_is_refused() {
        assert_eq!(restorable(&at(1900, 100, 900.0, 700.0), &[MAIN]), None);
    }

    #[test]
    fn fully_off_after_a_monitor_is_unplugged_is_refused() {
        assert_eq!(restorable(&at(2100, 100, 900.0, 700.0), &[MAIN]), None);
    }

    #[test]
    fn no_monitors_is_refused() {
        assert_eq!(restorable(&at(100, 100, 900.0, 700.0), &[]), None);
    }

    #[test]
    fn a_size_that_is_not_a_positive_number_is_refused() {
        for (width, height) in [
            (f64::NAN, 700.0),
            (900.0, f64::NAN),
            (f64::INFINITY, 700.0),
            (900.0, f64::NEG_INFINITY),
            (-900.0, 700.0),
            (900.0, -1.0),
            (0.0, 700.0),
            (900.0, 0.0),
        ] {
            assert_eq!(
                restorable(&at(100, 100, width, height), &[MAIN]),
                None,
                "{width} x {height}"
            );
        }
    }

    #[test]
    fn a_tiny_window_is_raised_to_the_minimum() {
        let kept =
            restorable(&at(100, 100, 5.0, 5.0), &[MAIN]).unwrap_or_else(|| at(0, 0, 0.0, 0.0));
        assert!((kept.width - 320.0).abs() < f64::EPSILON, "{kept:?}");
        assert!((kept.height - 240.0).abs() < f64::EPSILON, "{kept:?}");
    }

    #[test]
    fn a_window_larger_than_its_monitor_is_clamped_to_it() {
        let kept =
            restorable(&at(0, 0, 9000.0, 9000.0), &[MAIN]).unwrap_or_else(|| at(0, 0, 0.0, 0.0));
        assert!((kept.width - 1920.0).abs() < f64::EPSILON, "{kept:?}");
        assert!((kept.height - 1080.0).abs() < f64::EPSILON, "{kept:?}");
    }

    #[test]
    fn the_clamp_is_in_logical_pixels_on_a_scaled_monitor() {
        let scaled = monitor(0, 0, 3840, 2160, 2.0);
        let kept =
            restorable(&at(0, 0, 9000.0, 9000.0), &[scaled]).unwrap_or_else(|| at(0, 0, 0.0, 0.0));
        assert!((kept.width - 1920.0).abs() < f64::EPSILON, "{kept:?}");
        assert!((kept.height - 1080.0).abs() < f64::EPSILON, "{kept:?}");
    }

    #[test]
    fn a_window_on_the_second_monitor_is_kept_and_clamped_to_that_one() {
        let second = monitor(1920, 0, 1280, 720, 1.0);
        let saved = at(2000, 50, 1000.0, 2000.0);
        let kept = restorable(&saved, &[MAIN, second]).unwrap_or_else(|| at(0, 0, 0.0, 0.0));
        assert_eq!((kept.x, kept.y), (2000, 50));
        assert!((kept.width - 1000.0).abs() < f64::EPSILON, "{kept:?}");
        assert!((kept.height - 720.0).abs() < f64::EPSILON, "{kept:?}");
    }

    #[test]
    fn a_negative_origin_monitor_to_the_left_is_reachable() {
        let left = monitor(-1920, 0, 1920, 1080, 1.0);
        let saved = at(-1800, 100, 900.0, 700.0);
        assert_eq!(restorable(&saved, &[left, MAIN]), Some(saved));
    }
}
