//! The YAML lexer.
//!
//! Tiling is the gate, checked hardest against the documents a parser would reject — those are the
//! ones this exists to survive, and the reason a real YAML parser was not used. See the crate docs
//! for that evidence.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_language::{Language, SegmentKind, Token};
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_yaml::{Yaml, analyze};
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
fn a_mapping_lexes_into_keys_and_values() {
    let source = "name: value\ncount: 42\nenabled: true\nempty: null\n";
    assert!(has(source, Token::Key, "name"));
    assert!(has(source, Token::Number, "42"));
    assert!(has(source, Token::Literal, "true"));
    assert!(has(source, Token::Literal, "null"));
    assert!(tiles(source));
}

#[test]
fn comments_are_comments_and_a_hash_in_a_value_is_not() {
    let source =
        "# a whole-line note\nkey: value # trailing note\nurl: http://example.com/#anchor\n";
    assert!(has(source, Token::Comment, "# a whole-line note"));
    assert!(has(source, Token::Comment, "# trailing note"));
    assert!(
        !tokens(source)
            .iter()
            .any(|(token, text)| *token == Token::Comment && text.contains("anchor")),
        "a `#` with no space before it is part of the value: {:?}",
        tokens(source)
    );
    assert!(tiles(source));
}

#[test]
fn sequences_and_nesting_keep_their_dashes() {
    let source = "list:\n  - one\n  - two\nnested:\n  - - deep\n";
    assert!(has(source, Token::Punctuation, "-"));
    assert!(has(source, Token::Key, "list"));
    assert!(tiles(source));
}

#[test]
fn document_markers_are_marked() {
    let source = "---\na: 1\n...\n---\nb: 2\n";
    assert!(has(source, Token::Punctuation, "---"));
    assert!(has(source, Token::Punctuation, "..."));
    assert!(tiles(source));
}

#[test]
fn anchors_aliases_and_tags_read_as_names() {
    let source = "base: &anchor\n  a: 1\nuse: *anchor\ntagged: !!str 5\n";
    assert!(has(source, Token::Literal, "&anchor"));
    assert!(has(source, Token::Literal, "*anchor"));
    assert!(has(source, Token::Literal, "!!str"));
    assert!(tiles(source));
}

/// The case a line-oriented lexer gets wrong if it is careless: inside a `|` block, a `#` is text
/// and a `key:` is text. Only the indentation says where the block ends.
#[test]
fn a_block_scalar_holds_text_not_syntax() {
    let source = "script: |\n  # not a comment\n  key: not a key\nafter: 1\n";
    let lexed = tokens(source);

    assert!(
        !lexed
            .iter()
            .any(|(token, text)| *token == Token::Comment && text.contains("not a comment")),
        "a hash inside a block scalar is text: {lexed:?}"
    );
    assert!(
        !lexed
            .iter()
            .any(|(token, text)| *token == Token::Key && text.contains("key")),
        "and so is a colon: {lexed:?}"
    );
    assert!(
        lexed.contains(&(Token::Key, "after".to_owned())),
        "but the block ends when the indent does: {lexed:?}"
    );
    assert!(tiles(source));
}

#[test]
fn a_folded_block_with_chomping_is_also_a_block() {
    for indicator in ["|", "|-", "|+", ">", ">-", ">2"] {
        let source = format!("s: {indicator}\n  # text\nafter: 1\n");
        assert!(tiles(&source), "{indicator} did not tile");
        assert!(
            has(&source, Token::Key, "after"),
            "{indicator} swallowed the line after the block"
        );
    }
}

#[test]
fn quoted_scalars_survive_their_own_punctuation() {
    let source = "a: \"has: a colon\"\nb: 'and: another'\nc: \"escaped \\\" quote\"\n";
    assert!(
        has(source, Token::Key, "a")
            && has(source, Token::Key, "b")
            && has(source, Token::Key, "c"),
        "a colon inside quotes must not end the key: {:?}",
        tokens(source)
    );
    assert!(tiles(source));
}

#[test]
fn flow_collections_lex_inside() {
    let source = "list: [1, 2, three]\nmap: {a: 1, b: two}\n";
    assert!(has(source, Token::Punctuation, "["));
    assert!(has(source, Token::Punctuation, "{"));
    assert!(has(source, Token::Number, "1"));
    assert!(tiles(source));
}

/// The three documents `saphyr-parser` refused, which is why it is not being used.
#[test]
fn what_a_real_parser_rejects_still_lexes() {
    for rejected in [
        "a: 1\nb:\n  - x\n  - \nc: [1, 2",
        "a:\n   b: 1\n  c: 2\n",
        "a:\n\tb: 1\n",
    ] {
        assert!(tiles(rejected), "{rejected:?} did not tile");
    }
}

#[test]
fn a_document_being_typed_still_lexes() {
    for partial in [
        "", "a", "a:", "a: ", "a: [", "a: [1,", "a: \"", "a: '", "- ", "-", "&", "!", "|", "#",
        "---",
    ] {
        assert!(tiles(partial), "{partial:?} did not tile");
    }
}

#[test]
fn multi_byte_characters_are_never_split() {
    let source = "café: 🙂\nkey: \"naïve — value\"\n- 🙂\n";
    for segment in &analyze(source).segments {
        assert!(
            source.is_char_boundary(segment.span().start().get())
                && source.is_char_boundary(segment.span().end().get()),
            "{segment:?} does not sit on character boundaries"
        );
    }
    assert!(tiles(source));
}

#[test]
fn top_level_entries_become_blocks() {
    let source = "first: 1\nsecond:\n  nested: 2\n  also: 3\nthird: 4\n";
    let blocks = analyze(source).blocks;
    assert_eq!(blocks.len(), 3, "one per column-zero entry: {blocks:?}");
    assert!(
        blocks[1].span().slice(source).contains("nested"),
        "an entry owns the lines indented under it"
    );
}

#[test]
fn yaml_never_conceals() {
    assert!(!Yaml.conceals());
    assert_eq!(Yaml.name(), "YAML");
}

/// MDEDIT-36. A column-zero line is provably outside every block scalar — a block scalar's content
/// must be indented further than the line that opened it, and that line is itself at column zero —
/// so a region starting at one needs no state carried into it.
#[test]
fn a_region_starting_at_column_zero_is_outside_every_block_scalar() {
    let source = "script: |\n  # not a comment\n  key: not a key\nafter: 1\n";
    let whole = analyze(source);

    // The second top-level entry, re-lexed on its own.
    let second = whole.blocks[1].span();
    let region = mdedit_yaml::analyze_region(source, second);

    assert!(
        region
            .segments
            .iter()
            .any(|segment| segment.kind() == SegmentKind::Token(Token::Key)),
        "`after: 1` should lex as a key, not as the tail of the block scalar above it"
    );

    let rebuilt: String = region
        .segments
        .iter()
        .map(|segment| segment.span().slice(source))
        .collect();
    assert_eq!(rebuilt, second.slice(source), "and it tiles its own region");
}

/// The case the contract exists to exclude: starting *inside* a block scalar would read its text as
/// syntax. Pinned so that anyone loosening `scope_for` sees what it would cost.
#[test]
fn a_region_starting_inside_a_block_scalar_would_be_wrong() {
    let source = "script: |\n  # not a comment\nafter: 1\n";
    let inside = source.find("  # not").unwrap();
    let region = mdedit_yaml::analyze_region(
        source,
        ByteSpan::new(ByteOffset::new(inside), ByteOffset::new(source.len())),
    );

    assert!(
        region
            .segments
            .iter()
            .any(|segment| segment.kind() == SegmentKind::Token(Token::Comment)),
        "which is why a caller may only ever start a region at a block boundary"
    );
}

proptest! {
    /// The gate, over arbitrary input.
    #[test]
    fn segments_tile_any_input(source in ".*") {
        prop_assert!(tiles(&source));
    }

    /// And over YAML-shaped input, valid or otherwise.
    #[test]
    fn segments_tile_yaml_shaped_input(
        source in proptest::collection::vec(
            prop_oneof![
                Just("key:".to_owned()), Just("- ".to_owned()),
                Just(" ".to_owned()), Just("  ".to_owned()),
                Just("\n".to_owned()), Just("\t".to_owned()),
                Just("#".to_owned()), Just("---".to_owned()),
                Just("|".to_owned()), Just(">-".to_owned()),
                Just("&a".to_owned()), Just("*a".to_owned()), Just("!!str".to_owned()),
                Just("\"".to_owned()), Just("'".to_owned()),
                Just("[".to_owned()), Just("]".to_owned()),
                Just("{".to_owned()), Just("}".to_owned()), Just(",".to_owned()),
                Just("1.2".to_owned()), Just("true".to_owned()), Just("é".to_owned()),
            ],
            0..50,
        ).prop_map(|parts| parts.concat())
    ) {
        prop_assert!(tiles(&source));
    }

    #[test]
    fn spans_land_on_character_boundaries(source in ".*") {
        for segment in &analyze(&source).segments {
            prop_assert!(source.is_char_boundary(segment.span().start().get()));
            prop_assert!(source.is_char_boundary(segment.span().end().get()));
        }
    }
}
