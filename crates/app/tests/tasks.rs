//! Pressing a task's box ticks it (MDEDIT-142).
//!
//! The first thing in a document that is pressed rather than placed in. Every click here goes through
//! `Editor::click`, the dispatch the window uses, so the chrome it asks first is in the path too.
//!
//! Shift+click on a box extends the selection instead, and is checked by hand: no test can set the
//! modifiers winit reports, as `chords.rs` says of every shifted gesture.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

/// An editor showing `source`, laid out, with the caret at the end of it.
fn editor(source: &str) -> Editor<Grid> {
    let mut editor = Editor::new(source, Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(source.len())));
    editor.build_frame(600.0, 400.0, 1.0);
    editor
}

/// Presses the middle of character `column` on document row `row`, and draws the frame after it.
fn press(editor: &mut Editor<Grid>, row: usize, column: usize) -> bool {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    let (x, y) = (
        ADVANCE.mul_add(column as f32, MARGIN) + ADVANCE / 2.0,
        LINE_HEIGHT.mul_add(row as f32, MARGIN + mdedit_app::toolbar::HEIGHT) + LINE_HEIGHT / 2.0,
    );
    let acted = editor.click(x, y);
    editor.build_frame(600.0, 400.0, 1.0);
    acted
}

fn text(editor: &Editor<Grid>) -> String {
    editor.document().to_string()
}

#[test]
fn pressing_a_box_ticks_it_and_pressing_again_unticks_it() {
    let mut editor = editor("- [ ] buy milk\n");
    assert!(press(&mut editor, 0, 0));
    assert_eq!(text(&editor), "- [x] buy milk\n");
    assert!(press(&mut editor, 0, 0));
    assert_eq!(text(&editor), "- [ ] buy milk\n");
}

/// GFM reads a capital X as ticked, so it unticks like any other, and ticks back in lower case.
#[test]
fn a_capital_x_unticks() {
    let mut editor = editor("- [X] shout\n");
    press(&mut editor, 0, 0);
    assert_eq!(text(&editor), "- [ ] shout\n");
    press(&mut editor, 0, 0);
    assert_eq!(text(&editor), "- [x] shout\n");
}

/// The box sits after the number, and it is the box that is pressed.
#[test]
fn a_numbered_task_ticks_from_its_box() {
    let mut editor = editor("1. [ ] step\n");
    //  1  .  ␣  ☐
    press(&mut editor, 0, 3);
    assert_eq!(text(&editor), "1. [x] step\n");
}

/// Only the box of the row pressed, in a list of several and one level down.
#[test]
fn only_the_pressed_box_changes() {
    let mut editor = editor("- [ ] one\n  - [ ] two\n- [ ] three\n");
    //  ␣  ␣  ☐       the nested row
    press(&mut editor, 1, 2);
    assert_eq!(text(&editor), "- [ ] one\n  - [x] two\n- [ ] three\n");
}

/// **Pressing a box is not a click in the text.** The caret stays where it was, rather than
/// jumping to the item or into the brackets.
#[test]
fn the_caret_stays_where_it_was() {
    let source = "- [ ] buy milk\nsome prose\n";
    let mut editor = editor(source);
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(18)));
    press(&mut editor, 0, 0);
    assert_eq!(
        editor.document().selection(),
        Selection::caret(ByteOffset::new(18))
    );
}

/// One Ctrl+Z restores the box, and only the box: what was typed before it is still there.
#[test]
fn one_undo_restores_the_box_and_nothing_else() {
    let mut editor = editor("- [ ] buy\n");
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(9)));
    editor.insert_text(" milk");
    editor.build_frame(600.0, 400.0, 1.0);

    press(&mut editor, 0, 0);
    assert_eq!(text(&editor), "- [x] buy milk\n");

    assert!(editor.document_mut().undo());
    assert_eq!(text(&editor), "- [ ] buy milk\n", "the typing survives");
    assert_eq!(
        editor.document().selection(),
        Selection::caret(ByteOffset::new(14)),
        "and the caret is where the typing left it",
    );
}

/// A bullet is substituted too, and pressing one is a click like any other.
#[test]
fn pressing_a_bullet_changes_nothing() {
    let mut editor = editor("- buy milk\n");
    press(&mut editor, 0, 0);
    assert_eq!(text(&editor), "- buy milk\n");
    assert_eq!(editor.document().selection().head().get(), 0);
}

/// Pressing the item's text places the caret, however close it is to the box.
#[test]
fn pressing_the_text_after_a_box_places_the_caret() {
    let mut editor = editor("- [ ] buy milk\n");
    //  ☐  ␣  b  u  y     the middle of `u` is nearest the boundary after it
    //        6  7  8
    press(&mut editor, 0, 3);
    assert_eq!(text(&editor), "- [ ] buy milk\n");
    assert_eq!(editor.document().selection().head().get(), 8);
}

/// Plain markup draws the brackets, not a box, and a click on them places the caret among them.
#[test]
fn plain_markup_has_no_box_to_press() {
    let mut editor = editor("- [ ] buy milk\n");
    editor.toggle_mode();
    editor.build_frame(600.0, 400.0, 1.0);
    //  -  ␣  [       the `[` is the byte a box starts at, drawn as itself here
    press(&mut editor, 0, 2);
    assert_eq!(text(&editor), "- [ ] buy milk\n");
}

/// **Below the last row is not on it.** A point under the text resolves to the last row for placing a
/// caret, and a box on that row must not tick from a click in the space beneath the document.
#[test]
fn a_click_below_a_box_does_not_press_it() {
    let mut editor = editor("- [ ] last");
    press(&mut editor, 2, 0);
    assert_eq!(text(&editor), "- [ ] last");
}
