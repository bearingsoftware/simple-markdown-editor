//! Toggling inline formatting on and off.
//!
//! The interesting half is *off*. Wrapping a selection is arithmetic; removing the right pair of
//! delimiters from `**a _b_ c**` is what every editor that bolts Markdown buttons onto a text widget
//! gets wrong, and it is the one thing this editor can do properly because it already computes the
//! delimiter spans.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::format::Inline;
use mdedit_document::{Document, Selection};
use mdedit_offsets::{ByteOffset, ByteSpan};
use proptest::prelude::*;

/// A document with the caret at `caret`.
fn at(text: &str, caret: usize) -> Document {
    let mut doc = Document::from(text);
    doc.set_selection(Selection::caret(ByteOffset::new(caret)));
    doc
}

/// A document with `from..to` selected.
fn selecting(text: &str, from: usize, to: usize) -> Document {
    let mut doc = Document::from(text);
    doc.set_selection(Selection::new(ByteOffset::new(from), ByteOffset::new(to)));
    doc
}

// ---- turning it on ----

#[test]
fn a_selection_is_wrapped() {
    let mut doc = selecting("hello world\n", 0, 5);
    assert!(doc.toggle_inline(Inline::Bold));
    assert_eq!(doc.to_string(), "**hello** world\n");
}

/// Standing in a word and pressing the chord bolds the word — what Typora does, and what people
/// reach for.
#[test]
fn a_caret_mid_word_wraps_the_word() {
    let mut doc = at("hello world\n", 8);
    assert!(doc.toggle_inline(Inline::Bold));
    assert_eq!(doc.to_string(), "hello **world**\n");
}

/// At whitespace there is no word to take, so the pair goes in and the caret parks between them,
/// ready to type into.
#[test]
fn a_caret_at_whitespace_inserts_the_pair_and_parks_inside() {
    let mut doc = at("hello  world\n", 6);
    assert!(doc.toggle_inline(Inline::Bold));
    assert_eq!(doc.to_string(), "hello **** world\n");
    assert_eq!(
        doc.selection().head(),
        ByteOffset::new(8),
        "the caret should be between the delimiters, ready to type"
    );
}

#[test]
fn each_construct_uses_its_own_delimiter() {
    for (what, expected) in [
        (Inline::Bold, "**word**"),
        (Inline::Italic, "*word*"),
        (Inline::Strikethrough, "~~word~~"),
        (Inline::Code, "`word`"),
    ] {
        let mut doc = selecting("word", 0, 4);
        doc.toggle_inline(what);
        assert_eq!(doc.to_string(), expected, "{what:?}");
    }
}

/// The caret goes inside the content, never between the delimiter pairs — the reveal rule dims
/// markers the caret touches, and parking between them shows the construct at its least readable
/// the moment it is made.
#[test]
fn wrapping_leaves_the_selection_on_the_content() {
    let mut doc = selecting("hello world\n", 0, 5);
    doc.toggle_inline(Inline::Bold);

    let text = doc.to_string();
    assert_eq!(doc.selection().span().slice(&text), "hello");
}

// ---- turning it off ----

#[test]
fn a_caret_inside_a_construct_removes_it() {
    let mut doc = at("**bold** text\n", 4);
    assert!(doc.toggle_inline(Inline::Bold));
    assert_eq!(doc.to_string(), "bold text\n");
}

/// The case every regex-based implementation gets wrong.
#[test]
fn unbolding_from_inside_a_nested_italic_touches_only_the_italic() {
    // `**a _b_ c**`, caret in `b`.
    let mut doc = at("**a _b_ c**\n", 5);
    assert!(doc.toggle_inline(Inline::Italic));
    assert_eq!(
        doc.to_string(),
        "**a b c**\n",
        "only the italic delimiters should have gone"
    );
}

/// And the other way round: the outer construct comes off without disturbing the inner one.
#[test]
fn removing_the_outer_construct_leaves_the_inner_alone() {
    let mut doc = at("**a _b_ c**\n", 5);
    assert!(doc.toggle_inline(Inline::Bold));
    assert_eq!(doc.to_string(), "a _b_ c\n");
}

/// The caret has to follow the text it was in, or the second press of a repeated toggle acts
/// somewhere else.
#[test]
fn the_caret_follows_the_content_when_a_construct_is_removed() {
    let mut doc = at("**bold**\n", 4);
    doc.toggle_inline(Inline::Bold);
    assert_eq!(doc.selection().head(), ByteOffset::new(2));
}

#[test]
fn a_construct_can_be_removed_from_either_edge_of_its_content() {
    for caret in [2, 6] {
        let mut doc = at("**bold**\n", caret);
        assert!(doc.toggle_inline(Inline::Bold), "caret at {caret}");
        assert_eq!(doc.to_string(), "bold\n", "caret at {caret}");
    }
}

// ---- code spans ----

#[test]
fn code_can_be_toggled_on_and_off_like_the_rest() {
    let mut doc = selecting("value\n", 0, 5);
    doc.toggle_inline(Inline::Code);
    assert_eq!(doc.to_string(), "`value`\n");

    doc.set_selection(Selection::caret(ByteOffset::new(3)));
    doc.toggle_inline(Inline::Code);
    assert_eq!(doc.to_string(), "value\n");
}

/// A toggle that cannot be un-toggled is not a toggle: `**` inside a code span is not markup, so a
/// second press could not find it to take away. See the module docs.
#[test]
fn bold_inside_a_code_span_does_nothing() {
    let mut doc = at("`some code`\n", 6);
    assert!(!doc.toggle_inline(Inline::Bold));
    assert_eq!(doc.to_string(), "`some code`\n");

    assert!(!doc.toggle_inline(Inline::Italic));
    assert!(!doc.toggle_inline(Inline::Strikethrough));
    assert_eq!(doc.to_string(), "`some code`\n");
}

// ---- undo ----

/// Two delimiters is two edits, and a single press must undo as a single press.
#[test]
fn a_toggle_is_one_undo_step() {
    let mut doc = selecting("hello world\n", 0, 5);
    doc.toggle_inline(Inline::Bold);
    assert_eq!(doc.to_string(), "**hello** world\n");

    assert!(doc.undo());
    assert_eq!(doc.to_string(), "hello world\n");
    assert!(!doc.undo(), "there should be nothing else to take back");
}

#[test]
fn removing_a_construct_is_also_one_undo_step() {
    let mut doc = at("**bold**\n", 4);
    doc.toggle_inline(Inline::Bold);

    assert!(doc.undo());
    assert_eq!(doc.to_string(), "**bold**\n");
    assert!(!doc.undo());
}

// ---- bytes ----

/// The property the whole module rests on.
#[test]
fn toggling_twice_restores_the_exact_bytes() {
    for text in [
        "hello world\n",
        "one *two* three\n",
        "a **b** c\n",
        "text with `code` in it\n",
    ] {
        for what in [
            Inline::Bold,
            Inline::Italic,
            Inline::Strikethrough,
            Inline::Code,
        ] {
            let mut doc = at(text, 1);
            doc.toggle_inline(what);
            doc.toggle_inline(what);
            assert_eq!(doc.to_string(), text, "{what:?} in {text:?}");
        }
    }
}

/// The segments have to be right afterwards, or the next toggle acts on a stale answer.
#[test]
fn the_segmentation_is_correct_after_a_toggle() {
    let mut doc = selecting("hello world\n", 0, 5);
    doc.toggle_inline(Inline::Bold);

    let text = doc.to_string();
    let fresh = Document::from(text.as_str());
    assert_eq!(doc.segments(), fresh.segments());
}

/// Nothing outside the construct is touched — the byte-preservation invariant, checked at the level
/// this module can break it.
#[test]
fn only_the_delimiters_change() {
    let original = "before hello after\n";
    let mut doc = selecting(original, 7, 12);
    doc.toggle_inline(Inline::Bold);

    let after = doc.to_string();
    assert!(after.starts_with("before "));
    assert!(after.ends_with(" after\n"));
    assert_eq!(after.len(), original.len() + 4);
}

proptest! {
    /// Toggling twice at any position in any of these documents restores it byte for byte.
    #[test]
    fn toggling_twice_is_the_identity(
        words in prop::collection::vec("[a-z]{1,6}", 1..12),
        caret in 0_usize..40,
        which in 0_usize..4,
    ) {
        let text = format!("{}\n", words.join(" "));
        let what = [Inline::Bold, Inline::Italic, Inline::Strikethrough, Inline::Code][which];

        let mut doc = Document::from(text.as_str());
        let at = ByteOffset::new(caret.min(text.len().saturating_sub(1)));
        doc.set_selection(Selection::caret(at));

        doc.toggle_inline(what);
        doc.toggle_inline(what);

        prop_assert_eq!(doc.to_string(), text);
    }

    /// And one undo takes back a toggle whatever it did.
    #[test]
    fn one_undo_takes_back_a_toggle(
        words in prop::collection::vec("[a-z]{1,6}", 1..12),
        caret in 0_usize..40,
    ) {
        let text = format!("{}\n", words.join(" "));
        let mut doc = Document::from(text.as_str());
        let at = ByteOffset::new(caret.min(text.len().saturating_sub(1)));
        doc.set_selection(Selection::caret(at));

        if doc.toggle_inline(Inline::Bold) {
            prop_assert!(doc.undo());
        }
        prop_assert_eq!(doc.to_string(), text);
    }
}

/// A selection that already spans the whole construct, delimiters included, is a case worth pinning:
/// it wraps rather than unwrapping, because the caret is not *inside* the construct.
#[test]
fn selecting_a_whole_construct_wraps_it_again() {
    let mut doc = selecting("**bold**\n", 0, 8);
    doc.toggle_inline(Inline::Italic);
    assert_eq!(doc.to_string(), "***bold***\n");
    assert_eq!(
        doc.to_string().len(),
        "**bold**\n".len() + 2,
        "one pair added, none removed"
    );
    let _ = ByteSpan::new(ByteOffset::ZERO, ByteOffset::ZERO);
}

// ---- links ----

#[test]
fn a_selection_becomes_a_link() {
    let mut doc = selecting("see the docs here\n", 4, 12);
    assert!(doc.make_link(Some("https://example.com")));
    assert_eq!(
        doc.to_string(),
        "see [the docs](https://example.com) here\n"
    );
}

/// With no destination the slot is left empty and the caret parks in it — the "no UI at all" case,
/// where the next thing typed is the destination.
#[test]
fn a_link_with_no_url_parks_the_caret_in_the_slot() {
    let mut doc = selecting("hello world\n", 0, 5);
    doc.make_link(None);
    assert_eq!(doc.to_string(), "[hello]() world\n");

    // Typing now should land inside the parentheses.
    doc.insert(doc.selection().head(), "https://x.example")
        .unwrap();
    assert_eq!(doc.to_string(), "[hello](https://x.example) world\n");
}

/// With a destination supplied there is nothing left to type, so the caret goes past the construct.
#[test]
fn a_link_with_a_url_leaves_the_caret_after_it() {
    let mut doc = selecting("hello world\n", 0, 5);
    doc.make_link(Some("https://x.example"));

    doc.insert(doc.selection().head(), "!").unwrap();
    assert_eq!(doc.to_string(), "[hello](https://x.example)! world\n");
}

#[test]
fn a_caret_mid_word_links_the_word() {
    let mut doc = at("see docs here\n", 5);
    doc.make_link(Some("https://x.example"));
    assert_eq!(doc.to_string(), "see [docs](https://x.example) here\n");
}

#[test]
fn making_a_link_is_one_undo_step() {
    let original = "hello world\n";
    let mut doc = selecting(original, 0, 5);
    doc.make_link(Some("https://x.example"));

    assert!(doc.undo());
    assert_eq!(doc.to_string(), original);
    assert!(!doc.undo());
}

/// A false positive turns a paste into a link nobody wanted, which is worse than a false negative —
/// so this is strict, and these are the cases that decide it.
#[test]
fn what_counts_as_a_url() {
    use mdedit_document::format::looks_like_url;

    assert!(looks_like_url("https://example.com"));
    assert!(looks_like_url("http://example.com/a/b?c=d"));
    assert!(looks_like_url("mailto:someone@example.com"));
    assert!(looks_like_url("  https://example.com  "), "trimmed first");

    assert!(!looks_like_url("example.com"), "a bare host is also a word");
    assert!(!looks_like_url("www.example.com"), "and so is this");
    assert!(!looks_like_url("some words here"));
    assert!(!looks_like_url("https://a b"), "a space rules it out");
    assert!(
        !looks_like_url("https://"),
        "a scheme with nothing after it"
    );
    assert!(!looks_like_url(""));
    assert!(!looks_like_url("see https://example.com for more"));
}
