//! What the palettes are legible against (MDEDIT-71).
//!
//! # Why this is computed rather than a table of numbers
//!
//! The MDEDIT-69 design pack ships measured contrast for eleven roles in two modes. Every one of
//! those numbers is correct — they were recomputed by hand and matched to two decimal places. A
//! table of them checked into a test would go on being correct right up until somebody changed a
//! colour, which is the moment it needed to say something. So the ratio is computed from the palette
//! and the threshold is asserted.
//!
//! # Why the pairs are named
//!
//! Contrast is a property of a *pair*, and asserting every foreground against every background
//! measures combinations that never appear on screen: a JSON comment is never drawn on a tab, so
//! whatever it measures there says nothing. The pack itself measures everything against `bg` alone
//! and so misses every pair that is on chrome — which is how its gutter came to be 4.22:1 on a
//! panel, the one value MDEDIT-72 does not take from it.
//!
//! So [`PAIRS`] is the list of foreground-on-background combinations the renderer actually produces,
//! taken from the draw sites in `editor.rs`. Adding one there means adding one here.
//!
//! # Text only
//!
//! WCAG's 4.5:1 is a rule about text. The caret, the selection tint, the find highlight, the
//! caret-line tint and the scrollbar thumb are not text and are not held to it here — Nocturne's
//! thumb is 2.01:1 against the page, where the palette it replaced was 2.07:1. What the tints *are*
//! held to is that text stays legible on them, and that the find highlight and the selection differ
//! by hue rather than by weight, both of which are asserted below.

use mdedit_app::view::{Appearance, Rgba, Theme};

/// Relative luminance, per WCAG 2.x.
///
/// The palette is sRGB and the surface is deliberately **not** an sRGB one — `editor.rs` picks a
/// non-sRGB format so these bytes reach the framebuffer unlinearised, which is exactly the condition
/// this formula assumes. See the surface-format choice in `Renderer::new`.
fn luminance(colour: Rgba) -> f64 {
    fn channel(value: f32) -> f64 {
        let value = f64::from(value);
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(colour.r) + 0.7152 * channel(colour.g) + 0.0722 * channel(colour.b)
}

/// The WCAG contrast ratio between two opaque colours, 1.0 to 21.0.
fn ratio(one: Rgba, two: Rgba) -> f64 {
    let (a, b) = (luminance(one), luminance(two));
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// `src` over `dst` with straight alpha, which is what the pipeline's `ALPHA_BLENDING` does.
///
/// Needed because Nocturne's selection and find highlight are real tints rather than opaque colours:
/// what the eye compares is each of them *blended against the page*, not the colour in the palette.
fn over(src: Rgba, dst: Rgba) -> Rgba {
    let mix = |s: f32, d: f32| s * src.a + d * (1.0 - src.a);
    Rgba {
        r: mix(src.r, dst.r),
        g: mix(src.g, dst.g),
        b: mix(src.b, dst.b),
        a: 1.0,
    }
}

/// A foreground the renderer draws, and what it draws it on.
struct Pair {
    what: &'static str,
    foreground: fn(&Theme) -> Rgba,
    ground: fn(&Theme) -> Rgba,
}

/// Every text pair the renderer actually produces.
///
/// Traced from `editor.rs`: the document draws on `background`; the tab strip fills with `strip` and
/// then each tab with `tab_active` or `tab_inactive`; the file pane fills with `strip` and gives the
/// active row `tab_active`; the status bar draws on `strip`.
const PAIRS: &[Pair] = &[
    // The document.
    Pair {
        what: "text on the page",
        foreground: |t| t.text,
        ground: |t| t.background,
    },
    Pair {
        what: "revealed markers",
        foreground: |t| t.marker,
        ground: |t| t.background,
    },
    Pair {
        what: "json keys",
        foreground: |t| t.token_key,
        ground: |t| t.background,
    },
    Pair {
        what: "strings",
        foreground: |t| t.token_string,
        ground: |t| t.background,
    },
    Pair {
        what: "numbers",
        foreground: |t| t.token_number,
        ground: |t| t.background,
    },
    Pair {
        what: "true/false/null",
        foreground: |t| t.token_literal,
        ground: |t| t.background,
    },
    Pair {
        what: "punctuation",
        foreground: |t| t.token_punctuation,
        ground: |t| t.background,
    },
    Pair {
        what: "comments",
        foreground: |t| t.token_comment,
        ground: |t| t.background,
    },
    // Roles nothing reads yet. Checked now because their values are chosen now, and a colour that
    // fails here is cheaper to find than one that ships in the story that starts drawing it.
    Pair {
        what: "line numbers",
        foreground: |t| t.gutter,
        ground: |t| t.background,
    },
    Pair {
        what: "link destinations",
        foreground: |t| t.link,
        ground: |t| t.background,
    },
    Pair {
        what: "advisories",
        foreground: |t| t.warn,
        ground: |t| t.background,
    },
    Pair {
        what: "blockquote text",
        foreground: |t| t.muted,
        ground: |t| t.background,
    },
    Pair {
        what: "a diagnostic's span",
        foreground: |t| t.span_error,
        ground: |t| t.background,
    },
    Pair {
        what: "text on a panel",
        foreground: |t| t.text,
        ground: |t| t.panel,
    },
    Pair {
        what: "secondary text on a panel",
        foreground: |t| t.muted,
        ground: |t| t.panel,
    },
    // The tab strip and the file pane.
    Pair {
        what: "the active tab's name",
        foreground: |t| t.text,
        ground: |t| t.tab_active,
    },
    Pair {
        what: "an inactive tab's name",
        foreground: |t| t.marker,
        ground: |t| t.tab_inactive,
    },
    Pair {
        what: "a pane row's name",
        foreground: |t| t.marker,
        ground: |t| t.panel,
    },
    Pair {
        what: "the modified dot, active",
        foreground: |t| t.tab_modified,
        ground: |t| t.tab_active,
    },
    Pair {
        what: "the modified dot, inactive",
        foreground: |t| t.tab_modified,
        ground: |t| t.tab_inactive,
    },
    Pair {
        what: "the close cross, inactive",
        foreground: |t| t.marker,
        ground: |t| t.tab_inactive,
    },
    Pair {
        what: "the close cross, active",
        foreground: |t| t.marker,
        ground: |t| t.tab_active,
    },
    // The status bar.
    Pair {
        what: "status bar fields",
        foreground: |t| t.text,
        ground: |t| t.strip,
    },
    Pair {
        what: "the breadcrumb",
        foreground: |t| t.marker,
        ground: |t| t.strip,
    },
    Pair {
        what: "the diagnostic count",
        foreground: |t| t.status_error,
        ground: |t| t.strip,
    },
    // The formatting bar, which fills with `strip` too. MDEDIT-84's standard set brought a second
    // state to a surface that had only one: a button that cannot act right now is drawn in `gutter`
    // rather than `text`. **Dimmed has to mean legible**, or it is indistinguishable from a
    // rendering fault — which is why it is asserted against the same floor as everything else here
    // rather than allowed a lower one for being deliberately quiet.
    Pair {
        what: "a live toolbar button",
        foreground: |t| t.text,
        ground: |t| t.strip,
    },
    Pair {
        what: "a dimmed toolbar button",
        foreground: |t| t.gutter,
        ground: |t| t.strip,
    },
];

/// WCAG AA for body text, and the floor the design pack commits to.
const FLOOR: f64 = 4.5;

#[test]
fn every_text_pair_is_legible_in_the_light_palette() {
    let theme = Theme::light();
    for pair in PAIRS {
        let measured = ratio((pair.foreground)(&theme), (pair.ground)(&theme));
        assert!(
            measured >= FLOOR,
            "{} is {measured:.2}:1 in the light palette, under {FLOOR}",
            pair.what
        );
    }
}

/// No exceptions any more. MDEDIT-71 carried one — the close cross on the active tab, at 4.31:1 —
/// because that story changed no dark value; Nocturne clears it at 5.25:1, so the list is gone
/// rather than shorter.
#[test]
fn every_text_pair_is_legible_in_the_dark_palette() {
    let theme = Theme::dark();
    for pair in PAIRS {
        let measured = ratio((pair.foreground)(&theme), (pair.ground)(&theme));
        assert!(
            measured >= FLOOR,
            "{} is {measured:.2}:1 in the dark palette, under {FLOOR}",
            pair.what
        );
    }
}

/// The pack's eleven numbers again, against the dark palette they were measured for.
///
/// Two roles are deliberately not the pack's and are absent here rather than asserted at a value
/// nobody chose: `gutter` is a step lighter so it also clears on `panel` (the pack measures against
/// the page alone and misses it), and the amber `status_error` is not the pack's rose `error`, which
/// is `span_error` and is checked.
#[test]
fn the_dark_palette_matches_the_numbers_the_design_pack_published() {
    let theme = Theme::dark();
    let published: &[(&str, Rgba, f64)] = &[
        ("marker", theme.marker, 6.08),
        ("key", theme.token_key, 7.29),
        ("string", theme.token_string, 9.81),
        ("number", theme.token_number, 9.04),
        ("constant", theme.token_literal, 8.27),
        ("link", theme.link, 8.72),
        ("error", theme.span_error, 7.29),
        ("warn", theme.warn, 9.58),
        ("text", theme.text, 14.54),
        ("muted", theme.muted, 6.08),
    ];
    for (role, colour, expected) in published {
        let measured = ratio(*colour, theme.background);
        assert!(
            (measured - expected).abs() < 0.01,
            "{role} measures {measured:.2}:1 against the page; the pack published {expected:.2}"
        );
    }
}

/// The gutter is the one role deliberately lifted off the pack's value, and this is why.
#[test]
fn the_gutter_clears_the_floor_on_a_panel_as_well_as_on_the_page() {
    for (mode, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        let on_panel = ratio(theme.gutter, theme.panel);
        assert!(
            on_panel >= FLOOR,
            "the {mode} gutter is {on_panel:.2}:1 on a panel, under {FLOOR} — the pack's own \
             #82869a is 4.22 there, which is why this one is a step lighter"
        );
    }
}

/// The status bar's count is amber and an in-document span is rose, and MDEDIT-72 must not quietly
/// merge them into the pack's single `error` role.
#[test]
fn the_status_count_and_a_diagnostic_span_are_still_two_colours() {
    for (mode, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        assert_ne!(
            theme.status_error, theme.span_error,
            "the {mode} palette collapsed the amber count into the rose span"
        );
    }
}

/// MDEDIT-77 stacks two tints, and the pack models neither pairing.
///
/// The caret's line is tinted, a selection on that line is tinted over it, and the text is drawn
/// through both. The pack measures every role against `bg` alone and states that "selection tints
/// preserve at least 4.5:1" — which is a claim about one tint, not two.
#[test]
fn text_stays_legible_through_the_caret_line_and_a_selection_on_top_of_it() {
    for (mode, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        let lit = over(theme.caret_line, theme.background);
        let both = over(theme.selection, lit);
        for (what, ground) in [("the caret's line", lit), ("a selection on it", both)] {
            let measured = ratio(theme.text, ground);
            assert!(
                measured >= FLOOR,
                "{mode} text on {what} is {measured:.2}:1, under {FLOOR}"
            );
        }
        // And the gutter's own cue: the caret's number steps up to `text` over the same tint.
        let stepped = ratio(theme.text, lit);
        let ordinary = ratio(theme.gutter, theme.background);
        assert!(
            stepped > ordinary,
            "{mode}: the caret's line number ({stepped:.2}:1) must read as brighter than an \
             ordinary one ({ordinary:.2}:1), or the gutter's second cue says nothing"
        );
    }
}

/// The find highlight is told from the selection by hue, not by brightness.
///
/// Both are tints over the page, so they are compared blended. A ratio near 1.0 is the *intent*:
/// two grounds of the same weight, one warm and one cool. Told apart by lightness instead, they
/// would be a pair that peripheral vision reads as "the same thing, slightly different".
#[test]
fn the_find_highlight_and_the_selection_are_a_hue_apart_not_a_brightness_apart() {
    for (mode, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        let found = over(theme.find_match, theme.background);
        let selected = over(theme.selection, theme.background);
        // **They have to be different colours**, which this never checked (MDEDIT-110): the
        // brightness assertion below is happiest of all when the two are identical, and identical is
        // the one answer the whole warm/cool pair exists to rule out.
        let distance = (found.r - selected.r).abs()
            + (found.g - selected.g).abs()
            + (found.b - selected.b).abs();
        assert!(
            distance > 0.05,
            "the {mode} find highlight and selection blend to nearly the same colour \
             ({found:?} against {selected:?}) — told apart by neither hue nor weight"
        );

        let apart = ratio(found, selected);
        assert!(
            apart < 1.6,
            "the {mode} find highlight is {apart:.2}:1 against the selection — that is a brightness \
             distinction, and the point of the warm/cool pair is that it is not one"
        );
        assert!(
            ratio(theme.text, found) >= FLOOR,
            "{mode} text is illegible on a find highlight"
        );
        assert!(
            ratio(theme.text, over(theme.selection, found)) >= FLOOR,
            "{mode} text is illegible on the current match, where the selection sits over the \
             highlight"
        );
    }
}

/// The pack's own eleven numbers, against the light palette they were measured for.
///
/// Not a duplicate of the sweep above: this checks the *values* are the ones the design pack
/// specified, so a colour edited by hand is caught even if it happens to stay legible.
#[test]
fn the_light_palette_matches_the_numbers_the_design_pack_published() {
    let theme = Theme::light();
    let published: &[(&str, Rgba, f64)] = &[
        ("marker", theme.marker, 6.17),
        ("key", theme.token_key, 6.39),
        ("string", theme.token_string, 5.76),
        ("number", theme.token_number, 6.34),
        ("constant", theme.token_literal, 6.33),
        ("link", theme.link, 6.83),
        ("error", theme.span_error, 7.50),
        ("warn", theme.warn, 5.98),
        ("text", theme.text, 13.34),
        ("muted", theme.muted, 6.17),
        ("gutter", theme.gutter, 5.22),
    ];
    for (role, colour, expected) in published {
        let measured = ratio(*colour, theme.background);
        assert!(
            (measured - expected).abs() < 0.01,
            "{role} measures {measured:.2}:1 against the page; the pack published {expected:.2}"
        );
    }
}

/// Dark stays reachable through `Default`, so MDEDIT-71 changed no caller.
#[test]
fn the_default_palette_is_still_dark() {
    let by_default = Theme::default();
    let dark = Theme::dark();
    assert_eq!(by_default.background, dark.background);
    assert_eq!(by_default.text, dark.text);
    assert_eq!(by_default.marker, dark.marker);
    // Both come from the same literal, so they are bit-identical rather than merely close — but
    // clippy is right that a float is not a thing to compare with `==`, and a size that drifted by
    // a hair would be a real defect rather than a rounding artefact.
    assert!((by_default.body_size - dark.body_size).abs() < f32::EPSILON);
    assert_eq!(Theme::for_appearance(Appearance::default()).text, dark.text);
    assert_eq!(
        Theme::for_appearance(Appearance::Light).text,
        Theme::light().text
    );
}

/// The two palettes are genuinely two, and the light one is actually light.
#[test]
fn the_light_palette_is_lighter_than_its_text() {
    let light = Theme::light();
    assert!(
        luminance(light.background) > luminance(light.text),
        "the light palette's page is darker than its text"
    );
    let dark = Theme::dark();
    assert!(
        luminance(dark.background) < luminance(dark.text),
        "the dark palette's page is lighter than its text"
    );
}
