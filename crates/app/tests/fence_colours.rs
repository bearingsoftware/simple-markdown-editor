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

/// What the key is for: the colours are read at draw time, but the layout holds the glyphs, and a
/// Literal draws italic. An arriving answer must rebuild the fence's chunk — and only that chunk.
#[test]
fn an_arriving_answer_rebuilds_its_own_chunk_and_no_other() {
    let mut editor = editor_on("para one\n\n```rust\nlet a = true;\n```\n\npara two\n");
    editor.hold_highlighting();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let before = editor.text_backend().layouts();

    editor.start_highlighting();
    answered(&mut editor);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.text_backend().layouts(),
        before + 1,
        "the layout cache served the plain layout, or rebuilt more than the fence"
    );
}

/// Waits until the worker has answered everything asked of it, a few answers at most.
fn until_nothing_is_pending(editor: &mut Editor<Grid>) {
    for _ in 0..8 {
        if editor.highlight_counts().1 == 0 {
            return;
        }
        let _ = editor.await_highlights(PATIENCE);
    }
}

/// A token below the edit keeps its colour at its shifted offset while the answer is pending: the
/// suffix half of the reuse, which the prefix test does not touch.
#[test]
fn a_token_below_an_edit_keeps_its_colour_at_the_shifted_offset() {
    let mut editor = editor_on(SOURCE);
    answered(&mut editor);
    assert!(keyword_quads(&mut editor) > 0);

    editor.hold_highlighting();
    let at = SOURCE.find("fn one").unwrap();
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(at)));
    editor.insert_text("xyz ");
    let reused = keyword_quads(&mut editor);

    let mut baseline = editor_on(&SOURCE.replacen("fn one", "xyz fn one", 1));
    baseline.hold_highlighting();
    let plain = keyword_quads(&mut baseline);
    assert!(
        reused >= plain + 4,
        "{reused} keyword quads with shifted colours, {plain} with none: both `fn`s should keep theirs"
    );
}

/// Two requests for one fence queued before the worker runs: only the newest is highlighted, the
/// older is answered `Superseded` and forgotten, and going back to the older bytes asks again.
#[test]
fn only_the_newest_request_for_a_fence_is_highlighted_and_the_older_is_forgotten() {
    let mut editor = editor_on(SOURCE);
    editor.hold_highlighting();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    type_after(&mut editor, "fn two() {}", " ");
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.highlight_counts(),
        (0, 2, 0),
        "both versions are asked for"
    );

    editor.start_highlighting();
    until_nothing_is_pending(&mut editor);
    assert_eq!(
        editor.highlight_counts(),
        (1, 0, 0),
        "the older request should have been superseded, not highlighted"
    );

    assert!(editor.document_mut().undo());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.highlight_counts(),
        (1, 1, 0),
        "going back to the older bytes should ask again"
    );
}

/// More tagged fences laid out than the table of remembered versions holds (64): it stays full
/// rather than being wiped, and reuse still works for a fence in it.
#[test]
fn the_remembered_versions_stay_full_past_the_cap_and_reuse_still_works() {
    let source = "```rust\nfn f() {}\n```\n\n".repeat(100);
    let tall = 40_000.0;
    let mut editor = editor_on(&source);
    let _ = editor.build_frame(WIDTH, tall, 1.0);
    until_nothing_is_pending(&mut editor);
    let _ = editor.build_frame(WIDTH, tall, 1.0);
    assert_eq!(
        editor.remembered_fences(),
        64,
        "100 fences were laid out: the table should hold its cap, not be cleared at it"
    );

    // The last fence laid out is always remembered, whichever were evicted.
    editor.hold_highlighting();
    let at = source.rfind("fn f() {}").unwrap() + "fn f() {}".len();
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(at)));
    editor.insert_text(" ");
    let reused = {
        let key = Theme::dark().token_key;
        let wanted = [key.r, key.g, key.b, key.a];
        editor
            .build_frame(WIDTH, tall, 1.0)
            .iter()
            .filter(|quad| quad.colour == wanted)
            .count()
    };

    let mut edited = source.clone();
    edited.insert(at, ' ');
    let mut baseline = editor_on(&edited);
    baseline.hold_highlighting();
    let key = Theme::dark().token_key;
    let wanted = [key.r, key.g, key.b, key.a];
    let plain = baseline
        .build_frame(WIDTH, tall, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted)
        .count();
    assert!(
        reused > plain,
        "{reused} keyword quads reused, {plain} plain"
    );
}

fn boom(_: &str, _: &str) -> Option<Vec<(mdedit_offsets::ByteSpan, mdedit_language::Token)>> {
    panic!("a job that panics, on purpose");
}

/// A job that panics is answered plain, and the worker lives to answer the next fence: a dead
/// worker would leave every later fence plain for the session.
#[test]
fn a_panicking_job_is_answered_plain_and_the_worker_survives() {
    let mut editor = editor_on("```rust\nfn a() {}\n```\n\n```rust\nfn b() {}\n```\n");
    editor.use_highlight_job(boom);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    until_nothing_is_pending(&mut editor);
    assert_eq!(
        editor.highlight_counts(),
        (0, 0, 2),
        "both fences should be answered plain, the second by a worker that outlived the first"
    );
}

/// The thread that draws never loads the syntax set (MDEDIT-190).
///
/// **Needs a process of its own**, which nextest gives every test and CI runs; under plain
/// `cargo test` another test in this binary may have loaded the set first, and this panics rather
/// than skip, because a skipped check proves nothing. The worker is held back, so a loaded set
/// could only have come from the drawing thread.
#[test]
fn laying_a_frame_out_with_a_pending_fence_never_loads_the_syntax_set() {
    assert!(
        !mdedit_highlight::is_loaded(),
        "the syntax set is already loaded: this test must run under nextest, one process per test"
    );

    let mut editor = editor_on("```rust\nfn main() {}\n```\n");
    editor.hold_highlighting();
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(!quads.is_empty());
    assert!(
        !mdedit_highlight::is_loaded(),
        "the drawing thread loaded the syntax set"
    );

    editor.start_highlighting();
    assert!(editor.await_highlights(PATIENCE));
    assert!(mdedit_highlight::is_loaded(), "the worker never loaded it");
}
