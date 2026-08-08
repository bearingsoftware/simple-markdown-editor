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

/// A blank line has no glyphs, so it used to produce no line at all — and the vertical space it
/// occupies went missing along with it. Virtualization measures blocks by summing line heights, so
/// that silently shortened every document containing a paragraph break.
#[test]
fn a_blank_line_still_occupies_a_line() {
    let mut text = engine();
    let style = TextStyle::body(16.0);

    let one = text.layout(&[run("a", style)], 1000.0).unwrap();
    let three = text.layout(&[run("a\n\nb", style)], 1000.0).unwrap();

    assert_eq!(one.lines.len(), 1);
    assert_eq!(three.lines.len(), 3, "text, blank, text");
    assert!(
        three.lines[1].glyphs.is_empty(),
        "the middle line is the blank one"
    );
    assert!(three.lines[1].height > 0.0, "and it still takes up space");

    let single = one.height();
    assert!(
        (three.height() - single * 3.0).abs() < single * 0.1,
        "three lines should be about three times one: {} vs {}",
        three.height(),
        single
    );
}

/// Every line's height must be its own. They used to be indexed by position among the lines that
/// had glyphs, so one blank line shifted every height after it onto the wrong line.
#[test]
fn each_line_carries_its_own_height() {
    let mut text = engine();
    let layout = text
        .layout(
            &[
                run("small\n", TextStyle::body(12.0)),
                run("\n", TextStyle::body(12.0)),
                run("LARGE", TextStyle::body(48.0)),
            ],
            1000.0,
        )
        .unwrap();

    assert_eq!(layout.lines.len(), 3);
    assert!(
        layout.lines[2].height > layout.lines[0].height * 2.0,
        "the 48 px line should dwarf the 12 px one: {:?}",
        layout.lines.iter().map(|l| l.height).collect::<Vec<_>>()
    );
    // Baselines increase down the page, and the blank line between them is not skipped.
    assert!(layout.lines[0].baseline < layout.lines[1].baseline);
    assert!(layout.lines[1].baseline < layout.lines[2].baseline);
}

/// MDEDIT-73. The rule's position and thickness come from the font, which is the whole reason this
/// is the backend's answer — so this is the only place either can be asserted at all.
#[test]
fn an_underlined_run_gets_a_rule_from_the_font() {
    let mut text = engine();
    let source = "underlined";
    let layout = text
        .layout(&[run(source, TextStyle::body(16.0).underline())], 1000.0)
        .unwrap();

    let line = &layout.lines[0];
    assert_eq!(line.rules.len(), 1, "one run, one row, one rule");
    let rule = line.rules[0];

    assert!(rule.thickness > 0.0, "a rule with no thickness is no rule");
    assert!(
        rule.top > line.baseline,
        "an underline sits below the baseline: {} against {}",
        rule.top,
        line.baseline
    );
    let ends = layout
        .glyphs()
        .map(|g| g.pos.x + g.advance)
        .fold(0.0_f32, f32::max);
    assert!(
        rule.width > 0.0 && rule.width <= ends + 1.0,
        "the rule ({}) should run under the text ({ends}) and no further",
        rule.width
    );
    assert!(
        rule.source.get() < source.len(),
        "the rule should name a byte of the text it is under, got {}",
        rule.source.get()
    );
}

/// MDEDIT-91, and the same claim as the underline above it: the font decides.
///
/// **The two are told apart by which side of the baseline they land on.** DirectWrite measures both
/// offsets downwards from it, so a strikethrough's is negative and an underline's is positive — which
/// is why `Rule` needed a `kind` field and the renderer needed no branch.
#[test]
fn a_struck_run_gets_a_rule_through_it_from_the_font() {
    let mut text = engine();
    let source = "struck";
    let layout = text
        .layout(
            &[run(source, TextStyle::body(16.0).strikethrough())],
            1000.0,
        )
        .unwrap();

    let line = &layout.lines[0];
    assert_eq!(line.rules.len(), 1, "one run, one row, one rule");
    let rule = line.rules[0];

    assert_eq!(rule.kind, mdedit_text::RuleKind::Through);
    assert!(rule.thickness > 0.0, "a rule with no thickness is no rule");
    assert!(
        rule.top < line.baseline,
        "a strikethrough sits above the baseline: {} against {}",
        rule.top,
        line.baseline
    );
    let ends = layout
        .glyphs()
        .map(|g| g.pos.x + g.advance)
        .fold(0.0_f32, f32::max);
    assert!(
        rule.width > 0.0 && rule.width <= ends + 1.0,
        "the rule ({}) should run through the text ({ends}) and no further",
        rule.width
    );
    assert!(
        rule.source.get() < source.len(),
        "the rule should name a byte of the text it strikes, got {}",
        rule.source.get()
    );
}

/// A run can be both, which is why the two flags are separate rather than one enum: `~~[a](/url)~~`
/// is a struck-through link, and the destination is underlined either way.
#[test]
fn a_run_can_be_underlined_and_struck_at_once() {
    let mut text = engine();
    let layout = text
        .layout(
            &[run(
                "both",
                TextStyle::body(16.0).underline().strikethrough(),
            )],
            1000.0,
        )
        .unwrap();

    let kinds: Vec<_> = layout.lines[0].rules.iter().map(|rule| rule.kind).collect();
    assert_eq!(kinds.len(), 2, "one of each, got {kinds:?}");
    assert!(kinds.contains(&mdedit_text::RuleKind::Under));
    assert!(kinds.contains(&mdedit_text::RuleKind::Through));
}

/// Wrapping splits a strikethrough per row, as it does an underline — the rule follows the text
/// rather than the run, which is the whole reason `Rule` is a rectangle and not a flag.
#[test]
fn a_wrapped_struck_run_gets_one_rule_per_row() {
    let mut text = engine();
    let layout = text
        .layout(
            &[run(
                "a struck sentence long enough to need more than one row of it",
                TextStyle::body(16.0).strikethrough(),
            )],
            120.0,
        )
        .unwrap();

    assert!(layout.lines.len() > 1, "the fixture has to wrap");
    for (index, line) in layout.lines.iter().enumerate() {
        assert!(
            !line.rules.is_empty(),
            "row {index} has text and no rule through it"
        );
    }
}

#[test]
fn a_run_that_asked_for_no_underline_gets_none() {
    let mut text = engine();
    let layout = text
        .layout(&[run("plain", TextStyle::body(16.0))], 1000.0)
        .unwrap();

    assert!(layout.lines.iter().all(|line| line.rules.is_empty()));
}

/// The rule follows the text through wrapping rather than being one rectangle per run.
#[test]
fn a_wrapped_underlined_run_is_underlined_on_every_row() {
    let mut text = engine();
    let layout = text
        .layout(
            &[run(
                "one two three four five six seven eight nine ten",
                TextStyle::body(16.0).underline(),
            )],
            60.0,
        )
        .unwrap();

    assert!(layout.lines.len() > 1, "60 px should wrap this");
    for (row, line) in layout.lines.iter().enumerate() {
        if line.glyphs.is_empty() {
            continue;
        }
        assert!(
            !line.rules.is_empty(),
            "row {row} has text and no rule under it"
        );
    }
}

#[test]
fn only_the_underlined_run_carries_a_rule() {
    let mut text = engine();
    let plain = "before ";
    let linked = "underlined";
    let layout = text
        .layout(
            &[
                run(plain, TextStyle::body(16.0)),
                StyledRun {
                    text: linked,
                    style: TextStyle::body(16.0).underline(),
                    source: ByteSpan::new(
                        ByteOffset::new(plain.len()),
                        ByteOffset::new(plain.len() + linked.len()),
                    ),
                },
            ],
            1000.0,
        )
        .unwrap();

    let rules: Vec<_> = layout
        .lines
        .iter()
        .flat_map(|line| line.rules.iter())
        .collect();
    assert_eq!(rules.len(), 1);
    assert!(
        rules[0].source.get() >= plain.len(),
        "the rule belongs to the second run, so it should name one of its bytes; got {}",
        rules[0].source.get()
    );
    assert!(
        rules[0].x > 0.0,
        "the rule starts where the second run does, not at the left margin"
    );
}

/// Zoom is a larger style, so this is what says a rule zooms with the text under it.
#[test]
fn a_rule_grows_with_the_style() {
    let mut text = engine();
    let small = text
        .layout(&[run("text", TextStyle::body(16.0).underline())], 1000.0)
        .unwrap();
    let large = text
        .layout(&[run("text", TextStyle::body(48.0).underline())], 1000.0)
        .unwrap();

    let rule = |layout: &mdedit_text::Layout| layout.lines[0].rules[0];
    let drop = |layout: &mdedit_text::Layout| rule(layout).top - layout.lines[0].baseline;

    assert!(
        rule(&large).thickness > rule(&small).thickness,
        "48 px should draw a thicker rule than 16 px: {} against {}",
        rule(&large).thickness,
        rule(&small).thickness
    );
    assert!(
        drop(&large) > drop(&small),
        "and sit further below the baseline: {} against {}",
        drop(&large),
        drop(&small)
    );
}

// ---- what this backend does with newlines and trailing spaces (MDEDIT-110) ----

/// **A `\n` gets no glyph, and trailing spaces do.**
///
/// Both halves are the opposite of what `AGENTS.md` recorded from MDEDIT-46 — which said DirectWrite
/// "gives `\n` a glyph of its own" and "does not shape trailing whitespace at all". Measured here on
/// Windows 11 with the installed Segoe UI:
///
/// ```text
/// "ab\ncd"   -> glyphs at bytes [0, 1, 3, 4]      the newline at 2 is absent
/// "ab  \ncd" -> glyphs at bytes [0, 1, 2, 3, 5, 6] both spaces present, the newline at 4 absent
/// "\n"       -> no glyphs at all
/// "ab   "    -> glyphs at bytes [0, 1, 2, 3, 4]   every trailing space present
/// ```
///
/// This is a **characterisation** test: its job is to fail if that ever changes, because two pieces
/// of the renderer were written on the strength of the description rather than the measurement —
/// `view::ending_before`, which exists to answer "is the offset past the last glyph the terminator
/// or past it", and the whitespace marks, which walk trailing spaces out of the *text* because they
/// were believed to have no glyphs. MDEDIT-113 carries re-checking both against this.
///
/// It asserts source-byte mapping rather than metrics, so it does not depend on which build of which
/// font is installed — the thing the header of this file warns about.
#[test]
fn a_newline_gets_no_glyph_and_trailing_spaces_do() {
    let mut text = engine();
    let mapped = |source: &str| -> Vec<usize> {
        let mut engine = engine();
        let layout = engine
            .layout(&[run(source, TextStyle::body(16.0))], 1000.0)
            .unwrap();
        layout
            .glyphs()
            .map(|glyph| glyph.source.start().get())
            .collect()
    };

    assert_eq!(
        mapped("ab\ncd"),
        vec![0, 1, 3, 4],
        "a newline between two words should produce no glyph of its own"
    );
    assert_eq!(
        mapped("\n"),
        Vec::<usize>::new(),
        "a lone newline should lay out to nothing"
    );
    assert_eq!(
        mapped("ab   "),
        vec![0, 1, 2, 3, 4],
        "trailing spaces should each be shaped"
    );
    assert_eq!(
        mapped("ab  \ncd"),
        vec![0, 1, 2, 3, 5, 6],
        "spaces before a newline should be shaped and the newline should not"
    );

    // And the layout still reports two lines, so the newline does its work without a glyph.
    let layout = text
        .layout(&[run("ab\ncd", TextStyle::body(16.0))], 1000.0)
        .unwrap();
    assert_eq!(layout.lines.len(), 2, "a newline should still break a line");
}
