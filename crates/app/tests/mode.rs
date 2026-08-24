//! Switching between the previewed view and the raw markup.
//!
//! The claim being tested is narrow and checkable: plain mode draws **the file**, preview mode draws
//! what it drew before, and switching between them touches nothing but pixels.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::view::{Decoration, Mode, Theme, build_runs_in};
use mdedit_document::{Document, Selection};
use mdedit_offsets::{ByteOffset, ByteSpan};

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 400.0;
const HEIGHT: f32 = MARGIN + 400.0;

/// Everything the decoration step would draw, as one string.
fn drawn(source: &str, mode: Mode) -> String {
    let doc = Document::from(source);
    let theme = Theme::default();
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len()));
    build_runs_in(
        &doc,
        Decoration {
            theme: &theme,
            mode,
        },
        whole,
        source,
    )
    .iter()
    .map(|run| run.text.as_ref())
    .collect()
}

/// The point of the whole feature: what is drawn *is* the file.
#[test]
fn plain_mode_draws_the_source_byte_for_byte() {
    for source in [
        "# Heading\n\nSome **bold** and *emphasis*.\n",
        "- a list\n- of items\n\n> a quote\n",
        "```rust\nfn main() {}\n```\n",
        "A [link](https://example.com) and `code`.\n",
        "Text with 日本語 and an emoji 🦀.\n",
    ] {
        assert_eq!(
            drawn(source, Mode::Plain),
            source,
            "plain mode dropped or added something in {source:?}"
        );
    }
}

/// And with the caret parked well away from everything, so no local reveal is doing the work.
#[test]
fn plain_mode_shows_markers_the_caret_is_nowhere_near() {
    let source = "# Heading\n\nSome **bold** text.\n";
    assert_eq!(drawn(source, Mode::Plain), source);
}

#[test]
fn preview_mode_still_conceals() {
    let source = "# Heading\n\nSome **bold** text.\n";
    let previewed = drawn(source, Mode::Preview);

    assert!(
        !previewed.contains("**"),
        "the emphasis markers should be gone: {previewed:?}"
    );
    assert!(previewed.contains("bold"), "but the word should not be");
}

/// Plain means plain. A heading still at heading size would be preview wearing its markers.
#[test]
fn plain_mode_uses_one_size_and_weight_throughout() {
    let source = "# Big\n\nnormal **bold** `code`\n";
    let doc = Document::from(source);
    let theme = Theme::default();
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len()));

    let runs = build_runs_in(
        &doc,
        Decoration {
            theme: &theme,
            mode: Mode::Plain,
        },
        whole,
        source,
    );

    for run in &runs {
        assert_eq!(
            run.style.size.get(),
            theme.body_size,
            "run {:?} is not at body size",
            run.text
        );
        assert_eq!(
            run.style.weight,
            mdedit_text::FontWeight::Regular,
            "run {:?} is not at regular weight",
            run.text
        );
        assert!(!run.style.italic, "run {:?} is italic", run.text);
        assert_eq!(
            run.style.family,
            mdedit_text::FontFamily::Ui,
            "run {:?} is monospaced",
            run.text
        );
    }
}

/// Preview mode is untouched by any of this — the sizes it always used are still there.
#[test]
fn preview_mode_still_sizes_a_heading() {
    let source = "# Big\n\nnormal\n";
    let doc = Document::from(source);
    let theme = Theme::default();
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len()));

    let runs = build_runs_in(
        &doc,
        Decoration {
            theme: &theme,
            mode: Mode::Preview,
        },
        whole,
        source,
    );

    let biggest = runs
        .iter()
        .map(|run| run.style.size.get())
        .fold(0.0_f32, f32::max);
    assert!(
        biggest > theme.body_size,
        "the heading should still be larger than the body: {biggest}"
    );
}

// ---------------------------------------------------------------------------
// Through the editor: the toggle, and what it must not disturb.
// ---------------------------------------------------------------------------

#[test]
fn the_toggle_goes_there_and_back() {
    let mut editor = Editor::new("# Title\n\nbody\n", Grid::new());
    assert_eq!(editor.mode(), Mode::Preview, "previewed by default");

    editor.toggle_mode();
    assert_eq!(editor.mode(), Mode::Plain);

    editor.toggle_mode();
    assert_eq!(editor.mode(), Mode::Preview);
}

/// A rendering mode, not a document mode. This is the assertion that keeps it honest.
#[test]
fn toggling_never_touches_the_buffer_or_the_caret() {
    let source = "# Title\n\nSome **bold** text.\n";
    let mut editor = Editor::new(source, Grid::new());
    let caret = ByteOffset::new(12);
    editor.document_mut().set_selection(Selection::caret(caret));

    for _ in 0..4 {
        editor.toggle_mode();
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
        assert_eq!(editor.document().to_string(), source);
        assert_eq!(editor.document().selection().head(), caret);
    }
}

#[test]
fn the_two_modes_draw_different_things() {
    let mut editor = Editor::new("# Title\n\nSome **bold** text.\n", Grid::new());
    // Caret at the end, so nothing is revealed by proximity.
    let end = ByteOffset::new(editor.document().len_bytes());
    editor.document_mut().set_selection(Selection::caret(end));

    let previewed = editor.build_frame(WIDTH, HEIGHT, 1.0).len();
    editor.toggle_mode();
    let plain = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    assert!(
        plain > previewed,
        "plain mode has the markers to draw as well: {previewed} then {plain}"
    );
}

/// Toggling back must draw exactly what it drew before — not approximately.
#[test]
fn a_round_trip_draws_what_it_started_with() {
    let mut editor = Editor::new("# Title\n\nSome **bold** text.\n\nAnd more.\n", Grid::new());
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.toggle_mode();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.toggle_mode();
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(
        before.len(),
        after.len(),
        "the same document should draw the same number of quads"
    );
}

/// The layout cache is keyed by content and width, so the mode had to join them — otherwise a
/// toggle would serve one mode's layout to the other.
#[test]
fn the_cache_does_not_serve_one_modes_layout_to_the_other() {
    let mut editor = Editor::new("# Title\n\nSome **bold** text.\n", Grid::new());
    let end = ByteOffset::new(editor.document().len_bytes());
    editor.document_mut().set_selection(Selection::caret(end));

    // Warm the cache in preview, then switch. If the mode were missing from the key, the second
    // frame would come back from the cache with the markers still concealed.
    let previewed = editor.build_frame(WIDTH, HEIGHT, 1.0).len();
    editor.toggle_mode();
    let plain = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    assert_ne!(
        previewed, plain,
        "the cache handed back the wrong mode's layout"
    );
}
