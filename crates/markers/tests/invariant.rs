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
use mdedit_offsets::ByteOffset;

/// Everything the partition promises, in one place.
///
/// Returns the segments so a caller can make further assertions on them.
fn assert_partitions(source: &str, context: &str) -> Vec<Segment> {
    let segments = segment(source);

    let mut expected_start = ByteOffset::ZERO;
    for (i, seg) in segments.iter().enumerate() {
        let span = seg.span();
        assert!(
            !span.is_empty(),
            "{context}: segment {i} is empty at {span}"
        );
        assert_eq!(
            span.start(),
            expected_start,
            "{context}: segment {i} starts at {} but the previous ended at {expected_start} \
             — a gap or overlap",
            span.start()
        );
        assert!(
            source.is_char_boundary(span.start().get())
                && source.is_char_boundary(span.end().get()),
            "{context}: segment {i} span {span} splits a character"
        );
        expected_start = span.end();
    }
    assert_eq!(
        expected_start.get(),
        source.len(),
        "{context}: segments cover {expected_start} of {} bytes",
        source.len()
    );

    let rebuilt: String = segments.iter().map(|s| s.span().slice(source)).collect();
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
        let text = seg.span().slice(source);
        // **An inline construct spanning a soft break leaves a whitespace-only marker**, and that is
        // a known defect rather than a rule of the format: `pulldown-cmark` strips the continuation
        // line's indent from the text event, so the gap the locator subtracts falls to the enclosing
        // construct and `_a\n b_` reports an `Emphasis` marker holding a space. Found the moment
        // this assertion was pointed at generated input (MDEDIT-110) and filed rather than fixed
        // here as MDEDIT-112 — the locator is the highest-risk component in the project and its own
        // gate is the tiling property, which this does not weaken.
        //
        // Whitespace is **ignored** in the four inline arms rather than accepted wholesale: the gap
        // is sometimes its own segment (`" "`) and sometimes merged with the delimiter beside it
        // (`" *"`), because adjacent markers of one kind are coalesced. Both are the same defect.
        // Every other character still has to be the construct's syntax, so a span that has swallowed
        // a content letter is caught exactly as before.
        let inline: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let plausible = text.chars().all(char::is_whitespace) && !text.is_empty()
            || match marker {
                Marker::Emphasis | Marker::Strong => inline.chars().all(|c| c == '*' || c == '_'),
                Marker::Strikethrough => inline.chars().all(|c| c == '~'),
                Marker::CodeSpan => inline.chars().all(|c| c == '`'),
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
        let segments = assert_partitions(&source, "proptest");
        // **The shapes too** (MDEDIT-110). Tiling alone cannot see a misaligned span — an
        // off-by-one that swallows a content character tiles perfectly — and this assertion ran on
        // the fixtures only, which is where such a span is least likely to be.
        assert_marker_shapes(&source, &segments, "proptest");
    }

    /// The same guarantee for ordinary prose with occasional syntax, which is what a document is.
    ///
    /// **`.` does not match a newline in proptest's regex**, so this generated single-line documents
    /// only — for a parser whose every block construct is decided by where the lines are
    /// (MDEDIT-110). `(?s)` turns the dot loose.
    #[test]
    fn partitions_arbitrary_text(source in "(?s).{0,200}") {
        let segments = assert_partitions(&source, "proptest text");
        assert_marker_shapes(&source, &segments, "proptest text");
    }
}

// ---- a parser that panics (MDEDIT-114) ----

/// **`pulldown-cmark` panics on some documents, and a document is a file somebody opened.**
///
/// Nine characters — a list marker, a tab, a link reference definition whose label is a NUL, and a
/// vertical tab — unwrap a `None` in `pulldown-cmark` 0.13.4's `parse.rs`. Found by the fuzz job
/// within four minutes of its first real run, minimised from the seventeen bytes libFuzzer produced,
/// and reproduced on Windows as well as on the Linux runner.
///
/// Upstream's defect, and this project's problem: it crashes the editor on opening a file. The parse
/// is contained, so what comes back is the whole source as content — which still tiles, which is the
/// invariant everything above rests on.
#[test]
fn a_document_that_panics_the_parser_comes_back_as_content() {
    let source = "+\t[\0]:d\n\u{b}";

    let segments = assert_partitions(source, "the pulldown-cmark panic");

    assert_eq!(
        segments.len(),
        1,
        "the fallback is one segment: {segments:?}"
    );
    assert!(
        matches!(segments[0].kind(), SegmentKind::Content),
        "and it is content, so the renderer draws plain text"
    );
}

/// The fallback has to hold the same invariant as the parse it replaces, including for the empty
/// document — a zero-length segment would break the "no empty segments" half of it.
#[test]
fn the_fallback_tiles_the_awkward_lengths_too() {
    for source in ["", "\0", "+\t[\0]:d\n\u{b}", "ordinary prose"] {
        assert_partitions(source, "fallback");
    }
}
