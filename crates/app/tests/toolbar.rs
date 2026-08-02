//! The formatting bar: what it offers, what a click does, and what it costs per frame.
//!
//! Three claims are worth more than the rest, and they are the three every piece of chrome before
//! this one got wrong at least once:
//!
//! - a click in the bar never reaches the document, **including between two buttons**;
//! - a click below the bar still lands on the byte it would have without it — the drift test, which
//!   is the whole reason `ContentRect` exists;
//! - a still frame lays out no labels, which is the per-frame-layout trap the gutter, the status bar
//!   and the file pane each paid for in turn.
//!
//! The buttons themselves are checked by pressing them and reading the document, rather than by
//! asserting on quads: what a button *does* is the editor method it names, and that method is
//! already tested in `mdedit-document`. What is tested here is that the wiring reaches it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::toolbar::{self, Action};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// An editor whose one open file has this extension, so the language resolver sees it.
fn editor_named(dir: &TempDir, name: &str, contents: &str) -> Editor<Grid> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// Puts the caret at a byte, which is what lighting reads.
fn caret_at(editor: &mut Editor<Grid>, offset: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(offset)));
}

/// Selects `[start, end)`, deliberately **not** through `select_all` — which takes the trailing
/// newline with it, so `Ctrl+A` then bold gives `**prose\n**`. That is a real wart in the inline
/// toggles and it belongs to MDEDIT-50; a bar button reaches the same method either way, so these
/// select the words rather than let that wart decide what the toolbar looks like it did.
fn select(editor: &mut Editor<Grid>, start: usize, end: usize) {
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(start), ByteOffset::new(end)));
}

/// The middle of the button for `action`, in window coordinates.
fn centre_of(editor: &Editor<Grid>, action: Action) -> (f32, f32) {
    let button = editor
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == action)
        .unwrap_or_else(|| panic!("{action:?} should be on the bar"));
    (button.x + button.width / 2.0, toolbar::HEIGHT / 2.0)
}

// ---- what it offers ----

#[test]
fn a_markdown_document_gets_the_markdown_buttons() {
    let editor = editor("# Heading\n\nprose\n");
    let shown: Vec<Action> = editor
        .toolbar_buttons()
        .iter()
        .map(|button| button.action)
        .collect();
    assert_eq!(shown, toolbar::MARKDOWN, "at 900px all of them should fit");
}

/// A bar with nothing on it is worse than no bar: it takes height and offers nothing. JSON and YAML
/// have no formatting vocabulary of their own, so they get no bar at all.
#[test]
fn a_json_document_gets_no_bar_and_no_height() {
    let dir = TempDir::new().unwrap();
    // Both from a workspace, so both have a tab strip above them and the only difference between
    // the two top edges is the bar.
    let markdown = editor_named(&dir, "notes.md", "# Heading\n");
    let json = editor_named(&dir, "data.json", "{\"a\": 1}\n");

    assert!(json.toolbar_buttons().is_empty());
    assert_eq!(
        markdown.content_rect().top_edge() - json.content_rect().top_edge(),
        toolbar::HEIGHT,
        "a document with no bar should start exactly a bar higher up the window"
    );
}

/// Hiding it gives the height back, which is the point of hiding it.
#[test]
fn hiding_the_bar_gives_its_height_back() {
    let mut editor = editor("prose\n");
    let with = editor.content_rect().top_edge();

    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let without = editor.content_rect().top_edge();

    assert!(!editor.toolbar_shown());
    assert!(
        without < with,
        "hidden should reclaim the bar's height: {with} then {without}"
    );
    assert!(editor.toolbar_buttons().is_empty(), "and nothing to click");
}

#[test]
fn the_bar_comes_back() {
    let mut editor = editor("prose\n");
    let shown = editor.content_rect().top_edge();

    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(editor.toolbar_shown());
    assert_eq!(editor.content_rect().top_edge(), shown);
}

/// Narrow enough that the last buttons have nowhere to go. Stopping at the edge is the tab strip's
/// rule, and for the same reason: laying out a button nobody can see is work with no pixel.
#[test]
fn a_narrow_window_drops_the_buttons_that_do_not_fit() {
    let mut editor = Editor::new("prose\n", Grid::new());
    let _ = editor.build_frame(200.0, HEIGHT, 1.0);

    let shown = editor.toolbar_buttons();
    assert!(!shown.is_empty(), "there is room for some of them");
    assert!(shown.len() < toolbar::MARKDOWN.len());
    assert!(
        shown.last().unwrap().right() <= 200.0,
        "nothing may hang off the edge"
    );
}

// ---- what a click does ----

#[test]
fn every_markdown_button_does_something() {
    for action in toolbar::MARKDOWN {
        let mut editor = editor("some prose\n");
        select(&mut editor, 0, 10);
        let before = editor.document().to_string();

        let (x, y) = centre_of(&editor, *action);
        assert!(editor.click(x, y), "the bar should swallow the click");

        assert_ne!(
            editor.document().to_string(),
            before,
            "{action:?} should have changed the document"
        );
    }
}

#[test]
fn a_button_does_what_its_chord_does() {
    let cases: &[(Action, &str)] = &[
        (Action::Bold, "**prose**\n"),
        (Action::Italic, "*prose*\n"),
        (Action::Heading(2), "## prose\n"),
        (Action::Bullet, "- prose\n"),
        (Action::Quote, "> prose\n"),
    ];

    for (action, expected) in cases {
        let mut editor = editor("prose\n");
        select(&mut editor, 0, 5);
        let (x, y) = centre_of(&editor, *action);
        editor.click(x, y);
        assert_eq!(&editor.document().to_string(), expected, "{action:?}");
    }
}

/// The rule every piece of chrome carries: a click on it is not a click in the text.
#[test]
fn a_click_in_the_bar_never_reaches_the_document() {
    let mut editor = editor("hello world\n");
    let before = editor.document().selection();

    // Between two buttons, where `hit_test` finds nothing — the case a naive guard would let
    // through, putting the caret in the text under the bar.
    let buttons = editor.toolbar_buttons();
    let gap = f32::midpoint(buttons[3].right(), buttons[4].x);
    assert!(gap > buttons[3].right() && gap < buttons[4].x, "a real gap");

    assert!(
        editor.click(gap, toolbar::HEIGHT / 2.0),
        "the bar should swallow it"
    );
    assert_eq!(
        editor.document().selection(),
        before,
        "nothing should have moved"
    );
    assert_eq!(editor.document().to_string(), "hello world\n");
}

/// The drift test. A bar that pushes the document down without telling `ContentRect` puts every
/// click a bar-height out, which is the one failure mode chrome reliably introduces.
#[test]
fn a_click_below_the_bar_lands_on_the_right_byte() {
    let source = "hello world\n\nsecond line\n";

    let mut without = editor(source);
    without.toggle_toolbar();
    let _ = without.build_frame(WIDTH, HEIGHT, 1.0);
    let top = without.content_rect().to_window(0.0, 0.0, 0.0).1;
    let left = without.content_rect().left_edge();
    without.click_at(
        left + 4.0 * ADVANCE,
        top + LINE_HEIGHT * 2.0 + LINE_HEIGHT / 2.0,
    );
    let expected = without.document().selection().head();

    let mut with = editor(source);
    let top = with.content_rect().to_window(0.0, 0.0, 0.0).1;
    let left = with.content_rect().left_edge();
    with.click_at(
        left + 4.0 * ADVANCE,
        top + LINE_HEIGHT * 2.0 + LINE_HEIGHT / 2.0,
    );

    assert_eq!(
        with.document().selection().head(),
        expected,
        "the same column of the same line is the same byte, bar or no bar"
    );
}

// ---- lighting ----

#[test]
fn a_button_lights_for_what_the_caret_is_in() {
    let mut editor = editor("**bold** and plain\n");

    caret_at(&mut editor, 4);
    assert!(editor.is_lit(Action::Bold), "the caret is inside the bold");

    caret_at(&mut editor, 12);
    assert!(!editor.is_lit(Action::Bold), "and not once it is outside");

    // And pressing the lit button is what clears it.
    select(&mut editor, 2, 6);
    let (x, y) = centre_of(&editor, Action::Bold);
    editor.click(x, y);
    assert_eq!(editor.document().to_string(), "bold and plain\n");
}

#[test]
fn the_heading_button_lights_at_its_own_level_only() {
    let mut editor = editor("## a heading\n");
    caret_at(&mut editor, 5);

    assert!(editor.is_lit(Action::Heading(2)));
    assert!(!editor.is_lit(Action::Heading(1)));
    assert!(!editor.is_lit(Action::Heading(3)));
}

/// A setext heading carries no marker on its first line, so lighting it means asking the block list
/// rather than reading the line — the one case that separates the two.
#[test]
fn a_setext_heading_lights_too() {
    let mut editor = editor("A heading\n=========\n\nprose\n");
    caret_at(&mut editor, 3);
    assert!(editor.is_lit(Action::Heading(1)));
}

#[test]
fn the_list_buttons_tell_each_other_apart() {
    let mut bullet = editor("- a bullet\n");
    caret_at(&mut bullet, 5);
    assert!(bullet.is_lit(Action::Bullet));
    assert!(!bullet.is_lit(Action::Ordered));
    assert!(!bullet.is_lit(Action::Task), "a bullet is not a task");

    let mut task = editor("- [ ] a task\n");
    caret_at(&mut task, 8);
    assert!(task.is_lit(Action::Task));
    assert!(!task.is_lit(Action::Bullet), "a task is not a bullet");
}

#[test]
fn quoting_lights_alongside_the_list_it_quotes() {
    let mut editor = editor("> - quoted bullet\n");
    caret_at(&mut editor, 8);
    assert!(editor.is_lit(Action::Quote));
    assert!(
        editor.is_lit(Action::Bullet),
        "quoting is orthogonal, and so is its lighting"
    );
}

#[test]
fn nothing_lights_in_plain_prose() {
    let mut editor = editor("just some prose\n");
    caret_at(&mut editor, 5);
    for action in toolbar::MARKDOWN {
        assert!(!editor.is_lit(*action), "{action:?} should be dark");
    }
}

// ---- what it costs ----

/// The trap the gutter, the status bar and the file pane each hit in turn. A fourth would be
/// indefensible, so it is asserted rather than intended.
#[test]
fn a_still_frame_lays_out_no_toolbar_labels() {
    let mut editor = editor("# Heading\n\nprose\n");
    let after_first = editor.text_backend().layouts();

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let since = editor.text_backend().layouts() - after_first;

    assert!(
        since <= 2,
        "a second frame of an unchanged document should lay out the caret's chunk at most, \
         not {since}"
    );
}

/// Switching tabs swaps the bar live, rather than keeping whatever the last document had.
#[test]
fn switching_to_a_json_tab_takes_the_bar_away() {
    let dir = TempDir::new().unwrap();
    let markdown = dir.path().join("notes.md");
    let json = dir.path().join("data.json");
    std::fs::write(&markdown, "# Heading\n").unwrap();
    std::fs::write(&json, "{\"a\": 1}\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(markdown);
    workspace.add(json);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

    editor.show_tab(0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(!editor.toolbar_buttons().is_empty(), "Markdown has a bar");

    editor.show_tab(1);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(editor.toolbar_buttons().is_empty(), "JSON does not");

    editor.show_tab(0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(!editor.toolbar_buttons().is_empty(), "and back again");
}

// ---- geometry, on its own ----

#[test]
fn hit_testing_is_exclusive_at_the_edges() {
    let buttons = toolbar::layout(toolbar::MARKDOWN, &[10.0; 14], 900.0);
    let first = buttons[0];

    assert_eq!(
        toolbar::hit_test(&buttons, first.x, 1.0),
        Some(first.action),
        "the left edge belongs to the button"
    );
    assert_ne!(
        toolbar::hit_test(&buttons, first.right(), 1.0),
        Some(first.action),
        "its right edge belongs to whatever is next, not to it"
    );
}

#[test]
fn a_click_outside_the_bars_height_hits_nothing() {
    let buttons = toolbar::layout(toolbar::MARKDOWN, &[10.0; 14], 900.0);
    let x = buttons[0].x + 1.0;

    assert!(toolbar::hit_test(&buttons, x, -1.0).is_none());
    assert!(toolbar::hit_test(&buttons, x, toolbar::HEIGHT).is_none());
    assert!(toolbar::hit_test(&buttons, x, toolbar::HEIGHT / 2.0).is_some());
}
