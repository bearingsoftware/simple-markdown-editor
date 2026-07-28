//! Undo, redo, and the grouping rule that decides what one of them means.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::{Document, Selection};
use mdedit_offsets::{ByteOffset, ByteSpan};
use proptest::prelude::*;

/// Types `text` one character at a time, as a keyboard would.
fn type_out(doc: &mut Document, text: &str) {
    for ch in text.chars() {
        let at = doc.selection().head();
        doc.insert(at, &ch.to_string()).unwrap();
    }
}

/// Backspaces `count` characters, as a keyboard would.
fn backspace(doc: &mut Document, count: usize) {
    for _ in 0..count {
        let caret = doc.selection().head();
        let text = doc.to_string();
        let mut previous = caret.get();
        loop {
            if previous == 0 {
                return;
            }
            previous -= 1;
            if text.is_char_boundary(previous) {
                break;
            }
        }
        doc.delete(ByteSpan::new(ByteOffset::new(previous), caret))
            .unwrap();
    }
}

/// The headline acceptance criterion.
#[test]
fn typing_a_word_then_undoing_once_removes_the_whole_word() {
    let mut doc = Document::new();
    type_out(&mut doc, "hello");
    assert_eq!(doc.to_string(), "hello");

    assert!(doc.undo(), "there should be something to undo");
    assert_eq!(
        doc.to_string(),
        "",
        "five keystrokes are one undo step, not five"
    );
    assert!(!doc.can_undo(), "and the history is now empty");
}

#[test]
fn a_run_of_backspaces_is_also_one_step() {
    let mut doc = Document::from("hello world");
    doc.set_selection(Selection::caret(ByteOffset::new(11)));
    backspace(&mut doc, 5);
    assert_eq!(doc.to_string(), "hello ");

    assert!(doc.undo());
    assert_eq!(doc.to_string(), "hello world");
}

#[test]
fn undo_restores_the_caret_as_well_as_the_text() {
    let mut doc = Document::from("start");
    doc.set_selection(Selection::caret(ByteOffset::new(5)));
    type_out(&mut doc, "ing");
    assert_eq!(doc.selection().head(), ByteOffset::new(8));

    doc.undo();
    assert_eq!(
        doc.selection().head(),
        ByteOffset::new(5),
        "undo that leaves the caret elsewhere feels broken even when the text is right"
    );
}

#[test]
fn redo_puts_the_caret_back_where_the_edit_left_it() {
    let mut doc = Document::new();
    type_out(&mut doc, "abc");
    doc.undo();
    assert!(doc.redo());
    assert_eq!(doc.to_string(), "abc");
    assert_eq!(doc.selection().head(), ByteOffset::new(3));
}

#[test]
fn undo_redo_undo_is_stable() {
    let mut doc = Document::from("base");
    doc.set_selection(Selection::caret(ByteOffset::new(4)));
    type_out(&mut doc, " more");

    let edited = doc.to_string();
    doc.undo();
    let undone = doc.to_string();

    for _ in 0..5 {
        doc.redo();
        assert_eq!(doc.to_string(), edited);
        doc.undo();
        assert_eq!(doc.to_string(), undone);
    }
}

/// Moving the caret is what ends a typing run here, in place of a timer.
#[test]
fn moving_the_caret_starts_a_new_undo_group() {
    let mut doc = Document::new();
    type_out(&mut doc, "one");
    doc.set_selection(Selection::caret(ByteOffset::ZERO));
    type_out(&mut doc, "two");
    assert_eq!(doc.to_string(), "twoone");

    doc.undo();
    assert_eq!(
        doc.to_string(),
        "one",
        "the second run should undo on its own"
    );
    doc.undo();
    assert_eq!(doc.to_string(), "");
}

/// A newline is a natural boundary, so undo lands on paragraphs rather than the whole document.
#[test]
fn a_newline_ends_the_group() {
    let mut doc = Document::new();
    type_out(&mut doc, "first");
    doc.insert(doc.selection().head(), "\n").unwrap();
    type_out(&mut doc, "second");

    doc.undo();
    assert_eq!(doc.to_string(), "first\n", "only the second line goes");
}

#[test]
fn typing_somewhere_else_does_not_join_the_previous_run() {
    let mut doc = Document::from("aXb");
    // Insert at 0 and then at 3 without moving the caret between: not contiguous, so two groups.
    doc.insert(ByteOffset::ZERO, "1").unwrap();
    doc.insert(ByteOffset::new(4), "2").unwrap();
    assert_eq!(doc.to_string(), "1aXb2");

    doc.undo();
    assert_eq!(doc.to_string(), "1aXb");
}

#[test]
fn a_new_edit_discards_the_redo_stack() {
    let mut doc = Document::new();
    type_out(&mut doc, "abc");
    doc.undo();
    assert!(doc.can_redo());

    type_out(&mut doc, "z");
    assert!(
        !doc.can_redo(),
        "the future being redone no longer follows from the present"
    );
}

#[test]
fn undo_and_redo_report_whether_they_did_anything() {
    let mut doc = Document::new();
    assert!(!doc.undo(), "nothing to undo in a fresh document");
    assert!(!doc.redo());
    assert!(!doc.can_undo());
    assert!(!doc.can_redo());

    type_out(&mut doc, "x");
    assert!(doc.can_undo());
    assert!(doc.undo());
    assert!(!doc.undo());
}

#[test]
fn replacing_a_selection_is_one_step() {
    let mut doc = Document::from("hello cruel world");
    doc.replace(
        ByteSpan::new(ByteOffset::new(6), ByteOffset::new(11)),
        "kind",
    )
    .unwrap();
    assert_eq!(doc.to_string(), "hello kind world");

    doc.undo();
    assert_eq!(
        doc.to_string(),
        "hello cruel world",
        "the replaced bytes come back verbatim, not re-derived"
    );
}

#[test]
fn multibyte_text_survives_undo() {
    let mut doc = Document::new();
    type_out(&mut doc, "汉字🎉");
    assert_eq!(doc.to_string(), "汉字🎉");

    doc.undo();
    assert_eq!(doc.to_string(), "");

    doc.redo();
    assert_eq!(doc.to_string(), "汉字🎉");
}

/// History is bounded, so a long session cannot grow without limit.
#[test]
fn history_is_bounded() {
    let mut doc = Document::new();
    // Each iteration is its own group, because the caret move breaks the run.
    for index in 0..700 {
        doc.set_selection(Selection::caret(ByteOffset::ZERO));
        doc.insert(ByteOffset::ZERO, &format!("{}", index % 10))
            .unwrap();
    }

    let mut undone = 0;
    while doc.undo() {
        undone += 1;
        assert!(undone <= 700, "history should be bounded, not unbounded");
    }
    assert!(
        undone < 700,
        "700 groups were made and all {undone} survived; the bound is not working"
    );
}

/// Everything the partition promises, after an undo.
fn assert_partitions(doc: &Document, context: &str) {
    let text = doc.to_string();
    let rebuilt: String = doc
        .segments()
        .iter()
        .map(|s| s.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, text, "{context}: source not reproduced");
}

#[test]
fn the_partition_survives_undo_of_a_construct_breaking_edit() {
    let mut doc = Document::from("**bold**");
    doc.set_selection(Selection::caret(ByteOffset::new(8)));
    backspace(&mut doc, 2);
    assert_partitions(&doc, "after deleting the closing pair");

    doc.undo();
    assert_eq!(doc.to_string(), "**bold**");
    assert_partitions(&doc, "after undo");

    // And the segmentation must match a document parsed from scratch, not merely tile.
    let fresh = Document::from(doc.to_string().as_str());
    assert_eq!(doc.segments(), fresh.segments());
    assert_eq!(doc.blocks(), fresh.blocks());
}

const FRAGMENTS: &[&str] = &[
    "a", "word ", "\n", "\n\n", "*", "**", "`", "#", "## ", "> ", "- ", "汉", "🎉", "[", "](url)",
];

/// What one does to a document.
#[derive(Debug, Clone)]
enum Action {
    Type(String),
    Backspace(usize),
    MoveCaret(usize),
    Undo,
    Redo,
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        proptest::collection::vec(0..FRAGMENTS.len(), 1..4)
            .prop_map(|parts| Action::Type(parts.into_iter().map(|i| FRAGMENTS[i]).collect())),
        (1usize..4).prop_map(Action::Backspace),
        (0usize..40).prop_map(Action::MoveCaret),
        Just(Action::Undo),
        Just(Action::Redo),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(384))]

    /// The gate, extended to undo.
    ///
    /// Undo is a new way to reach a buffer state, and therefore a new way to desynchronise the
    /// segmentation from the text. Asserted after every single action, not just at the end.
    #[test]
    fn the_invariant_holds_after_every_undo_and_redo(
        actions in proptest::collection::vec(action_strategy(), 0..30),
    ) {
        let mut doc = Document::from("# Start\n\ninitial text\n");

        for (index, action) in actions.iter().enumerate() {
            match action {
                Action::Type(text) => {
                    let at = doc.selection().head();
                    let _ = doc.insert(at, text);
                }
                Action::Backspace(count) => backspace(&mut doc, *count),
                Action::MoveCaret(offset) => {
                    let text = doc.to_string();
                    let mut at = (*offset).min(text.len());
                    while at > 0 && !text.is_char_boundary(at) {
                        at -= 1;
                    }
                    doc.set_selection(Selection::caret(ByteOffset::new(at)));
                }
                Action::Undo => { doc.undo(); }
                Action::Redo => { doc.redo(); }
            }

            let text = doc.to_string();
            let rebuilt: String = doc
                .segments()
                .iter()
                .map(|s| s.span().slice(&text))
                .collect();
            prop_assert_eq!(&rebuilt, &text, "after action {}: {:?}", index, action);

            // Stronger than tiling: the cache must describe the text it actually holds.
            let fresh = Document::from(text.as_str());
            prop_assert_eq!(doc.segments(), fresh.segments(), "segments diverged at {}", index);
        }
    }

    /// Undoing everything returns the document to exactly where it started.
    #[test]
    fn undoing_everything_restores_the_original(
        actions in proptest::collection::vec(action_strategy(), 0..24),
    ) {
        let original = "# Start\n\ninitial text\n";
        let mut doc = Document::from(original);

        for action in actions {
            match action {
                Action::Type(text) => {
                    let at = doc.selection().head();
                    let _ = doc.insert(at, &text);
                }
                Action::Backspace(count) => backspace(&mut doc, count),
                Action::MoveCaret(offset) => {
                    let text = doc.to_string();
                    let mut at = offset.min(text.len());
                    while at > 0 && !text.is_char_boundary(at) {
                        at -= 1;
                    }
                    doc.set_selection(Selection::caret(ByteOffset::new(at)));
                }
                // Undo and redo are excluded here: replaying them would make "everything" ambiguous.
                Action::Undo | Action::Redo => {}
            }
        }

        while doc.undo() {}
        prop_assert_eq!(doc.to_string(), original);
    }
}
