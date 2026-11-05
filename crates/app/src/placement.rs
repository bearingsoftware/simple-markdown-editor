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

/// How the window should open, decided from the session file and the monitors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opening {
    /// The validated normal rectangle, or `None` for the default.
    pub rect: Option<WindowPlacement>,
    /// Whether to open maximised. Kept even when the rectangle was refused, so a user who left
    /// the window maximised on a monitor that has since gone still gets a maximised window.
    pub maximized: bool,
}

/// Decides how to open from what the session file said. See [`restorable`].
#[must_use]
pub fn opening(saved: Option<&WindowPlacement>, monitors: &[MonitorRect]) -> Opening {
    Opening {
        rect: saved.and_then(|saved| restorable(saved, monitors)),
        maximized: saved.is_some_and(|saved| saved.maximized),
    }
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

    #[test]
    fn title_strip_is_measured_in_physical_pixels_at_scale_two() {
        let hidpi = monitor(0, 0, 3840, 2160, 2.0);
        assert!(restorable(&at(100, 2128, 900.0, 700.0), &[hidpi]).is_some());
        assert_eq!(restorable(&at(100, 2129, 900.0, 700.0), &[hidpi]), None);
    }

    #[test]
    fn title_strip_height_is_scaled_at_scale_two() {
        let hidpi = monitor(0, 0, 3840, 2160, 2.0);
        assert!(restorable(&at(2840, 1000, 900.0, 700.0), &[hidpi]).is_some());
    }

    #[test]
    fn the_strip_area_is_scaled_on_both_axes() {
        let hidpi = monitor(0, 0, 3840, 2160, 2.0);
        assert_eq!(restorable(&at(2940, 2128, 900.0, 700.0), &[hidpi]), None);
    }

    #[test]
    fn height_is_clamped_to_logical_height_at_a_fractional_scale() {
        let fractional = monitor(0, 0, 1920, 1080, 1.5);
        let kept = restorable(&at(100, 100, 900.0, 1000.0), &[fractional])
            .unwrap_or_else(|| at(0, 0, 0.0, 0.0));
        assert!((kept.width - 900.0).abs() < f64::EPSILON, "{kept:?}");
        assert!((kept.height - 720.0).abs() < f64::EPSILON, "{kept:?}");
    }

    #[test]
    fn a_monitor_whose_scale_is_not_a_positive_finite_number_is_ignored() {
        let valid = monitor(1920, 0, 1280, 720, 1.0);
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let bad = monitor(0, 0, 1920, 1080, scale);
            assert_eq!(
                restorable(&at(5000, 5000, 900.0, 700.0), &[bad]),
                None,
                "{scale}"
            );
            let kept = restorable(&at(2000, 100, 9000.0, 9000.0), &[bad, valid])
                .unwrap_or_else(|| at(0, 0, 0.0, 0.0));
            assert!(
                (kept.width - 1280.0).abs() < f64::EPSILON,
                "{scale}: {kept:?}"
            );
            assert!(
                (kept.height - 720.0).abs() < f64::EPSILON,
                "{scale}: {kept:?}"
            );
        }
    }

    #[test]
    fn a_window_straddling_two_scales_is_clamped_to_the_monitor_holding_most_of_it() {
        let logical_one = monitor(0, 0, 1920, 1080, 1.0);
        let hidpi = monitor(1920, 0, 2560, 1440, 2.0);
        let saved = at(800, 100, 9000.0, 9000.0);
        for monitors in [[logical_one, hidpi], [hidpi, logical_one]] {
            let kept = restorable(&saved, &monitors).unwrap_or_else(|| at(0, 0, 0.0, 0.0));
            assert!(
                (kept.width - 1280.0).abs() < f64::EPSILON,
                "{monitors:?}: {kept:?}"
            );
            assert!(
                (kept.height - 720.0).abs() < f64::EPSILON,
                "{monitors:?}: {kept:?}"
            );
        }
    }
}

#[cfg(test)]
mod opening_tests {
    use super::*;

    const MAIN: MonitorRect = MonitorRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
    };

    fn saved(x: i32, maximized: bool) -> WindowPlacement {
        WindowPlacement {
            x,
            y: 100,
            width: 900.0,
            height: 700.0,
            maximized,
        }
    }

    #[test]
    fn nothing_saved_opens_the_default() {
        let opened = opening(None, &[MAIN]);
        assert_eq!(opened.rect, None);
        assert!(!opened.maximized);
    }

    #[test]
    fn a_maximised_placement_opens_maximised_on_its_rectangle() {
        let placement = saved(100, true);
        let opened = opening(Some(&placement), &[MAIN]);
        assert_eq!(opened.rect, Some(placement));
        assert!(opened.maximized);
    }

    #[test]
    fn a_refused_rectangle_still_opens_maximised_at_the_default() {
        let opened = opening(Some(&saved(5000, true)), &[MAIN]);
        assert_eq!(opened.rect, None);
        assert!(opened.maximized);
    }

    #[test]
    fn a_refused_normal_window_opens_the_default_unmaximised() {
        let opened = opening(Some(&saved(5000, false)), &[MAIN]);
        assert_eq!(opened.rect, None);
        assert!(!opened.maximized);
    }

    #[test]
    fn a_refused_maximised_placement_on_a_scaled_monitor_still_opens_maximised() {
        let hidpi = MonitorRect {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
            scale: 2.0,
        };
        let opened = opening(Some(&saved(5000, true)), &[hidpi]);
        assert_eq!(opened.rect, None);
        assert!(opened.maximized);
    }
}
