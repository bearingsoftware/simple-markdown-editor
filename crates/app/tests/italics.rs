//! The second cue the theme's contract needs (MDEDIT-74).
//!
//! Two of Nocturne's roles are "never colour alone": a blockquote is `muted` **and italic**, and a
//! `true` / `false` / `null` is `constant` **and italic**. Colour on its own fails a reader who
//! cannot tell two greys or two blues apart, which is the whole reason the pack states the cue.
//!
//! Asserted on the run builder rather than in a window: it knows the style of every run and needs no
//! GPU, and a slant is a property of the run rather than of how it was drawn.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::view::{Decoration, Mode, Theme, build_runs_in};
use mdedit_document::Document;
use mdedit_language::Language;
use mdedit_offsets::{ByteOffset, ByteSpan};

/// Whether every run whose text contains `needle` is italic, and that there was at least one.
fn italic(source: &str, needle: &str) -> bool {
    slanted(source, needle, &mdedit_markers::MARKDOWN, Mode::Preview)
}

fn slanted(source: &str, needle: &str, language: &'static dyn Language, mode: Mode) -> bool {
    let doc = Document::with_text(source, language);
    let decoration = Decoration {
        theme: &Theme::default(),
        mode,
    };
    let runs = build_runs_in(
        &doc,
        decoration,
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
        runs.iter().map(|run| run.text).collect::<Vec<_>>()
    );
    matching.iter().all(|run| run.style.italic)
}

#[test]
fn quoted_prose_is_italic() {
    assert!(italic("> a quotation\n", "a quotation"));
}

/// The end of the quote is the end of the slant, which is what a block *extent* buys over a toggle.
#[test]
fn the_italics_stop_where_the_quote_does() {
    let source = "> quoted\n\nplain again\n";
    assert!(italic(source, "quoted"));
    assert!(!italic(source, "plain again"));
}

/// `DecorState` resets at every newline. A quote spans lines by definition, so a toggle would be
/// cleared by the first one — this is the test that says the block list is being asked instead.
#[test]
fn a_quote_of_several_lines_is_italic_throughout() {
    let source = "> first line\n> second line\n> third line\n";
    for line in ["first line", "second line", "third line"] {
        assert!(italic(source, line), "{line} is not italic");
    }
}

#[test]
fn a_nested_quote_is_italic_and_what_follows_it_is_not() {
    let source = "> outer\n>\n> > inner\n\nafter\n";
    assert!(italic(source, "outer"));
    assert!(italic(source, "inner"));
    assert!(!italic(source, "after"));
}

/// A drawn `>` is syntax, and syntax that slants reads as prose.
///
/// **In plain markup**, which since MDEDIT-118 is the only place a marker is drawn at all. The claim
/// is unchanged — it is where it can be asserted that moved.
#[test]
fn the_quote_marker_itself_is_not_italic() {
    let source = "> quoted\n";
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
    let marker = runs
        .iter()
        .find(|run| run.text.starts_with('>'))
        .expect("plain markup draws the marker");
    assert!(!marker.style.italic);
}

/// Italic monospace is where alignment goes to die, and a fence inside a quote is still a fence.
#[test]
fn code_inside_a_quote_is_not_italic() {
    let source = "> ```\n> let x = 1;\n> ```\n";
    assert!(!italic(source, "let x = 1;"));
}

#[test]
fn json_constants_are_italic_and_strings_are_not() {
    let json = &mdedit_json::JSON;
    // Each constant on its own, so a run holding two of them cannot hide a miss.
    for constant in ["true", "false", "null"] {
        let only = format!(r#"{{"a": {constant}}}"#);
        assert!(
            slanted(&only, constant, json, Mode::Preview),
            "{constant} is not italic in JSON"
        );
    }
    // The one that says the token classification is being read rather than the text: a quoted
    // "true" is a string, and a string is not a constant.
    assert!(
        !slanted(r#"{"a": "true"}"#, "true", json, Mode::Preview),
        "a quoted \"true\" italicised, so its classification was ignored"
    );
}

#[test]
fn yaml_constants_are_italic() {
    let yaml = &mdedit_yaml::YAML;
    for constant in ["true", "false", "null"] {
        let source = format!("a: {constant}\n");
        assert!(
            slanted(&source, constant, yaml, Mode::Preview),
            "{constant} is not italic in YAML"
        );
    }
}

/// Plain markup is one size and one weight all the way down, and that includes the slant.
#[test]
fn plain_markup_slants_nothing() {
    assert!(!slanted(
        "> quoted\n",
        "quoted",
        &mdedit_markers::MARKDOWN,
        Mode::Plain
    ));
    assert!(!slanted(
        r#"{"a": true}"#,
        "true",
        &mdedit_json::JSON,
        Mode::Plain
    ));
}
