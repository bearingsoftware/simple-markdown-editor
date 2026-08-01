//! Block formatting: headings, lists, quote, fence and rule.
//!
//! The line-wise half of the formatting vocabulary. Most of it is prefix arithmetic; the cases worth
//! reading are the mixed selection, the renumbering, and removing a fence — which uses the block
//! extent rather than searching for backticks, for the same reason the inline toggles use the marker
//! spans.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::format::Block;
use mdedit_document::{Document, Selection};
use mdedit_offsets::ByteOffset;
use proptest::prelude::*;

/// A document with the caret at `caret`.
fn at(text: &str, caret: usize) -> Document {
    let mut doc = Document::from(text);
    doc.set_selection(Selection::caret(ByteOffset::new(caret)));
    doc
}

/// A document with everything selected.
fn all(text: &str) -> Document {
    let mut doc = Document::from(text);
    doc.set_selection(Selection::new(
        ByteOffset::ZERO,
        ByteOffset::new(text.len()),
    ));
    doc
}

// ---- headings ----

#[test]
fn a_line_becomes_a_heading() {
    let mut doc = at("some prose\n", 3);
    assert!(doc.set_block(Block::Heading(2)));
    assert_eq!(doc.to_string(), "## some prose\n");
}

/// Invoking the level a line already has clears it — which is how a heading becomes a paragraph
/// again, since Ctrl+0 belongs to zoom.
#[test]
fn invoking_the_same_level_clears_it() {
    let mut doc = at("## a heading\n", 5);
    doc.set_block(Block::Heading(2));
    assert_eq!(doc.to_string(), "a heading\n");
}

#[test]
fn changing_level_replaces_rather_than_stacks() {
    let mut doc = at("## a heading\n", 5);
    doc.set_block(Block::Heading(4));
    assert_eq!(doc.to_string(), "#### a heading\n");
}

// ---- lists ----

#[test]
fn lines_become_a_bullet_list() {
    let mut doc = all("one\ntwo\nthree\n");
    assert!(doc.set_block(Block::Bullet));
    assert_eq!(doc.to_string(), "- one\n- two\n- three\n");
}

/// The renumbering claim: an application counts from one, so a list built from lines that were
/// never numbered comes out consecutive.
#[test]
fn an_ordered_list_numbers_from_one() {
    let mut doc = all("one\ntwo\nthree\n");
    doc.set_block(Block::Ordered);
    assert_eq!(doc.to_string(), "1. one\n2. two\n3. three\n");
}

/// And converting renumbers too, so odd numbering in the source does not survive a round trip
/// through another list kind.
#[test]
fn converting_to_ordered_renumbers() {
    let mut doc = all("7. one\n3. two\n9. three\n");
    doc.set_block(Block::Bullet);
    doc.set_block(Block::Ordered);
    assert_eq!(doc.to_string(), "1. one\n2. two\n3. three\n");
}

/// Invoking Ordered on a list that is *already* ordered clears it, like every other toggle here.
/// Renumbering happens when applying, not as a second meaning for the same press.
#[test]
fn an_already_ordered_list_clears() {
    let mut doc = all("1. one\n2. two\n");
    doc.set_block(Block::Ordered);
    assert_eq!(doc.to_string(), "one\ntwo\n");
}

#[test]
fn converting_between_list_kinds_swaps_the_marker() {
    let mut doc = all("- one\n- two\n");
    doc.set_block(Block::Ordered);
    assert_eq!(doc.to_string(), "1. one\n2. two\n");

    doc.set_block(Block::Task);
    assert_eq!(doc.to_string(), "- [ ] one\n- [ ] two\n");

    doc.set_block(Block::Bullet);
    assert_eq!(doc.to_string(), "- one\n- two\n");
}

/// A task is a bullet with a checkbox, so the two must not both claim the same line.
#[test]
fn a_task_is_not_also_a_plain_bullet() {
    let mut doc = all("- [ ] a task\n");
    // Invoking Bullet on a task converts it, rather than clearing it as it would a plain bullet.
    doc.set_block(Block::Bullet);
    assert_eq!(doc.to_string(), "- a task\n");
}

/// The decision the ticket asked to be stated rather than improvised: half a list is not a thing
/// anyone means, so a mixed selection is made uniform.
#[test]
fn a_mixed_selection_applies_uniformly() {
    let mut doc = all("- one\ntwo\n- three\n");
    doc.set_block(Block::Bullet);
    assert_eq!(
        doc.to_string(),
        "- one\n- two\n- three\n",
        "the unlisted line should have joined the list, not the listed ones left it"
    );

    // And one more invocation clears them all.
    doc.set_block(Block::Bullet);
    assert_eq!(doc.to_string(), "one\ntwo\nthree\n");
}

#[test]
fn an_indented_line_keeps_its_indent() {
    let mut doc = all("    indented\n");
    doc.set_block(Block::Bullet);
    assert_eq!(doc.to_string(), "    - indented\n");
}

// ---- quote ----

#[test]
fn lines_become_quoted_and_unquoted() {
    let mut doc = all("one\ntwo\n");
    assert!(doc.toggle_quote());
    assert_eq!(doc.to_string(), "> one\n> two\n");

    doc.toggle_quote();
    assert_eq!(doc.to_string(), "one\ntwo\n");
}

/// Quoting is orthogonal to the rest: a quoted list is `> - item`, and neither operation may
/// disturb the other.
#[test]
fn quoting_a_list_keeps_the_list() {
    let mut doc = all("- one\n- two\n");
    doc.toggle_quote();
    assert_eq!(doc.to_string(), "> - one\n> - two\n");

    doc.set_block(Block::Ordered);
    assert_eq!(doc.to_string(), "> 1. one\n> 2. two\n");

    doc.toggle_quote();
    assert_eq!(doc.to_string(), "1. one\n2. two\n");
}

/// One level, not all of them.
#[test]
fn unquoting_removes_a_single_level() {
    let mut doc = all("> > deep\n");
    doc.toggle_quote();
    assert_eq!(doc.to_string(), "> deep\n");
}

// ---- fence ----

#[test]
fn lines_are_wrapped_in_a_fence() {
    let mut doc = all("let x = 1;\n");
    assert!(doc.toggle_fence());
    assert_eq!(doc.to_string(), "```\nlet x = 1;\n```\n");
}

/// The caret parks on the info string, because the first thing anybody types after making a code
/// block is the language.
#[test]
fn the_caret_lands_on_the_info_string() {
    let mut doc = all("let x = 1;\n");
    doc.toggle_fence();
    let caret = doc.selection().head();
    doc.insert(caret, "rust").unwrap();
    assert_eq!(doc.to_string(), "```rust\nlet x = 1;\n```\n");
}

/// Removing uses the block extent rather than searching for backticks.
#[test]
fn a_fence_is_removed_from_inside_it() {
    let mut doc = at("```rust\nlet x = 1;\n```\n", 10);
    assert!(doc.toggle_fence());
    assert_eq!(doc.to_string(), "let x = 1;\n");
}

#[test]
fn text_around_a_fence_is_untouched() {
    let source = "before\n\n```\ncode\n```\n\nafter\n";
    let mut doc = at(source, 14);
    doc.toggle_fence();
    assert_eq!(doc.to_string(), "before\n\ncode\n\nafter\n");
}

// ---- rule ----

/// `---` directly under a paragraph is a setext heading, not a rule. The blank line above is not
/// decoration, and this is the single easiest way to get thematic breaks wrong.
#[test]
fn a_rule_gets_the_blank_line_it_needs() {
    let mut doc = at("some prose\n", 4);
    assert!(doc.insert_rule());
    assert_eq!(doc.to_string(), "some prose\n\n---\n");

    let text = doc.to_string();
    let fresh = Document::from(text.as_str());
    assert!(
        fresh
            .blocks()
            .iter()
            .any(|b| b.kind() == mdedit_markers::BlockKind::ThematicBreak),
        "it has to parse as a rule, not as a setext heading: {:?}",
        fresh.blocks()
    );
}

// ---- undo ----

/// Twenty lines is twenty edits, and one press must undo as one press.
#[test]
fn a_multi_line_action_is_one_undo_step() {
    let source = "one\ntwo\nthree\nfour\nfive\n";
    let mut doc = all(source);
    doc.set_block(Block::Bullet);
    assert_eq!(doc.to_string(), "- one\n- two\n- three\n- four\n- five\n");

    assert!(doc.undo());
    assert_eq!(doc.to_string(), source);
    assert!(!doc.undo(), "there should be nothing else to take back");
}

#[test]
fn a_fence_is_one_undo_step_too() {
    let source = "code\n";
    let mut doc = all(source);
    doc.toggle_fence();
    assert!(doc.undo());
    assert_eq!(doc.to_string(), source);
    assert!(!doc.undo());
}

// ---- bytes ----

#[test]
fn applying_and_clearing_restores_the_exact_bytes() {
    for source in [
        "one\ntwo\n",
        "    indented\nplain\n",
        "> quoted\n> lines\n",
        "a\r\nb\r\n",
    ] {
        for what in [
            Block::Bullet,
            Block::Ordered,
            Block::Task,
            Block::Heading(3),
        ] {
            let mut doc = all(source);
            doc.set_block(what);
            doc.set_block(what);
            assert_eq!(doc.to_string(), source, "{what:?} on {source:?}");
        }

        let mut doc = all(source);
        doc.toggle_quote();
        doc.toggle_quote();
        assert_eq!(doc.to_string(), source, "quote on {source:?}");
    }
}

proptest! {
    /// Applying a block format and clearing it restores the document byte for byte, whatever it was.
    #[test]
    fn applying_and_clearing_is_the_identity(
        lines in prop::collection::vec("[a-z ]{0,12}", 1..8),
        which in 0_usize..4,
    ) {
        let source = format!("{}\n", lines.join("\n"));
        let what = [Block::Bullet, Block::Ordered, Block::Task, Block::Heading(2)][which];

        let mut doc = all(source.as_str());
        doc.set_block(what);
        doc.set_block(what);

        prop_assert_eq!(doc.to_string(), source);
    }

    /// And one undo takes back whatever it did.
    #[test]
    fn one_undo_takes_back_a_block_action(
        lines in prop::collection::vec("[a-z ]{0,12}", 1..8),
    ) {
        let source = format!("{}\n", lines.join("\n"));
        let mut doc = all(source.as_str());

        if doc.set_block(Block::Bullet) {
            prop_assert!(doc.undo());
        }
        prop_assert_eq!(doc.to_string(), source);
    }
}

// ---- setext headings ----

/// A setext heading is two lines — the text and the underline — and only the block list knows that.
/// Prefixing the first line would leave the underline behind, still making it a heading and now a
/// different one.
#[test]
fn a_setext_heading_converts_to_atx() {
    let mut doc = at("A heading\n=========\n\nprose\n", 3);
    assert!(doc.set_block(Block::Heading(3)));
    assert_eq!(doc.to_string(), "### A heading\n\nprose\n");
}

#[test]
fn a_setext_level_two_converts_too() {
    let mut doc = at("A heading\n---------\n\nprose\n", 3);
    doc.set_block(Block::Heading(1));
    assert_eq!(doc.to_string(), "# A heading\n\nprose\n");
}

/// And it can be turned into a list, which also has to take the underline with it.
#[test]
fn a_setext_heading_converts_to_a_list() {
    let mut doc = at("A heading\n=========\n\nprose\n", 3);
    doc.set_block(Block::Bullet);
    assert_eq!(doc.to_string(), "- A heading\n\nprose\n");
}

/// An ATX heading is one line and belongs to the ordinary prefix path — this is the control showing
/// the setext path has not swallowed it.
#[test]
fn an_atx_heading_still_takes_the_prefix_path() {
    let mut doc = at("## A heading\n\nprose\n", 5);
    doc.set_block(Block::Heading(4));
    assert_eq!(doc.to_string(), "#### A heading\n\nprose\n");
}
