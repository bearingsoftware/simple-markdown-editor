//! The correctness gate for scoped re-parsing (MDEDIT-18).
//!
//! A keystroke re-parses only the region around the edit. That is only sound if the result agrees
//! with what a whole-document parse would have said — so this asserts exactly that, over generated
//! documents and generated edits, and separately pins the two cases where it deliberately does not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;

use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};
use proptest::prelude::*;

/// Big enough that `Document` takes the scoped path rather than re-parsing the lot.
///
/// The threshold is an implementation detail, so this pads generously rather than sitting on it.
const PADDING_BLOCKS: usize = 400;

/// Filler that is unambiguous Markdown: separate top-level blocks, nothing left open.
fn padding(from: usize, count: usize) -> String {
    let mut text = String::new();
    for n in from..from + count {
        writeln!(
            text,
            "## Section {n}\n\nParagraph {n} with **bold** and a `span` in it.\n"
        )
        .expect("a String never fails to write");
    }
    text
}

/// A document with `body` buried in the middle of enough text to trigger scoped parsing.
fn buried(body: &str) -> (String, usize) {
    let head = padding(0, PADDING_BLOCKS);
    let at = head.len();
    let mut text = head;
    text.push_str(body);
    text.push_str(&padding(PADDING_BLOCKS, PADDING_BLOCKS));
    (text, at)
}

/// What a whole-document parse says, for comparison.
fn whole(text: &str) -> Vec<(ByteSpan, bool)> {
    mdedit_markers::segment(text)
        .iter()
        .map(|segment| (segment.span(), segment.is_marker()))
        .collect()
}

fn actual(doc: &Document) -> Vec<(ByteSpan, bool)> {
    doc.segments()
        .iter()
        .map(|segment| (segment.span(), segment.is_marker()))
        .collect()
}

fn assert_agrees(doc: &Document, context: &str) {
    let text = doc.to_string();
    let (mine, theirs) = (actual(doc), whole(&text));
    if mine == theirs {
        return;
    }

    // Printing two lists of forty thousand spans helps nobody. Find where they part company.
    let at = mine
        .iter()
        .zip(&theirs)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| mine.len().min(theirs.len()));
    let show = |list: &[(ByteSpan, bool)]| -> String {
        let mut out = String::new();
        for (span, marker) in list.iter().skip(at.saturating_sub(2)).take(5) {
            write!(
                out,
                "\n    {span} {} {:?}",
                if *marker { "marker " } else { "content" },
                span.slice(&text)
            )
            .expect("a String never fails to write");
        }
        out
    };
    panic!(
        "{context}: the scoped parse disagrees with a whole-document one at segment {at}\
         \n  scoped:{}\n  whole: {}",
        show(&mine),
        show(&theirs)
    );
}

/// The invariant that outranks agreement: whatever the segments say, they still tile the buffer.
fn assert_tiles(doc: &Document, context: &str) {
    let text = doc.to_string();
    let rebuilt: String = doc
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, text, "{context}: the segments lost bytes");
}

#[test]
fn a_large_document_takes_the_scoped_path() {
    let (text, at) = buried("A paragraph to type into.\n\n");
    let mut doc = Document::from(text.as_str());
    assert!(!doc.is_scoped(), "a fresh document is fully parsed");

    doc.insert(ByteOffset::new(at + 2), "x").unwrap();
    assert!(doc.is_scoped(), "the edit should have re-parsed a region");
    assert_agrees(&doc, "after one insert");
}

#[test]
fn a_small_document_is_parsed_whole_as_it_always_was() {
    let mut doc = Document::from("# Title\n\nShort.\n");
    doc.insert(ByteOffset::new(10), "x").unwrap();
    assert!(
        !doc.is_scoped(),
        "below the threshold nothing changes about how a document is parsed"
    );
}

#[test]
fn typing_a_delimiter_is_classified_the_same_as_a_whole_parse_would() {
    let (text, at) = buried("Plain words here.\n\n");
    let mut doc = Document::from(text.as_str());

    // Type `**` around a word, one character at a time, as somebody actually would.
    for (offset, character) in [(at, "*"), (at + 1, "*"), (at + 7, "*"), (at + 8, "*")] {
        doc.insert(ByteOffset::new(offset), character).unwrap();
        assert_tiles(&doc, "mid-emphasis");
    }
    assert_agrees(&doc, "after typing **Plain**");
}

#[test]
fn deleting_across_a_block_boundary_still_agrees() {
    let (text, at) = buried("First para.\n\nSecond para.\n\n");
    let mut doc = Document::from(text.as_str());

    // Remove the blank line between them, joining two blocks into one.
    doc.delete(ByteSpan::new(
        ByteOffset::new(at + 11),
        ByteOffset::new(at + 13),
    ))
    .unwrap();
    assert_agrees(&doc, "after joining two paragraphs");
}

#[test]
fn editing_inside_a_fence_still_agrees() {
    let (text, at) = buried("```rust\nfn a() {}\nfn b() {}\n```\n\n");
    let mut doc = Document::from(text.as_str());

    doc.insert(ByteOffset::new(at + 12), "b").unwrap();
    assert_agrees(&doc, "after typing inside a fence");
}

#[test]
fn an_edit_at_the_very_start_agrees() {
    let (text, _) = buried("body\n\n");
    let mut doc = Document::from(text.as_str());
    doc.insert(ByteOffset::new(0), "#").unwrap();
    assert_agrees(&doc, "after an edit at offset zero");
}

#[test]
fn an_edit_at_the_very_end_agrees() {
    let (text, _) = buried("body\n\n");
    let mut doc = Document::from(text.as_str());
    let end = ByteOffset::new(doc.len_bytes());
    doc.insert(end, "trailing").unwrap();
    assert_agrees(&doc, "after an edit at the end");
}

/// Undo and redo re-parse whole, so they are the reset that stops divergence accumulating.
#[test]
fn undo_and_redo_leave_a_whole_parse_behind() {
    let (text, at) = buried("body\n\n");
    let mut doc = Document::from(text.as_str());
    doc.insert(ByteOffset::new(at), "z").unwrap();
    assert!(doc.is_scoped());

    doc.undo();
    assert!(!doc.is_scoped(), "undo re-parses the whole buffer");
    assert_agrees(&doc, "after undo");

    doc.redo();
    assert_agrees(&doc, "after redo");
}

// ---------------------------------------------------------------------------
// Where it deliberately does not agree, and what fixes it.
// ---------------------------------------------------------------------------

/// Typing a fence opens a construct that swallows the rest of the document. A region parse cannot
/// see that, by definition — it is looking at the region. This is the case the reconcile exists for,
/// and it is pinned rather than hidden.
#[test]
fn opening_a_fence_diverges_until_reconciled() {
    let (text, at) = buried("body\n\n");
    let mut doc = Document::from(text.as_str());

    doc.insert(ByteOffset::new(at), "```\n").unwrap();
    let scoped = actual(&doc);
    assert_tiles(&doc, "with an unclosed fence");

    assert!(
        doc.reconcile(),
        "the fence changed what the rest of the document is"
    );
    assert!(!doc.is_scoped());
    assert_agrees(&doc, "after reconciling");
    assert_ne!(
        scoped,
        actual(&doc),
        "if these matched, this test would be asserting nothing"
    );
}

#[test]
fn reconciling_an_unscoped_document_does_nothing() {
    let mut doc = Document::from("# Title\n");
    assert!(!doc.reconcile());
}

/// The common case: a scoped parse that was already right leaves nothing to correct, so a caller
/// holding a cache does not have to throw it away every time the typing stops.
#[test]
fn reconciling_an_ordinary_edit_finds_nothing_to_correct() {
    let (text, at) = buried("A body paragraph.\n\n");
    let mut doc = Document::from(text.as_str());
    doc.insert(ByteOffset::new(at + 2), "x").unwrap();
    assert!(doc.is_scoped());
    assert!(
        !doc.reconcile(),
        "the scoped answer was already the right one"
    );
}

// ---------------------------------------------------------------------------
// The property test: arbitrary edits, arbitrary places.
// ---------------------------------------------------------------------------

/// Snaps an offset onto a character boundary at or below it.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Whatever is typed, wherever it is typed, the segments still tile the buffer. This is the
    /// crate's gate, and scoped parsing must not be able to weaken it.
    #[test]
    fn scoped_edits_still_tile_the_buffer(
        edits in prop::collection::vec(
            (0usize..40_000, 0usize..8, "[a-z *#`>\\-\\[\\]()\n]{0,6}"),
            1..12,
        ),
    ) {
        let (text, _) = buried("A body paragraph.\n\n");
        let mut doc = Document::from(text.as_str());

        for (at, len, replacement) in edits {
            let current = doc.to_string();
            let start = boundary(&current, at);
            let end = boundary(&current, start + len);
            doc.replace(
                ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
                &replacement,
            )
            .unwrap();
            assert_tiles(&doc, "during a generated edit sequence");
        }
    }

    /// And for an edit that does not open a block across the region's edge — which is what typing
    /// into prose is — the scoped answer *is* the whole-document answer.
    #[test]
    fn typing_prose_agrees_with_a_whole_parse(
        insertions in prop::collection::vec("[a-z ]{1,4}", 1..8),
    ) {
        let (text, at) = buried("A body paragraph to type into.\n\n");
        let mut doc = Document::from(text.as_str());

        let mut offset = at + 2;
        for insertion in insertions {
            doc.insert(ByteOffset::new(offset), &insertion).unwrap();
            offset += insertion.len();
            assert_agrees(&doc, "while typing prose");
        }
    }
}
