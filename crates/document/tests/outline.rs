//! The heading trail, and the line and column of a position.
//!
//! The breadcrumb's whole job is to answer "where am I". The cases worth pinning are the ones where
//! the honest answer is unusual: above every heading, inside a fence under a heading, and in a
//! document whose levels skip — because Markdown lets them, and a trail built on the assumption that
//! they do not would quietly drop a step.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
use mdedit_offsets::ByteOffset;

/// The trail at the first byte of `needle`, as levels and text.
fn trail_at(source: &str, needle: &str) -> Vec<(u8, String)> {
    let document = Document::from(source);
    let at = ByteOffset::new(source.find(needle).expect("the needle should be present"));
    document
        .heading_trail(at)
        .iter()
        .map(|heading| (heading.level(), document.heading_text(heading)))
        .collect()
}

#[test]
fn a_position_under_nested_headings_carries_the_whole_trail() {
    let source = "# Guide\n\nintro\n\n## Costs\n\n### Hosting\n\nthe bill\n";
    assert_eq!(
        trail_at(source, "the bill"),
        vec![
            (1, "Guide".to_owned()),
            (2, "Costs".to_owned()),
            (3, "Hosting".to_owned()),
        ]
    );
}

/// Nothing encloses a position above the first heading, and saying so is better than inventing a
/// root for it.
#[test]
fn nothing_above_the_first_heading() {
    let source = "some front matter prose\n\n# Guide\n\nintro\n";
    assert!(trail_at(source, "front matter").is_empty());
}

/// The caret being *in* a heading puts you under it, not under the one before.
#[test]
fn a_caret_inside_a_heading_is_under_it() {
    let source = "# Guide\n\n## Costs\n\ntext\n";
    assert_eq!(
        trail_at(source, "Costs"),
        vec![(1, "Guide".to_owned()), (2, "Costs".to_owned())]
    );
}

/// A sibling replaces its predecessor rather than stacking on it.
#[test]
fn a_later_sibling_replaces_the_earlier_one() {
    let source = "# Guide\n\n## Costs\n\na\n\n## Risks\n\nb\n";
    assert_eq!(
        trail_at(source, "\nb\n"),
        vec![(1, "Guide".to_owned()), (2, "Risks".to_owned())]
    );
}

/// CommonMark allows a document to go straight from `#` to `###`. Both are still the trail.
#[test]
fn skipped_levels_keep_both_steps() {
    let source = "# Guide\n\n### Hosting\n\nthe bill\n";
    assert_eq!(
        trail_at(source, "the bill"),
        vec![(1, "Guide".to_owned()), (3, "Hosting".to_owned())]
    );
}

/// A `#` inside a fence is code, not a heading — which is the parser's judgement, and the reason
/// this is built on blocks rather than on scanning for lines starting with a hash.
#[test]
fn a_hash_inside_a_fence_is_not_a_heading() {
    let source = "# Guide\n\n## Setup\n\n```sh\n# not a heading\necho hi\n```\n";
    assert_eq!(
        trail_at(source, "echo hi"),
        vec![(1, "Guide".to_owned()), (2, "Setup".to_owned())]
    );
}

/// Setext headings carry their markers on the *next* line, so trimming the front of the block would
/// find nothing to trim and leave the underline in the breadcrumb.
#[test]
fn a_setext_heading_loses_its_underline() {
    let source = "Guide\n=====\n\nSubject\n-------\n\ntext\n";
    assert_eq!(
        trail_at(source, "\ntext\n"),
        vec![(1, "Guide".to_owned()), (2, "Subject".to_owned())]
    );
}

/// Inline syntax inside a heading is syntax, and a breadcrumb shows the words.
#[test]
fn heading_text_drops_inline_markers() {
    let source = "# The **hard** part\n\ntext\n";
    assert_eq!(
        trail_at(source, "\ntext\n"),
        vec![(1, "The hard part".to_owned())]
    );
}

#[test]
fn an_empty_document_has_no_trail() {
    let document = Document::new();
    assert!(document.heading_trail(ByteOffset::ZERO).is_empty());
}

/// Both count from one when shown, but they are offsets here — zero-based, like every other offset
/// in the codebase. The `+ 1` belongs to whatever displays them.
#[test]
fn line_and_column_count_characters() {
    let source = "first\nsécond line\n";
    let document = Document::from(source);

    let (line, column) = document.line_and_column(ByteOffset::ZERO);
    assert_eq!((line.get(), column.get()), (0, 0));

    // After "sé" — three bytes in, two characters in. A byte count here would say three.
    let at = ByteOffset::new(source.find("cond").unwrap());
    let (line, column) = document.line_and_column(at);
    assert_eq!((line.get(), column.get()), (1, 2));
}

#[test]
fn the_end_of_the_document_is_a_position_like_any_other() {
    let source = "one\ntwo\n";
    let document = Document::from(source);
    let (line, column) = document.line_and_column(ByteOffset::new(source.len()));
    // The trailing newline opens a third line, and the caret sits at its start.
    assert_eq!((line.get(), column.get()), (2, 0));
}
