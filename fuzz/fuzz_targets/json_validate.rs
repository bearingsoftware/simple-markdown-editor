//! JSON's strict pass, which is a hand-rolled scanner over a file from somewhere.
//!
//! `AGENTS.md` names hand-rolled file parsing as the CVE class this project must not reintroduce —
//! the 2023 Notepad++ heap overflows were all in one. Rust removes the memory-unsafety half; what is
//! left is panics, non-termination, and spans that point at bytes that are not there. All three are
//! reachable from a file a user opened, and all three are what this looks for.
//!
//! Two properties beyond "does not panic" are asserted, because both are load-bearing:
//!
//! - **The 200 cap holds.** Nobody is fixing the 201st diagnostic, and the renderer scans the whole
//!   list per chunk on the strength of it being short.
//! - **Every span is real and on a character boundary.** Tools → Validate Document puts the caret on
//!   one and the renderer draws a squiggle under it, so a span past the end or through the middle of
//!   a character is a panic in the window rather than a wrong colour.

#![no_main]

use libfuzzer_sys::fuzz_target;

// Raw bytes with a UTF-8 gate rather than an `Arbitrary` `&str`: a corpus file is then literally
// a document, so real ones can be dropped in — and invalid UTF-8 cannot reach these functions in
// the editor anyway, because the file layer has already decoded.
fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let diagnostics = mdedit_json::validate(source);

    assert!(
        diagnostics.len() <= mdedit_json::LIMIT,
        "the diagnostic cap is a promise the renderer relies on: {} of them",
        diagnostics.len()
    );

    for diagnostic in &diagnostics {
        let span = diagnostic.span;
        assert!(span.start() <= span.end(), "a backwards span");
        assert!(
            span.end().get() <= source.len(),
            "a span past the end of the document"
        );
        assert!(source.is_char_boundary(span.start().get()));
        assert!(source.is_char_boundary(span.end().get()));
        // What the editor actually does with it, and the operation that would panic.
        let _ = span.slice(source);
    }
});
