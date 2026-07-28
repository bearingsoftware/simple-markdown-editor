//! The reveal-on-cursor predicate — the whole live-preview illusion in one function.
//!
//! Typora has two rules and conflating them is the usual mistake, so they are tested separately:
//! inline syntax reveals on touching the marker, block syntax reveals from anywhere in the block.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::{Document, Selection};
use mdedit_offsets::ByteOffset;
use proptest::prelude::*;

/// These tests are about byte positions as numbers, so the two constructors take raw offsets and
/// convert here — the only conversion in the file.
fn caret_at(offset: usize) -> Selection {
    Selection::caret(ByteOffset::new(offset))
}

fn selection(anchor: usize, head: usize) -> Selection {
    Selection::new(ByteOffset::new(anchor), ByteOffset::new(head))
}

/// The marker text revealed for a given caret position, in document order.
fn revealed_at(source: &str, caret: usize) -> Vec<String> {
    revealed(source, caret_at(caret))
}

fn revealed(source: &str, selection: Selection) -> Vec<String> {
    let doc = Document::from(source);
    let text = doc.to_string();
    doc.segments()
        .iter()
        .filter(|s| s.is_marker() && doc.is_revealed(s, selection))
        .map(|s| s.span().slice(&text).to_owned())
        .collect()
}

#[test]
fn inline_markers_hide_until_the_caret_touches_them() {
    //  * *  b o l d  * *
    //  0 1  2 3 4 5  6 7 8
    let source = "**bold**";

    assert_eq!(
        revealed_at(source, 0),
        vec!["**"],
        "caret before the opener"
    );
    assert_eq!(
        revealed_at(source, 1),
        vec!["**"],
        "caret inside the opener"
    );
    assert_eq!(
        revealed_at(source, 2),
        vec!["**"],
        "caret just after the opener"
    );
    assert_eq!(
        revealed_at(source, 4),
        Vec::<String>::new(),
        "caret in the content"
    );
    assert_eq!(
        revealed_at(source, 6),
        vec!["**"],
        "caret just before the closer"
    );
    assert_eq!(revealed_at(source, 8), vec!["**"], "caret after the closer");
}

#[test]
fn adjacency_counts_so_a_delimiter_can_be_deleted() {
    // Caret at 2 is *outside* `**` by half-open reckoning, but a user who has just typed the
    // delimiter expects backspace to work on it, so it must be visible.
    let doc = Document::from("**bold**");
    let text = doc.to_string();
    let opener = doc
        .segments()
        .iter()
        .find(|s| s.is_marker())
        .expect("has a marker")
        .clone();

    assert_eq!(opener.span().slice(&text), "**");
    assert!(doc.is_revealed(&opener, caret_at(2)));
}

#[test]
fn a_selection_spanning_a_marker_reveals_it() {
    let source = "a **bold** b";
    assert_eq!(
        revealed(source, selection(0, 12)),
        vec!["**", "**"],
        "select-all reveals every marker"
    );
    assert_eq!(
        revealed(source, selection(0, 1)),
        Vec::<String>::new(),
        "a selection clear of the markers reveals none"
    );
    assert_eq!(
        revealed(source, selection(4, 6)),
        Vec::<String>::new(),
        "selecting inside the content reveals none"
    );
}

#[test]
fn a_backwards_selection_behaves_the_same() {
    let source = "a **bold** b";
    assert_eq!(
        revealed(source, selection(12, 0)),
        revealed(source, selection(0, 12)),
        "anchor and head order must not matter"
    );
}

#[test]
fn block_markers_reveal_from_anywhere_on_their_line() {
    // The distinguishing case: at offset 5 the caret is in the heading *text*, nowhere near the
    // `## ` marker. Under the inline rule it would vanish while being edited, which is wrong.
    let source = "## Heading\n";
    assert_eq!(
        revealed_at(source, 5),
        vec!["## "],
        "caret in the heading text"
    );
    assert_eq!(
        revealed_at(source, 0),
        vec!["## "],
        "caret before the marker"
    );

    let two_lines = "## Heading\n\nplain paragraph\n";
    assert_eq!(
        revealed_at(two_lines, 20),
        Vec::<String>::new(),
        "caret on another line hides it again"
    );
}

#[test]
fn each_quoted_line_reveals_its_own_marker() {
    let source = "> one\n> two\n";
    assert_eq!(
        revealed_at(source, 3),
        vec!["> "],
        "caret on the first line"
    );
    assert_eq!(
        revealed_at(source, 9),
        vec!["> "],
        "caret on the second line"
    );
    assert_eq!(revealed(source, selection(0, 11)).len(), 2, "spanning both");
}

#[test]
fn list_markers_follow_the_block_rule() {
    let source = "- item one\n- item two\n";
    assert_eq!(
        revealed_at(source, 5),
        vec!["- "],
        "caret in the first item's text"
    );
    assert_eq!(
        revealed_at(source, 16),
        vec!["- "],
        "caret in the second item's text"
    );
}

#[test]
fn content_is_always_visible() {
    let doc = Document::from("## Heading\n\n**bold** text\n");
    let carets = [0, 3, 8, 12, 14, 20];
    for caret in carets {
        for segment in doc.segments() {
            if !segment.is_marker() {
                assert!(
                    doc.is_revealed(segment, caret_at(caret)),
                    "content hidden at caret {caret}"
                );
            }
        }
    }
}

#[test]
fn a_stale_caret_past_the_end_does_not_panic() {
    let doc = Document::from("## Heading\n");
    let segment = doc.segments().first().expect("has a segment").clone();

    // A caret left over from a longer document must clamp, not crash a render. It clamps to the end
    // of the buffer, which is the line *after* the heading, so the marker is correctly hidden —
    // what matters here is that asking does not panic.
    assert!(!doc.is_revealed(&segment, caret_at(9_999)));
    assert!(!doc.is_revealed(&segment, selection(9_999, 10_000)));
}

/// The gap MDEDIT-8 knowingly carried, closed by MDEDIT-12's block extents.
///
/// A fence is the one block marker that does not repeat on the lines it governs, so the line the
/// caret is on cannot answer the question. This used to assert the wrong-but-known behaviour; it
/// now asserts Typora's.
#[test]
fn a_fence_reveals_from_anywhere_inside_its_block() {
    //  ```rust \n f n   x ( )   { } \n ```  \n
    //  0......7    8...............17   18..21
    let source = "```rust\nfn x() {}\n```\n";
    let both = vec!["```rust".to_owned(), "```".to_owned()];

    assert_eq!(revealed_at(source, 2), both, "caret on the opening fence");
    assert_eq!(
        revealed_at(source, 12),
        both,
        "caret inside the code body — the case the line approximation got wrong"
    );
    assert_eq!(revealed_at(source, 19), both, "caret on the closing fence");
}

/// Closing that gap must not widen the markers that were already exact.
///
/// `>` and list bullets are written on every line they affect, so the line *is* the block for them.
/// Treating a whole quote as one unit would reveal every `>` in it because the caret is on the
/// first line, which is worse than what it replaced.
#[test]
fn quotes_and_lists_still_reveal_one_line_at_a_time() {
    let quote = "> one\n> two\n> three\n";
    assert_eq!(
        revealed_at(quote, 3).len(),
        1,
        "a caret in a three-line quote should reveal its own `>`, not all three"
    );

    let list = "- one\n- two\n- three\n";
    assert_eq!(revealed_at(list, 3).len(), 1, "same for list bullets");
}

/// A fence nested in a quote resolves to the fence, not the quote.
#[test]
fn the_innermost_block_wins() {
    let source = "> ```\n> code\n> ```\n";
    let doc = Document::from(source);
    let fences: Vec<_> = doc
        .segments()
        .iter()
        .filter(|s| matches!(s.kind(), mdedit_markers::SegmentKind::Marker(m) if m == mdedit_markers::Marker::CodeBlock))
        .cloned()
        .collect();

    assert!(!fences.is_empty(), "the nested fence produces markers");
    // Caret on the middle line, which carries no fence of its own.
    for fence in &fences {
        assert!(
            doc.is_revealed(fence, caret_at(9)),
            "a fence inside a quote should still reveal from inside the fence"
        );
    }
}

const FRAGMENTS: &[&str] = &[
    "a", "word ", "\n", "\n\n", "**bold**", "*em*", "`code`", "## head", "> quote", "- item",
    "[a](b)", "汉", "🎉", "\\*",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Total at every caret position in the document, including the very end.
    #[test]
    fn predicate_is_total_over_every_caret_position(
        parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..12),
    ) {
        let source: String = parts.into_iter().map(|i| FRAGMENTS[i]).collect();
        let doc = Document::from(source.as_str());
        let segments = doc.segments().to_vec();

        for caret in 0..=source.len() {
            for segment in &segments {
                let _ = doc.is_revealed(segment, caret_at(caret));
            }
        }
    }

    /// A caret inside a marker's own span always reveals it, whichever rule applies.
    #[test]
    fn a_caret_within_a_marker_always_reveals_it(
        parts in proptest::collection::vec(0..FRAGMENTS.len(), 1..12),
    ) {
        let source: String = parts.into_iter().map(|i| FRAGMENTS[i]).collect();
        let doc = Document::from(source.as_str());
        let segments = doc.segments().to_vec();

        for segment in segments.iter().filter(|s| s.is_marker()) {
            let span = segment.span();
            for caret in span.range() {
                prop_assert!(
                    doc.is_revealed(segment, caret_at(caret)),
                    "marker {:?} hidden with the caret at {caret} inside it",
                    span.slice(&source)
                );
            }
        }
    }
}
