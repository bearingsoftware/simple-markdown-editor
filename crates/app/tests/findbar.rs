//! The find bar as wired into the editor — the half `tests/find.rs` cannot see.
//!
//! `find.rs` proves the search is right and the geometry adds up. Everything here is about the
//! wiring, which is where this project's defects have actually been: a function that was tested and
//! a path to it that was not. So the claims are about routing and lifecycle —
//!
//! - a key goes to exactly one of the two things that take text, and it is the right one;
//! - a chord still reaches the document while the search box has the caret;
//! - the bar takes its height out of the document and gives it back, and a click below it lands on
//!   the byte it would have without it — the drift test that MDEDIT-52 needed and MDEDIT-39 is
//!   named after;
//! - navigation moves the *selection*, so the reveal and the highlight both follow for free.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::field::Press;
use mdedit_app::find::{self, Key};
use mdedit_app::view::Theme;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
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

/// Opens the bar and types `needle` into it, then draws — which is what refreshes the matches.
fn search(editor: &mut Editor<Grid>, needle: &str) {
    editor.apply_chord("f", false);
    editor.apply_find_key(&Key::Edit(Press::Text(needle.to_owned())), false);
    frame(editor);
}

/// Halfway down the find bar, in window coordinates.
///
/// Asked of the content rect rather than assumed to be at the top of the window: there is a
/// formatting bar above it, and a tab strip above that when a workspace is open. A test that
/// hardcoded a y would be testing the chrome stack rather than the find bar, and would break every
/// time something else was added to it.
fn bar_y(editor: &Editor<Grid>) -> f32 {
    editor.content_rect().top_edge() - MARGIN - find::HEIGHT / 2.0
}

/// Just inside the document, below every band of chrome.
fn text_y(editor: &Editor<Grid>) -> f32 {
    editor.content_rect().top_edge() + 4.0
}

fn selected(editor: &Editor<Grid>) -> (usize, usize) {
    let span = editor.document().selection().span();
    (span.start().get(), span.end().get())
}

/// Every caret drawn this frame, by x. The box's caret is one of them and the document's is the
/// other, which is enough: only one of the two moves when the box's caret does.
fn caret_xs(editor: &mut Editor<Grid>) -> Vec<u32> {
    let caret = Theme::default().caret;
    let wanted = [caret.r, caret.g, caret.b, caret.a];
    let mut xs: Vec<u32> = editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted)
        .map(|quad| quad.rect[0].to_bits())
        .collect();
    xs.sort_unstable();
    xs
}

fn caret_at(editor: &mut Editor<Grid>, offset: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(offset)));
}

// ---- opening and closing ----

#[test]
fn ctrl_f_opens_the_bar_with_the_keyboard() {
    let mut editor = editor("alpha beta alpha\n");
    assert!(!editor.find_open());
    editor.apply_chord("f", false);
    assert!(editor.find_open());
    assert!(editor.find_focused(), "the box should have the caret");
}

#[test]
fn ctrl_f_carries_the_selection_into_the_box() {
    let mut editor = editor("alpha beta alpha\n");
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(6), ByteOffset::new(10)));
    editor.apply_chord("f", false);
    assert_eq!(editor.find_bar().unwrap().field.text(), "beta");
}

#[test]
fn ctrl_f_keeps_the_previous_term_when_nothing_is_selected() {
    let mut editor = editor("alpha beta alpha\n");
    search(&mut editor, "alpha");
    editor.close_find();
    caret_at(&mut editor, 0);
    editor.apply_chord("f", false);
    assert_eq!(editor.find_bar().unwrap().field.text(), "alpha");
}

/// A whole paragraph is not a search term, and putting one in the box means the first thing Ctrl+F
/// does is something the user has to undo.
#[test]
fn ctrl_f_refuses_to_seed_from_an_enormous_selection() {
    let long = "word ".repeat(60);
    let mut editor = editor(&long);
    editor.select_all();
    editor.apply_chord("f", false);
    assert!(editor.find_bar().unwrap().field.is_empty());
}

#[test]
fn escape_closes_the_bar() {
    let mut editor = editor("alpha\n");
    search(&mut editor, "alpha");
    assert!(editor.apply_find_key(&Key::Close, false));
    assert!(!editor.find_open());
    assert!(!editor.find_focused());
}

#[test]
fn closing_an_already_closed_bar_says_so() {
    let mut editor = editor("alpha\n");
    assert!(!editor.close_find(), "nothing to close");
}

// ---- the room it takes ----

#[test]
fn the_bar_takes_its_height_out_of_the_document_and_gives_it_back() {
    let mut editor = editor("alpha\n");
    let before = editor.content_rect().top_edge();

    editor.apply_chord("f", false);
    frame(&mut editor);
    assert_eq!(
        editor.content_rect().top_edge() - before,
        find::HEIGHT,
        "the document should start exactly a bar lower"
    );

    editor.close_find();
    frame(&mut editor);
    assert_eq!(editor.content_rect().top_edge(), before);
}

/// The drift test. Chrome above the document must move what is below it, not draw over it — a click
/// that lands on a different byte with the bar open is the failure `ContentRect` exists to stop.
#[test]
fn a_click_below_the_bar_lands_on_the_byte_it_would_have() {
    let mut editor = editor("alpha beta gamma\nsecond line here\n");
    let point = (MARGIN + 30.0, MARGIN + 4.0);
    editor.click(point.0, point.1);
    let without = editor.document().selection().head();

    editor.apply_chord("f", false);
    frame(&mut editor);
    editor.click(point.0, point.1 + find::HEIGHT);
    assert_eq!(editor.document().selection().head(), without);
}

// ---- searching ----

#[test]
fn typing_in_the_box_finds_matches() {
    let mut editor = editor("alpha beta alpha\n");
    search(&mut editor, "alpha");
    assert_eq!(editor.find_bar().unwrap().matches().len(), 2);
}

#[test]
fn typing_in_the_box_does_not_edit_the_document() {
    let mut editor = editor("alpha\n");
    let before = editor.document().to_string();
    search(&mut editor, "zzz");
    assert_eq!(editor.document().to_string(), before);
}

/// The source is what is searched, so the markers are findable even though the reader never sees
/// them. A Markdown editor that could not find `**` would be hiding the file from its owner.
#[test]
fn concealed_syntax_is_searchable() {
    let mut editor = editor("a **bold** word\n");
    search(&mut editor, "**");
    assert_eq!(editor.find_bar().unwrap().matches().len(), 2);
}

#[test]
fn the_case_toggle_changes_what_is_found() {
    let mut editor = editor("Alpha alpha ALPHA\n");
    search(&mut editor, "alpha");
    assert_eq!(editor.find_bar().unwrap().matches().len(), 3);

    editor.toggle_find_case();
    frame(&mut editor);
    assert_eq!(editor.find_bar().unwrap().matches().len(), 1);
}

#[test]
fn editing_the_document_updates_the_matches() {
    let mut editor = editor("alpha\n");
    search(&mut editor, "alpha");
    assert_eq!(editor.find_bar().unwrap().matches().len(), 1);

    caret_at(&mut editor, 6);
    editor.insert_text("alpha\n");
    frame(&mut editor);
    assert_eq!(editor.find_bar().unwrap().matches().len(), 2);
}

// ---- navigating ----

#[test]
fn enter_selects_the_first_match() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    search(&mut editor, "beta");
    editor.apply_find_key(&Key::Next, false);
    assert_eq!(selected(&editor), (6, 10));
}

#[test]
fn enter_again_moves_on_rather_than_staying_put() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    search(&mut editor, "alpha");
    editor.apply_find_key(&Key::Next, false);
    assert_eq!(selected(&editor), (0, 5));
    editor.apply_find_key(&Key::Next, false);
    assert_eq!(selected(&editor), (11, 16));
}

#[test]
fn the_last_match_wraps_round_to_the_first() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    search(&mut editor, "alpha");
    editor.apply_find_key(&Key::Next, false);
    editor.apply_find_key(&Key::Next, false);
    editor.apply_find_key(&Key::Next, false);
    assert_eq!(selected(&editor), (0, 5));
}

#[test]
fn shift_enter_goes_backwards_and_wraps_the_other_way() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    search(&mut editor, "alpha");
    editor.apply_find_key(&Key::Previous, false);
    assert_eq!(selected(&editor), (11, 16), "backwards from the top wraps");
    editor.apply_find_key(&Key::Previous, false);
    assert_eq!(selected(&editor), (0, 5));
}

#[test]
fn navigating_with_nothing_found_does_nothing() {
    let mut editor = editor("alpha\n");
    caret_at(&mut editor, 2);
    search(&mut editor, "zzz");
    assert!(!editor.apply_find_key(&Key::Next, false));
    assert_eq!(selected(&editor), (2, 2));
}

/// A match a long way down materialises its chunk and scrolls to it, because navigating sets the
/// *selection* and the existing follow-caret reveal does the rest. Virtualization needed nothing new
/// for this, and this test is what says so.
#[test]
fn a_match_off_screen_is_scrolled_into_view() {
    let mut body = "filler paragraph\n\n".repeat(80);
    body.push_str("the needle is here\n");
    let mut editor = editor(&body);
    caret_at(&mut editor, 0);
    frame(&mut editor);
    assert_eq!(editor.scroll_offset(), 0.0);

    search(&mut editor, "needle");
    editor.apply_find_key(&Key::Next, false);
    frame(&mut editor);
    assert!(
        editor.scroll_offset() > 0.0,
        "the view should have followed the match, not stayed at the top"
    );
}

// ---- the count ----

#[test]
fn the_count_reads_from_the_selection() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    search(&mut editor, "alpha");
    editor.apply_find_key(&Key::Next, false);
    let selection = editor.document().selection().span();
    assert_eq!(
        editor.find_bar().unwrap().count_label(selection).as_deref(),
        Some("1 of 2")
    );
}

// ---- focus ----

#[test]
fn a_chord_still_reaches_the_document_from_the_search_box() {
    let mut editor = editor("alpha\n");
    search(&mut editor, "alpha");
    assert!(editor.find_focused());
    // Ctrl+A in the box selects the box, not the document — the one chord the field takes.
    editor.apply_find_key(&Key::Edit(Press::SelectAll), false);
    assert_eq!(
        editor.find_bar().unwrap().field.selected_text(),
        Some("alpha")
    );
}

#[test]
fn clicking_into_the_text_takes_focus_back_without_closing_the_bar() {
    let mut editor = editor("alpha beta gamma\n");
    search(&mut editor, "alpha");
    assert!(editor.find_focused());

    editor.click(MARGIN + 20.0, text_y(&editor));
    assert!(!editor.find_focused(), "the document should have it now");
    assert!(editor.find_open(), "and the bar should still be there");
}

#[test]
fn clicking_the_box_takes_focus_back_to_it() {
    let mut editor = editor("alpha beta gamma\n");
    search(&mut editor, "alpha");
    editor.click(MARGIN + 20.0, text_y(&editor));
    assert!(!editor.find_focused());

    let layout = editor.find_layout().expect("the bar was drawn").clone();
    editor.click(layout.field.0 + 4.0, bar_y(&editor));
    assert!(editor.find_focused());
}

// ---- clicking the buttons ----

#[test]
fn clicking_next_moves_the_selection() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    search(&mut editor, "alpha");
    let x = button_x(&editor, find::Button::Next);
    let y = bar_y(&editor);
    editor.click(x, y);
    assert_eq!(selected(&editor), (0, 5));
}

#[test]
fn clicking_close_closes_it() {
    let mut editor = editor("alpha\n");
    search(&mut editor, "alpha");
    let x = button_x(&editor, find::Button::Close);
    let y = bar_y(&editor);
    editor.click(x, y);
    assert!(!editor.find_open());
}

#[test]
fn clicking_the_case_button_toggles_it() {
    let mut editor = editor("Alpha alpha\n");
    search(&mut editor, "alpha");
    assert_eq!(editor.find_bar().unwrap().matches().len(), 2);
    let x = button_x(&editor, find::Button::Case);
    let y = bar_y(&editor);
    editor.click(x, y);
    frame(&mut editor);
    assert_eq!(editor.find_bar().unwrap().matches().len(), 1);
}

/// The gap between two buttons is still the bar. A click there reaching the document underneath is
/// the mistake every piece of chrome in this editor has made once.
#[test]
fn a_click_between_two_buttons_does_not_reach_the_document() {
    let mut editor = editor("alpha beta gamma\n");
    search(&mut editor, "alpha");
    caret_at(&mut editor, 3);
    let layout = editor.find_layout().expect("the bar was drawn").clone();
    // Between the count and the first button: bar background, and nothing else.
    let gap = layout.buttons[0].x - 2.0;
    let y = bar_y(&editor);
    assert!(editor.click(gap, y));
    assert_eq!(editor.document().selection().head().get(), 3);
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

// ---- more than one document ----

#[test]
fn switching_tab_searches_the_document_now_showing() {
    let dir = TempDir::new().unwrap();
    let one = dir.path().join("one.md");
    let two = dir.path().join("two.md");
    std::fs::write(&one, "alpha alpha alpha\n").unwrap();
    std::fs::write(&two, "alpha\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(one);
    workspace.add(two);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.show_tab(0);
    frame(&mut editor);

    search(&mut editor, "alpha");
    assert_eq!(editor.find_bar().unwrap().matches().len(), 3);

    editor.show_tab(1);
    frame(&mut editor);
    assert_eq!(
        editor.find_bar().unwrap().matches().len(),
        1,
        "the other document's offsets must not survive the switch"
    );
}

// ---- staleness stamps, and the identity they are keyed on (MDEDIT-105) ----

/// **The defect.** Every `(document, revision)` stamp used the tab *index*, and indices are reused:
/// closing tab 0 makes the old tab 1 into tab 0. Two documents nobody has typed in are both at
/// revision 0, so the stamp compared equal across the close and the survivor inherited the closed
/// tab's match list — offsets into a document that is no longer there, drawn over the one that is.
///
/// Nothing rescued it: the status-bar catch-up compares the same stamp, and a mouse-driven close
/// fires no idle tick.
#[test]
fn closing_a_tab_does_not_leave_its_matches_lit_on_the_next_one() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    // The needle is in the first document only, so a match on the second is provably inherited.
    std::fs::write(&first, "alpha alpha alpha\n").unwrap();
    std::fs::write(&second, "nothing to find here\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(first);
    workspace.add(second);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.workspace_mut().activate(0).unwrap();
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.apply_chord("f", false);
    editor.apply_find_key(&Key::Edit(Press::Text("alpha".to_owned())), false);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.find_bar().unwrap().matches().len(),
        3,
        "the first document should have three matches"
    );

    // Close the first tab. The second inherits index 0, and both are at revision 0.
    assert!(editor.apply_chord("w", false));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(
        editor.document().to_string(),
        "nothing to find here\n",
        "the second document should be showing"
    );
    assert!(
        editor.find_bar().unwrap().matches().is_empty(),
        "the closed tab's matches are lit over the surviving document: {:?}",
        editor.find_bar().unwrap().matches()
    );
}

/// The same class, with the word count: the survivor showed the closed tab's number until something
/// scheduled an idle tick, and closing with the mouse schedules none.
#[test]
fn closing_a_tab_does_not_leave_its_word_count_on_the_next_one() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "one two three four five six\n").unwrap();
    std::fs::write(&second, "just two\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(first);
    workspace.add(second);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.workspace_mut().activate(0).unwrap();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(editor.words(), Some(6));

    assert!(editor.apply_chord("w", false));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(
        editor.words(),
        Some(2),
        "the survivor is showing the closed tab's word count"
    );
}

/// **The box's caret draws where the caret is, including in the middle of a word.**
///
/// Found reviewing MDEDIT-141 rather than by using the editor, which is the uncomfortable part. The
/// box's caret is placed by finding the first glyph whose *source* offset is at or past it — the same
/// rule the document uses — and that story briefly had the text layer collapse every glyph of a run
/// with an empty source onto offset zero. Chrome passes exactly such a run, following what
/// `StyledRun` told it to pass.
///
/// **The middle is the whole point of this test, and the first version of it did not have one.** With
/// every glyph reporting zero, the caret at the *start* still finds the first glyph and the caret at
/// the *end* still falls back to the end of the text — both right, by luck. Only a position in
/// between distinguishes them, so this walks the caret one cell at a time and asks that it move each
/// time.
#[test]
fn the_boxs_caret_draws_where_the_caret_is() {
    let mut editor = editor("some body text\n");
    search(&mut editor, "hello");
    editor.apply_find_key(&Key::Edit(Press::Home), false);

    let mut seen = vec![caret_xs(&mut editor)];
    for step in 1..="hello".len() {
        editor.apply_find_key(&Key::Edit(Press::Right), false);
        let now = caret_xs(&mut editor);
        assert!(
            !seen.contains(&now),
            "after {step} presses of Right the box's caret is back somewhere it has already drawn"
        );
        seen.push(now);
    }
}
