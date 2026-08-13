//! Byte-level classification behind [`crate::segment`].
//!
//! Works on a per-byte scratch array rather than by merging ranges, because tiling then holds by
//! construction: every byte is written at most once per pass and read exactly once at the end.

use std::ops::Range;

use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::{Analysis, Block, BlockKind, Marker, Segment, SegmentKind};

// Ranges inside this module are raw `usize`, deliberately. There is exactly one coordinate space
// here — offsets into `source`, as `pulldown-cmark` reports them — and the scratch array is indexed
// by them directly. Typing them would add conversions at every index without separating anything.
// The boundary is `coalesce`, where offsets become the `ByteSpan`s the crate hands out.

/// Extensions we classify.
///
/// **Task lists are on because the editor's own button writes them** (MDEDIT-88). The formatting bar
/// has inserted `- [ ] ` since MDEDIT-52 while this line said the parser should not read it, so the
/// write path and the read path disagreed about the language and the editor rendered its own output
/// as a bullet followed by a literal pair of brackets.
///
/// Footnotes and definition lists are MDEDIT-90: each is a construct people write by hand and want
/// to see *recognised*, and colouring one costs a flag and a marker where rendering it would cost a
/// widget. **Colour only, no authoring** — nothing writes either of them.
///
/// # What is deliberately off, having been measured rather than assumed
///
/// - **`ENABLE_HEADING_ATTRIBUTES`**, which MDEDIT-90 set out to take and which **loses a line**.
///   It mishandles a lone `\r` as a terminator, so a heading followed by a CR-terminated line
///   swallows that line:
///
///   ```text
///   "# a\r$"   off: Heading 0..4, then a paragraph "$"
///              on:  Heading 0..5, its text event "a\r$" — the next line is inside the heading
///   "#\u{b}\r$" on:  Heading 0..4 with no text event at all, so `$` is *marker* and is concealed
///   ```
///
///   `\n` and `\r\n` are unaffected; only a bare CR. That is not a corner this editor can wave
///   away — it draws lone CRs (MDEDIT-46) and converts them only when asked (MDEDIT-59), precisely
///   because files arrive with them. A flag that makes a character vanish from the rendered line is
///   the MDEDIT-112 class, and this one is worse: it takes the whole line.
///
///   Found by the property test at 200,000 cases, minimised to five characters. Reported as
///   [pulldown-cmark#1132](https://github.com/pulldown-cmark/pulldown-cmark/issues/1132).
/// - ~~**GFM tables**, until there is a block widget to put one in.~~ **Taken by MDEDIT-126, which
///   is that widget.** The line above was right and is worth keeping the reason for: MDEDIT-89
///   proposed turning the flag on for colour alone, and could not, because MDEDIT-118 made a marker
///   something that is never drawn — so classifying the pipes as syntax without a grid to replace
///   them with *erases* the columns rather than colouring them. The flag and the renderer had to
///   arrive together, and this is where they do.
/// - **`ENABLE_GFM`**, which is *not* what its name suggests and not what MDEDIT-90 expected: in
///   0.13 it enables blockquote alert tags (`> [!NOTE]`) and nothing else — **no bare-URL
///   autolinks**. This editor does not render a callout, and turning it on changes no classification
///   at all: with it off, `[!NOTE]` is the quote's content; with it on, it lands inside the quote's
///   marker range and `normalise_line_syntax` hands it straight back to content. A flag that costs
///   parse time and changes nothing is not worth taking.
/// - **`ENABLE_SUBSCRIPT`**, because it **reinterprets documents that already exist**: with it on,
///   `~single~` is a subscript where today it is strikethrough. Silently changing what a file means
///   is not a highlighting feature.
/// - **`ENABLE_SUPERSCRIPT`**, which produced no event at all on `x^2^` — whatever it wants, it is
///   not the syntax anybody writes.
/// - **Smart punctuation**, which rewrites `--` and `"` in the *parser's* output. Nothing here reads
///   the text of an event, so it would change nothing — but it is exactly the kind of flag that
///   would one day quietly break byte preservation, and it is named here so nobody adds it casually.
fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_TABLES
}

pub(crate) fn analyze(source: &str) -> Analysis {
    let mut marks: Vec<Option<Marker>> = vec![None; source.len()];

    let Walk {
        mut constructs,
        contents,
        verbatim,
        blocks,
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

    Analysis {
        segments: coalesce(source, &marks),
        blocks,
    }
}

struct Walk {
    constructs: Vec<(Range<usize>, Marker)>,
    contents: Vec<Range<usize>>,
    /// Regions where a backslash is a literal character rather than an escape.
    verbatim: Vec<Range<usize>>,
    /// Block structure, outermost first, in document order.
    blocks: Vec<Block>,
}

/// The block kind a start tag opens, if it opens one at all.
fn block_kind(tag: &Tag<'_>) -> Option<BlockKind> {
    Some(match tag {
        Tag::Paragraph => BlockKind::Paragraph,
        Tag::Heading { level, .. } => {
            // `HeadingLevel` is 1-6 by construction, so the cast cannot be wrong; `as u8` on the
            // enum would depend on its discriminants, which is not something to rely on.
            let level = match level {
                pulldown_cmark::HeadingLevel::H1 => 1,
                pulldown_cmark::HeadingLevel::H2 => 2,
                pulldown_cmark::HeadingLevel::H3 => 3,
                pulldown_cmark::HeadingLevel::H4 => 4,
                pulldown_cmark::HeadingLevel::H5 => 5,
                pulldown_cmark::HeadingLevel::H6 => 6,
            };
            BlockKind::Heading(level)
        }
        Tag::CodeBlock(_) => BlockKind::CodeBlock,
        Tag::BlockQuote(_) => BlockKind::BlockQuote,
        Tag::List(_) => BlockKind::List,
        Tag::Item => BlockKind::ListItem,
        Tag::HtmlBlock => BlockKind::Html,
        // MDEDIT-126. Rows and cells are blocks because the grid is built from them: a column's
        // width is the widest cell in it, which cannot be asked of a single span covering the whole
        // table. They nest, so the depth counter has to see all three — see `closes_block`.
        Tag::Table(_) => BlockKind::Table,
        Tag::TableHead | Tag::TableRow => BlockKind::TableRow,
        Tag::TableCell => BlockKind::TableCell,
        Tag::FootnoteDefinition(_) => BlockKind::Other,
        // MDEDIT-90. `Other` rather than reusing `List`: nothing acts on the distinction, and
        // `BlockKind` grows a variant when a renderer needs a case rather than before.
        Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
            BlockKind::Other
        }
        // Everything else is inline and opens no block.
        _ => return None,
    })
}

/// Whether an end tag closes a block, so nesting depth can be tracked without a stack.
fn closes_block(tag: TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::CodeBlock
            | TagEnd::BlockQuote(_)
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::Table
            // Both opened a block above, so both have to close one or the depth counter drifts and
            // every block after the table reports as nested inside it.
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
    )
}

/// Collects construct ranges and the content ranges that will be subtracted from them.
fn walk(source: &str) -> Walk {
    // Reserving up front matters more than it looks: on a 310 KiB document these reach five figures,
    // and growth reallocation made overhead scale superlinearly with document size.
    let estimate = source.len() / 24 + 16;
    let mut constructs = Vec::with_capacity(estimate);
    let mut contents = Vec::with_capacity(estimate);
    let mut verbatim = Vec::new();

    let mut blocks: Vec<Block> = Vec::new();
    let mut depth: u16 = 0;

    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if matches!(tag, Tag::CodeBlock(_)) {
                    verbatim.push(range.clone());
                }
                if let Some(kind) = block_kind(&tag) {
                    // Pushed on the way in, so the list is already outermost-first in document
                    // order and needs no sort.
                    blocks.push(Block::new(
                        BlockId::new(u32::try_from(blocks.len()).unwrap_or(u32::MAX)),
                        kind,
                        ByteSpan::from(range.clone()),
                        depth,
                    ));
                    depth = depth.saturating_add(1);
                }
                // **A footnote definition's marker is its label, not the whole note** (MDEDIT-90).
                // The tag's range covers the note's body too, and the parser reports no content
                // event for a body that is itself a link reference definition — so subtracting
                // content left `[^*]:[a]:\n[` as one marker, the whole rest of the document
                // concealed. Measured directly, as `Event::Code`'s backtick run is and for the same
                // reason: the range covers delimiters and content with no child event between them.
                if matches!(tag, Tag::FootnoteDefinition(_)) {
                    constructs.push((range.start..label_end(source, &range), Marker::Footnote));
                } else if let Some(marker) = marker_for(&tag) {
                    constructs.push((range, marker));
                }
            }
            Event::End(tag) if closes_block(tag) => {
                depth = depth.saturating_sub(1);
            }
            Event::Rule => {
                blocks.push(Block::new(
                    BlockId::new(u32::try_from(blocks.len()).unwrap_or(u32::MAX)),
                    BlockKind::ThematicBreak,
                    ByteSpan::from(range.clone()),
                    depth,
                ));
                constructs.push((range, Marker::ThematicBreak));
            }
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

            // **The checkbox is syntax, and the whole reason it needs saying is that it is not a
            // tag.** `Tag::Item` covers the bullet; the `[ ]` after it arrives as this leaf event
            // with its own range and would otherwise fall to the item's content by subtraction —
            // drawn as prose, and concealed by nothing.
            Event::TaskListMarker(_) => constructs.push((range, Marker::TaskMarker)),

            // `[^1]` in the middle of a sentence, and it is a leaf event like the checkbox above —
            // so without this the reference reads as prose (MDEDIT-90).
            Event::FootnoteReference(_) => constructs.push((range, Marker::Footnote)),

            // Everything else contributes neither syntax nor content of its own.
            Event::End(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    Walk {
        constructs,
        contents,
        verbatim,
        blocks,
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
        // MDEDIT-90. The definition's range covers its `: ` and the title's covers nothing but the
        // line's terminator, which `normalise_line_syntax` gives back to content — so this is one
        // marker in practice even though two tags carry it.
        Tag::DefinitionListTitle | Tag::DefinitionListDefinition => Marker::DefinitionList,
        // MDEDIT-126. One construct covering the whole table, so everything the cells' text events
        // do not claim — the pipes, the spaces padding them, the delimiter row and the newlines —
        // is syntax by subtraction. Marking the *cells* instead would leave the pipes belonging to
        // nothing, and a byte belonging to nothing is a byte the tiling test catches.
        Tag::Table(_) => Marker::Table,
        _ => return None,
    })
}

/// Whether a byte could be part of a line's block prefix.
///
/// The characters a quote or list marker is ever drawn from. Deliberately the same set the
/// quote-and-list plausibility check in `normalise_line_syntax` uses, because they are answering the
/// same question from opposite ends: that one asks "could this be block syntax", and this asks "is
/// the block syntax still going".
fn is_block_prefix(byte: u8) -> bool {
    byte == b'>'
        || byte.is_ascii_whitespace()
        || byte.is_ascii_digit()
        || matches!(byte, b'-' | b'*' | b'+' | b'.' | b')')
}

/// Where a footnote definition's `[^label]:` ends, given the tag's whole range.
///
/// The first `]` after the opening `[^`, plus the `:` that must follow it and any spaces after that
/// — the indent belongs to the label in the same way a list item's does. A range that does not have
/// that shape gets nothing, which leaves every byte of it content: the safe end, because a marker is
/// concealed and content is not.
fn label_end(source: &str, range: &Range<usize>) -> usize {
    let text = &source[range.clone()];
    let Some(bracket) = text.find(']') else {
        return range.start;
    };
    let after = &text[bracket + 1..];
    if !after.starts_with(':') {
        return range.start;
    }
    let spaces = after[1..].len() - after[1..].trim_start_matches(' ').len();
    range.start + bracket + 2 + spaces
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
    let mut content_started = false;
    // Whether the walk is still in the run of bytes that could be this line's block prefix. Cleared
    // by the first byte that could not be part of one — **including a content byte**, which is why
    // it is updated before the content branch below rather than after it.
    let mut in_block_prefix = true;

    for i in 0..bytes.len() {
        let byte = bytes[i];

        // A newline is never syntax to conceal — concealing one would join two lines.
        if byte == b'\n' || byte == b'\r' {
            marks[i] = None;
            content_started = false;
            in_block_prefix = true;
            continue;
        }

        if !is_block_prefix(byte) {
            in_block_prefix = false;
        }

        if marks[i].is_none() {
            content_started = true;
            continue;
        }

        // A `>` in a line's **block prefix** is block-quote syntax. Range containment cannot express
        // that: a list item legitimately spans the continuation lines of a quote nested inside it,
        // so in `> > 1. one\n>>\n>>   two` the item's range covers the `>>` prefixes and, starting
        // later, is filled last and wins bytes that are the quote's.
        //
        // **The prefix is stricter than "no content yet on this line"**, and MDEDIT-90 is what made
        // the difference reachable: a footnote's label may be a `>`, so in `>[^>]:` the second `>`
        // has no content before it and is not a prefix either. Relabelling it split one construct
        // into three — `Footnote`, `BlockQuote`, `Footnote`. The prefix ends at the first byte that
        // could not be block-prefix material, which is the same set the quote-and-list plausibility
        // check below uses, and `[` is not in it.
        //
        // Confined to the prefix so it also cannot touch the `>` closing an autolink — content
        // precedes that one.
        if !is_block_prefix(byte) {
            in_block_prefix = false;
        }
        if in_block_prefix && byte == b'>' {
            marks[i] = Some(Marker::BlockQuote);
        }

        // **A delimiter run is never whitespace** (MDEDIT-112).
        //
        // `pulldown-cmark` strips a soft break's surrounding whitespace from the `Text` events, so
        // the gap between two children falls through the subtraction above and is emitted as the
        // *enclosing* construct's marker. In `_*\n `_~` the space opening the second line came back
        // as `Marker::Emphasis`, which is not emphasis syntax by any reading.
        //
        // It never mis-*covered* — the tiling gate held throughout, which is why it went unseen —
        // but a marker is **concealed** in preview, so the space vanished from the rendered line.
        // Systematic for any inline construct spanning a soft break, not something about emphasis.
        //
        // **Both sides of the break, not just the indent.** The first fix here only cleared a line's
        // leading whitespace, and the property test found `AA*〡\t\n¡*` in seconds: the tab is
        // *trailing*, stripped from the text event at the other end of the same soft break.
        //
        // Unconditional rather than confined to a line's edges, because a delimiter run is `*`, `_`,
        // `~` or a backtick and nothing else — whitespace carrying one is always this artefact. That
        // is why the test is `is_delimiter_run` and not "is inline": a **link's** closing marker
        // legitimately spans a newline and its indent between the destination and the title, and a
        // block marker's whitespace is real syntax.
        // **It leaves `in_prefix` and `content_started` alone**, and the first version of this did
        // not. A continuation line's indent is still that line's *prefix* — CommonMark allows three
        // spaces before a block marker — so declaring content started at it stopped the `>` after it
        // from being recognised as the quote's own syntax, and `">~~*\n >**~~#"` came back with a
        // `Marker::Strikethrough` holding `>`. Trailing whitespace does not care either way: content
        // preceded it, so both flags already say so.
        if is_markdown_space(byte) && is_delimiter_run(marks[i]) {
            marks[i] = None;
            continue;
        }

        if let Some(marker @ (Marker::BlockQuote | Marker::ListItem | Marker::DefinitionList)) =
            marks[i]
        {
            // Subtracting content from a construct's range assumes the parser reports content for
            // everything that is not syntax — but it emits no events at all for a link reference
            // definition, so `- item\n\n  [ref]: /url` leaves the definition inside the item's range
            // and unclaimed, and it would be concealed as list syntax.
            //
            // These markers are only ever drawn from a small set of characters, and only ever appear
            // before the line's content — the space in `[ref]: /url` is not a list indent. Anything
            // else degrades to content with its bytes intact.
            //
            // Subtracting content from a construct's range assumes the parser reports content for
            // everything that is not syntax, and it does not: it emits no events at all for a link
            // reference definition, so `- item\n\n  [ref]: /url` leaves the definition inside the
            // item's range and unclaimed, and it would be concealed as list syntax.
            //
            // **A definition list is here for the same reason** (MDEDIT-90), and the property test
            // found the same shape within a second: in `a  \n:[*]:*` the definition's range is
            // `:[*]:*` with no content event inside it, because what it holds is — again — a link
            // reference definition.
            //
            // **The set is per marker, and that is not tidiness.** It was one union, and a `)` is
            // list material that a quote's marker can never contain: in `>[\n)]:*` the quote's range
            // covers the lazy continuation line, so its `)` passed a shared check and came back as a
            // `BlockQuote` marker holding `)`. Pre-existing, and surfaced by MDEDIT-90 only because
            // its alphabet learned to spell `]:`.
            let plausible = byte.is_ascii_whitespace()
                || match marker {
                    Marker::BlockQuote => byte == b'>',
                    Marker::ListItem => {
                        byte.is_ascii_digit() || matches!(byte, b'-' | b'*' | b'+' | b'.' | b')')
                    }
                    _ => byte == b':',
                };

            if content_started || !plausible {
                marks[i] = None;
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
            // Extending the run rather than pushing a second of the same kind: the coalescing the
            // tiling invariant is stated in terms of. `moved_to` because the fields belong to
            // another crate now — see `mdedit_language`.
            Some(last) if last.kind() == kind => {
                *last = last
                    .clone()
                    .moved_to(ByteSpan::new(last.span().start(), end));
            }
            _ => out.push(Segment::new(
                ByteSpan::new(ByteOffset::new(offset), end),
                kind,
            )),
        }
    }

    out
}

/// Whether a marker is a **delimiter run** — syntax made only of repeated punctuation.
///
/// The distinction matters exactly once, in [`normalise_line_syntax`], and the line is *not*
/// inline-versus-block. A block marker's whitespace is real syntax: a list indent, the space after
/// `>`, the space in `# `. So is a **link's**, which is why `Link` and `Image` are absent here —
/// `[a](/url\n  "title")` puts a line's opening whitespace *inside* the closing marker, between the
/// destination and the title, and concealing it is correct. CommonMark spec example 510 says so, and
/// said so by failing when this list was drawn around "inline" instead.
///
/// What is left is the constructs whose markers are `*`, `_`, `~` and a backtick and nothing else. A
/// whitespace byte carrying one of those is never syntax. See MDEDIT-112.
fn is_delimiter_run(marker: Option<Marker>) -> bool {
    matches!(
        marker,
        Some(Marker::Emphasis | Marker::Strong | Marker::Strikethrough | Marker::CodeSpan)
    )
}

/// CommonMark's whitespace set, which is not Rust's.
///
/// `u8::is_ascii_whitespace` covers space, tab, newline, form feed and carriage return — but **not**
/// the vertical tab, which CommonMark counts and which `pulldown-cmark` therefore strips around a
/// soft break. The property test found `*A\u{b}\n!*:` within seconds of the first attempt at
/// MDEDIT-112 using the Rust predicate.
fn is_markdown_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}
