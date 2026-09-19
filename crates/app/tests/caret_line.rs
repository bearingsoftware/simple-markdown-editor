//! The caret's line, lit (MDEDIT-77).
//!
//! Two cues for one question — "which line am I on". A tint across the row, and the gutter number
//! stepping up from `gutter` to `text`. The tint is a 7% lavender, which is deliberately near
//! invisible and is exactly why the second cue exists: a reader who cannot see it still sees which
//! number is brighter.
//!
//! The decisions this file pins are the ones the ticket asked to be made explicitly: every row of a
//! wrapped line is lit, the highlight survives a selection, and it survives the find field taking
//! focus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // Quad colours come from the same palette constants the renderer read.
    clippy::float_cmp
)]

use mdedit_app::config::Config;
use mdedit_app::editor::Editor;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::view::{Rgba, Theme, ThemeChoice};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = MARGIN + mdedit_app::toolbar::HEIGHT + 400.0;

fn editor(source: &str) -> Editor<Grid> {
    let mut editor = Editor::new(source, Grid::new());
    editor.with_config(Config {
        line_numbers: true,
        // **Pinned, not left to `system`.** These assertions name colours from `Theme::dark`, and
        // since MDEDIT-78 a config with `theme: "system"` resolves against the *machine* — so on a
        // light-themed Windows, which is what a CI runner is, they would be looking for a palette
        // the editor is not drawing with.
        theme: ThemeChoice::Dark,
        ..Config::default()
    });
    // One frame, so the content rect is decided before any test reads it.
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn matching(quads: &[QuadInstance], colour: Rgba) -> Vec<&QuadInstance> {
    let wanted = [colour.r, colour.g, colour.b, colour.a];
    quads.iter().filter(|quad| quad.colour == wanted).collect()
}

/// Which rows carry the caret-line tint, counted from the top of the document.
fn lit_rows(editor: &mut Editor<Grid>) -> Vec<usize> {
    // A frame first: the content rect is decided by drawing one, and reading it before is reading
    // the default.
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let top = editor
        .content_rect()
        .to_window(0.0, 0.0, mdedit_app::content::Offset::down(0.0))
        .1;
    let mut rows: Vec<usize> = matching(&quads, Theme::default().caret_line)
        .iter()
        .map(|quad| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let row = ((quad.rect[1] - top) / LINE_HEIGHT).round().max(0.0) as usize;
            row
        })
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

fn put_caret(editor: &mut Editor<Grid>, at: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(at)));
}

#[test]
fn the_carets_row_is_the_one_that_is_lit() {
    let mut editor = editor("alpha\nbeta\ngamma\n");
    assert_eq!(lit_rows(&mut editor), vec![0]);

    put_caret(&mut editor, "alpha\nbe".len());
    assert_eq!(lit_rows(&mut editor), vec![1]);

    put_caret(&mut editor, "alpha\nbeta\ng".len());
    assert_eq!(lit_rows(&mut editor), vec![2]);
}

/// It follows a click as well as a set caret — the same path the user actually takes.
#[test]
fn it_follows_a_click() {
    let mut editor = editor("alpha\nbeta\ngamma\n");
    let left = editor.content_rect().left_edge();
    let top = editor
        .content_rect()
        .to_window(0.0, 0.0, mdedit_app::content::Offset::down(0.0))
        .1;

    editor.click_at(left + 2.0 * ADVANCE, top + 2.0f32.mul_add(LINE_HEIGHT, 5.0));
    assert_eq!(lit_rows(&mut editor), vec![2]);
}

/// **The decision.** A line is a source line — the gutter numbers it once and the status bar counts
/// it once — so the highlight lights every row of a wrapped one. Lighting only the row the caret sat
/// on would answer a question nobody asked, and is not what any editor does.
#[test]
fn every_row_of_a_wrapped_line_is_lit() {
    // Long enough to wrap several times at the stub's fixed advance.
    let long = "word ".repeat(60);
    let mut editor = editor(&format!("{long}\nsecond line\n"));
    put_caret(&mut editor, 3);

    let lit = lit_rows(&mut editor);
    assert!(
        lit.len() > 1,
        "the first line wraps, so more than one row belongs to it: {lit:?}"
    );
    assert!(
        lit.iter().enumerate().all(|(index, row)| *row == index),
        "the lit rows should be the wrapped line's, from the top: {lit:?}"
    );
}

/// **The decision.** The document's caret has not moved while the find field has focus — it is where
/// Esc puts you back — so the highlight stays. Blinking it off for the duration of a search would be
/// motion with no information in it.
#[test]
fn it_stays_lit_while_the_find_field_has_focus() {
    let mut editor = editor("alpha\nbeta\ngamma\n");
    put_caret(&mut editor, "alpha\nbe".len());

    assert!(editor.open_find());
    assert!(editor.find_focused());
    assert_eq!(lit_rows(&mut editor), vec![1]);
}

/// Under everything: the selection tint, the search highlights and the glyphs all go on top, because
/// the painter's order is the whole of the z-order here.
#[test]
fn the_tint_is_drawn_under_the_selection() {
    let mut editor = editor("alpha\nbeta\n");
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(0), ByteOffset::new(3)));

    let theme = Theme::default();
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let wanted = [
        theme.caret_line.r,
        theme.caret_line.g,
        theme.caret_line.b,
        theme.caret_line.a,
    ];
    let selection = [
        theme.selection.r,
        theme.selection.g,
        theme.selection.b,
        theme.selection.a,
    ];
    let tint = quads
        .iter()
        .position(|quad| quad.colour == wanted)
        .expect("the caret's line is lit");
    let selected = quads
        .iter()
        .position(|quad| quad.colour == selection)
        .expect("there is a selection to draw");
    assert!(
        tint < selected,
        "the caret-line tint must be pushed before the selection, or it covers it"
    );
}

// ---- the gutter's own cue ----

#[test]
fn the_carets_number_steps_up_and_the_others_do_not() {
    let theme = Theme::default();
    let mut editor = editor("alpha\nbeta\ngamma\n");
    put_caret(&mut editor, "alpha\nbe".len());

    let text_left = editor.content_rect().left_edge();
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let in_gutter = |colour: Rgba| {
        matching(&quads, colour)
            .iter()
            .filter(|quad| quad.rect[0] + quad.rect[2] <= text_left)
            .count()
    };

    assert!(
        in_gutter(theme.text) > 0,
        "the caret's line number should be drawn in the text colour"
    );
    assert!(
        in_gutter(theme.gutter) > 0,
        "and the other numbers in the gutter colour"
    );
    assert_eq!(
        in_gutter(theme.marker),
        0,
        "the gutter had been borrowing the marker colour; it has its own role now"
    );
}

/// Something wrong outranks where you are. A flagged line that also holds the caret stays rose.
#[test]
fn a_flagged_line_stays_flagged_when_the_caret_is_on_it() {
    let theme = Theme::default();
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("data.json");
    std::fs::write(&path, "{\n  \"a\": 1,\n}\n").unwrap();
    let mut workspace = mdedit_session::Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.with_config(Config {
        line_numbers: true,
        // Pinned, for the reason the helper above gives.
        theme: ThemeChoice::Dark,
        ..Config::default()
    });
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.on_idle();

    // The trailing comma is on the second line; put the caret there too.
    put_caret(&mut editor, "{\n  \"a\"".len());

    let text_left = editor.content_rect().left_edge();
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let rose = matching(&quads, theme.span_error)
        .iter()
        .filter(|quad| quad.rect[0] + quad.rect[2] <= text_left)
        .count();
    assert!(
        rose > 0,
        "the error colour outranks the caret's line in the gutter"
    );
}
