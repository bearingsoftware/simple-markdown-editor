//! Byte-level classification behind [`crate::segment`].
//!
//! Works on a per-byte scratch array rather than by merging ranges, because tiling then holds by
//! construction: every byte is written at most once per pass and read exactly once at the end.

use std::ops::Range;

use mdedit_offsets::{ByteOffset, ByteSpan};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

use crate::{Marker, Segment, SegmentKind};

// Ranges inside this module are raw `usize`, deliberately. There is exactly one coordinate space
// here — offsets into `source`, as `pulldown-cmark` reports them — and the scratch array is indexed
// by them directly. Typing them would add conversions at every index without separating anything.
// The boundary is `coalesce`, where offsets become the `ByteSpan`s the crate hands out.

/// Extensions we classify. GFM tables, footnotes and task lists are deliberately absent — they are
/// out of scope until the renderer handles them, and degrade to content.
fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH
}

pub(crate) fn segment(source: &str) -> Vec<Segment> {
    let mut marks: Vec<Option<Marker>> = vec![None; source.len()];

    let Walk {
        mut constructs,
        contents,
        verbatim,
    } = walk(source);

    // Outermost first so a nested construct overwrites its parent's classification: in `**a *b* c**`
    // the inner `*` must read as Emphasis, not Strong.
    constructs.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
    for (range, marker) in constructs {
        fill(&mut marks, &range, Some(marker));
    }

    for range in &contents {
        fill(&mut marks, range, None);
    }

    normalise_line_syntax(source, &mut marks);
    mark_escapes(source, verbatim, &mut marks);

    coalesce(source, &marks)
}

struct Walk {
    constructs: Vec<(Range<usize>, Marker)>,
    contents: Vec<Range<usize>>,
    /// Regions where a backslash is a literal character rather than an escape.
    verbatim: Vec<Range<usize>>,
}

/// Collects construct ranges and the content ranges that will be subtracted from them.
fn walk(source: &str) -> Walk {
    // Reserving up front matters more than it looks: on a 310 KiB document these reach five figures,
    // and growth reallocation made overhead scale superlinearly with document size.
    let estimate = source.len() / 24 + 16;
    let mut constructs = Vec::with_capacity(estimate);
    let mut contents = Vec::with_capacity(estimate);
    let mut verbatim = Vec::new();

    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if matches!(tag, Tag::CodeBlock(_)) {
                    verbatim.push(range.clone());
                }
                if let Some(marker) = marker_for(&tag) {
                    constructs.push((range, marker));
                }
            }
            Event::Rule => constructs.push((range, Marker::ThematicBreak)),
            Event::HardBreak => constructs.push((range, Marker::HardBreak)),

            // An inline code span reports one range covering delimiters *and* content, with no child
            // event to subtract, so the backtick runs have to be measured directly.
            Event::Code(_) => {
                let ticks = backtick_run(&source[range.clone()]);
                constructs.push((range.clone(), Marker::CodeSpan));
                verbatim.push(range.clone());
                let inner = range.start + ticks..range.end.saturating_sub(ticks);
                if inner.start < inner.end {
                    contents.push(inner);
                }
            }

            Event::Text(_) | Event::Html(_) | Event::InlineHtml(_) | Event::SoftBreak => {
                contents.push(range);
            }

            // Everything else contributes neither syntax nor content of its own.
            Event::End(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_) => {}
        }
    }

    Walk {
        constructs,
        contents,
        verbatim,
    }
}

fn marker_for(tag: &Tag<'_>) -> Option<Marker> {
    Some(match tag {
        Tag::Emphasis => Marker::Emphasis,
        Tag::Strong => Marker::Strong,
        Tag::Strikethrough => Marker::Strikethrough,
        Tag::Heading { .. } => Marker::Heading,
        Tag::BlockQuote(_) => Marker::BlockQuote,
        Tag::Item => Marker::ListItem,
        Tag::CodeBlock(CodeBlockKind::Fenced(_) | CodeBlockKind::Indented) => Marker::CodeBlock,
        Tag::Link { .. } => Marker::Link,
        Tag::Image { .. } => Marker::Image,
        _ => return None,
    })
}

/// Number of leading backticks in an inline code span.
fn backtick_run(text: &str) -> usize {
    text.bytes().take_while(|b| *b == b'`').count()
}

/// One left-to-right pass applying the three line-oriented corrections.
///
/// Each is described where it is applied below. They were three separate walks; fused because they
/// are all line-aware and each reads only positions at or before the one it writes, so a single pass
/// gives identical results for a third of the memory traffic.
fn normalise_line_syntax(source: &str, marks: &mut [Option<Marker>]) {
    let bytes = source.as_bytes();
    let mut in_prefix = true;
    let mut content_started = false;

    for i in 0..bytes.len() {
        let byte = bytes[i];

        // A newline is never syntax to conceal — concealing one would join two lines.
        if byte == b'\n' || byte == b'\r' {
            marks[i] = None;
            in_prefix = true;
            content_started = false;
            continue;
        }

        if marks[i].is_none() {
            in_prefix = false;
            content_started = true;
            continue;
        }

        // A `>` in a line's opening syntax run is always block-quote syntax. Range containment
        // cannot express this: a list item legitimately spans the continuation lines of a quote
        // nested inside it, so in `> > 1. one\n>>\n>>   two` the item's range covers the `>>`
        // prefixes and, starting later, is filled last and wins bytes that are the quote's.
        //
        // Confined to the prefix so it cannot touch the `>` closing an autolink — content precedes
        // that one, and `in_prefix` is already false.
        if in_prefix && byte == b'>' {
            marks[i] = Some(Marker::BlockQuote);
        }

        if matches!(marks[i], Some(Marker::BlockQuote | Marker::ListItem)) {
            // Subtracting content from a construct's range assumes the parser reports content for
            // everything that is not syntax — but it emits no events at all for a link reference
            // definition, so `- item\n\n  [ref]: /url` leaves the definition inside the item's range
            // and unclaimed, and it would be concealed as list syntax.
            //
            // A quote or list marker is only ever drawn from `>`, a bullet, digits and whitespace,
            // and only ever appears before the line's content — the space in `[ref]: /url` is not a
            // list indent. Anything else degrades to content with its bytes intact.
            let plausible = byte == b'>'
                || byte.is_ascii_whitespace()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'*' | b'+' | b'.' | b')');

            if content_started || !plausible {
                marks[i] = None;
                in_prefix = false;
                content_started = true;
            }
        }
    }
}

/// Marks the backslash of a backslash escape.
///
/// The parser reports no event for one, and it can open a block (`\\> not a quote`) so it cannot be
/// found by looking between adjacent content runs. Scanned directly instead, skipping code spans and
/// code blocks where a backslash is a literal character.
fn mark_escapes(source: &str, mut verbatim: Vec<Range<usize>>, marks: &mut [Option<Marker>]) {
    // Code spans and code blocks never nest, so the ranges are disjoint and a merge walk suffices —
    // previously a byte-per-source-byte bitmap.
    verbatim.sort_unstable_by_key(|r| r.start);
    let mut next = 0;

    let bytes = source.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        while next < verbatim.len() && verbatim[next].end <= i {
            next += 1;
        }
        let literal = verbatim.get(next).is_some_and(|r| r.contains(&i));

        // CommonMark escapes ASCII punctuation only; a backslash before anything else is literal.
        // The escaped character must also be content: an escape inside a link destination or title
        // is already concealed as part of that construct, and splitting the run would gain nothing.
        if bytes[i] == b'\\'
            && bytes[i + 1].is_ascii_punctuation()
            && !literal
            && marks[i + 1].is_none()
        {
            marks[i] = Some(Marker::Escape);
            i += 2; // the escaped character is content and cannot itself open an escape
        } else {
            i += 1;
        }
    }
}

fn fill(marks: &mut [Option<Marker>], range: &Range<usize>, value: Option<Marker>) {
    let end = range.end.min(marks.len());
    if range.start < end {
        marks[range.start..end].fill(value);
    }
}

/// Groups equal-classification bytes into segments, stepping by character so a returned range can
/// never split a multi-byte character.
fn coalesce(source: &str, marks: &[Option<Marker>]) -> Vec<Segment> {
    // Prose alternates long content runs with short markers; this is a floor, not a bound.
    let mut out: Vec<Segment> = Vec::with_capacity(source.len() / 32 + 8);

    for (offset, ch) in source.char_indices() {
        let kind = match marks[offset] {
            Some(marker) => SegmentKind::Marker(marker),
            None => SegmentKind::Content,
        };
        let end = ByteOffset::new(offset + ch.len_utf8());
        match out.last_mut() {
            Some(last) if last.kind == kind => last.span = ByteSpan::new(last.span.start(), end),
            _ => out.push(Segment {
                span: ByteSpan::new(ByteOffset::new(offset), end),
                kind,
            }),
        }
    }

    out
}
