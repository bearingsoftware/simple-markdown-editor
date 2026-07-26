//! Per-construct marker correctness.
//!
//! The partition invariant in `invariant.rs` cannot catch a misaligned span — one that swallows a
//! content character still tiles perfectly. These assert the markers are the *right* bytes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_markers::{Marker, SegmentKind, segment};

/// Markers only, as `(text, kind)` — the shape most assertions here care about.
fn markers(source: &str) -> Vec<(&str, Marker)> {
    segment(source)
        .into_iter()
        .filter_map(|s| match s.kind() {
            SegmentKind::Marker(m) => Some((&source[s.range()], m)),
            SegmentKind::Content => None,
        })
        .collect()
}

/// Content only, concatenated — what the reader sees once syntax is concealed.
fn visible(source: &str) -> String {
    segment(source)
        .into_iter()
        .filter(|s| !s.is_marker())
        .map(|s| &source[s.range()])
        .collect()
}

struct Case {
    name: &'static str,
    source: &'static str,
    expect: &'static [(&'static str, Marker)],
}

#[test]
fn locates_inline_markers() {
    let cases = &[
        Case {
            name: "emphasis with asterisk",
            source: "*em*",
            expect: &[("*", Marker::Emphasis), ("*", Marker::Emphasis)],
        },
        Case {
            name: "emphasis with underscore",
            source: "_em_",
            expect: &[("_", Marker::Emphasis), ("_", Marker::Emphasis)],
        },
        Case {
            name: "strong",
            source: "**bold**",
            expect: &[("**", Marker::Strong), ("**", Marker::Strong)],
        },
        Case {
            name: "strikethrough",
            source: "~~gone~~",
            expect: &[("~~", Marker::Strikethrough), ("~~", Marker::Strikethrough)],
        },
        Case {
            name: "code span",
            source: "`code`",
            expect: &[("`", Marker::CodeSpan), ("`", Marker::CodeSpan)],
        },
        Case {
            name: "code span with doubled backticks",
            source: "``a ` b``",
            expect: &[("``", Marker::CodeSpan), ("``", Marker::CodeSpan)],
        },
        Case {
            name: "emphasis nested in strong keeps both kinds distinct",
            source: "**a *b* c**",
            expect: &[
                ("**", Marker::Strong),
                ("*", Marker::Emphasis),
                ("*", Marker::Emphasis),
                ("**", Marker::Strong),
            ],
        },
        Case {
            name: "inline link conceals the destination but not the text",
            source: "[text](http://e.com)",
            expect: &[("[", Marker::Link), ("](http://e.com)", Marker::Link)],
        },
        Case {
            name: "link title is concealed without needing a reported offset",
            source: "[t](/u \"title\")",
            expect: &[("[", Marker::Link), ("](/u \"title\")", Marker::Link)],
        },
        Case {
            name: "image keeps the bang with the opening bracket",
            source: "![alt](i.png)",
            expect: &[("![", Marker::Image), ("](i.png)", Marker::Image)],
        },
        Case {
            // Without its definition the label does not resolve and this is not a link at all, so
            // the definition has to be present for the construct to exist.
            name: "reference link",
            source: "[ref][lbl]\n\n[lbl]: /url\n",
            expect: &[("[", Marker::Link), ("][lbl]", Marker::Link)],
        },
        Case {
            name: "autolink",
            source: "<http://a.b>",
            expect: &[("<", Marker::Link), (">", Marker::Link)],
        },
        Case {
            name: "backslash escape",
            source: "a\\*b",
            expect: &[("\\", Marker::Escape)],
        },
        Case {
            name: "escape opening a line is still an escape",
            source: "\\> not a quote",
            expect: &[("\\", Marker::Escape)],
        },
    ];

    for case in cases {
        assert_eq!(markers(case.source), case.expect, "{}", case.name);
    }
}

#[test]
fn locates_block_markers() {
    let cases = &[
        Case {
            name: "atx heading",
            source: "## Heading\n",
            expect: &[("## ", Marker::Heading)],
        },
        Case {
            name: "atx heading with closing sequence",
            source: "## Heading ##\n",
            expect: &[("## ", Marker::Heading), (" ##", Marker::Heading)],
        },
        Case {
            name: "setext heading underline",
            source: "Heading\n=======\n",
            expect: &[("=======", Marker::Heading)],
        },
        Case {
            name: "block quote marks every line",
            source: "> one\n> two\n",
            expect: &[("> ", Marker::BlockQuote), ("> ", Marker::BlockQuote)],
        },
        Case {
            name: "nested block quote",
            source: "> > deep\n",
            expect: &[("> > ", Marker::BlockQuote)],
        },
        Case {
            name: "bullet list marks every item",
            source: "- one\n- two\n",
            expect: &[("- ", Marker::ListItem), ("- ", Marker::ListItem)],
        },
        Case {
            name: "ordered list",
            source: "1. one\n2. two\n",
            expect: &[("1. ", Marker::ListItem), ("2. ", Marker::ListItem)],
        },
        Case {
            name: "fenced code keeps the info string with the fence",
            source: "```rust\nx\n```\n",
            expect: &[("```rust", Marker::CodeBlock), ("```", Marker::CodeBlock)],
        },
        Case {
            name: "thematic break",
            source: "***\n",
            expect: &[("***", Marker::ThematicBreak)],
        },
        Case {
            name: "hard break from trailing spaces",
            source: "one  \ntwo",
            expect: &[("  ", Marker::HardBreak)],
        },
    ];

    for case in cases {
        assert_eq!(markers(case.source), case.expect, "{}", case.name);
    }
}

#[test]
fn quote_prefix_wins_over_an_enclosing_list_item() {
    // The item's range spans the quote's continuation lines, so range containment alone would
    // attribute `>>` to the list. Regression guard for CommonMark spec example 259.
    let source = "   > > 1.  one\n>>\n>>     two\n";
    for (text, marker) in markers(source) {
        if text.contains('>') {
            assert_eq!(
                marker,
                Marker::BlockQuote,
                "{text:?} should be quote syntax"
            );
        }
    }
}

#[test]
fn unparsed_constructs_stay_visible() {
    // pulldown-cmark emits no events for a link reference definition. It must not be swallowed as
    // list syntax and concealed. Regression guard for CommonMark spec example 317.
    let source = "- item\n\n  [ref]: /url\n";
    assert!(
        visible(source).contains("[ref]: /url"),
        "reference definition was concealed: {:?}",
        visible(source)
    );
}

#[test]
fn concealing_markers_leaves_the_prose() {
    assert_eq!(
        visible("## Title\n\nSome **bold** text.\n"),
        "Title\n\nSome bold text.\n"
    );
    assert_eq!(visible("> quoted\n> lines\n"), "quoted\nlines\n");
}

#[test]
fn multibyte_characters_are_never_split() {
    for source in ["héllo *wörld*", "🎉 **emoji** 🎉", "日本語 `コード` です"] {
        for seg in segment(source) {
            let range = seg.range();
            assert!(
                source.is_char_boundary(range.start) && source.is_char_boundary(range.end),
                "{source:?}: {range:?} splits a character"
            );
        }
    }
}
