//! One button, two menus: the size picker outside a table and the edit list inside one (MDEDIT-128).
//!
//! **This is what kept a group of nine controls off a bar with room for one.** The plan had them
//! either replacing the formatting buttons while the caret was in a table, or going to the overflow;
//! both were answers to a width problem that reusing the Table button does not have. GFM has no
//! nested table, so the picker has nothing to offer inside one and the button is free to mean the
//! other thing there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_app::toolbar::{self, Action};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{Grid, MARGIN};

const WIDE: f32 = 1000.0;
const HEIGHT: f32 = MARGIN + 400.0;

const DOC: &str = "prose above\n\n| Keys   | Does |\n| ------ | ---- |\n| Ctrl+B | Bold |\n";

fn editor() -> Editor<Grid> {
    let mut editor = Editor::new(DOC, Grid::new());
    let _ = editor.build_frame(WIDE, HEIGHT, 1.0);
    editor
}

/// Puts the caret at the first byte of `needle` and redraws.
fn caret_at(editor: &mut Editor<Grid>, needle: &str) {
    let at = DOC.find(needle).expect("the needle is in the document");
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(at)));
    let _ = editor.build_frame(WIDE, HEIGHT, 1.0);
}

/// Clicks the Table button.
fn click_table(editor: &mut Editor<Grid>) {
    let button = editor
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == Action::Table)
        .copied()
        .expect("Table is on the bar");
    editor.click(button.x + button.width / 2.0, toolbar::HEIGHT / 2.0);
    let _ = editor.build_frame(WIDE, HEIGHT, 1.0);
}

#[test]
fn outside_a_table_the_button_opens_the_size_picker() {
    let mut editor = editor();
    caret_at(&mut editor, "prose above");
    click_table(&mut editor);

    assert_eq!(editor.menu_kind(), Some(toolbar::Menu::TableSize));
}

#[test]
fn inside_a_table_the_same_button_opens_the_edit_list() {
    let mut editor = editor();
    caret_at(&mut editor, "Ctrl+B");
    click_table(&mut editor);

    assert_eq!(editor.menu_kind(), Some(toolbar::Menu::TableEdit));
    assert_eq!(editor.menu_rows(), Some(toolbar::TABLE_EDIT));
}

/// The header row is in the table too — the menu is about the table, not about being past its first
/// line.
#[test]
fn the_header_row_is_inside_the_table_as_well() {
    let mut editor = editor();
    caret_at(&mut editor, "Keys");
    click_table(&mut editor);

    assert_eq!(editor.menu_kind(), Some(toolbar::Menu::TableEdit));
}

/// Every row says what it does in words, because none of them carries an icon.
#[test]
fn every_row_of_the_edit_list_is_read_by_its_words() {
    for action in toolbar::TABLE_EDIT {
        assert!(action.icon().is_none(), "{action:?} should carry no icon");
        assert!(
            !action.describes().is_empty(),
            "{action:?} must say what it does"
        );
        assert!(
            mdedit_app::popup::text_for(*action).is_some(),
            "{action:?} needs its text column"
        );
    }
}

// ---- the operations, through the menu ----

/// The whole path: caret in a table, open the list, click a row, and the bytes change.
#[test]
fn clicking_a_row_edits_the_table() {
    let mut editor = editor();
    caret_at(&mut editor, "Ctrl+B");
    assert!(editor.perform(Action::RowBelow));

    assert!(
        editor.document().to_string().ends_with(
            "| Keys   | Does |\n| ------ | ---- |\n| Ctrl+B | Bold |\n|        |      |\n"
        ),
        "a row went in below: {}",
        editor.document()
    );
}

#[test]
fn an_operation_outside_a_table_does_nothing() {
    let mut editor = editor();
    caret_at(&mut editor, "prose above");

    for action in toolbar::TABLE_EDIT {
        assert!(!editor.perform(*action), "{action:?} outside a table");
    }
    assert_eq!(editor.document().to_string(), DOC);
}

#[test]
fn select_row_takes_the_cells_and_not_the_pipes() {
    let mut editor = editor();
    caret_at(&mut editor, "Ctrl+B");
    assert!(editor.perform(Action::SelectRow));

    let text = editor.document().to_string();
    let span = editor.document().selection().span();
    assert_eq!(span.slice(&text), " Ctrl+B | Bold ");
}

#[test]
fn select_table_takes_the_whole_table_and_not_the_prose() {
    let mut editor = editor();
    caret_at(&mut editor, "Ctrl+B");
    assert!(editor.perform(Action::SelectTable));

    let text = editor.document().to_string();
    let span = editor.document().selection().span();
    assert!(span.slice(&text).starts_with("| Keys"));
    assert!(span.slice(&text).ends_with("| Bold |"));
    assert!(!span.slice(&text).contains("prose"));
}

/// None of these is a mode the caret can be in, so none of them lights.
#[test]
fn no_row_of_the_edit_list_lights() {
    let mut editor = editor();
    caret_at(&mut editor, "Ctrl+B");

    for action in toolbar::TABLE_EDIT {
        assert!(!editor.is_lit(*action), "{action:?} should not light");
    }
}
