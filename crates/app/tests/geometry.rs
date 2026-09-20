//! The built frame, through the real font stack (MDEDIT-133).
//!
//! Every other test of a frame in this crate draws through `stub::Grid`, whose glyphs are all eight
//! pixels wide, whose rows are all twenty tall, and which answers every question about a font by
//! construction. Three of the five defects the toolbar wave shipped were cases where **the stub
//! itself was the lie**: a bar that overflowed a 900 px window, a blank page that drew no caret
//! because the real backend returns no rows for empty input, and buttons that did not fit.
//!
//! **The rule that keeps this from becoming a forty-seventh copy of the stub tests: only assertions
//! whose answer depends on real font metrics live here.** A test that would pass identically on the
//! stub belongs with the stub, which is faster, deterministic and correct for everything else.
//!
//! **Relations, never magnitudes.** "Inside the window", "left of", "on the same row" — never "is
//! 844.0 px", because a font update moves every advance in the file. `icons.rs` set that pattern and
//! this follows it.
//!
//! Windows-only, like `icons.rs`: `mdedit-text-dwrite` compiles to nothing elsewhere, and there is
//! no DirectWrite to ask.

#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::status;
use mdedit_app::toolbar;
use mdedit_app::view::Theme;
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_text_dwrite::DirectWriteText;

/// The default window, from `Window::default_attributes` in `editor.rs`.
const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 600.0;

/// A solid quad has the atlas's single opaque texel; a glyph has a real region.
const SOLID: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn editor(source: &str) -> Editor<DirectWriteText> {
    let engine = DirectWriteText::new().expect("DirectWrite is available on any supported Windows");
    Editor::new(source, engine)
}

fn glyphs(quads: &[QuadInstance]) -> Vec<[f32; 4]> {
    quads
        .iter()
        .filter(|quad| quad.uv != SOLID)
        .map(|quad| quad.rect)
        .collect()
}

/// The quads drawn in `colour`, whatever they are.
fn in_colour(quads: &[QuadInstance], colour: mdedit_app::view::Rgba) -> Vec<[f32; 4]> {
    let colour = [colour.r, colour.g, colour.b, colour.a];
    quads
        .iter()
        .filter(|quad| quad.colour == colour)
        .map(|quad| quad.rect)
        .collect()
}

/// **A blank page has a caret** (MDEDIT-81), and only the real backend can say so.
///
/// `DirectWriteText::layout` returns `Layout::default()` — no rows at all — for input with nothing
/// in it, and everything downstream is keyed off rows. The stub fabricated a row for the same input,
/// so every test of a new document passed while File → New shipped a page with nowhere to type. The
/// repair is in `lay_out_chunk`; this is the test that would have caught its absence.
#[test]
fn a_blank_page_has_a_caret() {
    let quads = editor("").build_frame(WIDTH, HEIGHT, 1.0);
    let carets = in_colour(&quads, Theme::default().caret);

    assert_eq!(carets.len(), 1, "a blank page drew {carets:?}");
    assert!(carets[0][3] > 0.0, "the caret has no height: {carets:?}");
}

/// **Every button the bar drew is inside the window**, measured on the frame rather than on
/// `toolbar::overflowing`'s arithmetic — which is what `icons.rs` already checks, and which a stub
/// eight pixels to a glyph cannot falsify. A button that fell off the right-hand end is the defect
/// MDEDIT-116 shipped, at a window width nobody had measured with the real font.
///
/// **The narrow window is the one that can fail.** At the default 900 px the bar has room to spare,
/// so it fits however badly the overflow rule is written; at 500 px it fits only if the rule works.
#[test]
fn every_button_falls_inside_the_window() {
    for width in [WIDTH, 500.0] {
        let mut editor = editor("Text\n");
        let quads = editor.build_frame(width, HEIGHT, 1.0);

        let buttons = editor.toolbar_buttons();
        assert!(
            buttons.len() > 3,
            "{width} px drew {} buttons",
            buttons.len()
        );
        for button in buttons {
            assert!(
                button.right() <= width,
                "{width} px: {:?} ends at {}, past the window",
                button.action,
                button.right()
            );
        }

        // And the faces inside them: a glyph wider than its button is what the rects cannot say.
        let last = buttons.last().unwrap().right();
        let in_the_bar = glyphs(&quads)
            .into_iter()
            .filter(|rect| rect[1] < toolbar::HEIGHT);
        for rect in in_the_bar {
            assert!(
                rect[0] + rect[2] <= width,
                "{width} px: a button's glyph reaches {}, past the window",
                rect[0] + rect[2]
            );
            assert!(
                rect[0] < last,
                "{width} px: a glyph is drawn past the last button"
            );
        }
    }
}

/// **Wrapped text stays inside the content rectangle**, which is the general form of the defect
/// MDEDIT-92 found one instance of. Where a line breaks is the text engine's answer, and the stub
/// breaks at a character boundary it can predict; this asks the engine that ships.
#[test]
fn nothing_is_drawn_outside_the_content_rect() {
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(20);
    let mut editor = editor(&format!("{long}\n\n{long}\n"));
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let content = editor.content_rect();

    let document_rows = glyphs(&quads)
        .into_iter()
        .filter(|rect| rect[1] >= toolbar::HEIGHT && rect[1] < HEIGHT - status::BAR_HEIGHT);
    let mut seen = 0;
    for rect in document_rows {
        assert!(
            rect[0] >= content.left_edge(),
            "a glyph at {} is left of the content rect at {}",
            rect[0],
            content.left_edge()
        );
        assert!(
            rect[0] + rect[2] <= content.left_edge() + content.width(),
            "a glyph reaches {}, past the content rect's right edge at {}",
            rect[0] + rect[2],
            content.left_edge() + content.width()
        );
        seen += 1;
    }
    assert!(seen > 100, "only {seen} glyphs were drawn to check");
}

/// **A click lands on the row it looks like**, with real metrics under it: where a row sits and how
/// tall it is are the font's answer, and the stub's twenty-pixel cell makes it arithmetic the test
/// could do itself.
///
/// **The caret is the ruler.** A glyph's box is its ink, and with a real font every letter's ink
/// starts at a different height — grouping glyphs by their tops finds letterforms, not rows. The
/// caret is drawn a full row tall at the row's own position, so putting it on a line says exactly
/// where that line was drawn. Then clicking the middle of that box has to come back to the same
/// line, which is the drawing and the hit test agreeing about one number neither of them invented.
#[test]
fn a_click_lands_on_the_row_it_looks_like() {
    let source = "First line.\nSecond, a longer one.\nThird.\n";
    let mut editor = editor(source);

    for line in 0..3 {
        let start = source
            .split_inclusive('\n')
            .take(line)
            .map(str::len)
            .sum::<usize>();
        editor
            .document_mut()
            .set_selection(Selection::caret(ByteOffset::new(start)));
        let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
        let carets = in_colour(&quads, Theme::default().caret);
        assert_eq!(carets.len(), 1, "line {line} drew {carets:?}");
        let [x, top, _, height] = carets[0];

        assert!(editor.click_at(x + 1.0, top + height / 2.0));
        let (landed, column) = editor
            .document()
            .line_and_column(editor.document().selection().head());
        assert_eq!(landed.get(), line, "a click on line {line}'s own row");
        assert_eq!(column.get(), 0, "at the column the caret was drawn at");
    }
}
