//! Editing behaviour: the primitives, rejected spans, and the segmentation cache.
//!
//! The invariant tests in `edit_invariant.rs` prove the partition stays intact. These prove the
//! buffer does what was asked, and that a bad span changes nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::{Document, EditError};
use mdedit_markers::{Marker, SegmentKind};
use mdedit_offsets::{ByteOffset, ByteSpan};

struct Case {
    name: &'static str,
    source: &'static str,
    span: ByteSpan,
    replacement: &'static str,
    expect: &'static str,
}

#[test]
fn replace_is_the_single_primitive() {
    let cases = &[
        Case {
            name: "insert at start",
            source: "world",
            span: ByteSpan::empty_at(ByteOffset::ZERO),
            replacement: "hello ",
            expect: "hello world",
        },
        Case {
            name: "insert at end",
            source: "hello",
            span: ByteSpan::empty_at(ByteOffset::new(5)),
            replacement: " world",
            expect: "hello world",
        },
        Case {
            name: "delete a span",
            source: "hello cruel world",
            span: ByteSpan::new(ByteOffset::new(5), ByteOffset::new(11)),
            replacement: "",
            expect: "hello world",
        },
        Case {
            name: "replace a span",
            source: "hello cruel world",
            span: ByteSpan::new(ByteOffset::new(6), ByteOffset::new(11)),
            replacement: "kind",
            expect: "hello kind world",
        },
        Case {
            name: "replace the whole buffer",
            source: "old",
            span: ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(3)),
            replacement: "new",
            expect: "new",
        },
        Case {
            name: "empty edit is a no-op",
            source: "unchanged",
            span: ByteSpan::empty_at(ByteOffset::new(4)),
            replacement: "",
            expect: "unchanged",
        },
        Case {
            name: "multibyte span",
            source: "a汉字b",
            span: ByteSpan::new(ByteOffset::new(1), ByteOffset::new(7)),
            replacement: "x",
            expect: "axb",
        },
    ];

    for case in cases {
        let mut doc = Document::from(case.source);
        doc.replace(case.span, case.replacement)
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert_eq!(doc.to_string(), case.expect, "{}", case.name);
    }
}

#[test]
fn insert_and_delete_agree_with_replace() {
    let mut with_helpers = Document::from("hello world");
    with_helpers.insert(ByteOffset::new(5), ",").unwrap();
    with_helpers.delete((0..1).into()).unwrap();

    let mut with_replace = Document::from("hello world");
    with_replace.replace((5..5).into(), ",").unwrap();
    with_replace.replace((0..1).into(), "").unwrap();

    assert_eq!(with_helpers.to_string(), with_replace.to_string());
    assert_eq!(with_helpers.to_string(), "ello, world");
}

#[test]
// The reversed span is the point of one of these cases, not an accident.
#[allow(clippy::reversed_empty_ranges)]
fn a_rejected_edit_leaves_the_buffer_untouched() {
    let cases: &[(&str, std::ops::Range<usize>, &str)] = &[
        ("hello", 0..99, "end past the buffer"),
        ("hello", 99..99, "start past the buffer"),
        ("hello", 3..1, "reversed span"),
        ("汉", 1..1, "offset inside a multibyte character"),
        ("a汉b", 2..3, "both endpoints inside a character"),
    ];

    for (source, range, name) in cases {
        let mut doc = Document::from(*source);
        let before = doc.to_string();

        let result = doc.replace(range.clone().into(), "x");

        assert!(result.is_err(), "{name}: expected rejection");
        assert_eq!(doc.to_string(), before, "{name}: buffer was modified");
    }
}

#[test]
fn rejection_names_the_reason() {
    let mut doc = Document::from("汉");

    assert!(matches!(
        doc.replace((0..99).into(), ""),
        Err(EditError::OutOfBounds { len: 3, .. })
    ));
    assert!(matches!(
        doc.replace((1..1).into(), ""),
        Err(EditError::NotCharBoundary { offset }) if offset.get() == 1
    ));
}

#[test]
fn segmentation_reflects_the_current_buffer() {
    let mut doc = Document::from("plain text");
    assert!(
        doc.segments().iter().all(|s| !s.is_marker()),
        "no syntax yet"
    );

    doc.insert(ByteOffset::ZERO, "**").unwrap();
    doc.insert(ByteOffset::new(12), "**").unwrap();
    assert_eq!(doc.to_string(), "**plain text**");

    let markers: Vec<_> = {
        let text = doc.to_string();
        doc.segments()
            .iter()
            .filter_map(|s| match s.kind() {
                SegmentKind::Marker(m) => Some((s.span().slice(&text).to_owned(), m)),
                // Content, and any kind a language other than Markdown produces.
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        markers,
        vec![
            ("**".to_owned(), Marker::Strong),
            ("**".to_owned(), Marker::Strong)
        ]
    );
}

#[test]
fn an_edited_document_matches_a_fresh_parse() {
    let mut doc = Document::from("a");
    for _ in 0..10 {
        let end = ByteOffset::new(doc.len_bytes());
        doc.insert(end, "*b*").unwrap();
    }

    // The property that matters: an incrementally-edited document segments identically to one
    // parsed from scratch. That is precisely what a stale cache would break, and unlike a marker
    // count it does not depend on guessing how pulldown pairs ambiguous delimiters.
    let fresh = Document::from(doc.to_string().as_str());
    assert_eq!(doc.segments(), fresh.segments());
}

#[test]
fn empty_document_is_coherent() {
    let mut doc = Document::new();
    assert!(doc.is_empty());
    assert_eq!(doc.len_bytes(), 0);
    assert_eq!(doc.to_string(), "");
    assert!(doc.segments().is_empty());

    doc.insert(ByteOffset::ZERO, "# Title").unwrap();
    assert!(!doc.is_empty());
    assert!(
        doc.segments()
            .iter()
            .any(mdedit_markers::Segment::is_marker)
    );
}

#[test]
fn parses_from_a_string() {
    let doc: Document = "**bold**".parse().unwrap_or_else(|e| match e {});
    assert_eq!(doc.to_string(), "**bold**");
}

/// MDEDIT-54's cheap "has this changed" signal: crash recovery asks it on a timer, and walking the
/// buffer or holding a copy to compare against would defeat the point of asking cheaply.
#[test]
fn the_revision_moves_only_when_the_bytes_do() {
    let mut doc = Document::from("hello\n");
    let start = doc.revision();

    doc.set_selection(mdedit_document::Selection::caret(ByteOffset::new(1)));
    assert_eq!(doc.revision(), start, "moving the caret changes no bytes");

    doc.insert(ByteOffset::new(0), "x").unwrap();
    let after_insert = doc.revision();
    assert_ne!(after_insert, start, "an insert does");

    doc.undo();
    assert_ne!(doc.revision(), after_insert, "and so does undoing it");

    // Same bytes as the start, but a later revision: this is a change counter, not a content hash,
    // and a caller that needs to know "are these the same bytes" must compare the bytes.
    assert_eq!(doc.to_string(), "hello\n");
}
