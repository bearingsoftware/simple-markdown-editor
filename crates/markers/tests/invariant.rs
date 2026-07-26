//! The architecture gate.
//!
//! `docs/research/03-architecture.md` states the condition the decorate-the-source design rests on:
//! concatenating every segment reproduces the source byte for byte. If that cannot hold, the
//! architecture is wrong and the project needs to know before a renderer exists to throw away.
//!
//! Asserted here over all 652 CommonMark 0.31.2 spec examples, over a property-test generator, and
//! over the awkward inputs a spec suite does not contain.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_markers::{Marker, Segment, SegmentKind, segment};

/// Everything the partition promises, in one place.
///
/// Returns the segments so a caller can make further assertions on them.
fn assert_partitions(source: &str, context: &str) -> Vec<Segment> {
    let segments = segment(source);

    let mut expected_start = 0;
    for (i, seg) in segments.iter().enumerate() {
        let range = seg.range();
        assert!(
            range.start < range.end,
            "{context}: segment {i} is empty at {range:?}"
        );
        assert_eq!(
            range.start, expected_start,
            "{context}: segment {i} starts at {} but the previous ended at {expected_start} \
             — a gap or overlap",
            range.start
        );
        assert!(
            source.is_char_boundary(range.start) && source.is_char_boundary(range.end),
            "{context}: segment {i} range {range:?} splits a character"
        );
        expected_start = range.end;
    }
    assert_eq!(
        expected_start,
        source.len(),
        "{context}: segments cover {expected_start} of {} bytes",
        source.len()
    );

    let rebuilt: String = segments.iter().map(|s| &source[s.range()]).collect();
    assert_eq!(rebuilt, source, "{context}: source not reproduced");

    segments
}

/// Marker text has to *look like* the construct it claims to be.
///
/// The partition invariant alone cannot catch a misaligned span — an off-by-one that swallows a
/// content character still tiles perfectly. This is the assertion that notices.
fn assert_marker_shapes(source: &str, segments: &[Segment], context: &str) {
    for seg in segments {
        let SegmentKind::Marker(marker) = seg.kind() else {
            continue;
        };
        let text = &source[seg.range()];
        let plausible = match marker {
            Marker::Emphasis | Marker::Strong => text.chars().all(|c| c == '*' || c == '_'),
            Marker::Strikethrough => text.chars().all(|c| c == '~'),
            Marker::CodeSpan => text.chars().all(|c| c == '`'),
            Marker::ThematicBreak => text.chars().all(|c| "*-_ \t".contains(c)),
            Marker::Escape => text == "\\",
            Marker::HardBreak => text.chars().all(|c| c == ' ' || c == '\\'),
            // `### ###` is an empty heading and entirely syntax, and CommonMark strips a heading's
            // trailing whitespace — so a run may be all spaces. Constrain the characters, not the
            // shape; that still catches a span that has swallowed a content letter.
            Marker::Heading => text.chars().all(|c| "#=-".contains(c) || c.is_whitespace()),
            // Continuation lines carry only the indent that keeps content inside the block, so a
            // whitespace-only marker is legitimate for both of these.
            Marker::BlockQuote => text.contains('>') || text.trim().is_empty(),
            Marker::ListItem => {
                text.trim().is_empty()
                    || text.contains(|c: char| "*-+".contains(c) || c.is_ascii_digit())
            }
            // Link syntax spans `[`, `](dest "title")` and `)`; the destination is arbitrary.
            Marker::Link | Marker::Image => text.contains(|c: char| "[]()!<>".contains(c)),
            // A fence carries an arbitrary info string, an indented block's marker is whitespace,
            // and an unlisted kind is unconstrained by design.
            Marker::CodeBlock | _ => true,
        };
        assert!(
            plausible,
            "{context}: {marker:?} marker holds {text:?}, which is not that construct's syntax"
        );
    }
}

fn spec_examples() -> Vec<(usize, String, String)> {
    let raw = include_str!("fixtures/commonmark-0.31.2.json");
    let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
    parsed
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                usize::try_from(e["example"].as_u64().unwrap()).unwrap(),
                e["section"].as_str().unwrap().to_owned(),
                e["markdown"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn partitions_every_commonmark_spec_example() {
    let examples = spec_examples();
    assert_eq!(
        examples.len(),
        652,
        "spec fixture changed size unexpectedly"
    );

    for (number, section, markdown) in examples {
        let context = format!("spec example {number} ({section})");
        let segments = assert_partitions(&markdown, &context);
        assert_marker_shapes(&markdown, &segments, &context);
    }
}

#[test]
fn partitions_awkward_inputs() {
    let cases = [
        ("", "empty"),
        ("\n", "bare newline"),
        ("\r\n\r\n", "crlf only"),
        ("no markdown at all", "plain prose"),
        ("*", "unclosed emphasis"),
        ("**", "unclosed strong"),
        ("`", "unclosed code span"),
        ("[", "unclosed link"),
        ("[text](", "truncated link"),
        ("```", "unclosed fence"),
        ("> ", "empty quote"),
        ("- ", "empty list item"),
        ("#", "bare hash"),
        ("\\", "trailing backslash"),
        ("\\\\", "escaped backslash"),
        ("héllo *wörld* ünïcode", "multibyte around emphasis"),
        ("🎉 **emoji** 🎉", "astral plane"),
        ("a\u{200b}*b*", "zero-width space"),
        ("*a **b** c*", "strong nested in emphasis"),
        ("**a *b* c**", "emphasis nested in strong"),
        ("`` ` ``", "backtick inside code span"),
        ("<div>\nraw html\n</div>", "html block"),
        ("| a | b |\n| - | - |", "table syntax without the extension"),
        ("[ref][lbl]\n\n[lbl]: /url", "reference link and definition"),
        ("term\n: definition", "definition list syntax"),
        ("Line one  \nLine two", "hard break via trailing spaces"),
        ("Line one\\\nLine two", "hard break via backslash"),
        ("    indented code", "indented code block"),
        ("> - > nested\n", "quote list quote"),
    ];

    for (source, name) in cases {
        let segments = assert_partitions(source, name);
        assert_marker_shapes(source, &segments, name);
    }
}

#[test]
fn out_of_scope_constructs_degrade_to_content_not_panic() {
    // Not classified yet; the bytes must survive regardless.
    for source in ["<div>x</div>", "&amp; &#35;", "| a |\n| - |", "[^fn]: note"] {
        let segments = assert_partitions(source, source);
        assert!(!segments.is_empty());
    }
}

/// A delimiter-heavy alphabet. The spec corpus is well-formed by construction, so it never
/// exercises the half-open and interleaved syntax a real editor sees mid-keystroke.
const FRAGMENTS: &[&str] = &[
    "*", "_", "`", "~", "#", ">", "-", "+", "[", "]", "(", ")", "!", "\\", "\n", " ", "a", "é",
    "🎉", "1.", "```", "**", "~~", "  \n",
];

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(1024))]

    #[test]
    fn partitions_arbitrary_delimiter_soup(indices in proptest::collection::vec(0..FRAGMENTS.len(), 0..40)) {
        let source: String = indices.into_iter().map(|i| FRAGMENTS[i]).collect();
        assert_partitions(&source, "proptest");
    }

    /// The same guarantee for ordinary prose with occasional syntax, which is what a document is.
    #[test]
    fn partitions_arbitrary_text(source in ".{0,200}") {
        assert_partitions(&source, "proptest text");
    }
}
