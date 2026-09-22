//! The find bar as wired into the editor — the half `tests/find.rs` cannot see.
//!
//! `find.rs` proves the search is right and the geometry adds up. Everything here is about the
//! wiring, which is where this project's defects have actually been: a function that was tested and
//! a path to it that was not. So the claims are about routing and lifecycle —
//!
//! - a key goes to exactly one of the two things that take text, and it is the right one;
//! - a chord still reaches the document while the search box has the caret;
//! - the panel takes **no** height out of the document, and a click beside it lands on the byte it
//!   would have without it — the drift test that MDEDIT-52 needed and MDEDIT-39 is named after,
//!   which MDEDIT-177 turned from "a bar's height lower" into "exactly where it was";
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

/// Opens the bar and types `needle` into it, then draws — which is what refreshes the matches.
fn search(editor: &mut Editor<Grid>, needle: &str) {
    editor.apply_chord("f", false);
    editor.apply_find_key(&Key::Edit(Press::Text(needle.to_owned())), false);
    frame(editor);
}

/// The panel's layout, which is only there once a frame has been drawn with the bar open.
fn panel(editor: &Editor<Grid>) -> find::Layout {
    editor.find_layout().expect("the bar was drawn").clone()
}

/// A point on the panel, given in the panel's own coordinates (MDEDIT-177).
///
/// Asked of the layout rather than computed from the chrome stack: the panel floats over the
/// document at the top right now, so where it is depends on the content rect and on where the caret
/// is, and a test that worked either out itself would be asserting against its own copy of the
/// arithmetic.
fn on_panel(editor: &Editor<Grid>, x: f32, y: f32) -> (f32, f32) {
    let (left, top) = panel(editor).origin;
    (left + x, top + y)
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

// ---- the room it takes, which is none ----

/// MDEDIT-177's acceptance, on the geometry: the panel floats, so the document does not move when
/// it opens, when Replace adds a second row, or when it closes.
///
/// Was the opposite test until then — the bar was a band of chrome and this asserted that the
/// document started exactly a bar lower. The line you are reading moving under your eyes at the
/// moment you go looking for it is what that cost.
#[test]
fn opening_the_bar_does_not_move_the_document() {
    let mut editor = editor("alpha\n");
    let before = editor.content_rect().top_edge();

    editor.apply_chord("f", false);
    frame(&mut editor);
    assert_eq!(
        editor.content_rect().top_edge(),
        before,
        "the document should not have moved"
    );

    editor.open_replace();
    frame(&mut editor);
    assert_eq!(
        editor.content_rect().top_edge(),
        before,
        "nor when the panel grew a second row"
    );

    editor.close_find();
    frame(&mut editor);
    assert_eq!(editor.content_rect().top_edge(), before);
}

/// The same claim about what is drawn rather than about the rect: every row stays where it was.
///
/// The rect is what the rows are placed from, so this cannot fail while the test above passes —
/// which is the point of having it. It is the assertion a future bar that reached around
/// `ContentRect` would break, and `top_edge` alone would not notice.
#[test]
fn opening_the_bar_leaves_every_row_where_it_was() {
    let mut editor = editor("alpha beta gamma\nsecond line here\nthird line as well\n");
    let before = rows(&mut editor);
    assert!(before.len() >= 3, "three lines should draw three rows");

    editor.apply_chord("f", false);
    assert_eq!(rows(&mut editor), before, "opening Find moved a row");

    editor.open_replace();
    assert_eq!(rows(&mut editor), before, "Replace moved a row");

    editor.close_find();
    assert_eq!(rows(&mut editor), before, "closing moved a row");
}

/// The tops of the document's glyph rows, as `tests/lists.rs` reads them.
///
/// The panel's own glyphs are excluded by taking the text colour only — the fields and the buttons
/// draw in it too, but they draw at the panel's rows, which would otherwise appear and disappear
/// from this list as the bar opens and closes and say a row had moved when none had.
fn rows(editor: &mut Editor<Grid>) -> Vec<u32> {
    let text = Theme::default().text;
    let wanted = [text.r, text.g, text.b, text.a];
    // Drawn first and asked afterwards: the layout is what the frame just decided, and reading it
    // beforehand gets the *previous* frame's — which on the frame that opens the bar is `None`, so
    // the filter below would let every one of the panel's own glyphs through.
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let panel = editor.find_layout().cloned();
    let mut tops: Vec<u32> = quads
        .iter()
        .filter(|quad| quad.colour == wanted)
        .filter(|quad| {
            panel
                .as_ref()
                .is_none_or(|layout| !layout.contains(quad.rect[0], quad.rect[1]))
        })
        .map(|quad| quad.rect[1].to_bits())
        .collect();
    tops.sort_unstable();
    tops.dedup();
    tops
}

/// Top right of the document, which is where every editor puts it — and inside the content rather
/// than over the chrome above it or the scrollbar beside it.
#[test]
fn the_panel_floats_at_the_top_right_of_the_document() {
    let mut editor = editor("alpha\n");
    search(&mut editor, "alpha");
    let layout = panel(&editor);
    let content = editor.content_rect();
    let (left, top) = layout.origin;

    assert_eq!(top, content.top_edge(), "it starts where the document does");
    assert_eq!(
        left + layout.width,
        content.left_edge() + content.width(),
        "and ends where the document does, so it never covers the scrollbar",
    );
    assert!(
        left > content.left_edge(),
        "a panel that reached the left edge would be a band again: {left}",
    );
}

/// **The panel gets out of the caret's way** — the acceptance's "the current match is never hidden
/// behind it", since navigating to a match selects it and puts the caret on it.
///
/// A match at the end of the first line is exactly the case: top right is where the panel wants to
/// sit and where that match is.
#[test]
fn the_panel_moves_left_when_the_caret_is_behind_it() {
    let line = format!("{}needle\n", "x ".repeat(60));
    let mut editor = editor(&line);
    caret_at(&mut editor, 0);
    frame(&mut editor);
    let right = panel_origin_after(&mut editor, 0);

    let end = line.trim_end_matches('\n').len();
    let moved = panel_origin_after(&mut editor, end);
    assert!(
        moved.0 < right.0,
        "the panel should have moved left off the caret: {moved:?} against {right:?}",
    );
    assert_eq!(moved.1, right.1, "and not up or down");
    assert!(
        !panel(&editor).contains(caret_x(&mut editor), moved.1 + find::HEIGHT / 2.0),
        "the caret should be clear of it now",
    );
}

/// Where the panel ends up with the caret at `offset`, the bar open and a frame drawn.
fn panel_origin_after(editor: &mut Editor<Grid>, offset: usize) -> (f32, f32) {
    caret_at(editor, offset);
    if !editor.find_open() {
        editor.apply_chord("f", false);
    }
    frame(editor);
    panel(editor).origin
}

/// Where the document's caret was drawn, which is the rightmost of the two carets on screen when
/// the caret is at the end of a long line.
fn caret_x(editor: &mut Editor<Grid>) -> f32 {
    let xs = caret_xs(editor);
    f32::from_bits(*xs.last().expect("the document draws a caret"))
}

/// **A panel never narrower than its own buttons**, however little room the window leaves.
///
/// Found in the review of #183. The panel's width is clamped to the content's, and `layout` floors
/// each row's leftmost button at the padding — so in a window narrow enough, the last cells ran past
/// the panel's right edge. They were still *drawn*, since `draw_find_buttons` places them from the
/// panel's corner, and `Layout::contains` rejected them, so Close was visible and unpressable and
/// the click went into the document instead. It never arose while the bar was laid out in the
/// window's own width.
#[test]
fn a_narrow_window_keeps_every_button_on_the_panel() {
    let mut editor = editor("alpha\n");
    editor.apply_chord("f", false);
    // Narrower than the four buttons need, which is what the floor is for.
    let _ = editor.build_frame(120.0, HEIGHT, 1.0);

    let layout = panel(&editor);
    for rect in &layout.buttons {
        let (x, y) = (
            layout.origin.0 + rect.x + rect.width / 2.0,
            layout.origin.1 + rect.y + find::HEIGHT / 2.0,
        );
        assert!(
            layout.contains(x, y),
            "{:?} is off the panel: cell at {} of {} wide",
            rect.button,
            rect.x,
            layout.width,
        );
        assert_eq!(
            layout.hit_test(x, y),
            Some(rect.button),
            "{:?} answers no click",
            rect.button,
        );
    }
    assert!(editor.find_open());
    let close = layout
        .buttons
        .iter()
        .find(|rect| rect.button == find::Button::Close)
        .expect("find has a close button");
    editor.click(
        layout.origin.0 + close.x + close.width / 2.0,
        layout.origin.1 + find::HEIGHT / 2.0,
    );
    assert!(!editor.find_open(), "Close should still close it");
}

/// The drift test, inverted by MDEDIT-177. A panel that floats must leave the click where it was:
/// the *same* window point is the same byte with the bar open as without it.
#[test]
fn a_click_below_the_panel_lands_on_the_byte_it_would_have() {
    let mut editor = editor("alpha beta gamma\nsecond line here\n");
    let point = (MARGIN + 30.0, MARGIN + 4.0);
    editor.click(point.0, point.1);
    let without = editor.document().selection().head();

    editor.apply_chord("f", false);
    frame(&mut editor);
    assert!(
        !panel(&editor).contains(point.0, point.1),
        "the probe has to be off the panel, or it is not a click in the document",
    );
    editor.click(point.0, point.1);
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

    let field = panel(&editor).field;
    let (x, y) = on_panel(&editor, field.0 + 4.0, find::HEIGHT / 2.0);
    editor.click(x, y);
    assert!(editor.find_focused());
}

// ---- clicking the buttons ----

#[test]
fn clicking_next_moves_the_selection() {
    let mut editor = editor("alpha beta alpha\n");
    caret_at(&mut editor, 0);
    search(&mut editor, "alpha");
    let (x, y) = button_point(&editor, find::Button::Next);
    editor.click(x, y);
    assert_eq!(selected(&editor), (0, 5));
}

#[test]
fn clicking_close_closes_it() {
    let mut editor = editor("alpha\n");
    search(&mut editor, "alpha");
    let (x, y) = button_point(&editor, find::Button::Close);
    editor.click(x, y);
    assert!(!editor.find_open());
}

#[test]
fn clicking_the_case_button_toggles_it() {
    let mut editor = editor("Alpha alpha\n");
    search(&mut editor, "alpha");
    assert_eq!(editor.find_bar().unwrap().matches().len(), 2);
    let (x, y) = button_point(&editor, find::Button::Case);
    editor.click(x, y);
    frame(&mut editor);
    assert_eq!(editor.find_bar().unwrap().matches().len(), 1);
}

/// The gap between two buttons is still the panel. A click there reaching the document underneath
/// is the mistake every piece of chrome in this editor has made once — and since MDEDIT-177 what it
/// would reach is *directly* underneath rather than a band below, so it would land on the text the
/// panel is covering.
#[test]
fn a_click_between_two_buttons_does_not_reach_the_document() {
    let mut editor = editor("alpha beta gamma\n");
    search(&mut editor, "alpha");
    caret_at(&mut editor, 3);
    // Between the count and the first button: panel background, and nothing else.
    let gap = panel(&editor).buttons[0].x - 2.0;
    let (x, y) = on_panel(&editor, gap, find::HEIGHT / 2.0);
    assert!(editor.click(x, y));
    assert_eq!(editor.document().selection().head().get(), 3);
}

/// The middle of a button, in window coordinates.
fn button_point(editor: &Editor<Grid>, button: find::Button) -> (f32, f32) {
    let layout = panel(editor);
    let rect = layout
        .buttons
        .iter()
        .find(|rect| rect.button == button)
        .unwrap_or_else(|| panic!("{button:?} should be on the bar"));
    on_panel(
        editor,
        rect.x + rect.width / 2.0,
        rect.y + find::HEIGHT / 2.0,
    )
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
