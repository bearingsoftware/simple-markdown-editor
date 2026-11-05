//! YAML, lexed for highlighting rather than parsed for loading.
//!
//! # The parser decision, and the evidence for it
//!
//! MDEDIT-31 required this to be decided with evidence rather than assumed. Four options were
//! measured on 2026-07-31, against Rust 1.87 — the project's MSRV at the time, raised to 1.90 by
//! MDEDIT-189.
//!
//! **A real YAML parser is disqualified by error tolerance**, the same way `serde_json` was for JSON.
//! `saphyr-parser` was run over four inputs: valid YAML, a half-typed document, one with inconsistent
//! indentation, and one indented with a tab. It parsed the first and **failed the other three** —
//! "expected ',' or ']'", "did not find expected key", "tabs disallowed". Those three are not
//! pathological; they are what a file looks like while it is being written, which is most of the time
//! anybody is looking at it.
//!
//! **`syntect` was disqualified on the MSRV, and that ground has since gone.** `plist` and `time`,
//! both in its tree, required 1.88 against a floor of 1.87, and raising `rust-version` was not a
//! decision to take as a side effect of highlighting YAML. **The floor is 1.90 as of MDEDIT-189**,
//! taken for `muda` and `encoding_rs`, so the objection recorded here no longer holds — and
//! `AGENTS.md` already names syntect for code fences, where it would pay for itself twice. What it
//! was *not* measured against is error tolerance on a half-typed file, which is what disqualified
//! the real parser above and is the question to ask before adopting it. Nothing here has changed its
//! mind; the blocker has merely stopped being the MSRV. **MDEDIT-190** adopted syntect, with fancy-regex, for
//! fenced code — and it keeps this lexer, because what a lexer here guarantees is that concatenating
//! its spans reproduces the source byte-for-byte, and syntect returns styles rather than spans with
//! that property.
//!
//! **`tree-sitter` passes the MSRV check** — 20 dependencies, a 42-second cold build — and is
//! rejected on a different ground. It is a C parser, and `AGENTS.md` is explicit that file parsing is
//! a trust boundary and that the 2023 Notepad++ CVEs were heap overflows of exactly the class Rust
//! removes "provided we don't reintroduce it in `unsafe`". Handing untrusted file contents to a C
//! grammar reintroduces it wholesale, and not in code this project can review. `AGENTS.md` already
//! records a stack-corrupting overflow in `tree-sitter-md`'s `serialize()` as the reason that grammar
//! was rejected, which is evidence about the class rather than about Markdown.
//!
//! So: **hand-written**, as JSON's is. Zero dependencies, no C, error-tolerant by construction, and
//! exactly the spans the renderer needs. MDEDIT-30 proved the approach on a smaller grammar.
//!
//! # What this does and does not understand
//!
//! It is a **highlighter**, not a parser, and the difference is worth stating because YAML's grammar
//! is large enough that "handles YAML" would be a claim this cannot support.
//!
//! Understood: comments, document markers, block mapping keys, block sequence entries, single- and
//! double-quoted scalars, anchors, aliases, tags, flow collections, numbers, booleans and nulls, and
//! block scalars including their indentation — so a `#` inside a `|` block is text, not a comment.
//!
//! Approximated: a key is text followed by `:` at the end of a line or `: ` anywhere before a
//! comment. That is what a key looks like in every YAML anybody writes, and it is wrong for the
//! explicit `? key` form and for a plain scalar containing a colon-space inside a flow collection.
//! Both mis-*colour* rather than mis-*cover* — the bytes are still all there, which is the invariant
//! that actually matters.
//!
//! Not understood at all: multi-document semantics beyond colouring the `---`, merge keys, and the
//! difference a tag makes to a value's type. None of them changes what is drawn.
//!
//! # The invariant
//!
//! The same gate every language here has: **the segments tile the source exactly**.
//!
//! ```
//! # use mdedit_yaml::Yaml;
//! # use mdedit_language::Language;
//! let src = "# a note\nname: value\nlist:\n  - 1\n  - two\n";
//! let rebuilt: String = Yaml
//!     .analyze(src)
//!     .segments
//!     .iter()
//!     .map(|s| s.span().slice(src))
//!     .collect();
//! assert_eq!(rebuilt, src);
//! ```

use mdedit_language::{Analysis, Block, BlockKind, Language, Segment, SegmentKind, Token};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};

/// YAML, as this editor renders it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Yaml;

/// The one YAML there is, for a caller holding a `&'static dyn Language`.
pub static YAML: Yaml = Yaml;

impl Language for Yaml {
    fn name(&self) -> &'static str {
        "YAML"
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
/// **`region` must begin at the start of a line at column zero — a block reported by a previous
/// [`analyze`] — or at byte zero.** That is what `mdedit_document`'s `scope_for` passes, and it is
/// what makes a fresh lexer correct here rather than merely convenient.
///
/// The state a YAML lexer cannot recover from a slice is whether it is inside a block scalar, and a
/// column-zero line is the one place it provably is not: a block scalar's content must be indented
/// further than the line that opened it, and that line is itself at column zero. So a region
/// starting at column zero starts outside every block scalar, with no indentation context to carry.
///
/// MDEDIT-31 declined this, reasoning that significant indentation means a region "cannot see that
/// it has become the body of the block above". That is true, and it is a statement about an edit
/// *outside* the region rather than about restarting inside it — the same thing an unclosed fence
/// does to Markdown, repaired the same way, by the whole-document parse `Document::reconcile` runs
/// when the typing stops.
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
    /// While inside a `|` or `>` block, the indent its content must exceed. Content of a block
    /// scalar is literal text: a `#` in it is not a comment and a `:` in it is not a key.
    block_scalar: Option<usize>,
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
            block_scalar: None,
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
            return;
        }

        if let Some(minimum) = self.block_scalar {
            if indent > minimum {
                // Still inside the block scalar: literal text, whatever it looks like.
                self.push(start, end, SegmentKind::Content);
                self.extend_block(end);
                return;
            }
            self.block_scalar = None;
        }

        // A line at column zero opens a new top-level block; a more-indented one continues it. That
        // is what makes a top-level mapping entry and everything under it one unit to lay out.
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

        // `---` and `...` separate documents. Only at column zero, which the caller has not checked,
        // but a `---` anywhere else is a plain scalar and colouring it as a marker is harmless.
        if text == "---" || text == "..." || text.starts_with("--- ") {
            let marker = start + 3;
            self.push(start, marker, SegmentKind::Token(Token::Punctuation));
            self.value(marker, end);
            return;
        }

        if text.starts_with('#') {
            self.push(start, end, SegmentKind::Token(Token::Comment));
            return;
        }

        let mut at = start;

        // Block sequence entries nest: `- - x` is a sequence in a sequence.
        while self.source[at..end].starts_with("- ") || &self.source[at..end] == "-" {
            let dash = at + 1;
            self.push(at, dash, SegmentKind::Token(Token::Punctuation));
            at = (dash + 1).min(end);
            self.push(dash, at, SegmentKind::Content);
        }

        if at >= end {
            return;
        }

        match self.key_end(at, end) {
            Some(colon) => {
                self.scalar(at, colon, Token::Key);
                self.push(colon, colon + 1, SegmentKind::Token(Token::Punctuation));
                self.value(colon + 1, end);
            }
            None => self.value(at, end),
        }
    }

    /// Where this line's key ends, if it has one.
    ///
    /// A key is text followed by `:` at the end of the line, or by `: ` before any comment. Quoted
    /// keys are honoured, so a colon inside quotes does not end one.
    fn key_end(&self, start: usize, end: usize) -> Option<usize> {
        let text = &self.source[start..end];
        let bytes = text.as_bytes();
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
                    // An unquoted `#` after whitespace starts a comment, and a key cannot follow it.
                    b'#' if at > 0 && bytes[at - 1].is_ascii_whitespace() => return None,
                    b':' if at + 1 == bytes.len() || bytes[at + 1].is_ascii_whitespace() => {
                        return Some(start + at);
                    }
                    _ => {}
                },
            }
            at += 1;
        }
        None
    }

    /// Everything after a key's colon: a scalar, a flow collection, or a block scalar indicator.
    fn value(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let text = &self.source[start..end];
        let lead = text.len() - text.trim_start().len();
        let at = start + lead;
        self.push(start, at, SegmentKind::Content);
        if at >= end {
            return;
        }

        let rest = &self.source[at..end];
        if rest.starts_with('#') {
            self.push(at, end, SegmentKind::Token(Token::Comment));
            return;
        }

        // `|` and `>`, with optional chomping and indentation indicators, open a block scalar whose
        // content runs until the indentation drops back.
        if rest.starts_with('|') || rest.starts_with('>') {
            let indicator = rest
                .find(|c: char| !matches!(c, '|' | '>' | '+' | '-' | '0'..='9'))
                .map_or(end, |offset| at + offset);
            self.push(at, indicator, SegmentKind::Token(Token::Punctuation));
            // The indent of the line this indicator is on. Content has to beat it.
            let line_start = self.source[..at].rfind('\n').map_or(0, |offset| offset + 1);
            self.block_scalar = Some(at - line_start - self.leading(line_start, at));
            self.value(indicator, end);
            return;
        }

        self.flow(at, end);
    }

    /// How much of `start..at` is not indent.
    fn leading(&self, start: usize, at: usize) -> usize {
        let line = &self.source[start..at];
        line.trim_start_matches([' ', '\t']).len()
    }

    /// A run that may contain flow collections, and the scalars inside them.
    fn flow(&mut self, start: usize, end: usize) {
        let mut at = start;
        while at < end {
            let rest = &self.source[at..end];
            let byte = rest.as_bytes()[0];
            match byte {
                b'[' | b']' | b'{' | b'}' | b',' => {
                    self.push(at, at + 1, SegmentKind::Token(Token::Punctuation));
                    at += 1;
                }
                b'#' if at > start && self.source.as_bytes()[at - 1].is_ascii_whitespace() => {
                    self.push(at, end, SegmentKind::Token(Token::Comment));
                    return;
                }
                b' ' | b'\t' => {
                    let run = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                    self.push(at, at + run, SegmentKind::Content);
                    at += run;
                }
                _ => {
                    // Never less than one character. A lexer that can return the position it was
                    // given does not terminate, and the input that triggers it is whatever nobody
                    // thought of — so this is a guarantee rather than a check.
                    let stop = self.token_end(at, end).max(self.boundary(at + 1)).min(end);
                    // Inside a flow collection a key is still `text:`, so check again here.
                    if self.source[stop..end].starts_with(':') {
                        self.scalar(at, stop, Token::Key);
                        self.push(stop, stop + 1, SegmentKind::Token(Token::Punctuation));
                        at = stop + 1;
                    } else {
                        self.scalar(at, stop, Token::String);
                        at = stop;
                    }
                }
            }
        }
    }

    /// Where the token starting at `at` ends.
    fn token_end(&self, at: usize, end: usize) -> usize {
        let bytes = &self.source.as_bytes()[at..end];
        match bytes[0] {
            quote @ (b'\'' | b'"') => {
                let mut offset = 1;
                while offset < bytes.len() {
                    if bytes[offset] == b'\\' && quote == b'"' {
                        offset += 1;
                    } else if bytes[offset] == quote {
                        return self.boundary(at + offset + 1);
                    }
                    offset += 1;
                }
                end
            }
            // An anchor, an alias or a tag is a single word: `&name`, `*name`, `!!str`. Unlike a
            // plain scalar it cannot contain a space, so `!!str 5` is a tag and a value rather than
            // one long name.
            b'&' | b'*' | b'!' => {
                let stop = bytes
                    .iter()
                    .position(|byte| {
                        byte.is_ascii_whitespace()
                            || matches!(byte, b',' | b'[' | b']' | b'{' | b'}')
                    })
                    .map_or(end, |offset| at + offset);
                self.boundary(stop.max(at + 1))
            }
            _ => {
                // A plain scalar *may* contain spaces — `key: hello world` is one scalar — so it
                // ends at a flow indicator or at ` #`, which is where a trailing comment starts.
                // Stopping at every space instead would break the common case.
                let flow = bytes
                    .iter()
                    .position(|byte| matches!(byte, b',' | b'[' | b']' | b'{' | b'}' | b':'))
                    .map_or(end, |offset| at + offset);
                // Only as far as the flow indicator. The token stops at whichever comes first, so a
                // ` #` beyond `flow` could never be used — and scanning for one to the end of the
                // line regardless is what made this quadratic (MDEDIT-99). `end` is the *line's*
                // end, and a minified flow document is one line with tens of thousands of scalars
                // on it, so every scalar was re-reading the rest of the file: 36 seconds to lex a
                // megabyte, on the keystroke path.
                //
                // Nothing is missed at the seam: a pair straddling `flow` would need `#` *at*
                // `flow`, and `flow` is where a flow indicator is. When there is none it is `end`,
                // and the window then covers everything.
                let stop = bytes[..flow - at]
                    .windows(2)
                    .position(|pair| pair[0].is_ascii_whitespace() && pair[1] == b'#')
                    .map_or(flow, |offset| at + offset);
                // A trailing space belongs to the whitespace run, not to the token.
                let trimmed = self.source[at..stop].trim_end();
                self.boundary(at + trimmed.len())
            }
        }
    }

    /// The character boundary at or after `at`, so a span never splits a character.
    fn boundary(&self, at: usize) -> usize {
        let mut at = at.min(self.source.len());
        while at < self.source.len() && !self.source.is_char_boundary(at) {
            at += 1;
        }
        at
    }

    /// A single scalar, classified by what it looks like.
    ///
    /// `fallback` is what an unremarkable one is: a key when it sits before a colon, a string when
    /// it sits after one.
    fn scalar(&mut self, start: usize, end: usize, fallback: Token) {
        if start >= end {
            return;
        }
        let text = &self.source[start..end];
        let token = match text.as_bytes()[0] {
            // An anchor, an alias or a tag. All three are names rather than data.
            b'&' | b'*' | b'!' => Token::Literal,
            b'\'' | b'"' => fallback,
            _ => match text {
                "true" | "false" | "null" | "~" | "True" | "False" | "Null" | "yes" | "no"
                | "on" | "off" => Token::Literal,
                _ if looks_numeric(text) => Token::Number,
                _ if fallback == Token::Key => Token::Key,
                // A plain scalar is prose. Drawn as ordinary text, which is what it reads as.
                _ => return self.push(start, end, SegmentKind::Content),
            },
        };
        self.push(start, end, SegmentKind::Token(token));
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
        if let Some(last) = self.segments.last_mut()
            && last.kind() == kind
            && last.span().end().get() == start
        {
            *last = last
                .clone()
                .moved_to(ByteSpan::new(last.span().start(), ByteOffset::new(end)));
            return;
        }
        self.segments.push(Segment::new(
            ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
            kind,
        ));
    }
}

/// Whether a scalar looks like a number.
///
/// Loosely, as JSON's lexer is: `1.2.3` is what `1.23` looks like a keystroke before it is finished,
/// and colouring it as a number is more useful than colouring it as prose.
fn looks_numeric(text: &str) -> bool {
    let mut chars = text.chars();
    let first = chars.next().is_some_and(|c| c.is_ascii_digit() || c == '-');
    first
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E' | 'x' | 'o'))
}
