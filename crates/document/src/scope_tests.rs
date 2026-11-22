//! MDEDIT-207: `resegment_scope` splices in place instead of rebuilding, and has to produce exactly
//! what the rebuild produced. The oracle is that rebuild, kept here as it was.
//!
//! These are unit tests, not integration tests, because the oracle needs `scope_for`, which is
//! private: it is the scope the real path used that the reference has to be handed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::borrow::Cow;
use std::fmt::Write as _;
use std::ops::Range;

use mdedit_language::{Block, Language, Segment};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};
use proptest::prelude::*;

use super::{Document, push_merged};

/// `resegment_scope` as it was before MDEDIT-207: copy the whole rope, rebuild both lists by cloning.
fn reference(
    before: &Document,
    text: &str,
    scope: Range<usize>,
    removed: usize,
    inserted: usize,
) -> Option<(Vec<Segment>, Vec<Block>)> {
    let (old_segments, old_blocks) = (&before.segments, &before.blocks);
    let delta = inserted.cast_signed() - removed.cast_signed();
    let shift = |offset: usize| -> usize { offset.saturating_add_signed(delta) };

    let start = snap(text, scope.start);
    let end = snap(text, shift(scope.end).min(text.len())).max(start);
    let fresh = before
        .language
        .analyze_region(&text[start..end], ByteOffset::new(start))?;

    let mut segments: Vec<Segment> = Vec::new();
    for segment in old_segments {
        let span = segment.span();
        if span.end().get() <= scope.start {
            push_merged(&mut segments, segment.clone());
        } else if span.start().get() < scope.start {
            push_merged(
                &mut segments,
                segment
                    .clone()
                    .moved_to(ByteSpan::new(span.start(), ByteOffset::new(scope.start))),
            );
        }
    }
    for segment in fresh.segments {
        push_merged(&mut segments, segment);
    }
    for segment in old_segments {
        let span = segment.span();
        if span.start().get() >= scope.end {
            push_merged(
                &mut segments,
                segment.clone().moved_to(ByteSpan::new(
                    ByteOffset::new(shift(span.start().get())),
                    ByteOffset::new(shift(span.end().get())),
                )),
            );
        } else if span.end().get() > scope.end {
            push_merged(
                &mut segments,
                segment.clone().moved_to(ByteSpan::new(
                    ByteOffset::new(shift(scope.end)),
                    ByteOffset::new(shift(span.end().get())),
                )),
            );
        }
    }

    let mut blocks: Vec<Block> = Vec::new();
    blocks.extend(
        old_blocks
            .iter()
            .filter(|block| block.span().end().get() <= scope.start)
            .copied(),
    );
    blocks.extend(fresh.blocks);
    blocks.extend(
        old_blocks
            .iter()
            .filter(|block| block.span().start().get() >= scope.end)
            .map(|block| {
                block.moved_to(ByteSpan::new(
                    ByteOffset::new(shift(block.span().start().get())),
                    ByteOffset::new(shift(block.span().end().get())),
                ))
            }),
    );
    for (index, block) in blocks.iter_mut().enumerate() {
        *block = block.with_id(BlockId::new(u32::try_from(index).unwrap_or(u32::MAX)));
    }
    Some((segments, blocks))
}

/// Applies one edit through the public path and asserts it equals the reference and still tiles.
fn edit_and_compare(doc: &mut Document, span: ByteSpan, replacement: &str) {
    let before = doc.clone();
    let scope = before.scope_for(span);
    doc.replace(span, replacement).unwrap();

    let text = doc.to_string();
    let rebuilt: String = doc
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, text, "the segments lost bytes");

    let expected = scope.and_then(|scope| {
        reference(
            &before,
            &text,
            scope,
            span.end().get() - span.start().get(),
            replacement.len(),
        )
    });
    let Some((segments, blocks)) = expected else {
        return;
    };
    assert_eq!(doc.segments(), segments, "segments differ from the rebuild");
    assert_eq!(doc.blocks(), blocks, "blocks differ from the rebuild");
}

const UNIT: &str = "\
## Section heading

Some prose with **strong**, *emphasis*, `code` and a [link](http://example.com \"title\").

- a list item
- another with `code`

> a quoted line
> and its continuation

```rust
fn example() -> u32 { 42 }
```

";

const PIECES: [&str; 9] = [
    "[foo]\n\n[foo]: /url\n\n",
    UNIT,
    "é — 汉字 and a crab 🦀\n\n",
    "# \n\n",
    "```\n",
    "1. one\n2. two\n\n",
    "| a | b |\n|---|---|\n| 1 | 2 |\n\n",
    "Setext\n======\n\n",
    "\n\n\n",
];

fn snap(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Where an edit lands: anywhere, a top-level block start, the last byte, or byte zero.
fn target(doc: &Document, text: &str, kind: u8, at: usize) -> usize {
    match kind % 4 {
        0 => snap(text, at % (text.len() + 1)),
        1 => {
            let starts: Vec<usize> = doc
                .blocks()
                .iter()
                .filter(|block| block.depth() == 0)
                .map(|block| block.span().start().get())
                .collect();
            starts
                .get(at % starts.len().max(1))
                .copied()
                .unwrap_or_default()
        }
        2 => text.len(),
        _ => 0,
    }
}

/// The edit that puts a fresh run beside an untouched one of the same kind at the region's left edge,
/// which a uniformly random edit almost never finds in a document this size.
///
/// Breaking a link definition turns the `[foo]` above it from link markers back into content, which
/// meets the blank line before it. The right edge has no Markdown equivalent: a marker never holds
/// a line ending, so the byte before a block start is always content, and the toy language below is
/// what reaches that seam.
fn aimed(text: &str, kind: u8, at: usize) -> Option<(usize, usize, &'static str)> {
    let (needle, offset, len, replacement) = match kind % 6 {
        4 => ("[foo]:", 5, 1, ""),
        _ => return None,
    };
    let found: Vec<usize> = text.match_indices(needle).map(|(index, _)| index).collect();
    let start = *found.get(at % found.len().max(1))? + offset;
    Some((start, start + len, replacement))
}

type Edit = (u8, usize, usize, String);

fn run(language: &'static dyn Language, text: &str, edits: &[Edit]) {
    let mut doc = Document::with_text(text, language);
    for (kind, at, len, replacement) in edits {
        let current = doc.to_string();
        if let Some((start, end, replacement)) = aimed(&current, *kind, *at) {
            let span = ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end));
            edit_and_compare(&mut doc, span, replacement);
            continue;
        }
        let start = target(&doc, &current, *kind, *at);
        let end = snap(&current, start + len);
        edit_and_compare(
            &mut doc,
            ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
            replacement,
        );
    }
}

fn edits() -> impl Strategy<Value = Vec<Edit>> {
    prop::collection::vec(
        (
            any::<u8>(),
            0usize..60_000,
            0usize..9,
            "[a-zé汉 *#`>\\-\\[\\]()\n|=]{0,6}",
        ),
        1..10,
    )
}

fn data_document(which: usize, entries: usize) -> (&'static dyn Language, String) {
    let mut text = String::new();
    match which % 4 {
        0 => {
            text.push_str("{\n");
            for n in 0..entries {
                writeln!(
                    text,
                    "  \"key_{n}\": {{ \"name\": \"value é {n}\", \"n\": {n}, \"ok\": true }},"
                )
                .unwrap();
            }
            text.push_str("  \"last\": null\n}\n");
            (&mdedit_json::JSON, text)
        }
        1 => {
            for n in 0..entries {
                writeln!(text, "key_{n}:\n  name: value é {n}\n  n: {n}\n  ok: true").unwrap();
            }
            (&mdedit_yaml::YAML, text)
        }
        2 => {
            for n in 0..entries {
                writeln!(
                    text,
                    "[key_{n}]\nname = \"value é {n}\"\nn = {n}\nok = true\n"
                )
                .unwrap();
            }
            (&mdedit_toml::TOML, text)
        }
        _ => {
            for n in 0..entries {
                writeln!(text, "[key_{n}]\nname = value é {n}\nn = {n}\n").unwrap();
            }
            (&mdedit_ini::INI, text)
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn markdown_splice_equals_the_rebuild(
        head in 0usize..60,
        pieces in prop::collection::vec(0usize..PIECES.len(), 0..30),
        edits in edits(),
    ) {
        let mut text = UNIT.repeat(head);
        for piece in pieces {
            text.push_str(PIECES[piece]);
        }
        text.push_str(&UNIT.repeat(80));
        run(&mdedit_markers::MARKDOWN, &text, &edits);
    }

    #[test]
    fn data_languages_splice_equals_the_rebuild(
        which in 0usize..4,
        edits in edits(),
    ) {
        let (language, text) = data_document(which, 500);
        run(language, &text, &edits);
    }
}

/// A rope chunk is about a kilobyte, so a region that fits in one is borrowed and one that straddles
/// a seam is copied. Both are asserted from the chunk boundaries the rope really has.
#[test]
fn a_region_inside_one_chunk_is_borrowed_and_one_across_a_seam_is_copied() {
    let doc = Document::from(UNIT.repeat(5000).as_str());
    assert!(doc.len_bytes() > 1_000_000);

    let (chunk, chunk_start, ..) = doc.text.chunk_at_byte(doc.len_bytes() / 2);
    let end = chunk_start + chunk.len();
    assert!(chunk.len() > 16, "a chunk too small to test inside");
    let whole = doc.to_string();

    let (inside, at) = doc.region_text(chunk_start + 2, end - 2);
    assert!(
        matches!(inside, Cow::Borrowed(_)),
        "one chunk should borrow"
    );
    assert_eq!(at, chunk_start + 2);
    assert_eq!(inside, whole[chunk_start + 2..end - 2]);

    let (across, at) = doc.region_text(end - 3, end + 3);
    assert!(matches!(across, Cow::Owned(_)), "a seam needs a copy");
    assert_eq!(at, end - 3);
    assert_eq!(across, whole[end - 3..end + 3]);
}

/// An edit beside a chunk seam in a document well over a megabyte, where the region is copied.
#[test]
fn an_edit_beside_a_chunk_seam_in_a_large_document_matches_the_rebuild() {
    let mut doc = Document::from(UNIT.repeat(5000).as_str());
    let (chunk, chunk_start, ..) = doc.text.chunk_at_byte(doc.len_bytes() / 2);
    let seam = chunk_start + chunk.len();
    for at in [seam - 1, seam, seam + 1] {
        let at = ByteOffset::new(at);
        edit_and_compare(&mut doc, ByteSpan::new(at, at), "x");
        edit_and_compare(
            &mut doc,
            ByteSpan::new(at, ByteOffset::new(at.get() + 1)),
            "",
        );
    }
}

/// A region edge inside a character would panic `Rope::byte_slice`; the keystroke path snaps down
/// instead, as the languages' own `analyze_region` does.
#[test]
fn a_region_off_a_character_boundary_is_snapped_not_a_panic() {
    let doc = Document::from("aé汉🦀 tail".repeat(4000).as_str());
    let text = doc.to_string();
    for at in 0..40 {
        let (region, start) = doc.region_text(at, at + 5);
        assert!(text.is_char_boundary(start));
        assert!(text[start..].starts_with(&*region));
    }
    let (region, start) = doc.region_text(doc.len_bytes() + 9, doc.len_bytes() + 12);
    assert_eq!((region.as_ref(), start), ("", doc.len_bytes()));
}

/// A language where a line's neighbours decide its kind, so an edit can change what the region's
/// first or last segment *is* and make it meet the unchanged segment beside it.
///
/// A `#` line is a token unless the next line starts with `^` or the previous one with `~`; either
/// of those makes it content. Every line is one top-level block.
#[derive(Debug)]
struct Neighbours;

static NEIGHBOURS: Neighbours = Neighbours;

impl Language for Neighbours {
    fn name(&self) -> &'static str {
        "Neighbours"
    }

    fn analyze(&self, source: &str) -> mdedit_language::Analysis {
        self.analyze_region(source, ByteOffset::ZERO).unwrap()
    }

    fn analyze_region(&self, text: &str, at: ByteOffset) -> Option<mdedit_language::Analysis> {
        use mdedit_language::{BlockKind, SegmentKind, Token};

        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let mut segments: Vec<Segment> = Vec::new();
        let mut blocks = Vec::new();
        let mut offset = at.get();
        for (index, line) in lines.iter().enumerate() {
            let after_tilde = index > 0 && lines[index - 1].starts_with('~');
            let before_caret = lines
                .get(index + 1)
                .is_some_and(|next| next.starts_with('^'));
            let kind = if line.starts_with('#') && !after_tilde && !before_caret {
                SegmentKind::Token(Token::Key)
            } else {
                SegmentKind::Content
            };
            let span = ByteSpan::new(
                ByteOffset::new(offset),
                ByteOffset::new(offset + line.len()),
            );
            push_merged(&mut segments, Segment::new(span, kind));
            blocks.push(Block::new(
                BlockId::new(u32::try_from(index).unwrap()),
                BlockKind::Paragraph,
                span,
                0,
            ));
            offset += line.len();
        }
        Some(mdedit_language::Analysis { segments, blocks })
    }

    fn conceals(&self) -> bool {
        false
    }
}

/// The two seams of the splice. `push_merged` has to join the fresh run to the untouched segment
/// beside it when an edit makes them the same kind, which no generated Markdown edit can arrange.
#[test]
fn a_seam_between_fresh_and_untouched_segments_is_merged() {
    let pad = "ab\n".repeat(3000);
    for (text, edit_at, inserted) in [
        // `^` turns the `#t` above it into content, which then meets the content before it.
        (format!("{pad}#t\nb\nc\n{pad}"), pad.len() + 3, "^"),
        // `~` turns the `#u` below it into content, which then meets the content after it.
        (format!("{pad}a\nb\n#u\nc\n{pad}"), pad.len() + 2, "~"),
    ] {
        let mut doc = Document::with_text(&text, &NEIGHBOURS);
        let at = ByteOffset::new(edit_at);
        edit_and_compare(&mut doc, ByteSpan::new(at, at), inserted);
        assert!(doc.is_scoped());

        let rebuilt = NEIGHBOURS.analyze(&doc.to_string());
        assert_eq!(doc.segments(), rebuilt.segments, "seam left two runs");
    }
}
