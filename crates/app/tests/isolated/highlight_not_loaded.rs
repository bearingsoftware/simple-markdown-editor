//! The thread that draws never loads a syntax set (MDEDIT-190).
//!
//! **A binary of its own, with one test**, so nothing else in the process can have loaded the set
//! first; `cargo test` runs a binary's tests in one process, and a check that something is *not*
//! loaded is only real if it runs before anything could load it. The worker is held back with
//! `hold_highlighting`, so a loaded set could only have come from the drawing thread.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../stub/mod.rs"]
mod stub;

use mdedit_app::editor::Editor;

#[test]
fn laying_a_frame_out_with_a_pending_fence_never_loads_the_syntax_set() {
    assert!(!mdedit_highlight::is_loaded());

    let mut editor = Editor::new("```rust\nfn main() {}\n```\n", stub::Grid::new());
    editor.hold_highlighting();
    let quads = editor.build_frame(900.0, 600.0, 1.0);
    assert!(!quads.is_empty());
    assert!(
        !mdedit_highlight::is_loaded(),
        "the drawing thread loaded the syntax set"
    );

    editor.start_highlighting();
    assert!(editor.await_highlights(std::time::Duration::from_secs(20)));
    assert!(mdedit_highlight::is_loaded(), "the worker never loaded it");
}
