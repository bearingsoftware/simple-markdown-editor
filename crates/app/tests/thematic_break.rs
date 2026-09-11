//! A thematic break draws a rule instead of vanishing (MDEDIT-143).
//!
//! `---` is only its marker, so concealing the marker in preview removed the whole construct: the
//! author saw a divider in plain markup and blank nothing in preview.
//!
//! **Every assertion here is a difference between two frames.** The chrome draws hairlines in the
//! same `theme.line` the rule uses — the edge under the toolbar, a table's rules — so counting them
//! in one frame would be counting the chrome. A document with a break is compared with the same
//! document without one, and exactly one rule has to be the difference.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::view::Theme;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 600.0;

/// A solid quad has the atlas's single opaque texel; a glyph has a real region.
const SOLID: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn frame(source: &str) -> Vec<QuadInstance> {
    Editor::new(source, Grid::new()).build_frame(WIDTH, HEIGHT, 1.0)
}

/// Thin, wide solid quads in the hairline colour: what a rule looks like, and what chrome edges look
/// like too, which is why nothing below reads this on its own.
fn hairlines(quads: &[QuadInstance]) -> Vec<[f32; 4]> {
    let line = Theme::default().line;
    let line = [line.r, line.g, line.b, line.a];
    let mut found: Vec<[f32; 4]> = quads
        .iter()
        .filter(|quad| quad.uv == SOLID && quad.colour == line)
        .filter(|quad| quad.rect[3] <= 2.0 && quad.rect[2] >= WIDTH / 2.0)
        .map(|quad| quad.rect)
        .collect();
    found.sort_by(|a, b| a.partial_cmp(b).unwrap());
    found
}

/// The rules `with` draws that `without` does not.
fn added(with: &str, without: &str) -> Vec<[f32; 4]> {
    difference(hairlines(&frame(with)), &hairlines(&frame(without)))
}

/// `with`, less each of `without` once.
fn difference(mut with: Vec<[f32; 4]>, without: &[[f32; 4]]) -> Vec<[f32; 4]> {
    for rect in without {
        if let Some(at) = with.iter().position(|r| r == rect) {
            with.remove(at);
        }
    }
    with
}

/// The top of each row glyphs were drawn on in the document area, in order.
fn glyph_rows(quads: &[QuadInstance]) -> Vec<f32> {
    let mut rows: Vec<f32> = quads
        .iter()
        .filter(|quad| quad.uv != SOLID && quad.rect[0] >= MARGIN)
        .filter(|quad| quad.rect[1] >= MARGIN + mdedit_app::toolbar::HEIGHT)
        .filter(|quad| quad.rect[1] < HEIGHT - mdedit_app::toolbar::HEIGHT)
        .map(|quad| quad.rect[1])
        .collect();
    rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
    rows.dedup();
    rows
}

// ---------------------------------------------------------------------------------------------
// It draws.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_thematic_break_draws_one_rule() {
    // The defect, as the difference it makes: without the break there is no rule, with it there is
    // exactly one. Before MDEDIT-143 this was an empty vector.
    let rules = added("Above\n\n---\n\nBelow\n", "Above\n\nBelow\n");

    assert_eq!(rules.len(), 1, "{rules:?}");
}

#[test]
fn the_rule_sits_between_the_text_either_side_of_it() {
    let source = "Above\n\n---\n\nBelow\n";
    let rule = added(source, "Above\n\nBelow\n")[0];
    let rows = glyph_rows(&frame(source));

    assert_eq!(
        rows.len(),
        2,
        "one row of text above and one below: {rows:?}"
    );
    assert!(
        rows[0] < rule[1] && rule[1] < rows[1],
        "rule at {} is not between rows {rows:?}",
        rule[1]
    );
}

#[test]
fn the_rule_is_centred_on_the_breaks_own_row() {
    // "Between the text" is not enough: a rule at the top edge of its row, or in the blank line
    // after it, is still between the paragraphs. Hand-mutating the fix found both survived every
    // other test. The caret is drawn a full row tall, so a caret on the break's line says exactly
    // where that row is — and the rule belongs across the middle of it.
    let source = "Above\n\n---\n\nBelow\n";
    let rule = added(source, "Above\n\nBelow\n")[0];

    let mut editor = Editor::new(source, Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(
            source.find("---").unwrap(),
        )));
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let caret = Theme::default().caret;
    let caret = [caret.r, caret.g, caret.b, caret.a];
    let carets: Vec<[f32; 4]> = quads
        .iter()
        .filter(|quad| quad.colour == caret)
        .map(|quad| quad.rect)
        .collect();
    assert_eq!(carets.len(), 1, "{carets:?}");

    let row_middle = carets[0][1] + carets[0][3] / 2.0;
    let rule_middle = rule[1] + rule[3] / 2.0;
    assert!(
        (row_middle - rule_middle).abs() < 0.5,
        "rule centred at {rule_middle}, its row at {row_middle}"
    );
}

#[test]
fn the_rule_starts_where_the_text_does_and_crosses_the_column() {
    let source = "Above\n\n---\n\nBelow\n";
    let rule = added(source, "Above\n\nBelow\n")[0];
    let text_left = frame(source)
        .iter()
        .filter(|quad| quad.uv != SOLID && quad.rect[1] > rule[1])
        .map(|quad| quad.rect[0])
        .fold(f32::MAX, f32::min);

    assert_eq!(
        rule[0], text_left,
        "the rule and the text disagree on the margin"
    );
    assert!(
        rule[0] + rule[2] > WIDTH * 0.8,
        "a rule {} wide in a {WIDTH} window is not across the column",
        rule[2]
    );
}

#[test]
fn every_spelling_of_a_break_draws_a_rule() {
    for spelling in ["---", "***", "___", "- - -", "* * *", "_____", " ---"] {
        let rules = added(
            &format!("Above\n\n{spelling}\n\nBelow\n"),
            "Above\n\nBelow\n",
        );
        assert_eq!(rules.len(), 1, "{spelling:?} drew {rules:?}");
    }
}

#[test]
fn a_break_on_the_last_line_with_no_newline_still_draws() {
    // The shape where a text engine may be handed nothing visible at all and answer with no rows —
    // `DirectWriteText` returns `Layout::default()` for empty input (MDEDIT-81). A rule with no row
    // to sit on would silently not be drawn.
    let rules = added("Above\n\n---", "Above\n\n");

    assert_eq!(rules.len(), 1, "{rules:?}");
}

#[test]
fn a_document_that_is_only_a_break_draws_a_rule() {
    let rules = added("---\n", "\n");

    assert_eq!(rules.len(), 1, "{rules:?}");
}

// ---------------------------------------------------------------------------------------------
// It does not draw.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_setext_underline_is_a_heading_not_a_rule() {
    // `Title` then `---` is an H2 in CommonMark, and the dashes are the heading's marker. A rule
    // here would draw a divider under every setext heading in the document.
    let rules = added("Title\n---\n\nBelow\n", "Title\n\nBelow\n");

    assert!(rules.is_empty(), "a setext heading drew {rules:?}");
}

#[test]
fn plain_mode_draws_the_dashes_and_no_rule() {
    // `Ctrl+/` exists to show the bytes, and the bytes of a break are its dashes.
    let source = "Above\n\n---\n\nBelow\n";
    let mut plain = Editor::new(source, Grid::new());
    plain.toggle_mode();
    let quads = plain.build_frame(WIDTH, HEIGHT, 1.0);

    let mut baseline = Editor::new("Above\n\nBelow\n", Grid::new());
    baseline.toggle_mode();
    let without = hairlines(&baseline.build_frame(WIDTH, HEIGHT, 1.0));

    assert_eq!(hairlines(&quads), without, "plain mode drew a rule");
    assert_eq!(
        glyph_rows(&quads).len(),
        3,
        "the dashes are not drawn as text"
    );
}

#[test]
fn a_dash_in_a_sentence_is_not_a_rule() {
    let rules = added("Above\n\nSome --- text\n\nBelow\n", "Above\n\nBelow\n");

    assert!(rules.is_empty(), "{rules:?}");
}

#[test]
fn the_rule_scales_with_the_display() {
    // **Every other test here runs at 100%, where multiplying by the scale and dividing by it give
    // the same answer.** The mutants job on this pull request found exactly that: seven survivors,
    // all in the scale arithmetic, and a laptop at 150% or 200% is the ordinary case rather than the
    // exotic one. At 200% the rule is the same rule in twice as many pixels — everything doubles,
    // including its thickness, which is one logical pixel and so two physical ones.
    let at = |scale: f32| {
        let with = Editor::new("Above\n\n---\n\nBelow\n", Grid::new()).build_frame(
            WIDTH * scale,
            HEIGHT * scale,
            scale,
        );
        let without = Editor::new("Above\n\nBelow\n", Grid::new()).build_frame(
            WIDTH * scale,
            HEIGHT * scale,
            scale,
        );
        difference(hairlines(&with), &hairlines(&without))
    };

    let normal = at(1.0);
    let double = at(2.0);

    assert_eq!(normal.len(), 1, "{normal:?}");
    assert_eq!(double.len(), 1, "{double:?}");
    assert_eq!(
        double[0],
        normal[0].map(|value| value * 2.0),
        "at 200% the rule is not the 100% rule doubled"
    );
}

// ---------------------------------------------------------------------------------------------
// Where it goes when the view moves.
// ---------------------------------------------------------------------------------------------

#[test]
fn the_rule_stays_in_view_when_panned_sideways() {
    // With word wrap off a long line can be panned to, and everything in the document moves left.
    // A divider has no width of its own to scroll with, so it should span the same band of the
    // window either way — a rule drawn from document x 0 would slide out under the gutter.
    let long = "x".repeat(400);
    let with = format!("{long}\n\n---\n\nBelow\n");
    let without = format!("{long}\n\nBelow\n");
    // Every frame from an editor set up the same way, so the only difference is the break and the
    // pan. Measured once before panning, because `pan_by` clamps against the last frame.
    let hairlines_at = |source: &str, pan: f32| {
        let mut editor = Editor::new(source, Grid::new());
        assert!(editor.set_wrap(false));
        editor.build_frame(WIDTH, HEIGHT, 1.0);
        if pan > 0.0 {
            assert!(editor.pan_by(pan), "nothing to pan to");
        }
        hairlines(&editor.build_frame(WIDTH, HEIGHT, 1.0))
    };

    let still = difference(hairlines_at(&with, 0.0), &hairlines_at(&without, 0.0));
    let panned = difference(hairlines_at(&with, 200.0), &hairlines_at(&without, 200.0));

    // Before MDEDIT-143 this compared two empty lists and passed. There has to be a rule to keep.
    assert_eq!(still.len(), 1, "no rule to begin with: {still:?}");
    assert_eq!(still, panned, "the rule moved with the pan");
}

#[test]
fn a_rule_scrolls_with_the_document() {
    // The other axis, where it *should* move: a break is a place in the document.
    // On screen to begin with, with enough below it that the page can scroll.
    let source = format!(
        "{}---\n\n{}",
        "Line.\n\n".repeat(5),
        "Below.\n\n".repeat(40)
    );
    let mut editor = Editor::new(&source, Grid::new());
    let before = hairlines(&editor.build_frame(WIDTH, HEIGHT, 1.0));

    editor.scroll_by(100.0);
    let after = hairlines(&editor.build_frame(WIDTH, HEIGHT, 1.0));

    // The chrome's hairlines are in both frames where they were; the rule is the one that moved.
    let left = difference(before.clone(), &after);
    let arrived = difference(after, &before);
    assert_eq!(left.len(), 1, "no rule moved: {before:?}");
    assert_eq!(arrived.len(), 1);
    assert_eq!(
        left[0][1] - arrived[0][1],
        100.0,
        "the rule moved by something other than the scroll"
    );
}

// ---------------------------------------------------------------------------------------------
// Against the real text engine.
// ---------------------------------------------------------------------------------------------

/// The stub lays out an empty run as a row, and DirectWrite may not.
///
/// `DirectWriteText::layout` returns `Layout::default()` — no rows at all — for input with nothing
/// visible in it, which is what MDEDIT-81's blank page was. A break on the last line is exactly that
/// input once its marker is concealed: no text, no newline. The rule needs a row to sit on, so this
/// is the one case where the stub passing proves nothing about the product.
///
/// Windows-only, like `icons.rs`, because `mdedit-text-dwrite` compiles to nothing elsewhere.
#[cfg(windows)]
#[test]
fn a_rule_is_drawn_through_the_real_text_engine_too() {
    use mdedit_text_dwrite::DirectWriteText;

    let real = |source: &str| {
        let engine =
            DirectWriteText::new().expect("DirectWrite is available on any supported Windows");
        hairlines(&Editor::new(source, engine).build_frame(WIDTH, HEIGHT, 1.0))
    };

    for (with, without) in [
        ("Above\n\n---\n\nBelow\n", "Above\n\nBelow\n"),
        ("Above\n\n---", "Above\n\n"),
        ("---\n", "\n"),
    ] {
        let rules = difference(real(with), &real(without));
        assert_eq!(
            rules.len(),
            1,
            "{with:?} drew {rules:?} through DirectWrite"
        );
    }
}
