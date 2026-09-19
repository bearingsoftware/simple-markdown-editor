//! Replace and Go to Line as wired into the editor.
//!
//! `crates/document/tests/replace_all.rs` proves the edit is right and that one undo takes it back;
//! `tests/find.rs` proves the bar's geometry and its three fields. What is left, and what is here,
//! is the wiring — which is where this project's defects have actually been.
//!
//! Two claims are worth more than the rest:
//!
//! - **Replace does not replace whatever the caret is near.** Pressing it away from a match moves to
//!   one; the second press is what edits. A Replace that acted on the nearest thing would lose work.
//! - **A click on the second row is not a click on the first.** The bar is two rows tall in Replace
//!   mode, and every hit test in this editor has got a new band of chrome wrong at least once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::field::Press;
use mdedit_app::find::{self, Key};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn frame(editor: &mut Editor<Grid>) {
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
}

/// Opens the bar in Replace mode with both fields filled, then draws — which is what searches.
fn replacing(editor: &mut Editor<Grid>, needle: &str, with: &str) {
    editor.apply_chord("h", false);
    editor.apply_find_key(&Key::Edit(Press::Text(needle.to_owned())), false);
    editor.apply_find_key(&Key::NextField, false);
    editor.apply_find_key(&Key::Edit(Press::Text(with.to_owned())), false);
    frame(editor);
}

fn caret_at(editor: &mut Editor<Grid>, offset: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(offset)));
}

/// Halfway down a row of the find bar, in window coordinates. Asked of the content rect rather than
/// assumed, because there is a formatting bar above it.
fn row_y(editor: &Editor<Grid>, row: f32) -> f32 {
    let bar_top =
        editor.content_rect().top_edge() - MARGIN - find::height(editor.find_bar().unwrap().mode);
    bar_top + row + find::HEIGHT / 2.0
}

fn button_x(editor: &Editor<Grid>, button: find::Button) -> f32 {
    let layout = editor.find_layout().expect("the bar was drawn");
    let rect = layout
        .buttons
        .iter()
        .find(|rect| rect.button == button)
        .unwrap_or_else(|| panic!("{button:?} should be on the bar"));
    rect.x + rect.width / 2.0
}

// ---- opening ----

#[test]
fn ctrl_h_opens_the_bar_with_two_fields() {
    let mut editor = editor("alpha beta\n");
    editor.apply_chord("h", false);
    frame(&mut editor);
    assert_eq!(editor.find_bar().unwrap().mode, find::Mode::Replace);
    assert!(editor.find_layout().unwrap().replace.is_some());
}

#[test]
fn replace_is_two_rows_of_the_documents_height() {
    let mut editor = editor("alpha\n");
    let plain = editor.content_rect().top_edge();

    editor.apply_chord("f", false);
    frame(&mut editor);
    let finding = editor.content_rect().top_edge();

    editor.apply_chord("h", false);
    frame(&mut editor);
    assert_eq!(
        editor.content_rect().top_edge() - finding,
        find::HEIGHT,
        "replace should be exactly one row taller than find"
    );
    assert_eq!(finding - plain, find::HEIGHT);
}

#[test]
fn tab_moves_between_the_two_fields() {
    let mut editor = editor("alpha\n");
    replacing(&mut editor, "alpha", "delta");
    assert_eq!(editor.find_bar().unwrap().field.text(), "alpha");
    assert_eq!(editor.find_bar().unwrap().replace.text(), "delta");
}

/// Going back to Find leaves focus on a field that is no longer drawn unless something resets it.
#[test]
fn reopening_as_find_puts_focus_back_on_the_search_term() {
    let mut editor = editor("alpha\n");
    replacing(&mut editor, "alpha", "delta");
    assert_eq!(editor.find_bar().unwrap().focus, find::Focus::Secondary);

    editor.apply_chord("f", false);
    assert_eq!(editor.find_bar().unwrap().mode, find::Mode::Find);
    assert_eq!(editor.find_bar().unwrap().focus, find::Focus::Primary);
}

#[test]
fn the_replacement_survives_switching_modes() {
    let mut editor = editor("alpha\n");
    replacing(&mut editor, "alpha", "delta");
    editor.apply_chord("f", false);
    editor.apply_chord("h", false);
    assert_eq!(editor.find_bar().unwrap().replace.text(), "delta");
}

// ---- replacing one ----

/// The claim that matters. Replace with the caret away from a match moves to one rather than editing
/// whatever is nearby.
#[test]
fn replace_away_from_a_match_moves_to_one_first() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 7);
    replacing(&mut editor, "alpha", "delta");

    assert!(editor.replace_one());
    assert_eq!(
        editor.document().to_string(),
        "alpha beta alpha\n",
        "the first press should only have moved"
    );
    assert_eq!(editor.document().selection().span().start().get(), 11);
}

#[test]
fn replace_on_a_match_replaces_it_and_moves_on() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    replacing(&mut editor, "alpha", "delta");

    editor.find_next();
    assert!(editor.replace_one());
    assert_eq!(editor.document().to_string(), "delta beta alpha\n");
    // And it has moved on to the other one, so a second press replaces that.
    assert!(editor.replace_one());
    assert_eq!(editor.document().to_string(), "delta beta delta\n");
}

/// The classic infinite loop. Replacing `a` with `aa` must move past what it just wrote.
#[test]
fn a_replacement_containing_the_needle_does_not_loop() {
    let mut editor = editor("a b a\n");
    caret_at(&mut editor, 0);
    replacing(&mut editor, "a", "aa");

    editor.find_next();
    assert!(editor.replace_one());
    assert!(editor.replace_one());
    assert_eq!(editor.document().to_string(), "aa b aa\n");
}

#[test]
fn replace_does_nothing_in_find_mode() {
    let mut editor = editor("alpha\n");
    editor.apply_chord("f", false);
    editor.apply_find_key(&Key::Edit(Press::Text("alpha".to_owned())), false);
    frame(&mut editor);
    assert!(!editor.replace_one());
    assert_eq!(editor.replace_all(), 0);
}

// ---- replacing all ----

#[test]
fn replace_all_replaces_every_match_and_says_how_many() {
    let mut editor = editor("alpha beta alpha gamma alpha\n");
    replacing(&mut editor, "alpha", "delta");
    assert_eq!(editor.replace_all(), 3);
    assert_eq!(
        editor.document().to_string(),
        "delta beta delta gamma delta\n"
    );
}

/// The reason MDEDIT-45 was waiting on MDEDIT-63.
#[test]
fn one_undo_takes_back_a_whole_replace_all() {
    let text = "alpha beta alpha gamma alpha\n";
    let mut editor = editor(text);
    replacing(&mut editor, "alpha", "delta");
    assert_eq!(editor.replace_all(), 3);

    assert!(editor.document_mut().undo());
    assert_eq!(editor.document().to_string(), text);
    assert!(
        !editor.document().can_undo(),
        "three sites should have been one step"
    );
}

#[test]
fn replace_all_with_nothing_found_changes_nothing() {
    let mut editor = editor("alpha\n");
    replacing(&mut editor, "zzz", "delta");
    assert_eq!(editor.replace_all(), 0);
    assert_eq!(editor.document().to_string(), "alpha\n");
    assert!(!editor.document().can_undo());
}

#[test]
fn replace_all_honours_the_case_toggle() {
    let mut editor = editor("Alpha alpha\n");
    replacing(&mut editor, "alpha", "x");
    assert_eq!(editor.replace_all(), 2, "case-insensitive to begin with");

    editor.document_mut().undo();
    editor.toggle_find_case();
    frame(&mut editor);
    assert_eq!(editor.replace_all(), 1);
    assert_eq!(editor.document().to_string(), "Alpha x\n");
}

#[test]
fn an_empty_replacement_deletes_the_matches() {
    let mut editor = editor("a-b-c\n");
    editor.apply_chord("h", false);
    editor.apply_find_key(&Key::Edit(Press::Text("-".to_owned())), false);
    frame(&mut editor);
    assert_eq!(editor.replace_all(), 2);
    assert_eq!(editor.document().to_string(), "abc\n");
}

/// After a Replace All the matches are gone, so the bar has to say so rather than keep the old
/// count. That falls out of the revision moving, and this is what says it does.
#[test]
fn the_count_is_right_after_a_replace_all() {
    let mut editor = editor("alpha alpha\n");
    replacing(&mut editor, "alpha", "delta");
    assert_eq!(editor.replace_all(), 2);
    frame(&mut editor);
    assert!(editor.find_bar().unwrap().matches().is_empty());
}

// ---- clicking the second row ----

#[test]
fn clicking_replace_all_replaces_them_all() {
    let mut editor = editor("alpha beta alpha\n");
    replacing(&mut editor, "alpha", "delta");
    let x = button_x(&editor, find::Button::ReplaceAll);
    let y = row_y(&editor, find::HEIGHT);
    assert!(editor.click(x, y));
    assert_eq!(editor.document().to_string(), "delta beta delta\n");
}

#[test]
fn clicking_replace_replaces_one() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    replacing(&mut editor, "alpha", "delta");
    editor.find_next();

    let x = button_x(&editor, find::Button::Replace);
    let y = row_y(&editor, find::HEIGHT);
    assert!(editor.click(x, y));
    assert_eq!(editor.document().to_string(), "delta beta alpha\n");
}

/// A click on the second row that lands on the same x as a first-row button must not fire it.
#[test]
fn a_click_on_the_second_row_is_not_a_click_on_the_first() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    replacing(&mut editor, "alpha", "delta");

    let close = button_x(&editor, find::Button::Close);
    assert!(editor.click(close, row_y(&editor, find::HEIGHT)));
    assert!(
        editor.find_open(),
        "the second row must not reach the Close button above it"
    );
}

#[test]
fn clicking_the_second_field_focuses_it() {
    let mut editor = editor("alpha\n");
    replacing(&mut editor, "alpha", "delta");
    // Put focus back on the first row the way a user would.
    let layout = editor.find_layout().unwrap().clone();
    editor.click(layout.field.0 + 4.0, row_y(&editor, 0.0));
    assert_eq!(editor.find_bar().unwrap().focus, find::Focus::Primary);

    let replace = layout.replace.expect("a replacement box");
    editor.click(replace.0 + 4.0, row_y(&editor, find::HEIGHT));
    assert_eq!(editor.find_bar().unwrap().focus, find::Focus::Secondary);
}

/// The drift test again, for a bar that is now two rows tall.
#[test]
fn a_click_below_a_two_row_bar_lands_on_the_byte_it_would_have() {
    let mut editor = editor("alpha beta gamma\nsecond line here\n");
    let point = (MARGIN + 30.0, MARGIN + 4.0);
    editor.click(point.0, point.1);
    let without = editor.document().selection().head();

    editor.apply_chord("h", false);
    frame(&mut editor);
    editor.click(point.0, point.1 + find::HEIGHT * 2.0);
    assert_eq!(editor.document().selection().head(), without);
}

// ---- go to line ----

fn go_to(editor: &mut Editor<Grid>, digits: &str) -> bool {
    editor.apply_chord("g", false);
    editor.apply_find_key(&Key::Edit(Press::Text(digits.to_owned())), false);
    editor.apply_find_key(&Key::Next, false)
}

#[test]
fn ctrl_g_opens_a_line_box() {
    let mut editor = editor("one\ntwo\nthree\n");
    editor.apply_chord("g", false);
    frame(&mut editor);
    assert_eq!(editor.find_bar().unwrap().mode, find::Mode::GoTo);
    assert!(editor.find_bar().unwrap().line.is_empty());
}

#[test]
fn go_to_line_counts_from_one() {
    let mut editor = editor("one\ntwo\nthree\n");
    assert!(go_to(&mut editor, "1"));
    assert_eq!(editor.document().selection().head().get(), 0);
}

#[test]
fn go_to_line_lands_on_the_line_it_names() {
    let mut editor = editor("one\ntwo\nthree\n");
    assert!(go_to(&mut editor, "3"));
    assert_eq!(editor.document().selection().head().get(), 8);
}

#[test]
fn a_line_past_the_end_is_clamped_rather_than_refused() {
    let mut editor = editor("one\ntwo\n");
    assert!(go_to(&mut editor, "999"));
    let (line, _) = editor
        .document()
        .line_and_column(editor.document().selection().head());
    assert_eq!(line.get(), editor.document().rope().len_lines() - 1);
}

#[test]
fn going_to_a_line_closes_the_bar() {
    let mut editor = editor("one\ntwo\nthree\n");
    go_to(&mut editor, "2");
    assert!(!editor.find_open(), "there is nothing to step through");
}

#[test]
fn an_empty_line_box_goes_nowhere() {
    let mut editor = editor("one\ntwo\n");
    caret_at(&mut editor, 5);
    editor.apply_chord("g", false);
    assert!(!editor.go_to_line());
    assert_eq!(editor.document().selection().head().get(), 5);
}

#[test]
fn go_to_line_does_nothing_in_the_other_modes() {
    let mut editor = editor("one\ntwo\n");
    editor.apply_chord("f", false);
    assert!(!editor.go_to_line());
}

/// A line a long way down materialises its chunk and scrolls to it, for the same reason a match
/// does: the caret moves, and the reveal follows.
#[test]
fn a_line_off_screen_is_scrolled_into_view() {
    let body = "a paragraph\n\n".repeat(80);
    let mut editor = editor(&body);
    caret_at(&mut editor, 0);
    frame(&mut editor);
    assert_eq!(editor.scroll_offset(), 0.0);

    go_to(&mut editor, "140");
    frame(&mut editor);
    assert!(editor.scroll_offset() > 0.0);
}
