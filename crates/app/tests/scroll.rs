//! Scroll offset arithmetic.
//!
//! Clamping is the part that strands the last line off screen or lets the document drift into
//! empty space, and neither shows up in a screenshot until someone hits it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::scroll::Scroll;

const VIEWPORT: f32 = 600.0;
const LINE: f32 = 20.0;

#[test]
fn a_new_scroll_starts_at_the_top() {
    assert_eq!(Scroll::new().offset(), 0.0);
}

#[test]
fn scrolling_down_moves_by_the_requested_amount() {
    let mut scroll = Scroll::new();
    scroll.by(100.0, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 100.0);
    scroll.by(50.0, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 150.0);
}

#[test]
fn the_top_is_a_hard_stop() {
    let mut scroll = Scroll::new();
    scroll.by(-500.0, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 0.0, "cannot scroll above the first line");

    scroll.by(100.0, 2000.0, VIEWPORT);
    scroll.by(-1000.0, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 0.0);
}

#[test]
fn the_bottom_stops_with_the_last_line_in_view() {
    let mut scroll = Scroll::new();
    scroll.by(99_999.0, 2000.0, VIEWPORT);
    assert_eq!(
        scroll.offset(),
        1400.0,
        "the furthest down is content minus viewport, so the last line sits at the bottom edge"
    );
}

/// The case that would otherwise scroll a short note up into blank space.
#[test]
fn a_document_shorter_than_the_viewport_does_not_scroll_at_all() {
    let mut scroll = Scroll::new();
    scroll.by(500.0, 100.0, VIEWPORT);
    assert_eq!(scroll.offset(), 0.0);

    scroll.to_bottom(100.0, VIEWPORT);
    assert_eq!(scroll.offset(), 0.0);
    assert_eq!(Scroll::max_offset(100.0, VIEWPORT), 0.0);
}

#[test]
fn a_document_exactly_the_viewport_height_does_not_scroll() {
    assert_eq!(Scroll::max_offset(VIEWPORT, VIEWPORT), 0.0);
}

#[test]
fn jumping_to_the_ends() {
    let mut scroll = Scroll::new();
    scroll.to_bottom(2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 1400.0);
    scroll.to_top();
    assert_eq!(scroll.offset(), 0.0);
}

/// A page keeps one line of overlap, so the reader has something to anchor on.
#[test]
fn paging_keeps_a_line_of_overlap() {
    let mut scroll = Scroll::new();
    scroll.page(1.0, LINE, 10_000.0, VIEWPORT);
    assert_eq!(scroll.offset(), VIEWPORT - LINE);

    scroll.page(-1.0, LINE, 10_000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 0.0);
}

#[test]
fn paging_clamps_like_everything_else() {
    let mut scroll = Scroll::new();
    scroll.page(100.0, LINE, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 1400.0);
    scroll.page(-100.0, LINE, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 0.0);
}

/// A viewport shorter than one line must still move, or paging would freeze.
#[test]
fn paging_a_tiny_viewport_still_advances() {
    let mut scroll = Scroll::new();
    scroll.page(1.0, 100.0, 10_000.0, 50.0);
    assert!(scroll.offset() > 0.0, "got {}", scroll.offset());
}

// ---------------------------------------------------------------------------
// Scroll-follows-caret.
// ---------------------------------------------------------------------------

/// The rule that stops every keystroke jerking the page.
#[test]
fn a_caret_already_in_view_does_not_move_the_document() {
    let mut scroll = Scroll::new();
    scroll.by(300.0, 5000.0, VIEWPORT);

    // A line comfortably inside the visible band.
    scroll.reveal(400.0, 420.0, 5000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 300.0, "no movement was needed");
}

#[test]
fn a_caret_below_the_fold_pulls_the_view_down_by_the_minimum() {
    let mut scroll = Scroll::new();
    // Viewport shows 0..600. A caret at 900..920 is below it.
    scroll.reveal(900.0, 920.0, 5000.0, VIEWPORT);
    assert_eq!(
        scroll.offset(),
        320.0,
        "just enough to put the caret's bottom on the last visible row"
    );
}

#[test]
fn a_caret_above_the_view_pulls_it_up_by_the_minimum() {
    let mut scroll = Scroll::new();
    scroll.by(1000.0, 5000.0, VIEWPORT);
    scroll.reveal(200.0, 220.0, 5000.0, VIEWPORT);
    assert_eq!(
        scroll.offset(),
        200.0,
        "the caret's top sits at the top edge"
    );
}

#[test]
fn revealing_respects_the_document_bounds() {
    let mut scroll = Scroll::new();
    // A caret past the end of a short document cannot scroll past the clamp.
    scroll.reveal(5000.0, 5020.0, 700.0, VIEWPORT);
    assert_eq!(scroll.offset(), Scroll::max_offset(700.0, VIEWPORT));
}

/// A region taller than the viewport aligns to its top rather than its bottom.
#[test]
fn a_region_taller_than_the_viewport_shows_its_top() {
    let mut scroll = Scroll::new();
    scroll.by(2000.0, 10_000.0, VIEWPORT);
    scroll.reveal(500.0, 500.0 + VIEWPORT * 2.0, 10_000.0, VIEWPORT);
    assert_eq!(
        scroll.offset(),
        500.0,
        "losing sight of the caret is worse than losing the rest of a tall line"
    );
}

#[test]
fn setting_an_offset_directly_is_clamped_too() {
    let mut scroll = Scroll::new();
    scroll.set(-50.0, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 0.0);
    scroll.set(99_999.0, 2000.0, VIEWPORT);
    assert_eq!(scroll.offset(), 1400.0);
}

/// A degenerate content height must not produce a NaN offset that poisons every later frame.
#[test]
fn a_nonsense_content_height_does_not_produce_a_nan_offset() {
    let mut scroll = Scroll::new();
    scroll.set(100.0, f32::NAN, VIEWPORT);
    assert!(scroll.offset().is_finite(), "got {}", scroll.offset());

    scroll.set(100.0, 0.0, 0.0);
    assert!(scroll.offset().is_finite());
}
