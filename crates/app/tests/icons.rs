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
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::file_icon::Icon;
use mdedit_app::toolbar::{self, Action};
use mdedit_app::view::{Theme, as_styled, build_runs};
use mdedit_document::{Document, Selection};
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

    for action in faces() {
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

/// Every action whose icon the bar can draw: its buttons, and the faces the Lists button takes.
///
/// **Not just the buttons** since MDEDIT-144. Lists draws `Ordered`'s icon inside a numbered list,
/// and `Ordered` is no longer on the bar, so a sweep of the bar alone would never ask the font for
/// the face a numbered list shows.
fn faces() -> Vec<Action> {
    [toolbar::STANDARD, toolbar::MARKDOWN, toolbar::LISTS].concat()
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

    for action in faces() {
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
///
/// **MDEDIT-144 bought one button back**: Bullet and Numbered became Lists, and twenty-one buttons
/// are 827px again, leaving 73px. The ticket first said two buttons' worth, which was wrong — two
/// became one.
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

/// Where a task item's text starts, laid out as the editor lays it out.
fn task_text_left(text: &mut DirectWriteText, source: &str) -> f32 {
    let runs = build_runs(&Document::from(source), &Theme::default(), source);
    let layout = text.layout(&as_styled(&runs), 1000.0).unwrap();
    // The first glyph of the text, which starts at byte 6 in both spellings. The box and its space
    // are substituted, so they address the `[` at byte 2.
    layout
        .glyphs()
        .find(|glyph| glyph.source.start().get() >= 6)
        .expect("the item has text")
        .pos
        .x
}

/// **Ticking a box does not move the item's text** (MDEDIT-142).
///
/// It did, by 8 px, in the first build anyone looked at. Unicode's ☐ and ☑ look like a pair and
/// are not: ☑ has an emoji form, so DirectWrite drew it from Segoe UI Emoji at 22 px of advance and
/// ☐ from a symbol font at 13.8. The stub gives every glyph one width, so only the real font can
/// show it — which is why this is here and not in `lists.rs`.
#[test]
fn ticking_a_box_does_not_move_the_text() {
    let mut text = engine();
    let unticked = task_text_left(&mut text, "- [ ] buy milk\n");
    let ticked = task_text_left(&mut text, "- [x] buy milk\n");
    assert!(unticked > 0.0, "the box takes no room at all");
    assert_eq!(unticked, ticked, "the text moved when the box was ticked");
}

/// Both boxes are really in the icon font this machine has, and draw something.
#[test]
fn both_task_boxes_are_in_the_font() {
    let mut text = engine();
    for (source, name) in [("- [ ] a\n", "unticked"), ("- [x] a\n", "ticked")] {
        let runs = build_runs(&Document::from(source), &Theme::default(), source);
        let layout = text.layout(&as_styled(&runs), 1000.0).unwrap();
        let first = layout.glyphs().next().expect("a box is drawn");
        assert!(!first.key.glyph.is_notdef(), "the {name} box is .notdef");
        let image = text.rasterize(first.key, 1.0).unwrap();
        assert!(
            image.is_some_and(|image| image.coverage.iter().any(|px| *px > 0)),
            "the {name} box has no ink"
        );
    }
}

/// **Each file-pane icon is really in the icon font on this machine, and they are three different
/// shapes** (MDEDIT-174). The reference list is not evidence: its `Document`, `Page` and
/// `MusicInfo` code points rasterise to one identical glyph in this font, which is how the first
/// choice of three was found to be one.
#[test]
fn every_pane_icon_is_in_the_font_and_they_differ() {
    let mut text = engine();
    let icons = [Icon::Note, Icon::Structured, Icon::Text];

    let mut shapes: Vec<Vec<u8>> = Vec::new();
    for icon in icons {
        let glyph = icon.glyph();
        let advances = shape(&mut text, glyph, FontFamily::Icon, ICON_SIZE);
        assert!(
            advances.iter().all(|advance| *advance >= 0.0),
            "the icon font has no glyph for {icon:?} (U+{:04X})",
            u32::from(glyph),
        );
        let fraction = ink(&mut text, glyph);
        assert!(
            fraction.is_some_and(|lit| lit > 0.0),
            "{icon:?} is in the font and rasterises to nothing",
        );

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
        let key = layout.glyphs().next().unwrap().key;
        shapes.push(text.rasterize(key, 1.0).unwrap().unwrap().coverage);
    }

    for (first, second) in [(0, 1), (1, 2), (0, 2)] {
        assert_ne!(
            shapes[first], shapes[second],
            "{:?} and {:?} rasterise to the same shape",
            icons[first], icons[second],
        );
    }
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

/// **Both of Lists' faces are the same width** (MDEDIT-144), so the bar does not shift sideways when
/// the caret walks into a numbered list. Measured, because the two are different glyphs and the
/// button's width is its face's.
#[test]
fn the_lists_faces_are_one_width() {
    let mut text = engine();
    let bullet = measure(&mut text, Action::Bullet);
    let ordered = measure(&mut text, Action::Ordered);
    assert!(
        (bullet - ordered).abs() < 0.5,
        "the bullet face is {bullet:.1}px and the numbered one {ordered:.1}px, so the bar moves \
         as the caret changes list"
    );
    assert_eq!(measure(&mut text, Action::Lists), bullet);
}

/// **Inside a numbered list, Lists draws the numbered icon** — asked of a real editor on the real
/// font, because the stub has no icon font and draws each face's word, and `•` and `1.` differ even
/// when the icon behind them would not. A face keyed right and drawn from the wrong action's icon is
/// exactly the defect that word hides.
#[test]
fn the_lists_button_draws_the_carets_kind_of_icon() {
    let source = "- a bullet\n\n1. a number\n";
    let engine = DirectWriteText::new().expect("DirectWrite is available on any supported Windows");
    let mut editor = Editor::new(source, engine);
    for (caret, kind) in [(3, Action::Bullet), (source.len() - 3, Action::Ordered)] {
        editor
            .document_mut()
            .set_selection(Selection::caret(ByteOffset::new(caret)));
        let _ = editor.build_frame(1200.0, 600.0, 1.0);
        let icon = kind.icon().expect("both kinds have an icon").to_string();
        assert_eq!(
            editor.button_face_drawn(Action::Lists),
            Some(icon.as_str()),
            "caret at {caret}"
        );
    }
}
