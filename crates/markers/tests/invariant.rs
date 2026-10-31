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
    // Set by the first Link or Image marker that holds a bracket. See that arm below: a link's
    // destination is arbitrary text, so a *continuation* of one can hold anything, and the only
    // thing worth asserting about it is that a link was opened at all.
    let mut link_opened = false;

    for seg in segments {
        let SegmentKind::Marker(marker) = seg.kind() else {
            continue;
        };
        let text = seg.span().slice(source);
        // **The tolerance that used to be here is gone**, and MDEDIT-112 is why it could go.
        //
        // An inline construct spanning a soft break used to leave a whitespace-only marker:
        // `pulldown-cmark` strips the continuation line's indent from the text event, so the gap the
        // locator subtracts fell to the enclosing construct and `_a\n b_` reported an `Emphasis`
        // marker holding a space. This assertion tolerated it — whitespace ignored in the four
        // delimiter arms, every other character still constrained — because the defect was found the
        // moment the assertion was first pointed at generated input (MDEDIT-110) and the locator is
        // the highest-risk component in the project.
        //
        // `normalise_line_syntax` now gives that whitespace back to content, so a delimiter run is
        // **only** its delimiters and this asks for exactly that.
        let plausible = match marker {
            Marker::Emphasis | Marker::Strong => text.chars().all(|c| c == '*' || c == '_'),
            Marker::Strikethrough => text.chars().all(|c| c == '~'),
            Marker::CodeSpan => text.chars().all(|c| c == '`'),
            Marker::ThematicBreak => text.chars().all(|c| "*-_ \t".contains(c)),
            Marker::Escape => text == "\\",
            // Two spaces or a backslash is the *written* form, but the reported range covers
            // whatever trailing whitespace preceded the newline — a tab, a vertical tab. Only
            // visible once MDEDIT-112 removed the blanket whitespace tolerance above; it is the
            // format being loose rather than the locator being wrong, and a swallowed content
            // character is still caught.
            Marker::HardBreak => text.chars().all(|c| c.is_whitespace() || c == '\\'),
            // `### ###` is an empty heading and entirely syntax, and CommonMark strips a heading's
            // trailing whitespace — so a run may be all spaces. Constrain the characters, not the
            // shape; that still catches a span that has swallowed a content letter.
            //
            Marker::Heading => text.chars().all(|c| "#=-".contains(c) || c.is_whitespace()),
            // `[^id]` as a reference, or `[^id]: ` opening a definition. The identifier is arbitrary,
            // so the brackets are what there is to check.
            // Or the indent of a continuation line, which is the definition's syntax in the same way a
            // list item's indent is: it is what keeps the next line inside the note.
            Marker::Footnote => {
                text.trim().is_empty() || (text.starts_with("[^") && text.contains(']'))
            }
            // `: ` and the whitespace around it.
            Marker::DefinitionList => text.chars().all(|c| c == ':' || c.is_whitespace()),
            // Continuation lines carry only the indent that keeps content inside the block, so a
            // whitespace-only marker is legitimate for both of these.
            Marker::BlockQuote => text.contains('>') || text.trim().is_empty(),
            Marker::ListItem => {
                text.trim().is_empty()
                    || text.contains(|c: char| "*-+".contains(c) || c.is_ascii_digit())
            }
            // Link syntax spans `[`, `](dest "title")` and `)`.
            //
            // **A destination and a title are arbitrary text**, so a soft break inside the
            // parentheses leaves a middle segment that can hold literally anything:
            // `"[](  \n**\\  \n)"` reports three Link markers and the middle one is `**\  `. There
            // is nothing to check per-segment there that is not simply false — which the property
            // test demonstrated twice at 60,000 cases while MDEDIT-125 was being verified, first
            // against "must contain a bracket" and then against "whitespace and backslashes".
            //
            // What is still true, and is what this now says: **a link marker cannot appear before a
            // link has been opened.** A stray character labelled `Link` out of nowhere — the failure
            // this arm exists to catch — has no bracketed marker before it and still fails.
            Marker::Link | Marker::Image => {
                let opens = text.contains(|c: char| "[]()!<>".contains(c));
                link_opened |= opens;
                opens || link_opened
            }
            // Exactly three characters, and only two spellings of the middle one. MDEDIT-88 turned
            // the extension on, so this is a shape worth stating rather than leaving to the
            // catch-all below.
            Marker::TaskMarker => matches!(text, "[ ]" | "[x]" | "[X]"),
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

/// **A task-list checkbox is syntax** (MDEDIT-88), and the bullet before it is still the bullet's.
///
/// The editor's own formatting bar has written `- [ ] ` since MDEDIT-52 while `ENABLE_TASKLISTS` was
/// off here, so the write path and the read path disagreed about the language: the box came back as
/// *content* and was drawn as a literal pair of brackets after a bullet.
///
/// Three spans per item, and the shape is the assertion — a single `ListItem` marker swallowing the
/// box would tile perfectly and be exactly the old behaviour.
#[test]
fn a_task_list_checkbox_is_its_own_marker() {
    let source = "- [ ] todo\n- [x] done\n";
    let segments = assert_partitions(source, "MDEDIT-88");

    let boxes: Vec<_> = segments
        .iter()
        .filter(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::TaskMarker)))
        .map(|seg| seg.span().slice(source))
        .collect();
    assert_eq!(
        boxes,
        vec!["[ ]", "[x]"],
        "both boxes, unchecked and checked"
    );

    let content: Vec<_> = segments
        .iter()
        .filter(|seg| seg.kind() == SegmentKind::Content)
        .map(|seg| seg.span().slice(source))
        .collect();
    assert_eq!(
        content,
        vec!["todo\n", "done\n"],
        "the text of each item, and nothing of its syntax"
    );
}

/// A bullet without a box is untouched, which is what says the extension did not change lists.
#[test]
fn an_ordinary_list_item_gains_no_checkbox() {
    let source = "- plain\n";
    let segments = assert_partitions(source, "MDEDIT-88");

    assert!(
        !segments
            .iter()
            .any(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::TaskMarker))),
        "there is no box here to find"
    );
}

/// **Brackets that are not a checkbox stay content**, which is the risk of turning an extension on:
/// `[ ]` is only a task marker at the start of a list item.
#[test]
fn brackets_elsewhere_are_not_a_checkbox() {
    for source in ["a [ ] b\n", "- a [ ] b\n", "> [ ] quoted\n"] {
        let segments = assert_partitions(source, source);
        assert!(
            !segments
                .iter()
                .any(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::TaskMarker))),
            "{source:?} has no task item in it"
        );
    }
}

/// A delimiter-heavy alphabet. The spec corpus is well-formed by construction, so it never
/// exercises the half-open and interleaved syntax a real editor sees mid-keystroke.
const FRAGMENTS: &[&str] = &[
    "*", "_", "`", "~", "#", ">", "-", "+", "[", "]", "(", ")", "!", "\\", "\n", " ", "a", "é",
    "🎉", "1.", "```", "**", "~~", "  \n",
    // MDEDIT-90, so the generator can reach the three extensions turned on there. Without these the
    // property test runs over an alphabet that cannot spell a footnote, and the flags would be
    // covered by fixtures alone — which is where a misaligned span is least likely to be.
    "[^", "]:", "{#", "}", ":", "^",
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

/// The input that panicked `pulldown-cmark` 0.13.4 (upstream #1129), fixed in 0.13.5 by the
/// RUSTSEC-2026-0414 bump. It stays as a regression input and must still tile; the containment that
/// answers a panic is tested in `lib.rs` through `contain`.
#[test]
fn the_document_that_panicked_0_13_4_now_tiles() {
    let source = "+\t[\0]:d\n\u{b}";

    let segments = assert_partitions(source, "the pulldown-cmark panic");
    assert_marker_shapes(source, &segments, "the pulldown-cmark panic");
}

/// The awkward lengths must tile through the real parse, including the empty document — a
/// zero-length segment would break the "no empty segments" half of the invariant.
#[test]
fn the_awkward_lengths_tile() {
    for source in ["", "\0", "+\t[\0]:d\n\u{b}", "ordinary prose"] {
        assert_partitions(source, "fallback");
    }
}

/// **MDEDIT-112's own repro.** An inline construct spanning a soft break used to hand the
/// continuation line's indent to the enclosing construct as its marker.
///
/// The ticket's input is underscore, star, newline, space, backtick, underscore, tilde — and the
/// space at offset 3 came back as [`Marker::Emphasis`]. It never mis-*covered*, so the tiling gate
/// held and nothing caught it; but a marker is concealed in preview, so that space vanished from the
/// rendered line.
#[test]
fn a_soft_breaks_indent_is_content_not_emphasis() {
    let source = "_*\n `_~";
    let segments = assert_partitions(source, "MDEDIT-112");

    // By offset rather than by looking for a `" "` segment: now that the space is content it
    // coalesces with the content either side of it, which is the outcome rather than a complication.
    let indent = ByteOffset::new(3);
    let owner = segments
        .iter()
        .find(|seg| seg.span().start() <= indent && indent < seg.span().end())
        .expect("every byte belongs to a segment");
    assert!(
        matches!(owner.kind(), SegmentKind::Content),
        "the continuation indent is content, not syntax: {:?} holding {:?}",
        owner.kind(),
        owner.span().slice(source)
    );
}

/// The other side of the same break, which the first fix missed and the property test found in
/// seconds: `pulldown-cmark` strips the whitespace *before* a soft break too.
#[test]
fn a_soft_breaks_trailing_whitespace_is_content_too() {
    let source = "AA*x\t\n y*";
    let segments = assert_partitions(source, "MDEDIT-112 trailing");

    for seg in &segments {
        if let SegmentKind::Marker(Marker::Emphasis) = seg.kind() {
            let text = seg.span().slice(source);
            assert!(
                text.chars().all(|c| c == '*' || c == '_'),
                "an emphasis marker holds {text:?}, which is not a delimiter run"
            );
        }
    }
}

/// **A continuation line's indent is still that line's prefix**, and clearing the whitespace must not
/// say otherwise.
///
/// MDEDIT-125. The first version of MDEDIT-112's rule set `content_started` as it cleared the space,
/// which stopped the `>` immediately after it from being recognised as the quote's own syntax — so
/// this input came back with a `Marker::Strikethrough` holding `>`. CommonMark allows three spaces of
/// indent before a block marker, so a space is exactly the thing that does *not* end a prefix.
///
/// Found by the property test on CI rather than here, and the seed is in
/// `invariant.proptest-regressions` so it is re-run every time regardless of the case count.
#[test]
fn an_indented_quote_marker_after_a_break_is_still_the_quotes() {
    let source = ">~~*\n >**~~#";
    let segments = assert_partitions(source, "MDEDIT-125");

    let quotes: Vec<_> = segments
        .iter()
        .filter(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::BlockQuote)))
        .map(|seg| seg.span().slice(source))
        .collect();
    assert_eq!(
        quotes,
        vec![">", ">"],
        "both `>` belong to the quote; the second is the one that used to be strikethrough"
    );
}

/// A **link's** marker legitimately spans a newline and the indent after it — the whitespace sits
/// inside the parentheses, between the destination and the title. The fix must not reach it, and
/// CommonMark spec example 510 is what said so when an earlier attempt did.
#[test]
fn a_links_marker_may_still_span_a_break() {
    let source = "[a](/url\n  \"title\")\n";
    let segments = assert_partitions(source, "MDEDIT-112 links");

    let concealed: String = segments
        .iter()
        .filter(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::Link)))
        .map(|seg| seg.span().slice(source))
        .collect();
    assert!(
        concealed.contains("\"title\""),
        "the title is part of the link's syntax and must stay concealed: {concealed:?}"
    );
}

/// **A footnote's `[^1]` is syntax at both ends** (MDEDIT-90) — the reference in the sentence and the
/// label opening the definition.
///
/// The definition's marker is its **label only**. The tag's range covers the note's body as well,
/// and the parser reports no content event for a body that is itself a link reference definition, so
/// subtracting content left the whole rest of the document concealed. `label_end` measures it
/// directly instead, as `Event::Code`'s backtick run is measured and for the same reason.
#[test]
fn a_footnote_is_syntax_at_both_ends() {
    let source = "text [^1].\n\n[^1]: the note\n";
    let segments = assert_partitions(source, "MDEDIT-90 footnotes");

    let markers: Vec<_> = segments
        .iter()
        .filter(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::Footnote)))
        .map(|seg| seg.span().slice(source))
        .collect();
    assert_eq!(markers, vec!["[^1]", "[^1]: "]);

    let content: Vec<_> = segments
        .iter()
        .filter(|seg| seg.kind() == SegmentKind::Content)
        .map(|seg| seg.span().slice(source))
        .collect();
    assert_eq!(
        content,
        vec!["text ", ".\n\n", "the note\n"],
        "the note's text is the reader's, not the label's"
    );
}

/// The case the label measurement exists for: a body the parser emits nothing at all for.
#[test]
fn a_footnote_whose_body_is_a_reference_definition_does_not_swallow_it() {
    let source = "[^a]: [ref]: /url\n";
    let segments = assert_partitions(source, "MDEDIT-90 footnote body");

    let concealed: String = segments
        .iter()
        .filter(|seg| seg.is_marker())
        .map(|seg| seg.span().slice(source))
        .collect();
    assert_eq!(
        concealed, "[^a]: ",
        "everything past the label is the note, however little the parser says about it"
    );
}

/// **A definition list's `: ` is syntax** (MDEDIT-90), and the term above it is not.
#[test]
fn a_definition_lists_colon_is_syntax() {
    let source = "term\n: meaning\n";
    let segments = assert_partitions(source, "MDEDIT-90 definition lists");

    let markers: Vec<_> = segments
        .iter()
        .filter(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::DefinitionList)))
        .map(|seg| seg.span().slice(source))
        .collect();
    assert_eq!(markers, vec![": "]);
}

/// A colon that is not opening a definition stays content, which is the risk of every extension:
/// what it claims that it should not.
#[test]
fn a_colon_elsewhere_is_not_a_definition() {
    for source in ["a: b\n", "see: this\n", "- a: b\n"] {
        let segments = assert_partitions(source, source);
        assert!(
            !segments
                .iter()
                .any(|seg| matches!(seg.kind(), SegmentKind::Marker(Marker::DefinitionList))),
            "{source:?} is not a definition list"
        );
    }
}

/// **A document using none of them is classified exactly as before**, which is the other half of
/// turning extensions on: the flags must not change what ordinary Markdown means.
#[test]
fn ordinary_markdown_is_unchanged_by_the_extensions() {
    let source = "# Heading\n\nProse with **strong**, *emphasis*, `code`, [a link](/url) and \
                  ~~struck~~ text.\n\n- a bullet\n- another\n\n> a quote\n\n```rust\nfn x() {}\n```\n";
    let segments = assert_partitions(source, "MDEDIT-90 control");

    assert!(
        !segments.iter().any(|seg| matches!(
            seg.kind(),
            SegmentKind::Marker(Marker::Footnote | Marker::DefinitionList | Marker::TaskMarker)
        )),
        "no extension construct is in this document, so none should be reported"
    );
}
