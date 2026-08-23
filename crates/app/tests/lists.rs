//! A list looks like a list in preview (MDEDIT-141).
//!
//! Concealing a list marker drew nothing in its place, so a bulleted list was pixel-identical to
//! consecutive paragraphs — no bullet, no number, and no indent, which took nesting with it. The fix
//! substitutes text for the concealed marker rather than dropping its run.
//!
//! Every assertion here is about **which runs exist and where they sit**, which the grid stub answers
//! deterministically. Whether a real font's bullet collides with the text beside it is a different
//! question, and MDEDIT-133's is the harness for it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::view::{Theme, ViewRun, build_runs};
use mdedit_document::{Document, Selection};
use mdedit_offsets::ByteOffset;
use mdedit_text::{Layout, TextLayout};

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

fn runs_of(source: &str) -> Vec<ViewRun<'_>> {
    build_runs(&Document::from(source), &Theme::default(), source)
}

/// Everything drawn, concatenated — the preview as a reader sees it.
fn drawn(source: &str) -> String {
    runs_of(source)
        .iter()
        .map(|run| run.text.as_ref())
        .collect()
}

fn laid_out(source: &str) -> Layout {
    let runs = runs_of(source);
    let styled = mdedit_app::view::as_styled(&runs);
    Grid::new()
        .layout(&styled, f32::MAX)
        .expect("the stub lays out anything")
}

/// Where the first glyph that came from the buffer sits on `row`.
///
/// A substituted glyph is told from a real one by its empty source: it stands for bytes rather than
/// being them, which is exactly the distinction "where does the *text* start" needs.
fn text_left(layout: &Layout, row: usize) -> f32 {
    layout.lines[row]
        .glyphs
        .iter()
        .find(|glyph| !glyph.source.is_empty())
        .expect("the row has text of its own")
        .pos
        .x
}

// ---- what is drawn ----

#[test]
fn a_bulleted_item_draws_a_bullet() {
    assert_eq!(drawn("- alpha\n"), "\u{2022} alpha\n");
    assert_eq!(drawn("* alpha\n"), "\u{2022} alpha\n");
    assert_eq!(drawn("+ alpha\n"), "\u{2022} alpha\n");
}

/// CommonMark keeps the author's start number rather than renumbering from one, so the bytes are
/// both the right answer and the easy one. The work is not discarding them.
#[test]
fn a_numbered_item_keeps_the_authors_number() {
    assert_eq!(drawn("7. seven\n8. eight\n"), "7. seven\n8. eight\n");
    assert_eq!(drawn("12) twelve\n"), "12) twelve\n");
}

/// The marker's leading whitespace is part of the marker, so carrying it into the substitute is the
/// whole of what makes nesting visible — and a deeper level gets its own bullet.
#[test]
fn a_nested_item_starts_further_right_than_its_parent() {
    let source = "- parent\n  - child\n";
    assert_eq!(drawn(source), "\u{2022} parent\n  \u{25e6} child\n");

    let layout = laid_out(source);
    assert!(
        text_left(&layout, 1) > text_left(&layout, 0),
        "the child's text should be indented past the parent's: {:?} vs {:?}",
        text_left(&layout, 1),
        text_left(&layout, 0),
    );
}

// ---- what must not change ----

/// The rule that outranks everything: the bullet is drawn, never inserted.
#[test]
fn the_marker_bytes_are_unchanged() {
    let source = "- alpha\n";
    let document = Document::from(source);
    let runs = build_runs(&document, &Theme::default(), source);

    assert_eq!(document.to_string(), source, "the buffer was rewritten");
    let bullet = runs
        .iter()
        .find(|run| run.text.contains('\u{2022}'))
        .expect("a bullet was drawn");
    assert_eq!(
        bullet.source.slice(source),
        "- ",
        "the substituted run must still name the bytes it stands for",
    );
}

/// MDEDIT-118, on the new run. A substituted glyph occupies width, and the caret must not be able to
/// land in the middle of the bytes it stands for.
#[test]
fn the_caret_steps_over_the_substituted_marker() {
    let layout = laid_out("- alpha\n");
    let substituted: Vec<_> = layout.lines[0]
        .glyphs
        .iter()
        .filter(|glyph| glyph.source.is_empty())
        .collect();
    assert_eq!(substituted.len(), 2, "the bullet and the space after it");
    assert!(
        substituted
            .iter()
            .all(|glyph| glyph.source.start() == ByteOffset::ZERO),
        "every substituted glyph addresses the marker's start, never its interior",
    );

    let mut editor = Editor::new("- alpha\n", Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::ZERO));
    assert!(editor.press_right(false));
    assert_eq!(
        editor.document().selection().head().get(),
        3,
        "past the whole marker and the `a`, as it was before a bullet was drawn there",
    );
}

/// A click on the bullet is a click on syntax that is not there. It resolves to the line's start
/// rather than between the `-` and the space after it.
#[test]
fn clicking_the_bullet_does_not_land_inside_the_marker() {
    let mut editor = Editor::new("- alpha\n", Grid::new());
    editor.build_frame(600.0, 400.0, 1.0);
    editor.click_at(
        MARGIN + ADVANCE / 2.0,
        MARGIN + mdedit_app::toolbar::HEIGHT + LINE_HEIGHT / 2.0,
    );
    assert_eq!(editor.document().selection().head().get(), 0);
}
