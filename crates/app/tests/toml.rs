//! Opening and editing a `.toml` file.
//!
//! The unit tests in `mdedit-toml` prove the lexer. These prove the **join**: the editor is driven
//! end to end, a frame is built, and the dimension this language differs on — it re-parses a region
//! rather than the document — is the one varied.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::path::Path;

use mdedit_app::detect::language_for;
use mdedit_app::editor::Editor;
use mdedit_language::{SegmentKind, Token};
use mdedit_session::Workspace;
use tempfile::TempDir;

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn open(dir: &TempDir, name: &str, contents: &str) -> Editor<Grid> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor
}

/// Whether the document's segments still reproduce its bytes — the gate, asked of the editor.
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
fn a_toml_file_opens_as_toml() {
    assert_eq!(language_for(Path::new("Cargo.toml")).name(), "TOML");
    assert_eq!(language_for(Path::new("CONFIG.TOML")).name(), "TOML");

    let dir = TempDir::new().unwrap();
    let editor = open(&dir, "Cargo.toml", "[package]\nname = \"x\"\n");
    assert_eq!(editor.document().language().name(), "TOML");
}

#[test]
fn nothing_in_a_toml_file_is_ever_concealed() {
    let dir = TempDir::new().unwrap();
    let source = "# a note\n[server]\nhost = \"localhost\"\nports = [80, 443]\n";
    let editor = open(&dir, "config.toml", source);

    let document = editor.document();
    assert!(!document.language().conceals());
    assert!(
        document
            .segments()
            .iter()
            .all(|segment| document.is_revealed(segment)),
        "every byte of a TOML file is on screen"
    );
    assert!(tiles(&editor));
}

#[test]
fn a_toml_file_is_lexed_into_coloured_tokens() {
    let dir = TempDir::new().unwrap();
    let editor = open(
        &dir,
        "config.toml",
        "# note\n[server]\nhost = \"x\"\nport = 8080\nup = true\n",
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
        Token::Key,
        Token::String,
        Token::Number,
        Token::Literal,
        Token::Punctuation,
        Token::Comment,
    ] {
        assert!(kinds.contains(&wanted), "expected {wanted:?} in {kinds:?}");
    }
}

#[test]
fn a_toml_file_draws_in_more_than_one_colour() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(
        &dir,
        "config.toml",
        "[server]\nhost = \"x\"\nport = 8080\nup = true\n",
    );
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let colours: std::collections::BTreeSet<[u32; 4]> = quads
        .iter()
        .map(|quad| quad.colour.map(f32::to_bits))
        .collect();
    assert!(colours.len() > 2, "{colours:?}");
}

/// The documents a deserializer refuses, which is why one is not being used.
#[test]
fn a_half_typed_toml_file_still_draws() {
    let dir = TempDir::new().unwrap();
    for broken in [
        "[serv",
        "key = ",
        "key = \"",
        "key = \"\"\"\nstill typing",
        "list = [1,",
        "= 1",
        "[[a]",
    ] {
        let mut editor = open(&dir, "config.toml", broken);
        let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
        assert!(!quads.is_empty(), "{broken:?} drew nothing");
        assert!(tiles(&editor), "{broken:?} stopped tiling");
    }
}

/// **The region parse is the half a unit test cannot see.** Typing inside a table takes
/// `resegment_scope`'s scoped path, and what it produces has to agree with a whole-document parse —
/// including when the keystroke is the one that opens a multi-line string.
#[test]
fn typing_agrees_with_a_whole_document_parse() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "config.toml", "[a]\nx = 1\n[b]\ny = 2\n");
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::caret(
            mdedit_offsets::ByteOffset::new(9),
        ));

    for piece in ["2", "3", "\nz = \"\"", "\"", "\""] {
        editor.insert_text(piece);
        assert!(tiles(&editor), "stopped tiling after {piece:?}");

        let text = editor.document().to_string();
        let scoped: Vec<_> = editor.document().segments().to_vec();
        assert_eq!(
            scoped,
            mdedit_toml::analyze(&text).segments,
            "the scoped re-lex disagreed with a whole-document one after {piece:?}"
        );
    }
}

#[test]
fn opening_a_toml_file_does_not_touch_a_byte() {
    let dir = TempDir::new().unwrap();
    // Alignment padding, a tab, single quotes and no trailing newline: everything a formatter tidies.
    let source = "[a]\nlong_key   = 1\nk\t= 'lit'\narr = [ 1,2 ]";
    let editor = open(&dir, "config.toml", source);
    assert_eq!(editor.document().to_string(), source);
}
