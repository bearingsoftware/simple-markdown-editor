//! Byte-level classification behind [`crate::segment`].
//!
//! Works on a per-byte scratch array rather than by merging ranges, because tiling then holds by
//! construction: every byte is written at most once per pass and read exactly once at the end.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

use crate::{Marker, Segment, SegmentKind};

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

    // A newline is never syntax to conceal — concealing one would join two lines.
    for (i, byte) in source.bytes().enumerate() {
        if byte == b'\n' || byte == b'\r' {
            marks[i] = None;
        }
    }

    reattribute_line_prefixes(source, &mut marks);
    release_implausible_container_syntax(source, &mut marks);
    mark_escapes(source, &verbatim, &mut marks);

    coalesce(source, &marks)
}

/// Gives each `>` in a line's opening syntax run back to [`Marker::BlockQuote`].
///
/// Range containment cannot express this. A list item legitimately spans the continuation lines of a
/// quote nested inside it, so in `> > 1. one\n>>\n>>   two` the item's range covers the `>>`
/// prefixes — and because the item starts later than the quotes, it is filled last and wins bytes
/// that are quote syntax.
///
/// Confined to the leading run of syntax on each line, so it cannot touch a `>` that closes an
/// autolink: there, content precedes the `>` and the scan has already stopped.
fn reattribute_line_prefixes(source: &str, marks: &mut [Option<Marker>]) {
    let bytes = source.as_bytes();
    let line_starts = std::iter::once(0).chain(
        bytes
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .map(|(i, _)| i + 1),
    );

    for start in line_starts {
        for (i, byte) in bytes.iter().enumerate().skip(start) {
            if marks[i].is_none() {
                break;
            }
            if *byte == b'>' {
                marks[i] = Some(Marker::BlockQuote);
            }
        }
    }
}

struct Walk {
    constructs: Vec<(Range<usize>, Marker)>,
    contents: Vec<Range<usize>>,
    /// Regions where a backslash is a literal character rather than an escape.
    verbatim: Vec<Range<usize>>,
}

/// Collects construct ranges and the content ranges that will be subtracted from them.
fn walk(source: &str) -> Walk {
    let mut constructs = Vec::new();
    let mut contents = Vec::new();
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

/// Returns bytes a block container claimed but cannot own to [`SegmentKind::Content`].
///
/// Subtracting content from a construct's range assumes the parser reports content for everything
/// that is not syntax — but it emits no events at all for a link reference definition, so
/// `- item\n\n  [ref]: /url` leaves the definition inside the item's range and unclaimed, and it
/// would be concealed as list syntax.
///
/// A quote or list marker is only ever drawn from `>`, a bullet, digits and whitespace. Anything
/// else was never this construct's syntax, so it degrades to content with its bytes intact.
fn release_implausible_container_syntax(source: &str, marks: &mut [Option<Marker>]) {
    let bytes = source.as_bytes();
    let mut content_started = false;

    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            content_started = false;
            continue;
        }
        if !matches!(marks[i], Some(Marker::BlockQuote | Marker::ListItem)) {
            content_started |= marks[i].is_none();
            continue;
        }

        let plausible = *byte == b'>'
            || byte.is_ascii_whitespace()
            || byte.is_ascii_digit()
            || matches!(byte, b'-' | b'*' | b'+' | b'.' | b')');

        // Container syntax is always a line prefix, so anything after this line's content began is
        // content too — the space in `[ref]: /url` is not a list indent.
        if content_started || !plausible {
            marks[i] = None;
            content_started = true;
        }
    }
}

/// Marks the backslash of a backslash escape.
///
/// The parser reports no event for one, and it can open a block (`\\> not a quote`) so it cannot be
/// found by looking between adjacent content runs. Scanned directly instead, skipping code spans and
/// code blocks where a backslash is a literal character.
fn mark_escapes(source: &str, verbatim: &[Range<usize>], marks: &mut [Option<Marker>]) {
    let mut literal = vec![false; source.len()];
    for range in verbatim {
        let end = range.end.min(literal.len());
        if range.start < end {
            literal[range.start..end].fill(true);
        }
    }

    let bytes = source.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        // CommonMark escapes ASCII punctuation only; a backslash before anything else is literal.
        // The escaped character must also be content: an escape inside a link destination or title
        // is already concealed as part of that construct, and splitting the run would gain nothing.
        if bytes[i] == b'\\'
            && bytes[i + 1].is_ascii_punctuation()
            && !literal[i]
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
    let mut out: Vec<Segment> = Vec::new();

    for (offset, ch) in source.char_indices() {
        let kind = match marks[offset] {
            Some(marker) => SegmentKind::Marker(marker),
            None => SegmentKind::Content,
        };
        match out.last_mut() {
            Some(last) if last.kind == kind => last.range.end = offset + ch.len_utf8(),
            _ => out.push(Segment {
                range: offset..offset + ch.len_utf8(),
                kind,
            }),
        }
    }

    out
}
