//! A task item's box, as the document sees it (MDEDIT-142).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::{Document, Selection};
use mdedit_offsets::ByteOffset;

/// The box starting at `offset`, as the text it covers.
fn box_at(source: &str, offset: usize) -> Option<String> {
    let doc = Document::from(source);
    doc.task_box_at(ByteOffset::new(offset))
        .map(|span| span.slice(source).to_owned())
}

#[test]
fn a_box_is_found_at_its_opening_bracket() {
    assert_eq!(box_at("- [ ] todo\n", 2).as_deref(), Some("[ ]"));
    assert_eq!(box_at("1. [X] done\n", 3).as_deref(), Some("[X]"));
    assert_eq!(box_at("> - [x] quoted\n", 4).as_deref(), Some("[x]"));
}

/// Only its first byte: a glyph drawn for a box addresses where the box starts, and nothing else
/// about the line is a box.
#[test]
fn nowhere_else_on_the_line_is_a_box() {
    let source = "- [ ] todo\n";
    for offset in (0..source.len()).filter(|offset| *offset != 2) {
        assert_eq!(box_at(source, offset), None, "at {offset}");
    }
}

/// Brackets that GFM does not read as a task are text, and so are not a box.
#[test]
fn brackets_that_are_not_a_task_are_not_a_box() {
    assert_eq!(box_at("- [y] maybe\n", 2), None);
    assert_eq!(box_at("[ ] not a list\n", 0), None);
}

// ---- ticking one ----

/// What `toggle_task` at `offset` leaves in the buffer, and whether it did anything.
fn toggled(source: &str, offset: usize) -> (bool, String) {
    let mut doc = Document::from(source);
    let acted = doc.toggle_task(ByteOffset::new(offset));
    (acted, doc.to_string())
}

/// The byte between the brackets and nothing else, which is the invariant that outranks everything.
#[test]
fn ticking_writes_one_byte() {
    assert_eq!(toggled("- [ ] a\n", 2), (true, "- [x] a\n".to_owned()));
    assert_eq!(toggled("- [x] a\n", 2), (true, "- [ ] a\n".to_owned()));
    assert_eq!(toggled("- [X] a\n", 2), (true, "- [ ] a\n".to_owned()));
    assert_eq!(
        toggled("1. [ ] a\r\n", 3),
        (true, "1. [x] a\r\n".to_owned())
    );
}

/// Anywhere that is not a box's start is not a box, and nothing is written.
#[test]
fn ticking_nothing_writes_nothing() {
    for (source, offset) in [("- [ ] a\n", 0), ("- [ ] a\n", 3), ("- [y] a\n", 2)] {
        assert_eq!(
            toggled(source, offset),
            (false, source.to_owned()),
            "{source:?} at {offset}"
        );
    }
}

/// The caret is put back, and one undo restores the byte and the caret together.
#[test]
fn ticking_keeps_the_caret_and_undoes_in_one_step() {
    let mut doc = Document::from("- [ ] a\nprose\n");
    let caret = Selection::caret(ByteOffset::new(10));
    doc.set_selection(caret);

    assert!(doc.toggle_task(ByteOffset::new(2)));
    assert_eq!(doc.selection(), caret);

    assert!(doc.undo());
    assert_eq!(doc.to_string(), "- [ ] a\nprose\n");
    assert_eq!(doc.selection(), caret);
}

// ---- ticking from the keyboard (MDEDIT-175) ----

/// A document on `source` with `selection` set, after one Ctrl+Enter.
fn pressed(source: &str, selection: Selection) -> (bool, Document) {
    let mut doc = Document::from(source);
    doc.set_selection(selection);
    let acted = doc.toggle_selected_tasks();
    (acted, doc)
}

/// A selection from `anchor` to `head`, as byte offsets.
fn span(anchor: usize, head: usize) -> Selection {
    Selection::new(ByteOffset::new(anchor), ByteOffset::new(head))
}

/// The acceptance criterion, taken literally: **anywhere** on the line — on the bullet, in the box,
/// in the text, at the end — flips the box, changes that one byte, and leaves the caret alone.
#[test]
fn a_caret_anywhere_on_a_tasks_line_flips_its_box() {
    for (source, expected) in [
        ("- [ ] a\nprose\n", "- [x] a\nprose\n"),
        ("1. [x] b\nprose\n", "1. [ ] b\nprose\n"),
        ("> - [ ] c\nprose\n", "> - [x] c\nprose\n"),
    ] {
        let line_end = source.find('\n').unwrap();
        for offset in 0..=line_end {
            let caret = Selection::caret(ByteOffset::new(offset));
            let (acted, doc) = pressed(source, caret);
            assert!(acted, "{source:?} at {offset}");
            assert_eq!(doc.to_string(), expected, "{source:?} at {offset}");
            assert_eq!(doc.selection(), caret, "{source:?} at {offset}");
        }
    }
}

/// The line is the caret's, not the one after it — which is the box a search that did not stop at
/// the line's end would find first.
#[test]
fn a_line_with_no_task_changes_nothing_even_above_one() {
    let source = "prose\n- [ ] a\n";
    for offset in 0..=5 {
        let (acted, doc) = pressed(source, Selection::caret(ByteOffset::new(offset)));
        assert!(!acted, "at {offset}");
        assert_eq!(doc.to_string(), source, "at {offset}");
    }
}

/// Nothing changed, so there is nothing to undo — a press on prose must not leave an empty step that
/// the next Ctrl+Z spends itself on.
#[test]
fn a_press_that_changes_nothing_records_nothing() {
    let (acted, mut doc) = pressed("just prose\n", Selection::caret(ByteOffset::new(3)));
    assert!(!acted);
    assert!(!doc.undo());
}

#[test]
fn one_undo_restores_the_byte_and_the_caret() {
    let source = "- [ ] a\n- [ ] b\n";
    let caret = Selection::caret(ByteOffset::new(12));
    let (_, mut doc) = pressed(source, caret);
    assert_eq!(doc.to_string(), "- [ ] a\n- [x] b\n");

    assert!(doc.undo());
    assert_eq!(doc.to_string(), source);
    assert_eq!(doc.selection(), caret);
}

/// The mixed-selection rule: any unticked box means tick them all. Toggling each would have given
/// `[ ] [x] [ ]`.
#[test]
fn a_mixed_selection_ticks_every_box() {
    let source = "- [x] a\n- [ ] b\n- [x] c\n";
    let (acted, doc) = pressed(source, span(0, source.len()));
    assert!(acted);
    assert_eq!(doc.to_string(), "- [x] a\n- [x] b\n- [x] c\n");
}

/// And only when every one is ticked does a press untick them, so a second press is the way back.
#[test]
fn a_selection_of_ticked_boxes_unticks_them_all() {
    let source = "- [x] a\n- [X] b\n";
    let (acted, doc) = pressed(source, span(0, source.len()));
    assert!(acted);
    assert_eq!(doc.to_string(), "- [ ] a\n- [ ] b\n");
}

/// A box already in the state asked for is not written. `[X]` stays capital — rewriting it to `[x]`
/// would be the editor re-spelling a byte nobody asked it to touch.
#[test]
fn ticking_leaves_an_already_ticked_box_byte_for_byte() {
    let source = "- [X] a\n- [ ] b\n";
    let (_, doc) = pressed(source, span(0, source.len()));
    assert_eq!(doc.to_string(), "- [X] a\n- [x] b\n");
}

/// However many boxes, one press is one step back.
#[test]
fn a_selection_undoes_in_one_step() {
    let source = "- [ ] a\n- [ ] b\n- [ ] c\n";
    let selection = span(2, 20);
    let (_, mut doc) = pressed(source, selection);
    assert_eq!(doc.to_string(), "- [x] a\n- [x] b\n- [x] c\n");
    assert_eq!(doc.selection(), selection);

    assert!(doc.undo());
    assert_eq!(doc.to_string(), source);
    assert_eq!(doc.selection(), selection);
}

/// Prose in the selection is passed over rather than made a task.
#[test]
fn a_selection_over_prose_and_tasks_ticks_only_the_tasks() {
    let source = "intro\n- [ ] a\nmiddle\n- [ ] b\n";
    let (acted, doc) = pressed(source, span(0, source.len()));
    assert!(acted);
    assert_eq!(doc.to_string(), "intro\n- [x] a\nmiddle\n- [x] b\n");
}

/// The rule `set_block` keeps: a selection that ends at the very start of a line has not taken
/// that line, or selecting one whole line by its newline would tick two.
#[test]
fn a_selection_ending_at_a_lines_start_leaves_that_line() {
    let source = "- [ ] a\n- [ ] b\n";
    let (_, doc) = pressed(source, span(0, 8));
    assert_eq!(doc.to_string(), "- [x] a\n- [ ] b\n");
}

/// The boxes are the parse's. `[ ]` in a fence is code, and code is not ticked.
#[test]
fn brackets_in_a_code_block_are_not_a_task() {
    let source = "```\n- [ ] a\n```\n";
    let (acted, doc) = pressed(source, span(0, source.len()));
    assert!(!acted);
    assert_eq!(doc.to_string(), source);
}
