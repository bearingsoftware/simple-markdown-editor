//! Tab strip geometry and hit-testing.
//!
//! A click landing on the wrong tab, or closing one when it meant to select it, is invisible in a
//! screenshot and obvious to a user. So the arithmetic is tested rather than eyeballed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::tabs::{self, Hit, STRIP_HEIGHT};

#[test]
fn tabs_are_laid_out_left_to_right_without_gaps_or_overlap() {
    let rects = tabs::layout(&[80.0, 40.0, 120.0], 1000.0);
    assert_eq!(rects.len(), 3);

    assert_eq!(rects[0].x, 0.0, "the first tab starts at the left edge");
    for pair in rects.windows(2) {
        assert_eq!(
            pair[0].right(),
            pair[1].x,
            "tabs must abut exactly: no gap to click through, no overlap to fight over"
        );
    }
}

#[test]
fn a_tab_is_never_narrower_than_its_click_target_or_wider_than_the_cap() {
    // A one-character name still needs something to hit.
    let narrow = tabs::layout(&[1.0], 1000.0);
    assert!(narrow[0].width >= 70.0, "got {}", narrow[0].width);

    // And one very long name must not consume the whole strip.
    let wide = tabs::layout(&[10_000.0], 1000.0);
    assert!(wide[0].width <= 200.0, "got {}", wide[0].width);
}

/// The same discipline the document renderer is missing: do not lay out what cannot be seen.
#[test]
fn only_tabs_that_fit_are_laid_out() {
    let many = vec![100.0_f32; 200];
    let rects = tabs::layout(&many, 400.0);

    assert!(
        rects.len() < 10,
        "200 tabs in a 400px strip should produce a handful of rects, got {}",
        rects.len()
    );
    assert!(!rects.is_empty(), "at least one tab is visible");
    assert!(
        rects.last().unwrap().x < 400.0,
        "no tab should start past the right edge"
    );
}

#[test]
fn clicking_a_tab_body_selects_it() {
    let rects = tabs::layout(&[80.0, 80.0, 80.0], 1000.0);

    // A few pixels in from each tab's left edge is unambiguously its body.
    for rect in &rects {
        let hit = tabs::hit_test(&rects, rect.x + 5.0, STRIP_HEIGHT / 2.0);
        assert_eq!(hit, Some(Hit::Select(rect.index)), "at x {}", rect.x + 5.0);
    }
}

/// The close region sits inside the tab, so the body must not swallow it.
#[test]
fn clicking_the_close_region_closes_rather_than_selects() {
    let rects = tabs::layout(&[80.0, 80.0], 1000.0);

    for rect in &rects {
        let (close_x, close_width) = rect.close_region();
        let middle = close_x + close_width / 2.0;
        assert_eq!(
            tabs::hit_test(&rects, middle, STRIP_HEIGHT / 2.0),
            Some(Hit::Close(rect.index)),
            "the close region of tab {} must close it",
            rect.index
        );

        // And one pixel to its left is still the body.
        assert_eq!(
            tabs::hit_test(&rects, close_x - 1.0, STRIP_HEIGHT / 2.0),
            Some(Hit::Select(rect.index))
        );
    }
}

#[test]
fn clicking_below_the_strip_is_not_a_tab_click() {
    let rects = tabs::layout(&[80.0], 1000.0);
    assert_eq!(tabs::hit_test(&rects, 10.0, STRIP_HEIGHT + 1.0), None);
    assert_eq!(tabs::hit_test(&rects, 10.0, STRIP_HEIGHT), None);
    assert_eq!(
        tabs::hit_test(&rects, 10.0, STRIP_HEIGHT - 1.0),
        Some(Hit::Select(0)),
        "the last row of the strip still belongs to it"
    );
}

#[test]
fn clicking_empty_strip_space_hits_nothing() {
    let rects = tabs::layout(&[80.0], 1000.0);
    let past_the_end = rects[0].right() + 50.0;
    assert_eq!(tabs::hit_test(&rects, past_the_end, 5.0), None);
}

#[test]
fn no_tabs_means_no_hits() {
    assert_eq!(tabs::hit_test(&[], 10.0, 10.0), None);
    assert!(tabs::layout(&[], 1000.0).is_empty());
}

#[test]
fn a_strip_narrower_than_one_tab_still_shows_one() {
    // Better a clipped tab than an empty strip that looks broken.
    let rects = tabs::layout(&[80.0, 80.0], 10.0);
    assert_eq!(rects.len(), 1);
    assert_eq!(rects[0].index, 0);
}

#[test]
fn the_label_never_runs_under_the_close_region() {
    for width in [1.0_f32, 40.0, 500.0] {
        let rects = tabs::layout(&[width], 1000.0);
        let rect = rects[0];
        let (close_x, _) = rect.close_region();
        assert!(
            rect.label_x() + rect.label_width() <= close_x,
            "label for width {width} overlaps the close region"
        );
    }
}
