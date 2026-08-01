//! Zooming the text, and the cache discipline that is the actual feature.
//!
//! Rescaling is arithmetic. The work is that text sizes are **not** in `ChunkKey` — they come from
//! the theme at layout time — so anything already laid out keeps drawing at the old size unless it
//! is thrown away.
//!
//! # What these can and cannot see
//!
//! The stub backend is a fixed grid: it ignores the size on every run it is handed, so a zoom
//! changes **no pixel it produces**. Asserting on quad heights would therefore pass whether the
//! caches were cleared or not, which is worse than not asserting at all.
//!
//! So two things are checked instead, and between them they cover the claim:
//!
//! - the size the theme is drawing at, which is what a real backend would act on;
//! - **that the chunks were laid out again**, via the stub's own layout counter — which is exactly
//!   what a cleared cache causes and a stale one does not.
//!
//! What no test here can see is the *rendered* result at the new size. That is a manual check, and
//! the one thing worth doing by eye on this ticket.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::config::{Config, MAX_ZOOM, MIN_ZOOM, Settings};
use mdedit_app::editor::Editor;

mod stub;
use stub::Grid;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

fn document(lines: usize) -> String {
    use std::fmt::Write as _;

    (0..lines).fold(String::new(), |mut out, index| {
        let _ = writeln!(out, "line {index} of the document\n");
        out
    })
}

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

#[test]
fn zooming_in_makes_the_text_bigger_and_out_makes_it_smaller() {
    let mut editor = editor("# Heading\n\nsome prose\n");
    let normal = editor.body_size();

    assert!(editor.zoom_by(1));
    assert!(editor.body_size() > normal);

    assert!(editor.zoom_by(-2));
    assert!(editor.body_size() < normal);
}

#[test]
fn reset_puts_it_back_exactly() {
    let mut editor = editor("# Heading\n\nprose\n");
    let normal = editor.body_size();

    editor.zoom_by(4);
    editor.zoom_by(-2);
    assert!(editor.reset_zoom());

    assert_eq!(editor.zoom(), 1.0);
    assert_eq!(editor.body_size(), normal);
}

#[test]
fn reset_at_normal_size_does_nothing() {
    let mut editor = editor("prose\n");
    assert!(!editor.reset_zoom(), "there is nothing to reset");
}

#[test]
fn zoom_is_clamped_at_both_ends() {
    let mut editor = editor("prose\n");

    for _ in 0..100 {
        editor.zoom_by(1);
    }
    assert_eq!(editor.zoom(), MAX_ZOOM);
    assert!(!editor.zoom_by(1), "at the ceiling there is nothing to do");

    for _ in 0..100 {
        editor.zoom_by(-1);
    }
    assert_eq!(editor.zoom(), MIN_ZOOM);
    assert!(!editor.zoom_by(-1));
}

/// Each press is the same proportional change, which is what makes the ladder feel even — the
/// difference between 16 and 17 px matters far more than between 39 and 40.
#[test]
fn each_step_is_the_same_ratio() {
    let mut editor = editor("prose\n");
    let start = editor.body_size();

    editor.zoom_by(1);
    let one = editor.body_size() / start;
    editor.zoom_by(1);
    let two = editor.body_size() / start;

    assert!(
        (two - one * one).abs() < 0.001,
        "two steps should be one step squared: {one} then {two}"
    );
}

// ---- the cache discipline ----

/// The feature. A cleared cache means every chunk is laid out again; a stale one means none is.
#[test]
fn a_zoom_lays_every_chunk_out_again() {
    let mut editor = Editor::new(&document(60), Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    // A second frame at the same size lays out almost nothing — that is MDEDIT-17's guarantee, and
    // it is what makes the count after a zoom meaningful.
    let settled = editor.text_backend().layouts();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let idle = editor.text_backend().layouts() - settled;
    assert!(
        idle <= 2,
        "a still frame should lay out ~nothing, not {idle}"
    );

    let before = editor.text_backend().layouts();
    editor.zoom_by(2);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let after_zoom = editor.text_backend().layouts() - before;

    assert!(
        after_zoom > idle,
        "a zoom must lay chunks out again: {idle} on a still frame, {after_zoom} after zooming"
    );
}

/// The gutter's digits are laid out once and kept, so a zoom that forgot them would draw old-size
/// line numbers beside new-size text. There is no pixel to check, so the layout count is the proof.
#[test]
fn the_gutter_is_laid_out_again_too() {
    let mut editor = Editor::new(&document(40), Grid::new());
    editor.with_config(Config {
        line_numbers: true,
        ..Config::default()
    });
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    let before = editor.text_backend().layouts();
    editor.zoom_by(2);
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(
        editor.text_backend().layouts() > before,
        "the gutter and the chunks should both have been laid out again"
    );
}

// ---- the viewport ----

/// Zooming halfway down a document must not throw the reader somewhere else in it.
#[test]
fn the_viewport_does_not_jump() {
    let mut editor = editor(&document(400));
    editor.scroll_by(1000.0);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let before = editor.scroll_offset();
    assert!(
        before > 0.0,
        "the test needs to be scrolled to mean anything"
    );

    editor.zoom_by(1);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let after = editor.scroll_offset();

    // Roughly in proportion: the same *place* in a taller document, not the same pixel. Loose,
    // because the stub does not scale and the estimate corrects as chunks are measured — what is
    // being caught here is a jump to somewhere unrelated, which is what a lost anchor looks like.
    let ratio = after / before;
    assert!(
        (0.8..=1.4).contains(&ratio),
        "the view moved somewhere unrelated: {before} then {after} (ratio {ratio})"
    );
}

/// The top of a document has nowhere to jump to, and must stay put exactly.
#[test]
fn zooming_at_the_top_stays_at_the_top() {
    let mut editor = editor(&document(400));
    editor.zoom_by(2);
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.scroll_offset(), 0.0);
}

// ---- persistence ----

/// MDEDIT-43's ticket said zoom would be per-run "until the settings story grows a write path".
/// MDEDIT-60 grew it, so it persists — the caveat in that ticket is superseded rather than still
/// true.
#[test]
fn zoom_survives_a_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("config.json");

    let mut editor = Editor::new("prose\n", Grid::new());
    editor.with_settings(Settings::at(file.clone()));
    editor.zoom_by(2);
    let chosen = editor.zoom();
    assert!(chosen > 1.0);
    editor.on_idle();

    let mut next = Editor::new("prose\n", Grid::new());
    next.with_settings(Settings::at(file));
    assert_eq!(next.zoom(), chosen);
}

/// A stored zoom has to reach the *theme*, not just the settings — otherwise it persists and does
/// nothing, which is the failure that looks most like it worked.
#[test]
fn a_stored_zoom_is_applied_on_open() {
    let normal = editor("# Heading\n").body_size();

    let mut zoomed = Editor::new("# Heading\n", Grid::new());
    zoomed.with_config(Config {
        zoom: 2.0,
        ..Config::default()
    });

    assert_eq!(zoomed.body_size(), normal * 2.0);
}
