//! Cutting a document into chunks, and stacking their heights.
//!
//! Every assertion here is about arithmetic, so it runs anywhere. What a chunk *looks* like needs a
//! text engine and lives in `frame.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::blocks::{self, Chunk};
use mdedit_document::Document;
use mdedit_markers::BlockKind;

/// The invariant the whole scheme rests on: chunks tile the buffer.
fn assert_tiles(source: &str, chunks: &[Chunk]) {
    let mut cursor = 0;
    let mut rebuilt = String::new();
    for chunk in chunks {
        assert_eq!(
            chunk.span.start().get(),
            cursor,
            "a gap or an overlap in {chunks:?}"
        );
        rebuilt.push_str(chunk.span.slice(source));
        cursor = chunk.span.end().get();
    }
    assert_eq!(cursor, source.len(), "chunks stop short of the end");
    assert_eq!(rebuilt, source, "the chunks do not reproduce the source");
}

#[test]
fn a_document_is_cut_at_the_start_of_each_block() {
    let source = "# Title\n\nA paragraph.\n\nAnother.\n";
    let chunks = blocks::split(&Document::from(source), source);
    assert_tiles(source, &chunks);

    let kinds: Vec<Option<BlockKind>> = chunks.iter().map(|chunk| chunk.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Some(BlockKind::Heading(1)),
            Some(BlockKind::Paragraph),
            Some(BlockKind::Paragraph),
        ],
    );
    // The blank line after a block belongs to it, because the next cut is the next block's line.
    assert_eq!(chunks[0].span.slice(source), "# Title\n\n");
}

/// A document that opens with blank lines has nothing to attach them to, so they lead.
#[test]
fn leading_blank_lines_are_a_chunk_of_their_own() {
    let source = "\n\n# Title\n";
    let chunks = blocks::split(&Document::from(source), source);
    assert_tiles(source, &chunks);
    assert_eq!(chunks[0].kind, None);
    assert_eq!(chunks[0].span.slice(source), "\n\n");
}

#[test]
fn a_fence_is_one_chunk_however_many_lines_it_has() {
    let source = "para\n\n```rust\nfn a() {}\nfn b() {}\n```\n\nafter\n";
    let chunks = blocks::split(&Document::from(source), source);
    assert_tiles(source, &chunks);

    let fence = chunks
        .iter()
        .find(|chunk| chunk.kind == Some(BlockKind::CodeBlock))
        .expect("the fence is a chunk of its own");
    let body = fence.span.slice(source);
    assert!(body.contains("fn a()") && body.contains("fn b()"));
}

/// A list and its items are one chunk: an item laid out without its list loses its indentation.
#[test]
fn a_list_is_one_chunk_not_one_per_item() {
    let source = "- one\n- two\n- three\n";
    let chunks = blocks::split(&Document::from(source), source);
    assert_tiles(source, &chunks);
    assert_eq!(
        chunks.iter().filter(|chunk| chunk.kind.is_some()).count(),
        1
    );
}

#[test]
fn an_empty_document_is_still_one_chunk() {
    let chunks = blocks::split(&Document::from(""), "");
    assert_eq!(chunks.len(), 1, "the caret needs a line to sit on");
    assert_eq!(chunks[0].span.start().get(), 0);
    assert_eq!(chunks[0].span.end().get(), 0);
}

#[test]
fn a_document_that_is_only_blank_lines_tiles_too() {
    let source = "\n\n\n";
    assert_tiles(source, &blocks::split(&Document::from(source), source));
}

// ---------------------------------------------------------------------------
// Lines, and the one that belongs to the next chunk.
// ---------------------------------------------------------------------------

/// Laid out alone, a chunk ending in a newline has a trailing empty line. Stacked, that line is
/// where the next chunk starts — counting it twice would stretch the document by a line per chunk.
#[test]
fn a_chunk_gives_its_trailing_empty_line_to_the_next_one() {
    assert!(blocks::drops_last_line("para\n", false));
    assert_eq!(blocks::line_count("para\n", false), 1);

    assert!(!blocks::drops_last_line("para\n", true));
    assert_eq!(
        blocks::line_count("para\n", true),
        2,
        "at the end of the document that empty line is real and the caret can sit on it"
    );
}

#[test]
fn a_chunk_not_ending_in_a_newline_keeps_every_line() {
    assert!(!blocks::drops_last_line("para", false));
    assert_eq!(blocks::line_count("para", false), 1);
    assert_eq!(blocks::line_count("a\nb", false), 2);
}

#[test]
fn the_line_counts_of_a_split_document_add_up_to_the_whole() {
    let source = "# Title\n\nA paragraph.\n\n```\ncode\n```\n\nlast\n";
    let chunks = blocks::split(&Document::from(source), source);
    let last = chunks.len() - 1;

    let total: usize = chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| blocks::line_count(chunk.span.slice(source), index == last))
        .sum();

    // What one layout of the whole thing would produce: one line per newline, plus the last.
    let whole = source.bytes().filter(|byte| *byte == b'\n').count() + 1;
    assert_eq!(total, whole, "the split changed how many lines there are");
}

// ---------------------------------------------------------------------------
// Stacking, and which chunks the viewport covers.
// ---------------------------------------------------------------------------

#[test]
fn stacking_gives_each_chunk_the_sum_of_what_came_before() {
    assert_eq!(blocks::stack(&[10.0, 20.0, 5.0]), vec![0.0, 10.0, 30.0]);
    assert!(blocks::stack(&[]).is_empty());
}

#[test]
fn the_viewport_covers_the_chunks_it_touches() {
    let heights = [10.0, 10.0, 10.0, 10.0, 10.0];
    let tops = blocks::stack(&heights);

    assert_eq!(blocks::visible(&tops, &heights, 0.0, 25.0), 0..3);
    assert_eq!(
        blocks::visible(&tops, &heights, 15.0, 10.0),
        1..3,
        "a chunk straddling the top edge is still on screen"
    );
    assert_eq!(blocks::visible(&tops, &heights, 40.0, 10.0), 4..5);
}

#[test]
fn a_viewport_taller_than_the_document_covers_all_of_it() {
    let heights = [10.0, 10.0];
    let tops = blocks::stack(&heights);
    assert_eq!(blocks::visible(&tops, &heights, 0.0, 1000.0), 0..2);
}

#[test]
fn nothing_to_show_is_an_empty_range_rather_than_a_panic() {
    assert_eq!(blocks::visible(&[], &[], 0.0, 100.0), 0..0);
}

/// Zero-height chunks are real: an empty gap between two blocks measures nothing.
#[test]
fn a_zero_height_chunk_does_not_capture_the_viewport() {
    let heights = [10.0, 0.0, 10.0];
    let tops = blocks::stack(&heights);
    let range = blocks::visible(&tops, &heights, 10.0, 5.0);
    assert!(
        range.contains(&2),
        "the chunk with the pixels in it must be drawn: {range:?}"
    );
}
