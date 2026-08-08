//! Opening a `.json` file.
//!
//! Two claims. That the extension decides the language and nothing else does — no byte of the file
//! is looked at, because content sniffing is the trust boundary `AGENTS.md` names the 2023 Notepad++
//! CVEs for. And that a `.md` file is completely unaffected, which is the acceptance criterion that
//! matters most: a second language must not cost the first one anything.

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

mod stub;
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

#[test]
fn the_extension_decides_and_nothing_else_does() {
    assert_eq!(language_for(Path::new("data.json")).name(), "JSON");
    assert_eq!(language_for(Path::new("notes.md")).name(), "Markdown");
    // Case-insensitively, because Windows filesystems are.
    assert_eq!(language_for(Path::new("DATA.JSON")).name(), "JSON");
    // No extension, an unknown one, and a dotfile all fall back rather than guessing.
    assert_eq!(language_for(Path::new("README")).name(), "Markdown");
    assert_eq!(language_for(Path::new("notes.txt")).name(), "Markdown");
    assert_eq!(language_for(Path::new(".gitignore")).name(), "Markdown");
}

/// The deliberate cost of not sniffing: a JSON file under the wrong name opens as Markdown. Better
/// than a parser reachable by a crafted file, and fixed by a language picker when someone wants one.
#[test]
fn a_json_file_named_txt_is_not_json() {
    assert_eq!(
        language_for(Path::new("actually-json.txt")).name(),
        "Markdown"
    );
}

#[test]
fn a_json_file_opens_as_json() {
    let dir = TempDir::new().unwrap();
    let editor = open(&dir, "data.json", r#"{"a": 1}"#);
    assert_eq!(editor.document().language().name(), "JSON");
}

#[test]
fn a_markdown_file_still_opens_as_markdown() {
    let dir = TempDir::new().unwrap();
    let editor = open(&dir, "notes.md", "# Title\n\ntext\n");
    assert_eq!(editor.document().language().name(), "Markdown");
}

/// The acceptance criterion: every byte on screen, nothing concealed, wherever the caret is.
#[test]
fn nothing_in_a_json_file_is_ever_concealed() {
    let dir = TempDir::new().unwrap();
    let source = "{\n  \"key\": \"value\",\n  \"n\": 42\n}\n";
    let editor = open(&dir, "data.json", source);

    let document = editor.document();
    assert!(
        !document.language().conceals(),
        "JSON's braces are the document"
    );
    assert!(
        document
            .segments()
            .iter()
            .all(|segment| document.is_revealed(segment)),
        "every segment has to be drawn, whatever the caret is near"
    );

    // And the segments still cover the file exactly, so "every byte" is literal.
    let text = document.to_string();
    let rebuilt: String = document
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, source);
}

#[test]
fn a_json_file_is_lexed_into_coloured_tokens() {
    let dir = TempDir::new().unwrap();
    let editor = open(&dir, "data.json", r#"{"key": "value", "n": 42}"#);

    let kinds: Vec<Token> = editor
        .document()
        .segments()
        .iter()
        .filter_map(|segment| match segment.kind() {
            SegmentKind::Token(token) => Some(token),
            _ => None,
        })
        .collect();

    for wanted in [Token::Key, Token::String, Token::Number, Token::Punctuation] {
        assert!(kinds.contains(&wanted), "expected {wanted:?} in {kinds:?}");
    }
}

/// A frame has to come out of it, and the tokens have to reach the screen in more than one colour —
/// colour is the entire decoration for a language that conceals nothing.
#[test]
fn a_json_file_draws_in_more_than_one_colour() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(
        &dir,
        "data.json",
        "{\n  \"key\": \"value\",\n  \"n\": 42\n}\n",
    );
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let colours: std::collections::BTreeSet<[u32; 4]> = quads
        .iter()
        .map(|quad| quad.colour.map(f32::to_bits))
        .collect();
    assert!(
        colours.len() > 2,
        "keys, strings, numbers and punctuation should not all be one colour: {colours:?}"
    );
}

/// The state a file is in most of the time it is being written.
#[test]
fn a_half_typed_json_file_still_draws() {
    let dir = TempDir::new().unwrap();
    for broken in ["{", r#"{"a":"#, r#"{"a": [1,"#, r#"{"unterminated"#] {
        let mut editor = open(&dir, "data.json", broken);
        let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
        assert!(!quads.is_empty(), "{broken:?} drew nothing");
    }
}

/// Byte preservation, which matters more here rather than less — nobody forgives an editor that
/// reformats their JSON.
#[test]
fn opening_a_json_file_does_not_touch_a_byte() {
    let dir = TempDir::new().unwrap();
    // Deliberately not how a formatter would write it: odd whitespace, a trailing newline, and
    // numbers whose formatting a round trip through any parser would change.
    let source = "{ \"a\":1.50,   \"b\":[ ],\n\t\"c\":1e3 }\n";
    let editor = open(&dir, "data.json", source);
    assert_eq!(editor.document().to_string(), source);
}

// ---------------------------------------------------------------------------
// YAML, which is the same bargain with a larger grammar. See `mdedit_yaml`.
// ---------------------------------------------------------------------------

#[test]
fn both_yaml_extensions_are_recognised() {
    assert_eq!(language_for(Path::new("compose.yaml")).name(), "YAML");
    assert_eq!(language_for(Path::new("compose.yml")).name(), "YAML");
    assert_eq!(language_for(Path::new("CI.YML")).name(), "YAML");
}

#[test]
fn a_yaml_file_opens_highlighted_and_never_concealed() {
    let dir = TempDir::new().unwrap();
    let source = "# a note\nname: value\nlist:\n  - 1\n  - two\n";
    let editor = open(&dir, "config.yaml", source);

    let document = editor.document();
    assert_eq!(document.language().name(), "YAML");
    assert!(!document.language().conceals());
    assert!(
        document
            .segments()
            .iter()
            .all(|segment| document.is_revealed(segment)),
        "every byte of a YAML file is on screen"
    );

    let text = document.to_string();
    let rebuilt: String = document
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, source);
}

#[test]
fn a_yaml_file_draws_in_more_than_one_colour() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(
        &dir,
        "config.yaml",
        "# note\nname: value\ncount: 42\nflag: true\n",
    );
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let colours: std::collections::BTreeSet<[u32; 4]> = quads
        .iter()
        .map(|quad| quad.colour.map(f32::to_bits))
        .collect();
    assert!(colours.len() > 2, "{colours:?}");
}

/// The documents a real YAML parser refused, which is why one is not being used.
#[test]
fn a_yaml_file_a_parser_would_reject_still_draws() {
    let dir = TempDir::new().unwrap();
    for broken in [
        "a: 1\nb:\n  - x\n  - \nc: [1, 2",
        "a:\n   b: 1\n  c: 2\n",
        "a:\n\tb: 1\n",
    ] {
        let mut editor = open(&dir, "config.yaml", broken);
        let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
        assert!(!quads.is_empty(), "{broken:?} drew nothing");
    }
}

#[test]
fn opening_a_yaml_file_does_not_touch_a_byte() {
    let dir = TempDir::new().unwrap();
    // Trailing spaces, tabs in a value, an odd quote style and no trailing newline: everything a
    // round trip through a YAML library would tidy up.
    let source = "a:   1  \nb: 'quoted'\nc: |\n  literal\ttext\nd: [ 1,2 ]";
    let editor = open(&dir, "config.yml", source);
    assert_eq!(editor.document().to_string(), source);
}
