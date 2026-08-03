//! The file pane: its geometry, its hit-testing, and its own scrolling.
//!
//! The claim that matters most is the last one — that a click in the *text* still lands on the right
//! byte once the pane has moved the document's left edge. That is the failure `ContentRect` exists
//! to prevent, and the pane is the second feature to move that edge after the line-number gutter.
//!
//! The pane is also the first piece of chrome with internal state: a click at a given y means a
//! different row depending on how far the list is scrolled. So the offset is a parameter to both
//! `layout` and `hit_test` rather than something either remembers — they cannot disagree about where
//! a row is if neither of them knows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::content::Offset;
use mdedit_app::editor::Editor;
use mdedit_app::pane::{self, Hit, PANE_WIDTH, ROW_HEIGHT};
use mdedit_app::view::Chrome;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 700.0;
const HEIGHT: f32 = MARGIN + 400.0;

/// A workspace of `count` files, with the first showing.
fn editor_with_files(dir: &TempDir, count: usize, contents: &str) -> Editor<Grid> {
    let mut workspace = Workspace::new();
    for n in 0..count {
        let path = dir.path().join(format!("file-{n}.md"));
        std::fs::write(&path, contents).unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor
}

// ---------------------------------------------------------------------------
// Geometry, without a window.
// ---------------------------------------------------------------------------

#[test]
fn rows_stack_downwards_one_height_apart() {
    let rows = pane::layout(3, 400.0, 0.0);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].y, 0.0);
    assert_eq!(rows[1].y, ROW_HEIGHT);
    assert_eq!(rows[2].y, ROW_HEIGHT * 2.0);
}

/// The same reason the strip stops at the edge: laying out forty labels to draw eight is the mistake
/// the renderer already avoids with document blocks.
#[test]
fn only_the_visible_rows_are_laid_out() {
    let rows = pane::layout(200, ROW_HEIGHT * 5.0, 0.0);
    assert!(
        rows.len() <= 6,
        "a 200-file workspace should not lay out 200 rows to fill five: {}",
        rows.len()
    );
}

#[test]
fn scrolling_moves_which_rows_are_laid_out() {
    let unscrolled = pane::layout(50, ROW_HEIGHT * 4.0, 0.0);
    let scrolled = pane::layout(50, ROW_HEIGHT * 4.0, ROW_HEIGHT * 10.0);

    assert_eq!(unscrolled[0].index, 0);
    assert_eq!(scrolled[0].index, 10, "ten rows down");
    assert_eq!(scrolled[0].y, 0.0, "and the tenth is now at the top");
}

/// A partly-scrolled list shows part of a row at the top, which is what makes scrolling look
/// continuous rather than jumping a row at a time.
#[test]
fn a_half_scrolled_row_still_lays_out() {
    let rows = pane::layout(50, ROW_HEIGHT * 4.0, ROW_HEIGHT * 2.5);
    assert_eq!(rows[0].index, 2);
    assert_eq!(rows[0].y, -ROW_HEIGHT / 2.0, "half of it is above the top");
}

#[test]
fn a_list_that_fits_cannot_scroll() {
    assert_eq!(pane::max_scroll(3, ROW_HEIGHT * 10.0), 0.0);
    assert!(pane::max_scroll(50, ROW_HEIGHT * 10.0) > 0.0);
}

// ---------------------------------------------------------------------------
// Hit-testing.
// ---------------------------------------------------------------------------

#[test]
fn a_click_selects_the_row_it_lands_in() {
    let rows = pane::layout(5, 400.0, 0.0);
    assert_eq!(pane::hit_test(&rows, 20.0, 1.0), Some(Hit::Select(0)));
    assert_eq!(
        pane::hit_test(&rows, 20.0, ROW_HEIGHT + 1.0),
        Some(Hit::Select(1))
    );
}

/// The close region sits inside the row, so it has to be checked first or the body swallows it.
#[test]
fn a_click_on_the_close_region_closes() {
    let rows = pane::layout(5, 400.0, 0.0);
    let (close_x, _) = rows[0].close_region();
    assert_eq!(
        pane::hit_test(&rows, close_x + 2.0, 1.0),
        Some(Hit::Close(0))
    );
}

#[test]
fn a_click_outside_the_pane_is_not_the_panes() {
    let rows = pane::layout(5, 400.0, 0.0);
    assert_eq!(pane::hit_test(&rows, PANE_WIDTH + 10.0, 1.0), None);
    assert_eq!(
        pane::hit_test(&rows, 20.0, ROW_HEIGHT * 50.0),
        None,
        "below the last row is nothing"
    );
}

/// The point of passing the offset to both: a click at a fixed y means a different file once the
/// list has moved under it.
#[test]
fn a_click_follows_the_scroll() {
    let unscrolled = pane::layout(50, 400.0, 0.0);
    let scrolled = pane::layout(50, 400.0, ROW_HEIGHT * 10.0);

    assert_eq!(pane::hit_test(&unscrolled, 20.0, 1.0), Some(Hit::Select(0)));
    assert_eq!(pane::hit_test(&scrolled, 20.0, 1.0), Some(Hit::Select(10)));
}

// ---------------------------------------------------------------------------
// Through the editor.
// ---------------------------------------------------------------------------

#[test]
fn the_two_presentations_are_exclusive() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with_files(&dir, 3, "# One\n\ntext\n");

    assert_eq!(editor.chrome(), Chrome::Strip, "the strip is the default");
    let strip_left = {
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
        editor.content_rect().left_edge()
    };

    editor.toggle_chrome();
    assert_eq!(editor.chrome(), Chrome::Pane);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let pane_left = editor.content_rect().left_edge();

    assert!(
        pane_left > strip_left,
        "the pane takes horizontal space the strip did not: {strip_left} then {pane_left}"
    );
    assert_eq!(
        pane_left - strip_left,
        PANE_WIDTH,
        "exactly the pane's width, and nothing else moved"
    );

    editor.toggle_chrome();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.content_rect().left_edge(),
        strip_left,
        "and back again"
    );
}

/// The document gets the room the other presentation is not using.
#[test]
fn the_document_is_taller_with_the_pane_than_with_the_strip() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with_files(&dir, 3, "# One\n\ntext\n");

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let with_strip = editor.content_rect().height();

    editor.toggle_chrome();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let with_pane = editor.content_rect().height();

    assert!(
        with_pane > with_strip,
        "the strip's height goes back to the document: {with_strip} then {with_pane}"
    );
}

/// The failure `ContentRect` exists to prevent, now that a second feature moves the left edge.
#[test]
fn a_click_in_the_text_lands_on_the_right_byte_with_the_pane_open() {
    let dir = TempDir::new().unwrap();
    let source = "hello world\n\nsecond line\n";

    let mut with_strip = editor_with_files(&dir, 1, source);
    let _ = with_strip.build_frame(WIDTH, HEIGHT, 1.0);
    let strip_left = with_strip.content_rect().left_edge();
    let strip_top = with_strip
        .content_rect()
        .to_window(0.0, 0.0, Offset::down(0.0))
        .1;
    with_strip.click_at(
        strip_left + 4.0 * ADVANCE,
        strip_top + LINE_HEIGHT * 2.0 + LINE_HEIGHT / 2.0,
    );
    let expected = with_strip.document().selection().head();

    let mut with_pane = editor_with_files(&dir, 1, source);
    with_pane.toggle_chrome();
    let _ = with_pane.build_frame(WIDTH, HEIGHT, 1.0);
    let pane_left = with_pane.content_rect().left_edge();
    let pane_top = with_pane
        .content_rect()
        .to_window(0.0, 0.0, Offset::down(0.0))
        .1;
    with_pane.click_at(
        pane_left + 4.0 * ADVANCE,
        pane_top + LINE_HEIGHT * 2.0 + LINE_HEIGHT / 2.0,
    );

    assert_eq!(
        with_pane.document().selection().head(),
        expected,
        "the same column of the same line is the same byte, whichever chrome is showing"
    );
}

#[test]
fn clicking_a_row_activates_that_file() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with_files(&dir, 3, "# One\n\ntext\n");
    editor.toggle_chrome();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.workspace().active(), Some(0));
    editor.click(20.0, mdedit_app::toolbar::HEIGHT + ROW_HEIGHT * 2.0 + 2.0);
    assert_eq!(
        editor.workspace().active(),
        Some(2),
        "the third row shows the third file"
    );
}

#[test]
fn the_pane_scrolls_independently_of_the_document() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with_files(&dir, 60, "# One\n\ntext\n");
    editor.toggle_chrome();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let document_before = editor.scroll_offset();
    assert!(editor.scroll_pane(ROW_HEIGHT * 5.0), "the list should move");
    assert_eq!(
        editor.scroll_offset(),
        document_before,
        "and the document should not"
    );

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    // Two pixels into the pane's *first row*, which starts below the formatting bar rather than at
    // the top of the window.
    editor.click(20.0, mdedit_app::toolbar::HEIGHT + 2.0);
    assert!(
        editor.workspace().active() != Some(0),
        "the top row is no longer the first file"
    );
}

#[test]
fn a_pane_with_few_files_does_not_scroll() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with_files(&dir, 2, "# One\n\ntext\n");
    editor.toggle_chrome();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(
        !editor.scroll_pane(1000.0),
        "two rows fit, so there is nothing to scroll"
    );
}

#[test]
fn the_strip_does_not_scroll_as_a_pane() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with_files(&dir, 60, "# One\n\ntext\n");
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        !editor.scroll_pane(100.0),
        "the strip is showing; the pane has no offset to move"
    );
}

/// An empty workspace has nothing to list, so the pane takes no room — as the strip already does.
#[test]
fn an_empty_workspace_shows_no_pane() {
    let mut editor = Editor::new("# Scratch\n", Grid::new());
    editor.toggle_chrome();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.content_rect().left_edge(),
        MARGIN,
        "no files, no pane"
    );
}

/// The trap the gutter and the status bar both hit: chrome is redrawn every frame, so anything it
/// lays out per frame is a text-engine call on the path a keystroke takes.
#[test]
fn a_still_frame_lays_out_no_pane_labels() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with_files(&dir, 40, "# One\n\ntext\n");
    editor.toggle_chrome();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let after_first = editor.text_backend().layouts();

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let since = editor.text_backend().layouts() - after_first;

    assert!(
        since <= 2,
        "a second frame of an unchanged 40-file pane should lay out the caret's chunk at most, \
         not {since}"
    );
}

/// The point of MDEDIT-60: the editor used to open on the strip every time, whatever you had
/// chosen. Toggling now records it, and a fresh editor reading the same file opens on it.
#[test]
fn the_chrome_choice_survives_a_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("config.json");

    let mut editor = Editor::new("hello\n", Grid::new());
    editor.with_settings(mdedit_app::config::Settings::at(file.clone()));
    editor.toggle_chrome();
    assert_eq!(editor.chrome(), Chrome::Pane);
    // Written on the idle timer, not on the keystroke — which is why this has to be asked for.
    editor.on_idle();

    let mut next = Editor::new("hello\n", Grid::new());
    next.with_settings(mdedit_app::config::Settings::at(file));
    assert_eq!(
        next.chrome(),
        Chrome::Pane,
        "the next run should open on whichever was chosen"
    );
}
