//! INI, lexed for highlighting rather than parsed for loading.
//!
//! # There is no INI specification, and that is the design constraint
//!
//! Every other language here has one document to be right against. INI has none: it is a family of
//! conventions that agree on `[section]` and `key = value` and disagree on nearly everything else.
//! So this crate states the dialect it reads rather than claiming to read "INI", and each choice
//! below is the one that mis-reads the fewest real files.
//!
//! - **A comment is `;` or `#` at the start of a line**, after any indent — and *not* mid-line.
//!   Python's `configparser` defaults to exactly this, and the reason is that an inline `;` cannot be
//!   told from a semicolon somebody meant: `path = C:\a;C:\b` is a value, not a value and a comment.
//!   Guessing wrong here loses bytes off the end of a value on screen.
//! - **Either `=` or `:` separates a key from a value**, whichever comes first.
//! - **A value is the whole rest of the line**, one run, classified as a unit. So `key = a=b` has a
//!   value of `a=b` rather than a second separator in it — the first separator is the only one.
//! - **A line with no separator is a continuation** of the value above it. `configparser` decides
//!   that by indentation; this decides it by the absence of a separator, which reads the same way to
//!   a person and also copes with the files that indent their keys under a section header.
//!
//! # `.cfg` is not a format
//!
//! It is a file extension, and treating it as INI is a guess. Python's `setup.cfg` is INI; plenty of
//! `.cfg` files elsewhere are something else entirely. It is a **better** guess than the plain text
//! it would otherwise get and it is still a guess, which is why it is written down here and beside
//! the mapping in `detect.rs` rather than left implied by a line of code.
//!
//! # A section header at column zero is a safe restart point
//!
//! So this answers [`analyze_region`](Language::analyze_region). It is the easiest of the languages
//! here to say that about: the only state a line carries is whether it continues the one above, and a
//! line at column zero holding a separator provably does not.
//!
//! # The invariant
//!
//! The same gate every language here has: **the segments tile the source exactly**.
//!
//! ```
//! # use mdedit_ini::Ini;
//! # use mdedit_language::Language;
//! let src = "; a note\n[server]\nhost = localhost\nport: 8080\n";
//! let rebuilt: String = Ini
//!     .analyze(src)
//!     .segments
//!     .iter()
//!     .map(|s| s.span().slice(src))
//!     .collect();
//! assert_eq!(rebuilt, src);
//! ```

use mdedit_language::{Analysis, Block, BlockKind, Language, Segment, SegmentKind, Token};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};

/// INI, as this editor renders it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ini;

/// The one INI there is, for a caller holding a `&'static dyn Language`.
pub static INI: Ini = Ini;

impl Language for Ini {
    fn name(&self) -> &'static str {
        "INI"
    }

    fn analyze(&self, source: &str) -> Analysis {
        analyze(source)
    }

    fn analyze_region(&self, source: &str, region: ByteSpan) -> Option<Analysis> {
        Some(analyze_region(source, region))
    }

    fn conceals(&self) -> bool {
        false
    }
}

/// Lexes `source` into coloured spans and top-level blocks.
///
/// Never fails. See the [crate docs](crate) for the tiling guarantee and for the dialect it reads.
#[must_use]
pub fn analyze(source: &str) -> Analysis {
    let mut lexer = Lexer::new(source);
    lexer.run();
    Analysis {
        segments: lexer.segments,
        blocks: lexer.blocks,
    }
}

/// Lexes only `region`, as if the rest of `source` were not there, in `source`'s coordinates.
///
/// # The contract that makes this safe
///
/// **`region` must begin at a [`Block`] reported by a previous [`analyze`], or at byte zero.** A
/// block begins at a column-zero line, and the only state a line carries here is whether it
/// continues the one above — which a column-zero line with a separator on it provably does not.
///
/// A region outside the source, or one whose ends are not character boundaries, is clamped and
/// snapped rather than rejected.
#[must_use]
pub fn analyze_region(source: &str, region: ByteSpan) -> Analysis {
    let start = floor_boundary(source, region.start().get().min(source.len()));
    let end = floor_boundary(source, region.end().get().clamp(start, source.len()));

    let mut analysis = analyze(&source[start..end]);
    if start == 0 {
        return analysis;
    }
    for segment in &mut analysis.segments {
        *segment = segment.clone().moved_to(shift(segment.span(), start));
    }
    for block in &mut analysis.blocks {
        *block = block.moved_to(shift(block.span(), start));
    }
    analysis
}

fn shift(span: ByteSpan, by: usize) -> ByteSpan {
    ByteSpan::new(
        ByteOffset::new(span.start().get() + by),
        ByteOffset::new(span.end().get() + by),
    )
}

/// The character boundary at or before `at`.
fn floor_boundary(source: &str, at: usize) -> usize {
    let mut at = at;
    while at > 0 && !source.is_char_boundary(at) {
        at -= 1;
    }
    at
}

struct Lexer<'a> {
    source: &'a str,
    segments: Vec<Segment>,
    blocks: Vec<Block>,
    /// Where the current top-level block started, and the byte after its last line.
    open_block: Option<(usize, usize)>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        let estimate = source.len() / 12 + 8;
        Self {
            source,
            segments: Vec::with_capacity(estimate),
            blocks: Vec::new(),
            open_block: None,
        }
    }

    fn run(&mut self) {
        let mut at = 0;
        while at < self.source.len() {
            let end = self.source[at..]
                .find('\n')
                .map_or(self.source.len(), |offset| at + offset + 1);
            self.line(at, end);
            at = end;
        }
        self.close_block();
    }

    /// One line, including its newline if it has one.
    fn line(&mut self, start: usize, end: usize) {
        let line = &self.source[start..end];
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        let body = line[indent..].trim_end_matches(['\n', '\r']);
        let body_end = start + indent + body.len();

        // A blank line belongs to whatever block surrounds it and starts nothing.
        if body.is_empty() {
            self.push(start, end, SegmentKind::Content);
            self.extend_block(end);
            return;
        }

        if indent == 0 {
            self.close_block();
            self.open_block = Some((start, end));
        } else {
            self.extend_block(end);
        }

        self.push(start, start + indent, SegmentKind::Content);
        self.body(start + indent, body_end);
        self.push(body_end, end, SegmentKind::Content);
    }

    /// The part of a line that is not its indent or its line ending.
    fn body(&mut self, start: usize, end: usize) {
        let text = &self.source[start..end];
        if text.starts_with(';') || text.starts_with('#') {
            self.push(start, end, SegmentKind::Token(Token::Comment));
            return;
        }
        if text.starts_with('[') {
            self.header(start, end);
            return;
        }
        match self.separator(start, end) {
            Some(at) => {
                self.name(start, at);
                self.push(at, at + 1, SegmentKind::Token(Token::Punctuation));
                self.value(at + 1, end);
            }
            // A line with no separator continues the value above it — or is a key halfway through
            // being typed, which reads the same and is coloured the same.
            None => self.push(start, end, SegmentKind::Content),
        }
    }

    /// `[section]`, and whatever follows it on the line.
    fn header(&mut self, start: usize, end: usize) {
        self.push(start, start + 1, SegmentKind::Token(Token::Punctuation));
        let Some(offset) = self.source[start + 1..end].find(']') else {
            // A header that never closes: everything typed so far is still the name.
            self.name(start + 1, end);
            return;
        };
        let close = start + 1 + offset;
        self.name(start + 1, close);
        self.push(close, close + 1, SegmentKind::Token(Token::Punctuation));
        // Whatever trails a header is not a value and not a comment. Text is what it reads as.
        self.push(close + 1, end, SegmentKind::Content);
    }

    /// A name with its surrounding whitespace, which is content rather than part of the name.
    fn name(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let text = &self.source[start..end];
        let lead = text.len() - text.trim_start().len();
        // All whitespace, so the two trims overlap and there is no name between them.
        if lead == text.len() {
            self.push(start, end, SegmentKind::Content);
            return;
        }
        let trailing = text.len() - text.trim_end().len();
        self.push(start, start + lead, SegmentKind::Content);
        self.push(start + lead, end - trailing, SegmentKind::Token(Token::Key));
        self.push(end - trailing, end, SegmentKind::Content);
    }

    /// Where this line's separator is: the first `=` or `:`, whichever comes first.
    fn separator(&self, start: usize, end: usize) -> Option<usize> {
        self.source[start..end]
            .find(['=', ':'])
            .map(|offset| start + offset)
    }

    /// Everything after the separator, as one run.
    fn value(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let text = &self.source[start..end];
        let lead = text.len() - text.trim_start().len();
        // An empty value — `key =` with nothing after it, which is legal and common.
        if lead == text.len() {
            self.push(start, end, SegmentKind::Content);
            return;
        }
        let trailing = text.len() - text.trim_end().len();
        let (from, to) = (start + lead, end - trailing);
        self.push(start, from, SegmentKind::Content);

        let body = &self.source[from..to];
        let kind = match body {
            "true" | "false" | "yes" | "no" | "on" | "off" | "True" | "False" | "Yes" | "No" => {
                SegmentKind::Token(Token::Literal)
            }
            _ if quoted(body) => SegmentKind::Token(Token::String),
            _ if looks_numeric(body) => SegmentKind::Token(Token::Number),
            // Everything else is text, which is what most INI values are. Colouring an arbitrary
            // run of words as a string would say it was quoted when it is not.
            _ => SegmentKind::Content,
        };
        self.push(from, to, kind);
        self.push(to, end, SegmentKind::Content);
    }

    /// Notes that the open top-level block reaches at least this far.
    fn extend_block(&mut self, end: usize) {
        if let Some((_, last)) = &mut self.open_block {
            *last = end;
        }
    }

    /// Emits the open top-level block, if there is one.
    fn close_block(&mut self) {
        let Some((start, end)) = self.open_block.take() else {
            return;
        };
        if start >= end {
            return;
        }
        self.blocks.push(Block::new(
            BlockId::new(u32::try_from(self.blocks.len()).unwrap_or(u32::MAX)),
            BlockKind::Paragraph,
            ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
            0,
        ));
    }

    /// Appends a span, joining it to the one before if they are the same kind and meet.
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

/// Whether a value is wrapped in matching quotes.
fn quoted(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 2 && matches!(bytes[0], b'"' | b'\'') && bytes[bytes.len() - 1] == bytes[0]
}

/// Whether a value looks like a number.
///
/// Loosely, as the other lexers here are: `1.2.3` is what `1.23` looks like a keystroke before it is
/// finished, and a version number is more useful coloured as a number than as prose.
fn looks_numeric(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
}
