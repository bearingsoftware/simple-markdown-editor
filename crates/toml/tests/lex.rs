//! The TOML lexer.
//!
//! Tiling is the gate. The named cases are the ones a deserializer would refuse — a half-typed table
//! header, an unclosed array, a string that never ends — because those are what this exists for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_language::{Language, SegmentKind, Token};
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_toml::{Toml, analyze, analyze_region};
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

#[test]
fn a_table_and_its_entries_lex_into_keys_and_values() {
    let source = "[server]\nhost = \"localhost\"\nport = 8080\nenabled = true\n";
    assert!(has(source, Token::Key, "server"));
    assert!(has(source, Token::Key, "host"));
    assert!(has(source, Token::String, "\"localhost\""));
    assert!(has(source, Token::Number, "8080"));
    assert!(has(source, Token::Literal, "true"));
    assert!(tiles(source));
}

#[test]
fn an_array_table_keeps_both_brackets() {
    let source = "[[products]]\nname = \"hammer\"\n";
    assert!(has(source, Token::Punctuation, "[["));
    assert!(has(source, Token::Punctuation, "]]"));
    assert!(has(source, Token::Key, "products"));
    assert!(tiles(source));
}

#[test]
fn a_dotted_key_is_names_and_separators() {
    let source = "a.b.c = 1\n";
    assert_eq!(
        tokens(source)
            .iter()
            .filter(|(token, _)| *token == Token::Key)
            .count(),
        3,
        "{:?}",
        tokens(source)
    );
    assert!(has(source, Token::Punctuation, "."));
    assert!(tiles(source));
}

#[test]
fn a_quoted_key_is_a_key_rather_than_a_string() {
    let source = "[tool.\"my crate\"]\n\"odd key\" = 1\n";
    assert!(has(source, Token::Key, "\"my crate\""));
    assert!(has(source, Token::Key, "\"odd key\""));
    assert!(tiles(source));
}

#[test]
fn comments_are_comments_and_a_hash_in_a_string_is_not() {
    let source = "# a note\nkey = \"value\" # trailing\nurl = \"http://x/#anchor\"\n";
    assert!(has(source, Token::Comment, "# a note"));
    assert!(has(source, Token::Comment, "# trailing"));
    assert!(has(source, Token::String, "\"http://x/#anchor\""));
    assert!(tiles(source));
}

#[test]
fn a_multi_line_string_swallows_the_hash_inside_it() {
    let source = "text = \"\"\"\nnot # a comment\n[not a table]\n\"\"\"\nafter = 1\n";
    assert!(
        !tokens(source)
            .iter()
            .any(|(token, text)| *token == Token::Comment && text.contains("a comment")),
        "{:?}",
        tokens(source)
    );
    assert!(
        has(source, Token::Key, "after"),
        "and the document resumes after it closes: {:?}",
        tokens(source)
    );
    assert!(tiles(source));
}

#[test]
fn a_multi_line_literal_string_does_the_same() {
    let source = "text = '''\nnot # a comment\n'''\nafter = 1\n";
    assert!(has(source, Token::Key, "after"));
    assert!(tiles(source));
}

#[test]
fn a_multi_line_string_that_never_closes_does_not_lose_bytes() {
    let source = "text = \"\"\"\nstill typing\nand typing\n";
    assert!(tiles(source));
}

#[test]
fn a_float_is_one_number_rather_than_two_and_a_dot() {
    let source = "pi = 3.14\n";
    assert!(has(source, Token::Number, "3.14"), "{:?}", tokens(source));
    assert!(tiles(source));
}

#[test]
fn a_date_is_a_number() {
    let source = "when = 1979-05-27T07:32:00Z\n";
    assert!(
        has(source, Token::Number, "1979-05-27T07:32:00Z"),
        "{:?}",
        tokens(source)
    );
    assert!(tiles(source));
}

#[test]
fn integers_in_every_base_are_numbers() {
    for text in ["0xDEADBEEF", "0o755", "0b1010", "1_000_000", "-17", "+99"] {
        let source = format!("n = {text}\n");
        assert!(has(&source, Token::Number, text), "{:?}", tokens(&source));
    }
}

#[test]
fn an_inline_table_names_its_own_keys() {
    let source = "point = { x = 1, y = 2 }\n";
    assert!(has(source, Token::Key, "x"), "{:?}", tokens(source));
    assert!(has(source, Token::Key, "y"));
    assert!(has(source, Token::Number, "1"));
    assert!(tiles(source));
}

#[test]
fn an_array_spanning_lines_stays_inside_itself() {
    // The continuation lines are not entries, so `two` is a string rather than a key.
    let source = "list = [\n  \"one\",\n  \"two\",\n]\nafter = 1\n";
    assert!(has(source, Token::String, "\"one\""));
    assert!(has(source, Token::String, "\"two\""));
    assert!(has(source, Token::Key, "after"));
    assert!(tiles(source));
}

#[test]
fn a_half_typed_table_header_still_colours_what_is_there() {
    let source = "[serv";
    assert!(has(source, Token::Key, "serv"), "{:?}", tokens(source));
    assert!(tiles(source));
}

#[test]
fn a_line_with_no_separator_yet_does_not_lose_bytes() {
    for source in ["key", "key ", "= 1", "\"unclosed", "'"] {
        assert!(tiles(source), "{source:?}");
    }
}

#[test]
fn a_block_is_a_column_zero_line_and_what_hangs_under_it() {
    let source = "[server]\nhost = \"x\"\nlist = [\n  1,\n]\n";
    let analysis = analyze(source);
    let spans: Vec<_> = analysis
        .blocks
        .iter()
        .map(|block| block.span().slice(source))
        .collect();
    assert_eq!(
        spans,
        vec!["[server]\n", "host = \"x\"\n", "list = [\n  1,\n]\n"],
        "an open array keeps its continuation lines in one block"
    );
}

/// The contract `analyze_region` is safe under: a block start is outside every multi-line string
/// and has no array open, which is exactly what a fresh lexer assumes.
#[test]
fn a_region_starting_at_a_block_agrees_with_the_whole_document() {
    let source = "[a]\nx = 1\n[b]\ny = \"two\"\n";
    let start = source.find("[b]").unwrap();
    let region = analyze_region(
        source,
        ByteSpan::new(ByteOffset::new(start), ByteOffset::new(source.len())),
    );

    let whole = analyze(source);
    let tail: Vec<_> = whole
        .segments
        .iter()
        .filter(|segment| segment.span().start().get() >= start)
        .cloned()
        .collect();
    assert_eq!(region.segments, tail);
}

/// The case the contract exists to exclude, pinned so anyone loosening `scope_for` sees the cost.
#[test]
fn a_region_starting_inside_a_multi_line_string_would_be_wrong() {
    let source = "text = \"\"\"\n[not a table]\n\"\"\"\n";
    let inside = source.find("[not").unwrap();
    let region = analyze_region(
        source,
        ByteSpan::new(ByteOffset::new(inside), ByteOffset::new(source.len())),
    );
    assert!(
        region
            .segments
            .iter()
            .any(|segment| segment.kind() == SegmentKind::Token(Token::Key)),
        "which is why a caller may only ever start a region at a block boundary"
    );
}

#[test]
fn toml_conceals_nothing() {
    assert!(!Toml.conceals());
    assert_eq!(Toml.name(), "TOML");
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

    /// And over TOML-shaped input, valid or otherwise.
    #[test]
    fn segments_tile_toml_shaped_input(
        source in proptest::collection::vec(
            prop_oneof![
                Just("[a]".to_owned()), Just("[[a]]".to_owned()),
                Just("key".to_owned()), Just(" = ".to_owned()), Just("=".to_owned()),
                Just(".".to_owned()), Just(",".to_owned()),
                Just("[".to_owned()), Just("]".to_owned()),
                Just("{".to_owned()), Just("}".to_owned()),
                Just("\"".to_owned()), Just("'".to_owned()),
                Just("\"\"\"".to_owned()), Just("'''".to_owned()),
                Just("#".to_owned()), Just(" ".to_owned()), Just("\n".to_owned()),
                Just("\t".to_owned()), Just("\\".to_owned()),
                Just("1979-05-27".to_owned()), Just("3.14".to_owned()),
                Just("true".to_owned()), Just("inf".to_owned()), Just("é".to_owned()),
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

    /// A region starting at any block agrees with the whole-document parse over the same bytes.
    /// This is the property `analyze_region`'s contract is worth having, checked rather than argued.
    #[test]
    fn any_block_is_a_safe_restart_point(
        source in proptest::collection::vec(
            prop_oneof![
                Just("[a]\n".to_owned()), Just("k = 1\n".to_owned()),
                Just("s = \"\"\"\nx\n\"\"\"\n".to_owned()),
                Just("l = [\n1,\n]\n".to_owned()),
                Just("# c\n".to_owned()), Just("\n".to_owned()),
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
