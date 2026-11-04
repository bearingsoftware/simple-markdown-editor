//! A fence's colours arrive from the worker, and nothing waits for them (MDEDIT-190).
//!
//! Driven through the real `Editor` and the real highlight worker. The worker is held back with
//! `hold_highlighting` when a test needs colours genuinely pending, and released with
//! `start_highlighting`; waiting is on the worker's answer, never on a clock.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::time::Duration;

use mdedit_app::editor::Editor;
use mdedit_app::view::{Theme, ThemeChoice};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub::Grid;

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 600.0;
const PATIENCE: Duration = Duration::from_secs(20);

const SOURCE: &str = "```rust\nfn one() {}\nfn two() {}\n```\n";

fn editor_on(source: &str) -> Editor<Grid> {
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.set_theme_choice(ThemeChoice::Dark);
    editor
}

/// How many quads one frame draws in the keyword colour.
fn keyword_quads(editor: &mut Editor<Grid>) -> usize {
    let key = Theme::dark().token_key;
    let wanted = [key.r, key.g, key.b, key.a];
    editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted)
        .count()
}

fn answered(editor: &mut Editor<Grid>) {
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        editor.await_highlights(PATIENCE),
        "the highlight worker answered nothing in {PATIENCE:?}"
    );
}

/// Types `text` at the end of the fence's last line of code.
fn type_after(editor: &mut Editor<Grid>, needle: &str, text: &str) {
    let at = SOURCE.find(needle).unwrap() + needle.len();
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(at)));
    editor.insert_text(text);
}

/// Colours that are pending draw plain, and an answer rebuilds that chunk with them: the layout
/// cache must not serve the plain layout for ever.
#[test]
fn an_arriving_answer_colours_the_fence() {
    let mut editor = editor_on(SOURCE);
    editor.hold_highlighting();
    let plain = keyword_quads(&mut editor);

    editor.start_highlighting();
    answered(&mut editor);
    let coloured = keyword_quads(&mut editor);
    assert!(
        coloured > plain,
        "{coloured} keyword quads after the answer, {plain} before"
    );
}

#[test]
fn a_fence_with_no_info_string_is_never_coloured() {
    let mut editor = editor_on("```\nfn one() {}\n```\n");
    editor.hold_highlighting();
    let before = keyword_quads(&mut editor);
    editor.start_highlighting();
    let _ = editor.await_highlights(Duration::from_millis(300));
    assert_eq!(keyword_quads(&mut editor), before);
}

/// Typing inside a highlighted fence does not flash it to plain: the lines before the edit keep
/// their colours while the answer for the new text is pending.
#[test]
fn an_edit_keeps_the_colours_before_it_while_the_answer_is_pending() {
    let mut editor = editor_on(SOURCE);
    answered(&mut editor);
    assert!(keyword_quads(&mut editor) > 0);

    editor.hold_highlighting();
    type_after(&mut editor, "fn two() {}", " ");
    let reused = keyword_quads(&mut editor);

    let mut baseline = editor_on(SOURCE);
    baseline.hold_highlighting();
    type_after(&mut baseline, "fn two() {}", " ");
    let plain = keyword_quads(&mut baseline);

    assert!(
        reused > plain,
        "{reused} keyword quads with the previous colours, {plain} with none"
    );
}

/// A different info string may name another language, so the old spans mean nothing.
#[test]
fn an_edit_to_the_info_string_does_not_reuse_the_old_colours() {
    let mut editor = editor_on(SOURCE);
    answered(&mut editor);
    let coloured = keyword_quads(&mut editor);

    editor.hold_highlighting();
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new("```rust".len())));
    editor.insert_text("x");
    let after = keyword_quads(&mut editor);

    let mut baseline = editor_on("```rustx\nfn one() {}\nfn two() {}\n```\n");
    baseline.hold_highlighting();
    let plain = keyword_quads(&mut baseline);
    assert!(coloured > plain);
    assert_eq!(
        after, plain,
        "the old spans were reused under a new info string"
    );
}
