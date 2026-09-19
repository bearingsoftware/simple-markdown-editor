//! A document with nothing in it (MDEDIT-81).
//!
//! An empty buffer is what File → New produces and what a zero-byte file opens as, so it is the
//! first thing a new user sees. It drew **no caret, no line number and no caret-line tint**:
//! `DirectWriteText::layout` returns `Layout::default()` for empty input — no rows at all — and
//! everything the renderer draws is keyed off rows. Keystrokes were accepted and nothing said where
//! they would land.
//!
//! Repaired where the engine's answer arrives rather than in each of the three: `lay_out` gives an
//! empty layout the one empty row a blank page actually has, which is what every editor shows.
//!
//! No headless test could see it, because the stub fabricated a line for empty input. That is
//! corrected in `stub/mod.rs`, which is what makes this file possible at all — the same shape as
//! MDEDIT-46's two defects, where the stub shaped text more simply than DirectWrite does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // Quad colours come from the same constants the renderer read, so equality is exact.
    clippy::float_cmp
)]

use mdedit_app::config::Config;
use mdedit_app::editor::Editor;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::view::{Appearance, Rgba, Theme, ThemeChoice};

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn matching(quads: &[QuadInstance], colour: Rgba) -> Vec<QuadInstance> {
    let wanted = [colour.r, colour.g, colour.b, colour.a];
    quads
        .iter()
        .filter(|quad| quad.colour == wanted)
        .copied()
        .collect()
}

/// The defect, in one line: a blank page has to say where typing will go.
#[test]
fn a_blank_page_draws_a_caret() {
    let mut editor = Editor::new("", Grid::new());
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let carets = matching(&quads, Theme::default().caret);
    assert_eq!(
        carets.len(),
        1,
        "a document with nothing in it draws no caret, so nothing says where typing will land"
    );
}

/// And it is a caret rather than a smear: somewhere sensible, and a line tall.
#[test]
fn the_caret_on_a_blank_page_is_where_the_first_character_would_go() {
    let mut editor = Editor::new("", Grid::new());
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let caret = matching(&quads, Theme::default().caret)[0];

    let content = editor.content_rect();
    assert!(
        (caret.rect[0] - content.left_edge()).abs() < 1.0,
        "the caret should be at the left margin, not at {}",
        caret.rect[0]
    );
    assert!(
        (caret.rect[1] - content.top_edge()).abs() < 1.0,
        "and on the first row, not at {}",
        caret.rect[1]
    );
    assert!(
        caret.rect[3] > 1.0,
        "a caret with no height is not a caret: {}",
        caret.rect[3]
    );
}

/// Typing into it works — which it always did — and the caret keeps up.
#[test]
fn typing_into_a_blank_page_moves_the_caret_along() {
    let mut editor = Editor::new("", Grid::new());
    let before = matching(
        &editor.build_frame(WIDTH, HEIGHT, 1.0),
        Theme::default().caret,
    )[0];

    editor.insert_text("a");
    let after = matching(
        &editor.build_frame(WIDTH, HEIGHT, 1.0),
        Theme::default().caret,
    )[0];

    assert!(
        after.rect[0] > before.rect[0],
        "the caret should have moved right of {}, not to {}",
        before.rect[0],
        after.rect[0]
    );
}

/// **An empty document is a document with one line**, which is the other half of MDEDIT-81: the
/// gutter's `1` and the caret-line tint are both keyed off rows, and a page with no rows had
/// neither.
#[test]
fn a_blank_page_has_a_line_number_and_a_lit_row() {
    let mut editor = Editor::new("", Grid::new());
    editor.with_config(Config {
        line_numbers: true,
        // **Pinned, or this test is about the machine it runs on.** `with_config` resolves
        // `ThemeChoice::System` by reading the registry, so on a light-themed Windows — which is
        // what a CI runner is — the assertions below would be looking for a palette the editor is
        // not drawing with. `caret_line.rs` documents the same trap; this test did not follow it,
        // and main went red for exactly that.
        theme: ThemeChoice::Dark,
        ..Config::default()
    });
    // Says out loud what the pin above is for, so a pin that stopped binding fails here rather
    // than by quietly comparing against the wrong palette.
    assert_eq!(editor.appearance(), Appearance::Dark);

    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(
        matching(&quads, Theme::default().caret_line).len(),
        1,
        "the caret's line is lit on every other document, and a blank one has a caret on it"
    );
    // The gutter draws its digits as glyph quads to the *left* of the content, and the caret's own
    // number is the brighter `text` colour — which on a one-line document is the only number there
    // is (MDEDIT-77).
    let left_edge = editor.content_rect().left_edge();
    let digits: Vec<_> = matching(&quads, Theme::default().text)
        .into_iter()
        .filter(|quad| quad.rect[0] < left_edge)
        .collect();
    assert!(
        !digits.is_empty(),
        "a blank page should still be numbered line 1"
    );
}
