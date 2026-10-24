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
use mdedit_app::view::Mode;
use mdedit_document::Selection;
use mdedit_offsets::{Affinity, ByteOffset};

use crate::stub;
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

// ---- deleting one marker must not strand the other (MDEDIT-129) ----

/// **The defect MDEDIT-118 shipped and MDEDIT-129 found.** Taking a concealed marker whole was
/// right; taking *half a construct* whole was not.
///
/// `see [the docs](https://example.com) here` shows as `see the docs here`. Backspace just before
/// the link text deleted the `[` alone — and the stranded `](https://example.com)` stopped being a
/// marker and became **visible text**. A URL nobody could see, appearing mid-sentence, from a
/// keystroke that looked like it should have taken a letter.
#[test]
fn backspace_before_a_links_text_removes_the_whole_link() {
    let mut editor = editor("see [the docs](https://example.com) here\n", 5);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "see the docs here\n");
}

/// And from the other end, where the stranded half was an orphan `[`.
#[test]
fn backspace_after_a_links_text_removes_the_whole_link() {
    let mut editor = editor("see [the docs](https://example.com) here\n", 14);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "see the docs here\n");
}

/// Forward delete has the same hole and the same fix.
#[test]
fn delete_at_a_links_opening_removes_the_whole_link() {
    let mut editor = editor("see [the docs](https://example.com) here\n", 4);
    assert!(editor.press_delete());
    assert_eq!(editor.document().to_string(), "see the docs here\n");
}

/// **One transaction**, so one Ctrl+Z puts the link back rather than half of it.
#[test]
fn one_undo_restores_a_deleted_link() {
    let source = "see [the docs](https://example.com) here\n";
    let mut editor = editor(source, 5);
    editor.press_backspace();
    assert!(editor.document_mut().undo());
    assert_eq!(editor.document().to_string(), source);
    assert!(
        !editor.document().can_undo(),
        "removing both markers must be one transaction"
    );
}

/// **The case that made the direction matter.** Pairing by "the nearest marker of the same kind"
/// alone matches the *next link's* opening bracket from this one's closing marker, and deleting both
/// takes the words between two different links with it.
#[test]
fn deleting_one_link_leaves_the_next_one_alone() {
    let mut editor = editor("[a](u) and [b](v)\n", 4);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "a and [b](v)\n");
}

#[test]
fn an_images_markers_go_together_too() {
    let mut editor = editor("![alt](pic.png)\n", 2);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "alt\n");
}

/// **A delimiter run is deliberately not paired**, and this is what says so rather than leaving it
/// to be discovered. `**` looks identical at both ends, so nothing in the bytes says which is which
/// and `**a** and **b**` would mis-pair exactly as two links would. It also lacks the failure that
/// motivated this: `bold**` is ugly, and it is still the characters that were already there.
#[test]
fn a_delimiter_run_still_goes_one_marker_at_a_time() {
    let mut editor = editor("**bold** text\n", 2);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "bold** text\n");
}

// ---- plain markup conceals nothing (MDEDIT-194) ----
//
// Everything above is preview's rule. `Ctrl+/` draws every marker, so there each byte is a place the
// caret may be and each key takes one. Until MDEDIT-194 the document's language-level answer was
// asked regardless of mode, and the escape hatch for editing a marker could not reach one from the
// keyboard.

/// The same editor in plain markup.
fn plain(source: &str, caret: usize) -> Editor<Grid> {
    let mut editor = editor(source, caret);
    editor.toggle_mode();
    assert_eq!(editor.mode(), Mode::Plain);
    editor
}

/// Every offset Right visits from `start`, until it stops moving. Bounded, because `press_right`
/// always reports true and a caret that stops moving would otherwise hang the test.
fn walk_right(editor: &mut Editor<Grid>) -> Vec<usize> {
    let mut seen = vec![caret_of(editor)];
    for _ in 0..64 {
        editor.press_right(false);
        let at = caret_of(editor);
        if seen.last() == Some(&at) {
            break;
        }
        seen.push(at);
    }
    seen
}

/// The ticket's scenario, Right half: from byte 0 of `# Title`, one byte a press.
#[test]
fn in_plain_markup_right_steps_into_a_heading_marker() {
    let mut editor = plain("# Title", 0);
    editor.press_right(false);
    assert_eq!(caret_of(&editor), 1, "onto the space, not over `# T`");
    editor.press_right(false);
    assert_eq!(caret_of(&editor), 2);
}

/// The ticket's scenario, Left half: from byte 2 Left reaches the marker rather than staying put.
#[test]
fn in_plain_markup_left_steps_back_through_a_heading_marker() {
    let mut editor = plain("# Title", 3);
    for expected in [2, 1, 0] {
        editor.press_left(false);
        assert_eq!(caret_of(&editor), expected);
    }
}

/// Every byte of every kind of marker is a stop, both ways, so no construct is special-cased.
#[test]
fn in_plain_markup_every_byte_is_a_caret_stop() {
    let source = "# H\n\n- [ ] **b** [l](u) `c`\n\n> q\n";
    let mut editor = plain(source, 0);
    let forward = walk_right(&mut editor);
    assert_eq!(forward, (0..=source.len()).collect::<Vec<_>>());

    let mut back = vec![caret_of(&editor)];
    for _ in 0..source.len() {
        editor.press_left(false);
        back.push(caret_of(&editor));
    }
    assert_eq!(back, (0..=source.len()).rev().collect::<Vec<_>>());
}

/// **The control**: the same keys from the same byte in preview still take a marker in one step,
/// so the test above sees the mode rather than a document that stopped concealing.
#[test]
fn preview_still_steps_over_the_marker_plain_steps_into() {
    let mut preview = editor("# Title", 0);
    let mut plain = plain("# Title", 0);
    preview.press_right(false);
    plain.press_right(false);
    assert_eq!((caret_of(&preview), caret_of(&plain)), (3, 1));
}

/// Back to preview, and the marker is one step again: the answer follows the mode as it changes.
#[test]
fn toggling_back_to_preview_conceals_the_marker_again() {
    let mut editor = plain("# Title", 0);
    editor.toggle_mode();
    editor.press_right(false);
    assert_eq!(caret_of(&editor), 3);
}

/// Shift extends a byte at a time too, so a part of a marker can be selected.
#[test]
fn in_plain_markup_shift_selects_part_of_a_marker() {
    let mut editor = plain("**bold**", 0);
    editor.press_right(true);
    let selection = editor.document().selection();
    assert_eq!((selection.anchor().get(), selection.head().get()), (0, 1));
}

/// Backspace after a drawn marker takes one byte of it, as it would any other character. Taking the
/// marker whole is preview's answer to bytes nobody can see, and in plain markup they can be seen.
#[test]
fn in_plain_markup_backspace_takes_one_byte_of_a_marker() {
    let mut editor = plain("# Title", 2);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "#Title");
    assert_eq!(caret_of(&editor), 1);
}

#[test]
fn in_plain_markup_delete_takes_one_byte_of_a_marker() {
    let mut editor = plain("**bold**", 0);
    assert!(editor.press_delete());
    assert_eq!(editor.document().to_string(), "*bold**");
}

/// Nor is a link's other half taken with it: `delete_marker`'s pairing exists because the stranded
/// half would appear from nowhere, and in plain markup it was never hidden.
#[test]
fn in_plain_markup_backspace_leaves_the_other_half_of_a_link() {
    let mut editor = plain("[text](https://example.com)", 1);
    assert!(editor.press_backspace());
    assert_eq!(editor.document().to_string(), "text](https://example.com)");
}

/// **Leaving plain markup moves a caret out of the marker it is in.** In preview byte 1 of `# Title`
/// draws just before the `T`, and typing there un-headed the line: `#x Title`.
#[test]
fn back_in_preview_a_caret_inside_a_marker_moves_past_it() {
    let mut editor = plain("# Title\n", 0);
    editor.press_right(false);
    editor.toggle_mode();
    assert_eq!(caret_of(&editor), 2);

    editor.insert_text("x");
    assert_eq!(
        editor.document().to_string(),
        "# xTitle\n",
        "still a heading"
    );
}

/// A selection's anchor moves too, so deleting it cannot leave half a marker behind as text.
#[test]
fn back_in_preview_a_selection_inside_a_marker_moves_both_ends() {
    let mut editor = plain("**bold** after", 0);
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(1), ByteOffset::new(7)));
    editor.toggle_mode();
    let selection = editor.document().selection();
    assert_eq!((selection.anchor().get(), selection.head().get()), (2, 8));
}

/// A caret already somewhere it may be is left alone, affinity and all: the end of a wrapped row is
/// drawn there only because of its affinity, and a toggle there and back must not move it a row.
#[test]
fn a_toggle_leaves_a_caret_outside_any_marker_alone() {
    let mut editor = editor(&"word ".repeat(60), 150);
    editor.build_frame(400.0, 600.0, 1.0);
    editor.press_end(false);
    let before = editor.document().selection();
    assert_eq!(before.affinity(), Affinity::Upstream, "setup");

    editor.toggle_mode();
    editor.toggle_mode();
    assert_eq!(editor.document().selection(), before);
}
