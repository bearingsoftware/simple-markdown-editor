//! Replacing many sites as one operation, and the undo step it has to be.
//!
//! The property at the bottom is the one that matters and it is the byte-preservation invariant
//! wearing a different hat: a Replace All followed by one undo has to give back the document that
//! was there, byte for byte, not a re-derived approximation of it. Everything above it is the
//! arithmetic that has to hold for that to be possible.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
use mdedit_offsets::ByteSpan;
use proptest::prelude::*;

fn span(from: usize, to: usize) -> ByteSpan {
    ByteSpan::from(from..to)
}

/// Every occurrence of `needle`, which is all these tests need of a search.
fn sites(text: &str, needle: &str) -> Vec<ByteSpan> {
    text.match_indices(needle)
        .map(|(at, found)| ByteSpan::from(at..at + found.len()))
        .collect()
}

#[test]
fn every_site_is_replaced() {
    let mut doc = Document::from("alpha beta alpha gamma alpha\n");
    let found = sites(&doc.to_string(), "alpha");
    assert_eq!(doc.replace_all(&found, "delta"), 3);
    assert_eq!(doc.to_string(), "delta beta delta gamma delta\n");
}

/// The case that makes the direction matter. A replacement longer than what it replaces shifts every
/// site after it, so a loop running forwards would put the second one in the wrong place.
#[test]
fn a_longer_replacement_does_not_shift_the_sites_after_it() {
    let mut doc = Document::from("a b a b a\n");
    let found = sites(&doc.to_string(), "a");
    assert_eq!(doc.replace_all(&found, "xxxx"), 3);
    assert_eq!(doc.to_string(), "xxxx b xxxx b xxxx\n");
}

#[test]
fn a_shorter_replacement_works_the_same_way() {
    let mut doc = Document::from("alpha beta alpha\n");
    let found = sites(&doc.to_string(), "alpha");
    assert_eq!(doc.replace_all(&found, "x"), 2);
    assert_eq!(doc.to_string(), "x beta x\n");
}

#[test]
fn deleting_every_site_is_a_replacement_with_nothing() {
    let mut doc = Document::from("a-b-c-d\n");
    let found = sites(&doc.to_string(), "-");
    assert_eq!(doc.replace_all(&found, ""), 3);
    assert_eq!(doc.to_string(), "abcd\n");
}

/// A replacement containing the thing being replaced is the classic way a naive implementation
/// loops for ever. It cannot here: the sites are computed once, before anything is edited.
#[test]
fn a_replacement_containing_the_needle_does_not_loop() {
    let mut doc = Document::from("cat cat\n");
    let found = sites(&doc.to_string(), "cat");
    assert_eq!(doc.replace_all(&found, "cathedral"), 2);
    assert_eq!(doc.to_string(), "cathedral cathedral\n");
}

#[test]
fn the_sites_need_not_arrive_in_order() {
    let mut doc = Document::from("a b a\n");
    // Deliberately back to front, which is what a caller walking a list backwards would hand over.
    assert_eq!(doc.replace_all(&[span(4, 5), span(0, 1)], "z"), 2);
    assert_eq!(doc.to_string(), "z b z\n");
}

/// Overlapping sites are a caller error — a search never produces them. What matters is that the
/// buffer stays coherent rather than which one wins; the loop runs backwards, so the later one is
/// applied first and the earlier one, which now reaches into bytes that have gone, is skipped.
#[test]
fn overlapping_sites_are_skipped_rather_than_applied() {
    let mut doc = Document::from("aaaa\n");
    assert_eq!(doc.replace_all(&[span(0, 3), span(2, 4)], "z"), 1);
    assert_eq!(doc.to_string(), "aaz\n");
}

#[test]
fn a_multi_byte_site_is_replaced_whole() {
    let mut doc = Document::from("汉字编辑器\n");
    let found = sites(&doc.to_string(), "编辑");
    assert_eq!(doc.replace_all(&found, "x"), 1);
    assert_eq!(doc.to_string(), "汉字x器\n");
}

// ---- one undo step ----

#[test]
fn one_undo_takes_back_every_replacement() {
    let text = "alpha beta alpha gamma alpha\n";
    let mut doc = Document::from(text);
    let found = sites(text, "alpha");
    assert_eq!(doc.replace_all(&found, "delta"), 3);

    assert!(doc.undo());
    assert_eq!(doc.to_string(), text);
    assert!(
        !doc.can_undo(),
        "three replacements should have been one step, not three"
    );
}

#[test]
fn one_redo_puts_them_all_back() {
    let mut doc = Document::from("alpha beta alpha\n");
    let found = sites(&doc.to_string(), "alpha");
    let _ = doc.replace_all(&found, "delta");
    let after = doc.to_string();

    assert!(doc.undo());
    assert!(doc.redo());
    assert_eq!(doc.to_string(), after);
    assert!(!doc.can_redo(), "one step in each direction");
}

/// A Replace All that matched nothing must not leave an undo press that appears to do nothing.
#[test]
fn replacing_nothing_records_no_history() {
    let mut doc = Document::from("alpha\n");
    assert_eq!(doc.replace_all(&[], "delta"), 0);
    assert!(!doc.can_undo());
    assert_eq!(doc.to_string(), "alpha\n");
}

/// Typing after a Replace All must not join it. The transaction is a complete operation, and undoing
/// the character you just typed should not also undo the replacement.
#[test]
fn typing_afterwards_is_its_own_step() {
    let mut doc = Document::from("alpha\n");
    let found = sites(&doc.to_string(), "alpha");
    let _ = doc.replace_all(&found, "delta");
    doc.insert(mdedit_offsets::ByteOffset::new(5), "!").unwrap();

    assert!(doc.undo());
    assert_eq!(doc.to_string(), "delta\n");
    assert!(doc.undo());
    assert_eq!(doc.to_string(), "alpha\n");
}

/// The segmentation has to be right afterwards, not just the bytes. A transaction defers the parse
/// to the end, so a document left un-reparsed would draw its old markers over its new text.
#[test]
fn the_document_is_reparsed_once_at_the_end() {
    let mut doc = Document::from("plain text here\n");
    let found = sites(&doc.to_string(), "plain");
    let _ = doc.replace_all(&found, "**bold**");
    assert_eq!(doc.to_string(), "**bold** text here\n");
    assert!(
        doc.segments()
            .iter()
            .any(|segment| segment.span().slice(&doc.to_string()) == "**"),
        "the new emphasis markers should be in the partition"
    );
}

proptest! {
    /// **The one that matters.** Whatever was replaced with whatever, one undo gives back the exact
    /// bytes — which is the byte-preservation invariant applied to the operation most likely to
    /// break it.
    #[test]
    fn replace_all_then_undo_restores_the_original_bytes(
        words in prop::collection::vec("[ab]{1,4}", 1..14),
        needle in "[ab]{1,3}",
        replacement in "[a-z]{0,5}",
    ) {
        let text = format!("{}\n", words.join(" "));
        let mut doc = Document::from(text.as_str());
        let found = sites(&text, &needle);

        let done = doc.replace_all(&found, &replacement);
        if done == 0 {
            prop_assert_eq!(doc.to_string(), text);
            prop_assert!(!doc.can_undo());
            return Ok(());
        }

        prop_assert!(doc.undo());
        prop_assert_eq!(doc.to_string(), text);
        prop_assert!(!doc.can_undo(), "however many sites, one step");
    }

    /// And redo puts back exactly what undo took away.
    #[test]
    fn redo_reapplies_the_whole_operation(
        words in prop::collection::vec("[ab]{1,4}", 1..14),
        needle in "[ab]{1,3}",
        replacement in "[a-z]{0,5}",
    ) {
        let text = format!("{}\n", words.join(" "));
        let mut doc = Document::from(text.as_str());
        let found = sites(&text, &needle);

        if doc.replace_all(&found, &replacement) == 0 {
            return Ok(());
        }
        let after = doc.to_string();

        prop_assert!(doc.undo());
        prop_assert!(doc.redo());
        prop_assert_eq!(doc.to_string(), after);
    }
}
