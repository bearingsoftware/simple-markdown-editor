//! Block formatting: headings, lists, quote, fence and rule.
//!
//! The line-wise half of the formatting vocabulary. Most of it is prefix arithmetic; the cases worth
//! reading are the mixed selection, the renumbering, and removing a fence — which uses the block
//! extent rather than searching for backticks, for the same reason the inline toggles use the marker
//! spans.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::format::{Block, Inline};
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

// ---- table ----

/// GFM will not let a table interrupt a paragraph, so a table written directly under one is not a
/// table at all — the same trap the rule above documents, arriving by a different route.
#[test]
fn a_table_gets_the_blank_line_it_needs() {
    let mut doc = at("some prose\n", 4);
    assert!(doc.insert_table(2, 2));
    assert_eq!(
        doc.to_string(),
        "some prose\n\n|     |     |\n| --- | --- |\n|     |     |\n"
    );
}

/// And exactly one. A blank line already there is not doubled.
#[test]
fn a_table_after_a_blank_line_gets_no_second_one() {
    let mut doc = at("some prose\n\n", 12);
    assert!(doc.insert_table(1, 1));
    assert_eq!(doc.to_string(), "some prose\n\n|     |\n| --- |\n");
}

/// The blank line *below* only exists when something follows. At the end of the document it would be
/// litter.
#[test]
fn a_table_is_separated_from_what_follows_it() {
    let mut doc = at("first\n\nsecond\n", 2);
    assert!(doc.insert_table(1, 1));
    assert_eq!(doc.to_string(), "first\n\n|     |\n| --- |\n\nsecond\n");
}

/// **`rows` counts the header**, which is what the picker showed: three lit squares mean three rows
/// of table, one of which is the header the delimiter sits under.
#[test]
fn the_size_asked_for_is_the_size_written() {
    let mut doc = at("", 0);
    assert!(doc.insert_table(3, 4));

    let text = doc.to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "three rows and the delimiter: {lines:?}");
    assert_eq!(lines[1], "| --- | --- | --- | --- |");
    for line in &lines {
        assert_eq!(
            line.matches('|').count(),
            5,
            "four columns need five pipes: {line}"
        );
    }
}

/// Every row is written to the width of the `---` under it, so the source is readable in the tools
/// that are not this editor — and is already in the shape MDEDIT-128's re-pad will keep it in.
#[test]
fn the_pipes_line_up_in_the_source() {
    let mut doc = at("", 0);
    assert!(doc.insert_table(4, 3));

    let text = doc.to_string();
    let mut lines = text.lines();
    let first = lines.next().expect("a table has rows");
    for line in lines {
        assert_eq!(
            line.len(),
            first.len(),
            "`{line}` is not the width of `{first}`"
        );
    }
}

/// Straight into the first cell, ready to type. A table you have to click into is a table the picker
/// only half inserted.
#[test]
fn the_caret_lands_in_the_first_cell() {
    let mut doc = at("some prose\n", 4);
    assert!(doc.insert_table(2, 2));

    let text = doc.to_string();
    let caret = doc.selection().head().get();
    assert_eq!(
        &text[caret - 2..caret],
        "| ",
        "the caret should sit just inside the first cell"
    );
    assert!(doc.selection().is_collapsed(), "and select nothing");
}

/// One transaction, so one Ctrl+Z restores every byte — including the blank line the table forced
/// above itself.
#[test]
fn one_undo_takes_the_whole_table_back() {
    let source = "some prose\n";
    let mut doc = at(source, 4);
    assert!(doc.insert_table(5, 5));
    assert_ne!(doc.to_string(), source);

    assert!(doc.undo());
    assert_eq!(doc.to_string(), source);
}

/// A table of no rows or no columns is not a table, and writing the pipes for one would put
/// malformed bytes in the buffer rather than refusing.
#[test]
fn a_table_with_no_rows_or_no_columns_is_not_written() {
    let mut doc = at("prose\n", 2);
    assert!(!doc.insert_table(0, 3));
    assert!(!doc.insert_table(3, 0));
    assert_eq!(doc.to_string(), "prose\n");
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

// ---- states a document passes through while it is being typed (MDEDIT-107) ----

/// **Data loss.** An unclosed fence is what a fence looks like halfway through being typed, and
/// `unwrap_fence` treated the block's last line as the closing one and deleted it.
#[test]
fn unwrapping_an_unclosed_fence_keeps_every_line_of_code() {
    let mut doc = at("```\nsome code\nmore code", 6);
    doc.toggle_fence();
    assert!(
        doc.to_string().contains("more code"),
        "the last line was destroyed: {:?}",
        doc.to_string()
    );
}

/// **Data loss.** A setext heading's text may span lines — CommonMark says so — and `rewrite_setext`
/// kept only the first, throwing the rest away.
#[test]
fn a_multi_line_setext_heading_keeps_all_of_its_text() {
    let mut doc = at("Foo\nbar\n===\n", 1);
    doc.set_block(Block::Heading(2));
    assert!(
        doc.to_string().contains("bar"),
        "the second line was destroyed: {:?}",
        doc.to_string()
    );
}

/// **A toggle that cannot be un-toggled.** `toggle_inline` refuses inside a code *span* and did not
/// inside a code *block*, where `**` is equally not markup — so the first press inserted delimiters
/// the parser reports no markers for, and the second could not find them to take away.
#[test]
fn inline_formatting_does_nothing_inside_a_fenced_block() {
    let mut doc = at("```\nsome code\n```\n", 6);
    let before = doc.to_string();

    assert!(!doc.toggle_inline(mdedit_document::format::Inline::Bold));
    assert_eq!(doc.to_string(), before);

    assert!(!doc.toggle_inline(mdedit_document::format::Inline::Italic));
    assert!(!doc.toggle_inline(mdedit_document::format::Inline::Strikethrough));
    assert_eq!(doc.to_string(), before);
}

/// The same for an indented code block, whose marker is the indent rather than a fence.
#[test]
fn inline_formatting_does_nothing_inside_an_indented_block() {
    let mut doc = at("paragraph\n\n    some code\n", 18);
    let before = doc.to_string();

    assert!(!doc.toggle_inline(mdedit_document::format::Inline::Bold));
    assert_eq!(doc.to_string(), before);
}

proptest! {
    /// MDEDIT-107. The three states above are what a document looks like *while it is being typed*,
    /// and every one of them lost content. This runs the inline toggle over them at every caret and
    /// demands the same thing the ordinary property does: putting a construct on and taking it
    /// straight off again leaves every byte where it was.
    ///
    /// Inside a code block that is satisfied by both presses doing nothing, which is the fix.
    #[test]
    fn toggling_twice_in_a_half_typed_document_is_the_identity(
        which_document in 0_usize..4,
        caret in 0_usize..40,
        which in 0_usize..4,
    ) {
        let text = [
            // An unclosed fence, mid-typing.
            "```\nsome code\nmore code",
            // A setext heading whose text spans lines.
            "Foo\nbar\n===\n\nprose\n",
            // A closed fence with prose either side of it.
            "before\n\n```\nsome code\n```\n\nafter\n",
            // An indented code block.
            "prose\n\n    some code\n\nmore prose\n",
        ][which_document];
        let what = [Inline::Bold, Inline::Italic, Inline::Strikethrough, Inline::Code][which];

        let mut doc = Document::from(text);
        let at = ByteOffset::new(boundary(text, caret));
        doc.set_selection(Selection::caret(at));

        // The same two preconditions the ordinary property carries: a press that *removes* markup is
        // not required to put back the same extent, and an empty pair inserted hard against an
        // existing delimiter lengthens the run instead of making a pair.
        let mark = what.delimiter().as_bytes()[0];
        let touching = text.as_bytes().get(at.get().wrapping_sub(1)) == Some(&mark)
            || text.as_bytes().get(at.get()) == Some(&mark);
        if doc.is_inside(what) || touching {
            return Ok(());
        }

        doc.toggle_inline(what);
        doc.toggle_inline(what);

        prop_assert_eq!(doc.to_string(), text);
    }
}

/// Snaps an offset onto a character boundary at or below it.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}
