//! Reformatting JSON: what it produces, what it refuses, and what it must never quietly change.
//!
//! The properties at the bottom are the ones that matter, and they are the byte-preservation
//! invariant wearing the only hat it is allowed to take off. This is the single sanctioned
//! whole-document rewrite in the editor, so the question is not "does it look nice" but **"is the
//! document that comes out the same document"** — same key order, same numbers, same escapes. A
//! formatter that answers no is a formatter that loses the user's data while appearing to tidy it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_json::{format, meaningful_bytes, validate};
use proptest::prelude::*;

fn formatted(source: &str) -> String {
    format(source).expect("should be valid JSON")
}

// ---- what it produces ----

#[test]
fn a_messy_document_comes_back_tidy() {
    let source = r#"{"name":"mdedit","tags":["a","b"],"nested":{"deep":true}}"#;
    assert_eq!(
        formatted(source),
        "{\n  \"name\": \"mdedit\",\n  \"tags\": [\n    \"a\",\n    \"b\"\n  ],\n  \"nested\": {\n    \"deep\": true\n  }\n}\n"
    );
}

/// Two spaces per level, and a level per container.
#[test]
fn nesting_indents_by_two_spaces_a_level() {
    assert_eq!(
        formatted("[[[1]]]"),
        "[\n  [\n    [\n      1\n    ]\n  ]\n]\n"
    );
}

/// An empty container stays on one line, which is what every formatter does and what anybody
/// reading `"tags": []` expects to see.
#[test]
fn empty_containers_stay_on_one_line() {
    assert_eq!(formatted("{}"), "{}\n");
    assert_eq!(formatted("[]"), "[]\n");
    assert_eq!(
        formatted(r#"{"a":{},"b":[]}"#),
        "{\n  \"a\": {},\n  \"b\": []\n}\n"
    );
}

#[test]
fn a_bare_value_is_left_alone_but_for_the_newline() {
    assert_eq!(formatted("42"), "42\n");
    assert_eq!(formatted(r#"  "hello"  "#), "\"hello\"\n");
    assert_eq!(formatted("null"), "null\n");
}

/// An empty document is not turned into a newline. There is nothing to format, and inventing a byte
/// would be an edit nobody asked for — the same rule that keeps the validator quiet about it.
#[test]
fn an_empty_document_is_returned_unchanged() {
    assert_eq!(formatted(""), "");
    assert_eq!(formatted("   \n"), "   \n");
}

/// Already-formatted input is a no-op, which is what makes pressing the menu item twice harmless.
#[test]
fn formatting_is_idempotent() {
    let once = formatted(r#"{"a":[1,{"b":null}],"c":{}}"#);
    assert_eq!(formatted(&once), once);
}

// ---- what it refuses ----

/// The gate. A malformed document is not formatted at all, because formatting one would mean
/// inventing the structure the user has not finished typing.
#[test]
fn a_malformed_document_is_refused_with_its_first_problem() {
    let error = format(r#"{"a": 1,}"#).unwrap_err();
    assert_eq!(error.message, "trailing comma");
    assert_eq!(error.span.start().get(), 7);
}

#[test]
fn every_kind_of_malformed_document_is_refused() {
    for bad in [
        r#"{"a": 1"#,
        "[1 2]",
        "{1: 2}",
        r#""unterminated"#,
        "01",
        "tru",
        "{} {}",
    ] {
        assert!(format(bad).is_err(), "{bad} should have been refused");
    }
}

/// And the refusal changes nothing, which is the acceptance criterion phrased as a test: there is no
/// partial output to accidentally apply.
#[test]
fn a_refusal_yields_no_document_at_all() {
    assert!(format("[1,").is_err());
}

// ---- what must survive ----

/// The one that would be silently wrong if this went through a number type. `1.50` parsed as an f64
/// and printed back is `1.5`; `1e3` is `1000`; `-0` is `0`. All three are different bytes, and two of
/// them are a different document to anything reading it with a decimal library.
#[test]
fn number_spellings_survive_exactly() {
    let source = "[1.50,1e3,1E3,-0,0.0,1e+3,1e-3,123456789012345678901234567890]";
    let out = formatted(source);
    for spelling in [
        "1.50",
        "1e3",
        "1E3",
        "-0",
        "0.0",
        "1e+3",
        "1e-3",
        "123456789012345678901234567890",
    ] {
        assert!(out.contains(spelling), "{spelling} should have survived");
    }
}

/// Escapes are bytes, not characters. `A` must not become `A`, and `\/` must not become `/` —
/// both are the same *string* and a different *document*.
#[test]
fn string_escapes_survive_exactly() {
    let source = r#"{"a":"A \/ \\ \" \n \t"}"#;
    let out = formatted(source);
    assert!(out.contains(r#""A \/ \\ \" \n \t""#), "got {out}");
}

/// Never sorted. There is no map to lose the order in, which is the point of formatting a token
/// stream rather than a value.
#[test]
fn key_order_survives() {
    let out = formatted(r#"{"z":1,"a":2,"m":3}"#);
    let z = out.find("\"z\"").unwrap();
    let a = out.find("\"a\"").unwrap();
    let m = out.find("\"m\"").unwrap();
    assert!(z < a && a < m, "keys were reordered: {out}");
}

#[test]
fn multi_byte_content_survives() {
    let out = formatted(r#"{"汉字":"🦀 café"}"#);
    assert!(out.contains(r#""汉字": "🦀 café""#), "got {out}");
}

proptest! {
    /// **The one that matters.** Whatever it formats, the result is still valid JSON and still the
    /// same document: the token stream either side is identical, byte for byte, once whitespace is
    /// out of the way. That is the byte-preservation invariant applied to the one operation allowed
    /// to break its letter.
    #[test]
    fn formatting_changes_only_the_whitespace(source in valid_json()) {
        let out = format(&source).expect("the generator only makes valid JSON");
        prop_assert!(validate(&out).is_empty(), "formatting produced invalid JSON: {out}");
        prop_assert_eq!(
            meaningful_bytes(&out),
            meaningful_bytes(&source),
            "formatting changed something other than whitespace"
        );
    }

    /// And doing it again changes nothing, so the menu item is safe to press twice.
    #[test]
    fn formatting_twice_is_formatting_once(source in valid_json()) {
        let once = format(&source).expect("the generator only makes valid JSON");
        prop_assert_eq!(format(&once).expect("formatted output stays valid"), once);
    }
}

/// Valid JSON documents, nested a few levels, with the spellings that matter in them.
fn valid_json() -> impl Strategy<Value = String> {
    let scalar = prop_oneof![
        Just("null".to_owned()),
        Just("true".to_owned()),
        Just("false".to_owned()),
        Just("1.50".to_owned()),
        Just("-0".to_owned()),
        Just("1e3".to_owned()),
        Just("0".to_owned()),
        Just(r#""""#.to_owned()),
        Just(r#""A \/ x""#.to_owned()),
        Just(r#""汉""#.to_owned()),
    ];
    scalar.prop_recursive(4, 32, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4)
                .prop_map(|items| format!("[{}]", items.join(","))),
            prop::collection::vec(inner, 0..4).prop_map(|items| {
                let members: Vec<String> = items
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| format!("\"k{index}\":{value}"))
                    .collect();
                format!("{{{}}}", members.join(","))
            }),
        ]
    })
}
