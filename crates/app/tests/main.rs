//! Every integration test of `mdedit-app`, as one test binary (MDEDIT-182).
//!
//! **One binary, not one per file, because each binary is a full link of the editor.** Cargo builds
//! every file under `tests/` as its own crate, and every one of them depends on `mdedit-app`, so a
//! change to the library recompiles and relinks all of them — seventy-five links of wgpu, winit and
//! the rest. That is what a mutant costs in `cargo-mutants`, once per mutant: measured on MDEDIT-182,
//! rebuilding the tests after one change to `editor.rs` took **135 s as seventy-five binaries and
//! 18 s as one**. The mutants job had been running out of time on every large pull request and
//! holding the shared runner pool while it did.
//!
//! Nothing about how a test runs changes: nextest still gives every test its own process. What
//! changes is the name — `mdedit-app::app quotes::a_quote_draws_one_bar` rather than
//! `mdedit-app::quotes a_quote_draws_one_bar` — and how to run one file's tests:
//! `cargo nextest run -p mdedit-app -E 'test(/^quotes::/)'`.
//!
//! **A new test file must be declared below**, or it is never compiled. Cargo's own discovery is
//! off (`autotests = false`) because it would otherwise build each file a second time as its own
//! binary. `every_test_file_is_declared_here` fails on a file that was forgotten, which is the
//! failure that would otherwise be silent: a test that does not exist cannot go red.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

mod chordlist;
mod stub;

mod blank_page;
mod blocks;
mod caret_line;
mod chords;
mod clipboard;
mod close;
mod concealed_caret;
mod config;
mod content;
mod context_menu;
mod contrast;
mod diagnostics;
mod dialog;
mod encodings;
mod endings;
mod external;
mod field;
mod file_commands;
mod find;
mod find_cost;
mod findbar;
mod format;
mod frame;
mod frame_cost;
mod geometry;
mod gutter;
mod icons;
mod image_placement;
mod images;
mod ime;
mod ini;
mod italics;
mod json;
mod json_typing_cost;
mod latency;
mod list_cost;
mod lists;
mod log;
mod marker_colours;
mod mode;
mod open;
mod pane;
mod path_tooltips;
mod plain;
mod pointer;
mod popup;
mod quotes;
mod recovery;
mod render;
mod replace;
mod scroll;
mod scrollbar;
mod selection_clicks;
mod shortcuts;
mod status;
mod strikethrough;
mod tab_overflow;
mod table;
mod table_clicks;
mod table_menu;
mod table_metrics;
mod tabs;
mod tasks;
mod thematic_break;
mod theme_switch;
mod toml;
mod toolbar;
mod tooltip;
mod validate_cost;
mod view;
mod view_menu;
mod whitespace;
mod wrap;
mod wrap_cost;
mod wrapped_caret;
mod xml;
mod zoom;

/// **Every file in `tests/` is declared above.** With Cargo's discovery off, a file nobody added to
/// this list is not compiled, and its tests pass by not existing — the one failure this layout adds,
/// so it is the one it checks for.
#[test]
fn every_test_file_is_declared_here() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let this = include_str!("main.rs");
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        if stem != "main" && !this.contains(&format!("\nmod {stem};")) {
            missing.push(stem);
        }
    }
    assert!(
        missing.is_empty(),
        "not declared in tests/main.rs, so never compiled: {missing:?}"
    );
}
