//! The XML lexer.
//!
//! Tiling is the gate, and it is checked hardest against the documents a real parser would reject —
//! those are the ones this exists to survive, and the reason a parser crate was not used. See the
//! crate docs for that argument.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_language::{Language, SegmentKind, Token};
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_xml::{Xml, analyze};
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
fn an_element_name_is_a_tag_and_an_attribute_name_is_a_key() {
    let source = "<config debug=\"true\">text</config>";
    assert!(has(source, Token::Tag, "config"));
    assert!(has(source, Token::Key, "debug"));
    assert!(has(source, Token::String, "\"true\""));
    assert!(
        content(source).contains(&"text".to_owned()),
        "text between tags is content: {:?}",
        content(source)
    );
    assert!(tiles(source));
}

#[test]
fn the_name_in_a_closing_tag_is_a_tag_too() {
    let source = "<a></a>";
    let names: Vec<_> = tokens(source)
        .into_iter()
        .filter(|(token, _)| *token == Token::Tag)
        .collect();
    assert_eq!(names.len(), 2, "both names are tags: {names:?}");
    assert!(tiles(source));
}

#[test]
fn a_self_closing_tag_does_not_open_a_level() {
    // If `<b/>` opened a level, `<c>` would be its child rather than the root's, and only one block
    // would come back instead of three.
    let source = "<root><a/><b/><c/></root>";
    assert_eq!(analyze(source).blocks.len(), 3);
    assert!(tiles(source));
}

#[test]
fn comments_are_comments_and_the_markup_in_them_is_not() {
    let source = "<!-- <a href=\"x\"> --><b/>";
    assert!(has(source, Token::Comment, "<!-- <a href=\"x\"> -->"));
    assert!(has(source, Token::Tag, "b"));
    assert!(tiles(source));
}

#[test]
fn an_unterminated_comment_runs_to_the_end_rather_than_losing_bytes() {
    let source = "<a/><!-- still typing";
    assert!(has(source, Token::Comment, "<!-- still typing"));
    assert!(tiles(source));
}

#[test]
fn a_cdata_body_is_text_however_much_it_looks_like_markup() {
    let source = "<a><![CDATA[<b>&amp;</b>]]></a>";
    assert!(
        content(source).contains(&"<b>&amp;</b>".to_owned()),
        "the body is literal text: {:?}",
        content(source)
    );
    assert!(
        !has(source, Token::Tag, "b"),
        "nothing inside a CDATA section is a tag"
    );
    assert!(tiles(source));
}

#[test]
fn a_processing_instruction_lexes_its_pseudo_attributes() {
    let source = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a/>";
    assert!(has(source, Token::Tag, "xml"));
    assert!(has(source, Token::Key, "version"));
    assert!(has(source, Token::String, "\"1.0\""));
    assert!(has(source, Token::Tag, "a"), "and the document after it");
    assert!(tiles(source));
}

#[test]
fn a_doctype_ends_at_the_bracket_that_closes_its_internal_subset() {
    // The `>` inside the subset does not end the declaration, so `<html>` after it is still an
    // element rather than the tail of one.
    let source = "<!DOCTYPE html [<!ENTITY x \"y\">]>\n<html/>";
    assert!(has(source, Token::Tag, "DOCTYPE"));
    assert!(has(source, Token::Tag, "html"), "{:?}", tokens(source));
    assert!(tiles(source));
}

#[test]
fn entities_are_literals_in_text_and_in_attribute_values() {
    let source = "<a title=\"one &amp; two\">three &#169; four</a>";
    assert!(has(source, Token::Literal, "&amp;"));
    assert!(has(source, Token::Literal, "&#169;"));
    assert!(tiles(source));
}

#[test]
fn a_bare_ampersand_is_text_rather_than_the_start_of_a_scan() {
    let source = "<a>fish & chips & more</a>";
    assert!(
        !tokens(source)
            .iter()
            .any(|(token, _)| *token == Token::Literal),
        "nothing here is an entity: {:?}",
        tokens(source)
    );
    assert!(tiles(source));
}

#[test]
fn a_stray_angle_bracket_is_text() {
    let source = "<a>2 < 3 and 4 > 1</a>";
    assert!(
        content(source).iter().any(|text| text.contains('<')),
        "a `<` that opens nothing is text: {:?}",
        content(source)
    );
    assert!(tiles(source));
}

#[test]
fn an_unclosed_tag_does_not_swallow_the_one_after_it() {
    // What the file looks like the moment before the `>` is typed.
    let source = "<a href=\"x\"\n<b/>";
    assert!(has(source, Token::Tag, "b"), "{:?}", tokens(source));
    assert!(tiles(source));
}

#[test]
fn an_unclosed_attribute_value_ends_at_the_line() {
    // Without this the rest of the file turns into a string behind the caret.
    let source = "<a href=\"x\n<b/>";
    assert!(has(source, Token::Tag, "b"), "{:?}", tokens(source));
    assert!(tiles(source));
}

#[test]
fn an_unquoted_attribute_value_is_still_a_value() {
    let source = "<a href=x class=y/>";
    assert!(has(source, Token::String, "x"));
    assert!(has(source, Token::Key, "href"));
    assert!(has(source, Token::Key, "class"));
    assert!(tiles(source));
}

#[test]
fn a_depth_bomb_does_not_overflow() {
    // A file from the internet, opened in an editor. Recursive descent dies here; a counter does
    // not. The assertion is that this test finishes at all — a stack overflow aborts the process.
    let depth = 200_000;
    let source: String = "<a>".repeat(depth);
    let analysis = analyze(&source);
    assert_eq!(
        analysis.segments.len(),
        depth * 2 + 1,
        "a name each, and one punctuation run per `><` boundary plus the two ends"
    );
    assert!(tiles(&source));
}

#[test]
fn more_end_tags_than_start_tags_does_not_underflow() {
    let source = "</a></a></a><b/>";
    assert!(has(source, Token::Tag, "b"));
    assert!(tiles(source));
}

#[test]
fn a_block_is_a_child_of_the_root() {
    let source = "<root>\n  <item>one</item>\n  <item>two</item>\n</root>\n";
    let analysis = analyze(source);
    let spans: Vec<_> = analysis
        .blocks
        .iter()
        .map(|block| block.span().slice(source))
        .collect();
    assert_eq!(spans, vec!["<item>one</item>", "<item>two</item>"]);
    assert!(analysis.blocks.iter().all(|block| block.depth() == 0));
}

#[test]
fn a_grandchild_is_not_a_block() {
    let source = "<root><item><deep>x</deep></item></root>";
    let analysis = analyze(source);
    assert_eq!(analysis.blocks.len(), 1);
    assert_eq!(
        analysis.blocks[0].span().slice(source),
        "<item><deep>x</deep></item>"
    );
}

#[test]
fn a_region_is_refused_because_there_is_no_safe_place_to_restart() {
    let source = "<root><item>one</item></root>";
    assert!(
        Xml.analyze_region(source, ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(6)))
            .is_none(),
        "depth is the whole of the state and no offset carries it"
    );
}

#[test]
fn xml_conceals_nothing() {
    assert!(!Xml.conceals());
    assert_eq!(Xml.name(), "XML");
    assert!(
        !analyze("<a>text</a>")
            .segments
            .iter()
            .any(mdedit_language::Segment::is_marker),
        "a tag is the document, not scaffolding for prose"
    );
}

#[test]
fn an_empty_document_produces_nothing_rather_than_an_empty_span() {
    let analysis = analyze("");
    assert!(analysis.segments.is_empty());
    assert!(analysis.blocks.is_empty());
}

proptest! {
    /// The gate, over arbitrary input.
    #[test]
    fn segments_tile_any_input(source in "(?s).*") {
        prop_assert!(tiles(&source));
    }

    /// And over XML-shaped input, well-formed or otherwise — which is the half that matters, since
    /// well-formed is the state a file being edited is briefly in.
    #[test]
    fn segments_tile_xml_shaped_input(
        source in proptest::collection::vec(
            prop_oneof![
                Just("<a".to_owned()), Just("</a".to_owned()), Just("<".to_owned()),
                Just(">".to_owned()), Just("/>".to_owned()), Just("/".to_owned()),
                Just("<!--".to_owned()), Just("-->".to_owned()),
                Just("<![CDATA[".to_owned()), Just("]]>".to_owned()),
                Just("<?xml".to_owned()), Just("?>".to_owned()),
                Just("<!DOCTYPE".to_owned()), Just("[".to_owned()), Just("]".to_owned()),
                Just(" ".to_owned()), Just("\n".to_owned()), Just("\t".to_owned()),
                Just("=".to_owned()), Just("\"".to_owned()), Just("'".to_owned()),
                Just("&amp;".to_owned()), Just("&".to_owned()), Just(";".to_owned()),
                Just("text".to_owned()), Just("é".to_owned()), Just("日本".to_owned()),
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

    /// Blocks are inside the source and never inverted, whatever the nesting did.
    #[test]
    fn blocks_are_well_formed(source in "(?s).*") {
        for block in &analyze(&source).blocks {
            prop_assert!(block.span().start().get() < block.span().end().get());
            prop_assert!(block.span().end().get() <= source.len());
        }
    }
}
