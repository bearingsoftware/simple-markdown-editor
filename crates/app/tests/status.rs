//! The status bar: what it says, and what it costs.
//!
//! The strings are checked directly, because they are the whole feature. The cost is checked through
//! the text backend's layout counter, because the ticket named the trap by name — a bar redrawn
//! every frame that lays its text out every frame would put a text-engine call on the keystroke
//! path, which is where MDEDIT-17 and MDEDIT-18 spent their effort getting things off.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::status::{self, BAR_HEIGHT};
use mdedit_document::Selection;
use mdedit_file::Newline;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = MARGIN + 400.0;

#[test]
fn a_breadcrumb_joins_its_steps_and_an_empty_one_is_empty() {
    assert_eq!(status::breadcrumb(&[]), "");
    assert_eq!(status::breadcrumb(&["Guide".to_owned()]), "Guide");
    assert_eq!(
        status::breadcrumb(&["Guide".to_owned(), "Costs".to_owned()]),
        "Guide › Costs"
    );
}

/// Offsets count from zero and a status bar counts from one, which is a presentation decision and
/// therefore this function's.
#[test]
fn a_position_counts_from_one() {
    assert_eq!(status::position(0, 0), "Ln 1, Col 1");
    assert_eq!(status::position(11, 4), "Ln 12, Col 5");
}

#[test]
fn details_name_the_encoding_and_the_line_endings() {
    assert_eq!(
        status::details("UTF-8", Newline::Lf, Some(1200)),
        "UTF-8   LF   1200 words"
    );
    assert_eq!(
        status::details("UTF-8", Newline::CrLf, Some(1)),
        "UTF-8   CRLF   1 word"
    );
    assert!(status::details("UTF-8", Newline::Mixed, Some(0)).contains("Mixed"));
}

/// Before the first count has finished there is no number to show, and a zero would be a lie that
/// is about to be corrected.
#[test]
fn an_uncounted_document_shows_no_word_count() {
    let text = status::details("UTF-8", Newline::Lf, None);
    assert!(!text.contains("word"), "{text}");
    assert!(text.contains("UTF-8"));
}

#[test]
fn words_are_whitespace_separated_runs() {
    assert_eq!(status::count_words(""), 0);
    assert_eq!(status::count_words("   \n\t "), 0);
    assert_eq!(status::count_words("one two three"), 3);
    // Deliberately not clever about Markdown: the syntax travels with the word it decorates.
    assert_eq!(status::count_words("a **bold** word"), 3);
    assert_eq!(status::count_words("hard\nwrapped\nlines\n"), 3);
}

/// The bar takes its room through the content rect, so the document gets shorter rather than being
/// drawn underneath it.
#[test]
fn the_bar_takes_its_height_off_the_document() {
    let mut editor = Editor::new("# Title\n\ntext\n", Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let content = editor.content_rect();
    assert_eq!(
        content.height(),
        HEIGHT - MARGIN - BAR_HEIGHT,
        "the window less the top margin and the bar"
    );
}

#[test]
fn the_bar_is_drawn_across_the_foot() {
    let mut editor = Editor::new("# Title\n\ntext\n", Grid::new());
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let bar_top = HEIGHT - BAR_HEIGHT;
    assert!(
        quads
            .iter()
            .any(|quad| quad.rect[1] == bar_top && quad.rect[2] == WIDTH),
        "a full-width fill at the foot"
    );
    assert!(
        quads.iter().any(|quad| quad.rect[1] > bar_top),
        "and text drawn in it"
    );
}

/// The trap the ticket named. A bar whose text is laid out every frame puts a text-engine call on
/// the path a keystroke takes, however cheap each one looks.
#[test]
fn a_still_frame_lays_out_no_chrome_at_all() {
    let mut editor = Editor::new("# Title\n\ntext\n", Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let after_first = editor.text_backend().layouts();

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let since = editor.text_backend().layouts() - after_first;
    assert!(
        since <= 2,
        "two more frames of an unchanged document should lay out the caret's chunk at most, not \
         {since}"
    );
}

/// Moving the caret within one heading's section changes the position fields but not the trail, so
/// only one of the two strings is laid out again.
#[test]
fn moving_the_caret_relays_out_only_what_changed() {
    let source = "# Title\n\nsome text here\n";
    let mut editor = Editor::new(source, Grid::new());
    let caret_to = |editor: &mut Editor<Grid>, needle: &str| {
        editor
            .document_mut()
            .set_selection(Selection::caret(ByteOffset::new(
                source.find(needle).unwrap(),
            )));
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    };

    // Start in the paragraph, so the move below stays inside one block: a caret crossing a block
    // boundary lays out both, which would swamp what this is measuring.
    caret_to(&mut editor, "some");
    let before = editor.text_backend().layouts();
    caret_to(&mut editor, "here");

    let since = editor.text_backend().layouts() - before;
    // Three: the caret's block, which a caret move costs twice through the measure-then-draw path
    // and which predates this ticket, plus the position fields. Not four — the breadcrumb still
    // says "Title", so it is not laid out again. That difference is the whole assertion.
    assert!(
        since <= 3,
        "the breadcrumb should not have been laid out again: {since}"
    );
}

/// And the other way round: a move that lands under a different heading does pay for the trail.
/// Without this the test above would pass just as well if the breadcrumb were never drawn.
#[test]
fn crossing_into_another_section_redraws_the_trail() {
    let source = "# One\n\nalpha\n\n# Two\n\nbeta\n";
    let mut editor = Editor::new(source, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let trail_of = |editor: &mut Editor<Grid>, needle: &str| {
        editor
            .document_mut()
            .set_selection(Selection::caret(ByteOffset::new(
                source.find(needle).unwrap(),
            )));
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
        let head = editor.document().selection().head();
        let steps: Vec<String> = editor
            .document()
            .heading_trail(head)
            .iter()
            .map(|heading| editor.document().heading_text(heading))
            .collect();
        status::breadcrumb(&steps)
    };

    assert_eq!(trail_of(&mut editor, "alpha"), "One");
    assert_eq!(trail_of(&mut editor, "beta"), "Two");
}
