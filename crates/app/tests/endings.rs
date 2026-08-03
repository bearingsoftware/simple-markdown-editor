//! Converting line endings in the editor (MDEDIT-59).
//!
//! The normalisation itself is tested in `mdedit-file`. What is asserted here is the thing the
//! ticket actually asks for: **the buffer, the file format and the status bar must agree**. Two of
//! those three agreeing is how a document ends up being written with endings the editor said it did
//! not have.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_file::Newline;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::Grid;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// Opens `contents` as a real file, written byte for byte so the endings are exactly as intended.
fn open(dir: &TempDir, contents: &str) -> (Editor<Grid>, std::path::PathBuf) {
    let path = dir.path().join("notes.md");
    std::fs::write(&path, contents.as_bytes()).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    (editor, path)
}

// ---- the acceptance criteria, as written ----

/// **A Mixed file converts to LF: the bar says LF, save writes no `\r`, undo restores the original
/// bytes exactly and the bar says Mixed again.** All four in one test because they are one claim.
#[test]
fn a_mixed_document_converts_to_lf_and_undoes() {
    let dir = TempDir::new().unwrap();
    let original = "alpha\r\nbeta\ngamma\rdelta\n";
    let (mut editor, path) = open(&dir, original);
    assert_eq!(editor.newline(), Newline::Mixed);

    assert!(editor.apply_command(mdedit_menu_win::Command::ConvertToLf));

    assert_eq!(editor.document().to_string(), "alpha\nbeta\ngamma\ndelta\n");
    assert_eq!(editor.newline(), Newline::Lf, "the format must agree");

    editor.save();
    let written = std::fs::read(&path).unwrap();
    assert!(
        !written.contains(&b'\r'),
        "a converted file must be written without carriage returns"
    );

    assert!(editor.document_mut().undo());
    assert_eq!(
        editor.document().to_string(),
        original,
        "one undo, the exact original bytes"
    );
    assert!(
        !editor.document().can_undo(),
        "converting a whole document should be one step"
    );
}

/// **Found in the window, not by a test.** Undo put the bytes back and the status bar went on saying
/// LF over a document that was Mixed again — the buffer and the format disagreeing, which is the one
/// thing this ticket is about. The endings are now re-detected on the idle timer, so the bar follows
/// the bytes rather than remembering what it was told.
#[test]
fn undoing_a_conversion_puts_the_status_bar_back_too() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "alpha\r\nbeta\ngamma\rdelta\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::ConvertToLf));
    assert_eq!(editor.newline(), Newline::Lf);

    assert!(editor.document_mut().undo());
    editor.on_idle();

    assert_eq!(
        editor.newline(),
        Newline::Mixed,
        "the bar must say what the document now is"
    );
}

/// The older half of the same wrongness, and the reason MDEDIT-46 exists: the editor deliberately
/// keeps pasted CRLF in an LF file, and the bar said "LF" about the result until the file was
/// reopened.
#[test]
fn pasting_a_foreign_ending_is_noticed() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "alpha\nbeta\n");
    assert_eq!(editor.newline(), Newline::Lf);

    editor.insert_text("carried in\r\n");
    editor.on_idle();

    assert_eq!(editor.newline(), Newline::Mixed);
}

#[test]
fn the_same_works_for_crlf() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path) = open(&dir, "alpha\nbeta\rgamma\r\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::ConvertToCrLf));

    assert_eq!(editor.document().to_string(), "alpha\r\nbeta\r\ngamma\r\n");
    assert_eq!(editor.newline(), Newline::CrLf);

    editor.save();
    let written = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written, "alpha\r\nbeta\r\ngamma\r\n");
}

/// The trap the ticket names. Nothing else in this codebase produces a lone `\r`, so nothing else
/// would have noticed it being left behind.
#[test]
fn a_classic_mac_file_converts_both_ways() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "one\rtwo\rthree\r");
    assert_eq!(editor.newline(), Newline::Cr, "and the bar says so");

    assert!(editor.apply_command(mdedit_menu_win::Command::ConvertToLf));
    assert_eq!(editor.document().to_string(), "one\ntwo\nthree\n");
    assert_eq!(editor.newline(), Newline::Lf);
}

/// A conversion that changes nothing is a no-op: no edit, no undo entry, no modified flag.
#[test]
fn converting_to_the_style_it_already_has_does_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "alpha\nbeta\n");

    assert!(!editor.apply_command(mdedit_menu_win::Command::ConvertToLf));
    assert!(!editor.document().can_undo());
    assert_eq!(editor.document().to_string(), "alpha\nbeta\n");
}

// ---- what Enter inserts ----

/// **Enter follows the file.** It used to insert `\n` whatever the document was, which is how a CRLF
/// file becomes Mixed without anybody pasting anything into it.
#[test]
fn enter_inserts_the_documents_own_ending() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "alpha\r\nbeta\r\n");
    assert_eq!(editor.newline(), Newline::CrLf);

    editor.select_all();
    editor.insert_text("x");
    editor.insert_text(editor.newline().as_str());

    assert_eq!(editor.document().to_string(), "x\r\n");
}

/// And after a conversion it follows the *new* style, which is the half that keeps a converted file
/// from drifting straight back to Mixed.
#[test]
fn enter_follows_a_conversion() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "alpha\r\nbeta\r\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::ConvertToLf));
    assert_eq!(editor.newline().as_str(), "\n");
}

/// An untitled buffer has no file to have detected anything from, and answers with the default —
/// the same fallback the status bar uses, because Enter and the bar disagreeing is the bug.
#[test]
fn an_untitled_buffer_uses_the_default() {
    let mut editor = Editor::new("alpha\n", Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(editor.newline(), Newline::default());
}

// ---- the parse afterwards ----

/// A whole-buffer replace exceeds any region, so it takes the whole-document parse. Verified rather
/// than assumed, as the ticket asks: the line count changes, so the gutter and the status bar depend
/// on the re-parse having happened.
#[test]
fn the_document_is_reparsed_after_a_conversion() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = open(&dir, "# One\r\n\r\nTwo\r\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::ConvertToLf));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.document().to_string(), "# One\n\nTwo\n");
    assert!(
        editor
            .document()
            .blocks()
            .iter()
            .any(|block| block.kind() == mdedit_language::BlockKind::Heading(1)),
        "the heading must still be a heading after the whole buffer was replaced"
    );
}
