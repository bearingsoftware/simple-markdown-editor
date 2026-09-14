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
