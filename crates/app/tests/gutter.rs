//! The line-number gutter.
//!
//! Two claims worth pinning. That nothing changes when it is off — this is opt-in, and an opt-in
//! feature that alters the default rendering is a bug. And that clicking still lands on the right
//! byte when it is on, because the gutter moves where the text starts, and the drawing and the
//! hit-testing agreeing about that is the whole point of `ContentRect`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;

use mdedit_app::config::Config;
use mdedit_app::editor::Editor;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor_with(source: &str, line_numbers: bool) -> Editor<Grid> {
    let mut editor = Editor::new(source, Grid::new());
    editor.with_config(Config { line_numbers });
    editor
}

/// A document of `count` numbered lines.
fn lines(count: usize) -> String {
    let mut text = String::new();
    for n in 0..count {
        writeln!(text, "line {n}").expect("a String never fails to write");
    }
    text
}

/// Where the text starts after a frame has decided it.
fn text_left(source: &str, line_numbers: bool) -> f32 {
    let mut editor = editor_with(source, line_numbers);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.content_rect().left_edge()
}

fn line_y(n: f32) -> f32 {
    MARGIN + n.mul_add(LINE_HEIGHT, LINE_HEIGHT / 2.0)
}

/// The rightmost edge any line number reaches, and where the text starts.
///
/// The gutter's quads are the ones left of the text and above the status bar, whose breadcrumb also
/// begins over on the left.
fn number_right_and_text_left(source: &str) -> (f32, f32) {
    let mut editor = editor_with(source, true);
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let left = editor.content_rect().left_edge();
    let bar_top = HEIGHT - mdedit_app::status::BAR_HEIGHT;

    let right = quads
        .iter()
        .filter(|quad| quad.rect[0] < left && quad.rect[1] < bar_top)
        .map(|quad| quad.rect[0] + quad.rect[2])
        .fold(0.0_f32, f32::max);
    (right, left)
}

/// An opt-in feature that changes the default rendering is a bug.
#[test]
fn nothing_changes_when_it_is_off() {
    let source = "# Title\n\nSome text here.\n\nMore text.\n";
    let mut without = Editor::new(source, Grid::new());
    let mut configured_off = editor_with(source, false);

    assert_eq!(
        without.build_frame(WIDTH, HEIGHT, 1.0).len(),
        configured_off.build_frame(WIDTH, HEIGHT, 1.0).len(),
        "an absent config and an explicit false must draw the same thing"
    );
    assert_eq!(
        text_left(source, false),
        MARGIN,
        "with it off the text starts at the margin, as it always did"
    );
}

#[test]
fn turning_it_on_draws_more() {
    let source = "# Title\n\nSome text here.\n\nMore text.\n";
    let mut off = editor_with(source, false);
    let mut on = editor_with(source, true);

    let without = off.build_frame(WIDTH, HEIGHT, 1.0).len();
    let with = on.build_frame(WIDTH, HEIGHT, 1.0).len();
    assert!(
        with > without,
        "the numbers have to be drawn as well: {without} then {with}"
    );
}

#[test]
fn the_text_moves_right_to_make_room() {
    let source = "hello\n";
    assert!(
        text_left(source, true) > text_left(source, false),
        "the gutter takes its space from the left"
    );
}

/// The failure this whole arrangement exists to prevent: text drawn in one place and clicks landing
/// in another. With the gutter on, a click has to follow the text right by the gutter's width.
#[test]
fn a_click_follows_the_text_when_the_gutter_pushes_it_right() {
    let source = "hello world\n\nsecond line\n";

    let mut off = editor_with(source, false);
    let _ = off.build_frame(WIDTH, HEIGHT, 1.0);
    let bare_left = off.content_rect().left_edge();
    off.click_at(bare_left + 4.0 * ADVANCE, line_y(2.0));
    let without_gutter = off.document().selection().head();

    let mut on = editor_with(source, true);
    let _ = on.build_frame(WIDTH, HEIGHT, 1.0);
    let gutter_left = on.content_rect().left_edge();
    on.click_at(gutter_left + 4.0 * ADVANCE, line_y(2.0));

    assert_eq!(
        on.document().selection().head(),
        without_gutter,
        "the same column of the same line should be the same byte either way"
    );
}

/// Clicking in the gutter itself is not a dead zone — it puts the caret at the start of that row,
/// as clicking to the left of any line does.
#[test]
fn clicking_in_the_gutter_lands_at_the_start_of_the_row() {
    let source = "first line\n\nthird line\n";
    let mut editor = editor_with(source, true);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.click_at(2.0, line_y(2.0));
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(12),
        "the start of the third line, not a no-op"
    );
}

/// MDEDIT-34: numbers sat against the text rather than in the gutter.
///
/// The assertion the original tests were missing. They all measured where the *text* started, which
/// was right either way — the numbers were being drawn past the gutter and into the document's left
/// margin, and nothing looked at where they landed.
///
/// A number must not cross into the margin. The gutter is a whole number of digit columns plus a
/// padding either side, so a full-width number ends one padding short of the gutter's own right
/// edge, and there is a clear margin's worth of space between it and the first character of text.
#[test]
fn a_number_stays_inside_the_gutter() {
    // Two digits, which is the gutter's floor, so the widest number exactly fills its column.
    let (right, text_left) = number_right_and_text_left(&lines(12));

    assert!(
        right < text_left - MARGIN,
        "a number reaching {right} has crossed out of the gutter and into the {MARGIN}px margin \
         before the text at {text_left}"
    );

    // And specifically: right-aligned one padding in from the gutter's edge, not from the text's.
    let gutter = text_left - MARGIN;
    let padding = gutter - right;
    assert!(
        padding > 0.0 && padding < MARGIN,
        "the gap between the number's end and the gutter's edge should be the gutter's own padding, \
         not the document margin: {padding}"
    );
}

/// The ones column still lines up when the numbers are not all the same width — which is what
/// centring each number individually would have broken.
#[test]
fn numbers_of_different_widths_share_a_right_edge() {
    // Spans 9 to 10, so both a one-digit and a two-digit number are on screen at once.
    let (right, _) = number_right_and_text_left(&lines(12));
    let (wider, _) = number_right_and_text_left(&lines(120));

    assert!(
        wider > right,
        "a three-digit gutter is wider, so its numbers end further right: {right} then {wider}"
    );
}

/// The gutter is sized for the largest number it will show, so it widens when the document does.
#[test]
fn the_gutter_widens_when_the_document_crosses_a_power_of_ten() {
    let short = lines(5);
    let long = lines(150);

    assert!(
        text_left(&long, true) > text_left(&short, true),
        "a 150-line document needs a digit more room than a 5-line one"
    );
}

/// Two digits is the floor, so a short file does not visibly resize its gutter the moment it grows
/// past nine lines.
#[test]
fn a_tiny_document_still_gets_a_two_digit_gutter() {
    let one = text_left("a\n", true);
    let eight = lines(8);

    assert_eq!(
        one,
        text_left(&eight, true),
        "both fit in two digits, so the gutter should be the same width"
    );
}

/// The point of the feature: the numbers match the file. A wrapped paragraph is one source line
/// however many rows it takes, so its continuation rows carry no number.
#[test]
fn a_wrapped_line_is_numbered_once() {
    // Narrow enough that the long line has to wrap: the stub advances 8px per character.
    let narrow = MARGIN * 3.0 + ADVANCE * 12.0;
    let source = "short\n\nthis line is much longer and will wrap over several rows\n\nlast\n";

    let mut editor = editor_with(source, true);
    let quads = editor.build_frame(narrow, HEIGHT, 1.0);
    let left = editor.content_rect().left_edge();

    // Numbers are the only thing drawn left of where the text starts and above the status bar,
    // whose breadcrumb also begins left of the text.
    let bar_top = HEIGHT - mdedit_app::status::BAR_HEIGHT;
    let numbered: BTreeSet<i32> = quads
        .iter()
        .filter(|quad| quad.rect[0] < left && quad.rect[1] < bar_top)
        .map(|quad| {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "quad positions are small pixel coordinates"
            )]
            let row = quad.rect[1].round() as i32;
            row
        })
        .collect();

    // Six source lines: three with text, two blank, and the trailing one.
    assert_eq!(
        numbered.len(),
        6,
        "one number per source line, however many rows each wraps to: {numbered:?}"
    );

    // And the long line really did wrap, or this proves nothing.
    let rows: BTreeSet<i32> = quads
        .iter()
        .filter(|quad| quad.rect[0] >= left && quad.rect[1] < bar_top)
        .map(|quad| {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "quad positions are small pixel coordinates"
            )]
            let row = quad.rect[1].round() as i32;
            row
        })
        .collect();
    assert!(
        rows.len() > numbered.len(),
        "the document should occupy more rows than it has source lines: {rows:?}"
    );
}
