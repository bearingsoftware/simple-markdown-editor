//! The reveal-on-cursor predicate — the whole live-preview illusion in one function.
//!
//! Typora has two rules and conflating them is the usual mistake, so they are tested separately:
//! inline syntax reveals on touching the marker, block syntax reveals from anywhere in the block.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::{Document, Selection};
use proptest::prelude::*;

/// The marker text revealed for a given caret position, in document order.
fn revealed_at(source: &str, caret: usize) -> Vec<String> {
    revealed(source, Selection::caret(caret))
}

fn revealed(source: &str, selection: Selection) -> Vec<String> {
    let doc = Document::from(source);
    let text = doc.to_string();
    doc.segments()
        .iter()
        .filter(|s| s.is_marker() && doc.is_revealed(s, selection))
        .map(|s| text[s.range()].to_owned())
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

    assert_eq!(&text[opener.range()], "**");
    assert!(doc.is_revealed(&opener, Selection::caret(2)));
}

#[test]
fn a_selection_spanning_a_marker_reveals_it() {
    let source = "a **bold** b";
    assert_eq!(
        revealed(source, Selection::new(0, 12)),
        vec!["**", "**"],
        "select-all reveals every marker"
    );
    assert_eq!(
        revealed(source, Selection::new(0, 1)),
        Vec::<String>::new(),
        "a selection clear of the markers reveals none"
    );
    assert_eq!(
        revealed(source, Selection::new(4, 6)),
        Vec::<String>::new(),
        "selecting inside the content reveals none"
    );
}

#[test]
fn a_backwards_selection_behaves_the_same() {
    let source = "a **bold** b";
    assert_eq!(
        revealed(source, Selection::new(12, 0)),
        revealed(source, Selection::new(0, 12)),
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
    assert_eq!(
        revealed(source, Selection::new(0, 11)).len(),
        2,
        "spanning both"
    );
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
                    doc.is_revealed(segment, Selection::caret(caret)),
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
    assert!(!doc.is_revealed(&segment, Selection::caret(9_999)));
    assert!(!doc.is_revealed(&segment, Selection::new(9_999, 10_000)));
}

/// The gap this design knowingly carries — asserted so it changes deliberately rather than by
/// accident. Typora reveals a fence from anywhere inside the block; the line approximation does not.
#[test]
fn multiline_fences_reveal_only_on_their_own_line() {
    let source = "```rust\nfn x() {}\n```\n";
    assert_eq!(
        revealed_at(source, 2),
        vec!["```rust"],
        "caret on the opening fence"
    );
    assert_eq!(
        revealed_at(source, 12),
        Vec::<String>::new(),
        "caret inside the code body — Typora would still show the fences"
    );
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
                let _ = doc.is_revealed(segment, Selection::caret(caret));
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
            let range = segment.range();
            for caret in range.clone() {
                prop_assert!(
                    doc.is_revealed(segment, Selection::caret(caret)),
                    "marker {:?} hidden with the caret at {caret} inside it",
                    &source[range.clone()]
                );
            }
        }
    }
}
