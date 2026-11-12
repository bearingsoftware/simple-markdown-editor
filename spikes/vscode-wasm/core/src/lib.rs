//! The headless editor core behind a wasm-bindgen surface (MDEDIT-206).
//!
//! Everything crosses as flat `u32` arrays so the JS side pays one copy per call, not one object per
//! span. Segment layout: `start, end, kind, sub`. Block layout: `id, kind, sub, start, end, depth`.
//! `analyze` and `Session::flat` return `[n_segments, n_blocks, segments.., blocks..]`.

use mdedit_document::Document;
use mdedit_language::{Analysis, Block, BlockKind, Marker, Segment, SegmentKind, Token};
use mdedit_offsets::{ByteOffset, ByteSpan};
use wasm_bindgen::prelude::*;

pub fn marker_code(m: Marker) -> u32 {
    match m {
        Marker::Emphasis => 1,
        Marker::Strong => 2,
        Marker::Strikethrough => 3,
        Marker::CodeSpan => 4,
        Marker::CodeBlock => 5,
        Marker::Heading => 6,
        Marker::BlockQuote => 7,
        Marker::ListItem => 8,
        Marker::ThematicBreak => 9,
        Marker::Link => 10,
        Marker::Image => 11,
        Marker::Escape => 12,
        Marker::HardBreak => 13,
        Marker::Footnote => 14,
        Marker::DefinitionList => 15,
        Marker::TaskMarker => 16,
        Marker::Table => 17,
        _ => 99,
    }
}

fn token_code(t: Token) -> u32 {
    match t {
        Token::Key => 1,
        Token::Tag => 2,
        Token::String => 3,
        Token::Number => 4,
        Token::Literal => 5,
        Token::Punctuation => 6,
        Token::Comment => 7,
        _ => 99,
    }
}

fn segment_row(s: &Segment, out: &mut Vec<u32>) {
    let (kind, sub) = match s.kind() {
        SegmentKind::Content => (0, 0),
        SegmentKind::Marker(m) => (1, marker_code(m)),
        SegmentKind::Token(t) => (2, token_code(t)),
        _ => (0, 0),
    };
    out.extend([s.span().start().get() as u32, s.span().end().get() as u32, kind, sub]);
}

fn block_row(b: &Block, out: &mut Vec<u32>) {
    let (kind, sub) = match b.kind() {
        BlockKind::Paragraph => (1, 0),
        BlockKind::Heading(l) => (2, u32::from(l)),
        BlockKind::CodeBlock => (3, 0),
        BlockKind::BlockQuote => (4, 0),
        BlockKind::List => (5, 0),
        BlockKind::ListItem => (6, 0),
        BlockKind::ThematicBreak => (7, 0),
        BlockKind::Html => (8, 0),
        BlockKind::Table => (9, 0),
        BlockKind::TableRow => (10, 0),
        BlockKind::TableCell => (11, 0),
        _ => (0, 0),
    };
    out.extend([
        b.id().get() as u32,
        kind,
        sub,
        b.span().start().get() as u32,
        b.span().end().get() as u32,
        u32::from(b.depth()),
    ]);
}

fn flatten(segments: &[Segment], blocks: &[Block]) -> Vec<u32> {
    let mut out = Vec::with_capacity(2 + segments.len() * 4 + blocks.len() * 6);
    out.extend([segments.len() as u32, blocks.len() as u32]);
    for s in segments {
        segment_row(s, &mut out);
    }
    for b in blocks {
        block_row(b, &mut out);
    }
    out
}

/// Whole-document analysis, flattened for JS.
#[wasm_bindgen]
pub fn analyze(source: &str) -> Vec<u32> {
    let Analysis { segments, blocks } = mdedit_markers::analyze(source);
    flatten(&segments, &blocks)
}

/// Whole-document analysis that returns only the segment count, so a timing excludes the marshal.
#[wasm_bindgen]
pub fn analyze_count(source: &str) -> u32 {
    mdedit_markers::analyze(source).segments.len() as u32
}

/// Highlight spans of a fence body: `start, end, token` triples, relative to `code`.
#[cfg(feature = "highlight")]
#[wasm_bindgen]
pub fn highlight(info: &str, code: &str) -> Vec<u32> {
    let mut out = Vec::new();
    if let Some(spans) = mdedit_highlight::highlight(info, code) {
        for (span, token) in spans {
            out.extend([span.start().get() as u32, span.end().get() as u32, token_code(token)]);
        }
    }
    out
}

/// One open document: the rope, the analysis, the undo history.
#[wasm_bindgen]
pub struct Session {
    doc: Document,
}

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new(text: &str) -> Session {
        Session { doc: Document::with_text(text, &mdedit_markers::MARKDOWN) }
    }

    pub fn len_bytes(&self) -> u32 {
        self.doc.len_bytes() as u32
    }

    pub fn text(&self) -> String {
        self.doc.rope().to_string()
    }

    pub fn flat(&self) -> Vec<u32> {
        flatten(self.doc.segments(), self.doc.blocks())
    }

    /// One byte-span replace, the only edit primitive the front end uses.
    pub fn replace(&mut self, start: u32, end: u32, text: &str) -> Result<(), JsError> {
        let span = ByteSpan::new(ByteOffset::new(start as usize), ByteOffset::new(end as usize));
        self.doc.replace(span, text).map_err(|e| JsError::new(&e.to_string()))
    }

    /// The whole-document parse a scoped edit owes when typing stops.
    pub fn reconcile(&mut self) -> bool {
        self.doc.reconcile()
    }

    pub fn is_scoped(&self) -> bool {
        self.doc.is_scoped()
    }

    /// VS Code position of a byte: `[line, UTF-16 column]`. A byte between a CRLF pair steps back to before the CR.
    pub fn position(&self, byte: u32) -> Vec<u32> {
        let rope = self.doc.rope();
        let mut byte = (byte as usize).min(rope.len_bytes());
        if byte > 0 && byte < rope.len_bytes() && rope.byte(byte) == 0x0a && rope.byte(byte - 1) == 0x0d {
            byte -= 1;
        }
        let char_idx = rope.byte_to_char(byte);
        let line = rope.char_to_line(char_idx);
        let column = rope.char_to_utf16_cu(char_idx) - rope.char_to_utf16_cu(rope.line_to_char(line));
        vec![line as u32, column as u32]
    }

    pub fn undo(&mut self) -> bool {
        self.doc.undo()
    }

    pub fn revision(&self) -> f64 {
        self.doc.revision() as f64
    }
}

/// The benchmark fixture from `crates/markers/benches/segment.rs`, repeated to `target_bytes`.
#[wasm_bindgen]
pub fn fixture(target_bytes: usize) -> String {
    let unit = "\
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
    let mut out = String::with_capacity(target_bytes + unit.len());
    while out.len() < target_bytes {
        out.push_str(unit);
    }
    out
}
