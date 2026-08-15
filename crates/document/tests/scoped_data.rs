//! The correctness gate for scoped re-parsing of JSON, YAML and TOML (MDEDIT-36, MDEDIT-95).
//!
//! The same gate `scoped_parse.rs` holds for Markdown, and it is the whole of the argument for this
//! change: a keystroke re-lexes only the region around the edit, which is sound only if the result
//! agrees with what a whole-document lex would have said.
//!
//! MDEDIT-30 and MDEDIT-31 both declined to do this, reasoning that neither language can recover its
//! state from a slice. That is true of an *arbitrary* slice and beside the point for the only ones
//! the caller ever asks for: `Document::scope_for` snaps every region to a depth-zero block start,
//! and at such a boundary JSON's depth is one and YAML is provably outside every block scalar. These
//! tests are what make that reasoning checkable rather than merely plausible.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;

use mdedit_document::Document;
use mdedit_offsets::ByteOffset;
use proptest::prelude::*;

/// Comfortably past the 16 KB threshold, so the scoped path is actually taken.
const PADDING_ENTRIES: usize = 600;

fn json_padding(from: usize, count: usize) -> String {
    let mut text = String::new();
    for n in from..from + count {
        writeln!(
            text,
            "  \"key_{n}\": {{ \"name\": \"value {n}\", \"n\": {n}, \"ok\": true }},"
        )
        .expect("a String never fails to write");
    }
    text
}

fn yaml_padding(from: usize, count: usize) -> String {
    let mut text = String::new();
    for n in from..from + count {
        writeln!(text, "key_{n}:\n  name: value {n}\n  n: {n}\n  ok: true")
            .expect("a String never fails to write");
    }
    text
}

/// A JSON document with `body` buried deep enough to trigger scoped lexing, and where it starts.
fn buried_json(body: &str) -> (String, usize) {
    let mut text = String::from("{\n");
    text.push_str(&json_padding(0, PADDING_ENTRIES));
    let at = text.len();
    text.push_str(body);
    text.push_str(&json_padding(PADDING_ENTRIES, PADDING_ENTRIES));
    text.push_str("  \"last\": null\n}\n");
    (text, at)
}

fn buried_yaml(body: &str) -> (String, usize) {
    let mut text = yaml_padding(0, PADDING_ENTRIES);
    let at = text.len();
    text.push_str(body);
    text.push_str(&yaml_padding(PADDING_ENTRIES, PADDING_ENTRIES));
    (text, at)
}

fn toml_padding(from: usize, count: usize) -> String {
    let mut text = String::new();
    for n in from..from + count {
        writeln!(
            text,
            "[key_{n}]\nname = \"value {n}\"\nn = {n}\nok = true\n"
        )
        .expect("a String never fails to write");
    }
    text
}

fn buried_toml(body: &str) -> (String, usize) {
    let mut text = toml_padding(0, PADDING_ENTRIES);
    let at = text.len();
    text.push_str(body);
    text.push_str(&toml_padding(PADDING_ENTRIES, PADDING_ENTRIES));
    (text, at)
}

/// Edits `document` at `at`, then compares its analysis with a fresh whole-document one.
///
/// Returns `(scoped, whole)` as comparable strings — the segment and block lists, which is the whole
/// of what a renderer reads.
fn after_edit(source: &str, at: usize, insert: &str, name: &str) -> (String, String) {
    let language = match name {
        "json" => &mdedit_json::JSON as &'static dyn mdedit_language::Language,
        "toml" => &mdedit_toml::TOML,
        _ => &mdedit_yaml::YAML,
    };

    let mut document = Document::with_text(source, language);
    document
        .insert(ByteOffset::new(at), insert)
        .expect("an edit at a character boundary");
    assert!(
        document.is_scoped(),
        "the document should have taken the scoped path, or this proves nothing"
    );
    let scoped = describe(&document);

    let mut whole = Document::with_text(&document.to_string(), language);
    // A no-op edit that forces the whole-document path, so both sides are built the same way.
    whole.reconcile();
    (scoped, describe(&whole))
}

fn describe(document: &Document) -> String {
    let text = document.to_string();
    let mut out = String::new();
    for segment in document.segments() {
        let _ = writeln!(
            out,
            "{:?} {:?}",
            segment.kind(),
            segment.span().slice(&text)
        );
    }
    for block in document.blocks() {
        let _ = writeln!(out, "block {:?} {:?}", block.kind(), block.span());
    }
    out
}

/// The gate, for JSON.
#[test]
fn a_scoped_json_lex_agrees_with_a_whole_one() {
    for (body, insert) in [
        ("  \"target\": \"value\",\n", "x"),
        ("  \"target\": 42,\n", "9"),
        ("  \"target\": [1, 2, 3],\n", ", 4"),
        ("  \"target\": { \"deep\": true },\n", " "),
        ("  \"target\": null,\n", "\"a\": 1, "),
    ] {
        let (source, at) = buried_json(body);
        // Into the middle of the buried body, not at its edge.
        let (scoped, whole) = after_edit(&source, at + body.len() / 2, insert, "json");
        assert_eq!(scoped, whole, "inserting {insert:?} into {body:?}");
    }
}

/// And for YAML.
#[test]
fn a_scoped_yaml_lex_agrees_with_a_whole_one() {
    for (body, insert) in [
        ("target:\n  name: value\n", "x"),
        ("target:\n  n: 42\n", "9"),
        ("target:\n  list: [1, 2]\n", ", 3"),
        ("target: &anchor\n  flag: true\n", " "),
        ("target: |\n  literal text\n", "more "),
    ] {
        let (source, at) = buried_yaml(body);
        let (scoped, whole) = after_edit(&source, at + body.len() / 2, insert, "yaml");
        assert_eq!(scoped, whole, "inserting {insert:?} into {body:?}");
    }
}

/// And for TOML (MDEDIT-95). Its restart point is a column-zero line outside every multi-line
/// string with no array open, which is what a block is defined as there.
#[test]
fn a_scoped_toml_lex_agrees_with_a_whole_one() {
    for (body, insert) in [
        ("[target]\nname = \"value\"\n", "x"),
        ("[target]\nn = 42\n", "9"),
        ("[target]\nlist = [1, 2]\n", ", 3"),
        ("[target]\nwhen = 1979-05-27T07:32:00Z\n", "0"),
        ("[target]\ndoc = \"\"\"\ntext\n\"\"\"\n", "more "),
    ] {
        let (source, at) = buried_toml(body);
        let (scoped, whole) = after_edit(&source, at + body.len() / 2, insert, "toml");
        assert_eq!(scoped, whole, "inserting {insert:?} into {body:?}");
    }
}

/// Tiling still holds after a scoped lex — the invariant every language is judged against, now
/// applied to a spliced analysis rather than a fresh one.
#[test]
fn a_spliced_analysis_still_tiles() {
    for name in ["json", "yaml", "toml"] {
        let (source, at) = match name {
            "json" => buried_json("  \"target\": \"value\",\n"),
            "toml" => buried_toml("[target]\nname = \"value\"\n"),
            _ => buried_yaml("target:\n  name: value\n"),
        };
        let language = match name {
            "json" => &mdedit_json::JSON as &'static dyn mdedit_language::Language,
            "toml" => &mdedit_toml::TOML,
            _ => &mdedit_yaml::YAML,
        };

        let mut document = Document::with_text(&source, language);
        document.insert(ByteOffset::new(at + 5), "xyz").unwrap();

        let text = document.to_string();
        let rebuilt: String = document
            .segments()
            .iter()
            .map(|segment| segment.span().slice(&text))
            .collect();
        assert_eq!(rebuilt, text, "{name} did not tile after a scoped lex");
    }
}

/// Byte preservation across a scoped lex, which is the rule that outranks everything.
#[test]
fn a_scoped_lex_does_not_touch_the_buffer() {
    let (source, at) = buried_json("  \"target\": \"value\",\n");
    let mut document = Document::with_text(&source, &mdedit_json::JSON);
    document
        .insert(ByteOffset::new(at), "  \"added\": 1,\n")
        .unwrap();

    let expected = {
        let mut text = source.clone();
        text.insert_str(at, "  \"added\": 1,\n");
        text
    };
    assert_eq!(document.to_string(), expected);
}

proptest! {
    /// Over generated edits at generated positions, which is where a boundary bug hides.
    #[test]
    fn any_json_edit_agrees_with_a_whole_lex(
        // Deliberately no braces, brackets or quotes. Those change the structure *enclosing* the
        // region — which a region cannot see, by definition — and that divergence is pinned below
        // rather than asserted away here.
        insert in "[a-z0-9 ,:]{0,8}",
        position in 0usize..2000,
    ) {
        let (source, at) = buried_json("  \"target\": \"value\",\n");
        // Somewhere in the padded middle, snapped to a character boundary.
        let mut offset = (at + position).min(source.len());
        while !source.is_char_boundary(offset) {
            offset -= 1;
        }
        let (scoped, whole) = after_edit(&source, offset, &insert, "json");
        prop_assert_eq!(scoped, whole);
    }

    #[test]
    fn any_yaml_edit_agrees_with_a_whole_lex(
        insert in "[a-z0-9 :\\-\\n]{0,8}",
        position in 0usize..2000,
    ) {
        let (source, at) = buried_yaml("target:\n  name: value\n");
        let mut offset = (at + position).min(source.len());
        while !source.is_char_boundary(offset) {
            offset -= 1;
        }
        let (scoped, whole) = after_edit(&source, offset, &insert, "yaml");
        prop_assert_eq!(scoped, whole);
    }

    #[test]
    fn any_toml_edit_agrees_with_a_whole_lex(
        // No quotes or brackets, for the reason given on the JSON generator: those change the
        // structure *enclosing* the region, which a region cannot see by definition.
        insert in "[a-z0-9 =.\\-\\n]{0,8}",
        position in 0usize..2000,
    ) {
        let (source, at) = buried_toml("[target]\nname = \"value\"\n");
        let mut offset = (at + position).min(source.len());
        while !source.is_char_boundary(offset) {
            offset -= 1;
        }
        let (scoped, whole) = after_edit(&source, offset, &insert, "toml");
        prop_assert_eq!(scoped, whole);
    }
}

// ---------------------------------------------------------------------------
// Where it deliberately does not agree, and what fixes it.
// ---------------------------------------------------------------------------

fn segments_of(document: &Document) -> String {
    let text = document.to_string();
    let mut out = String::new();
    for segment in document.segments() {
        let _ = writeln!(
            out,
            "{:?} {:?}",
            segment.kind(),
            segment.span().slice(&text)
        );
    }
    out
}

fn blocks_of(document: &Document) -> String {
    let mut out = String::new();
    for block in document.blocks() {
        let _ = writeln!(out, "{:?} {:?}", block.kind(), block.span());
    }
    out
}

/// Typing `}` closes the root container early, so every member below it is one level shallower than
/// it was. A region cannot see that — it is looking at the region. This is the case the reconcile
/// exists for, and it is pinned rather than hidden.
///
/// It also pins *how far* the damage goes, which is the part worth knowing: JSON's token classes do
/// not depend on nesting depth, so what diverges is the block list — the chunking — and not a single
/// colour on screen.
#[test]
fn closing_a_container_early_diverges_until_reconciled() {
    let (source, at) = buried_json("  \"target\": \"value\",\n");
    let mut document = Document::with_text(&source, &mdedit_json::JSON);

    document.insert(ByteOffset::new(at), "}").unwrap();
    assert!(document.is_scoped());
    let scoped_segments = segments_of(&document);
    let scoped_blocks = blocks_of(&document);

    assert!(
        document.reconcile(),
        "the brace changed what the rest of the document is"
    );
    assert!(!document.is_scoped());

    assert_eq!(
        scoped_segments,
        segments_of(&document),
        "the colours were right all along: a token's class does not depend on its depth"
    );
    assert_ne!(
        scoped_blocks,
        blocks_of(&document),
        "if these matched, this test would be asserting nothing"
    );
}

/// An unterminated quote is the other kind of divergence, and the one that *is* visible: everything
/// after it becomes one long string, which changes colours rather than only chunking.
///
/// Pinned for the same reason as the brace, and it is worth knowing the two are different: this one
/// the user sees, for as long as the quote stays open and no longer than the reconcile.
#[test]
fn an_unterminated_quote_diverges_visibly_until_reconciled() {
    let (source, at) = buried_json("  \"target\": \"value\",\n");
    let mut document = Document::with_text(&source, &mdedit_json::JSON);

    document.insert(ByteOffset::new(at), "\"").unwrap();
    assert!(document.is_scoped());
    let scoped_segments = segments_of(&document);

    assert!(document.reconcile());
    assert_ne!(
        scoped_segments,
        segments_of(&document),
        "an open quote changes what everything after it is, and the region cannot see that"
    );
}

/// The reconcile is not optional for either case, so this pins that the editor is told to run one.
#[test]
fn a_scoped_analysis_says_it_is_scoped() {
    let (source, at) = buried_json("  \"target\": \"value\",\n");
    let mut document = Document::with_text(&source, &mdedit_json::JSON);
    assert!(!document.is_scoped(), "a fresh document is whole");

    document.insert(ByteOffset::new(at), "x").unwrap();
    assert!(
        document.is_scoped(),
        "which is what schedules the whole-document parse when the typing stops"
    );
}
