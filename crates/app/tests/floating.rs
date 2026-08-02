//! The bar that follows a selection: when it appears, where it goes, and when it leaves.
//!
//! The buttons are MDEDIT-52's, already tested, and so is the dispatch behind them. What is new here
//! is **lifecycle and positioning**, which is also the half of this feature that is easy to get
//! subtly wrong and impossible to check by looking at a screenshot.
//!
//! The lifecycle rules are asserted one at a time and by name, because they were written down as
//! rules before they were built. A bar that appears mid-drag, or one that chases a scrolling
//! viewport, is the classic bad implementation of this pattern — so "never during a drag" and
//! "hidden on scroll, not moved" get their own tests rather than being implied by another.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::toolbar::{self, Action, Anchor};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn select(editor: &mut Editor<Grid>, start: usize, end: usize) {
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(start), ByteOffset::new(end)));
}

/// A selection that has settled, and a frame drawn so the bar has somewhere to be.
fn settled(text: &str, start: usize, end: usize) -> Editor<Grid> {
    let mut editor = editor(text);
    select(&mut editor, start, end);
    assert!(editor.settle_selection(), "the selection should settle");
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

// ---- when it appears ----

#[test]
fn a_settled_selection_shows_the_bar() {
    let editor = settled("hello world\n", 0, 5);
    assert!(editor.floating_shown());
    assert!(editor.floating_at().is_some(), "and it has been placed");
}

/// The rule this feature lives or dies on. A bar that appeared while the button was still down
/// would chase the pointer across the document.
#[test]
fn it_never_appears_during_a_drag() {
    let mut editor = editor("hello world\n");
    editor.click_at(
        editor.content_rect().left_edge() + 1.0,
        editor.content_rect().to_window(0.0, 0.0, 0.0).1 + LINE_HEIGHT / 2.0,
    );
    assert!(editor.dragging(), "the click should have armed a drag");

    select(&mut editor, 0, 5);
    assert!(
        !editor.settle_selection(),
        "a selection still being dragged has not settled"
    );
    assert!(!editor.floating_shown());
}

#[test]
fn a_caret_alone_shows_nothing() {
    let mut editor = editor("hello world\n");
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(3)));
    assert!(!editor.settle_selection());
    assert!(!editor.floating_shown());
}

/// A selection extended with Shift settles on the same timer everything else debounced onto.
#[test]
fn a_keyboard_selection_settles_on_the_idle_timer() {
    let mut editor = editor("hello world\n");
    select(&mut editor, 0, 5);
    assert!(!editor.floating_shown(), "not until the typing stops");

    assert!(editor.on_idle(), "the idle pass should report the change");
    assert!(editor.floating_shown());
}

// ---- when it leaves ----

#[test]
fn escape_dismisses_it() {
    let mut editor = settled("hello world\n", 0, 5);
    assert!(editor.dismiss_floating());
    assert!(!editor.floating_shown());

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(editor.floating_at().is_none(), "and it stops being drawn");
}

/// Hidden, not moved. Following a viewport that is still moving is jitter.
#[test]
fn scrolling_dismisses_it_rather_than_moving_it() {
    let mut editor = settled("one\n\ntwo\n\nthree\n", 0, 3);
    editor.scroll_by(LINE_HEIGHT);
    assert!(!editor.floating_shown());
}

#[test]
fn collapsing_the_selection_takes_it_away() {
    let mut editor = settled("hello world\n", 0, 5);
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(3)));
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        editor.floating_at().is_none(),
        "there is nothing left for it to act on"
    );
}

/// The acceptance criterion: with the bar up, a click in the text still lands on the right byte and
/// the bar goes.
#[test]
fn a_click_in_the_text_dismisses_it_and_lands_correctly() {
    // Eight lines, and the click goes to the last of them — well clear of the bar, which sits over
    // the selection up at the top. A click that landed *on* the bar is a different test.
    let source = "hello world\n\nsecond line\n\nthird\n\nfourth\n\nfifth line\n";
    let mut without = editor(source);
    let top = without.content_rect().to_window(0.0, 0.0, 0.0).1;
    let left = without.content_rect().left_edge();
    let far_down = top + LINE_HEIGHT * 8.5;
    without.click_at(left + 4.0 * stub::ADVANCE, far_down);
    let expected = without.document().selection().head();

    let mut with = settled(source, 0, 5);
    let (_, bar_y, _) = with.floating_at().unwrap();
    assert!(
        far_down > bar_y + toolbar::HEIGHT,
        "the click has to miss the bar for this to test anything"
    );
    with.click(left + 4.0 * stub::ADVANCE, far_down);

    assert!(!with.floating_shown());
    assert_eq!(
        with.document().selection().head(),
        expected,
        "the click should land where it would have with no bar in the window"
    );
}

/// A selection scrolled off the top has no first row to hang on, so there is nowhere to draw it —
/// separately from the scroll rule, because this is geometry rather than lifecycle.
#[test]
fn a_selection_off_screen_has_nowhere_to_put_it() {
    use std::fmt::Write as _;
    let long = (0..200).fold(String::new(), |mut out, n| {
        let _ = writeln!(out, "line {n}");
        out
    });
    let mut editor = settled(&long, 0, 6);
    assert!(editor.floating_at().is_some());

    // Scrolled far past it, then settled again so the lifecycle is not what is being tested.
    editor.scroll_by(2000.0);
    assert!(editor.settle_selection());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(editor.floating_at().is_none());
}

#[test]
fn switching_tabs_takes_it_away() {
    let dir = TempDir::new().unwrap();
    let mut workspace = Workspace::new();
    for name in ["one.md", "two.md"] {
        let path = dir.path().join(name);
        std::fs::write(&path, "hello world\n").unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.show_tab(0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    select(&mut editor, 0, 5);
    assert!(editor.settle_selection());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(editor.floating_at().is_some());

    editor.show_tab(1);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        editor.floating_at().is_none(),
        "the bar belonged to the other document's selection"
    );
}

// ---- what a click on it does ----

#[test]
fn a_button_acts_and_the_bar_stays_with_its_state_changed() {
    let mut editor = settled("hello world\n", 0, 5);
    let (x, y, _) = editor.floating_at().unwrap();
    let bold = editor
        .floating_buttons()
        .iter()
        .find(|button| button.action == Action::Bold)
        .copied()
        .unwrap();

    assert!(editor.click(bold.x + bold.width / 2.0, y + toolbar::HEIGHT / 2.0));
    assert_eq!(editor.document().to_string(), "**hello** world\n");
    assert!(
        editor.floating_shown(),
        "it stays, so bold then italic is two clicks rather than two selections"
    );
    assert!(x >= editor.content_rect().left_edge());

    // And the state it shows has followed the edit.
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(editor.is_lit(Action::Bold));
}

#[test]
fn a_click_in_its_padding_is_still_a_click_on_it() {
    let mut editor = settled("hello world\n", 0, 5);
    let (x, y, _) = editor.floating_at().unwrap();
    let before = editor.document().to_string();

    // One pixel in from the left edge, which is inside the bar but before the first button.
    assert!(editor.click(x + 1.0, y + toolbar::HEIGHT / 2.0));
    assert_eq!(editor.document().to_string(), before, "and it did nothing");
    assert!(editor.floating_shown(), "and did not dismiss it either");
}

#[test]
fn the_selection_bar_offers_inline_actions_and_headings_only() {
    let editor = settled("hello world\n", 0, 5);
    let shown: Vec<Action> = editor
        .floating_buttons()
        .iter()
        .map(|button| button.action)
        .collect();
    assert_eq!(shown, toolbar::SELECTION);
    assert!(
        !shown.contains(&Action::Rule) && !shown.contains(&Action::Bullet),
        "the line-wise actions belong to the fixed bar"
    );
}

#[test]
fn a_json_selection_gets_no_bar() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("data.json");
    std::fs::write(&path, "{\"a\": 1}\n").unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

    select(&mut editor, 0, 5);
    assert!(editor.settle_selection());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(editor.floating_at().is_none());
}

// ---- positioning, on its own ----

const ROW: Anchor = Anchor {
    x: 200.0,
    y: 300.0,
    width: 100.0,
    height: 20.0,
};

#[test]
fn it_sits_centred_above_the_row() {
    let (x, y) = toolbar::place(80.0, ROW, 0.0, 1000.0, 0.0);
    assert_eq!(x, 210.0, "centred on the row's middle at 250");
    assert_eq!(y, 300.0 - toolbar::HEIGHT - toolbar::FLOAT_GAP);
}

/// It must never cover the selection it acts on, so with no room above it goes below the row rather
/// than over it.
#[test]
fn it_flips_below_when_there_is_no_room_above() {
    let near_top = Anchor { y: 10.0, ..ROW };
    let (_, y) = toolbar::place(80.0, near_top, 0.0, 1000.0, 0.0);
    assert_eq!(y, 10.0 + near_top.height + toolbar::FLOAT_GAP);
    assert!(y > near_top.y + near_top.height, "clear of the row");
}

#[test]
fn it_clamps_inside_the_content_at_both_edges() {
    let right_edge = Anchor { x: 960.0, ..ROW };
    let (x, _) = toolbar::place(80.0, right_edge, 0.0, 1000.0, 0.0);
    assert_eq!(x, 920.0, "its right edge stops at the content's");

    let left_edge = Anchor { x: 5.0, ..ROW };
    let (x, _) = toolbar::place(80.0, left_edge, 24.0, 1000.0, 0.0);
    assert_eq!(x, 24.0);
}

/// A window narrower than the bar: the left edge wins, because a bar clipped at the right is
/// readable from its start and one clipped at the left is not.
#[test]
fn a_window_too_narrow_for_it_keeps_its_left_edge() {
    let (x, _) = toolbar::place(400.0, ROW, 24.0, 300.0, 0.0);
    assert_eq!(x, 24.0);
}

/// The anchor is the first row, not the bounding box — the centre of a tall selection is over the
/// middle of the document, and the bar would appear nowhere near what was dragged over.
///
/// The selection starts on the fourth line so there is room above it; a selection at the very top of
/// the document has none, and that case is [`it_flips_below_when_there_is_no_room_above`].
#[test]
fn a_multi_line_selection_anchors_to_its_first_row() {
    let source = "one\n\ntwo\n\nfrom here\nto here\nand here\n";
    let start = source.find("from").unwrap();
    let mut editor = settled(source, start, source.len() - 1);

    let (_, y, _) = editor.floating_at().unwrap();
    let content_top = editor.content_rect().to_window(0.0, 0.0, 0.0).1;
    // Four visual rows down: "one", blank, "two", blank — the selection's first row is the fifth.
    let first_row_top = content_top + LINE_HEIGHT * 4.0;

    assert!(
        y < first_row_top && y > content_top,
        "above the selection's own first row at {first_row_top}, not over the middle of a \
         three-line selection and not off the top: {y}"
    );

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let (_, again, _) = editor.floating_at().unwrap();
    assert_eq!(again, y, "and a still frame puts it back in the same place");
}

// ---- what it costs ----

#[test]
fn a_still_frame_lays_out_no_floating_labels() {
    let mut editor = settled("hello world\n", 0, 5);
    let after_first = editor.text_backend().layouts();

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let since = editor.text_backend().layouts() - after_first;

    assert!(
        since <= 2,
        "the bar's glyphs are the fixed bar's, already cached; laid out {since} times"
    );
}
