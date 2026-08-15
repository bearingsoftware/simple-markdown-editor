//! The INI lexer.
//!
//! Tiling is the gate. The named cases are the dialect decisions the crate docs state — which
//! comment characters, which separators, what a value is — because those are choices rather than
//! facts about a specification, and a choice nobody pinned is a choice somebody changes by accident.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_ini::{Ini, analyze, analyze_region};
use mdedit_language::{Language, SegmentKind, Token};
use mdedit_offsets::{ByteOffset, ByteSpan};
use proptest::prelude::*;

fn tiles(source: &str) -> bool {
    let rebuilt: String = analyze(source)
        .segments
        .iter()
        .map(|segment| segment.span().slice(source))
        .collect();
    rebuilt == source
}

fn tokens(source: &str) -> Vec<(Token, String)> {
    analyze(source)
        .segments
        .iter()
        .filter_map(|segment| match segment.kind() {
            SegmentKind::Token(token) => Some((token, segment.span().slice(source).to_owned())),
            _ => None,
        })
        .collect()
}

fn has(source: &str, token: Token, text: &str) -> bool {
    tokens(source).contains(&(token, text.to_owned()))
}

fn content(source: &str) -> Vec<String> {
    analyze(source)
        .segments
        .iter()
        .filter(|segment| segment.kind() == SegmentKind::Content)
        .map(|segment| segment.span().slice(source).to_owned())
        .collect()
}

#[test]
fn a_section_and_its_entries_lex_into_keys_and_values() {
    let source = "[server]\nhost = localhost\nport = 8080\nup = yes\n";
    assert!(has(source, Token::Key, "server"));
    assert!(has(source, Token::Key, "host"));
    assert!(has(source, Token::Number, "8080"));
    assert!(has(source, Token::Literal, "yes"));
    assert!(tiles(source));
}

#[test]
fn both_separators_work_and_the_first_one_wins() {
    let source = "a = 1\nb: 2\n";
    assert!(has(source, Token::Punctuation, "="));
    assert!(has(source, Token::Punctuation, ":"));
    assert!(has(source, Token::Key, "a"));
    assert!(has(source, Token::Key, "b"));
    assert!(tiles(source));
}

#[test]
fn both_comment_characters_work() {
    let source = "; a note\n# another note\nkey = value\n";
    assert!(has(source, Token::Comment, "; a note"));
    assert!(has(source, Token::Comment, "# another note"));
    assert!(tiles(source));
}

/// **The dialect decision that loses bytes if it goes the other way.** A mid-line `;` is part of the
/// value, because `C:\a;C:\b` is a path list and not a value with a comment after it.
#[test]
fn a_semicolon_inside_a_value_is_not_a_comment() {
    let source = "path = C:\\a;C:\\b\nnote = text # not a comment\n";
    assert!(
        !tokens(source)
            .iter()
            .any(|(token, _)| *token == Token::Comment),
        "nothing here is a comment: {:?}",
        tokens(source)
    );
    assert!(tiles(source));
}

/// The case the ticket named: the first separator is the only one.
#[test]
fn a_value_may_contain_an_equals_sign() {
    let source = "query = a=b&c=d\n";
    assert_eq!(
        tokens(source)
            .iter()
            .filter(|(token, text)| *token == Token::Punctuation && text == "=")
            .count(),
        1,
        "{:?}",
        tokens(source)
    );
    assert!(
        content(source).iter().any(|text| text.contains("a=b&c=d")),
        "the value is one uninterrupted run: {:?}",
        content(source)
    );
    assert!(tiles(source));
}

#[test]
fn a_quoted_value_is_a_string() {
    let source = "greeting = \"hello there\"\nother = 'single'\n";
    assert!(has(source, Token::String, "\"hello there\""));
    assert!(has(source, Token::String, "'single'"));
    assert!(tiles(source));
}

#[test]
fn a_continuation_line_is_part_of_the_value_rather_than_a_key() {
    let source = "paths =\n    /one\n    /two\nnext = 1\n";
    assert!(
        !tokens(source)
            .iter()
            .any(|(token, text)| *token == Token::Key && text.contains('/')),
        "an indented line with no separator is not a key: {:?}",
        tokens(source)
    );
    assert!(has(source, Token::Key, "next"));
    assert!(tiles(source));
}

#[test]
fn a_section_header_that_never_closes_still_colours_its_name() {
    let source = "[server\nhost = x\n";
    assert!(has(source, Token::Key, "server"), "{:?}", tokens(source));
    assert!(has(source, Token::Key, "host"));
    assert!(tiles(source));
}

/// Legal and common — a `.properties` file, or a `.cfg` with no sections at all.
#[test]
fn a_file_with_no_sections_is_still_lexed() {
    let source = "name = value\ncount = 3\n";
    assert!(has(source, Token::Key, "name"));
    assert!(has(source, Token::Number, "3"));
    assert_eq!(analyze(source).blocks.len(), 2);
    assert!(tiles(source));
}

#[test]
fn an_empty_value_is_not_a_token() {
    let source = "key =\nother=\n";
    assert!(
        !tokens(source)
            .iter()
            .any(|(token, text)| *token == Token::String && text.is_empty()),
    );
    assert!(tiles(source));
}

#[test]
fn a_block_is_a_column_zero_line_and_the_indented_lines_under_it() {
    let source = "[a]\npaths =\n    /one\n[b]\nx = 1\n";
    let analysis = analyze(source);
    let spans: Vec<_> = analysis
        .blocks
        .iter()
        .map(|block| block.span().slice(source))
        .collect();
    assert_eq!(
        spans,
        vec!["[a]\n", "paths =\n    /one\n", "[b]\n", "x = 1\n"]
    );
}

#[test]
fn a_region_starting_at_a_block_agrees_with_the_whole_document() {
    let source = "[a]\nx = 1\n[b]\ny = two\n";
    let start = source.find("[b]").unwrap();
    let region = analyze_region(
        source,
        ByteSpan::new(ByteOffset::new(start), ByteOffset::new(source.len())),
    );
    let tail: Vec<_> = analyze(source)
        .segments
        .into_iter()
        .filter(|segment| segment.span().start().get() >= start)
        .collect();
    assert_eq!(region.segments, tail);
}

#[test]
fn ini_conceals_nothing() {
    assert!(!Ini.conceals());
    assert_eq!(Ini.name(), "INI");
    assert!(
        !analyze("a = 1\n")
            .segments
            .iter()
            .any(mdedit_language::Segment::is_marker)
    );
}

proptest! {
    /// The gate, over arbitrary input.
    #[test]
    fn segments_tile_any_input(source in "(?s).*") {
        prop_assert!(tiles(&source));
    }

    /// And over INI-shaped input, valid or otherwise.
    #[test]
    fn segments_tile_ini_shaped_input(
        source in proptest::collection::vec(
            prop_oneof![
                Just("[a]".to_owned()), Just("[".to_owned()), Just("]".to_owned()),
                Just("key".to_owned()), Just("=".to_owned()), Just(":".to_owned()),
                Just(";".to_owned()), Just("#".to_owned()),
                Just(" ".to_owned()), Just("    ".to_owned()), Just("\t".to_owned()),
                Just("\n".to_owned()), Just("\r\n".to_owned()),
                Just("\"".to_owned()), Just("'".to_owned()),
                Just("1.2".to_owned()), Just("yes".to_owned()), Just("é".to_owned()),
            ],
            0..60,
        ).prop_map(|parts| parts.concat())
    ) {
        prop_assert!(tiles(&source));
    }

    #[test]
    fn spans_land_on_character_boundaries(source in "(?s).*") {
        for segment in &analyze(&source).segments {
            prop_assert!(source.is_char_boundary(segment.span().start().get()));
            prop_assert!(source.is_char_boundary(segment.span().end().get()));
        }
    }

    /// Every block is a safe restart point, checked rather than argued.
    #[test]
    fn any_block_is_a_safe_restart_point(
        source in proptest::collection::vec(
            prop_oneof![
                Just("[a]\n".to_owned()), Just("k = 1\n".to_owned()),
                Just("k:\n    more\n".to_owned()), Just("; c\n".to_owned()),
                Just("\n".to_owned()), Just("    orphan\n".to_owned()),
            ],
            0..12,
        ).prop_map(|parts| parts.concat())
    ) {
        let whole = analyze(&source);
        for block in &whole.blocks {
            let start = block.span().start().get();
            let region = analyze_region(
                &source,
                ByteSpan::new(ByteOffset::new(start), ByteOffset::new(source.len())),
            );
            let tail: Vec<_> = whole
                .segments
                .iter()
                .filter(|segment| segment.span().start().get() >= start)
                .cloned()
                .collect();
            prop_assert_eq!(region.segments, tail, "restarting at {}", start);
        }
    }
}
