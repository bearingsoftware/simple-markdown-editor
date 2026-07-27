//! The gate, extended to a buffer that is being edited.
//!
//! MDEDIT-3 proved the segmentation tiles a *fixed* string. That is the weaker half of what the
//! architecture needs. This proves it still tiles after arbitrary sequences of insert, delete and
//! replace — an editor whose invariant only holds at rest is an editor that corrupts documents while
//! someone types into it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
use proptest::prelude::*;

/// Asserts everything the partition promises, against the buffer's current contents.
fn assert_partitions(doc: &mut Document, context: &str) {
    let text = doc.to_string();
    let segments = doc.segments();

    let mut expected_start = 0;
    for (i, seg) in segments.iter().enumerate() {
        let range = seg.range();
        assert!(
            range.start < range.end,
            "{context}: segment {i} is empty at {range:?}"
        );
        assert_eq!(
            range.start, expected_start,
            "{context}: segment {i} starts at {} but the previous ended at {expected_start}",
            range.start
        );
        assert!(
            text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
            "{context}: segment {i} range {range:?} splits a character"
        );
        expected_start = range.end;
    }
    assert_eq!(
        expected_start,
        text.len(),
        "{context}: segments cover {expected_start} of {} bytes",
        text.len()
    );

    let rebuilt: String = segments.iter().map(|s| &text[s.range()]).collect();
    assert_eq!(rebuilt, text, "{context}: source not reproduced");
}

/// An edit expressed independently of any particular buffer, so it stays applicable as the buffer
/// changes underneath the sequence.
#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    len: usize,
    replacement: String,
}

/// Snaps an arbitrary offset down to a character boundary within the buffer.
///
/// The generator cannot know the buffer's boundaries, so the test resolves them here. Rejecting
/// mid-character offsets instead would throw away most generated cases.
fn snap(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Fragments biased toward Markdown syntax, so edits actually make and break constructs rather than
/// shuffling prose around.
const FRAGMENTS: &[&str] = &[
    "", "a", "word ", "\n", "\n\n", "*", "**", "_", "`", "```", "~~", "#", "## ", ">", "> ", "- ",
    "1. ", "[", "](url)", "![", "\\", "é", "汉", "🎉", "  \n",
];

fn edit_strategy() -> impl Strategy<Value = Edit> {
    (
        any::<u16>(),
        any::<u16>(),
        proptest::collection::vec(0..FRAGMENTS.len(), 0..4),
    )
        .prop_map(|(start, len, parts)| Edit {
            start: start as usize,
            len: len as usize,
            replacement: parts.into_iter().map(|i| FRAGMENTS[i]).collect(),
        })
}

fn document_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(0..FRAGMENTS.len(), 0..24)
        .prop_map(|parts| parts.into_iter().map(|i| FRAGMENTS[i]).collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The gate. Every intermediate state of an arbitrary edit sequence still partitions.
    #[test]
    fn partitions_after_every_edit_in_a_sequence(
        initial in document_strategy(),
        edits in proptest::collection::vec(edit_strategy(), 0..24),
    ) {
        let mut doc = Document::from(initial.as_str());
        assert_partitions(&mut doc, "initial");

        for (i, edit) in edits.iter().enumerate() {
            let text = doc.to_string();
            let start = snap(&text, edit.start);
            let end = snap(&text, start.saturating_add(edit.len));

            doc.replace(start..end, &edit.replacement)
                .expect("range was snapped to a valid char boundary within the buffer");

            assert_partitions(&mut doc, &format!("after edit {i}: {edit:?}"));

            // An edited document must segment identically to one parsed from scratch. This is what
            // a stale cache breaks, and it is stronger than the partition check alone: a cache that
            // was correct-but-outdated could still tile perfectly while describing older text.
            let mut fresh = Document::from(doc.to_string().as_str());
            prop_assert_eq!(doc.segments(), fresh.segments(), "cache diverged after edit {}", i);
        }
    }

    /// The buffer itself must survive editing exactly as a `String` would. If this diverges, the
    /// partition being intact says nothing — it would be describing the wrong bytes.
    #[test]
    fn buffer_matches_the_equivalent_string_edits(
        initial in document_strategy(),
        edits in proptest::collection::vec(edit_strategy(), 0..24),
    ) {
        let mut doc = Document::from(initial.as_str());
        let mut reference = initial;

        for edit in edits {
            let start = snap(&reference, edit.start);
            let end = snap(&reference, start.saturating_add(edit.len));

            doc.replace(start..end, &edit.replacement).unwrap();
            reference.replace_range(start..end, &edit.replacement);

            prop_assert_eq!(doc.to_string(), reference.clone());
        }
    }
}

#[test]
fn partitions_after_edits_that_break_constructs() {
    // Hand-picked because a generator rarely lands an edit exactly on a delimiter, and that is the
    // case most likely to desynchronise the segmentation from the text.
    let cases: &[(&str, std::ops::Range<usize>, &str, &str)] = &[
        (
            "**bold**",
            0..1,
            "",
            "remove one asterisk of an opening pair",
        ),
        ("**bold**", 6..8, "", "remove the closing pair"),
        ("**bold**", 2..2, "*", "add a third asterisk inside"),
        ("plain", 0..0, "**", "start a construct"),
        ("plain", 5..5, "**", "leave an unclosed construct"),
        ("# Heading", 0..1, "", "remove the hash"),
        ("> quote", 0..2, "", "remove the quote marker"),
        ("- item", 0..2, "", "remove the bullet"),
        ("`code`", 3..3, "`", "add a backtick mid-span"),
        ("[a](b)", 3..3, "x", "corrupt link syntax"),
        ("a\\*b", 1..2, "", "remove an escape"),
        (
            "汉字",
            3..3,
            "**",
            "insert syntax between multibyte characters",
        ),
        ("🎉🎉", 4..4, "*", "insert between astral characters"),
    ];

    for (source, range, replacement, name) in cases {
        let mut doc = Document::from(*source);
        doc.replace(range.clone(), replacement)
            .unwrap_or_else(|e| panic!("{name}: edit rejected: {e}"));
        assert_partitions(&mut doc, name);
    }
}
