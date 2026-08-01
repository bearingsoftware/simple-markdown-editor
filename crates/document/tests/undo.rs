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

// ---- transactions (MDEDIT-63) ----

/// Replaces every occurrence of `needle`, from the end backwards.
///
/// Backwards because the transaction deliberately does not reorder anything — see `Document::transact`.
/// Every caller that edits several spans has to do this, and doing it here is what makes these tests
/// exercise the real shape of a Replace All.
fn replace_all(doc: &mut Document, needle: &str, with: &str) -> usize {
    let text = doc.to_string();
    let sites: Vec<usize> = text.match_indices(needle).map(|(at, _)| at).collect();
    for at in sites.iter().rev() {
        let span = ByteSpan::new(ByteOffset::new(*at), ByteOffset::new(at + needle.len()));
        doc.replace(span, with).unwrap();
        // Moving the caret between sites is exactly what ends a group without a transaction.
        doc.set_selection(Selection::caret(ByteOffset::new(*at)));
    }
    sites.len()
}

#[test]
fn a_transaction_is_one_undo_step() {
    let mut doc = Document::from("aa bb aa bb aa\n");

    let replaced = doc.transact(|doc| replace_all(doc, "aa", "XX"));
    assert_eq!(replaced, 3);
    assert_eq!(doc.to_string(), "XX bb XX bb XX\n");

    assert!(doc.undo());
    assert_eq!(
        doc.to_string(),
        "aa bb aa bb aa\n",
        "one undo should have put back every site"
    );
    assert!(!doc.undo(), "and there should be nothing left to undo");
}

/// Without the transaction the same edits are three steps, which is the behaviour this exists to
/// replace — asserted so the test above is measuring something.
fn count_undos(doc: &mut Document) -> usize {
    let mut steps = 0;
    while doc.undo() {
        steps += 1;
    }
    steps
}

#[test]
fn without_a_transaction_the_same_edits_are_several_steps() {
    let mut doc = Document::from("aa bb aa bb aa\n");
    replace_all(&mut doc, "aa", "XX");

    assert!(
        count_undos(&mut doc) > 1,
        "the caret moved between sites, so these should not have grouped on their own"
    );
}

#[test]
fn redo_reapplies_a_transaction_in_one_step() {
    let mut doc = Document::from("aa bb aa\n");
    doc.transact(|doc| replace_all(doc, "aa", "XX"));
    doc.undo();

    assert!(doc.redo());
    assert_eq!(doc.to_string(), "XX bb XX\n");
    assert!(!doc.redo());
}

/// A Replace All that matched nothing must not cost the user an undo press.
#[test]
fn an_empty_transaction_records_nothing() {
    let mut doc = Document::from("hello\n");
    doc.insert(ByteOffset::new(5), "!").unwrap();

    doc.transact(|doc| replace_all(doc, "nowhere", "x"));

    assert!(doc.undo(), "the insert should still be there to undo");
    assert_eq!(doc.to_string(), "hello\n");
    assert!(!doc.undo(), "and the empty transaction added nothing");
}

/// Undo restores where the whole operation started, not where some step inside it ended.
#[test]
fn undo_restores_the_caret_from_before_the_transaction() {
    let mut doc = Document::from("aa bb aa\n");
    doc.set_selection(Selection::caret(ByteOffset::new(3)));

    doc.transact(|doc| replace_all(doc, "aa", "XX"));
    doc.undo();

    assert_eq!(doc.selection().head(), ByteOffset::new(3));
}

/// Joining rather than refusing: "these edits are one step" composes.
#[test]
fn a_nested_transaction_joins_the_outer_one() {
    let mut doc = Document::from("aa bb aa\n");

    doc.transact(|doc| {
        doc.replace(ByteSpan::new(ByteOffset::new(0), ByteOffset::new(2)), "XX")
            .unwrap();
        doc.transact(|doc| {
            doc.set_selection(Selection::caret(ByteOffset::new(6)));
            doc.replace(ByteSpan::new(ByteOffset::new(6), ByteOffset::new(8)), "YY")
                .unwrap();
        });
    });
    assert_eq!(doc.to_string(), "XX bb YY\n");

    assert!(doc.undo());
    assert_eq!(doc.to_string(), "aa bb aa\n");
    assert!(
        !doc.undo(),
        "the inner transaction must not be its own step"
    );
}

/// The analysis has to be right at the end, however many parses were skipped getting there.
#[test]
fn the_segmentation_is_correct_after_a_transaction() {
    let mut doc = Document::from("# One\n\n*two*\n\n# Three\n");
    doc.transact(|doc| replace_all(doc, "#", "##"));

    let text = doc.to_string();
    let rebuilt: String = doc
        .segments()
        .iter()
        .map(|s| s.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, text, "the segments must still tile the buffer");

    let fresh = Document::from(text.as_str());
    assert_eq!(
        doc.segments(),
        fresh.segments(),
        "and must match what parsing the result from scratch gives"
    );
}

/// A transaction over a large buffer must leave a whole-document analysis, not a scoped one — its
/// edits could be anywhere, and a scoped answer would be a claim nothing checked.
#[test]
fn a_transaction_leaves_a_whole_document_analysis() {
    let big = "a paragraph of text\n\n".repeat(2000);
    let mut doc = Document::from(big.as_str());
    assert!(
        doc.len_bytes() > 16 * 1024,
        "the scoped path needs a large buffer"
    );

    doc.transact(|doc| {
        doc.replace(ByteSpan::new(ByteOffset::new(0), ByteOffset::new(1)), "A")
            .unwrap();
    });

    assert!(
        !doc.is_scoped(),
        "the parse taken at commit should be a whole-document one"
    );
}

proptest! {
    /// The property that matters: however many replacements a transaction makes, one undo puts the
    /// document back byte for byte.
    #[test]
    fn one_undo_restores_a_transaction_byte_for_byte(
        words in prop::collection::vec("[ab]{1,4}", 1..40),
    ) {
        let original = format!("{}\n", words.join(" "));
        let mut doc = Document::from(original.as_str());

        doc.transact(|doc| replace_all(doc, "a", "LONGER"));
        doc.undo();

        prop_assert_eq!(doc.to_string(), original);
    }

    /// And redo puts it back to what the transaction produced.
    #[test]
    fn redo_restores_what_the_transaction_produced(
        words in prop::collection::vec("[ab]{1,4}", 1..40),
    ) {
        let original = format!("{}\n", words.join(" "));
        let mut doc = Document::from(original.as_str());

        doc.transact(|doc| replace_all(doc, "a", "LONGER"));
        let after = doc.to_string();
        doc.undo();
        doc.redo();

        prop_assert_eq!(doc.to_string(), after);
    }
}
