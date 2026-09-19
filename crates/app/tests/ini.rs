//! Opening and editing a `.ini` or `.cfg` file.
//!
//! The unit tests in `mdedit-ini` prove the lexer. These prove the **join**: the editor driven end to
//! end, a frame built, and the dialect decisions checked where a user would see them.

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

/// **`.cfg` is a guess and is written down as one.** Pinned here so removing the mapping is a
/// deliberate act rather than an edit nobody notices.
#[test]
fn both_extensions_open_as_ini() {
    assert_eq!(language_for(Path::new("app.ini")).name(), "INI");
    assert_eq!(language_for(Path::new("setup.cfg")).name(), "INI");
    assert_eq!(language_for(Path::new("APP.INI")).name(), "INI");

    let dir = TempDir::new().unwrap();
    let editor = open(&dir, "setup.cfg", "[metadata]\nname = thing\n");
    assert_eq!(editor.document().language().name(), "INI");
}

#[test]
fn nothing_in_an_ini_file_is_ever_concealed() {
    let dir = TempDir::new().unwrap();
    let source = "; a note\n[server]\nhost = localhost\nport = 8080\n";
    let editor = open(&dir, "app.ini", source);

    let document = editor.document();
    assert!(!document.language().conceals());
    assert!(
        document
            .segments()
            .iter()
            .all(|segment| document.is_revealed(segment)),
        "every byte of an INI file is on screen"
    );
    assert!(tiles(&editor));
}

#[test]
fn an_ini_file_is_lexed_into_coloured_tokens() {
    let dir = TempDir::new().unwrap();
    let editor = open(
        &dir,
        "app.ini",
        "; note\n[server]\nhost = \"local\"\nport = 8080\nup = yes\n",
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
fn an_ini_file_draws_in_more_than_one_colour() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "app.ini", "; note\n[server]\nport = 8080\nup = yes\n");
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let colours: std::collections::BTreeSet<[u32; 4]> = quads
        .iter()
        .map(|quad| quad.colour.map(f32::to_bits))
        .collect();
    assert!(colours.len() > 2, "{colours:?}");
}

#[test]
fn a_half_typed_ini_file_still_draws() {
    let dir = TempDir::new().unwrap();
    for broken in ["[serv", "key", "key =", "= 1", "]", "; "] {
        let mut editor = open(&dir, "app.ini", broken);
        let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
        assert!(!quads.is_empty(), "{broken:?} drew nothing");
        assert!(tiles(&editor), "{broken:?} stopped tiling");
    }
}

/// The scoped path, which a unit test cannot reach: typing inside a section re-lexes a region, and
/// what it produces has to agree with a whole-document parse.
#[test]
fn typing_agrees_with_a_whole_document_parse() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "app.ini", "[a]\nx = 1\n[b]\ny = 2\n");
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::caret(
            mdedit_offsets::ByteOffset::new(9),
        ));

    for piece in ["2", "3", "\nz = yes", "\n    more"] {
        editor.insert_text(piece);
        assert!(tiles(&editor), "stopped tiling after {piece:?}");

        let text = editor.document().to_string();
        assert_eq!(
            editor.document().segments().to_vec(),
            mdedit_ini::analyze(&text).segments,
            "the scoped re-lex disagreed with a whole-document one after {piece:?}"
        );
    }
}

#[test]
fn opening_an_ini_file_does_not_touch_a_byte() {
    let dir = TempDir::new().unwrap();
    // Alignment padding, a tab, a Windows path with a semicolon in it, and no trailing newline.
    let source = "[a]\nlong_key   = 1\nk\t: two\npath = C:\\a;C:\\b";
    let editor = open(&dir, "app.ini", source);
    assert_eq!(editor.document().to_string(), source);
}
