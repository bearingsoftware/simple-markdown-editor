//! The marker locator, against the gate the whole editor rests on.
//!
//! `AGENTS.md`: "concatenating all marker and content spans, in order, reproduces the source
//! byte-for-byte. If that test is red, nothing else in the editor is trustworthy." It is
//! property-tested against the CommonMark spec suite already; what a fuzzer adds is the input nobody
//! wrote a spec example for — and this is the component the research names as the highest-risk one in
//! the project, computed rather than parsed because no Rust Markdown parser emits delimiter nodes.

#![no_main]

use libfuzzer_sys::fuzz_target;

// Raw bytes with a UTF-8 gate rather than an `Arbitrary` `&str`: a corpus file is then literally
// a document, so real ones can be dropped in — and invalid UTF-8 cannot reach these functions in
// the editor anyway, because the file layer has already decoded.
fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let segments = mdedit_markers::segment(source);

    // The tiling invariant, stated exactly as the property test states it.
    let rebuilt: String = segments
        .iter()
        .map(|segment| segment.span().slice(source))
        .collect();
    assert_eq!(rebuilt, source, "the segments lost or duplicated bytes");

    // And the shape that makes the tiling meaningful: spans in order, none empty, none overlapping.
    // A run of empty segments would tile perfectly and say nothing.
    let mut at = 0;
    for segment in &segments {
        let span = segment.span();
        assert_eq!(span.start().get(), at, "a gap or an overlap between spans");
        assert!(span.end().get() > span.start().get(), "an empty segment");
        assert!(source.is_char_boundary(span.start().get()));
        assert!(source.is_char_boundary(span.end().get()));
        at = span.end().get();
    }
    assert_eq!(at, source.len(), "the segments stopped short of the end");
});
