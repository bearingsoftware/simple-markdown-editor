//! Concealment, and the caret's way across what it conceals (MDEDIT-118).
//!
//! This file used to test reveal-on-caret — markers hidden until the selection came near, then shown
//! dimmed so they could be edited in place, which the predicate's own doc comment called "the whole
//! live-preview illusion". **Nothing is revealed now.** A heading is large text with no `#` in front
//! of it, ever, and what says which level it is is the caret's own height.
//!
//! So the interesting assertions moved. There is almost nothing left to say about *whether* a marker
//! shows; what needs saying is that the caret can still get past one it cannot see, which is what
//! [`Document::concealed_at`] is for and what most of this file is about.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
use mdedit_offsets::ByteOffset;
use proptest::prelude::*;

/// Every marker in `source` that would be drawn, in document order. Expected to be empty.
fn revealed(source: &str) -> Vec<String> {
    let doc = Document::from(source);
    let text = doc.to_string();
    doc.segments()
        .iter()
        .filter(|s| s.is_marker() && doc.is_revealed(s))
        .map(|s| s.span().slice(&text).to_owned())
        .collect()
}

/// The concealed span at `offset`, as the text it covers.
fn concealed_at(source: &str, offset: usize) -> Option<String> {
    let doc = Document::from(source);
    let text = doc.to_string();
    doc.concealed_at(ByteOffset::new(offset))
        .map(|span| span.slice(&text).to_owned())
}

// ---- nothing is revealed ----

/// The whole of the change, in one assertion per construct that has a marker.
#[test]
fn no_marker_is_ever_drawn() {
    for source in [
        "**bold**",
        "*em*",
        "`code`",
        "## Heading\n",
        "> quote\n",
        "- item\n",
        "[label](/url)",
        "```rust\nfn x() {}\n```\n",
        "~~struck~~",
    ] {
        assert_eq!(
            revealed(source),
            Vec::<String>::new(),
            "{source:?} drew syntax"
        );
    }
}

/// **The case that changed.** Every one of these carets used to reveal something: touching an inline
/// delimiter, sitting anywhere on a block marker's line, sitting anywhere inside a fence.
///
/// The predicate no longer takes a selection at all, so this is asserted the only way left — by
/// moving the document's own caret and finding nothing drawn either way.
#[test]
fn the_caret_being_on_the_marker_changes_nothing() {
    let source = "## Heading\n\n**bold** and ```\nfence\n```\n";
    let mut doc = Document::from(source);

    for caret in 0..=source.len() {
        doc.set_selection(mdedit_document::Selection::caret(ByteOffset::new(caret)));
        let drawn: Vec<_> = doc
            .segments()
            .iter()
            .filter(|s| s.is_marker() && doc.is_revealed(s))
            .collect();
        assert!(drawn.is_empty(), "caret at {caret} revealed {drawn:?}");
    }
}

#[test]
fn content_is_always_visible() {
    let doc = Document::from("## Heading\n\n**bold** text\n");
    for segment in doc.segments() {
        if !segment.is_marker() {
            assert!(doc.is_revealed(segment), "content hidden");
        }
    }
}

/// A language that does not conceal is untouched by any of this — its syntax *is* its document, and
/// its caret has nothing invisible to step over.
#[test]
fn a_language_that_does_not_conceal_still_shows_everything() {
    let mut doc = Document::in_language(&mdedit_json::Json);
    doc.insert(ByteOffset::ZERO, "{\"a\": 1}").unwrap();

    assert!(
        doc.segments().iter().all(|s| doc.is_revealed(s)),
        "JSON hid something"
    );
    for caret in 0..=doc.len_bytes() {
        assert_eq!(
            doc.concealed_at(ByteOffset::new(caret)),
            None,
            "JSON has nothing for the caret to step over, but claimed something at {caret}"
        );
    }
}

// ---- what the caret has to step over ----

/// Half-open, and the half matters: the position *after* a marker is the one the caret rests at, and
/// the one Backspace acts from. If it were inside, Backspace could never reach the text.
#[test]
fn a_concealed_span_covers_its_bytes_and_not_the_one_after() {
    //  #  ␣  T  i  t  l  e
    //  0  1  2  3  4  5  6  7
    let source = "# Title";

    assert_eq!(concealed_at(source, 0).as_deref(), Some("# "));
    assert_eq!(concealed_at(source, 1).as_deref(), Some("# "));
    assert_eq!(
        concealed_at(source, 2),
        None,
        "the first letter is not in the marker"
    );
    assert_eq!(concealed_at(source, 6), None);
}

#[test]
fn both_delimiters_of_an_inline_run_are_concealed() {
    //  *  *  b  o  l  d  *  *
    //  0  1  2  3  4  5  6  7  8
    let source = "**bold**";

    assert_eq!(concealed_at(source, 0).as_deref(), Some("**"));
    assert_eq!(concealed_at(source, 1).as_deref(), Some("**"));
    assert_eq!(concealed_at(source, 3), None, "content");
    assert_eq!(concealed_at(source, 6).as_deref(), Some("**"));
    assert_eq!(concealed_at(source, 7).as_deref(), Some("**"));
    assert_eq!(
        concealed_at(source, 8),
        None,
        "past the end of the last delimiter"
    );
}

/// Both fences of a block, which are two separate markers with content between them — the caret
/// crosses each on its own.
#[test]
fn each_fence_is_its_own_crossing() {
    //  ```rust \n  f  n  \n  ```  \n
    //  0......7 7  8..     11...13
    let source = "```rust\nfn\n```\n";

    assert_eq!(concealed_at(source, 0).as_deref(), Some("```rust"));
    assert_eq!(concealed_at(source, 9), None, "inside the code");
    assert_eq!(concealed_at(source, 11).as_deref(), Some("```"));
}

/// A stale caret from a longer document must clamp rather than crash a render or an arrow key.
#[test]
fn a_caret_past_the_end_is_not_in_anything() {
    let doc = Document::from("## Heading\n");
    assert_eq!(doc.concealed_at(ByteOffset::new(9_999)), None);
}

/// An empty document has no segments at all, which is the case a `last()` on a binary-searched
/// prefix gets wrong if it is written as an index.
#[test]
fn an_empty_document_has_nothing_concealed() {
    let doc = Document::from("");
    assert_eq!(doc.concealed_at(ByteOffset::ZERO), None);
}

const FRAGMENTS: &[&str] = &[
    "a", "word ", "\n", "\n\n", "**bold**", "*em*", "`code`", "## head", "> quote", "- item",
    "[a](b)", "汉", "🎉", "\\*",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Total at every caret position in the document, including the very end.
    #[test]
    fn concealed_at_is_total_over_every_caret_position(
        parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..12),
    ) {
        let source: String = parts.into_iter().map(|i| FRAGMENTS[i]).collect();
        let doc = Document::from(source.as_str());

        for caret in 0..=source.len() {
            let _ = doc.concealed_at(ByteOffset::new(caret));
        }
    }

    /// **Every byte of every marker is concealed, and no byte of any content is.**
    ///
    /// The two halves together are what the caret's crossing rests on: skipping a span that is not a
    /// marker would step over text, and failing to skip one would leave the caret in a position that
    /// draws nowhere.
    #[test]
    fn concealed_at_answers_exactly_the_marker_bytes(
        parts in proptest::collection::vec(0..FRAGMENTS.len(), 1..12),
    ) {
        let source: String = parts.into_iter().map(|i| FRAGMENTS[i]).collect();
        let doc = Document::from(source.as_str());

        for segment in doc.segments() {
            let span = segment.span();
            for caret in span.range() {
                let found = doc.concealed_at(ByteOffset::new(caret));
                if segment.is_marker() {
                    prop_assert_eq!(
                        found,
                        Some(span),
                        "marker {:?} not concealed at {}",
                        span.slice(&source),
                        caret
                    );
                } else {
                    prop_assert_eq!(
                        found,
                        None,
                        "content {:?} concealed at {}",
                        span.slice(&source),
                        caret
                    );
                }
            }
        }
    }
}
