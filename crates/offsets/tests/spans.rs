//! The span predicates, which the reveal rule depends on being different from each other.

use mdedit_offsets::{ByteOffset, ByteSpan};

fn span(start: usize, end: usize) -> ByteSpan {
    ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end))
}

#[test]
fn slices_the_bytes_it_addresses() {
    assert_eq!(span(2, 6).slice("**bold**"), "bold");
    assert_eq!(span(0, 2).slice("**bold**"), "**");
    assert_eq!(span(4, 4).slice("**bold**"), "");
}

#[test]
fn overlap_requires_a_shared_byte() {
    assert!(span(0, 4).overlaps(span(2, 6)));
    assert!(span(2, 6).overlaps(span(0, 4)));

    // Adjacent, not overlapping: `0..2` ends where `2..4` begins.
    assert!(!span(0, 2).overlaps(span(2, 4)));
    assert!(!span(0, 2).overlaps(span(4, 6)));
}

#[test]
fn an_empty_span_overlaps_nothing_but_touches_its_neighbours() {
    let caret = span(2, 2);
    assert!(!caret.overlaps(span(0, 4)));
    assert!(caret.touches(span(0, 2)));
    assert!(caret.touches(span(2, 4)));
    assert!(!caret.touches(span(3, 4)));
}

#[test]
fn touching_admits_the_edges_overlap_excludes() {
    assert!(span(0, 2).touches(span(2, 4)));
    assert!(!span(0, 2).touches(span(3, 4)));
}

#[test]
fn length_of_an_inverted_span_saturates_rather_than_wrapping() {
    // overflow-checks is on in release, so a wrap here would panic in production but not in tests.
    assert_eq!(span(6, 2).len(), 0);
    assert!(span(6, 2).is_empty());
}
