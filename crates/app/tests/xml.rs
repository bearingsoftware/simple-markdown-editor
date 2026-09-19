//! Opening and editing a `.xml` file.
//!
//! The unit tests in `mdedit-xml` prove the lexer. These prove the **join**, which is where both of
//! MDEDIT-126's defects lived: the editor is driven end to end, a frame is built, and the dimension
//! this language differs on — it is the only one that refuses a region parse — is the one varied.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::path::Path;

use mdedit_app::detect::language_for;
use mdedit_app::editor::Editor;
use mdedit_app::view::Theme;
use mdedit_language::{SegmentKind, Token};
use mdedit_session::Workspace;
use tempfile::TempDir;

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = MARGIN + 400.0;

/// Opens `name` holding `contents`, through the workspace the app uses.
fn open(dir: &TempDir, name: &str, contents: &str) -> Editor<Grid> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor
}

/// Whether the document's segments still reproduce its bytes. The gate, asked of the editor rather
/// than of the lexer, because that is the half a unit test cannot see.
fn tiles(editor: &Editor<Grid>) -> bool {
    let document = editor.document();
    let text = document.to_string();
    let rebuilt: String = document
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    rebuilt == text
}

#[test]
fn an_xml_file_opens_as_xml() {
    assert_eq!(language_for(Path::new("pom.xml")).name(), "XML");
    assert_eq!(language_for(Path::new("POM.XML")).name(), "XML");
    // Only `.xml`, deliberately. Every other XML dialect is a line in `detect.rs` when it is wanted.
    assert_eq!(language_for(Path::new("icon.svg")).name(), "Plain Text");

    let dir = TempDir::new().unwrap();
    let editor = open(&dir, "pom.xml", "<project><name>x</name></project>");
    assert_eq!(editor.document().language().name(), "XML");
}

#[test]
fn nothing_in_an_xml_file_is_ever_concealed() {
    let dir = TempDir::new().unwrap();
    let source =
        "<?xml version=\"1.0\"?>\n<config debug=\"true\">\n  <entry>value</entry>\n</config>\n";
    let editor = open(&dir, "config.xml", source);

    let document = editor.document();
    assert!(
        !document.language().conceals(),
        "a tag is the document, not scaffolding for prose"
    );
    assert!(
        document
            .segments()
            .iter()
            .all(|segment| document.is_revealed(segment)),
        "every segment has to be drawn, wherever the caret is"
    );
    assert!(tiles(&editor));
}

#[test]
fn an_xml_file_is_lexed_into_coloured_tokens() {
    let dir = TempDir::new().unwrap();
    let editor = open(
        &dir,
        "config.xml",
        "<!-- note -->\n<a href=\"x\">text &amp; more</a>\n",
    );

    let kinds: Vec<Token> = editor
        .document()
        .segments()
        .iter()
        .filter_map(|segment| match segment.kind() {
            SegmentKind::Token(token) => Some(token),
            _ => None,
        })
        .collect();

    for wanted in [
        Token::Tag,
        Token::Key,
        Token::String,
        Token::Punctuation,
        Token::Comment,
        Token::Literal,
    ] {
        assert!(kinds.contains(&wanted), "expected {wanted:?} in {kinds:?}");
    }
}

/// **An element name and an attribute name must not draw the same**, which is the whole of what
/// `Token::Tag` was added for. A test that only checked the token would pass with the renderer
/// mapping both to one colour.
#[test]
fn a_tag_name_and_an_attribute_name_are_different_colours() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "config.xml", "<a href=\"x\"/>\n");
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let colours: std::collections::BTreeSet<[u32; 4]> = quads
        .iter()
        .map(|quad| quad.colour.map(f32::to_bits))
        .collect();
    let theme = Theme::dark();
    // Named rather than counted: a count passes with both roles mapped to one colour, since the
    // punctuation and the value supply the other two on their own.
    assert_ne!(theme.token_tag, theme.token_key);
    for (what, colour) in [
        ("the tag", theme.token_tag),
        ("the attribute", theme.token_key),
    ] {
        let wanted = [colour.r, colour.g, colour.b, colour.a].map(f32::to_bits);
        assert!(
            colours.contains(&wanted),
            "{what} did not reach the screen in its own colour: {colours:?}"
        );
    }
}

/// The state a file is in most of the time it is being written, and the reason a parser crate was
/// not used.
#[test]
fn a_half_typed_xml_file_still_draws() {
    let dir = TempDir::new().unwrap();
    for broken in [
        "<a",
        "<a href=",
        "<a href=\"",
        "<a><b></a>",
        "</a>",
        "<!-- unclosed",
        "<![CDATA[unclosed",
    ] {
        let mut editor = open(&dir, "config.xml", broken);
        let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
        assert!(!quads.is_empty(), "{broken:?} drew nothing");
        assert!(tiles(&editor), "{broken:?} stopped tiling");
    }
}

/// **XML is the only language that refuses a region parse**, so an edit takes the whole-document
/// path in `resegment_scope` that nothing had exercised before. Typing a tag out one character at a
/// time is what that path has to survive.
#[test]
fn typing_re_lexes_the_whole_document_correctly() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "config.xml", "<root>\n  \n</root>\n");
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::caret(
            mdedit_offsets::ByteOffset::new(9),
        ));

    for piece in ["<", "i", "t", "e", "m", "/", ">"] {
        editor.insert_text(piece);
        assert!(tiles(&editor), "stopped tiling after {piece:?}");
    }

    assert_eq!(
        editor.document().to_string(),
        "<root>\n  <item/>\n</root>\n"
    );
    let kinds: Vec<Token> = editor
        .document()
        .segments()
        .iter()
        .filter_map(|segment| match segment.kind() {
            SegmentKind::Token(token) => Some(token),
            _ => None,
        })
        .collect();
    assert!(
        kinds.iter().filter(|token| **token == Token::Tag).count() == 3,
        "root, item and the closing root: {kinds:?}"
    );
}

#[test]
fn opening_an_xml_file_does_not_touch_a_byte() {
    let dir = TempDir::new().unwrap();
    // Single quotes, a tab, trailing spaces and no trailing newline: everything a serializer tidies.
    let source = "<a  b='1'\tc=\"2\" >  text  </a>";
    let editor = open(&dir, "config.xml", source);
    assert_eq!(editor.document().to_string(), source);
}
