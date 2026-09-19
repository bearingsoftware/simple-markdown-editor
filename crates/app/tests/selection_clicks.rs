//! Double-click, triple-click, and the unit a drag sticks to.
//!
//! Two layers. `clicks::Clicks` counts presses and is pure arithmetic over timestamps, so it is
//! tested directly with explicit instants rather than by sleeping. The editor layer is driven
//! through `click_at_time`, which exists so these tests can name the moment a press happened.
//!
//! What is not covered is the hit test: `offset_at_point` needs a laid-out frame, and the stub
//! backend gives one, so the coordinates below are grid positions rather than real glyph metrics.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use mdedit_app::clicks::{Clicks, Granularity, SLOP};
use mdedit_app::editor::Editor;

use crate::stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// An editor showing `text`, laid out once so there is something to hit-test against.
fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// The window position of character `column` on row `row`, in the middle of the cell.
fn at(column: usize, row: usize) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    (
        ADVANCE.mul_add(column as f32, MARGIN) + ADVANCE / 2.0,
        LINE_HEIGHT.mul_add(row as f32, MARGIN + mdedit_app::toolbar::HEIGHT) + LINE_HEIGHT / 2.0,
    )
}

/// What the editor currently has selected.
fn selected(editor: &Editor<Grid>) -> String {
    let text = editor.document().to_string();
    editor.document().selection().span().slice(&text).to_owned()
}

// ---- the counter itself ----

#[test]
fn presses_in_time_and_in_place_count_up() {
    let mut clicks = Clicks::with_interval(Duration::from_millis(500));
    let start = Instant::now();

    assert_eq!(clicks.press(start, 10.0, 10.0), 1);
    assert_eq!(
        clicks.press(start + Duration::from_millis(100), 10.0, 10.0),
        2
    );
    assert_eq!(
        clicks.press(start + Duration::from_millis(200), 10.0, 10.0),
        3
    );
}

#[test]
fn a_slow_second_press_starts_again() {
    let mut clicks = Clicks::with_interval(Duration::from_millis(500));
    let start = Instant::now();

    assert_eq!(clicks.press(start, 10.0, 10.0), 1);
    assert_eq!(
        clicks.press(start + Duration::from_millis(501), 10.0, 10.0),
        1
    );
}

/// A quick pair that lands somewhere else is two clicks, not a double-click.
#[test]
fn a_press_that_moved_too_far_starts_again() {
    let mut clicks = Clicks::with_interval(Duration::from_millis(500));
    let start = Instant::now();

    clicks.press(start, 10.0, 10.0);
    assert_eq!(
        clicks.press(start + Duration::from_millis(50), 10.0 + SLOP + 1.0, 10.0),
        1
    );
}

/// A hand that drifts a pixel between presses still meant a double-click.
#[test]
fn a_press_inside_the_slop_still_counts() {
    let mut clicks = Clicks::with_interval(Duration::from_millis(500));
    let start = Instant::now();

    clicks.press(start, 10.0, 10.0);
    assert_eq!(
        clicks.press(start + Duration::from_millis(50), 10.0 + SLOP, 10.0 + SLOP),
        2
    );
}

/// The interval is honoured rather than assumed — somebody who has lengthened the system setting
/// has done so because the default does not work for them.
#[test]
fn a_longer_interval_is_used_when_given() {
    let mut clicks = Clicks::with_interval(Duration::from_millis(900));
    let start = Instant::now();

    clicks.press(start, 10.0, 10.0);
    assert_eq!(
        clicks.press(start + Duration::from_millis(800), 10.0, 10.0),
        2
    );
}

/// A fourth click starts again at a caret rather than growing to something bigger than a line.
#[test]
fn a_run_of_clicks_cycles_rather_than_growing() {
    assert_eq!(Granularity::of(1), Granularity::Character);
    assert_eq!(Granularity::of(2), Granularity::Word);
    assert_eq!(Granularity::of(3), Granularity::Line);
    assert_eq!(Granularity::of(4), Granularity::Character);
}

#[test]
fn resetting_ends_the_run() {
    let mut clicks = Clicks::with_interval(Duration::from_millis(500));
    let start = Instant::now();

    clicks.press(start, 10.0, 10.0);
    clicks.reset();
    assert_eq!(
        clicks.press(start + Duration::from_millis(50), 10.0, 10.0),
        1
    );
}

// ---- the editor ----

#[test]
fn a_double_click_selects_the_word_under_it() {
    let mut editor = editor("hello world\n");
    let now = Instant::now();
    let (x, y) = at(7, 0);

    editor.click_at_time(x, y, now);
    editor.click_at_time(x, y, now + Duration::from_millis(80));

    assert_eq!(selected(&editor), "world");
}

#[test]
fn a_triple_click_selects_the_source_line() {
    let mut editor = editor("first line\nsecond line\n");
    let now = Instant::now();
    let (x, y) = at(3, 1);

    editor.click_at_time(x, y, now);
    editor.click_at_time(x, y, now + Duration::from_millis(80));
    editor.click_at_time(x, y, now + Duration::from_millis(160));

    assert_eq!(
        selected(&editor),
        "second line\n",
        "the newline is part of it, so deleting the selection removes the line"
    );
}

/// The regression that matters most: everything that worked before must still work when the clicks
/// are slow.
#[test]
fn slow_clicks_still_just_place_the_caret() {
    let mut editor = editor("hello world\n");
    let now = Instant::now();
    let (x, y) = at(7, 0);

    editor.click_at_time(x, y, now);
    editor.click_at_time(x, y, now + Duration::from_secs(2));

    assert!(
        selected(&editor).is_empty(),
        "a slow second click must place a caret, not select a word"
    );
}

#[test]
fn a_single_click_places_a_caret() {
    let mut editor = editor("hello world\n");
    let (x, y) = at(7, 0);

    editor.click_at_time(x, y, Instant::now());

    assert!(selected(&editor).is_empty());
}

/// The part casual implementations skip: after a double-click the drag grows by whole words, so
/// dragging out of the word you just selected does not snap back to characters.
#[test]
fn dragging_after_a_double_click_extends_by_words() {
    let mut editor = editor("alpha beta gamma\n");
    let now = Instant::now();
    let (x, y) = at(1, 0);

    editor.click_at_time(x, y, now);
    editor.click_at_time(x, y, now + Duration::from_millis(80));
    assert_eq!(selected(&editor), "alpha");

    // Into the middle of "gamma", not to its end.
    let (dx, dy) = at(13, 0);
    editor.drag_to(dx, dy);

    assert_eq!(
        selected(&editor),
        "alpha beta gamma",
        "the far word should be taken whole"
    );
}

/// And backwards, which is where taking the *near* end of the reached word matters.
#[test]
fn dragging_backwards_after_a_double_click_takes_whole_words() {
    let mut editor = editor("alpha beta gamma\n");
    let now = Instant::now();
    let (x, y) = at(12, 0);

    editor.click_at_time(x, y, now);
    editor.click_at_time(x, y, now + Duration::from_millis(80));
    assert_eq!(selected(&editor), "gamma");

    let (dx, dy) = at(2, 0);
    editor.drag_to(dx, dy);

    assert_eq!(selected(&editor), "alpha beta gamma");
}

#[test]
fn dragging_after_a_triple_click_extends_by_lines() {
    let mut editor = editor("one\ntwo\nthree\n");
    let now = Instant::now();
    let (x, y) = at(1, 0);

    editor.click_at_time(x, y, now);
    editor.click_at_time(x, y, now + Duration::from_millis(80));
    editor.click_at_time(x, y, now + Duration::from_millis(160));
    assert_eq!(selected(&editor), "one\n");

    // Into the middle of the third line.
    let (dx, dy) = at(2, 2);
    editor.drag_to(dx, dy);

    assert_eq!(selected(&editor), "one\ntwo\nthree\n");
}

/// A plain drag is unchanged: character-wise, exactly as before.
#[test]
fn a_plain_drag_is_still_character_wise() {
    let mut editor = editor("alpha beta\n");
    let (x, y) = at(1, 0);

    editor.click_at_time(x, y, Instant::now());
    let (dx, dy) = at(8, 0);
    editor.drag_to(dx, dy);

    // Columns 2..9: a press in the middle of a cell rounds to the boundary after it, which is what
    // puts the caret where the pointer looks like it is rather than a character behind.
    assert_eq!(selected(&editor), "pha bet");
}

/// Double-clicking whitespace selects the run, so a double-click never selects nothing.
#[test]
fn a_double_click_on_whitespace_selects_the_run() {
    let mut editor = editor("a    b\n");
    let now = Instant::now();
    let (x, y) = at(3, 0);

    editor.click_at_time(x, y, now);
    editor.click_at_time(x, y, now + Duration::from_millis(80));

    assert_eq!(selected(&editor), "    ");
}
