//! The YAML lexer, which is hand-written for reasons the crate docs evidence.
//!
//! A real parser was rejected because it refuses the documents this has to survive — a half-typed
//! flow sequence, inconsistent indentation, a tab. Those are what a file looks like while somebody is
//! writing it, and they are also, near enough, what a fuzzer produces. So this is the crate where the
//! fuzzer's inputs and the crate's stated purpose line up most exactly.
//!
//! Same gate as every other language here: **the segments tile the source**. It mis-colours rather
//! than mis-covers, deliberately, so tiling is what there is to assert — plus termination, which is
//! not free in a lexer whose token walk has a `max(at + 1)` in it precisely because a lexer that can
//! return the position it was given does not stop.

#![no_main]

use libfuzzer_sys::fuzz_target;

use mdedit_language::Language as _;

// Raw bytes with a UTF-8 gate rather than an `Arbitrary` `&str`: a corpus file is then literally
// a document, so real ones can be dropped in — and invalid UTF-8 cannot reach these functions in
// the editor anyway, because the file layer has already decoded.
fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let analysis = mdedit_yaml::Yaml.analyze(source);

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

    // Blocks are what `scope_for` restarts a region parse at, so a block span that is not real is a
    // scoped re-parse pointed at the wrong bytes — MDEDIT-97's failure mode, one crate down.
    for block in &analysis.blocks {
        let span = block.span();
        assert!(span.start() <= span.end(), "a backwards block");
        assert!(span.end().get() <= source.len(), "a block past the end");
        assert!(source.is_char_boundary(span.start().get()));
        assert!(source.is_char_boundary(span.end().get()));
    }
});
