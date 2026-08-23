//! Struck-through text is drawn struck (MDEDIT-91).
//!
//! Strikethrough is the one extension the parser has had on since day one, and the bar has had a
//! button for it since MDEDIT-52 — but the text came out looking like every other word. The markers
//! concealed and nothing else happened.
//!
//! **Two halves, and only one of them can be tested here.** Whether the run *asks* for a rule is the
//! decoration step's answer and is asserted below. Where the rule sits and how thick it is are the
//! font's, so they are in `crates/text-dwrite/tests/layout.rs` against the real stack — the headless
//! stub has no font and reports no rules at all, which is exactly the split MDEDIT-73 established.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::view::{Decoration, Mode, Theme, build_runs_in};
use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};

/// Whether every run holding `needle` asked for a rule through it, and that there was one.
fn struck(source: &str, needle: &str, mode: Mode) -> bool {
    let doc = Document::from(source);
    let runs = build_runs_in(
        &doc,
        Decoration {
            theme: &Theme::default(),
            mode,
        },
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
        source,
    );

    let matching: Vec<_> = runs
        .iter()
        .filter(|run| run.text.contains(needle))
        .collect();
    assert!(
        !matching.is_empty(),
        "no run holds {needle:?}; the runs are {:?}",
        runs.iter().map(|run| run.text.as_ref()).collect::<Vec<_>>()
    );
    matching.iter().all(|run| run.style.strikethrough)
}

#[test]
fn text_between_two_tildes_is_struck() {
    assert!(struck("~~gone~~", "gone", Mode::Preview));
}

#[test]
fn the_prose_either_side_is_not() {
    let source = "before ~~gone~~ after\n";
    assert!(struck(source, "gone", Mode::Preview));
    assert!(!struck(source, "before ", Mode::Preview));
    assert!(!struck(source, " after\n", Mode::Preview));
}

/// It pairs within a line, like the emphasis delimiters beside it and unlike a quote. An unclosed
/// `~~` must not strike the rest of the document.
#[test]
fn an_unclosed_run_stops_at_the_end_of_its_line() {
    let source = "~~never closed\n\nplain paragraph\n";
    assert!(!struck(source, "plain paragraph", Mode::Preview));
}

/// Both at once, which is why `TextStyle` carries two flags rather than one enum.
#[test]
fn a_struck_link_is_struck_and_its_destination_still_underlined() {
    let source = "~~[a](/url)~~";
    let doc = Document::from(source);
    let runs = build_runs_in(
        &doc,
        Decoration {
            theme: &Theme::default(),
            mode: Mode::Plain,
        },
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
        source,
    );

    let destination = runs
        .iter()
        .find(|run| run.text == "/url")
        .expect("plain markup draws the destination");
    assert!(
        destination.style.underline,
        "the pack's rule is that `link` is never colour alone, struck or not"
    );
}

/// **Nothing but content.** A run's own `~~` is concealed in preview, so there is nothing to strike;
/// in plain markup the markers are the point, and a line through them would hide what is being read.
#[test]
fn the_markers_themselves_are_never_struck() {
    let source = "~~gone~~";
    let doc = Document::from(source);
    let runs = build_runs_in(
        &doc,
        Decoration {
            theme: &Theme::default(),
            mode: Mode::Plain,
        },
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
        source,
    );

    for run in runs.iter().filter(|run| run.text.contains('~')) {
        assert!(!run.style.strikethrough, "{:?} was struck", run.text);
    }
}

/// Plain markup is what you switch to in order to read the bytes, and it flattens every preview
/// decoration — the size, the weight, the slant, and this.
#[test]
fn plain_markup_strikes_nothing() {
    assert!(!struck("~~gone~~", "gone", Mode::Plain));
}
