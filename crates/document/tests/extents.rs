//! Word and line extents — what a double-click and a triple-click select.
//!
//! The interesting cases are not English. An alphanumeric-run classifier passes every ASCII test
//! here and fails half the rest, which is the whole argument for Unicode segmentation rather than a
//! hand-rolled one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
use mdedit_offsets::ByteOffset;

/// What `word_at` selects at `at`, as text.
fn word(text: &str, at: usize) -> String {
    let doc = Document::from(text);
    doc.word_at(ByteOffset::new(at)).slice(text).to_owned()
}

/// What `source_line` selects at `at`, as text.
fn line(text: &str, at: usize) -> String {
    let doc = Document::from(text);
    doc.source_line(ByteOffset::new(at)).slice(text).to_owned()
}

#[test]
fn a_click_inside_a_word_takes_the_whole_word() {
    assert_eq!(word("hello world\n", 2), "hello");
    assert_eq!(word("hello world\n", 8), "world");
}

/// The convention everywhere: a double-click never selects nothing.
#[test]
fn whitespace_and_punctuation_are_words_too() {
    assert_eq!(word("a    b\n", 3), "    ");
    assert_eq!(word("a---b\n", 2), "---");
}

/// The case an alphanumeric classifier gets wrong when the accent is a combining mark: it would cut
/// the word at the é.
#[test]
fn a_combining_accent_does_not_split_a_word() {
    // "cafe" + U+0301 COMBINING ACUTE ACCENT.
    let text = "cafe\u{301} au lait\n";
    assert_eq!(word(text, 1), "cafe\u{301}");
}

/// A precomposed accent, which is the same word spelled differently.
#[test]
fn a_precomposed_accent_does_not_split_a_word_either() {
    assert_eq!(word("café au lait\n", 1), "café");
}

/// The case an alphanumeric classifier gets most wrong: it would treat a whole CJK run as one word,
/// where segmentation breaks it where the language does.
#[test]
fn cjk_breaks_into_words_rather_than_one_run() {
    let text = "汉字编辑器\n";
    let selected = word(text, 0);
    assert!(!selected.is_empty());
    assert!(
        selected.len() < text.len() - 1,
        "the whole run was taken as one word: {selected:?}"
    );
}

#[test]
fn an_apostrophe_stays_inside_its_word() {
    assert_eq!(word("don't stop\n", 2), "don't");
}

/// A click exactly on a boundary belongs to the word on its left, which is what puts the caret at
/// the end of a word rather than the start of the next.
#[test]
fn a_click_on_a_boundary_takes_the_word_to_its_left() {
    // Offset 5 is the space between "hello" and "world".
    assert_eq!(word("hello world\n", 5), " ");
}

#[test]
fn a_click_past_the_end_of_a_line_selects_nothing() {
    assert_eq!(word("hi\n", 3), "");
}

#[test]
fn an_empty_document_has_an_empty_word() {
    assert_eq!(word("", 0), "");
}

/// Positions past the buffer are clamped rather than panicking: they arrive from a hit test against
/// a layout that may be a frame behind the buffer.
#[test]
fn a_position_past_the_end_is_clamped() {
    assert_eq!(word("hi\n", 999), "");
    assert_eq!(line("hi\n", 999), "");
}

#[test]
fn a_line_includes_its_trailing_newline() {
    assert_eq!(line("one\ntwo\n", 1), "one\n");
    assert_eq!(line("one\ntwo\n", 5), "two\n");
}

/// The last line of a file with no final newline has none to include.
#[test]
fn the_last_line_without_a_newline_ends_at_the_buffer() {
    assert_eq!(line("one\ntwo", 5), "two");
}

/// Source line, not visual row. A long paragraph is one line however many rows it draws — which is
/// the thing that makes triple-click select a paragraph.
#[test]
fn a_long_paragraph_is_one_line() {
    let long = format!("{}\nnext\n", "word ".repeat(200));
    let doc = Document::from(long.as_str());
    let selected = doc.source_line(ByteOffset::new(500)).slice(&long);
    assert_eq!(
        selected.len(),
        1001,
        "the whole paragraph, plus its newline"
    );
}

#[test]
fn a_blank_line_is_just_its_newline() {
    assert_eq!(line("one\n\ntwo\n", 4), "\n");
}
