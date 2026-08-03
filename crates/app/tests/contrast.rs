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
//! measures combinations that never appear on screen. `marker` against `tab_active` is 4.31:1 in the
//! dark palette, which sounds like a defect and is one — but `token_comment` against `tab_active` is
//! 4.13:1 and is not, because a JSON comment is never drawn on a tab. The pack itself measures
//! everything against `bg` alone and so misses the pairs that are on chrome.
//!
//! So [`PAIRS`] is the list of foreground-on-background combinations the renderer actually produces,
//! taken from the draw sites in `editor.rs`. Adding one there means adding one here.
//!
//! # Text only
//!
//! WCAG's 4.5:1 is a rule about text. The caret, the selection tint, the find highlight, the
//! caret-line tint and the scrollbar thumb are not text and are not held to it here — the scrollbar
//! thumb is 2.07:1 against the page and has always been, which is a real finding recorded on
//! MDEDIT-72 rather than something this story may fix, because MDEDIT-71 changes no dark value.

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
        ground: |t| t.strip,
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
];

/// WCAG AA for body text, and the floor the design pack commits to.
const FLOOR: f64 = 4.5;

/// Pairs that do not clear [`FLOOR`] in the dark palette today.
///
/// **This list may only ever shrink.** It exists because MDEDIT-71 restructures the palette without
/// changing a single dark value — that was its acceptance criterion — so a shortfall already in the
/// editor cannot be fixed here. MDEDIT-72 replaces these values with Nocturne's and is where it goes.
///
/// It is asserted to be exactly this, not merely a superset, so a *new* shortfall fails the build
/// rather than joining the list quietly.
const KNOWN_DARK_SHORTFALLS: &[&str] = &["the close cross, active"];

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

#[test]
fn every_text_pair_is_legible_in_the_dark_palette() {
    let theme = Theme::dark();
    for pair in PAIRS {
        if KNOWN_DARK_SHORTFALLS.contains(&pair.what) {
            continue;
        }
        let measured = ratio((pair.foreground)(&theme), (pair.ground)(&theme));
        assert!(
            measured >= FLOOR,
            "{} is {measured:.2}:1 in the dark palette, under {FLOOR}",
            pair.what
        );
    }
}

/// The close cross on the *active* tab really is too dim, and saying so is the point of the list.
///
/// If MDEDIT-72 fixes it this test fails, which is the reminder to delete the entry.
#[test]
fn the_known_dark_shortfalls_are_still_exactly_that() {
    let theme = Theme::dark();
    let close_cross = ratio(theme.marker, theme.tab_active);
    assert!(
        close_cross < FLOOR,
        "the close cross on an active tab now clears {FLOOR}:1 at {close_cross:.2} — remove it from \
         KNOWN_DARK_SHORTFALLS"
    );
    assert_eq!(
        KNOWN_DARK_SHORTFALLS.len(),
        1,
        "a shortfall was added to the list rather than fixed"
    );
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
