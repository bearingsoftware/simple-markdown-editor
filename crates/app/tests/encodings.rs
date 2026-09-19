//! Reopening and saving with a chosen encoding (MDEDIT-42).
//!
//! `mdedit-file` tests what the encodings do to bytes. This is the wiring, and the two rules that
//! make the feature safe rather than merely present: **reopening asks before it discards**, and a
//! save the encoding cannot hold **fails loudly and writes nothing**.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use mdedit_app::editor::Editor;
use mdedit_app::prompt::{Closing, Unsaved, UnsavedPrompt};
use mdedit_session::Workspace;
use tempfile::TempDir;

use crate::stub;
use stub::Grid;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// Indices into `mdedit_file::choices()`, named so the tests read as intentions.
const UTF8: usize = 0;
const UTF8_BOM: usize = 1;
const WINDOWS_1252: usize = 4;
const GBK: usize = 5;

/// A prompt that answers the same way every time and counts how often it was asked.
#[derive(Debug, Clone)]
struct Asked(Rc<RefCell<(usize, Unsaved)>>);

impl Asked {
    fn saying(answer: Unsaved) -> Self {
        Self(Rc::new(RefCell::new((0, answer))))
    }
    fn count(&self) -> usize {
        self.0.borrow().0
    }
}

impl UnsavedPrompt for Asked {
    fn ask(&mut self, _name: &str, _what: Closing) -> Unsaved {
        let mut state = self.0.borrow_mut();
        state.0 += 1;
        state.1
    }
}

/// What the editor tried to tell the user.
#[derive(Debug, Default, Clone)]
struct Said(Rc<RefCell<Vec<String>>>);

impl Said {
    fn last(&self) -> Option<String> {
        self.0.borrow().last().cloned()
    }
}

fn open(dir: &TempDir, name: &str, bytes: &[u8]) -> (Editor<Grid>, std::path::PathBuf, Said) {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

    let said = Said::default();
    let sink = said.clone();
    editor.with_messages(Box::new(move |message| {
        sink.0.borrow_mut().push(message.to_owned());
    }));
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    (editor, path, said)
}

// ---- reopening ----

/// **The acceptance criterion.** A windows-1252 file is not valid UTF-8, so it opens as an error;
/// reopening as what it actually is reads it, and saving it back is byte-identical.
#[test]
fn a_legacy_file_reopens_correctly_and_saves_byte_identical() {
    let dir = TempDir::new().unwrap();
    let bytes = b"caf\xe9 na\xefve\n";
    let (mut editor, path, _) = open(&dir, "legacy.txt", bytes);

    assert!(editor.reopen_with(WINDOWS_1252));
    assert_eq!(editor.document().to_string(), "café naïve\n");

    editor.save();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

/// The word count was taken while the tab held nothing, so without clearing it the status bar reads
/// "0 words" over a document full of them until the next keystroke. Noticed in the window.
#[test]
fn the_word_count_follows_a_reopen() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "legacy.txt", b"caf\xe9 na\xefve today\n");

    assert!(editor.reopen_with(WINDOWS_1252));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.words(), Some(3));
}

/// Through the command, not the method. Everything else here calls `reopen_with` directly, which
/// would keep passing if the menu's `Command` never reached it — and that is exactly the half that
/// broke: the ids for these items are minted after `forward_to` snapshots the command map, so they
/// went to a map nobody was reading. A native menu cannot be clicked by a test, but the arm can.
#[test]
fn the_commands_reach_the_methods() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "legacy.txt", b"caf\xe9\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::ReopenWith(WINDOWS_1252)));
    assert_eq!(editor.document().to_string(), "café\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::SaveWith(UTF8)));
    assert_eq!(editor.encoding_choice(), Some(UTF8));
}

/// Reopening discards whatever is in the buffer, so it asks first — through the prompt Close uses,
/// not a second dialog meaning the same thing.
#[test]
fn reopening_with_unsaved_changes_asks_and_cancel_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "legacy.txt", b"caf\xe9\n");
    editor.reopen_with(WINDOWS_1252);

    editor.insert_text("edited ");
    let before = editor.document().to_string();

    let prompt = Asked::saying(Unsaved::Cancel);
    editor.with_prompt(Box::new(prompt.clone()));

    assert!(!editor.reopen_with(UTF8));
    assert_eq!(prompt.count(), 1, "it must ask");
    assert_eq!(
        editor.document().to_string(),
        before,
        "cancel must leave the buffer untouched"
    );
}

#[test]
fn discarding_lets_the_reopen_through() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "legacy.txt", b"caf\xe9\n");
    editor.reopen_with(WINDOWS_1252);
    editor.insert_text("edited ");

    let prompt = Asked::saying(Unsaved::Discard);
    editor.with_prompt(Box::new(prompt.clone()));

    assert!(editor.reopen_with(WINDOWS_1252));
    assert_eq!(prompt.count(), 1);
    assert_eq!(editor.document().to_string(), "café\n");
}

/// An unmodified buffer has nothing to lose, so it is not asked. Asking anyway is how a safeguard
/// becomes a formality people click through.
#[test]
fn an_unmodified_buffer_is_not_asked_about() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "legacy.txt", b"caf\xe9\n");

    let prompt = Asked::saying(Unsaved::Cancel);
    editor.with_prompt(Box::new(prompt.clone()));

    assert!(editor.reopen_with(WINDOWS_1252));
    assert_eq!(prompt.count(), 0);
}

/// A wrong guess costs nothing: the read happens before anything is replaced, so the buffer is
/// exactly as it was and the user is told why. Trying several is how somebody finds out what a file
/// is.
#[test]
fn a_wrong_guess_leaves_the_buffer_alone_and_says_why() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, said) = open(&dir, "legacy.txt", b"caf\xe9\n");
    editor.reopen_with(WINDOWS_1252);
    let before = editor.document().to_string();

    assert!(!editor.reopen_with(UTF8), "not valid UTF-8");
    assert_eq!(editor.document().to_string(), before);
    let message = said.last().expect("the user should have been told");
    assert!(message.contains("UTF-8"), "got {message}");
}

// ---- saving with a different encoding ----

/// Choosing an encoding writes nothing by itself — it changes what the *next* save produces.
#[test]
fn choosing_an_encoding_takes_effect_at_the_next_save() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path, _) = open(&dir, "plain.txt", b"hello\n");

    assert!(editor.save_with(UTF8_BOM));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"hello\n",
        "nothing is written until a save"
    );

    editor.save();
    assert_eq!(std::fs::read(&path).unwrap(), b"\xef\xbb\xbfhello\n");
}

#[test]
fn choosing_the_encoding_it_already_has_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "plain.txt", b"hello\n");
    assert!(!editor.save_with(UTF8));
}

/// **The refusal.** Content the target cannot hold fails the save, writes nothing, and says so —
/// rather than writing `&#27721;` where the user had 汉 and reporting success.
#[test]
fn content_the_encoding_cannot_hold_fails_the_save_and_says_so() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path, said) = open(&dir, "cjk.txt", "汉字\n".as_bytes());
    let before = std::fs::read(&path).unwrap();

    assert!(editor.save_with(WINDOWS_1252));
    editor.save();

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused save must leave the file exactly as it was"
    );
    let message = said.last().expect("a failed save must not be silent");
    assert!(message.contains("Could not save"), "got {message}");
}

/// And an encoding that *can* hold it goes through, so the refusal is about the content rather than
/// about legacy encodings being distrusted.
#[test]
fn the_same_content_in_an_encoding_that_can_hold_it_saves() {
    let dir = TempDir::new().unwrap();
    let (mut editor, path, _) = open(&dir, "cjk.txt", "汉字\n".as_bytes());

    assert!(editor.save_with(GBK));
    editor.save();

    let written = std::fs::read(&path).unwrap();
    assert_ne!(written, "汉字\n".as_bytes(), "it should be GBK now");
    // Read back through the same choice the menu offers, rather than by naming `encoding_rs` — which
    // this crate deliberately does not depend on.
    assert_eq!(
        mdedit_file::load_with(&path, mdedit_file::choices()[GBK])
            .unwrap()
            .text(),
        "汉字\n"
    );
}

// ---- what the menu is told ----

#[test]
fn the_current_encoding_is_the_one_ticked() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "bom.txt", b"\xef\xbb\xbfhello\n");

    assert_eq!(editor.encoding_choice(), Some(UTF8_BOM));
    assert!(editor.save_with(UTF8));
    assert_eq!(editor.encoding_choice(), Some(UTF8));
}

/// An untitled buffer has nothing on disk to reopen and settles its encoding at Save As, so both
/// submenus are greyed rather than left to fail when pressed.
#[test]
fn an_untitled_buffer_cannot_choose() {
    let mut editor = Editor::new("hello\n", Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(!editor.can_choose_encoding());
    assert_eq!(editor.encoding_choice(), None);
    assert!(!editor.reopen_with(WINDOWS_1252));
    assert!(!editor.save_with(UTF8_BOM));
}

/// An index that is not in the list does nothing rather than panicking. It cannot arrive from the
/// menu, but the command carries a number and a number is the kind of thing that gets stale.
#[test]
fn an_index_off_the_end_is_ignored() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _, _) = open(&dir, "plain.txt", b"hello\n");

    assert!(!editor.reopen_with(99));
    assert!(!editor.save_with(99));
}
