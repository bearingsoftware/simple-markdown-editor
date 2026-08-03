//! Converting line endings, and telling the truth about which a document has.
//!
//! The property at the bottom is the acceptance criterion MDEDIT-59 asks for, and it is the
//! byte-preservation invariant applied to an operation whose whole purpose is to change bytes:
//! converting A → B → A must give back exactly the document that was there, or "convert to CRLF and
//! back" is a way of quietly losing work.
//!
//! **The lone `\r` is the case to watch.** Nothing else in this codebase produces one, so nothing
//! else would have noticed it being left behind — and it is the ending a user is most likely to be
//! trying to get rid of.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_file::{Newline, normalise_newlines};
use proptest::prelude::*;

// ---- what a document is ----

#[test]
fn each_style_is_recognised() {
    assert_eq!(Newline::detect("a\nb\nc"), Newline::Lf);
    assert_eq!(Newline::detect("a\r\nb\r\nc"), Newline::CrLf);
    assert_eq!(Newline::detect("a\rb\rc"), Newline::Cr);
    assert_eq!(Newline::detect("a\nb\r\nc"), Newline::Mixed);
    assert_eq!(Newline::detect("a\rb\nc"), Newline::Mixed);
}

/// **Was reported as LF, which was a lie about a document containing not one of them.** MDEDIT-46
/// draws these endings in the warning colour, which put the bar's wrong answer next to the evidence.
#[test]
fn a_classic_mac_file_is_not_called_lf() {
    assert_eq!(Newline::detect("first\rsecond\rthird\r"), Newline::Cr);
}

#[test]
fn a_document_with_no_endings_at_all_is_lf() {
    assert_eq!(Newline::detect(""), Newline::Lf);
    assert_eq!(Newline::detect("no endings here"), Newline::Lf);
}

// ---- what Enter inserts ----

#[test]
fn each_style_says_what_to_insert() {
    assert_eq!(Newline::Lf.as_str(), "\n");
    assert_eq!(Newline::CrLf.as_str(), "\r\n");
    assert_eq!(Newline::Cr.as_str(), "\r");
    // A document that is already both has no answer to give, and the one that does not add a byte
    // is the better guess.
    assert_eq!(Newline::Mixed.as_str(), "\n");
}

// ---- converting ----

#[test]
fn every_terminator_is_converted() {
    let mixed = "lf\ncrlf\r\ncr\rend";
    assert_eq!(
        normalise_newlines(mixed, Newline::Lf).unwrap(),
        "lf\ncrlf\ncr\nend"
    );
    assert_eq!(
        normalise_newlines(mixed, Newline::CrLf).unwrap(),
        "lf\r\ncrlf\r\ncr\r\nend"
    );
}

/// The trap named on the ticket. A lone `\r` mid-document is a terminator like any other.
#[test]
fn a_lone_cr_is_converted_in_both_directions() {
    assert_eq!(
        normalise_newlines("a\rb\rc", Newline::Lf).unwrap(),
        "a\nb\nc"
    );
    assert_eq!(
        normalise_newlines("a\rb\rc", Newline::CrLf).unwrap(),
        "a\r\nb\r\nc"
    );
}

/// A conversion that changes nothing makes no edit — no undo entry, no modified flag.
#[test]
fn converting_to_what_it_already_is_is_a_no_op() {
    assert_eq!(normalise_newlines("a\nb\nc\n", Newline::Lf), None);
    assert_eq!(normalise_newlines("a\r\nb\r\n", Newline::CrLf), None);
    assert_eq!(normalise_newlines("", Newline::Lf), None);
    assert_eq!(normalise_newlines("no endings", Newline::CrLf), None);
}

/// A `\r\n` is one terminator, not two. Converting it to LF gives one `\n`, and a naive
/// character-by-character pass would give two.
#[test]
fn crlf_is_one_terminator() {
    assert_eq!(normalise_newlines("a\r\nb", Newline::Lf).unwrap(), "a\nb");
}

#[test]
fn a_trailing_terminator_survives() {
    assert_eq!(normalise_newlines("a\n", Newline::CrLf).unwrap(), "a\r\n");
    assert_eq!(normalise_newlines("a\r\n", Newline::Lf).unwrap(), "a\n");
}

#[test]
fn consecutive_terminators_each_convert() {
    assert_eq!(
        normalise_newlines("a\n\n\nb", Newline::CrLf).unwrap(),
        "a\r\n\r\n\r\nb"
    );
    assert_eq!(
        normalise_newlines("a\r\n\r\nb", Newline::Lf).unwrap(),
        "a\n\nb"
    );
}

#[test]
fn multi_byte_content_is_untouched() {
    assert_eq!(
        normalise_newlines("汉字\n🦀\r\ncafé\r!", Newline::Lf).unwrap(),
        "汉字\n🦀\ncafé\n!"
    );
}

/// The converted document says what it now is, which is what keeps the buffer, the format and the
/// status bar in agreement.
#[test]
fn a_converted_document_detects_as_its_new_style() {
    for target in [Newline::Lf, Newline::CrLf] {
        let out = normalise_newlines("a\nb\r\nc\rd", target).unwrap();
        assert_eq!(Newline::detect(&out), target);
    }
}

proptest! {
    /// **The one the ticket asks for.** Converting to one style and back gives the original bytes,
    /// as long as the original had one style to begin with — which is the case a user can actually
    /// get back to. A document that started mixed cannot round-trip, and nothing claims it can.
    #[test]
    fn converting_there_and_back_restores_the_bytes(
        lines in prop::collection::vec("[ab]{0,4}", 1..12),
        crlf in any::<bool>(),
    ) {
        let (from, to) = if crlf { (Newline::CrLf, Newline::Lf) } else { (Newline::Lf, Newline::CrLf) };
        let original = lines.join(from.as_str());

        let there = normalise_newlines(&original, to).unwrap_or_else(|| original.clone());
        let back = normalise_newlines(&there, from).unwrap_or_else(|| there.clone());

        prop_assert_eq!(back, original);
    }

    /// And whatever it is handed, the result uses only the target's terminator.
    #[test]
    fn nothing_of_the_old_style_survives(
        pieces in prop::collection::vec(prop_oneof![
            Just("x".to_owned()),
            Just("\n".to_owned()),
            Just("\r\n".to_owned()),
            Just("\r".to_owned()),
        ], 0..30),
        crlf in any::<bool>(),
    ) {
        let target = if crlf { Newline::CrLf } else { Newline::Lf };
        let source: String = pieces.concat();
        let out = normalise_newlines(&source, target).unwrap_or(source);

        if target == Newline::Lf {
            // No carriage returns at all survive a conversion to LF.
            prop_assert!(!out.contains('\r'));
        } else {
            // And every `\r` is part of a `\r\n`, with no lone `\n` left over.
            prop_assert_eq!(out.matches('\r').count(), out.matches("\r\n").count());
            prop_assert_eq!(out.matches('\n').count(), out.matches("\r\n").count());
        }
    }
}
