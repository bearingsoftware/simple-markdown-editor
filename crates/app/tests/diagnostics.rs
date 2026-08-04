//! MDEDIT-47 in the editor: when the check runs, when its answers are thrown away, and where the
//! menu item takes you.
//!
//! The checker itself is tested in `mdedit-json`. What is asserted here is the wiring, which is
//! where this project's defects actually live — the count has to appear on a timer and vanish on an
//! edit, and "go to the first problem" has to recompute before it moves the caret rather than acting
//! on offsets from before the last keystroke.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // Quad colours come from the same palette constants the renderer read, so they are bit-identical
    // rather than merely close.
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::Grid;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// Opens `name` holding `contents`, through the workspace the app uses, and draws one frame.
///
/// No snapshot store, and none is wanted: since MDEDIT-68 an editor is built with nowhere to keep
/// unsaved work, so a test that calls `on_idle` writes nothing anywhere. It did not used to be, and
/// these tests were how that was found.
fn open(dir: &TempDir, name: &str, contents: &str) -> Editor<Grid> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn json(dir: &TempDir, contents: &str) -> Editor<Grid> {
    open(dir, "data.json", contents)
}

// ---- when it runs ----

/// The motivating case, end to end: a trailing comma is counted once the typing stops, and fixing it
/// clears the count.
#[test]
fn a_broken_document_is_counted_after_the_idle_timer() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);
    assert_eq!(editor.diagnostics()[0].message, "trailing comma");
}

/// **Found by eye, not by a test.** The idle timer is set by a key event, so a document that was
/// only *opened* never reached it — a file with three errors in it showed none until you typed in
/// it, which is the state it is least useful in. The word count has the same gap and the same
/// once-per-document catch-up on the drawing path; this is that, and this test is the one that was
/// missing. Note it never calls `on_idle`.
#[test]
fn a_document_that_was_only_opened_is_still_checked() {
    let dir = TempDir::new().unwrap();
    let editor = json(&dir, "{\"a\": 1,}\n");

    assert_eq!(
        editor.diagnostic_count(),
        1,
        "one frame is enough; nobody has typed and the idle timer has never fired"
    );
}

/// And the catch-up must not become a per-frame check. Once a document has been looked at, drawing
/// it again does not re-run the walk — the idle timer owns it from then on, which is the whole
/// reason the check is not on this path.
#[test]
fn drawing_again_after_an_edit_does_not_recheck() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    assert_eq!(editor.diagnostic_count(), 1);

    editor.insert_text("  ");
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.diagnostic_count(),
        0,
        "a frame after an edit must not pay for the check; the count waits for the timer"
    );
}

#[test]
fn a_valid_document_counts_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 0);
}

/// A Markdown document never has a count, because Markdown has no notion of invalid. It opts out of
/// the whole feature by not implementing it.
#[test]
fn markdown_never_reports_anything() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "notes.md", "# A heading\n\nSome {\"unclosed prose.\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 0);
}

/// An empty document is not shouted at, which matters here rather than only in the checker: a new
/// file is empty, and an error on it before anything is typed would be the first thing the user saw.
#[test]
fn an_empty_document_has_no_count() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 0);
}

// ---- when its answers are thrown away ----

/// The stale-offset rule. An edit invalidates every span the last check produced, so the count goes
/// to nothing until the next idle rather than showing spans that point at bytes which have moved.
#[test]
fn an_edit_clears_the_count_until_the_next_idle() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);

    editor.insert_text("  ");
    assert_eq!(
        editor.diagnostic_count(),
        0,
        "an edit makes the last check's spans stale, so they must not be shown"
    );

    editor.on_idle();
    assert_eq!(
        editor.diagnostic_count(),
        1,
        "and the next idle repopulates"
    );
}

/// The `counted_for` lesson applied again: two documents neither of which has been typed in are both
/// at revision zero, so the revision alone cannot say which one a check belongs to.
#[test]
fn switching_tab_does_not_show_the_other_documents_problems() {
    let dir = TempDir::new().unwrap();
    let broken = dir.path().join("broken.json");
    let fine = dir.path().join("fine.json");
    std::fs::write(&broken, "{\"a\": 1,}\n").unwrap();
    std::fs::write(&fine, "{\"a\": 1}\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(broken);
    workspace.add(fine);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.show_tab(0);
    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);

    editor.show_tab(1);
    assert_eq!(
        editor.diagnostic_count(),
        0,
        "the first document's problems must not follow the tab switch"
    );
}

// ---- where the menu item goes ----

#[test]
fn validate_selects_the_first_problem() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::Validate));

    let selection = editor.document().selection();
    assert_eq!(selection.span().start().get(), 7, "the comma");
    assert_eq!(selection.span().end().get(), 8);
}

/// It recomputes before it moves. The count on the bar is at most one idle interval old, but a jump
/// that used offsets from before the last keystroke would put the caret somewhere innocent.
#[test]
fn validate_recomputes_before_it_jumps() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    editor.on_idle();

    // Push the comma along without ever running the idle timer.
    let start = mdedit_offsets::ByteOffset::ZERO;
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::new(start, start));
    editor.insert_text("    ");

    assert!(editor.apply_command(mdedit_menu_win::Command::Validate));
    assert_eq!(
        editor.document().selection().span().start().get(),
        11,
        "the comma has moved four bytes and the jump must have noticed"
    );
}

#[test]
fn validate_on_a_clean_document_does_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1}\n");

    assert!(!editor.apply_command(mdedit_menu_win::Command::Validate));
    assert!(editor.document().selection().is_collapsed());
}

/// A jump is the editor placing a selection, not the user sweeping one out — so it must not raise
/// the floating formatting bar, for the same reason a search match does not.
#[test]
fn the_jump_does_not_raise_the_floating_bar() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::Validate));
    assert!(!editor.settle_selection());
}

// ---- what it looks like (MDEDIT-76) ----

/// How many quads this frame draws in the diagnostic colour.
///
/// The squiggle is a row of them and the flagged gutter number is a few more, so the count is not a
/// number worth asserting on its own — what it answers is "is anything rose on screen", which is
/// exactly the question every test below asks.
fn rose(editor: &mut Editor<Grid>) -> usize {
    let rose = mdedit_app::view::Theme::default().span_error;
    let wanted = [rose.r, rose.g, rose.b, rose.a];
    editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted)
        .count()
}

#[test]
fn a_broken_document_is_drawn_on_not_just_counted() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);
    assert!(
        rose(&mut editor) > 0,
        "a counted diagnostic that draws nothing is the gap this story closed"
    );
}

/// The same rule the count follows, for the same reason: a squiggle under bytes that have moved is
/// worse than no squiggle, so an edit takes it away until the timer has looked again.
#[test]
fn an_edit_takes_the_squiggle_away_and_the_idle_timer_brings_it_back() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    editor.on_idle();
    assert!(rose(&mut editor) > 0);

    editor.insert_text("  ");
    assert_eq!(
        rose(&mut editor),
        0,
        "the spans are stale the instant the bytes move"
    );

    editor.on_idle();
    assert!(rose(&mut editor) > 0, "and come back when they are not");
}

#[test]
fn a_valid_document_is_drawn_on_not_at_all() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1}\n");
    editor.on_idle();
    assert_eq!(rose(&mut editor), 0, "silence, not a clean bill of health");
}

#[test]
fn an_empty_document_draws_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "");
    editor.on_idle();
    assert_eq!(rose(&mut editor), 0);
}

/// Markdown opts out of the whole feature by not implementing it, so there is nothing to draw.
#[test]
fn markdown_is_never_marked_up() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "notes.md", "# A heading\n\nSome {\"unclosed prose.\n");
    editor.on_idle();
    assert_eq!(rose(&mut editor), 0);
}

/// The gutter mark is the cue that does not depend on colour vision, so it has to be *more* than a
/// recolouring of the number: a `!` and a bold set of digits, which is more quads than the plain
/// number drew.
#[test]
fn the_flagged_line_number_is_marked_as_well_as_recoloured() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\n  \"a\": 1,\n}\n");
    editor.on_idle();

    let without_numbers = rose(&mut editor);
    editor.toggle_line_numbers();
    let with_numbers = rose(&mut editor);

    assert!(
        with_numbers > without_numbers,
        "turning the gutter on drew no extra rose quads, so the line is not marked: \
         {without_numbers} then {with_numbers}"
    );

    // And a clean document's numbers are not rose, so the mark means something.
    let mut clean = json(&dir, "{\n  \"a\": 1\n}\n");
    clean.toggle_line_numbers();
    clean.on_idle();
    assert_eq!(rose(&mut clean), 0);
}

/// Which rows carry a rose quad inside the gutter, as row indices from the top of the document.
fn flagged_rows(editor: &mut Editor<Grid>) -> Vec<usize> {
    let rose = mdedit_app::view::Theme::default().span_error;
    let wanted = [rose.r, rose.g, rose.b, rose.a];
    let gutter = editor.content_rect().left_edge();
    let top = editor
        .content_rect()
        .to_window(0.0, 0.0, mdedit_app::content::Offset::down(0.0))
        .1;
    let mut rows: Vec<usize> = editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted && quad.rect[0] < gutter)
        .map(|quad| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // Digits sit on the row's baseline, one pixel up from it in the stub's metrics.
            let row = ((quad.rect[1] + 1.0 - stub::ASCENT - top) / stub::LINE_HEIGHT)
                .round()
                .max(0.0) as usize;
            row
        })
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// The mark has to be on the line the error is on. An off-by-one here points the reader at innocent
/// bytes, which is worse than not marking at all.
#[test]
fn the_mark_lands_on_the_line_the_error_is_on() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\n  \"a\": 1,\n}\n");
    editor.toggle_line_numbers();
    editor.on_idle();

    assert_eq!(
        flagged_rows(&mut editor),
        vec![1],
        "the trailing comma is on the second line, which is row 1"
    );
}
