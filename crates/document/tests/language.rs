//! The language boundary, exercised by a language that is not Markdown.
//!
//! Extracting a trait proves nothing on its own — a trait with one implementation is a rename. What
//! these check is that the *second* implementation is actually reachable: that a language which
//! never conceals gets everything revealed, and that one which reports no safe restart boundary gets
//! a whole-document parse instead of a wrong regional one.
//!
//! The stub is deliberately trivial. Its job is to be reached, not to be a language.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::Document;
use mdedit_language::{Analysis, Block, BlockKind, Language, Marker, Segment, SegmentKind};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};

/// A language that never conceals, as JSON and YAML will not.
///
/// One marker segment covering the first byte, so there is something whose concealment can be asked
/// about, and one content segment for the rest — which together tile, because that invariant is not
/// optional for any language.
#[derive(Debug)]
struct NeverConceals;

fn tiling(source: &str) -> Analysis {
    if source.is_empty() {
        return Analysis::default();
    }
    let first = source.chars().next().unwrap().len_utf8();
    let mut segments = vec![Segment::new(
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(first)),
        SegmentKind::Marker(Marker::Heading),
    )];
    if first < source.len() {
        segments.push(Segment::new(
            ByteSpan::new(ByteOffset::new(first), ByteOffset::new(source.len())),
            SegmentKind::Content,
        ));
    }
    Analysis {
        segments,
        blocks: vec![Block::new(
            BlockId::new(0),
            BlockKind::Paragraph,
            ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len())),
            0,
        )],
    }
}

impl Language for NeverConceals {
    fn name(&self) -> &'static str {
        "Never"
    }

    fn analyze(&self, source: &str) -> Analysis {
        tiling(source)
    }

    fn analyze_region(&self, _source: &str, _region: ByteSpan) -> Option<Analysis> {
        // No safe restart boundary, which is the answer JSON will give inside a deep object.
        None
    }

    fn conceals(&self) -> bool {
        false
    }
}

static NEVER: NeverConceals = NeverConceals;

/// The colour-only rendering model, which is the whole reason the boundary is shaped this way.
///
/// A marker segment in a concealing language is hidden when the caret is elsewhere. In a
/// non-concealing one it never is — JSON's braces *are* the document.
#[test]
fn a_language_that_never_conceals_reveals_every_segment() {
    let source = "# Title\n\nbody text\n";

    let mut markdown = Document::from(source);
    // Away from the heading, or its `#` is revealed by the caret touching it and the control case
    // conceals nothing.
    markdown.set_selection(mdedit_document::Selection::caret(ByteOffset::new(
        source.find("body").unwrap(),
    )));
    let concealed = markdown
        .segments()
        .iter()
        .filter(|segment| !markdown.is_revealed(segment, markdown.selection()))
        .count();
    assert!(
        concealed > 0,
        "the Markdown case has to conceal something, or this proves nothing"
    );

    let mut other = Document::in_language(&NEVER);
    other.insert(ByteOffset::ZERO, source).unwrap();
    assert!(
        other
            .segments()
            .iter()
            .all(|segment| other.is_revealed(segment, other.selection())),
        "a language that does not conceal must reveal everything, wherever the caret is"
    );
}

/// The tiling invariant is the gate, and it is the same gate for every language.
#[test]
fn the_segments_tile_whatever_the_language() {
    let source = "some text in another language\n";
    let mut document = Document::in_language(&NEVER);
    document.insert(ByteOffset::ZERO, source).unwrap();

    let text = document.to_string();
    let rebuilt: String = document
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, text, "the segments must reproduce the source");
}

/// A language with no safe restart boundary gets a whole-document parse: correct, merely slower.
///
/// The document has to be over the 16 KB threshold, or the scoped path is not taken at all and this
/// would pass without reaching what it is about.
#[test]
fn no_restart_boundary_falls_back_to_a_whole_parse() {
    let big = "x".repeat(32 * 1024);
    let mut document = Document::in_language(&NEVER);
    document.insert(ByteOffset::ZERO, &big).unwrap();

    document.insert(ByteOffset::new(9), "y").unwrap();

    assert!(
        !document.is_scoped(),
        "a language answering `None` must have been given a whole-document parse"
    );
    let text = document.to_string();
    let rebuilt: String = document
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, text, "and the result still tiles");
}

/// Markdown is one implementation of the trait, and says what it is.
#[test]
fn markdown_is_a_language_like_any_other() {
    let document = Document::from("# Title\n");
    assert_eq!(document.language().name(), "Markdown");
    assert!(
        document.language().conceals(),
        "Markdown conceals; that is the editor's entire premise"
    );
    assert!(
        document
            .language()
            .analyze_region(
                "# Title\n",
                ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(8))
            )
            .is_some(),
        "a top-level block boundary is a safe restart, which is what MDEDIT-18 rests on"
    );
}
