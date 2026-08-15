//! The XML lexer, which is hand-written for the reason the crate docs evidence.
//!
//! Same gate as every other language here: **the segments tile the source**, plus termination, which
//! is not free in a lexer whose word walk has a `max(at + 1)` in it precisely because one that can
//! return the position it was given does not stop.
//!
//! The input this target is worth having on is the one a real parser refuses — an unclosed tag, an
//! unquoted attribute, a stray angle bracket — which is near enough what a fuzzer produces anyway.
//! It also reaches the nesting counter, which exists because `<a><a><a>…` is a file from the internet
//! opened in an editor and safe Rust does not save you from a stack overflow.

#![no_main]

use libfuzzer_sys::fuzz_target;

use mdedit_language::Language as _;

// Raw bytes with a UTF-8 gate rather than an `Arbitrary` `&str`, as `yaml_analyze` explains.
fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let analysis = mdedit_xml::Xml.analyze(source);

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
});
