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

/// What a deliberate letterform is set at — the heading buttons. `TOOLBAR_LETTERS`.
const LETTERS_SIZE: f32 = 16.0 * 1.25;

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
            &[StyledRun::from_buffer(
                &owned,
                style,
                ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(owned.len())),
            )],
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

/// How much ink one icon rasterises to, as a fraction of its bitmap. `None` when the glyph has no
/// ink at all, which is what [`TextLayout::rasterize`] returns for a space.
fn ink(text: &mut DirectWriteText, glyph: char) -> Option<f32> {
    let owned = glyph.to_string();
    let style = TextStyle {
        family: FontFamily::Icon,
        ..TextStyle::body(ICON_SIZE)
    };
    let layout = text
        .layout(
            &[StyledRun::from_buffer(
                &owned,
                style,
                ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(owned.len())),
            )],
            1000.0,
        )
        .unwrap();
    let key = layout.glyphs().next()?.key;
    let image = text.rasterize(key, 1.0).unwrap()?;
    let lit = image.coverage.iter().filter(|px| **px > 0).count();
    #[allow(
        clippy::cast_precision_loss,
        reason = "a glyph bitmap is a few hundred pixels"
    )]
    let fraction = lit as f32 / image.coverage.len().max(1) as f32;
    Some(fraction)
}

/// **Every icon the bar draws has ink, which is not the claim the test below makes.**
///
/// `every_icon_the_bar_asks_for_exists` checks the glyph is not `.notdef`. A font can answer a
/// private-use code point with a *blank* glyph that passes that check, measures a normal advance,
/// and draws nothing — leaving a button-shaped gap on the bar with no overflow marker to explain it.
/// Nothing here could have caught that, which is the lesson `AGENTS.md` records about the tooltip
/// tests: a check that cannot fail for the reason you care about is not a check.
///
/// **There is no upper bound, and finding out why is what this test was worth.** The first version
/// rejected anything over 95% coverage on the reasoning that a solid box is the other way a wrong
/// code point goes — and `Rule` failed it at 100%. `Rule` is `U+E738`, which is literally a
/// horizontal line: its bitmap is a few pixels tall and every one of them is ink. A correct icon
/// can fill its own bounding box, so the only defensible assertion is that there is ink at all.
#[test]
fn every_icon_the_bar_draws_has_ink() {
    let mut text = engine();
    let mut blank = Vec::new();

    for action in [toolbar::STANDARD, toolbar::MARKDOWN].concat() {
        let Some(icon) = action.icon() else {
            continue;
        };
        if ink(&mut text, icon).is_none_or(|fraction| fraction <= 0.0) {
            blank.push(format!("{action:?} (U+{:04X})", u32::from(icon)));
        }
    }

    assert!(
        blank.is_empty(),
        "these code points are in the font, are not .notdef, and rasterise to nothing — so the bar \
         draws a gap where the button is: {}",
        blank.join(", ")
    );
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
            let advances = shape(&mut text, character, FontFamily::Ui, LETTERS_SIZE);
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
        .map(|action| measure(&mut text, *action))
        .collect();

    let all = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    let row = toolbar::overflowing(&all, &widths, DEFAULT_WIDTH);

    assert!(
        row.dropped.is_empty(),
        "{} of {} buttons did not fit {DEFAULT_WIDTH}px — the row is {:.0}px wide. Nothing is lost \
         now that MDEDIT-116 has an overflow button, but the default window having one at all is a \
         decision rather than an accident",
        row.dropped.len(),
        all.len(),
        toolbar::width_of(&toolbar::layout(&all, &widths, f32::MAX))
    );
}

/// **The room MDEDIT-116 made, now spent.**
///
/// This used to guard a future: collapsing three heading buttons into one is what paid for Image and
/// Table, and it asserted that adding two more buttons would still fit. Both have landed — Image on
/// MDEDIT-87, Table on MDEDIT-127 — so it now asserts the arithmetic that argued for doing it that
/// way instead of shaving the padding again.
///
/// **The numbers, measured on this machine rather than taken from the plan.** Twenty-one buttons is
/// 827px; Table's own button is 34px; twenty-two is 861px of a 900px default window, leaving 39px.
/// MDEDIT-127's plan quoted 861px as the figure *without* Table, which it was not — that measurement
/// was taken on MDEDIT-87's branch while Table was still on it. The correction is in this direction:
/// there is one more button's room than the plan thought, not one less.
///
/// The headroom is the interesting number rather than the pass. One 34px button still fits inside
/// 39px; the one after it does not, and is the conversation.
#[test]
fn the_bar_still_fits_with_the_table_button() {
    let mut text = engine();
    let all = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    assert!(
        all.contains(&Action::Image) && all.contains(&Action::Table),
        "this is the test that says they fit, so it has to be looking at a bar that has them"
    );

    let widths: Vec<f32> = all
        .iter()
        .map(|action| measure(&mut text, *action))
        .collect();
    let row = toolbar::width_of(&toolbar::layout(&all, &widths, f32::MAX));

    assert!(
        row <= DEFAULT_WIDTH,
        "the row is {row:.0}px against a {DEFAULT_WIDTH}px default window, so a fresh install opens \
         with an overflow button — which is a decision rather than an accident"
    );
    // Not a hard floor, and deliberately generous: what it catches is the row creeping up to the
    // edge unnoticed, at which point the *next* button is the conversation.
    let headroom = DEFAULT_WIDTH - row;
    assert!(
        headroom >= 20.0,
        "only {headroom:.0}px of headroom left at {DEFAULT_WIDTH}px — the bar is one button from \
         overflowing on a default window, and that is worth deciding rather than discovering"
    );
}

/// **The bullets a list draws are really in the font this machine has** (MDEDIT-141, risk R2).
///
/// A substituted list marker is drawn monospaced, like every marker, so `view::BULLETS` is asked of
/// the mono family rather than the UI one. A code point that is not there comes back `.notdef` and
/// the editor draws tofu where the bullet should be — which is MDEDIT-92's failure mode, and the
/// reason it is asked here rather than assumed against a Unicode chart.
#[test]
fn every_bullet_a_list_draws_is_in_the_font() {
    let mut text = engine();
    let mut missing = Vec::new();

    for bullet in ['\u{2022}', '\u{25e6}', '\u{25aa}'] {
        let advances = shape(&mut text, bullet, FontFamily::Mono, 16.0);
        if advances.iter().any(|advance| *advance < 0.0) {
            missing.push(format!("U+{:04X}", u32::from(bullet)));
        }
    }

    assert!(
        missing.is_empty(),
        "the mono font on this machine has no glyph for: {}",
        missing.join(", ")
    );
}

/// One button's face, measured as the bar measures it.
fn measure(text: &mut DirectWriteText, action: Action) -> f32 {
    match action.icon() {
        Some(icon) => shape(text, icon, FontFamily::Icon, ICON_SIZE).iter().sum(),
        None => action
            .label()
            .chars()
            .map(|c| {
                shape(text, c, FontFamily::Ui, LETTERS_SIZE)
                    .iter()
                    .sum::<f32>()
            })
            .sum(),
    }
}
