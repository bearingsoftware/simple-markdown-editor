//! The TOML lexer, and the region parse it claims is safe.
//!
//! Same gate as every other language here: **the segments tile the source**, plus termination, which
//! is not free in a lexer whose word walk has a `max(at + 1)` in it precisely because one that can
//! return the position it was given does not stop.
//!
//! It asserts one thing the others cannot. TOML answers `analyze_region`, and its whole claim is that
//! **a block is a safe restart point** — a column-zero line outside every multi-line string with no
//! array open. So this restarts at every block the analysis reports and checks the result against the
//! same bytes of the whole-document parse. A generated document is exactly where that claim breaks if
//! it is going to, and getting it wrong points a scoped re-parse at the wrong bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;

use mdedit_language::Language as _;
use mdedit_offsets::{ByteOffset, ByteSpan};

// Raw bytes with a UTF-8 gate rather than an `Arbitrary` `&str`, as `yaml_analyze` explains.
fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let analysis = mdedit_toml::Toml.analyze(source);

    let mut at = 0;
    for segment in &analysis.segments {
        let span = segment.span();
        assert_eq!(span.start().get(), at, "a gap or an overlap between spans");
        assert!(span.end().get() > span.start().get(), "an empty segment");
        assert!(source.is_char_boundary(span.start().get()));
        assert!(source.is_char_boundary(span.end().get()));
        at = span.end().get();
    }
    assert_eq!(at, source.len(), "the segments did not cover the source");

    for block in &analysis.blocks {
        let span = block.span();
        assert!(span.start() <= span.end(), "a backwards block");
        assert!(span.end().get() <= source.len(), "a block past the end");
        assert!(source.is_char_boundary(span.start().get()));
        assert!(source.is_char_boundary(span.end().get()));
    }

    for block in &analysis.blocks {
        let start = block.span().start().get();
        let region = mdedit_toml::analyze_region(
            source,
            ByteSpan::new(ByteOffset::new(start), ByteOffset::new(source.len())),
        );
        let tail: Vec<_> = analysis
            .segments
            .iter()
            .filter(|segment| segment.span().start().get() >= start)
            .cloned()
            .collect();
        assert_eq!(
            region.segments, tail,
            "restarting at block {start} disagreed with the whole-document parse"
        );
    }
});
