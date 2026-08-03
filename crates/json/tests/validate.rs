//! The strict JSON check: what it accepts in silence, and what it points at.
//!
//! Two halves. Everything the lexer already renders must still *validate* to nothing when it is
//! valid — that is the half that decides whether the feature is usable, because a validator that
//! cries wolf on working files gets switched off. The other half asserts each defect class is caught
//! at the right offset, because a diagnostic in the wrong place is worse than none: MDEDIT-47's
//! menu item selects the span, so a wrong one moves the caret somewhere innocent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_json::validate;
use proptest::prelude::*;

/// The messages, in document order. What a test usually wants to assert.
fn messages(source: &str) -> Vec<&'static str> {
    validate(source).into_iter().map(|d| d.message).collect()
}

/// The bytes each diagnostic points at.
fn pointed_at(source: &str) -> Vec<String> {
    validate(source)
        .into_iter()
        .map(|d| d.span.slice(source).to_owned())
        .collect()
}

fn assert_valid(source: &str) {
    assert_eq!(
        validate(source),
        Vec::new(),
        "should be valid JSON: {source}"
    );
}

// ---- silence ----

#[test]
fn valid_documents_say_nothing() {
    for source in [
        "{}",
        "[]",
        "null",
        "true",
        "false",
        "0",
        "-0",
        "1e3",
        "-1.5E-10",
        r#""""#,
        r#"{"a": 1}"#,
        r#"{"a": [1, 2, {"b": null}], "c": "d"}"#,
        "[[[[[]]]]]",
        r#"  {  "a"  :  1  }  "#,
        "{\n\t\"a\": 1\r\n}",
        r#"{"esc": "\" \\ \/ \b \f \n \r \t é"}"#,
        r#"{"unicode": "汉字 🦀"}"#,
    ] {
        assert_valid(source);
    }
}

/// The empty case is deliberate rather than an oversight: a new file is empty, and an editor that
/// opens with an error on a document nobody has typed in is diagnosing the user's existence.
#[test]
fn an_empty_document_is_not_shouted_at() {
    assert_valid("");
    assert_valid("   ");
    assert_valid("\n\n\t\r\n");
}

/// The gate that decides whether anyone leaves it on. Every JSON fixture the lexer tests render must
/// validate clean, or the count in the status bar is noise.
#[test]
fn a_realistic_document_is_silent() {
    let source = r#"{
  "name": "mdedit",
  "version": "0.1.0",
  "keywords": ["markdown", "editor"],
  "nested": {"deep": {"deeper": [1, 2.5, -3e10, true, false, null]}},
  "escaped": "a \"quoted\" \\ path\/hereA",
  "empty": {"object": {}, "array": []}
}"#;
    assert_valid(source);
}

// ---- structure ----

#[test]
fn an_unclosed_container_is_pointed_at_where_it_opened() {
    assert_eq!(messages(r#"{"a": 1"#), ["this is never closed"]);
    assert_eq!(pointed_at(r#"{"a": 1"#), ["{"]);
}

/// An unclosed container is noticed at the end of the file but points at its opener, so it has to be
/// sorted back into place — otherwise "jump to the first error" jumps to whichever one was *found*
/// first, which is the last one a reader would reach.
#[test]
fn diagnostics_come_back_in_document_order() {
    let messages = messages("{!");
    assert_eq!(messages.as_slice(), ["this is never closed", "not JSON"]);
    assert_eq!(pointed_at("{!").as_slice(), ["{", "!"]);
}

/// Outermost first, and each one named, because "3 errors" with one span is not actionable.
#[test]
fn every_unclosed_container_is_reported() {
    assert_eq!(
        messages(r#"{"a": [{"#).as_slice(),
        [
            "this is never closed",
            "this is never closed",
            "this is never closed"
        ]
    );
    assert_eq!(pointed_at(r#"{"a": [{"#).as_slice(), ["{", "[", "{"]);
}

#[test]
fn a_closer_that_closes_nothing_is_caught() {
    assert_eq!(messages("}"), ["this `}` closes nothing"]);
    assert_eq!(messages("[1]]"), ["this `]` closes nothing"]);
}

/// Both ends are named. Knowing a `]` is wrong is half the information; knowing which `{` it was
/// meant for is the half that lets you fix it.
#[test]
fn a_mismatched_closer_names_both_ends() {
    assert_eq!(
        messages("{]").as_slice(),
        [
            "this `{` is closed by the wrong bracket",
            "this `]` closes something that is not an array"
        ]
    );
    assert_eq!(pointed_at("{]").as_slice(), ["{", "]"]);
}

#[test]
fn a_trailing_comma_blames_the_comma() {
    assert_eq!(messages(r#"{"a": 1,}"#), ["trailing comma"]);
    assert_eq!(pointed_at(r#"{"a": 1,}"#), [","]);
    assert_eq!(messages("[1, 2,]"), ["trailing comma"]);
    assert_eq!(pointed_at("[1, 2,]"), [","]);
}

#[test]
fn a_missing_colon_is_caught() {
    assert_eq!(
        messages(r#"{"a" 1}"#),
        ["a `:` belongs between a key and its value"]
    );
    assert_eq!(pointed_at(r#"{"a" 1}"#), ["1"]);
}

#[test]
fn a_missing_comma_is_caught() {
    assert_eq!(
        messages("[1 2]"),
        ["a `,` belongs between one value and the next"]
    );
}

#[test]
fn a_non_string_key_is_caught() {
    assert_eq!(messages("{1: 2}"), ["an object key must be a string"]);
}

#[test]
fn more_than_one_top_level_value_is_caught() {
    assert_eq!(messages("1 2"), ["a JSON document holds one value"]);
    assert_eq!(messages("{} {}"), ["a JSON document holds one value"]);
}

/// A document abandoned mid-value is reported where the unfinished thing *opened*, not at the end of
/// the file. `{"a":` is a `{` that is never closed, and pointing at the last byte would send the
/// caret past everything the user can act on.
#[test]
fn a_document_that_stops_mid_value_is_blamed_at_its_opener() {
    assert_eq!(messages("[1,"), ["this is never closed"]);
    assert_eq!(pointed_at("[1,"), ["["]);
    assert_eq!(messages(r#"{"a":"#), ["this is never closed"]);
    assert_eq!(
        messages(r#"{"a"#),
        ["this is never closed", "this string is never closed"]
    );
}

/// And a stray separator at the top level is one problem, not that problem plus an end-of-document
/// one. There is deliberately no end-of-document diagnostic at all.
#[test]
fn a_stray_separator_reports_once() {
    assert_eq!(messages(":"), ["a `:` belongs after an object key"]);
    assert_eq!(messages("[1],"), ["a `,` separates nothing here"]);
}

// ---- tokens ----

#[test]
fn an_unterminated_string_is_caught() {
    assert_eq!(
        messages(r#"{"a": "oops}"#).as_slice(),
        ["this is never closed", "this string is never closed"]
    );
}

/// The one that bites in real files: a literal newline inside a string looks perfectly fine and is
/// not JSON.
#[test]
fn a_raw_control_character_in_a_string_is_caught() {
    assert_eq!(
        messages("\"a\nb\""),
        ["a control character must be escaped"]
    );
    assert_eq!(
        messages("\"a\tb\""),
        ["a control character must be escaped"]
    );
}

#[test]
fn a_bad_escape_is_caught() {
    assert_eq!(messages(r#""\q""#), ["not an escape JSON knows"]);
    assert_eq!(pointed_at(r#""\q""#), [r"\q"]);
    assert_eq!(messages(r#""\u12""#), ["`\\u` needs four hex digits"]);
    assert_eq!(messages(r#""\uZZZZ""#), ["`\\u` needs four hex digits"]);
}

/// JSON's number grammar is stricter than people expect, and staying quiet about it would let the
/// user write a file the next tool rejects.
#[test]
fn numbers_are_judged_by_jsons_grammar_not_by_rusts() {
    for bad in ["01", "1.", ".5", "+1", "1e", "1e+", "1.2.3", "-", "--1"] {
        assert_eq!(
            messages(bad).first(),
            Some(&"not a number JSON accepts"),
            "{bad} should not be a number"
        );
    }
    for good in [
        "0", "-0", "1", "-1", "1.0", "1e3", "1E3", "1e+3", "1e-3", "0.5",
    ] {
        assert_valid(good);
    }
}

#[test]
fn a_bare_word_that_is_not_a_literal_is_caught() {
    assert_eq!(
        messages("tru"),
        ["only `true`, `false` and `null` are bare words in JSON"]
    );
    assert_eq!(
        messages("[True]"),
        ["only `true`, `false` and `null` are bare words in JSON"]
    );
}

/// A no-break space is not whitespace to JSON. It is also invisible, which is what makes it worth a
/// diagnostic — it is what a document pasted out of a word processor is full of.
#[test]
fn a_byte_that_begins_nothing_is_reported_where_it_sits() {
    assert_eq!(messages("[1,\u{a0}2]"), ["not JSON"]);
    assert_eq!(pointed_at("[1,\u{a0}2]"), ["\u{a0}"]);
}

// ---- the properties ----

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    // Recursive descent dies somewhere in the low thousands. This is the case that makes the
    // explicit stack a security decision rather than a style one — the file comes from outside.
    let deep = format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
    assert_valid(&deep);

    let unclosed = "[".repeat(200_000);
    assert_eq!(validate(&unclosed).len(), mdedit_json::LIMIT);
}

#[test]
fn a_document_of_pure_garbage_stops_at_the_limit() {
    let garbage = "@".repeat(10_000);
    assert_eq!(validate(&garbage).len(), mdedit_json::LIMIT);
}

proptest! {
    /// Every diagnostic must be sliceable and selectable: on character boundaries, in order, and
    /// inside the document. The menu item moves the caret to one of these, so a span that is not is
    /// a panic from a render on a file the user merely opened.
    #[test]
    fn every_span_is_usable(source in ".{0,200}") {
        let mut previous = 0;
        for diagnostic in validate(&source) {
            let (start, end) = (diagnostic.span.start().get(), diagnostic.span.end().get());
            prop_assert!(start <= end, "backwards span in {source:?}");
            prop_assert!(end <= source.len(), "span past the end in {source:?}");
            prop_assert!(source.is_char_boundary(start), "start off a boundary in {source:?}");
            prop_assert!(source.is_char_boundary(end), "end off a boundary in {source:?}");
            // Document order, which is what "jump to the first error" depends on meaning anything.
            prop_assert!(start >= previous, "out of order in {source:?}");
            previous = start;
        }
    }

    /// Whatever it is handed, it terminates and stays within its budget. Both are the sort of thing
    /// a hand-rolled scanner gets wrong on input nobody imagined.
    #[test]
    fn it_always_terminates_and_stays_bounded(source in "[\\{\\}\\[\\]\",:0-9a-z \\\\n]{0,300}") {
        prop_assert!(validate(&source).len() <= mdedit_json::LIMIT);
    }
}
