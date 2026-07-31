//! The JSON lexer.
//!
//! The gate is tiling, and it is checked against **invalid** documents as hard as against valid
//! ones — a file being written is broken most of the time, and that is the case the editor actually
//! lives in. A lexer that tiles only well-formed input would drop bytes from the screen at exactly
//! the moment somebody is looking at them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_json::{Json, analyze};
use mdedit_language::{Language, SegmentKind, Token};
use mdedit_offsets::{ByteOffset, ByteSpan};
use proptest::prelude::*;

/// Concatenating the segments must reproduce the source.
fn tiles(source: &str) -> bool {
    let analysis = analyze(source);
    let rebuilt: String = analysis
        .segments
        .iter()
        .map(|segment| segment.span().slice(source))
        .collect();
    rebuilt == source
}

/// The token classes in order, for asserting what a document lexed as.
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

#[test]
fn an_object_lexes_into_its_parts() {
    let source = r#"{"name": "value", "n": 42, "ok": true, "nope": null}"#;
    let lexed = tokens(source);

    assert!(lexed.contains(&(Token::Key, r#""name""#.to_owned())));
    assert!(lexed.contains(&(Token::String, r#""value""#.to_owned())));
    assert!(lexed.contains(&(Token::Number, "42".to_owned())));
    assert!(lexed.contains(&(Token::Literal, "true".to_owned())));
    assert!(lexed.contains(&(Token::Literal, "null".to_owned())));
    assert!(tiles(source));
}

/// A key and a string value are the same syntax and different things, which is why they are
/// coloured differently by every editor. The difference is what follows the closing quote.
#[test]
fn a_key_is_a_string_followed_by_a_colon() {
    let lexed = tokens(r#"{"a": "a"}"#);
    let strings: Vec<Token> = lexed.iter().map(|(token, _)| *token).collect();
    assert!(
        strings.contains(&Token::Key) && strings.contains(&Token::String),
        "the same text either side of the colon should lex differently: {lexed:?}"
    );

    // Whitespace and a newline between the string and its colon do not stop it being a key.
    assert!(
        tokens("{\"a\"\n  : 1}")
            .iter()
            .any(|(token, _)| *token == Token::Key)
    );
}

#[test]
fn an_escaped_quote_does_not_end_a_string() {
    let source = r#"{"a": "he said \"hi\"", "b": 1}"#;
    let lexed = tokens(source);
    assert!(
        lexed.contains(&(Token::String, r#""he said \"hi\"""#.to_owned())),
        "the string should run past the escaped quotes: {lexed:?}"
    );
    assert!(
        lexed.contains(&(Token::Key, r#""b""#.to_owned())),
        "and what follows should still lex as itself"
    );
    assert!(tiles(source));
}

/// The state every document passes through on its way to being written.
#[test]
fn a_document_being_typed_still_lexes() {
    for partial in [
        "{",
        "{\"",
        "{\"a",
        "{\"a\"",
        "{\"a\":",
        "{\"a\": ",
        "{\"a\": 1",
        "{\"a\": 1,",
        "[1, 2,",
        "tru",
        "-",
        "1.",
        "\"unterminated",
    ] {
        assert!(tiles(partial), "{partial:?} did not tile");
    }
}

/// Not an error, deliberately: `1.2.3` is what `1.23` looks like one keystroke before it is done.
#[test]
fn a_malformed_number_is_still_a_number() {
    assert!(tokens("[1.2.3]").contains(&(Token::Number, "1.2.3".to_owned())));
    assert!(tiles("[1.2.3]"));
}

/// Bytes that are not JSON at all are still covered. Dropping them would take them off the screen.
#[test]
fn nonsense_keeps_its_bytes() {
    for rubbish in ["<html>", "\u{0}\u{1}", "}{", "@@@", "café", "🙂"] {
        assert!(tiles(rubbish), "{rubbish:?} did not tile");
    }
}

/// A multi-byte character split down the middle produces a span that is not on a character
/// boundary, and `ByteSpan::slice` panics on one — from a render, on a file merely opened.
#[test]
fn multi_byte_characters_are_never_split() {
    let source = "{\"emoji\": \"🙂🙂\", \"accents\": \"café\", 🙂: 1}";
    let analysis = analyze(source);
    for segment in &analysis.segments {
        assert!(
            source.is_char_boundary(segment.span().start().get())
                && source.is_char_boundary(segment.span().end().get()),
            "{segment:?} does not sit on character boundaries"
        );
    }
    assert!(tiles(source));
}

#[test]
fn top_level_members_become_blocks() {
    let source = "{\n  \"a\": 1,\n  \"b\": [2, 3],\n  \"c\": {\"d\": 4}\n}";
    let blocks = analyze(source).blocks;
    assert_eq!(
        blocks.len(),
        3,
        "one block per top-level member: {blocks:?}"
    );

    // Each block covers its own member and no more — a block that swallowed the comma or the next
    // member would make the layout cache key wrong for both.
    for block in &blocks {
        let text = block.span().slice(source);
        assert!(!text.contains(",\n"), "{text:?} runs past its member");
        assert!(text.starts_with('"'), "{text:?} should start at its key");
    }
}

#[test]
fn json_never_conceals() {
    assert!(!Json.conceals(), "JSON's braces are the document");
    assert_eq!(Json.name(), "JSON");
}

/// MDEDIT-36. A region starting at a member boundary is one container deep, and a region starting at
/// byte zero is none — which is the whole of the state a JSON lexer cannot recover from a slice.
#[test]
fn a_region_lexes_at_the_depth_its_start_implies() {
    let source = r#"{"a": 1, "b": {"c": 2}, "d": 3}"#;
    let whole = analyze(source);

    // Every top-level member, re-lexed on its own from its own boundary.
    for block in &whole.blocks {
        let region = mdedit_json::analyze_region(source, block.span());
        assert_eq!(
            region.blocks.len(),
            1,
            "{:?} should lex as exactly the one member it is",
            block.span().slice(source)
        );
        assert_eq!(region.blocks[0].span(), block.span());
    }

    // And from byte zero it is the whole document again.
    let all = mdedit_json::analyze_region(
        source,
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
    );
    assert_eq!(all.blocks.len(), whole.blocks.len());
}

/// A region's spans come back in the source's coordinates, or a splice would put them in the wrong
/// place — silently, since they would still be a valid tiling of something.
#[test]
fn a_regions_spans_are_in_the_documents_coordinates() {
    let source = r#"{"a": 1, "b": 2}"#;
    let whole = analyze(source);
    let second = whole.blocks[1].span();

    let region = mdedit_json::analyze_region(source, second);
    assert_eq!(region.segments[0].span().start(), second.start());
    let rebuilt: String = region
        .segments
        .iter()
        .map(|segment| segment.span().slice(source))
        .collect();
    assert_eq!(rebuilt, second.slice(source), "and it tiles its own region");
}

#[test]
fn an_empty_document_is_not_a_special_case() {
    assert!(tiles(""));
    assert!(analyze("").segments.is_empty());
    assert!(analyze("").blocks.is_empty());
}

proptest! {
    /// The gate, over arbitrary bytes: whatever it is handed, the segments reproduce it.
    #[test]
    fn segments_tile_any_input(source in ".*") {
        prop_assert!(tiles(&source));
    }

    /// And over things shaped like JSON, valid or otherwise — a generator of only well-formed
    /// documents would never produce the half-typed states this has to survive.
    #[test]
    fn segments_tile_json_shaped_input(
        source in proptest::collection::vec(
            prop_oneof![
                Just("{".to_owned()), Just("}".to_owned()),
                Just("[".to_owned()), Just("]".to_owned()),
                Just(",".to_owned()), Just(":".to_owned()),
                Just("\"a\"".to_owned()), Just("\"".to_owned()),
                Just("1".to_owned()), Just("-1.2e3".to_owned()),
                Just("true".to_owned()), Just("nul".to_owned()),
                Just(" ".to_owned()), Just("\n".to_owned()),
                Just("\\".to_owned()), Just("é".to_owned()),
            ],
            0..40,
        ).prop_map(|parts| parts.concat())
    ) {
        prop_assert!(tiles(&source));
    }

    /// Every span sits on a character boundary, whatever it was handed.
    #[test]
    fn spans_land_on_character_boundaries(source in ".*") {
        for segment in &analyze(&source).segments {
            prop_assert!(source.is_char_boundary(segment.span().start().get()));
            prop_assert!(source.is_char_boundary(segment.span().end().get()));
        }
    }
}
