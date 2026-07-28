//! Block extents.
//!
//! Blocks are additional structure *over* the same bytes — never a re-serialization, which is the
//! rule that outranks everything in `AGENTS.md`. So the properties asserted here are about extents
//! and ordering; that the bytes survive is `invariant.rs`'s job and still holds, because `segment`
//! and `analyze` are the same parse.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_markers::{Block, BlockKind, analyze};
use proptest::prelude::*;

fn blocks(source: &str) -> Vec<Block> {
    analyze(source).blocks
}

fn kinds(source: &str) -> Vec<BlockKind> {
    blocks(source).iter().map(Block::kind).collect()
}

#[test]
fn a_heading_carries_its_level() {
    assert_eq!(kinds("# one\n"), vec![BlockKind::Heading(1)]);
    assert_eq!(kinds("### three\n"), vec![BlockKind::Heading(3)]);
    assert_eq!(kinds("###### six\n"), vec![BlockKind::Heading(6)]);
    // Setext headings are headings too, and level comes from the underline.
    assert_eq!(kinds("one\n===\n"), vec![BlockKind::Heading(1)]);
    assert_eq!(kinds("one\n---\n"), vec![BlockKind::Heading(2)]);
}

/// The extent that justifies the whole type: a fence's block covers the lines between its markers,
/// which carry no marker of their own.
#[test]
fn a_fence_spans_from_its_opener_to_its_closer() {
    let source = "```rust\nfn x() {}\n```\n";
    let fence = blocks(source)
        .into_iter()
        .find(|b| b.kind() == BlockKind::CodeBlock)
        .expect("a fenced block");

    let covered = fence.span().slice(source);
    assert!(
        covered.starts_with("```rust"),
        "should start at the opening fence, got {covered:?}"
    );
    assert!(
        covered.contains("fn x() {}"),
        "should cover the body, got {covered:?}"
    );
    assert!(
        covered.trim_end().ends_with("```"),
        "should reach the closing fence, got {covered:?}"
    );
}

#[test]
fn nesting_is_reported_by_depth() {
    // A paragraph inside a quote: the quote is outer, the paragraph inner.
    let found = blocks("> quoted\n");
    let quote = found
        .iter()
        .find(|b| b.kind() == BlockKind::BlockQuote)
        .expect("a quote");
    let para = found
        .iter()
        .find(|b| b.kind() == BlockKind::Paragraph)
        .expect("a paragraph inside it");

    assert_eq!(quote.depth(), 0);
    assert!(
        para.depth() > quote.depth(),
        "the paragraph nests inside the quote"
    );
    assert!(
        quote.span().start() <= para.span().start() && para.span().end() <= quote.span().end(),
        "the inner block must lie within the outer one"
    );
}

#[test]
fn a_list_contains_items_which_contain_paragraphs() {
    let found = blocks("- one\n- two\n");
    let list = found.iter().find(|b| b.kind() == BlockKind::List).unwrap();
    let items: Vec<_> = found
        .iter()
        .filter(|b| b.kind() == BlockKind::ListItem)
        .collect();

    assert_eq!(items.len(), 2, "two bullets, two items");
    for item in items {
        assert!(item.depth() > list.depth());
    }
}

#[test]
fn blocks_arrive_outermost_first_in_document_order() {
    let found = blocks("# head\n\npara\n\n> quote\n");
    for pair in found.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        // Either `b` starts at or after `a`, or `b` nests inside `a` — never earlier.
        assert!(
            a.span().start() <= b.span().start(),
            "{:?} at {} precedes {:?} at {}",
            b.kind(),
            b.span(),
            a.kind(),
            a.span()
        );
    }
}

#[test]
fn ids_are_unique_and_index_the_list() {
    let found = blocks("# head\n\npara\n\n- a\n- b\n\n```\ncode\n```\n\n> quote\n\n---\n");
    assert!(found.len() > 5, "the fixture should produce several blocks");
    for (index, block) in found.iter().enumerate() {
        assert_eq!(
            block.id().get() as usize,
            index,
            "a block's id should be its position in the list"
        );
    }
}

#[test]
fn every_block_lies_within_the_source_and_on_character_boundaries() {
    for source in [
        "",
        "plain\n",
        "# héllo\n\n汉字编辑器\n",
        "> 🎉 quoted\n",
        "```\n汉\n```\n",
        "- 汉\n- 字\n",
    ] {
        for block in blocks(source) {
            let span = block.span();
            assert!(
                span.end().get() <= source.len(),
                "{source:?}: block {span} runs past the end"
            );
            assert!(
                source.is_char_boundary(span.start().get())
                    && source.is_char_boundary(span.end().get()),
                "{source:?}: block {span} splits a character"
            );
        }
    }
}

#[test]
fn a_thematic_break_is_a_block() {
    assert!(kinds("---\n").contains(&BlockKind::ThematicBreak));
    assert!(kinds("***\n").contains(&BlockKind::ThematicBreak));
}

#[test]
fn an_empty_document_has_no_blocks() {
    assert!(blocks("").is_empty());
}

/// Blocks are derived from the same parse as the segments, so the two must describe the same text.
#[test]
fn analyze_agrees_with_segment() {
    for source in ["# head\n\npara\n", "```\ncode\n```\n", "> a\n> b\n"] {
        let analysis = analyze(source);
        assert_eq!(
            analysis.segments,
            mdedit_markers::segment(source),
            "analyze and segment must not diverge"
        );
    }
}

const FRAGMENTS: &[&str] = &[
    "",
    "a",
    "word ",
    "\n",
    "\n\n",
    "# ",
    "## ",
    "> ",
    "- ",
    "1. ",
    "```",
    "```rust",
    "---",
    "*",
    "**",
    "`",
    "汉",
    "🎉",
    "    indented",
];

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(512))]

    /// Whatever the input, block extents stay inside it, on boundaries, and in order.
    #[test]
    fn block_extents_are_always_well_formed(
        parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..20),
    ) {
        let source: String = parts.into_iter().map(|i| FRAGMENTS[i]).collect();
        let found = blocks(&source);

        for block in &found {
            let span = block.span();
            prop_assert!(span.start() <= span.end(), "inverted span {}", span);
            prop_assert!(span.end().get() <= source.len(), "span {} past the end", span);
            prop_assert!(source.is_char_boundary(span.start().get()));
            prop_assert!(source.is_char_boundary(span.end().get()));
        }

        for pair in found.windows(2) {
            prop_assert!(
                pair[0].span().start() <= pair[1].span().start(),
                "blocks out of document order"
            );
        }
    }
}
