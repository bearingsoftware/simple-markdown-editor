//! Fenced-code highlighting (MDEDIT-190), which is the one parser here that takes a fence's body from a
//! document and runs a regex engine over it.
//!
//! The body is untrusted file contents, and `fancy-regex` backtracks: one long line can cost far more
//! than its length suggests, which is why `highlight` caps bytes and gives up on a line. The fuzzer is
//! here for the other failure: `parse_line`'s error arm is never reached by a unit test, because no
//! known input reaches it. When it does, `highlight` stops and the rest of the fence draws plain, so the
//! only way to see a span that is wrong rather than merely absent is to generate the input and look.
//!
//! Same UTF-8 gate as the lexers. The input is split at its first newline: the first line is the info
//! string and the rest is the body. With no newline the whole input is the info string and the body is
//! empty, which exercises the "nothing to highlight" path.
//!
//! The assertions are the ones the editor relies on when it draws the spans over a buffer: sorted and
//! non-overlapping, non-empty, inside the body, and on character boundaries. `None` is a valid answer.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let (info, code) = source.split_once('\n').unwrap_or((source, ""));

    let Some(spans) = mdedit_highlight::highlight(info, code) else {
        return;
    };

    let mut previous_end = 0;
    for (span, _) in &spans {
        let (start, end) = (span.start().get(), span.end().get());
        assert!(start >= previous_end, "spans are unsorted or overlap");
        assert!(start < end, "an empty span");
        assert!(end <= code.len(), "a span past the end of the body");
        assert!(
            code.is_char_boundary(start),
            "a span starts inside a character"
        );
        assert!(code.is_char_boundary(end), "a span ends inside a character");
        previous_end = end;
    }
});
