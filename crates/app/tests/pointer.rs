//! What shape the pointer takes, region by region (MDEDIT-176).
//!
//! The editor drew the arrow everywhere, including over the text. These ask `pointer_at` the same
//! way `tests/tasks.rs` asks `click`: through the grid stub, at points worked out from the stub's
//! own metrics, so a region's shape is asserted without a window to move a real pointer over.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_app::pointer::Pointer;
use mdedit_app::toolbar;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

fn editor(source: &str) -> Editor<Grid> {
    let mut editor = Editor::new(source, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// A point on document row `row`, `columns` character widths in.
fn in_document(row: usize, columns: f32) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    let y = LINE_HEIGHT.mul_add(row as f32, MARGIN + toolbar::HEIGHT) + LINE_HEIGHT / 2.0;
    (ADVANCE.mul_add(columns, MARGIN), y)
}

fn pointer_at(editor: &Editor<Grid>, at: (f32, f32)) -> Pointer {
    editor.pointer_at(at.0, at.1)
}

// ---- the document ----

#[test]
fn over_the_text_it_is_an_i_beam() {
    let editor = editor("some prose to point at\n");
    assert_eq!(pointer_at(&editor, in_document(0, 3.5)), Pointer::Text);
}

/// Past the end of a short line is still the document, and still somewhere a click puts a caret.
#[test]
fn past_the_end_of_a_line_it_is_still_an_i_beam() {
    let editor = editor("short\n");
    assert_eq!(pointer_at(&editor, in_document(0, 40.0)), Pointer::Text);
}

/// **The one thing in a document that is pressed rather than typed into** (MDEDIT-142).
#[test]
fn over_a_task_box_it_is_a_hand() {
    let editor = editor("- [ ] buy milk\n");
    assert_eq!(pointer_at(&editor, in_document(0, 0.5)), Pointer::Hand);
    // And the item's text beside it is not.
    assert_eq!(pointer_at(&editor, in_document(0, 3.5)), Pointer::Text);
}

/// Plain markup draws the brackets as themselves, so there is no box to press.
#[test]
fn in_plain_markup_a_box_is_an_i_beam() {
    let mut editor = editor("- [ ] buy milk\n");
    editor.toggle_mode();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(pointer_at(&editor, in_document(0, 2.5)), Pointer::Text);
}

/// A bullet is substituted too, and it is not pressable.
#[test]
fn over_a_bullet_it_is_an_i_beam() {
    let editor = editor("- an item\n");
    assert_eq!(pointer_at(&editor, in_document(0, 0.5)), Pointer::Text);
}

// ---- the chrome ----

#[test]
fn over_the_formatting_bar_it_is_an_arrow() {
    let editor = editor("prose\n");
    assert_eq!(
        pointer_at(&editor, (WIDTH / 2.0, toolbar::HEIGHT / 2.0)),
        Pointer::Arrow
    );
}

/// The status bar is below the content, and `click` sends nothing down there to the document.
#[test]
fn over_the_status_bar_it_is_an_arrow() {
    let editor = editor("prose\n");
    assert_eq!(
        pointer_at(&editor, (WIDTH / 2.0, HEIGHT - 2.0)),
        Pointer::Arrow
    );
}

/// The gutter is left of the text and selects nothing when clicked.
#[test]
fn over_the_line_numbers_it_is_an_arrow() {
    let mut editor = editor("prose\n");
    editor.toggle_line_numbers();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        editor.content_rect().left_edge() > MARGIN,
        "no gutter drawn"
    );

    let y = MARGIN + toolbar::HEIGHT + LINE_HEIGHT / 2.0;
    assert_eq!(pointer_at(&editor, (2.0, y)), Pointer::Arrow);
}

#[test]
fn over_the_find_bar_a_field_is_an_i_beam_and_a_button_is_not() {
    let mut editor = editor("prose\n");
    editor.open_find();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let layout = editor.find_layout().expect("the bar is laid out").clone();

    // The bar sits under the formatting bar, and its own rects are measured from its top edge.
    let top = toolbar::HEIGHT;
    let (field_x, field_width) = layout.field;
    assert_eq!(
        pointer_at(&editor, (field_x + field_width / 2.0, top + 4.0)),
        Pointer::Text,
    );

    let button = *layout.buttons.first().expect("the bar has a button");
    assert_eq!(
        pointer_at(
            &editor,
            (button.x + button.width / 2.0, top + button.y + 4.0)
        ),
        Pointer::Arrow,
    );
}

/// A file pane takes width from the document rather than overlaying it, and its rows are controls.
#[test]
fn over_a_file_pane_row_it_is_an_arrow() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "prose\n").unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);

    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.toggle_chrome();
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    let y = toolbar::HEIGHT + LINE_HEIGHT;
    assert_eq!(pointer_at(&editor, (10.0, y)), Pointer::Arrow);
}

/// The scrollbar is a control, so it is the arrow — and it gets there without an arm of its own,
/// because it is drawn in the window's right margin rather than over the text.
#[test]
fn over_the_scrollbar_it_is_an_arrow() {
    let editor = editor(&"a line of prose\n".repeat(200));
    let bar = editor.scrollbar().expect("a long document has a scrollbar");
    let at = (
        bar.x + mdedit_app::scrollbar::WIDTH / 2.0,
        bar.thumb_top + 1.0,
    );
    assert!(
        !editor.content_rect().contains(at.0, at.1),
        "the bar is outside the content, which is why it needs no arm: {at:?}",
    );
    assert_eq!(pointer_at(&editor, at), Pointer::Arrow);
}

/// **A dialog is modal and drawn over the text**, so every point inside it is the dialog's: an
/// I-beam on a field, the arrow on everything else, and the document's shape nowhere inside it.
#[test]
fn over_a_dialog_a_field_is_an_i_beam_and_its_panel_is_not() {
    let mut editor = editor("prose\n");
    editor.perform(toolbar::Action::Link);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let fields = editor.dialog().expect("Link opens a dialog").fields().len();

    let at = mdedit_app::dialog::place(WIDTH, HEIGHT, mdedit_app::dialog::height_of(fields));
    let (fx, fy, fw, fh) = mdedit_app::dialog::field_rect(0, at);
    assert_eq!(
        pointer_at(&editor, (fx + fw / 2.0, fy + fh / 2.0)),
        Pointer::Text,
    );

    // Its own background, which swallows a click and points at nothing.
    assert_eq!(
        pointer_at(&editor, (at.0 + 4.0, at.1 + 4.0)),
        Pointer::Arrow
    );
}

// ---- a gesture holds its own shape ----

/// **Mid-drag the question is no longer "what is under the pointer".** Dragging a selection out of
/// the text and over the chrome keeps the I-beam, because the drag is still selecting text.
#[test]
fn a_selection_being_dragged_keeps_the_i_beam() {
    let mut editor = editor("some prose to select\n");
    let (x, y) = in_document(0, 2.0);
    editor.click(x, y);
    assert!(editor.dragging(), "the click should have started a drag");

    assert_eq!(
        pointer_at(&editor, (WIDTH / 2.0, toolbar::HEIGHT / 2.0)),
        Pointer::Text,
        "over the formatting bar, mid-drag",
    );
}

/// And a held scrollbar thumb keeps the arrow over the text, for the same reason.
#[test]
fn a_held_scrollbar_thumb_keeps_the_arrow() {
    let mut editor = editor(&"a line of prose\n".repeat(200));
    let bar = editor.scrollbar().expect("a long document has a scrollbar");
    editor.click(
        bar.x + mdedit_app::scrollbar::WIDTH / 2.0,
        bar.thumb_top + 1.0,
    );

    assert_eq!(
        pointer_at(&editor, in_document(0, 2.0)),
        Pointer::Arrow,
        "over the text, while the thumb is held",
    );
}

/// **A panel over the text takes the shape, because it takes the click.** The shapes and the clicks
/// ask the same predicates in the same order, and this is what says so.
#[test]
fn a_menu_over_the_document_wins() {
    let mut editor = editor("prose\n");
    // Opened the way a user opens it, so the panel lands where the editor really puts it.
    let button = editor
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == toolbar::Action::Headings)
        .copied()
        .expect("the headings button is on the bar");
    editor.click(button.x + button.width / 2.0, toolbar::HEIGHT / 2.0);
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    let (x, y) = editor.menu_at().expect("a list should be open");
    let width = editor.menu_width().expect("a list should be open");
    let rows = editor.menu_rows().expect("a list should be open").len();
    let in_the_list = (
        x + width / 2.0,
        y + mdedit_app::popup::row_top(rows - 1) + mdedit_app::popup::ROW_HEIGHT / 2.0,
    );
    assert!(
        editor.content_rect().contains(in_the_list.0, in_the_list.1),
        "the probe must be where the list covers the document: {in_the_list:?}",
    );

    assert_eq!(pointer_at(&editor, in_the_list), Pointer::Arrow);
}

// ---- the window is told only when the shape changes ----

/// A pointer crossing one region costs a comparison per event rather than a call into the window per
/// event, and the shape it ends up wearing is the one its point deserves.
#[test]
fn the_window_is_told_only_when_the_shape_changes() {
    let mut editor = editor("- [ ] buy milk and other things\n");

    let (x, y) = in_document(0, 4.0);
    assert!(editor.move_pointer(x, y), "arrow to I-beam is a change");
    assert_eq!(editor.pointer(), Pointer::Text);

    let (x, y) = in_document(0, 6.0);
    assert!(!editor.move_pointer(x, y), "still over text");
    assert_eq!(editor.pointer(), Pointer::Text);

    let (x, y) = in_document(0, 0.5);
    assert!(editor.move_pointer(x, y), "onto the box");
    assert_eq!(editor.pointer(), Pointer::Hand);
}

/// **What is under a still pointer changes without it moving** — a scroll, a panel, Ctrl+/ — so a
/// frame gives it its shape again. Here the pointer rests where a box is, and plain markup takes the
/// box away beneath it.
#[test]
fn a_frame_gives_a_still_pointer_its_shape_again() {
    let mut editor = editor("- [ ] buy milk\n");
    let (x, y) = in_document(0, 0.5);
    editor.move_pointer(x, y);
    assert_eq!(editor.pointer(), Pointer::Hand);

    editor.toggle_mode();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(editor.pointer(), Pointer::Text, "the box is gone");
}
