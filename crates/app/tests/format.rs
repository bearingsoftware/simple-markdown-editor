//! Format Document in the editor: what it does, what it refuses, and the undo step it has to be.
//!
//! The formatter itself is tested in `mdedit-json`. What is asserted here is the wiring and the one
//! rule that outranks the feature — **one press of Ctrl+Z puts every byte back**. This is the only
//! command in the editor allowed to rewrite the whole buffer, so the undo behind it is not a
//! convenience, it is the thing that makes the rewrite permissible at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use mdedit_app::editor::Editor;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::Grid;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// What the editor tried to tell the user.
#[derive(Debug, Default, Clone)]
struct Said(Rc<RefCell<Vec<String>>>);

impl Said {
    fn last(&self) -> Option<String> {
        self.0.borrow().last().cloned()
    }

    fn count(&self) -> usize {
        self.0.borrow().len()
    }
}

fn open(dir: &TempDir, name: &str, contents: &str) -> (Editor<Grid>, Said) {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

    let said = Said::default();
    let sink = said.clone();
    editor.with_messages(Box::new(move |message| {
        sink.0.borrow_mut().push(message.to_owned());
    }));
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    (editor, said)
}

// ---- what it does ----

#[test]
fn a_messy_json_document_is_formatted() {
    let dir = TempDir::new().unwrap();
    let (mut editor, said) = open(&dir, "data.json", r#"{"a":1,"b":[2,3]}"#);

    assert!(editor.apply_command(mdedit_menu_win::Command::Format));

    assert_eq!(
        editor.document().to_string(),
        "{\n  \"a\": 1,\n  \"b\": [\n    2,\n    3\n  ]\n}\n"
    );
    assert_eq!(said.count(), 0, "a success says nothing");
}

/// **The rule that makes the rewrite permissible.** One press, every byte back.
#[test]
fn one_undo_puts_every_byte_back() {
    let dir = TempDir::new().unwrap();
    let source = "{\"a\":1,\"b\":[2,3]}\n";
    let (mut editor, _) = open(&dir, "data.json", source);

    assert!(editor.apply_command(mdedit_menu_win::Command::Format));
    assert_ne!(editor.document().to_string(), source);

    assert!(editor.document_mut().undo());
    assert_eq!(editor.document().to_string(), source);
    assert!(
        !editor.document().can_undo(),
        "formatting a whole document should have been one step, not one per line"
    );
}

/// Typing afterwards must not join it, or undoing the character you just typed would also undo the
/// reformat.
#[test]
fn typing_afterwards_is_its_own_step() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "data.json", r#"{"a":1}"#);

    assert!(editor.apply_command(mdedit_menu_win::Command::Format));
    let formatted = editor.document().to_string();
    editor.insert_text(" ");

    assert!(editor.document_mut().undo());
    assert_eq!(editor.document().to_string(), formatted);
}

/// An already-formatted document is left completely alone — no edit, so no undo press that appears
/// to do nothing, and no modified dot on a tab nobody changed.
#[test]
fn formatting_an_already_formatted_document_does_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "data.json", "{\n  \"a\": 1\n}\n");

    assert!(!editor.apply_command(mdedit_menu_win::Command::Format));
    assert!(
        !editor.document().can_undo(),
        "no edit, so no undo press that appears to do nothing"
    );
}

// ---- what it refuses ----

/// The gate. A document part way through being typed is not "fixed" by formatting it.
#[test]
fn a_malformed_document_is_refused_and_left_alone() {
    let dir = TempDir::new().unwrap();
    let source = r#"{"a": 1,}"#;
    let (mut editor, said) = open(&dir, "data.json", source);

    assert!(!editor.apply_command(mdedit_menu_win::Command::Format));

    assert_eq!(editor.document().to_string(), source, "nothing may change");
    assert!(!editor.document().can_undo());
    let message = said.last().expect("the user should have been told why");
    assert!(message.contains("trailing comma"), "got {message}");
    assert!(message.contains("line 1"), "got {message}");
}

/// The message names a place a person can go to, which is why it is line and column rather than a
/// byte offset.
#[test]
fn the_refusal_names_the_line_and_column() {
    let dir = TempDir::new().unwrap();
    let (mut editor, said) = open(&dir, "data.json", "{\n  \"a\": 1,\n}\n");

    assert!(!editor.apply_command(mdedit_menu_win::Command::Format));
    let message = said.last().unwrap();
    // `  "a": 1,` — the comma is the ninth character, counting from one as the status bar does.
    assert!(message.contains("line 2"), "got {message}");
    assert!(message.contains("column 9"), "got {message}");
}

// ---- which languages ----

/// The per-language decision, asserted rather than left in a comment. Markdown has no canonical
/// form and this editor's architecture exists to avoid inventing one.
#[test]
fn markdown_does_not_format_and_says_so_by_being_greyed() {
    let dir = TempDir::new().unwrap();
    let (mut editor, said) = open(&dir, "notes.md", "#  A heading\n\n\n\nSome    prose.\n");

    assert!(!editor.can_format());
    assert!(!editor.apply_command(mdedit_menu_win::Command::Format));

    assert_eq!(
        editor.document().to_string(),
        "#  A heading\n\n\n\nSome    prose.\n",
        "Markdown must come back untouched"
    );
    assert_eq!(
        said.count(),
        0,
        "a greyed item explains itself; a message would be explaining twice"
    );
}

/// YAML likewise, and for a different reason: round-tripping it loses comments and anchors.
#[test]
fn yaml_does_not_format() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "config.yaml", "a:   1 # keep me\n");

    assert!(!editor.can_format());
    assert!(!editor.apply_command(mdedit_menu_win::Command::Format));
    assert_eq!(editor.document().to_string(), "a:   1 # keep me\n");
}

#[test]
fn json_does_format() {
    let dir = TempDir::new().unwrap();
    let (editor, _) = open(&dir, "data.json", "{}");
    assert!(editor.can_format());
}

/// **Enablement follows the tab, and is compared rather than hooked.** A menu item that only knew
/// about the paths somebody remembered to update would be wrong on the tenth one.
#[test]
fn enablement_follows_the_active_tab() {
    let dir = TempDir::new().unwrap();
    let json = dir.path().join("data.json");
    let markdown = dir.path().join("notes.md");
    std::fs::write(&json, "{}").unwrap();
    std::fs::write(&markdown, "# Notes\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(json);
    workspace.add(markdown);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.show_tab(0);
    assert!(editor.can_format(), "JSON formats");

    editor.show_tab(1);
    assert!(!editor.can_format(), "Markdown does not");
}
