//! What the block list guarantees about its own shape, and the two lookups that assume it.
//!
//! `Document::block_at` binary-searches the list and then walks backwards, stopping the moment it
//! passes a depth-0 block that does not contain the offset. `in_block_quote` and `code_block_at` do
//! the same. Every one of those walks rests on **containment** — that a block of depth `d + 1` lies
//! inside the nearest preceding block of depth `d` — and that was assumed everywhere and asserted
//! nowhere (MDEDIT-110). A list that broke it would make all three return the wrong answer on a
//! document nobody had thought to write a fixture for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
use mdedit_offsets::ByteOffset;
use proptest::prelude::*;

/// Every block lies inside the one it is nested in, and the list is in document order.
fn assert_contained(source: &str) {
    let doc = Document::from(source);
    let blocks = doc.blocks();

    // The span of the innermost open block at each depth, as the walk goes.
    let mut open: Vec<mdedit_offsets::ByteSpan> = Vec::new();
    let mut previous_start = ByteOffset::ZERO;

    for block in blocks {
        let span = block.span();
        assert!(
            span.start() >= previous_start,
            "blocks are out of document order at {span} in {source:?}"
        );
        assert!(
            span.start() <= span.end(),
            "backwards block span {span} in {source:?}"
        );
        assert!(
            span.end().get() <= source.len(),
            "block span {span} past the end of {} bytes in {source:?}",
            source.len()
        );
        previous_start = span.start();

        let depth = usize::from(block.depth());
        open.truncate(depth);
        assert_eq!(
            open.len(),
            depth,
            "a block at depth {depth} with no block at depth {} above it, in {source:?}",
            depth.saturating_sub(1)
        );
        if let Some(parent) = open.last() {
            assert!(
                parent.start() <= span.start() && span.end() <= parent.end(),
                "a depth-{depth} block at {span} is not inside its depth-{} parent at {parent}, \
                 in {source:?}",
                depth - 1
            );
        }
        open.push(span);
    }
}

#[test]
fn nesting_is_contained_in_the_documents_that_nest() {
    for source in [
        "> quoted\n",
        "> - a list in a quote\n> - and another\n",
        "- outer\n  - inner\n    - innermost\n",
        "> > doubly quoted\n",
        "- item\n\n  ```\n  code in a list\n  ```\n",
        "> - > a quote in a list in a quote\n",
        "# Heading\n\nparagraph\n\n> quote\n\n- list\n",
        "",
        "\n\n\n",
    ] {
        assert_contained(source);
    }
}

/// `in_block_quote` asks whether *anything* enclosing an offset is a quote, which `block_at` cannot
/// answer — it returns the innermost block, and the bytes of a quoted paragraph are a paragraph.
///
/// It had no test in this crate at all (MDEDIT-110); the only thing exercising it was the renderer.
#[test]
fn a_quote_encloses_its_paragraph_however_deep_it_is() {
    let doc = Document::from("before\n\n> quoted prose\n\nafter\n");
    let at = |offset: usize| doc.in_block_quote(ByteOffset::new(offset));

    assert!(!at(0), "before the quote");
    assert!(
        at(12),
        "inside the quoted paragraph — the innermost block is the paragraph"
    );
    assert!(!at(26), "after it");
}

#[test]
fn a_doubly_quoted_line_is_still_quoted() {
    let doc = Document::from("> > deep\n");
    assert!(doc.in_block_quote(ByteOffset::new(5)));
}

/// A list inside a quote is quoted; a list outside one is not. This is the case the flat block list
/// makes easy to get wrong, because the innermost block is the list item either way.
#[test]
fn a_list_inside_a_quote_is_quoted_and_one_outside_is_not() {
    let quoted = Document::from("> - item\n");
    assert!(quoted.in_block_quote(ByteOffset::new(5)));

    let plain = Document::from("- item\n");
    assert!(!plain.in_block_quote(ByteOffset::new(3)));
}

#[test]
fn an_offset_past_the_end_is_not_in_anything() {
    let doc = Document::from("> quoted\n");
    assert!(!doc.in_block_quote(ByteOffset::new(1_000)));
}

proptest! {
    /// Containment over generated documents, including the newline-bearing shapes the tiling
    /// properties used to exclude — a block list is decided entirely by where the lines are.
    #[test]
    fn nesting_is_contained_whatever_the_document(source in "(?s)[a-z >\\-*#`\n]{0,200}") {
        assert_contained(&source);
    }

    /// And the answer `in_block_quote` gives agrees with the block list it is derived from: an
    /// offset is in a quote exactly when some block containing it is one.
    #[test]
    fn in_block_quote_agrees_with_the_block_list(
        source in "(?s)[a-z >\\-#\n]{0,120}",
        offset in 0_usize..120,
    ) {
        let doc = Document::from(source.as_str());
        let at = ByteOffset::new(offset.min(source.len()));

        let expected = doc.blocks().iter().any(|block| {
            block.kind() == mdedit_language::BlockKind::BlockQuote
                && block.span().start() <= at
                && at < block.span().end()
        });
        prop_assert_eq!(doc.in_block_quote(at), expected, "in {:?} at {}", source, at);
    }
}
