//! The icon bar, against the real font stack.
//!
//! **The only test file in this crate that does not use the stub, and it has to be.** Every other
//! integration test here draws through a grid whose glyphs are all eight pixels wide, which is
//! exactly what cannot answer the two questions this file exists for: whether Windows actually has a
//! glyph at `U+E8DD`, and whether twenty-two of them fit across a default window. A stub says yes to
//! both by construction.
//!
//! Windows-only, and skipped elsewhere — `mdedit-text-dwrite` compiles to nothing off Windows, and
//! there is no Segoe anything to ask.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::toolbar::{self, Action};
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{FontFamily, StyledRun, TextLayout, TextStyle};
use mdedit_text_dwrite::DirectWriteText;

/// The default window, from `Window::default_attributes` in `editor.rs`.
const DEFAULT_WIDTH: f32 = 900.0;

/// What `Editor::draw_toolbar` asks for: the body size times `TOOLBAR_ICON`.
const ICON_SIZE: f32 = 16.0 * 1.375;

fn engine() -> DirectWriteText {
    DirectWriteText::new().expect("DirectWrite is available on any supported Windows")
}

/// Lays out one character as the bar would, and reports its glyphs.
fn shape(text: &mut DirectWriteText, glyph: char, family: FontFamily, size: f32) -> Vec<f32> {
    let owned = glyph.to_string();
    let style = TextStyle {
        family,
        ..TextStyle::body(size)
    };
    let layout = text
        .layout(
            &[StyledRun {
                text: &owned,
                style,
                source: ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(owned.len())),
            }],
            1000.0,
        )
        .unwrap();
    layout
        .glyphs()
        .map(|g| {
            if g.key.glyph.is_notdef() {
                -1.0
            } else {
                g.advance
            }
        })
        .collect()
}

/// Every code point in the table is really in the font on this machine.
///
/// **Taken from the installed font rather than from a reference page.** A private-use code point is
/// a promise about a font, not a standard: `U+E8DD` is Bold in Segoe Fluent Icons and nothing at all
/// in a font that has not got it. Two of these were not on the plan's list at all — Fence and Rule
/// were "weak — check", and were chosen by rasterising the candidates and looking at them.
#[test]
fn every_icon_the_bar_asks_for_exists() {
    let mut text = engine();
    let mut missing = Vec::new();

    for action in [toolbar::STANDARD, toolbar::MARKDOWN].concat() {
        let Some(icon) = action.icon() else {
            continue;
        };
        let advances = shape(&mut text, icon, FontFamily::Icon, ICON_SIZE);
        if advances.iter().any(|advance| *advance < 0.0) {
            missing.push(format!("{action:?} (U+{:04X})", icon as u32));
        }
    }

    assert!(
        missing.is_empty(),
        "the icon font on this machine has no glyph for: {}",
        missing.join(", ")
    );
}

/// And the fallback has something to fall back *from*. Without this the test above could pass on a
/// machine where `.notdef` never comes back at all, and would then be measuring nothing.
#[test]
fn an_invented_code_point_comes_back_notdef() {
    let mut text = engine();
    let advances = shape(&mut text, '\u{F8FF}', FontFamily::Icon, ICON_SIZE);

    assert!(
        advances.iter().any(|advance| *advance < 0.0),
        "an unassigned private-use code point should be .notdef, which is the signal \
         `Editor::button_face` reads to draw a word instead"
    );
}

/// The three heading buttons have no icon, deliberately, and would be tofu if they were given one.
///
/// Asserted rather than assumed: the exception is only defensible while it is *true* that neither
/// font has a heading glyph, and this is the closest thing to evidence available — the letterforms
/// the bar draws instead resolve in the text font, and nothing is asked of the icon font at all.
#[test]
fn the_heading_buttons_have_no_icon_and_letter_instead() {
    let mut text = engine();

    for level in 1..=3u8 {
        let action = Action::Heading(level);
        assert_eq!(action.icon(), None, "{action:?} should have no code point");

        for character in action.label().chars() {
            let advances = shape(&mut text, character, FontFamily::Ui, 16.0 * 0.62);
            assert!(
                advances.iter().all(|advance| *advance >= 0.0),
                "{action:?} letters as {:?}, and {character:?} is not in the text font",
                action.label()
            );
        }
    }
}

/// **The whole bar fits a default window, measured against the real glyph advances.**
///
/// This is the guard for a defect the stub could not see and nearly shipped. At 22-pixel icons with
/// the mock-up's 8-pixel padding the row came to 930 logical pixels against a 900-pixel window, so
/// `toolbar::layout` did exactly what it is supposed to and dropped the last three buttons — Fence,
/// Rule and Link — on a fresh install, silently. Every integration test passed, because the stub
/// draws every glyph eight pixels wide.
///
/// The number below is therefore not decoration. If a future button, a larger icon or a wider pad
/// pushes it over again, this says so instead of the user finding out.
#[test]
fn all_of_the_buttons_fit_a_default_window() {
    let mut text = engine();

    let widths: Vec<f32> = [toolbar::STANDARD, toolbar::MARKDOWN]
        .concat()
        .iter()
        .map(|action| match action.icon() {
            Some(icon) => shape(&mut text, icon, FontFamily::Icon, ICON_SIZE)
                .iter()
                .sum(),
            // A lettered button measures the whole word, not one character of it.
            None => action
                .label()
                .chars()
                .map(|c| {
                    shape(&mut text, c, FontFamily::Ui, 16.0 * 0.62)
                        .iter()
                        .sum::<f32>()
                })
                .sum(),
        })
        .collect();

    let all = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    let placed = toolbar::layout(&all, &widths, DEFAULT_WIDTH);

    assert_eq!(
        placed.len(),
        all.len(),
        "only {} of {} buttons fit {DEFAULT_WIDTH}px — the row is {:.0}px wide and the ones cut are \
         at the far end, which is where Link lives",
        placed.len(),
        all.len(),
        toolbar::width_of(&toolbar::layout(&all, &widths, f32::MAX))
    );
}
