//! The keyboard-shortcuts reference, and whether it still describes the editor.
//!
//! A help page and the code it describes drift the moment nothing checks them against each other.
//! These are that check — imperfect, because nothing here can read `handle_chord`, but enough to
//! catch a chord that was added or renamed and never written down.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_app::shortcuts;

mod stub;
use stub::Grid;

/// Every chord the reference claims, flattened.
fn claimed() -> Vec<&'static str> {
    shortcuts::ALL
        .iter()
        .flat_map(|group| group.shortcuts.iter().map(|shortcut| shortcut.keys))
        .collect()
}

/// The chords `Editor::handle_chord` actually answers to, as this test knows them.
///
/// Kept by hand and deliberately so: the alternative is deriving the list from the match arms, which
/// nothing can do, or asserting nothing. A chord added to `handle_chord` and not to this list makes
/// no test fail — but one added here and not to the reference does, which is the direction the
/// drift actually goes.
const IMPLEMENTED: &[&str] = &[
    "Ctrl+N",
    "Ctrl+O",
    "Ctrl+S",
    "Ctrl+Shift+S",
    "Ctrl+W",
    "Ctrl+Z",
    "Ctrl+Y",
    "Ctrl+X",
    "Ctrl+C",
    "Ctrl+V",
    "Ctrl+A",
    "Ctrl+/",
    "Ctrl+B",
    "Ctrl+I",
    "Ctrl+Shift+X",
    "Ctrl+E",
    "Ctrl+Shift+E",
    "Ctrl+Tab",
];

#[test]
fn every_implemented_chord_is_written_down() {
    let claimed = claimed();
    let missing: Vec<_> = IMPLEMENTED
        .iter()
        .filter(|chord| !claimed.contains(*chord))
        .collect();
    assert!(
        missing.is_empty(),
        "these chords work but the reference does not mention them: {missing:?}"
    );
}

#[test]
fn the_document_is_markdown_with_a_table_per_group() {
    let document = shortcuts::document();
    assert!(document.starts_with("# Keyboard Shortcuts\n"));
    for group in shortcuts::ALL {
        assert!(
            document.contains(&format!("## {}", group.name)),
            "no heading for {}",
            group.name
        );
    }
    assert_eq!(
        document.matches("| Keys | Does |").count(),
        shortcuts::ALL.len()
    );
}

#[test]
fn showing_it_opens_a_tab_with_the_reference_in_it() {
    let mut editor = Editor::new("", Grid::new());
    assert!(editor.show_shortcuts());

    assert_eq!(
        editor.workspace().tabs()[0].display_name(),
        shortcuts::TITLE
    );
    assert!(editor.document().to_string().contains("Ctrl+O"));
}

/// The wart this exists to avoid: a generated reference that asks to be saved on the way out is a
/// nuisance rather than a safeguard, so it carries its own text as its record of disk.
#[test]
fn the_reference_does_not_count_as_unsaved_work() {
    let mut editor = Editor::new("", Grid::new());
    editor.show_shortcuts();

    assert!(!editor.is_modified());
    assert!(!editor.workspace().has_unsaved_changes());
}

/// A menu item people click twice must not leave two of them open.
#[test]
fn showing_it_twice_reuses_the_tab() {
    let mut editor = Editor::new("", Grid::new());
    editor.show_shortcuts();
    editor.show_shortcuts();

    assert_eq!(editor.workspace().tabs().len(), 1);
}

/// It is content, not a handle, so the session file has nothing to say about it — and a stale copy
/// of a reference document restored next launch would be worse than regenerating it.
#[test]
fn it_is_not_written_to_the_session() {
    let dir = tempfile::TempDir::new().unwrap();
    let session = dir.path().join("session.json");

    let mut editor = Editor::new("", Grid::new());
    editor.show_shortcuts();
    editor.save_session(&session);

    let written = std::fs::read_to_string(&session).unwrap();
    assert!(
        !written.contains("Keyboard"),
        "the reference should not be in the session file: {written}"
    );
}
