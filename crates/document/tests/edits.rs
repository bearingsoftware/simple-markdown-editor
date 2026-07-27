//! Editing behaviour: the primitives, rejected ranges, and the segmentation cache.
//!
//! The invariant tests in `edit_invariant.rs` prove the partition stays intact. These prove the
//! buffer does what was asked, and that a bad range changes nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::{Document, EditError};
use mdedit_markers::{Marker, SegmentKind};

struct Case {
    name: &'static str,
    source: &'static str,
    range: std::ops::Range<usize>,
    replacement: &'static str,
    expect: &'static str,
}

#[test]
fn replace_is_the_single_primitive() {
    let cases = &[
        Case {
            name: "insert at start",
            source: "world",
            range: 0..0,
            replacement: "hello ",
            expect: "hello world",
        },
        Case {
            name: "insert at end",
            source: "hello",
            range: 5..5,
            replacement: " world",
            expect: "hello world",
        },
        Case {
            name: "delete a span",
            source: "hello cruel world",
            range: 5..11,
            replacement: "",
            expect: "hello world",
        },
        Case {
            name: "replace a span",
            source: "hello cruel world",
            range: 6..11,
            replacement: "kind",
            expect: "hello kind world",
        },
        Case {
            name: "replace the whole buffer",
            source: "old",
            range: 0..3,
            replacement: "new",
            expect: "new",
        },
        Case {
            name: "empty edit is a no-op",
            source: "unchanged",
            range: 4..4,
            replacement: "",
            expect: "unchanged",
        },
        Case {
            name: "multibyte span",
            source: "a汉字b",
            range: 1..7,
            replacement: "x",
            expect: "axb",
        },
    ];

    for case in cases {
        let mut doc = Document::from(case.source);
        doc.replace(case.range.clone(), case.replacement)
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert_eq!(doc.to_string(), case.expect, "{}", case.name);
    }
}

#[test]
fn insert_and_delete_agree_with_replace() {
    let mut with_helpers = Document::from("hello world");
    with_helpers.insert(5, ",").unwrap();
    with_helpers.delete(0..1).unwrap();

    let mut with_replace = Document::from("hello world");
    with_replace.replace(5..5, ",").unwrap();
    with_replace.replace(0..1, "").unwrap();

    assert_eq!(with_helpers.to_string(), with_replace.to_string());
    assert_eq!(with_helpers.to_string(), "ello, world");
}

#[test]
// The reversed range is the point of one of these cases, not an accident.
#[allow(clippy::reversed_empty_ranges)]
fn a_rejected_edit_leaves_the_buffer_untouched() {
    let cases: &[(&str, std::ops::Range<usize>, &str)] = &[
        ("hello", 0..99, "end past the buffer"),
        ("hello", 99..99, "start past the buffer"),
        ("hello", 3..1, "reversed range"),
        ("汉", 1..1, "offset inside a multibyte character"),
        ("a汉b", 2..3, "both endpoints inside a character"),
    ];

    for (source, range, name) in cases {
        let mut doc = Document::from(*source);
        let before = doc.to_string();

        let result = doc.replace(range.clone(), "x");

        assert!(result.is_err(), "{name}: expected rejection");
        assert_eq!(doc.to_string(), before, "{name}: buffer was modified");
    }
}

#[test]
fn rejection_names_the_reason() {
    let mut doc = Document::from("汉");

    assert!(matches!(
        doc.replace(0..99, ""),
        Err(EditError::OutOfBounds { len: 3, .. })
    ));
    assert!(matches!(
        doc.replace(1..1, ""),
        Err(EditError::NotCharBoundary { offset: 1 })
    ));
}

#[test]
fn segmentation_reflects_the_current_buffer() {
    let mut doc = Document::from("plain text");
    assert!(
        doc.segments().iter().all(|s| !s.is_marker()),
        "no syntax yet"
    );

    doc.replace(0..0, "**").unwrap();
    doc.replace(12..12, "**").unwrap();
    assert_eq!(doc.to_string(), "**plain text**");

    let markers: Vec<_> = {
        let text = doc.to_string();
        doc.segments()
            .iter()
            .filter_map(|s| match s.kind() {
                SegmentKind::Marker(m) => Some((text[s.range()].to_owned(), m)),
                SegmentKind::Content => None,
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
fn several_edits_resegment_once() {
    // The cache is lazy so a transaction pays for one re-parse, not one per edit. Observable only
    // through the result being correct after a burst — timing would be flaky — so this guards
    // against the cache going stale, which is the failure that laziness risks.
    let mut doc = Document::from("a");
    for _ in 0..10 {
        let end = doc.len_bytes();
        doc.replace(end..end, "*b*").unwrap();
    }

    // The property that matters: an incrementally-edited document segments identically to one
    // parsed from scratch. That is precisely what a stale cache would break, and unlike a marker
    // count it does not depend on guessing how pulldown pairs ambiguous delimiters.
    let mut fresh = Document::from(doc.to_string().as_str());
    assert_eq!(doc.segments(), fresh.segments());
}

#[test]
fn empty_document_is_coherent() {
    let mut doc = Document::new();
    assert!(doc.is_empty());
    assert_eq!(doc.len_bytes(), 0);
    assert_eq!(doc.to_string(), "");
    assert!(doc.segments().is_empty());

    doc.insert(0, "# Title").unwrap();
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
