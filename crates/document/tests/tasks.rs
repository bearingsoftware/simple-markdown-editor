//! A task item's box, as the document sees it (MDEDIT-142).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
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
