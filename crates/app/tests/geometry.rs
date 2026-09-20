//! The built frame, through the real font stack (MDEDIT-133).
//!
//! Nearly every frame test in this crate draws through `stub::Grid`, whose glyphs are all eight
//! pixels wide and whose rows are all twenty tall — it answers every question about a font by
//! construction. Three of the five defects the toolbar wave shipped were cases where **the stub
//! itself was the lie**: a bar that overflowed a 900 px window, buttons that did not fit, and a
//! blank page that drew no caret because the real backend returns no rows for empty input.
//!
//! A handful of other files already ask the real engine for one question each — `icons.rs` for
//! whether Windows has a glyph at a code point, `table_metrics.rs` for column widths, and a
//! Windows-only test apiece in `quotes.rs`, `images.rs` and `thematic_break.rs`. This is the file
//! for the **frame as a whole**.
//!
//! **The rule that keeps it from becoming another copy of the stub tests: only assertions whose
//! answer depends on real font metrics live here.** A test that would pass identically on the stub
//! belongs with the stub, which is faster, deterministic and correct for everything else.
//!
//! **Relations, never magnitudes.** "Inside the window", "past the last button", "on the same row" —
//! never "is 844.0 px", because a font update moves every advance in the file. `icons.rs` set that
//! pattern and this follows it.
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

use mdedit_app::content::ContentRect;
use mdedit_app::editor::Editor;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::status;
use mdedit_app::toolbar;
use mdedit_app::view::{Rgba, Theme};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_text_dwrite::DirectWriteText;

/// The window `Window::default_attributes` asks for — `editor.rs`'s `LogicalSize::new(900.0, 700.0)`
/// — so what these tests measure is the window a fresh install opens.
const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 700.0;

/// A solid quad has the atlas's single opaque texel; a glyph has a real region.
const SOLID: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn editor(source: &str) -> Editor<DirectWriteText> {
    let engine = DirectWriteText::new().expect("DirectWrite is available on any supported Windows");
    Editor::new(source, engine)
}

/// Every glyph drawn, as `[x, y, width, height]`.
fn glyphs(quads: &[QuadInstance]) -> Vec<[f32; 4]> {
    quads
        .iter()
        .filter(|quad| quad.uv != SOLID)
        .map(|quad| quad.rect)
        .collect()
}

/// The carets drawn inside the document's own rectangle.
///
/// **Bounded by the content rectangle, not by colour alone.** `Theme::dark` gives `caret` and
/// `tab_modified` the same value, so a colour match would count the dot on a modified tab as a
/// caret — passing here only because these fixtures open no files, which is a condition no reader
/// of the assertion would know about.
fn carets(quads: &[QuadInstance], content: ContentRect) -> Vec<[f32; 4]> {
    let caret = Theme::default().caret;
    let caret = [caret.r, caret.g, caret.b, caret.a];
    quads
        .iter()
        .filter(|quad| quad.colour == caret && content.contains(quad.rect[0], quad.rect[1]))
        .map(|quad| quad.rect)
        .collect()
}

/// **A blank page has a caret, asked of the backend that had the defect** (MDEDIT-81).
///
/// `DirectWriteText::layout` returns `Layout::default()` for empty input — no rows at all — and
/// everything the renderer draws is keyed off rows, so File → New shipped a page with nowhere to
/// type. `blank_page.rs` covers the repair, and covers it **on the stub**, whose empty-input
/// behaviour is a deliberate copy of this backend's. This is the copy checked against the original:
/// if DirectWrite's answer for empty input ever changes, or the stub's imitation of it drifts, every
/// headless test in that file goes on passing and this one does not.
#[test]
fn a_blank_page_has_a_caret() {
    let mut editor = editor("");
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let drawn = carets(&quads, editor.content_rect());

    assert_eq!(drawn.len(), 1, "a blank page drew {drawn:?}");
    assert!(drawn[0][3] > 0.0, "the caret has no height: {drawn:?}");
}

/// **Every button's glyph is inside the window**, which is what the rects cannot say.
///
/// `toolbar::overflowing` reserves room for `More` and stops at the edge, so `ButtonRect::right()`
/// is inside the window by construction whatever the font does — asserting that is asserting
/// arithmetic. What the arithmetic does not know is how wide the **glyph** inside a button is, and
/// that is the number the real font supplies: MDEDIT-116's defect was three buttons off the end of a
/// 900 px window because a stub eight pixels to a glyph said they fitted.
///
/// **The narrow window is the one that can fail.** At the default width the bar has room to spare,
/// so it fits however badly the rule is written; at 500 px it fits only if the rule works.
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
        let last = buttons.last().unwrap().right();

        let in_the_bar: Vec<[f32; 4]> = glyphs(&quads)
            .into_iter()
            .filter(|rect| rect[1] < toolbar::HEIGHT)
            .collect();
        assert!(
            in_the_bar.len() >= buttons.len(),
            "{width} px: {} buttons drew {} glyphs",
            buttons.len(),
            in_the_bar.len()
        );
        for rect in in_the_bar {
            let right = rect[0] + rect[2];
            assert!(
                right <= width,
                "{width} px: a button's glyph reaches {right}, past the window"
            );
            assert!(
                right <= last,
                "{width} px: a glyph reaches {right}, past the last button's {last}"
            );
        }
    }
}

/// **Text stays inside the content rectangle's column.**
///
/// Where a line breaks is the text engine's answer — the stub breaks at a character boundary the
/// test could predict — so this is the general form of MDEDIT-92's defect: ink outside the column it
/// belongs to. The document is three hundred paragraphs so that every row on screen is a wrapped
/// one, which is where a width mistake shows.
///
/// **The body's own colour, below the bar, is what tells its glyphs from the chrome's.** The status
/// bar draws in `marker` and `status_error`, so nothing it draws is confused for the document; the
/// toolbar *does* draw a label in `theme.text`, which is why the filter takes the bar's height into
/// account as well. Counting glyphs by position alone was the first version, and it could not work:
/// the status bar says more about a long document than about an empty one, so the count differs for
/// a reason that is not a defect.
///
/// **Downwards there is nothing to assert, and finding that out is worth writing down.** The last
/// row on screen is drawn in full and the chrome paints over what hangs below it — painter's order
/// is the whole of the answer (`docs/decisions/chrome.md`) — so a hard bottom edge is not true of
/// the product. A version of this bounded the overhang at one row instead, and three separate
/// mutations could not make it fail: off-screen rows are not drawn because off-screen chunks are
/// never laid out, which `blocks.rs` and `scroll.rs` already assert on the stub. An assertion that
/// cannot fail is worse than no assertion, so the vertical claim here is only the top edge.
#[test]
fn nothing_is_drawn_outside_the_content_rect() {
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(20);
    let paragraphs = format!("{long}\n\n").repeat(150);

    let mut editor = editor(&paragraphs);
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let content = editor.content_rect();
    let right = content.left_edge() + content.width();
    let bottom = content.top_edge() + content.height();
    assert!(
        bottom <= HEIGHT - status::BAR_HEIGHT,
        "the content rectangle overlaps the status bar"
    );

    let body = Theme::default().text;
    let body = [body.r, body.g, body.b, body.a];
    let mut seen = 0;
    let below_the_bar = quads
        .iter()
        .filter(|quad| quad.colour == body && quad.rect[1] >= toolbar::HEIGHT);
    for quad in below_the_bar {
        let [x, y, width, _] = quad.rect;
        // **The ink's right edge**, against the column's. Its left edge is not asserted against the
        // margin: a glyph with a negative left side bearing legitimately puts ink left of the pen,
        // so the honest question is whether any of the glyph is inside the column at all.
        assert!(
            x + width <= right,
            "a glyph reaches {}, past the content rect's right edge at {right}",
            x + width
        );
        assert!(
            x + width > content.left_edge(),
            "a glyph at {x} is entirely left of the content rect at {}",
            content.left_edge()
        );
        assert!(
            y >= content.top_edge(),
            "a glyph at {y} is above the column's top edge at {}",
            content.top_edge()
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
        let drawn = carets(&quads, editor.content_rect());
        assert_eq!(drawn.len(), 1, "line {line} drew {drawn:?}");
        let [x, top, _, height] = drawn[0];

        assert!(editor.click_at(x + 1.0, top + height / 2.0));
        let (landed, column) = editor
            .document()
            .line_and_column(editor.document().selection().head());
        assert_eq!(landed.get(), line, "a click on line {line}'s own row");
        assert_eq!(column.get(), 0, "at the column the caret was drawn at");
    }
}

/// Keeps the `Rgba` import honest: the theme collision the caret filter exists for.
#[test]
fn the_dark_theme_draws_a_caret_and_a_modified_tab_in_one_colour() {
    let dark = Theme::dark();
    let same: fn(Rgba, Rgba) -> bool = |a, b| a.r == b.r && a.g == b.g && a.b == b.b;
    assert!(
        same(dark.caret, dark.tab_modified),
        "if these ever differ, `carets` can go back to matching on colour alone"
    );
}
