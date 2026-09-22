//! The right-click caret rule, and what the context menu would offer.
//!
//! The popup itself is a native Win32 menu running its own modal loop — it cannot be clicked by a
//! test, and is checked by eye like every other menu here. What *can* be tested is the part that was
//! actually a decision: where a right-click leaves the caret, and which entries would be enabled.
//!
//! The commands the menu sends are the menu bar's own, already covered by `tabs.rs` and
//! `clipboard.rs`. A second door, not a second implementation — so there is nothing new to assert
//! about what Cut does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::clipboard::Clipboard;
use mdedit_app::content::Offset;
use mdedit_app::editor::Editor;
use mdedit_app::find;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// A clipboard holding whatever it was given.
#[derive(Debug, Default)]
struct Held(Option<String>);

impl Clipboard for Held {
    fn read(&mut self) -> Option<String> {
        self.0.clone()
    }
    fn write(&mut self, text: &str) {
        self.0 = Some(text.to_owned());
    }
}

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// The window position of character `column` on row 0.
///
/// Asked of the content rect rather than assumed to be `MARGIN`: chrome above the document moves
/// its top edge, and a test that hardcodes the edge fails the day a bar is added rather than the day
/// the hit-testing breaks.
fn at(editor: &Editor<Grid>, column: usize) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    (
        ADVANCE.mul_add(column as f32, editor.content_rect().left_edge()) + ADVANCE / 2.0,
        editor
            .content_rect()
            .to_window(0.0, 0.0, Offset::down(0.0))
            .1
            + LINE_HEIGHT / 2.0,
    )
}

fn select(editor: &mut Editor<Grid>, from: usize, to: usize) {
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(from), ByteOffset::new(to)));
}

/// The rule this ticket is really about: right-clicking inside the selection must leave it alone,
/// or Cut and Copy grey out just as the user reaches for them.
#[test]
fn a_right_click_inside_the_selection_keeps_it() {
    let mut editor = editor("hello world\n");
    select(&mut editor, 0, 5);

    let (x, y) = at(&editor, 2);
    assert!(editor.right_click_at(x, y));

    assert_eq!(
        editor.document().selection().span().start(),
        ByteOffset::new(0)
    );
    assert_eq!(
        editor.document().selection().span().end(),
        ByteOffset::new(5)
    );
    assert!(editor.context_state().has_selection);
}

/// And outside it, the caret moves there first — a menu acting on something the pointer is nowhere
/// near is worse than one that acts on nothing.
#[test]
fn a_right_click_outside_the_selection_moves_the_caret() {
    let mut editor = editor("hello world\n");
    select(&mut editor, 0, 5);

    let (x, y) = at(&editor, 8);
    assert!(editor.right_click_at(x, y));

    assert!(
        editor.document().selection().is_collapsed(),
        "the selection should have collapsed to a caret"
    );
    assert!(!editor.context_state().has_selection);
}

/// The boundary case the `<` in the rule decides: the offset one past the end of the selection is
/// outside it.
#[test]
fn the_end_of_the_selection_counts_as_outside() {
    let mut editor = editor("hello world\n");
    select(&mut editor, 0, 5);

    let (x, y) = at(&editor, 5);
    editor.right_click_at(x, y);

    assert!(editor.document().selection().is_collapsed());
}

#[test]
fn a_right_click_with_nothing_selected_just_places_the_caret() {
    let mut editor = editor("hello world\n");

    let (x, y) = at(&editor, 6);
    assert!(editor.right_click_at(x, y));

    assert!(editor.document().selection().is_collapsed());
    // 7, not 6: a press in the middle of a cell rounds to the boundary after it, which is what puts
    // the caret where the pointer looks like it is rather than a character behind.
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(7),
        "the caret should be where the pointer was"
    );
}

/// Chrome has no context menu of its own, and a document menu over the tab strip would be worse
/// than nothing.
#[test]
fn a_right_click_on_chrome_shows_nothing() {
    let mut editor = editor("hello world\n");
    assert!(
        !editor.right_click_at(5.0, 2.0),
        "above the document is not the document"
    );
}

/// **The find panel is the one piece of chrome inside the content rect** (MDEDIT-177), so it needs
/// an arm of its own here that the bands above the document never did.
///
/// Found in the review of #183. Right-clicking the search field to paste a term placed the caret in
/// the text *underneath* the panel, collapsed the current match's selection, and popped the
/// document's menu over the panel — because this path asked `ContentRect::contains` alone, which was
/// a sound question only while the bar sat outside the rect.
#[test]
fn a_right_click_on_the_find_panel_is_not_a_right_click_in_the_document() {
    let mut editor = editor("hello world\n");
    select(&mut editor, 0, 5);
    editor.open_find();
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    let layout = editor.find_layout().expect("the panel was drawn").clone();
    let (left, top) = layout.origin;
    let field = layout.field;
    let on_the_field = (left + field.0 + 4.0, top + find::HEIGHT / 2.0);
    assert!(
        editor
            .content_rect()
            .contains(on_the_field.0, on_the_field.1),
        "the point has to be inside the content, or this asserts nothing new",
    );

    assert!(
        !editor.right_click_at(on_the_field.0, on_the_field.1),
        "the panel has no context menu, and the document's is not its",
    );
    assert_eq!(
        (
            editor.document().selection().span().start().get(),
            editor.document().selection().span().end().get()
        ),
        (0, 5),
        "and the selection under it must not have been collapsed",
    );
}

#[test]
fn cut_and_copy_are_offered_only_with_a_selection() {
    let mut editor = editor("hello world\n");
    assert!(!editor.context_state().has_selection);

    select(&mut editor, 0, 5);
    assert!(editor.context_state().has_selection);
}

#[test]
fn paste_is_offered_only_when_the_clipboard_has_text() {
    let mut editor = editor("hello world\n");

    editor.with_clipboard(Box::new(Held::default()));
    assert!(!editor.context_state().can_paste);

    editor.with_clipboard(Box::new(Held(Some("something".to_owned()))));
    assert!(editor.context_state().can_paste);

    // An empty string on the clipboard is nothing to paste, not something.
    editor.with_clipboard(Box::new(Held(Some(String::new()))));
    assert!(!editor.context_state().can_paste);
}

/// A right-click is not a click: it must not arm the selection drag, so moving the pointer
/// afterwards does nothing rather than sweeping out a selection.
///
/// Asserted on the drag *state* rather than by calling `drag_to`, which is what the winit handler
/// guards on — calling `drag_to` directly would step over the guard and test nothing.
#[test]
fn a_right_click_does_not_arm_a_selection_drag() {
    // A left click does arm one, which is what makes the assertion below mean something.
    let mut left = editor("hello world\n");
    let (x, y) = at(&left, 2);
    left.click(x, y);
    assert!(left.dragging());

    let mut right = editor("hello world\n");
    right.right_click_at(x, y);
    assert!(!right.dragging());
    assert!(!right.dragging_thumb());
}

/// Select All is a context-menu entry, and it goes through the same command the chord does.
#[test]
fn select_all_from_the_menu_selects_everything() {
    let mut editor = editor("hello world\n");
    editor.apply_command(mdedit_menu_win::Command::SelectAll);

    assert_eq!(
        editor.document().selection().span().end(),
        ByteOffset::new(editor.document().len_bytes())
    );
}
