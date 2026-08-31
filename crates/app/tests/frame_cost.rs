//! What a frame costs on a document with no images in it (MDEDIT-146).
//!
//! Ignored by default, like `find_cost` and `list_cost`: a measurement, not an assertion.
//!
//! MDEDIT-146 puts one string check on every paragraph that is laid out, and one `Vec::clear` on
//! every frame. Neither should be visible, and `AGENTS.md` wants a number rather than that sentence.
//!
//! **Run it paired.** This machine drifts by a factor of three between runs, so a number from this
//! branch compared against a number taken from `main` an hour ago says nothing. Build both and
//! alternate them; the ratio is the only part that means anything.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::time::Instant;

use mdedit_app::editor::Editor;

mod stub;
use stub::Grid;

const ROUNDS: u32 = 20;

/// A document with no image in it, which is the case that must not get slower.
fn prose() -> String {
    "A paragraph of ordinary prose about the alpha channel and other matters.\n\n".repeat(2000)
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn a_frame_of_prose() {
    let source = prose();
    let mut editor = Editor::new(&source, Grid::new());
    // Warm: the first frame measures every visible chunk, and that is not what is being compared.
    editor.build_frame(900.0, 600.0, 1.0);

    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let started = Instant::now();
        let quads = editor.build_frame(900.0, 600.0, 1.0);
        best = best.min(started.elapsed().as_micros());
        assert!(!quads.is_empty());
    }
    println!("frame of prose, best of {ROUNDS}: {best} us");
}

/// The other half of the pair: a document that is nothing but images, so the guard runs and fails on
/// every chunk. Nothing is on disk, so this measures the check and the refusal, not a decode.
#[test]
#[ignore = "a measurement, not an assertion"]
fn a_frame_of_broken_images() {
    let source = "![alt](nowhere.png)\n\n".repeat(2000);
    let mut editor = Editor::new(&source, Grid::new());
    editor.build_frame(900.0, 600.0, 1.0);

    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let started = Instant::now();
        editor.build_frame(900.0, 600.0, 1.0);
        best = best.min(started.elapsed().as_micros());
    }
    println!("frame of broken images, best of {ROUNDS}: {best} us");
}
