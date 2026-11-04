//! The decoration step: which bytes get drawn, at what size, in what colour.
//!
//! No GPU and no platform text stack, so this runs everywhere and is where the renderer's
//! *behaviour* is actually pinned down.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::view::{Decoration, Mode, Theme, build_runs, build_runs_in, colour_at};
use mdedit_document::{Document, Selection};
use mdedit_offsets::{ByteOffset, ByteSpan};

fn drawn(source: &str) -> String {
    let doc = Document::from(source);
    let runs = build_runs(&doc, &Theme::default(), source);
    runs.iter().map(|run| run.text.as_ref()).collect()
}

/// The same, with the document's own caret parked somewhere. **The answer must not change** — that
/// is MDEDIT-118 in one function, and it is a helper rather than a single test so that the caret can
/// be swept across everything below.
fn drawn_with_caret(source: &str, caret: usize) -> String {
    let mut doc = Document::from(source);
    doc.set_selection(Selection::caret(ByteOffset::new(caret)));
    let runs = build_runs(&doc, &Theme::default(), source);
    runs.iter().map(|run| run.text.as_ref()).collect()
}

#[test]
fn concealed_markers_are_absent_from_the_drawn_text() {
    // Not blanked, not spaced — gone. That is what makes the result read as prose.
    assert_eq!(drawn("a **bold** b"), "a bold b");
    assert_eq!(drawn("a *em* b"), "a em b");
    assert_eq!(drawn("# Heading\n\nbody\n"), "Heading\n\nbody\n");
}

/// **The MDEDIT-118 assertion.** A marker under the caret used to be drawn — that was the whole
/// live-preview illusion, and a `**` appearing as you typed past it was the feature working.
///
/// Now nothing appears, from any caret position, and this sweeps every one of them rather than
/// picking the two that used to reveal. A `for` loop over the document is the only shape of this
/// test that cannot be satisfied by an off-by-one in the predicate it replaced.
#[test]
fn no_caret_position_reveals_anything() {
    for source in ["a **bold** b", "## Heading\n", "> quote\n", "`code`"] {
        let expected = drawn(source);
        for caret in 0..=source.len() {
            assert_eq!(
                drawn_with_caret(source, caret),
                expected,
                "{source:?} drew differently with the caret at {caret}"
            );
        }
    }
}

#[test]
fn the_buffer_is_never_modified_by_drawing() {
    let source = "# Heading\n\n**bold**\n";
    let doc = Document::from(source);
    let before = doc.to_string();
    let _ = build_runs(&doc, &Theme::default(), source);
    assert_eq!(
        doc.to_string(),
        before,
        "decoration must not touch the bytes"
    );
}

/// The bug the first screenshot showed.
///
/// `mdedit_markers` coalesces adjacent same-kind runs, so one content segment spans a heading's
/// text, the blank line after it, and the whole next paragraph. Styling per segment instead of per
/// line drew the entire document at heading size.
#[test]
fn heading_size_does_not_leak_into_the_next_paragraph() {
    let theme = Theme::default();
    let source = "# Heading\n\nOrdinary paragraph text.\n";
    let doc = Document::from(source);
    let runs = build_runs(&doc, &theme, source);

    let heading = runs
        .iter()
        .find(|r| r.text.starts_with("Heading"))
        .expect("the heading text is drawn");
    let body = runs
        .iter()
        .find(|r| r.text.starts_with("Ordinary"))
        .expect("the paragraph is drawn");

    assert_eq!(heading.style.size.get(), theme.heading_size(1));
    assert_eq!(
        body.style.size.get(),
        theme.body_size,
        "the paragraph after a heading must be body size, not {}",
        heading.style.size.get()
    );
}

#[test]
fn heading_level_sets_the_size() {
    let theme = Theme::default();
    for (source, level) in [("# a\n", 1), ("## a\n", 2), ("### a\n", 3)] {
        let doc = Document::from(source);
        let runs = build_runs(&doc, &theme, source);
        let text = runs.iter().find(|r| r.text.starts_with('a')).unwrap();
        assert_eq!(
            text.style.size.get(),
            theme.heading_size(level),
            "{source:?} should be an h{level}"
        );
    }
}

/// Something the old heuristic could not do at all.
///
/// Heading size used to be recovered by counting `#` on the marker, so a setext heading — whose
/// syntax is an underline on the *following* line — never got one. Reading the block gives it for
/// free, because the parser already knew.
#[test]
fn a_setext_heading_is_sized_like_its_atx_equivalent() {
    let theme = Theme::default();
    for (source, level) in [("Title\n=====\n", 1), ("Title\n-----\n", 2)] {
        let doc = Document::from(source);
        let runs = build_runs(&doc, &theme, source);
        let title = runs
            .iter()
            .find(|r| r.text.starts_with("Title"))
            .unwrap_or_else(|| panic!("{source:?} draws its title"));
        assert_eq!(
            title.style.size.get(),
            theme.heading_size(level),
            "{source:?} should be an h{level}"
        );
    }
}

/// Code inside a fence keeps its own inline characters literal.
#[test]
fn a_fence_is_monospaced_from_its_block_not_from_its_markers() {
    // No closing fence: the block runs to the end of the document. A toggle-based implementation
    // would have turned monospacing on and left it on; reading the block gets the same answer for
    // the right reason, and stops where the block stops.
    let source = "```\nunterminated\n";
    let doc = Document::from(source);
    let runs = build_runs(&doc, &Theme::default(), source);
    let body = runs
        .iter()
        .find(|r| r.text.contains("unterminated"))
        .expect("the body is drawn");
    assert_eq!(body.style.family, mdedit_text::FontFamily::Mono);
}

#[test]
fn emphasis_applies_to_the_text_between_its_markers() {
    let source = "plain **bold** plain\n";
    let doc = Document::from(source);
    let runs = build_runs(&doc, &Theme::default(), source);

    let bold = runs.iter().find(|r| r.text == "bold").expect("bold run");
    assert_eq!(bold.style.weight, mdedit_text::FontWeight::Bold);

    let plain = runs
        .iter()
        .find(|r| r.text.starts_with("plain"))
        .expect("plain run");
    assert_eq!(plain.style.weight, mdedit_text::FontWeight::Regular);
}

#[test]
fn code_spans_are_monospaced() {
    let source = "call `func()` now\n";
    let doc = Document::from(source);
    let runs = build_runs(&doc, &Theme::default(), source);
    let code = runs.iter().find(|r| r.text == "func()").expect("code run");
    assert_eq!(code.style.family, mdedit_text::FontFamily::Mono);
}

/// Unclosed emphasis must not run away with the rest of the document.
#[test]
fn an_unclosed_marker_stops_at_the_end_of_its_line() {
    let source = "**never closed\n\nplain paragraph\n";
    let doc = Document::from(source);
    let runs = build_runs(&doc, &Theme::default(), source);

    let plain = runs
        .iter()
        .find(|r| r.text.starts_with("plain"))
        .expect("the later paragraph is drawn");
    assert_eq!(
        plain.style.weight,
        mdedit_text::FontWeight::Regular,
        "an unterminated ** must not embolden the rest of the file"
    );
}

/// The second bug the screenshots caught.
///
/// Line-scoped state has to reset at a newline so unclosed emphasis cannot run away, but a fenced
/// block spans lines by definition. Resetting both together silently un-monospaced every fence.
#[test]
fn a_fenced_block_stays_monospaced_across_its_lines() {
    let source = "```rust\nfn one() {}\nfn two() {}\n```\n\nprose after\n";
    let doc = Document::from(source);
    let runs = build_runs(&doc, &Theme::default(), source);

    for needle in ["fn one() {}", "fn two() {}"] {
        let line = runs
            .iter()
            .find(|r| r.text.contains(needle))
            .unwrap_or_else(|| panic!("{needle} is drawn"));
        assert_eq!(
            line.style.family,
            mdedit_text::FontFamily::Mono,
            "{needle:?} should be monospaced inside the fence"
        );
    }

    let prose = runs
        .iter()
        .find(|r| r.text.contains("prose after"))
        .expect("prose after the fence is drawn");
    assert_eq!(
        prose.style.family,
        mdedit_text::FontFamily::Ui,
        "the closing fence must end monospacing"
    );
}

/// Markers are drawn in the marker colour **in plain markup**, which is the only mode that draws
/// them since MDEDIT-118. Preview draws the content and nothing else, which the second half says.
#[test]
fn drawn_markers_are_in_the_marker_colour() {
    let theme = Theme::default();
    let source = "**bold**";
    let doc = Document::from(source);

    let plain = build_runs_in(
        &doc,
        Decoration {
            theme: &theme,
            mode: Mode::Plain,
        },
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
        source,
    );
    let marker = plain.iter().find(|r| r.text == "**").expect("drawn `**`");
    assert_eq!(marker.colour, theme.marker);

    let runs = build_runs(&doc, &theme, source);
    let content = runs.iter().find(|r| r.text == "bold").expect("content");
    assert_eq!(content.colour, theme.text);
    assert!(
        runs.iter().all(|r| r.text != "**"),
        "preview drew a delimiter"
    );
}

#[test]
fn runs_are_in_source_order_and_do_not_overlap() {
    let doc = Document::from(mdedit_app::SAMPLE);
    let runs = build_runs(&doc, &Theme::default(), mdedit_app::SAMPLE);

    for pair in runs.windows(2) {
        assert!(
            pair[0].source.end() <= pair[1].source.start(),
            "runs {:?} and {:?} overlap or are out of order",
            pair[0].text,
            pair[1].text
        );
    }
}

#[test]
fn colour_lookup_finds_the_run_a_byte_belongs_to() {
    let theme = Theme::default();
    let source = "**bold**";
    let doc = Document::from(source);
    let runs = build_runs(&doc, &theme, source);

    // Byte 2 is the first letter of "bold".
    assert_eq!(colour_at(&runs, ByteOffset::new(2), theme.text), theme.text);
    // Byte 0 belongs to a delimiter that preview does not draw, so there is no run holding it and
    // the fallback is the answer. That is the same shape as the stale-caret case below and not a
    // special one: `colour_at` answers from the runs it was given.
    assert_eq!(colour_at(&runs, ByteOffset::ZERO, theme.text), theme.text);
    // Past the end falls back rather than panicking, because a stale caret must not crash a frame.
    assert_eq!(
        colour_at(&runs, ByteOffset::new(9_999), theme.text),
        theme.text
    );
}

#[test]
fn the_sample_document_draws_without_panicking_at_every_caret() {
    let doc = Document::from(mdedit_app::SAMPLE);
    let theme = Theme::default();
    for caret in 0..=doc.len_bytes() {
        if !mdedit_app::SAMPLE.is_char_boundary(caret) {
            continue;
        }
        let runs = build_runs(&doc, &theme, mdedit_app::SAMPLE);
        assert!(!runs.is_empty(), "caret {caret} produced nothing to draw");
    }
}
