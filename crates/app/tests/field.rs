//! The single-line text field, on its own.
//!
//! It is the first thing in this editor other than the document that takes a keystroke, so the
//! things a text field is expected to do are asserted here rather than assumed: a selection is
//! replaced by what is typed, a left arrow with a selection collapses rather than moves, and a
//! multi-byte character is one press of Backspace.

use mdedit_app::field::Field;

fn typed(text: &str) -> Field {
    let mut field = Field::new();
    field.insert(text);
    field
}

#[test]
fn a_new_field_is_empty() {
    let field = Field::new();
    assert_eq!(field.text(), "");
    assert!(field.is_empty());
    assert_eq!(field.caret(), 0);
    assert_eq!(field.selected_text(), None);
}

#[test]
fn typing_appends_and_moves_the_caret() {
    let field = typed("hello");
    assert_eq!(field.text(), "hello");
    assert_eq!(field.caret(), 5);
}

#[test]
fn typing_replaces_the_selection() {
    let mut field = typed("hello");
    field.select_all();
    field.insert("bye");
    assert_eq!(field.text(), "bye");
    assert_eq!(field.caret(), 3);
    assert_eq!(field.selected_text(), None);
}

#[test]
fn a_paste_of_several_lines_becomes_one() {
    let mut field = Field::new();
    field.insert("first\nsecond\r\nthird");
    assert_eq!(field.text(), "firstsecondthird");
}

#[test]
fn backspace_removes_one_character_not_one_byte() {
    let mut field = typed("café");
    assert!(field.backspace());
    assert_eq!(field.text(), "caf");
}

#[test]
fn backspace_at_the_start_does_nothing() {
    let mut field = typed("a");
    field.home(false);
    assert!(!field.backspace());
    assert_eq!(field.text(), "a");
}

#[test]
fn backspace_removes_the_selection_rather_than_a_character() {
    let mut field = typed("hello");
    field.select_all();
    assert!(field.backspace());
    assert_eq!(field.text(), "");
}

#[test]
fn delete_removes_the_character_after_the_caret() {
    let mut field = typed("héllo");
    field.home(false);
    assert!(field.delete());
    assert_eq!(field.text(), "éllo");
}

#[test]
fn delete_at_the_end_does_nothing() {
    let mut field = typed("a");
    assert!(!field.delete());
    assert_eq!(field.text(), "a");
}

#[test]
fn arrows_step_by_character() {
    let mut field = typed("héllo");
    field.left(false);
    field.left(false);
    field.left(false);
    // Three characters back from the end is between `h` and `é`, which is byte 1 and not byte 3.
    assert_eq!(field.caret(), 1 + "é".len());
}

#[test]
fn a_left_arrow_with_a_selection_collapses_to_its_start() {
    let mut field = typed("hello");
    field.select_all();
    field.left(false);
    assert_eq!(field.caret(), 0);
    assert_eq!(field.selected_text(), None);
}

#[test]
fn a_right_arrow_with_a_selection_collapses_to_its_end() {
    let mut field = typed("hello");
    field.select_all();
    field.home(false);
    field.end(true);
    field.right(false);
    assert_eq!(field.caret(), 5);
    assert_eq!(field.selected_text(), None);
}

#[test]
fn shift_with_a_motion_drags_a_selection() {
    let mut field = typed("hello");
    field.left(true);
    field.left(true);
    assert_eq!(field.selected_text(), Some("lo"));
}

#[test]
fn home_and_end_reach_the_ends() {
    let mut field = typed("hello");
    field.home(false);
    assert_eq!(field.caret(), 0);
    field.end(false);
    assert_eq!(field.caret(), 5);
}

#[test]
fn shift_home_selects_back_to_the_start() {
    let mut field = typed("hello");
    field.home(true);
    assert_eq!(field.selected_text(), Some("hello"));
}

#[test]
fn setting_the_text_selects_it_so_the_next_keystroke_replaces_it() {
    let mut field = Field::new();
    field.set_text("previous search");
    assert_eq!(field.selected_text(), Some("previous search"));
    field.insert("new");
    assert_eq!(field.text(), "new");
}

#[test]
fn setting_the_text_flattens_line_breaks() {
    let mut field = Field::new();
    field.set_text("two\nlines");
    assert_eq!(field.text(), "two lines");
}

#[test]
fn clearing_resets_the_caret_too() {
    let mut field = typed("hello");
    field.clear();
    assert_eq!(field.text(), "");
    assert_eq!(field.caret(), 0);
    assert_eq!(field.selected_text(), None);
}

#[test]
fn typing_in_the_middle_inserts_rather_than_appends() {
    let mut field = typed("helo");
    field.left(false);
    field.left(false);
    field.insert("l");
    assert_eq!(field.text(), "hello");
    assert_eq!(field.caret(), 3);
}
