//! The scrollbar: its arithmetic, its hit regions, and what it does under a correcting estimate.
//!
//! The geometry half is pure and tested directly. The editor half goes through `build_frame` with
//! the stub backend, so the numbers are grid metrics rather than real glyph ones — which is what
//! makes the assertions about *positions* meaningful rather than font-dependent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::scrollbar::{self, MIN_THUMB, WIDTH};

mod stub;
use stub::Grid;

const WINDOW: f32 = 400.0;
const VIEWPORT: f32 = 200.0;

/// A document of `lines` lines, tall enough to need scrolling.
fn document(lines: usize) -> String {
    use std::fmt::Write as _;

    (0..lines).fold(String::new(), |mut out, index| {
        let _ = writeln!(out, "line {index}");
        out
    })
}

// ---- geometry ----

#[test]
fn a_document_that_fits_has_no_bar() {
    assert!(scrollbar::layout(WINDOW, 0.0, VIEWPORT, VIEWPORT, 0.0).is_none());
    assert!(scrollbar::layout(WINDOW, 0.0, VIEWPORT, VIEWPORT - 1.0, 0.0).is_none());
}

#[test]
fn the_bar_sits_against_the_right_edge() {
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, VIEWPORT * 4.0, 0.0).unwrap();
    assert_eq!(bar.x, WINDOW - WIDTH);
}

/// The thumb is as much of the track as the viewport is of the document.
#[test]
fn the_thumb_is_the_visible_fraction() {
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, VIEWPORT * 4.0, 0.0).unwrap();
    assert_eq!(bar.thumb_height, VIEWPORT / 4.0);
}

/// Without a floor, a 10 MB document gives a thumb under a pixel high — visible, and impossible to
/// grab.
#[test]
fn a_huge_document_still_has_a_grabbable_thumb() {
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, 10_000_000.0, 0.0).unwrap();
    assert_eq!(bar.thumb_height, MIN_THUMB);
}

/// And the floor cannot make a thumb taller than the track it sits in.
#[test]
fn a_tiny_viewport_does_not_get_an_oversized_thumb() {
    let tiny = MIN_THUMB / 2.0;
    let bar = scrollbar::layout(WINDOW, 0.0, tiny, tiny * 100.0, 0.0).unwrap();
    assert!(bar.thumb_height <= tiny);
}

#[test]
fn the_thumb_starts_at_the_top_and_ends_at_the_bottom() {
    let content = VIEWPORT * 4.0;
    let top = scrollbar::layout(WINDOW, 0.0, VIEWPORT, content, 0.0).unwrap();
    assert_eq!(top.thumb_top, 0.0);

    // The furthest the view can go, which is the content less one screenful — not the content
    // height, which the offset never reaches.
    let bottom = scrollbar::layout(WINDOW, 0.0, VIEWPORT, content, content - VIEWPORT).unwrap();
    assert_eq!(bottom.thumb_bottom(), VIEWPORT);
}

/// The track starts below whatever chrome is above the document, not at the window's top.
#[test]
fn the_track_starts_where_it_is_told_to() {
    let bar = scrollbar::layout(WINDOW, 30.0, VIEWPORT, VIEWPORT * 4.0, 0.0).unwrap();
    assert_eq!(bar.top, 30.0);
    assert_eq!(bar.thumb_top, 30.0);
}

// ---- hit regions ----

#[test]
fn the_three_regions_are_where_they_look() {
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, VIEWPORT * 4.0, VIEWPORT).unwrap();
    let x = bar.x + WIDTH / 2.0;

    assert_eq!(
        scrollbar::hit_test(Some(bar), x, bar.thumb_top - 1.0),
        Some(scrollbar::Hit::PageUp)
    );
    assert_eq!(
        scrollbar::hit_test(Some(bar), x, bar.thumb_top + 1.0),
        Some(scrollbar::Hit::Thumb)
    );
    assert_eq!(
        scrollbar::hit_test(Some(bar), x, bar.thumb_bottom() + 1.0),
        Some(scrollbar::Hit::PageDown)
    );
}

/// The drift test every piece of chrome here carries: a press one pixel left of the bar is not on
/// the bar.
#[test]
fn a_press_just_left_of_the_bar_misses_it() {
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, VIEWPORT * 4.0, 0.0).unwrap();
    assert!(scrollbar::hit_test(Some(bar), bar.x - 0.5, 50.0).is_none());
    assert!(scrollbar::hit_test(Some(bar), bar.x, 50.0).is_some());
}

#[test]
fn a_press_outside_the_track_misses_it() {
    let bar = scrollbar::layout(WINDOW, 30.0, VIEWPORT, VIEWPORT * 4.0, 0.0).unwrap();
    let x = bar.x + WIDTH / 2.0;
    assert!(scrollbar::hit_test(Some(bar), x, 29.0).is_none());
    assert!(scrollbar::hit_test(Some(bar), x, 30.0 + VIEWPORT).is_none());
}

#[test]
fn there_is_nothing_to_hit_when_there_is_no_bar() {
    assert!(scrollbar::hit_test(None, 100.0, 100.0).is_none());
}

// ---- the mapping ----

#[test]
fn dragging_the_thumb_to_the_bottom_reaches_the_end() {
    let content = VIEWPORT * 4.0;
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, content, 0.0).unwrap();

    let offset = scrollbar::offset_for_thumb(&bar, content, bar.top + bar.travel());
    assert_eq!(offset, content - VIEWPORT);
}

#[test]
fn dragging_the_thumb_past_the_ends_is_clamped() {
    let content = VIEWPORT * 4.0;
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, content, 0.0).unwrap();

    assert_eq!(scrollbar::offset_for_thumb(&bar, content, -500.0), 0.0);
    assert_eq!(
        scrollbar::offset_for_thumb(&bar, content, 5000.0),
        content - VIEWPORT
    );
}

/// The honesty this ticket is really about. The total corrects as chunks are measured, and the
/// mapping follows it rather than holding the total the drag started with — a stale total is what
/// makes the mapping tear mid-drag.
#[test]
fn the_mapping_follows_a_corrected_total() {
    let estimated = VIEWPORT * 4.0;
    let bar = scrollbar::layout(WINDOW, 0.0, VIEWPORT, estimated, 0.0).unwrap();
    let halfway = bar.top + bar.travel() / 2.0;

    let before = scrollbar::offset_for_thumb(&bar, estimated, halfway);
    // The document turned out to be taller than estimated once its chunks were measured.
    let measured = VIEWPORT * 6.0;
    let after = scrollbar::offset_for_thumb(&bar, measured, halfway);

    assert!(
        after > before,
        "the same thumb position in a taller document must mean a larger offset: {before} then \
         {after}"
    );
}

// ---- the editor ----

#[test]
fn a_tall_document_gets_a_bar_and_a_short_one_does_not() {
    let mut tall = Editor::new(&document(200), Grid::new());
    tall.build_frame(WINDOW, VIEWPORT, 1.0);
    assert!(tall.scrollbar().is_some());

    let mut short = Editor::new("one line\n", Grid::new());
    short.build_frame(WINDOW, VIEWPORT, 1.0);
    assert!(short.scrollbar().is_none());
}

/// The bar overlays the text, so with no bar the right edge is text — and a click there must reach
/// the document rather than being swallowed.
#[test]
fn with_no_bar_a_click_at_the_right_edge_reaches_the_document() {
    let mut editor = Editor::new("hello world\n", Grid::new());
    editor.build_frame(WINDOW, VIEWPORT, 1.0);

    assert!(editor.click(WINDOW - 1.0, 30.0));
    assert!(!editor.dragging_thumb());
}

#[test]
fn a_press_on_the_thumb_starts_a_drag_and_release_ends_it() {
    let mut editor = Editor::new(&document(200), Grid::new());
    editor.build_frame(WINDOW, VIEWPORT, 1.0);
    let bar = editor.scrollbar().unwrap();

    editor.click(bar.x + WIDTH / 2.0, bar.thumb_top + 1.0);
    assert!(editor.dragging_thumb());
}

/// A press on the track pages, and the caret stays where it was — this is a view movement, as the
/// page keys already are.
#[test]
fn a_press_below_the_thumb_pages_without_moving_the_caret() {
    let mut editor = Editor::new(&document(200), Grid::new());
    editor.build_frame(WINDOW, VIEWPORT, 1.0);
    let bar = editor.scrollbar().unwrap();
    let caret = editor.document().selection().head();

    editor.click(bar.x + WIDTH / 2.0, bar.thumb_bottom() + 5.0);

    assert!(editor.scroll_offset() > 0.0, "it should have paged down");
    assert_eq!(
        editor.document().selection().head(),
        caret,
        "scrolling must not move the caret"
    );
}

#[test]
fn a_press_above_the_thumb_pages_back() {
    let mut editor = Editor::new(&document(200), Grid::new());
    editor.build_frame(WINDOW, VIEWPORT, 1.0);
    editor.scroll_by(400.0);
    editor.build_frame(WINDOW, VIEWPORT, 1.0);
    let scrolled = editor.scroll_offset();
    let bar = editor.scrollbar().unwrap();

    editor.click(bar.x + WIDTH / 2.0, bar.thumb_top - 5.0);

    assert!(editor.scroll_offset() < scrolled);
}

/// The thumb moves with the pointer rather than jumping so its top sits under it.
#[test]
fn grabbing_the_middle_of_the_thumb_does_not_jump_it() {
    let mut editor = Editor::new(&document(200), Grid::new());
    editor.build_frame(WINDOW, VIEWPORT, 1.0);
    let bar = editor.scrollbar().unwrap();

    // Grab the middle of the thumb while the view is at the very top.
    editor.click(bar.x + WIDTH / 2.0, bar.thumb_top + bar.thumb_height / 2.0);
    editor.build_frame(WINDOW, VIEWPORT, 1.0);

    assert_eq!(
        editor.scroll_offset(),
        0.0,
        "merely grabbing the thumb must not scroll"
    );
}

/// The scrollbar draws last of all the chrome, per the MDEDIT-39 ordering rule — so nothing painted
/// after it can cover it.
#[test]
fn the_thumb_is_the_last_thing_drawn() {
    let mut editor = Editor::new(&document(200), Grid::new());
    let quads = editor.build_frame(WINDOW, VIEWPORT, 1.0);
    let bar = editor.scrollbar().unwrap();

    let last = quads.last().expect("the frame should not be empty");
    assert_eq!(
        last.rect[0], bar.x,
        "the last quad should be the thumb, at the bar's x"
    );
    assert_eq!(last.rect[2], WIDTH);
}
