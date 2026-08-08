//! Getting the caret past syntax nobody can see (MDEDIT-118).
//!
//! With nothing revealed, a heading's `# ` draws nowhere — so offsets 0, 1 and 2 in `# Title` are
//! all the same x. Left and Right through them were a key that appeared to do nothing, twice, and
//! Backspace was a key that did nothing and then suddenly un-headed the line on the third press.
//!
//! **This is the half of MDEDIT-118 that is not about drawing.** Shipping the concealment on its own
//! would have given an editor whose headings could not be un-headed, which is why the ticket was
//! three changes rather than one.
//!
//! Driven through `press_left` / `press_right` / `press_backspace` / `press_delete` rather than
//! through the key handler, for the reason `apply_chord` gives: a winit `KeyEvent` carries a private
//! platform field and cannot be constructed by a test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::Grid;

fn editor(source: &str, caret: usize) -> Editor<Grid> {
    let mut editor = Editor::new(source, Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(caret)));
    editor
}

fn caret_of(editor: &Editor<Grid>) -> usize {
    editor.document().selection().head().get()
}

// ---- Right ----

/// The ticket's own scenario. `# Title`: Right from the line's start must land **after the T**, not
/// inside the marker and not merely at it — bytes 0 and 2 draw at the same place, so stopping at 2
/// would be a keypress with no visible effect.
#[test]
fn right_from_the_start_of_a_heading_lands_after_its_first_letter() {
    let mut editor = editor("# Title", 0);
    assert!(editor.press_right(false));
    assert_eq!(caret_of(&editor), 3, "should have crossed `# ` and the T");
}

#[test]
fn right_crosses_an_inline_delimiter_in_one_press() {
    //  *  *  b  o  l  d  *  *
    //  0  1  2  3  4  5  6  7  8
    let mut editor = editor("**bold**", 5);
    assert!(editor.press_right(false));
    assert_eq!(
        caret_of(&editor),
        8,
        "past the `d` and the whole closing delimiter, which draw at two distinct x and one"
    );
}

/// Two markers can abut with no content between them, which is why the crossing is a loop and not a
/// single step. `> ` and `` ` `` are separate segments here.
#[test]
fn right_crosses_two_abutting_markers_at_once() {
    let source = "> `code`\n";
    let mut editor = editor(source, 0);
    assert!(editor.press_right(false));
    assert_eq!(
        source.as_bytes()[caret_of(&editor) - 1],
        b'c',
        "should be past `> ` and the backtick, on the first letter's far side"
    );
}

#[test]
fn right_still_steps_one_character_through_ordinary_text() {
    let mut editor = editor("plain text", 0);
    for expected in 1..=4 {
        assert!(editor.press_right(false));
        assert_eq!(caret_of(&editor), expected);
    }
}

// ---- Left ----

/// The asymmetry worth stating: a marker's *start* is as unreachable as its middle, because it draws
/// at the same x as its end. So Left from a heading's first letter goes to **the end of the previous
/// line**, which is what a reader means by pressing Left at the start of a heading.
#[test]
fn left_from_a_headings_first_letter_leaves_the_line() {
    //  a  \n  #  ␣  T
    //  0  1   2  3  4
    let mut editor = editor("a\n# T", 4);
    assert!(editor.press_left(false));
    assert_eq!(
        caret_of(&editor),
        1,
        "the end of the line above, not byte 2"
    );
}

/// When the marker starts the document there is nowhere further left to go, so Left is a no-op —
/// the same answer Left gives at the start of any document, arrived at two bytes early.
#[test]
fn left_at_the_first_line_of_the_document_does_not_move() {
    let mut editor = editor("# Title", 2);
    editor.press_left(false);
    assert_eq!(caret_of(&editor), 2, "must not land inside the marker");
}

#[test]
fn left_and_right_are_inverses_across_a_marker() {
    let mut editor = editor("a\n# T", 4);
    editor.press_left(false);
    editor.press_right(false);
    assert_eq!(caret_of(&editor), 4, "back where it started");
}

/// **No caret position may rest inside a concealed marker**, whichever way it was reached. This is
/// the property the two directions are individual cases of, swept over a document with one of
/// everything.
#[test]
fn no_amount_of_arrowing_lands_inside_a_marker() {
    let source = "# Heading\n\n> a **bold** `x` quote\n\n- item\n";
    let doc = mdedit_document::Document::from(source);

    for from in 0..=source.len() {
        if !source.is_char_boundary(from) || doc.concealed_at(ByteOffset::new(from)).is_some() {
            continue;
        }
        for extend in [false, true] {
            let mut right = editor(source, from);
            right.press_right(extend);
            assert_eq!(
                doc.concealed_at(ByteOffset::new(caret_of(&right))),
                None,
                "Right from {from} landed inside a marker"
            );

            let mut left = editor(source, from);
            left.press_left(extend);
            assert_eq!(
                doc.concealed_at(ByteOffset::new(caret_of(&left))),
                None,
                "Left from {from} landed inside a marker"
            );
        }
    }
}

// ---- Backspace and Delete ----

/// The ticket's scenario, and the reason the concealment could not ship alone: without this, a
/// heading cannot be un-headed.
#[test]
fn backspace_at_the_text_removes_the_whole_marker() {
    let mut editor = editor("## Title", 3);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "Title");
    assert_eq!(caret_of(&editor), 0);
}

/// **One edit, so one undo.** Deleting three bytes as three transactions would need three presses of
/// Ctrl+Z to put back what one press of Backspace took away.
///
/// The `can_undo` at the end is what makes this say that: without it the assertion is satisfied by
/// any implementation whose *first* undo restores something, including one that took the marker off
/// a byte at a time.
#[test]
fn one_undo_restores_the_whole_marker() {
    let mut editor = editor("## Title", 3);
    editor.press_backspace();
    assert!(editor.document_mut().undo());
    assert_eq!(editor.document().to_string(), "## Title");
    assert!(
        !editor.document().can_undo(),
        "removing the marker must be one transaction, not three"
    );
}

#[test]
fn backspace_elsewhere_still_removes_one_character() {
    let mut editor = editor("## Title", 5);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "## Ttle");
}

#[test]
fn backspace_takes_a_whole_inline_delimiter() {
    let mut editor = editor("**bold**", 2);
    assert!(editor.press_backspace());
    assert_eq!(
        editor.document().to_string(),
        "bold**",
        "half a delimiter left behind would be two asterisks appearing from nowhere"
    );
}

#[test]
fn backspace_at_the_start_of_the_document_does_nothing() {
    let mut editor = editor("# Title", 0);
    assert!(!editor.press_backspace());
    assert_eq!(editor.document().to_string(), "# Title");
}

#[test]
fn delete_takes_a_whole_marker_forward() {
    //  a  \n  #  ␣  T
    let mut editor = editor("a\n# T", 2);
    assert!(editor.press_delete());
    assert_eq!(editor.document().to_string(), "a\nT");
}

#[test]
fn delete_elsewhere_still_removes_one_character() {
    let mut editor = editor("# Title", 3);
    assert!(editor.press_delete());
    assert_eq!(editor.document().to_string(), "# Ttle");
}

#[test]
fn delete_at_the_end_of_the_document_does_nothing() {
    let source = "# Title";
    let mut editor = editor(source, source.len());
    assert!(!editor.press_delete());
    assert_eq!(editor.document().to_string(), source);
}

/// A selection wins over both, which is what makes either one edit and one undo step.
#[test]
fn a_selection_is_what_gets_deleted() {
    for whole in [false, true] {
        let mut editor = Editor::new("# Title", Grid::new());
        editor
            .document_mut()
            .set_selection(Selection::new(ByteOffset::new(2), ByteOffset::new(7)));
        assert!(if whole {
            editor.press_delete()
        } else {
            editor.press_backspace()
        });
        assert_eq!(editor.document().to_string(), "# ");
    }
}

// A language that conceals nothing has nothing here to test: `concealed_at` answers `None` for every
// offset in it, which `crates/document/tests/reveal.rs` sweeps, and every path below then reduces to
// the plain boundary it always was.
