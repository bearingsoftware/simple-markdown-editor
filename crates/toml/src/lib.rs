//! TOML, lexed for highlighting rather than parsed for loading.
//!
//! # Why not the `toml` crate
//!
//! It deserializes, and this needs the two things a value cannot give: **spans**, because the editor
//! decorates the source in place and never re-serializes it, and **error tolerance**, because
//! `port = ` is what every entry looks like a keystroke before it is finished. The argument is
//! `mdedit-json`'s about `serde_json` and `mdedit-yaml`'s about `saphyr`, unchanged — a strict parser
//! refuses the half-typed document an editor spends its whole life looking at.
//!
//! # Why TOML does not share INI's lexer
//!
//! They look alike and are not alike. `key = "value"` is one token in INI — everything after the
//! separator is the value, quotes and all — and in TOML it is a quoted string with escapes,
//! surrounded by a syntax that also has arrays, inline tables, dates and three other string forms.
//! A shared lexer would have to be told which language it was, which is two lexers wearing one name.
//!
//! # What it classifies
//!
//! Tables and array tables, bare and dotted and quoted keys, basic and literal strings including
//! both multi-line forms, integers in four bases, floats, booleans, dates and times, arrays, inline
//! tables, and `#` comments. Nothing is concealed: TOML's brackets and quotes *are* the document.
//!
//! Approximated, deliberately: **anything starting with a digit or a sign is a number**, which makes
//! `1979-05-27` a number and `1.2.3` a number too. The second is what `1.23` looks like a keystroke
//! before it is finished, and colouring it as a number says more than colouring it as prose. It
//! mis-*colours* rather than mis-*covers*, which is the distinction that matters.
//!
//! # A table header at column zero is a safe restart point
//!
//! So this answers [`analyze_region`](Language::analyze_region) rather than sending the caller back
//! to a whole-document parse. The state a TOML lexer cannot recover from an arbitrary offset is
//! whether it is inside a multi-line string or an unclosed array, and a line that begins a block is
//! provably inside neither — that is what [`Analysis::blocks`] is defined by here.
//!
//! # The invariant
//!
//! The same gate every language here has: **the segments tile the source exactly**.
//!
//! ```
//! # use mdedit_toml::Toml;
//! # use mdedit_language::Language;
//! let src = "# a note\n[server]\nhost = \"localhost\"\nports = [80, 443]\n";
//! let rebuilt: String = Toml
//!     .analyze(src)
//!     .segments
//!     .iter()
//!     .map(|s| s.span().slice(src))
//!     .collect();
//! assert_eq!(rebuilt, src);
//! ```

use mdedit_language::{Analysis, Block, BlockKind, Language, Segment, SegmentKind, Token};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};

/// TOML, as this editor renders it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Toml;

/// The one TOML there is, for a caller holding a `&'static dyn Language`.
pub static TOML: Toml = Toml;

impl Language for Toml {
    fn name(&self) -> &'static str {
        "TOML"
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
/// Never fails. See the [crate docs](crate) for the tiling guarantee and for what it understands.
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
/// block here begins at a column-zero line that is outside every multi-line string and has no array
/// or inline table open — which is exactly the state a fresh lexer starts in, and the whole of what
/// one cannot otherwise recover from a slice.
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

/// The delimiter closing an open multi-line string.
const BASIC: &str = "\"\"\"";
/// As [`BASIC`], for the literal form.
const LITERAL: &str = "'''";

struct Lexer<'a> {
    source: &'a str,
    segments: Vec<Segment>,
    blocks: Vec<Block>,
    /// The delimiter that will close the multi-line string currently open, if one is.
    multiline: Option<&'static str>,
    /// How many arrays and inline tables are open across line ends.
    brackets: usize,
    /// Where the current top-level block started, and the byte after its last line.
    open_block: Option<(usize, usize)>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        let estimate = source.len() / 10 + 8;
        Self {
            source,
            segments: Vec::with_capacity(estimate),
            blocks: Vec::new(),
            multiline: None,
            brackets: 0,
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
        // Read before anything on this line is lexed: a line that *opens* a string still began
        // outside one, and it is the state on entry that decides whether this is a restart point.
        let restartable = self.multiline.is_none() && self.brackets == 0;

        if let Some(delimiter) = self.multiline {
            self.continuation(start, end, delimiter);
            return;
        }

        let line = &self.source[start..end];
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        let body = line[indent..].trim_end_matches(['\n', '\r']);
        let body_end = start + indent + body.len();

        if restartable && indent == 0 && !body.is_empty() {
            self.close_block();
            self.open_block = Some((start, end));
        } else {
            self.extend_block(end);
        }

        self.push(start, start + indent, SegmentKind::Content);
        self.body(start + indent, body_end);
        self.push(body_end, end, SegmentKind::Content);
    }

    /// A line inside a multi-line string, which ends it or does not.
    fn continuation(&mut self, start: usize, end: usize, delimiter: &'static str) {
        self.extend_block(end);
        let Some(offset) = self.source[start..end].find(delimiter) else {
            // Still inside: the whole line, newline included, is the string's content.
            self.push(start, end, SegmentKind::Token(Token::String));
            return;
        };
        let stop = start + offset + delimiter.len();
        self.multiline = None;
        self.push(start, stop, SegmentKind::Token(Token::String));

        let body_end = stop + self.source[stop..end].trim_end_matches(['\n', '\r']).len();
        self.values(stop, body_end);
        self.push(body_end, end, SegmentKind::Content);
    }

    /// The part of a line that is not its indent or its line ending.
    fn body(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        // An array or inline table opened on an earlier line: this line is more of its contents,
        // not a new entry.
        if self.brackets > 0 {
            self.values(start, end);
            return;
        }

        let text = &self.source[start..end];
        if text.starts_with('#') {
            self.push(start, end, SegmentKind::Token(Token::Comment));
            return;
        }
        if text.starts_with('[') {
            self.header(start, end);
            return;
        }

        match self.assignment(start, end) {
            Some(equals) => {
                self.key_run(start, equals);
                self.push(equals, equals + 1, SegmentKind::Token(Token::Punctuation));
                self.values(equals + 1, end);
            }
            // No separator yet, which is what a line looks like while its key is being typed.
            None => self.values(start, end),
        }
    }

    /// `[table]` or `[[array of tables]]`, and whatever follows on the line.
    fn header(&mut self, start: usize, end: usize) {
        let double = self.source[start..end].starts_with("[[");
        let closer = if double { "]]" } else { "]" };
        let open = start + closer.len();
        self.push(start, open, SegmentKind::Token(Token::Punctuation));

        let Some(offset) = self.source[open..end].find(closer) else {
            // A header that never closes: everything typed so far is still the name.
            self.key_run(open, end);
            return;
        };
        let close = open + offset;
        self.key_run(open, close);
        let after = close + closer.len();
        self.push(close, after, SegmentKind::Token(Token::Punctuation));
        self.values(after, end);
    }

    /// Where this line's `=` is, if it has one outside quotes.
    fn assignment(&self, start: usize, end: usize) -> Option<usize> {
        let bytes = &self.source.as_bytes()[start..end];
        let mut quote: Option<u8> = None;
        let mut at = 0;
        while at < bytes.len() {
            let byte = bytes[at];
            match quote {
                Some(open) => {
                    if byte == b'\\' && open == b'"' {
                        at += 1;
                    } else if byte == open {
                        quote = None;
                    }
                }
                None => match byte {
                    b'\'' | b'"' => quote = Some(byte),
                    b'#' => return None,
                    b'=' => return Some(start + at),
                    _ => {}
                },
            }
            at += 1;
        }
        None
    }

    /// A dotted key path: `a.b."c d"`.
    fn key_run(&mut self, start: usize, end: usize) {
        let mut at = start;
        while at < end {
            let byte = self.source.as_bytes()[at];
            match byte {
                b'.' => {
                    self.push(at, at + 1, SegmentKind::Token(Token::Punctuation));
                    at += 1;
                }
                b'"' | b'\'' => {
                    let stop = self.quoted_end(at, end);
                    self.push(at, stop, SegmentKind::Token(Token::Key));
                    at = stop;
                }
                _ if byte.is_ascii_whitespace() => {
                    let stop = self.whitespace_end(at, end);
                    self.push(at, stop, SegmentKind::Content);
                    at = stop;
                }
                _ => {
                    let stop = self.word_end(at, end, true);
                    self.push(at, stop, SegmentKind::Token(Token::Key));
                    at = stop;
                }
            }
        }
    }

    /// Everything after an `=`, and everything inside an array or inline table.
    fn values(&mut self, start: usize, end: usize) {
        let mut at = start;
        while at < end {
            let byte = self.source.as_bytes()[at];
            match byte {
                b'#' => {
                    self.push(at, end, SegmentKind::Token(Token::Comment));
                    return;
                }
                b'[' | b'{' => {
                    self.push(at, at + 1, SegmentKind::Token(Token::Punctuation));
                    self.brackets = self.brackets.saturating_add(1);
                    at += 1;
                }
                b']' | b'}' => {
                    self.push(at, at + 1, SegmentKind::Token(Token::Punctuation));
                    self.brackets = self.brackets.saturating_sub(1);
                    at += 1;
                }
                // No `.` here, deliberately: in a value a dot belongs to the float it is inside, and
                // a dotted key in an inline table is one run that `scalar` classifies as a key.
                b',' | b'=' => {
                    self.push(at, at + 1, SegmentKind::Token(Token::Punctuation));
                    at += 1;
                }
                b'"' | b'\'' => {
                    // The one construct that can outlive the line, which is why this returns.
                    if let Some(stop) = self.string(at, end) {
                        at = stop;
                    } else {
                        return;
                    }
                }
                _ if byte.is_ascii_whitespace() => {
                    let stop = self.whitespace_end(at, end);
                    self.push(at, stop, SegmentKind::Content);
                    at = stop;
                }
                _ => {
                    let stop = self.word_end(at, end, false);
                    self.scalar(at, stop, end);
                    at = stop;
                }
            }
        }
    }

    /// A string starting at `at`. `None` when it is a multi-line one that outlives this line.
    fn string(&mut self, at: usize, end: usize) -> Option<usize> {
        let quote = self.source.as_bytes()[at];
        let delimiter = if quote == b'"' { BASIC } else { LITERAL };

        if !self.source[at..].starts_with(delimiter) {
            let stop = self.quoted_end(at, end);
            self.push(at, stop, SegmentKind::Token(Token::String));
            return Some(stop);
        }

        // Searched across the whole source rather than the line, so a `"""` that closes three lines
        // down is found and one that never closes leaves the string open, which is what it is.
        let body = at + delimiter.len();
        match self.source[body..].find(delimiter) {
            Some(offset) if body + offset + delimiter.len() <= end => {
                let stop = body + offset + delimiter.len();
                self.push(at, stop, SegmentKind::Token(Token::String));
                Some(stop)
            }
            _ => {
                self.push(at, end, SegmentKind::Token(Token::String));
                self.multiline = Some(delimiter);
                None
            }
        }
    }

    /// A bare run, classified by what it looks like.
    ///
    /// `line_end` is where the line's body stops, so a key inside an inline table can be told from a
    /// value by looking for the `=` that would follow it.
    fn scalar(&mut self, start: usize, end: usize, line_end: usize) {
        let text = &self.source[start..end];
        let kind = match text {
            "true" | "false" | "inf" | "nan" | "+inf" | "-inf" | "+nan" | "-nan" => Token::Literal,
            _ if self.followed_by_equals(end, line_end) => Token::Key,
            _ if looks_numeric(text) => Token::Number,
            // A bare word that is neither. Prose is what it reads as, and it is what half a
            // half-typed `true` is.
            _ => return self.push(start, end, SegmentKind::Content),
        };
        self.push(start, end, SegmentKind::Token(kind));
    }

    /// Whether the next thing after `at` on this line is an `=`, which makes what precedes it a key.
    fn followed_by_equals(&self, at: usize, end: usize) -> bool {
        self.source[at..end]
            .trim_start_matches([' ', '\t'])
            .starts_with('=')
    }

    /// The byte after the quoted run starting at `at`, or `end` if it never closes on this line.
    fn quoted_end(&self, at: usize, end: usize) -> usize {
        let quote = self.source.as_bytes()[at];
        let bytes = &self.source.as_bytes()[at..end];
        let mut offset = 1;
        while offset < bytes.len() {
            // Only a basic string has escapes. `'\'` is a complete literal string in TOML.
            if bytes[offset] == b'\\' && quote == b'"' {
                offset += 1;
            } else if bytes[offset] == quote {
                return self.boundary(at + offset + 1);
            }
            offset += 1;
        }
        end
    }

    fn whitespace_end(&self, at: usize, end: usize) -> usize {
        let run = &self.source[at..end];
        at + (run.len() - run.trim_start_matches([' ', '\t']).len()).max(1)
    }

    /// Where a bare run ends. Never returns `at`, so the walk always advances.
    ///
    /// `dotted` says whether a `.` ends it. In a key path it does — `a.b` is two names and a
    /// separator. In a value it does not, because there it is the point in `3.14`.
    fn word_end(&self, at: usize, end: usize, dotted: bool) -> usize {
        let mut stop = end;
        for (offset, c) in self.source[at..end].char_indices() {
            let separates = c.is_whitespace()
                || matches!(c, ',' | '=' | '[' | ']' | '{' | '}' | '#' | '"' | '\'')
                || (dotted && c == '.');
            if offset > 0 && separates {
                stop = at + offset;
                break;
            }
        }
        self.boundary(stop.max(at + 1)).min(end.max(at + 1))
    }

    /// The character boundary at or after `at`, so a span never splits a character.
    fn boundary(&self, at: usize) -> usize {
        let mut at = at.min(self.source.len());
        while at < self.source.len() && !self.source.is_char_boundary(at) {
            at += 1;
        }
        at
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
            // As JSON's members and YAML's entries are: `Paragraph` is everything the renderer has
            // ever needed a block to be.
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

/// Whether a bare run looks like a number, a date or a time.
///
/// Loosely, as JSON's and YAML's do. `1979-05-27T07:32:00Z` is a number here and so is `1.2.3`,
/// which is what `1.23` looks like a keystroke before it is finished.
fn looks_numeric(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | ':' | '_' | '.'))
}
