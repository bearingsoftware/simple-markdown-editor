//! A block quote shows how deeply it is quoted (MDEDIT-145).
//!
//! Before this, `>` and `>>` drew identically — muted italic text, no bar — so a reply inside a
//! reply read as one quote. Now each level draws a bar at the left, and the text hangs clear of them
//! on every row, wrapped or not.
//!
//! **Bars are told apart from the chrome by difference.** The chrome draws `theme.line` too, so each
//! test compares a quoted document with the same window showing unquoted text, and the bars are
//! what the quote added.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::quote::{BAR, STEP};
use mdedit_app::view::Theme;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{ADVANCE, ASCENT, Grid, LINE_HEIGHT};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 600.0;

/// A solid quad has the atlas's single opaque texel; a glyph has a real region.
const SOLID: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn frame(source: &str) -> Vec<QuadInstance> {
    Editor::new(source, Grid::new()).build_frame(WIDTH, HEIGHT, 1.0)
}

/// Narrow, upright solid quads in the hairline colour: what a quote bar looks like.
fn uprights(quads: &[QuadInstance]) -> Vec<[f32; 4]> {
    let line = Theme::default().line;
    let line = [line.r, line.g, line.b, line.a];
    let mut found: Vec<[f32; 4]> = quads
        .iter()
        .filter(|quad| quad.uv == SOLID && quad.colour == line)
        .filter(|quad| quad.rect[2] <= BAR + 0.5 && quad.rect[3] >= LINE_HEIGHT / 2.0)
        .map(|quad| quad.rect)
        .collect();
    found.sort_by(|a, b| a.partial_cmp(b).unwrap());
    found
}

/// The bars `source` draws, less anything the chrome draws in the same window.
///
/// Merged by column: a level's bar can be drawn as several quads where the depth beside it changes,
/// and what a reader sees — and what these tests mean by "a bar" — is one continuous stroke.
fn bars(source: &str) -> Vec<[f32; 4]> {
    let mut extra = uprights(&frame(source));
    for rect in uprights(&frame("unquoted\n")) {
        if let Some(at) = extra.iter().position(|r| *r == rect) {
            extra.remove(at);
        }
    }
    extra.sort_by(|a, b| (a[0], a[1]).partial_cmp(&(b[0], b[1])).unwrap());
    let mut merged: Vec<[f32; 4]> = Vec::new();
    for rect in extra {
        match merged.last_mut() {
            Some(last) if last[0] == rect[0] && (last[1] + last[3] - rect[1]).abs() < 0.5 => {
                last[3] += rect[3];
            }
            _ => merged.push(rect),
        }
    }
    merged
}

/// The leftmost glyph of every row a frame drew glyphs on, as `(x, y)`, top to bottom — chrome
/// included; [`document_row_starts`] keeps only the document's rows.
fn glyph_rows(source: &str) -> Vec<(f32, f32)> {
    let mut rows: Vec<(f32, f32)> = frame(source)
        .iter()
        .filter(|quad| quad.uv != SOLID)
        .map(|quad| (quad.rect[1], quad.rect[0]))
        .collect();
    rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // First glyph of each row: the leftmost x at each distinct y.
    let mut firsts: Vec<(f32, f32)> = Vec::new();
    for (y, x) in rows {
        match firsts.last_mut() {
            Some((last_y, last_x)) if *last_y == y => *last_x = last_x.min(x),
            _ => firsts.push((y, x)),
        }
    }
    firsts.into_iter().map(|(y, x)| (x, y)).collect()
}

/// Where the text starts on a row with no quote in it — the left margin text normally sits at.
///
/// From the document's rows only. The first version took the leftmost glyph in the whole frame,
/// which is a toolbar label, and made every indent in this file look twelve pixels too deep.
fn margin() -> f32 {
    document_row_starts("unquoted\n")[0]
}

/// Where each of the document's rows starts, top to bottom: the chrome draws glyphs too, above the
/// document in the toolbar and below it in the status bar, and those rows are left out.
fn document_row_starts(source: &str) -> Vec<f32> {
    let top = mdedit_app::toolbar::HEIGHT + stub::MARGIN;
    glyph_rows(source)
        .into_iter()
        .filter(|(_, y)| *y >= top && *y < HEIGHT - mdedit_app::toolbar::HEIGHT)
        .map(|(x, _)| x)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Depth is visible.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_quote_draws_one_bar() {
    let drawn = bars("> quoted\n");

    assert_eq!(drawn.len(), 1, "{drawn:?}");
}

#[test]
fn a_nested_quote_draws_two_bars_in_either_spelling() {
    // The defect: `>` and `>>` drew identically. Both spellings of a nested quote are one coalesced
    // marker in the parse, which is why depth comes from the block tree.
    for source in [">> deep\n", "> > deep\n"] {
        let drawn = bars(source);
        assert_eq!(drawn.len(), 2, "{source:?} drew {drawn:?}");
        assert_eq!(
            drawn[1][0] - drawn[0][0],
            STEP,
            "{source:?}: the bars are not a step apart"
        );
    }
}

#[test]
fn three_levels_draw_three_bars() {
    assert_eq!(bars("> > > deepest\n").len(), 3);
}

#[test]
fn quoted_text_is_indented_by_its_depth() {
    let margin = margin();

    assert_eq!(document_row_starts("> quoted\n"), vec![margin + STEP]);
    assert_eq!(document_row_starts(">> deep\n"), vec![margin + 2.0 * STEP]);
}

#[test]
fn every_bar_is_left_of_the_text_it_marks() {
    let source = "> > deep\n";
    let text = document_row_starts(source)[0];

    for bar in bars(source) {
        assert!(
            bar[0] + bar[2] < text,
            "a bar ends at {} and the text starts at {text}",
            bar[0] + bar[2]
        );
    }
}

#[test]
fn text_after_a_quote_is_not_indented() {
    let starts = document_row_starts("> quoted\n\nafter\n");

    assert_eq!(starts.len(), 2, "{starts:?}");
    assert_eq!(
        starts[1],
        margin(),
        "the paragraph after the quote was indented"
    );
}

// ---------------------------------------------------------------------------------------------
// The shapes a quote comes in.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_wrapped_quoted_line_keeps_its_indent_on_every_row() {
    // The reason this places rows rather than substituting the marker: a substituted `> ` would
    // indent the first row and let every wrapped row run back under the bar.
    let long = "word ".repeat(60);
    let starts = document_row_starts(&format!("> {long}\n"));

    assert!(starts.len() >= 3, "did not wrap: {starts:?}");
    for (row, x) in starts.iter().enumerate() {
        assert_eq!(*x, margin() + STEP, "row {row} is not indented");
    }
}

#[test]
fn a_wrapped_quoted_line_reaches_no_further_right_than_unquoted_text() {
    // Laid out at the width the indent leaves, so its rows end where any other text's do. Laid out
    // at the full width and *then* moved right, they would end one step past it.
    //
    // Compared with unquoted text rather than with the window, because the first version of this
    // asserted "fits in the window" — and the stub's glyphs are points and the window has a right
    // margin, so text a step too wide still fitted. That test could not fail.
    let long = "word ".repeat(60);
    let rightmost = |source: &str| {
        let top = mdedit_app::toolbar::HEIGHT + stub::MARGIN;
        frame(source)
            .iter()
            .filter(|quad| quad.uv != SOLID)
            .filter(|quad| {
                quad.rect[1] >= top && quad.rect[1] < HEIGHT - mdedit_app::toolbar::HEIGHT
            })
            .map(|quad| quad.rect[0])
            .fold(0.0_f32, f32::max)
    };

    let quoted = rightmost(&format!("> {long}\n"));
    let unquoted = rightmost(&format!("{long}\n"));

    assert!(
        quoted <= unquoted,
        "quoted text reached {quoted}, past unquoted text's {unquoted}"
    );
}

#[test]
fn a_lazy_continuation_line_is_at_its_quotes_depth() {
    // `continued` has no `>`, and is part of the quoted paragraph all the same.
    let starts = document_row_starts("> lazy\ncontinued\n");

    assert_eq!(starts, vec![margin() + STEP, margin() + STEP]);
    let drawn = bars("> lazy\ncontinued\n");
    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert!(
        drawn[0][3] >= 2.0 * LINE_HEIGHT - 0.5,
        "the bar does not reach the lazy line: {drawn:?}"
    );
}

#[test]
fn a_blank_line_inside_a_quote_keeps_the_bar_whole() {
    let drawn = bars("> first\n>\n> third\n");

    assert_eq!(drawn.len(), 1, "the bar broke at the blank line: {drawn:?}");
    assert!(drawn[0][3] >= 3.0 * LINE_HEIGHT - 0.5, "{drawn:?}");
}

#[test]
fn two_quotes_separated_by_a_blank_line_draw_two_bars() {
    let drawn = bars("> one\n\n> two\n");

    assert_eq!(drawn.len(), 2, "{drawn:?}");
    assert_eq!(drawn[0][0], drawn[1][0], "same depth, same column");
    assert!(
        drawn[0][1] + drawn[0][3] < drawn[1][1],
        "no gap between two separate quotes: {drawn:?}"
    );
}

#[test]
fn depth_can_change_inside_one_quote() {
    let source = "> outer\n>\n> > inner\n>\n> outer again\n";
    let starts = document_row_starts(source);
    let margin = margin();

    assert_eq!(
        starts,
        vec![margin + STEP, margin + 2.0 * STEP, margin + STEP],
        "outer, inner, outer"
    );

    let drawn = bars(source);
    assert_eq!(drawn.len(), 2, "one outer bar and one inner: {drawn:?}");
    let (outer, inner) = (drawn[0], drawn[1]);
    assert!(outer[3] > inner[3], "the outer bar is the whole quote");
    assert!(
        inner[1] > outer[1] && inner[1] + inner[3] < outer[1] + outer[3],
        "the inner bar is inside the outer one: {drawn:?}"
    );
}

#[test]
fn a_blank_quote_line_at_the_end_of_the_file_still_has_a_row() {
    // The last unit of a quote can be `>` with no newline: once the marker is concealed there is
    // nothing to shape, and a text engine answers with no rows. The bar would stop short of a line
    // the caret can still be put on.
    let drawn = bars("> text\n>");

    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert!(
        drawn[0][3] >= 2.0 * LINE_HEIGHT - 0.5,
        "the last line lost its row: {drawn:?}"
    );
}

#[test]
fn the_caret_on_a_blank_quoted_line_is_at_its_indent_not_on_the_bar() {
    // Found in review of #166. A blank `>` line has no glyphs, and a row with no glyphs put the caret
    // at x 0 — on the first bar, whatever the depth — and the first keystroke then jumped it right
    // to where the text goes. It belongs where that text will appear.
    let caret = Theme::default().caret;
    let caret = [caret.r, caret.g, caret.b, caret.a];
    let caret_x = |source: &str, at: usize| {
        let mut editor = Editor::new(source, Grid::new());
        editor
            .document_mut()
            .set_selection(Selection::caret(ByteOffset::new(at)));
        let carets: Vec<f32> = editor
            .build_frame(WIDTH, HEIGHT, 1.0)
            .iter()
            .filter(|quad| quad.colour == caret)
            .map(|quad| quad.rect[0])
            .collect();
        assert_eq!(carets.len(), 1, "{source:?}: {carets:?}");
        carets[0]
    };
    let margin = margin();

    assert_eq!(caret_x("> a\n>\n> b\n", 5), margin + STEP, "one level");
    assert_eq!(
        caret_x("> > a\n> >\n> > b\n", 9),
        margin + 2.0 * STEP,
        "two levels"
    );
    // And a blank line that is not quoted keeps the margin — the fix must not indent everything.
    assert_eq!(caret_x("> a\n\nafter\n", 4), margin, "outside the quote");
}

// ---------------------------------------------------------------------------------------------
// What does not change.
// ---------------------------------------------------------------------------------------------

#[test]
fn plain_mode_draws_the_markers_and_no_bars() {
    let mut plain = Editor::new("> > deep\n", Grid::new());
    plain.toggle_mode();
    let quads = plain.build_frame(WIDTH, HEIGHT, 1.0);

    let mut baseline = Editor::new("unquoted\n", Grid::new());
    baseline.toggle_mode();

    assert_eq!(
        uprights(&quads),
        uprights(&baseline.build_frame(WIDTH, HEIGHT, 1.0)),
        "plain mode drew a bar"
    );
}

#[test]
fn a_click_on_quoted_text_lands_on_the_letter_under_it() {
    // Every glyph moved right by the indent; a click has to follow it there. The stub gives every
    // glyph the same advance, so a quarter of the way into the third letter is nearest the boundary
    // before it — between `qu` and `oted` — and not a coin toss the way its exact middle would be.
    let source = "> > quoted\n";
    let mut editor = Editor::new(source, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let start = document_row_starts(source)[0];
    let (_, y) = glyph_rows(source)
        .into_iter()
        .find(|(x, _)| *x == start)
        .unwrap();

    // The stub draws a glyph as a point on its baseline, so the row is *above* `y`: aiming half a
    // line below it, as this first did, lands in the blank line after the quote.
    editor.click(start + 2.25 * ADVANCE, y - ASCENT / 2.0);

    assert_eq!(
        editor.document().selection().head().get(),
        source.find("quoted").unwrap() + 2,
        "the click did not land between `qu` and `oted`"
    );
}

// ---------------------------------------------------------------------------------------------
// Against the real text engine.
// ---------------------------------------------------------------------------------------------

/// The stub answers every layout question by construction; this is the one through DirectWrite.
#[cfg(windows)]
#[test]
fn a_nested_quote_is_indented_and_barred_through_the_real_text_engine_too() {
    use mdedit_text_dwrite::DirectWriteText;

    let real = |source: &str| {
        let engine =
            DirectWriteText::new().expect("DirectWrite is available on any supported Windows");
        Editor::new(source, engine).build_frame(WIDTH, HEIGHT, 1.0)
    };
    let bars_of = |quads: &[QuadInstance]| uprights(quads).len();

    let quoted = real("> > deep\n");
    let plain = real("unquoted\n");

    assert_eq!(bars_of(&quoted) - bars_of(&plain), 2);
    let text_left = |quads: &[QuadInstance]| {
        quads
            .iter()
            .filter(|quad| quad.uv != SOLID && quad.rect[1] > mdedit_app::toolbar::HEIGHT)
            .map(|quad| quad.rect[0])
            .fold(f32::MAX, f32::min)
    };
    assert_eq!(text_left(&quoted) - text_left(&plain), 2.0 * STEP);
}
