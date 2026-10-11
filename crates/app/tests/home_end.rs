//! Home and End move within the caret's visual row; Ctrl+Home and Ctrl+End reach the document's
//! ends (MDEDIT-192).
//!
//! Until this ticket the plain keys did what the chords do, so two of the four bindings were the
//! same command twice, and no test pressed any of them. The four are told apart here by pressing
//! each one from the same caret.
//!
//! The wrapped-row tests measure the row the caret is **drawn** on the way `caret_affinity.rs`
//! does, caret against caret, so no test states a pixel. End's row is MDEDIT-188's affinity set
//! from the keyboard: the end of a wrapped row and the start of its continuation are one byte, and
//! only the gesture knows which it meant.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::view::{Mode, Theme};
use mdedit_document::Selection;
use mdedit_offsets::{Affinity, ByteOffset};

use crate::stub;
use stub::{Grid, LINE_HEIGHT};

/// Narrow enough that a sixty-word line wraps several times.
const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 600.0;

const LINES: &str = "first line\nsecond line\nthird\n";
/// The middle of `second`, and where that line starts and ends.
const MIDDLE: usize = 14;
const SECOND_START: usize = 11;
const SECOND_END: usize = 22;

fn long() -> String {
    "word ".repeat(60)
}

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn caret_at(editor: &mut Editor<Grid>, offset: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(offset)));
    editor.build_frame(WIDTH, HEIGHT, 1.0);
}

fn head(editor: &Editor<Grid>) -> usize {
    editor.document().selection().head().get()
}

/// The top of the caret drawn this frame. See `caret_affinity.rs` for why it is found this way.
fn caret_top(editor: &mut Editor<Grid>) -> f32 {
    let caret = Theme::default().caret;
    let wanted = [caret.r, caret.g, caret.b, caret.a];
    let tops: Vec<f32> = editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted && quad.rect[3] > quad.rect[2])
        .map(|quad| quad.rect[1])
        .collect();
    assert_eq!(tops.len(), 1, "expected exactly one caret, got {tops:?}");
    tops[0]
}

/// The caret top at offset 0, which is row 0 of any document, to measure other rows against.
fn reference(editor: &mut Editor<Grid>) -> f32 {
    let before = editor.document().selection();
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::ZERO));
    let top = caret_top(editor);
    editor.document_mut().set_selection(before);
    top
}

/// Which row the caret is drawn on, without touching the selection.
fn caret_row(editor: &mut Editor<Grid>, reference: f32) -> i64 {
    let top = caret_top(editor);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a row index in a 600 px window"
    )]
    {
        ((top - reference) / LINE_HEIGHT).round() as i64
    }
}

// ---- the four keys are four commands ----

#[test]
fn home_and_end_stay_on_the_carets_line() {
    let mut editor = editor(LINES);
    caret_at(&mut editor, MIDDLE);
    assert!(editor.press_home(false));
    assert_eq!(head(&editor), SECOND_START);

    caret_at(&mut editor, MIDDLE);
    assert!(editor.press_end(false));
    assert_eq!(
        head(&editor),
        SECOND_END,
        "before the newline, not after it"
    );
}

/// **The criterion that was missing**: from the same caret, the plain keys and the chords land in
/// four different places.
#[test]
fn the_chords_reach_the_documents_ends_and_the_keys_do_not() {
    let mut editor = editor(LINES);
    let mut landed = Vec::new();
    for press in 0..4 {
        caret_at(&mut editor, MIDDLE);
        match press {
            0 => editor.press_home(false),
            1 => editor.press_end(false),
            2 => editor.press_document_edge(false, false),
            _ => editor.press_document_edge(true, false),
        };
        landed.push(head(&editor));
    }
    assert_eq!(landed, vec![SECOND_START, SECOND_END, 0, LINES.len()]);
}

#[test]
fn shift_extends_the_selection_from_where_it_was_anchored() {
    let mut editor = editor(LINES);
    caret_at(&mut editor, MIDDLE);
    editor.press_home(true);
    let selection = editor.document().selection();
    assert_eq!(selection.anchor().get(), MIDDLE);
    assert_eq!(selection.head().get(), SECOND_START);

    editor.press_end(true);
    let selection = editor.document().selection();
    assert_eq!(selection.anchor().get(), MIDDLE, "the anchor stays put");
    assert_eq!(selection.head().get(), SECOND_END);

    editor.press_document_edge(true, true);
    assert_eq!(editor.document().selection().anchor().get(), MIDDLE);
    assert_eq!(head(&editor), LINES.len());
}

/// Home at the row's start and End at its end stay where they are, rather than stepping on to the
/// next row as an arrow would.
#[test]
fn pressing_again_stays_put() {
    let mut editor = editor(LINES);
    caret_at(&mut editor, MIDDLE);
    editor.press_home(false);
    editor.press_home(false);
    assert_eq!(head(&editor), SECOND_START);

    editor.press_end(false);
    editor.press_end(false);
    assert_eq!(head(&editor), SECOND_END);
}

/// Before the first frame there are no rows to be on, so nothing moves.
#[test]
fn before_the_first_frame_nothing_moves() {
    let mut editor = Editor::new(LINES, Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(MIDDLE)));
    assert!(!editor.press_home(false));
    assert!(!editor.press_end(false));
    assert_eq!(head(&editor), MIDDLE);
}

// ---- visual rows ----

/// A wrapped line's continuation has a start of its own, and Home goes there rather than to the
/// start of the source line.
#[test]
fn home_on_a_continuation_row_goes_to_that_rows_start() {
    let text = long();
    let mut editor = editor(&text);
    let reference = reference(&mut editor);
    caret_at(&mut editor, 150);
    let row = caret_row(&mut editor, reference);
    assert!(row >= 1, "150 bytes in should be on a continuation row");

    editor.press_home(false);
    let start = head(&editor);
    assert!(0 < start && start <= 150, "landed at {start}");
    assert_eq!(caret_row(&mut editor, reference), row, "on the same row");

    // And from anywhere else on that row Home lands on the same byte, so it is the row's start
    // rather than wherever the hit test happened to round to.
    caret_at(&mut editor, start + 3);
    editor.press_home(false);
    assert_eq!(head(&editor), start);
}

/// **End on a wrapped row stays drawn on that row**, which is MDEDIT-188's affinity from the
/// keyboard: the row's end is the same byte as the next row's start.
#[test]
fn end_on_a_wrapped_row_stays_on_that_row() {
    let text = long();
    let mut editor = editor(&text);
    let reference = reference(&mut editor);
    caret_at(&mut editor, 150);
    let row = caret_row(&mut editor, reference);

    editor.press_end(false);
    let end = head(&editor);
    assert!(150 <= end && end < text.len(), "landed at {end}");
    assert_eq!(editor.document().selection().affinity(), Affinity::Upstream);
    assert_eq!(
        caret_row(&mut editor, reference),
        row,
        "drawn on the row it ended"
    );

    // And Home from there goes back to this row's start, not the next one's: the affinity is what
    // says which row the caret is on.
    editor.press_home(false);
    assert_eq!(caret_row(&mut editor, reference), row);
    assert!(head(&editor) < end);
}

/// Typing after End writes at the byte End put the caret on.
#[test]
fn typing_after_end_writes_at_the_rows_end() {
    let text = long();
    let mut editor = editor(&text);
    caret_at(&mut editor, 150);
    editor.press_end(false);
    let end = head(&editor);

    editor.insert_text("X");
    let after = editor.document().to_string();
    assert_eq!(&after[end..=end], "X");
    assert_eq!(&after[..end], &text[..end]);
}

/// The last row of a wrapped line ends where the line does, and no next row shares that byte, so
/// the affinity is the ordinary one.
#[test]
fn end_on_the_last_row_of_a_wrapped_line_reaches_the_lines_end() {
    let text = format!("{}\nnext\n", long().trim_end());
    let line_end = text.find('\n').unwrap();
    let mut editor = editor(&text);
    caret_at(&mut editor, line_end - 2);
    editor.press_end(false);
    assert_eq!(head(&editor), line_end);
    assert_eq!(
        editor.document().selection().affinity(),
        Affinity::Downstream
    );
}

// ---- concealed markers ----

/// **In preview, Home stops after the marker**, because a caret before a concealed marker draws at
/// the same x as its far edge and every motion keeps it out (`past_concealed`).
#[test]
fn home_in_preview_lands_after_a_concealed_marker() {
    for (source, caret, start) in [
        ("# Title\n", 5, 2),
        ("- an item\n", 6, 2),
        ("1. a number\n", 6, 3),
        ("- [ ] a task\n", 9, 6),
        ("> quoted\n", 5, 2),
    ] {
        let mut editor = editor(source);
        assert_eq!(editor.mode(), Mode::Preview);
        caret_at(&mut editor, caret);
        editor.press_home(false);
        assert_eq!(head(&editor), start, "{source:?}");
    }
}

/// Plain markup conceals nothing, so Home reaches the row's true start.
#[test]
fn home_in_plain_markup_reaches_the_marker() {
    for (source, caret) in [("# Title\n", 5), ("- [ ] a task\n", 9), ("> quoted\n", 5)] {
        let mut editor = editor(source);
        editor.apply_chord("/", false);
        assert_eq!(editor.mode(), Mode::Plain);
        caret_at(&mut editor, caret);
        editor.press_home(false);
        assert_eq!(head(&editor), 0, "{source:?}");
    }
}

/// End is the same in both modes on an ordinary line: nothing concealed sits after the text.
#[test]
fn end_reaches_the_lines_end_in_both_modes() {
    for plain in [false, true] {
        let mut editor = editor("# Title\nnext\n");
        if plain {
            editor.apply_chord("/", false);
        }
        caret_at(&mut editor, 3);
        editor.press_end(false);
        assert_eq!(head(&editor), 7, "plain: {plain}");
    }
}
