//! Markdown's syntax, coloured per kind (MDEDIT-75).
//!
//! All thirteen `Marker` variants used to collapse to one colour: a heading's `#`, a bullet, a
//! fence and a link's `](` were the same green. The design pack's markdown scope map assumes they
//! are separable, and this is where that separation is pinned down.
//!
//! Everything here is asserted on the run builder, which knows a run's colour and style without a
//! window. Concealment is a separate question — what these say is that a marker which *is* drawn is
//! drawn in the right colour, and one test says concealment did not change while that happened.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::view::{Decoration, Mode, Rgba, Theme, build_runs_in};
use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{FontFamily, TextStyle};

/// One drawn run, owned so the document it came from does not have to outlive the assertion.
struct Drawn {
    text: String,
    colour: Rgba,
    style: TextStyle,
}

/// The runs for `source` in **plain markup**, which since MDEDIT-118 is the only mode that draws a
/// marker at all. What a marker is coloured is still a real question — plain markup is where you go
/// to read and edit the bytes, and a document whose syntax is all one green is what MDEDIT-75 fixed.
///
/// This used to take a caret, because reveal-on-caret decided what was drawn. Nothing reveals now,
/// so a preview run list has no markers in it to assert a colour on.
fn drawn(source: &str) -> Vec<Drawn> {
    drawn_in(source, Mode::Plain)
}

fn drawn_in(source: &str, mode: Mode) -> Vec<Drawn> {
    let doc = Document::from(source);
    let theme = Theme::default();
    build_runs_in(
        &doc,
        Decoration {
            theme: &theme,
            mode,
        },
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
        source,
    )
    .iter()
    .map(|run| Drawn {
        text: run.text.clone().into_owned(),
        colour: run.colour,
        style: run.style,
    })
    .collect()
}

/// The one run whose text is exactly `needle`.
fn run_of<'a>(runs: &'a [Drawn], needle: &str) -> &'a Drawn {
    runs.iter()
        .find(|run| run.text == needle)
        .unwrap_or_else(|| {
            panic!(
                "no run is exactly {needle:?}; the runs are {:?}",
                runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>()
            )
        })
}

fn colour_of(source: &str, needle: &str) -> Rgba {
    run_of(&drawn(source), needle).colour
}

#[test]
fn a_bullet_is_not_the_colour_a_heading_marker_is() {
    let theme = Theme::default();
    // Caret on the line, which is what reveals a block marker.
    let bullet = colour_of("- an item\n", "- ");
    assert_eq!(bullet, theme.token_key, "a bullet takes the key colour");

    let hash = colour_of("# A heading\n", "# ");
    assert_eq!(hash, theme.marker, "a heading marker stays marker-coloured");
    assert_ne!(
        bullet, hash,
        "a bullet and a heading marker are still one colour, which is what this story removed"
    );
}

/// The largest of the three, because a URL in the marker colour is indistinguishable from the
/// brackets holding it.
#[test]
fn a_link_destination_is_link_coloured_and_underlined() {
    let theme = Theme::default();
    let source = "see [the docs](https://example.com) here\n";
    let runs = drawn(source);

    let destination = run_of(&runs, "https://example.com");
    assert_eq!(destination.colour, theme.link);
    assert!(
        destination.style.underline,
        "the pack's `link` role is underlined, and MDEDIT-73 is what made that drawable"
    );

    // The brackets around it are still syntax, and are not underlined.
    for bracket in ["](", ")"] {
        let run = run_of(&runs, bracket);
        assert_eq!(run.colour, theme.marker, "{bracket} should be syntax");
        assert!(!run.style.underline, "{bracket} should not be underlined");
    }
}

#[test]
fn link_text_takes_the_key_colour() {
    let theme = Theme::default();
    let source = "see [the docs](https://example.com) here\n";
    assert_eq!(colour_of(source, "the docs"), theme.token_key);
    // And the prose either side of it does not.
    assert_eq!(colour_of(source, "see "), theme.text);
    assert_eq!(colour_of(source, " here\n"), theme.text);
}

#[test]
fn an_image_destination_is_coloured_like_a_link() {
    let theme = Theme::default();
    let source = "![alt](picture.png)\n";
    assert_eq!(colour_of(source, "picture.png"), theme.link);
    // `![` is its own segment, so it is revealed by a caret on *it* rather than by one in the
    // destination — which is the concealment rule doing its job, not a colouring miss.
    assert_eq!(colour_of(source, "!["), theme.marker);
}

/// **Preview**, not plain markup: this is about the code *content*, which is drawn in both, and the
/// mono family is a preview decoration — plain markup draws the whole document in the UI face.
#[test]
fn code_content_is_string_coloured_and_stays_mono() {
    let theme = Theme::default();

    let span = drawn_in("a `code span` b\n", Mode::Preview);
    let inline = run_of(&span, "code span");
    assert_eq!(inline.colour, theme.token_string);
    assert_eq!(inline.style.family, FontFamily::Mono);

    let fenced = drawn_in("```\nlet x = 1;\n```\n", Mode::Preview);
    let block = run_of(&fenced, "let x = 1;\n");
    assert_eq!(block.colour, theme.token_string);
    assert_eq!(block.style.family, FontFamily::Mono);
}

/// The fence itself is syntax, not the code it opens.
#[test]
fn a_fence_is_marker_coloured() {
    let theme = Theme::default();
    let runs = drawn("```\nlet x = 1;\n```\n");
    let fence = runs
        .iter()
        .find(|run| run.text.starts_with("```"))
        .expect("plain markup draws the fence");
    assert_eq!(fence.colour, theme.marker);
}

/// Colour is not concealment, and this story changed only the first — so a marker with a colour of
/// its own is still not drawn in preview.
#[test]
fn markers_still_vanish_in_preview() {
    let source = "# A heading\n\n- an item\n\nsee [the docs](https://example.com) here\n";
    let text: String = drawn_in(source, Mode::Preview)
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert_eq!(
        text, "A heading\n\n\u{2022} an item\n\nsee the docs here\n",
        "a marker that stayed visible because it got its own colour would be a different editor"
    );
}

/// Plain markup shows every byte, whatever colour it is in.
#[test]
fn plain_markup_still_shows_everything() {
    let source = "# A heading\n\n- an item\n\n[docs](https://example.com)\n";
    let text: String = drawn_in(source, Mode::Plain)
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert_eq!(text, source);
}

/// **A task-list checkbox is syntax, and it is not the bullet's colour** (MDEDIT-88).
///
/// The bar has written `- [ ] ` since MDEDIT-52 while the parser was told not to read task lists, so
/// the box came back as *content* — drawn as a literal pair of brackets after a bullet, in prose
/// colour, and concealed by nothing.
///
/// `token_literal` rather than the bullet's `token_key`, because the two say different things: a
/// bullet is scaffolding and a box is **state**. It is the pack's constant role, which is where
/// `true` and `false` go, and a checkbox is the same kind of fact.
#[test]
fn a_task_checkbox_is_syntax_and_not_the_bullets_colour() {
    let theme = Theme::default();
    let runs = drawn("- [x] done\n");

    let checkbox = run_of(&runs, "[x]");
    assert_eq!(checkbox.colour, theme.token_literal);

    let bullet = run_of(&runs, "- ");
    assert_eq!(bullet.colour, theme.token_key, "the bullet is unchanged");
    assert_ne!(
        checkbox.colour, bullet.colour,
        "state and scaffolding are told apart, which is the whole reason it has a marker of its own"
    );
}

/// And in preview it draws a box, in the same colour. It drew nothing from MDEDIT-88 until MDEDIT-142,
/// which made a task list indistinguishable from a bullet list — and a literal pair of brackets, the
/// defect MDEDIT-88 fixed, is still not what comes back.
#[test]
fn a_task_checkbox_draws_a_box_in_preview() {
    let runs = drawn_in("- [x] done\n", Mode::Preview);
    let text: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(text, "\u{E73A} done\n");
    assert_eq!(
        run_of(&runs, "\u{E73A} ").colour,
        Theme::default().token_literal,
        "a box is state, drawn or not"
    );
}

/// **The box is drawn from the icon font, a size down, and a bullet is not.** The pair in that font
/// is what keeps the text still when a box is ticked (`tests/icons.rs` measures that against the real
/// font). This is the half a headless run can check: the stub gives every glyph one width whatever
/// its family and size, so without it CI could not tell a box drawn in the wrong face or at the wrong
/// size from the right one.
#[test]
fn a_box_is_drawn_from_the_icon_font_and_a_bullet_is_not() {
    let theme = Theme::default();
    let runs = drawn_in("- [ ] todo\n- plain\n", Mode::Preview);

    let task = run_of(&runs, "\u{E739} ");
    assert_eq!(task.style.family, FontFamily::Icon);
    assert_eq!(task.style.size.get(), theme.body_size * 0.875);

    let bullet = run_of(&runs, "\u{2022} ");
    assert_eq!(
        bullet.style.family,
        FontFamily::Mono,
        "a bullet is a marker like any other"
    );
    assert_eq!(bullet.style.size.get(), theme.body_size);
}

/// **A footnote's label is a landmark, not scaffolding** (MDEDIT-90) — the same argument that sends
/// a link's destination to `link` and a list bullet to `key` rather than leaving every marker one
/// colour. `[^1]` is the thing a reader's eye goes to and the thing they follow.
#[test]
fn a_footnote_label_is_link_coloured() {
    let theme = Theme::default();
    let runs = drawn("text [^1].\n\n[^1]: the note\n");

    assert_eq!(run_of(&runs, "[^1]").colour, theme.link, "the reference");
    assert_eq!(run_of(&runs, "[^1]: ").colour, theme.link, "the definition");
    assert_eq!(
        run_of(&runs, "the note\n").colour,
        theme.text,
        "and the note itself is prose"
    );
}

/// A definition list's `: ` is ordinary syntax, and the pack has nothing louder to say about it.
#[test]
fn a_definition_lists_colon_is_marker_coloured() {
    let theme = Theme::default();
    let runs = drawn("term\n: meaning\n");
    assert_eq!(run_of(&runs, ": ").colour, theme.marker);
}

/// And in preview both vanish, like every other marker.
#[test]
fn the_new_markers_are_concealed_in_preview() {
    let text: String = drawn_in("text [^1].\n\n[^1]: the note\n", Mode::Preview)
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert_eq!(text, "text .\n\nthe note\n");
}
