//! Copy, cut, paste, select-all — and the correctness fix underneath them.
//!
//! Against a fake clipboard rather than the system one, so these assert what the editor did rather
//! than what the machine happened to have copied, and so they run on a headless CI runner without
//! racing each other.
//!
//! The half that matters most is not the clipboard at all: typing with a selection active used to
//! insert *beside* it, leaving the selected text in the buffer. That is asserted here too, because it
//! is the bug this ticket set out to fix.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::cell::RefCell;
use std::rc::Rc;

use mdedit_app::clipboard::Clipboard;
use mdedit_app::editor::Editor;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::Grid;

/// A clipboard in a variable. Shared with the test so it can be read back after a copy.
#[derive(Debug, Default, Clone)]
struct Fake(Rc<RefCell<Option<String>>>);

impl Fake {
    fn holding(text: &str) -> Self {
        Self(Rc::new(RefCell::new(Some(text.to_owned()))))
    }

    fn contents(&self) -> Option<String> {
        self.0.borrow().clone()
    }
}

impl Clipboard for Fake {
    fn read(&mut self) -> Option<String> {
        self.0.borrow().clone()
    }

    fn write(&mut self, text: &str) {
        *self.0.borrow_mut() = Some(text.to_owned());
    }
}

fn editor_with(source: &str, clipboard: &Fake) -> Editor<Grid> {
    let mut editor = Editor::new(source, Grid::new());
    editor.with_clipboard(Box::new(clipboard.clone()));
    editor
}

fn select(editor: &mut Editor<Grid>, from: usize, to: usize) {
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(from), ByteOffset::new(to)));
}

// ---------------------------------------------------------------------------
// The correctness fix: a selection is replaced, not typed beside.
// ---------------------------------------------------------------------------

#[test]
fn typing_over_a_selection_replaces_it() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 0, 5);

    editor.insert_text("goodbye");
    assert_eq!(editor.document().to_string(), "goodbye world\n");
}

/// And in one undo step, not two. This is why it goes through `replace` rather than delete-then-type.
#[test]
fn replacing_a_selection_is_one_undo_step() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 0, 5);
    editor.insert_text("goodbye");

    editor.document_mut().undo();
    assert_eq!(
        editor.document().to_string(),
        "hello world\n",
        "one undo should put back both what was removed and what was typed"
    );
}

#[test]
fn backspace_with_a_selection_removes_the_selection() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 6, 11);

    assert!(editor.delete_selected());
    assert_eq!(editor.document().to_string(), "hello \n");
    assert_eq!(
        editor.document().selection().head(),
        ByteOffset::new(6),
        "the caret is left where the selection started"
    );
}

#[test]
fn with_nothing_selected_there_is_nothing_to_delete() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello\n", &clipboard);
    assert!(
        !editor.delete_selected(),
        "so the caller does the character"
    );
    assert_eq!(editor.document().to_string(), "hello\n");
}

// ---------------------------------------------------------------------------
// Copy, cut, paste.
// ---------------------------------------------------------------------------

#[test]
fn copy_puts_the_selection_on_the_clipboard_and_leaves_the_buffer_alone() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 6, 11);

    editor.copy();
    assert_eq!(clipboard.contents().as_deref(), Some("world"));
    assert_eq!(editor.document().to_string(), "hello world\n");
}

#[test]
fn copy_with_nothing_selected_does_not_clear_the_clipboard() {
    let clipboard = Fake::holding("kept");
    let mut editor = editor_with("hello\n", &clipboard);

    editor.copy();
    assert_eq!(
        clipboard.contents().as_deref(),
        Some("kept"),
        "an accidental Ctrl+C must not throw away what was already there"
    );
}

#[test]
fn cut_takes_the_text_and_removes_it() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 0, 6);

    assert!(editor.cut());
    assert_eq!(clipboard.contents().as_deref(), Some("hello "));
    assert_eq!(editor.document().to_string(), "world\n");
}

#[test]
fn one_undo_puts_back_a_cut() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 0, 6);
    editor.cut();

    editor.document_mut().undo();
    assert_eq!(editor.document().to_string(), "hello world\n");
}

#[test]
fn cut_with_nothing_selected_does_nothing() {
    let clipboard = Fake::holding("kept");
    let mut editor = editor_with("hello\n", &clipboard);

    assert!(!editor.cut());
    assert_eq!(editor.document().to_string(), "hello\n");
    assert_eq!(clipboard.contents().as_deref(), Some("kept"));
}

#[test]
fn paste_inserts_at_the_caret() {
    let clipboard = Fake::holding("there ");
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 6, 6);

    assert!(editor.paste());
    assert_eq!(editor.document().to_string(), "hello there world\n");
}

#[test]
fn paste_over_a_selection_replaces_it() {
    let clipboard = Fake::holding("everyone");
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 6, 11);

    assert!(editor.paste());
    assert_eq!(editor.document().to_string(), "hello everyone\n");
}

/// A paste is one insertion however many lines it carries, so undo takes one step rather than one
/// per line.
#[test]
fn pasting_several_lines_is_one_edit() {
    let clipboard = Fake::holding("one\ntwo\nthree\n");
    let mut editor = editor_with("start\n", &clipboard);
    select(&mut editor, 6, 6);

    assert!(editor.paste());
    assert_eq!(editor.document().to_string(), "start\none\ntwo\nthree\n");

    editor.document_mut().undo();
    assert_eq!(editor.document().to_string(), "start\n");
}

#[test]
fn pasting_an_empty_clipboard_does_nothing() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello\n", &clipboard);
    assert!(!editor.paste());
    assert_eq!(editor.document().to_string(), "hello\n");

    let empty = Fake::holding("");
    let mut editor = editor_with("hello\n", &empty);
    assert!(!editor.paste(), "an empty string is not an edit");
    assert_eq!(editor.document().to_string(), "hello\n");
}

// ---------------------------------------------------------------------------
// Select all.
// ---------------------------------------------------------------------------

#[test]
fn select_all_covers_the_buffer() {
    let clipboard = Fake::default();
    let mut editor = editor_with("hello world\n", &clipboard);

    editor.select_all();
    let selection = editor.document().selection();
    assert_eq!(selection.anchor(), ByteOffset::ZERO);
    assert_eq!(selection.head(), ByteOffset::new(12));
}

#[test]
fn select_all_then_paste_replaces_the_whole_document_in_one_edit() {
    let clipboard = Fake::holding("replacement\n");
    let mut editor = editor_with("the old contents\nover two lines\n", &clipboard);

    editor.select_all();
    assert!(editor.paste());
    assert_eq!(editor.document().to_string(), "replacement\n");

    editor.document_mut().undo();
    assert_eq!(
        editor.document().to_string(),
        "the old contents\nover two lines\n"
    );
}

#[test]
fn select_all_on_an_empty_document_selects_nothing() {
    let clipboard = Fake::default();
    let mut editor = editor_with("", &clipboard);
    editor.select_all();
    assert!(editor.document().selection().is_collapsed());
    assert_eq!(editor.selected_text(), None);
}

// ---------------------------------------------------------------------------
// The invariant that outranks all of it.
// ---------------------------------------------------------------------------

/// Clipboard text is untrusted input in the same way file contents are: it goes into the buffer, and
/// what comes out has to be exactly what went in.
#[test]
fn pasted_bytes_survive_a_round_trip_unchanged() {
    for text in [
        "日本語のテキスト",
        "with\ttabs\tand \r\n mixed \n endings",
        "emoji 🦀 and a combining é",
        "trailing spaces   ",
    ] {
        let clipboard = Fake::holding(text);
        let mut editor = editor_with("", &clipboard);
        assert!(editor.paste(), "pasting {text:?}");
        assert_eq!(editor.document().to_string(), text, "pasting {text:?}");

        editor.select_all();
        assert_eq!(
            editor.selected_text().as_deref(),
            Some(text),
            "copying {text:?} back out"
        );
    }
}

// ---------------------------------------------------------------------------
// The real one, and the default that must not be it.
// ---------------------------------------------------------------------------

/// MDEDIT-67. An editor nobody has handed a clipboard to must not reach the machine's.
///
/// This is the whole of that bug. `Editor::new` used to install the system clipboard, so every test
/// binary read the real one — from as many threads as the harness runs tests on, and reading the
/// Windows clipboard from two threads of one process corrupts the heap. `context_menu` aborted with
/// `STATUS_HEAP_CORRUPTION` 15 runs in 20 because three of its tests ask `context_state()` whether
/// there is anything to paste.
///
/// Asserted on the default rather than on the crash, because the crash needs a race to show itself
/// and this does not. Every other collaborator that touches the user's machine — the prompts, the
/// file pickers, the recent-files list — already defaults to an inert one; this pins the clipboard to
/// the same rule.
#[test]
fn a_fresh_editor_has_no_clipboard_of_the_users() {
    let mut editor = Editor::new("hello\n", Grid::new());

    assert!(
        !editor.context_state().can_paste,
        "the default clipboard must hold nothing, whatever the machine has copied"
    );

    // And it takes a copy without passing it on, so nothing a test does reaches the real one either.
    editor.select_all();
    editor.copy();
    assert!(!editor.context_state().can_paste);
}

/// Proves the `arboard` integration actually works, rather than only the trait around it.
///
/// Ignored by default: it touches the machine's real clipboard, so it would trample whatever the
/// person running the tests had copied, and two of these running at once would race. Run it by hand
/// with `cargo test -p mdedit-app --test clipboard -- --ignored`.
#[test]
#[ignore = "touches the real system clipboard"]
fn the_system_clipboard_round_trips() {
    use mdedit_app::clipboard::System;

    let mut system = System::default();
    let text = "mdedit round trip 日本語 🦀";
    system.write(text);
    assert_eq!(system.read().as_deref(), Some(text));
}

// ---- links from the clipboard (MDEDIT-50) ----

/// **Copy a link, select some words, press Ctrl+K, press Enter.** The case the whole clipboard-URL
/// path exists for, and MDEDIT-129 moved it one keystroke: the dialog opens pre-filled from both,
/// so what used to happen invisibly now happens where you can see it before it is written.
#[test]
fn ctrl_k_prefills_the_dialog_from_the_clipboard_and_the_selection() {
    let clipboard = Fake::holding("https://example.com");
    let mut editor = editor_with("see the docs here\n", &clipboard);
    select(&mut editor, 4, 12);

    assert!(editor.link());
    let open = editor.dialog().expect("Ctrl+K opens the dialog");
    assert_eq!(open.fields()[0].text(), "https://example.com");
    assert_eq!(open.fields()[1].text(), "the docs");
    assert_eq!(
        editor.document().to_string(),
        "see the docs here\n",
        "and nothing is written until it is accepted"
    );

    assert!(editor.accept_dialog());
    assert_eq!(
        editor.document().to_string(),
        "see [the docs](https://example.com) here\n"
    );
    assert!(editor.dialog().is_none(), "and it closed behind itself");
}

/// With something that is not a URL on the clipboard, the destination is left empty rather than
/// filled with whatever happened to be copied — and the caret starts there, because that is the
/// field with something to type in.
#[test]
fn ctrl_k_ignores_a_clipboard_that_is_not_a_url() {
    let clipboard = Fake::holding("some copied prose");
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 0, 5);

    editor.link();
    let open = editor.dialog().expect("the dialog is open");
    assert_eq!(open.fields()[0].text(), "", "not a URL, so not offered");
    assert_eq!(open.fields()[1].text(), "hello");
    assert_eq!(open.focused(), 0, "the caret starts where there is work");
}

/// **Escape leaves the document exactly as it was**, which is the half of a dialog that a skeleton
/// insertion never had: backing out used to mean an undo.
#[test]
fn escaping_the_dialog_writes_nothing() {
    let clipboard = Fake::holding("https://example.com");
    let mut editor = editor_with("see the docs here\n", &clipboard);
    select(&mut editor, 4, 12);

    editor.link();
    assert!(editor.cancel_dialog());
    assert_eq!(editor.document().to_string(), "see the docs here\n");
    assert!(
        !editor.document().can_undo(),
        "and there is nothing to undo"
    );
}

/// Pasting a URL over a selection links it. What every chat client and note app does, so its
/// absence reads as missing rather than as deliberate simplicity.
#[test]
fn pasting_a_url_over_a_selection_makes_a_link() {
    let clipboard = Fake::holding("https://example.com");
    let mut editor = editor_with("see the docs here\n", &clipboard);
    select(&mut editor, 4, 12);

    assert!(editor.paste());
    assert_eq!(
        editor.document().to_string(),
        "see [the docs](https://example.com) here\n"
    );
}

/// And with nothing selected it is still a paste — turning it into `[](url)` would invent link text
/// nobody wrote.
#[test]
fn pasting_a_url_with_nothing_selected_is_still_a_paste() {
    let clipboard = Fake::holding("https://example.com");
    let mut editor = editor_with("", &clipboard);

    editor.paste();
    assert_eq!(editor.document().to_string(), "https://example.com");
}

/// Pasting ordinary text over a selection replaces it, exactly as it always did.
#[test]
fn pasting_prose_over_a_selection_still_replaces_it() {
    let clipboard = Fake::holding("goodbye");
    let mut editor = editor_with("hello world\n", &clipboard);
    select(&mut editor, 0, 5);

    editor.paste();
    assert_eq!(editor.document().to_string(), "goodbye world\n");
}
