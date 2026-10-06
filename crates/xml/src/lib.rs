//! XML, lexed for highlighting rather than parsed for loading.
//!
//! # Why hand-written, again
//!
//! The argument is `mdedit-yaml`'s and it is not repeated here at length: a real parser refuses
//! half-typed input, and half-typed input is what a file being edited *is*. `<person id=` is what
//! every attribute looks like a keystroke before it is finished, and a parser that stopped there
//! would leave the rest of the document uncoloured at exactly the moment somebody is looking at it.
//!
//! The two candidate crates fail on the same ground. `quick-xml` and `roxmltree` both surface the
//! first ill-formed construct as an error and stop; neither will hand back spans for the bytes after
//! it. `tree-sitter` is rejected for the reason `crates/yaml/src/lib.rs` records — a C grammar over
//! untrusted file contents reintroduces the CVE class `AGENTS.md` names.
//!
//! # Nesting is a counter, and that is the security decision
//!
//! XML nests without limit, so the obvious recursive descent overflows the stack on
//! `<a><a><a>…` — a file, from the internet, opened in an editor. Safe Rust does not save you from a
//! stack overflow, and `AGENTS.md` puts file parsing at a trust boundary. MDEDIT-47's JSON validator
//! is the precedent: an explicit container stack on the heap rather than the call stack.
//!
//! This goes one step further and keeps a **counter** rather than a stack of names, because nothing
//! reads the names. Highlighting a `</b>` does not depend on which element it closes, and this crate
//! has no [`validate`](mdedit_language::Language::validate) to tell it that the wrong one was typed.
//! So a depth bomb costs one `usize` that never grows, which is a stronger answer than a bounded
//! `Vec` and needs no cap to be safe. The counter saturates rather than wrapping, so
//! `overflow-checks` cannot turn a pathological file into a panic.
//!
//! # What it classifies
//!
//! Element names, attribute names, attribute values, comments, CDATA, processing instructions,
//! doctype declarations, entity references, and the punctuation holding them together. Text between
//! tags is content, and nothing is concealed — an XML tag *is* the document rather than scaffolding
//! for prose, which is the same reason JSON and YAML answer
//! [`conceals`](mdedit_language::Language::conceals) with `false`.
//!
//! An element name is [`Token::Tag`] and an attribute name is [`Token::Key`]. That was the one
//! decision left open on MDEDIT-94, and it is settled in favour of the new variant: in
//! `<a href="x">` the two names are different things, every highlighter colours them differently,
//! and collapsing both into `Key` would draw the landmark and the detail identically.
//!
//! # The invariant
//!
//! The same gate every language here has: **the segments tile the source exactly**.
//!
//! ```
//! # use mdedit_xml::Xml;
//! # use mdedit_language::Language;
//! let src = "<!-- a note -->\n<config debug=\"true\">text &amp; more</config>\n";
//! let rebuilt: String = Xml
//!     .analyze(src)
//!     .segments
//!     .iter()
//!     .map(|s| s.span().slice(src))
//!     .collect();
//! assert_eq!(rebuilt, src);
//! ```

use mdedit_language::{Analysis, Block, BlockKind, Language, Segment, SegmentKind, Token};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan};

/// XML, as this editor renders it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Xml;

/// The one XML there is, for a caller holding a `&'static dyn Language`.
pub static XML: Xml = Xml;

impl Language for Xml {
    fn name(&self) -> &'static str {
        "XML"
    }

    fn analyze(&self, source: &str) -> Analysis {
        analyze(source)
    }

    /// Always `None` — **there is no safe place to restart**, which is what the trait's `None` is
    /// for.
    ///
    /// Every other language here recovers its state from a column-zero boundary: JSON reads its
    /// nesting depth from one, YAML reads that it is not inside a block scalar. XML has no such
    /// boundary. A line beginning `</item>` is at column zero and says nothing about how deep it
    /// is, and depth is the whole of the state — an element at depth one is a block and the same
    /// element at depth two is not. So the honest answer is a whole-document parse that is merely
    /// slower, rather than a fast wrong one.
    fn analyze_region(&self, source: &str, region: ByteSpan) -> Option<Analysis> {
        let _ = (source, region);
        None
    }

    fn conceals(&self) -> bool {
        false
    }
}

/// Lexes `source` into coloured spans and blocks.
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

/// What ends the run of attributes being lexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terminator {
    /// A start or end tag: `>`, or `/>`.
    Tag,
    /// A processing instruction: `?>`.
    Instruction,
}

struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    at: usize,
    segments: Vec<Segment>,
    blocks: Vec<Block>,
    /// How many elements are open. See the [crate docs](crate) for why this is not a stack.
    depth: usize,
    /// Where the open child of the root element started, if one is open.
    child: Option<usize>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        let estimate = source.len() / 8 + 8;
        Self {
            source,
            bytes: source.as_bytes(),
            at: 0,
            segments: Vec::with_capacity(estimate),
            blocks: Vec::new(),
            depth: 0,
            child: None,
        }
    }

    fn run(&mut self) {
        while self.at < self.bytes.len() {
            if self.bytes[self.at] == b'<' {
                self.markup();
            } else {
                self.text();
            }
        }
    }

    /// Character data between tags, and the entity references in it.
    fn text(&mut self) {
        let mut run = self.at;
        while self.at < self.bytes.len() && self.bytes[self.at] != b'<' {
            if self.bytes[self.at] == b'&'
                && let Some(end) = self.entity(self.at)
            {
                self.push(run, self.at, SegmentKind::Content);
                self.push(self.at, end, SegmentKind::Token(Token::Literal));
                self.at = end;
                run = end;
                continue;
            }
            self.at += 1;
        }
        self.push(run, self.at, SegmentKind::Content);
    }

    /// Whatever the `<` at `self.at` opens.
    fn markup(&mut self) {
        let start = self.at;
        let rest = &self.source[start..];
        if rest.starts_with("<!--") {
            self.comment(start);
        } else if rest.starts_with("<![CDATA[") {
            self.cdata(start);
        } else if rest.starts_with("<!") {
            self.declaration(start);
        } else if rest.starts_with("<?") {
            self.instruction(start);
        } else if rest.starts_with("</") {
            self.end_tag(start);
        } else if self.is_name_start(start + 1) {
            self.start_tag(start);
        } else {
            // A `<` that opens nothing: a stray one in prose, or the first keystroke of a tag. Text
            // is what it reads as either way, and it is one byte so the walk still advances.
            self.push(start, start + 1, SegmentKind::Content);
            self.at = start + 1;
        }
    }

    /// `<!-- … -->`, to the end of the source if it never closes.
    fn comment(&mut self, start: usize) {
        let body = start + 4;
        let end = self.source[body..]
            .find("-->")
            .map_or(self.bytes.len(), |offset| body + offset + 3);
        self.push(start, end, SegmentKind::Token(Token::Comment));
        self.at = end;
    }

    /// `<![CDATA[ … ]]>`, whose body is literal text by definition.
    fn cdata(&mut self, start: usize) {
        let open = start + 9;
        let (body, end) = self.source[open..]
            .find("]]>")
            .map_or((self.bytes.len(), self.bytes.len()), |offset| {
                (open + offset, open + offset + 3)
            });
        self.push(start, open, SegmentKind::Token(Token::Punctuation));
        self.push(open, body, SegmentKind::Content);
        self.push(body, end, SegmentKind::Token(Token::Punctuation));
        self.at = end;
    }

    /// `<!DOCTYPE …>` and its relatives.
    ///
    /// The terminator is the first `>` outside both quotes and the bracketed internal subset, which
    /// may legally contain one.
    fn declaration(&mut self, start: usize) {
        let name = start + 2;
        let name_end = self.name_end(name);
        self.push(start, name, SegmentKind::Token(Token::Punctuation));
        self.push(name, name_end, SegmentKind::Token(Token::Tag));
        self.at = name_end;

        let mut quote: Option<u8> = None;
        let mut brackets = 0_usize;
        let mut run = self.at;
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            match quote {
                Some(open) => {
                    self.at += 1;
                    if byte == open {
                        self.push(run, self.at, SegmentKind::Token(Token::String));
                        run = self.at;
                        quote = None;
                    }
                }
                None => match byte {
                    b'"' | b'\'' => {
                        self.push(run, self.at, SegmentKind::Content);
                        run = self.at;
                        quote = Some(byte);
                        self.at += 1;
                    }
                    b'[' => {
                        brackets += 1;
                        self.at += 1;
                    }
                    b']' => {
                        brackets = brackets.saturating_sub(1);
                        self.at += 1;
                    }
                    b'>' if brackets == 0 => {
                        self.push(run, self.at, SegmentKind::Content);
                        self.push(self.at, self.at + 1, SegmentKind::Token(Token::Punctuation));
                        self.at += 1;
                        return;
                    }
                    _ => self.at += 1,
                },
            }
        }
        self.push(run, self.at, SegmentKind::Content);
    }

    /// `<?target … ?>`, whose pseudo-attributes lex as a tag's do.
    fn instruction(&mut self, start: usize) {
        let name = start + 2;
        let name_end = self.name_end(name);
        self.push(start, name, SegmentKind::Token(Token::Punctuation));
        self.push(name, name_end, SegmentKind::Token(Token::Tag));
        self.at = name_end;
        self.attributes(Terminator::Instruction);
    }

    /// `</name>`, which closes an element and may close a block.
    fn end_tag(&mut self, start: usize) {
        let name = start + 2;
        let name_end = self.name_end(name);
        self.push(start, name, SegmentKind::Token(Token::Punctuation));
        self.push(name, name_end, SegmentKind::Token(Token::Tag));
        self.at = name_end;
        self.attributes(Terminator::Tag);

        // Saturating rather than checked: a document with more end tags than start tags is one
        // somebody is halfway through cutting and pasting, not an error to report.
        self.depth = self.depth.saturating_sub(1);
        if self.depth == 1 {
            self.finish_child(self.at);
        }
    }

    /// `<name …>` or `<name … />`.
    fn start_tag(&mut self, start: usize) {
        let name = start + 1;
        let name_end = self.name_end(name);
        self.push(start, name, SegmentKind::Token(Token::Punctuation));
        self.push(name, name_end, SegmentKind::Token(Token::Tag));
        self.at = name_end;
        let empty = self.attributes(Terminator::Tag);

        if self.depth == 1 && self.child.is_none() {
            self.child = Some(start);
        }
        if empty {
            if self.depth == 1 {
                self.finish_child(self.at);
            }
        } else {
            self.depth = self.depth.saturating_add(1);
        }
    }

    /// Everything between a tag's name and the punctuation ending it, and whether it closed itself.
    fn attributes(&mut self, terminator: Terminator) -> bool {
        let mut expect_value = false;
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            match byte {
                b'>' => {
                    self.push(self.at, self.at + 1, SegmentKind::Token(Token::Punctuation));
                    self.at += 1;
                    return false;
                }
                b'/' if terminator == Terminator::Tag && self.peek(self.at + 1) == Some(b'>') => {
                    self.push(self.at, self.at + 2, SegmentKind::Token(Token::Punctuation));
                    self.at += 2;
                    return true;
                }
                b'?' if terminator == Terminator::Instruction
                    && self.peek(self.at + 1) == Some(b'>') =>
                {
                    self.push(self.at, self.at + 2, SegmentKind::Token(Token::Punctuation));
                    self.at += 2;
                    return false;
                }
                // A tag that never closed, which is what one looks like while it is being typed.
                // The `<` belongs to whatever comes next rather than being swallowed by this.
                b'<' => return false,
                b'=' => {
                    self.push(self.at, self.at + 1, SegmentKind::Token(Token::Punctuation));
                    self.at += 1;
                    expect_value = true;
                }
                b'"' | b'\'' => {
                    self.value(byte);
                    expect_value = false;
                }
                _ if byte.is_ascii_whitespace() => {
                    let start = self.at;
                    while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
                        self.at += 1;
                    }
                    self.push(start, self.at, SegmentKind::Content);
                }
                _ => {
                    let start = self.at;
                    let end = self.word_end(start);
                    // An unquoted value is not well-formed XML and is common mid-edit. It is still a
                    // value, and colouring it as one says so better than colouring it as a name.
                    let kind = if expect_value {
                        Token::String
                    } else {
                        Token::Key
                    };
                    self.push(start, end, SegmentKind::Token(kind));
                    self.at = end;
                    expect_value = false;
                }
            }
        }
        false
    }

    /// A quoted attribute value, and the entity references in it.
    fn value(&mut self, quote: u8) {
        let start = self.at;
        self.at += 1;
        let mut run = start;
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            if byte == quote {
                self.at += 1;
                break;
            }
            if byte == b'&'
                && let Some(end) = self.entity(self.at)
            {
                self.push(run, self.at, SegmentKind::Token(Token::String));
                self.push(self.at, end, SegmentKind::Token(Token::Literal));
                self.at = end;
                run = end;
                continue;
            }
            // A newline ends an unclosed value. XML allows one inside a value and nobody writes one;
            // an opening quote with no closer is a keystroke, and without this the rest of the file
            // turns into a string behind the caret.
            if byte == b'\n' {
                break;
            }
            self.at += 1;
        }
        self.push(run, self.at, SegmentKind::Token(Token::String));
    }

    /// The end of the entity reference starting at `at`, if that is what it is.
    ///
    /// Bounded, so a lone `&` in text costs a handful of bytes rather than a scan to the end of the
    /// file. MDEDIT-99 is what an unbounded one costs on the keystroke path.
    fn entity(&self, at: usize) -> Option<usize> {
        /// Longer than `&#x10FFFF;` with room to spare, and shorter than anything worth scanning.
        const LONGEST: usize = 32;

        let end = at.saturating_add(LONGEST).min(self.bytes.len());
        let mut offset = at + 1;
        if offset < end && self.bytes[offset] == b'#' {
            offset += 1;
        }
        let name = offset;
        while offset < end && self.bytes[offset].is_ascii_alphanumeric() {
            offset += 1;
        }
        if offset > name && offset < end && self.bytes[offset] == b';' {
            Some(offset + 1)
        } else {
            None
        }
    }

    /// Records that the root's open child reached `end`, and that it is a block.
    ///
    /// **A block is a child of the root**, which is JSON's rule rather than a new one: a whole
    /// document is one element and laying it out as a single unit would defeat the per-block cache,
    /// while every element at every depth would nest blocks the renderer has no use for.
    fn finish_child(&mut self, end: usize) {
        let Some(start) = self.child.take() else {
            return;
        };
        if start >= end {
            return;
        }
        self.blocks.push(Block::new(
            BlockId::new(u32::try_from(self.blocks.len()).unwrap_or(u32::MAX)),
            // As JSON's members are: `Paragraph` is all the renderer has ever needed a block to be,
            // and an `XmlElement` kind would make every renderer learn a case to treat it the same.
            BlockKind::Paragraph,
            ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
            0,
        ));
    }

    /// Whether a name could start at `at`.
    fn is_name_start(&self, at: usize) -> bool {
        self.source
            .get(at..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == ':')
    }

    /// Where the name starting at `at` ends. May be `at`, for a `<!>` with no name in it.
    fn name_end(&self, at: usize) -> usize {
        let Some(rest) = self.source.get(at..) else {
            return at;
        };
        let mut end = at;
        for (offset, c) in rest.char_indices() {
            if !(c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.')) {
                break;
            }
            end = at + offset + c.len_utf8();
        }
        end
    }

    /// Where an unquoted run inside a tag ends.
    ///
    /// Never returns `at`: a lexer that can return the position it was given does not terminate, and
    /// the input that finds it is whatever nobody thought of.
    fn word_end(&self, at: usize) -> usize {
        let Some(rest) = self.source.get(at..) else {
            return at;
        };
        let mut end = at;
        for (offset, c) in rest.char_indices() {
            if offset > 0
                && (c.is_whitespace() || matches!(c, '=' | '>' | '/' | '?' | '"' | '\'' | '<'))
            {
                return at + offset;
            }
            end = at + offset + c.len_utf8();
        }
        end
    }

    fn peek(&self, at: usize) -> Option<u8> {
        self.bytes.get(at).copied()
    }

    /// Appends a span, joining it to the one before if they are the same kind and meet.
    ///
    /// Coalescing for the reason `mdedit-json` does it: two adjacent runs of one kind and one run of
    /// that kind cover the same bytes, and the tiling property is stated in terms of the coalesced
    /// form.
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
