//! JSON, lexed for an editor rather than parsed for a program.
//!
//! # Why not `serde_json`
//!
//! Deserializing gives a *value*. This needs two things a value cannot provide.
//!
//! **Spans** — where each token sits in the bytes — because the editor decorates the source in
//! place and never re-serializes it. A `serde_json::Value` has thrown away the byte offsets, the
//! key order, the whitespace and the number formatting by the time you can look at it, and writing
//! it back out would reformat the user's file. `AGENTS.md` calls byte preservation the one rule that
//! outranks everything, and it matters more here than in Markdown, not less: nobody forgives an
//! editor that reformats their JSON on save.
//!
//! **Error tolerance**, because a file being written is invalid most of the time. `{"a":` is what
//! every JSON document looks like halfway through being typed, and a parser that stops at the first
//! error would leave the rest of the document unhighlighted at exactly the moment the user is
//! looking at it. So this never fails. There is no error type, because there is no failure — every
//! byte is classified, and bytes that make no sense are classified as text.
//!
//! # The invariant
//!
//! The same gate the Markdown locator has: **the segments tile the source exactly**, so
//! concatenating them reproduces it byte for byte.
//!
//! ```
//! # use mdedit_json::Json;
//! # use mdedit_language::Language;
//! let src = r#"{"a": [1, true], "b": null}"#;
//! let rebuilt: String = Json
//!     .analyze(src)
//!     .segments
//!     .iter()
//!     .map(|s| s.span().slice(src))
//!     .collect();
//! assert_eq!(rebuilt, src);
//! ```
//!
//! That holds for invalid input too, which is the case an editor actually lives in, and it is
//! property-tested over generated documents including broken ones.
//!
//! # Never concealed
//!
//! JSON's braces, brackets, commas and quotes **are** the document. Hiding them would produce
//! something that is not JSON and cannot be edited back into JSON, so [`Json::conceals`] is `false`
//! and every byte is on screen always. Colour is the only decoration.

use mdedit_language::{Analysis, Block, BlockKind, Language, Segment, SegmentKind, Token};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};

/// JSON, as this editor renders it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Json;

/// The one JSON there is, for a caller holding a `&'static dyn Language`.
pub static JSON: Json = Json;

impl Language for Json {
    fn name(&self) -> &'static str {
        "JSON"
    }

    fn analyze(&self, source: &str) -> Analysis {
        analyze(source)
    }

    fn analyze_region(&self, _source: &str, _region: ByteSpan) -> Option<Analysis> {
        // JSON nests without limit and a region carries no record of how deep it started, so an edit
        // inside an object may have no safe restart short of the top of the file. Saying so gets a
        // whole-document parse — correct, and merely slower. Claiming a boundary that does not exist
        // would get a fast wrong answer, and MDEDIT-18's reconcile could not repair it, because a
        // JSON document has no equivalent of a blank line to resynchronise on.
        None
    }

    fn conceals(&self) -> bool {
        false
    }
}

/// Lexes `source` into coloured spans and top-level members.
///
/// Never fails. See the [crate docs](crate) for the tiling guarantee.
#[must_use]
pub fn analyze(source: &str) -> Analysis {
    let mut lexer = Lexer::new(source);
    lexer.run();
    Analysis {
        segments: lexer.segments,
        blocks: lexer.blocks,
    }
}

/// What the lexer is doing to the bytes.
struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    at: usize,
    segments: Vec<Segment>,
    blocks: Vec<Block>,
    /// How deep in containers we are, so top-level members can be recognised.
    depth: u16,
    /// Where the current top-level member started, once one has.
    member: Option<usize>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        // One segment per token is the right order of magnitude; JSON is punctuation-dense.
        let estimate = source.len() / 8 + 8;
        Self {
            source,
            bytes: source.as_bytes(),
            at: 0,
            segments: Vec::with_capacity(estimate),
            blocks: Vec::new(),
            depth: 0,
            member: None,
        }
    }

    fn run(&mut self) {
        while self.at < self.bytes.len() {
            let start = self.at;
            match self.bytes[self.at] {
                b'{' | b'[' => {
                    self.open(start);
                }
                b'}' | b']' => {
                    self.close(start);
                }
                b',' => {
                    self.at += 1;
                    // A comma ends a member before it is emitted, so the member's span stops at the
                    // value rather than swallowing the separator.
                    self.finish_member(start);
                    self.push(start, self.at, SegmentKind::Token(Token::Punctuation));
                }
                b':' => {
                    self.at += 1;
                    self.push(start, self.at, SegmentKind::Token(Token::Punctuation));
                }
                b'"' => {
                    self.begin_member(start);
                    self.string(start);
                }
                b'-' | b'0'..=b'9' => {
                    self.begin_member(start);
                    self.number(start);
                }
                b't' | b'f' | b'n' => {
                    self.begin_member(start);
                    self.literal(start);
                }
                byte if byte.is_ascii_whitespace() => {
                    while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
                        self.at += 1;
                    }
                    self.push(start, self.at, SegmentKind::Content);
                }
                _ => {
                    // Something that is not JSON. Its bytes are still covered — that is the whole
                    // difference between lexing for an editor and parsing for a program.
                    self.begin_member(start);
                    self.advance_one();
                    self.push(start, self.at, SegmentKind::Content);
                }
            }
        }
        // A document that ends mid-member still has to report it, or the last member of every file
        // without a trailing newline would be missing from the block list.
        self.finish_member(self.bytes.len());
    }

    /// Steps over one whole character, so a multi-byte one is never split.
    ///
    /// Splitting one would produce a span that is not on a character boundary, and `ByteSpan::slice`
    /// would panic on it — from a render, on a file the user merely opened.
    fn advance_one(&mut self) {
        self.at += 1;
        while self.at < self.bytes.len() && !self.source.is_char_boundary(self.at) {
            self.at += 1;
        }
    }

    fn open(&mut self, start: usize) {
        self.begin_member(start);
        self.at += 1;
        self.push(start, self.at, SegmentKind::Token(Token::Punctuation));
        self.depth = self.depth.saturating_add(1);
    }

    fn close(&mut self, start: usize) {
        // The member ends before the brace that closes its container, not after it.
        if self.depth == 1 {
            self.finish_member(start);
        }
        self.at += 1;
        self.push(start, self.at, SegmentKind::Token(Token::Punctuation));
        self.depth = self.depth.saturating_sub(1);
    }

    /// A string, which is a key when the next thing that is not whitespace is a colon.
    fn string(&mut self, start: usize) {
        self.at += 1;
        while self.at < self.bytes.len() {
            match self.bytes[self.at] {
                // A backslash escapes whatever follows, including a quote. Stepping two keeps
                // `"a\""` from ending at the escaped quote.
                b'\\' => {
                    self.at += 1;
                    if self.at < self.bytes.len() {
                        self.advance_one();
                    }
                }
                b'"' => {
                    self.at += 1;
                    break;
                }
                // An unterminated string runs to the end of the file rather than to the end of the
                // line. That is what JSON says a string is, and a newline inside one is invalid
                // rather than a terminator — but the bytes are still covered either way.
                _ => self.advance_one(),
            }
        }

        let kind = if self.followed_by_colon() {
            Token::Key
        } else {
            Token::String
        };
        self.push(start, self.at, SegmentKind::Token(kind));
    }

    /// Whether the next non-whitespace byte is a colon.
    fn followed_by_colon(&self) -> bool {
        self.bytes[self.at..]
            .iter()
            .find(|byte| !byte.is_ascii_whitespace())
            == Some(&b':')
    }

    /// A number, taken loosely: the run of bytes a number could be made of.
    ///
    /// `1.2.3` lexes as one number rather than as an error. An editor's job here is to colour what
    /// the user is typing, and `1.2.3` is what `1.23` looks like a keystroke before it is finished.
    fn number(&mut self, start: usize) {
        self.at += 1;
        while self.at < self.bytes.len() {
            match self.bytes[self.at] {
                b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => self.at += 1,
                _ => break,
            }
        }
        self.push(start, self.at, SegmentKind::Token(Token::Number));
    }

    /// `true`, `false` or `null` — or the run of letters someone is partway through typing.
    fn literal(&mut self, start: usize) {
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_alphabetic() {
            self.at += 1;
        }
        self.push(start, self.at, SegmentKind::Token(Token::Literal));
    }

    /// Notes that a top-level member has started here, if one has.
    fn begin_member(&mut self, start: usize) {
        if self.depth == 1 && self.member.is_none() {
            self.member = Some(start);
        }
    }

    /// Closes the current top-level member at `end`, if one is open.
    fn finish_member(&mut self, end: usize) {
        let Some(start) = self.member.take() else {
            return;
        };
        if start >= end {
            return;
        }
        self.blocks.push(Block::new(
            BlockId::new(u32::try_from(self.blocks.len()).unwrap_or(u32::MAX)),
            // A member is a run of the document that can be laid out and cached as a unit, which is
            // all `Paragraph` has ever meant to the renderer. Naming it `JsonMember` would make
            // every renderer learn a case to treat it identically.
            BlockKind::Paragraph,
            ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
            0,
        ));
    }

    /// Appends a span, joining it to the one before if they are the same kind and meet.
    ///
    /// Coalescing for the same reason `mdedit-markers` does it: two adjacent runs of one kind and
    /// one run of that kind cover the same bytes, and the tiling property is stated in terms of the
    /// coalesced form.
    fn push(&mut self, start: usize, end: usize, kind: SegmentKind) {
        if start >= end {
            return;
        }
        if let Some(last) = self.segments.last_mut() {
            if last.kind() == kind && last.span().end().get() == start {
                *last = last
                    .clone()
                    .moved_to(ByteSpan::new(last.span().start(), ByteOffset::new(end)));
                return;
            }
        }
        self.segments.push(Segment::new(
            ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
            kind,
        ));
    }
}
