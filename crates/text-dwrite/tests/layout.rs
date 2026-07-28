//! DirectWrite layout and rasterization, against the real system font stack.
//!
//! These run only on Windows and only against installed fonts, so they assert properties rather
//! than exact numbers — a metric that depends on which build of Segoe UI is present would be a
//! test that fails on someone else's machine for no reason.
//!
//! The exception is [`han_resolves_to_a_real_glyph`], which asserts something specific and
//! load-bearing: the claim that justified choosing DirectWrite over parley.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{StyledRun, TextLayout, TextStyle};
use mdedit_text_dwrite::DirectWriteText;

fn engine() -> DirectWriteText {
    DirectWriteText::new().expect("DirectWrite is available on any supported Windows")
}

fn run(text: &str, style: TextStyle) -> StyledRun<'_> {
    StyledRun {
        text,
        style,
        source: ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(text.len())),
    }
}

#[test]
fn lays_out_latin_text_into_glyphs() {
    let mut text = engine();
    let style = TextStyle::body(16.0);
    let layout = text
        .layout(&[run("The quick brown fox", style)], 1000.0)
        .unwrap();

    assert_eq!(layout.lines.len(), 1, "one short line should not wrap");
    let glyphs: Vec<_> = layout.glyphs().collect();
    assert!(
        glyphs.len() >= 15,
        "19 characters should shape to a comparable number of glyphs, got {}",
        glyphs.len()
    );
    assert!(
        glyphs.windows(2).all(|w| w[0].pos.x <= w[1].pos.x),
        "glyphs on one line should advance left to right"
    );
}

/// The assertion this whole implementation exists for.
///
/// MDEDIT-5 established that `fontique` returns `.notdef` for these characters on Windows even
/// with ten capable families installed, because it resolves `script = Hani` to a single fallback
/// family and stops. Glyph 0 is `.notdef` in every OpenType font by definition, so this is a
/// direct test of the same failure — and it must pass here.
#[test]
fn han_resolves_to_a_real_glyph() {
    let mut text = engine();
    let layout = text
        .layout(&[run("汉字编辑器", TextStyle::body(16.0))], 1000.0)
        .unwrap();

    let glyphs: Vec<_> = layout.glyphs().collect();
    assert!(!glyphs.is_empty(), "Han text produced no glyphs at all");

    let tofu: Vec<_> = glyphs.iter().filter(|g| g.key.glyph.is_notdef()).collect();
    assert!(
        tofu.is_empty(),
        "{} of {} Han glyphs came back as .notdef — this is the parley#644 failure, and \
         DirectWrite was chosen precisely because it does not have it",
        tofu.len(),
        glyphs.len()
    );
}

/// Fallback has to switch mid-run, not just once per layout.
#[test]
fn mixed_scripts_in_one_run_all_resolve() {
    let mut text = engine();
    let layout = text
        .layout(
            &[run("Editor 编辑器 محرر 🎉", TextStyle::body(16.0))],
            1000.0,
        )
        .unwrap();

    let glyphs: Vec<_> = layout.glyphs().collect();
    let tofu = glyphs.iter().filter(|g| g.key.glyph.is_notdef()).count();
    assert_eq!(
        tofu,
        0,
        "mixed-script line produced {tofu} tofu glyphs of {}",
        glyphs.len()
    );

    let fonts: std::collections::BTreeSet<_> = glyphs.iter().map(|g| g.key.font).collect();
    assert!(
        fonts.len() > 1,
        "four scripts in one line should resolve to more than one face; got {}",
        fonts.len()
    );
}

#[test]
fn a_larger_style_produces_a_wider_line() {
    let mut text = engine();
    let body = text
        .layout(&[run("Heading", TextStyle::body(16.0))], 1000.0)
        .unwrap();
    let heading = text
        .layout(&[run("Heading", TextStyle::body(32.0).bold())], 1000.0)
        .unwrap();

    let width = |layout: &mdedit_text::Layout| -> f32 {
        layout
            .glyphs()
            .map(|g| g.pos.x + g.advance)
            .fold(0.0_f32, f32::max)
    };

    assert!(
        width(&heading) > width(&body),
        "32px heading ({}) should be wider than 16px body ({})",
        width(&heading),
        width(&body)
    );
}

#[test]
fn glyphs_carry_the_source_bytes_they_came_from() {
    let mut text = engine();
    let source = "hello";
    let layout = text
        .layout(&[run(source, TextStyle::body(16.0))], 1000.0)
        .unwrap();

    for glyph in layout.glyphs() {
        let start = glyph.source.start().get();
        assert!(
            start < source.len(),
            "glyph maps to byte {start}, outside the {} byte run",
            source.len()
        );
    }

    let first = layout.glyphs().next().unwrap();
    assert_eq!(
        first.source.start(),
        ByteOffset::ZERO,
        "the first glyph of a run should map to its first byte"
    );
}

#[test]
fn rasterizing_a_letter_produces_ink() {
    let mut text = engine();
    let layout = text
        .layout(&[run("H", TextStyle::body(48.0))], 1000.0)
        .unwrap();
    let glyph = layout.glyphs().next().expect("H produced a glyph");

    let image = text
        .rasterize(glyph.key, 1.0)
        .unwrap()
        .expect("a capital H has ink");

    assert!(image.width > 0 && image.height > 0);
    assert_eq!(
        image.coverage.len(),
        (image.width * image.height) as usize,
        "coverage must be exactly one byte per pixel"
    );
    assert!(
        image.coverage.iter().any(|&px| px > 0),
        "every pixel came back transparent, so nothing was rasterized"
    );
    assert!(
        image.top < 0,
        "a capital letter sits above the baseline, so its top offset should be negative; got {}",
        image.top
    );
}

#[test]
fn a_space_has_no_ink_and_is_not_an_error() {
    let mut text = engine();
    let layout = text
        .layout(&[run("a b", TextStyle::body(24.0))], 1000.0)
        .unwrap();

    // Whichever glyph is the space, rasterizing every glyph must not error.
    let blanks = layout
        .glyphs()
        .filter(|g| text.rasterize(g.key, 1.0).unwrap().is_none())
        .count();
    assert!(
        blanks >= 1,
        "the space in \"a b\" should rasterize to no ink rather than an empty bitmap"
    );
}

#[test]
fn rasterizing_at_a_higher_scale_factor_produces_a_bigger_bitmap() {
    let mut text = engine();
    let layout = text
        .layout(&[run("H", TextStyle::body(16.0))], 1000.0)
        .unwrap();
    let key = layout.glyphs().next().unwrap().key;

    let at_1x = text.rasterize(key, 1.0).unwrap().unwrap();
    let at_2x = text.rasterize(key, 2.0).unwrap().unwrap();

    assert!(
        at_2x.height > at_1x.height,
        "200% DPI should rasterize taller: {} vs {}",
        at_2x.height,
        at_1x.height
    );
}

#[test]
fn an_unknown_font_id_is_rejected_rather_than_panicking() {
    let mut text = engine();
    let key = mdedit_text::GlyphKey {
        font: mdedit_text::FontId::new(9_999),
        glyph: mdedit_text::GlyphId::new(1),
        size: mdedit_text::FontSize::new(16.0),
    };
    assert!(matches!(
        text.rasterize(key, 1.0),
        Err(mdedit_text::TextError::UnknownGlyph { .. })
    ));
}

#[test]
fn empty_input_lays_out_to_nothing() {
    let mut text = engine();
    assert!(text.layout(&[], 1000.0).unwrap().lines.is_empty());
    assert!(
        text.layout(&[run("", TextStyle::body(16.0))], 1000.0)
            .unwrap()
            .lines
            .is_empty()
    );
}
