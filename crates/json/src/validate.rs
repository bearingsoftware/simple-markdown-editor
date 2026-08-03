//! A strict JSON check, which is a different question from lexing and therefore different code.
//!
//! # Why this is not the lexer with an error type bolted on
//!
//! [`analyze`](crate::analyze) is error-tolerant *by design* and must stay that way — see the crate
//! docs. It classifies every byte and never fails, because `{"a":` is what a JSON document looks like
//! halfway through being typed and the rest of it still has to render. Adding failure to it would
//! trade the one property an editor needs for one it does not.
//!
//! So this is a second pass over the same bytes, asking "is this JSON?" rather than "how should this
//! be coloured?". It runs on the idle timer, never on the keystroke path.
//!
//! # It has an explicit stack, and that is a security decision rather than a style one
//!
//! JSON nests without limit, so the obvious recursive-descent checker overflows the stack on
//! `[[[[[…` — a file, from the internet, opened in an editor. `AGENTS.md` puts file parsing at a
//! trust boundary and names hand-rolled parsing as the CVE class Rust removes *provided we do not
//! reintroduce it*; a stack overflow is exactly that reintroduction, and safe Rust does not save you
//! from it. The container stack is a [`Vec`], so depth costs heap that the input has to pay for and
//! nothing worse.
//!
//! # It reports more than the first problem, up to a point
//!
//! One error per document would make the count in the status bar a boolean. So the state machine
//! recovers where recovery is unambiguous — a closer closes, a stray comma is skipped — and keeps
//! going. Past [`LIMIT`] it stops: nobody is fixing the two hundred and first error, and walking a
//! megabyte of prose that happens to be named `.json` to count the ways it is not JSON is work
//! nobody asked for.

use mdedit_language::Diagnostic;
use mdedit_offsets::{ByteOffset, ByteSpan};

/// The most problems one document reports.
///
/// A document with more than this is not being fixed error by error, and the count stops being
/// information somewhere long before here.
pub const LIMIT: usize = 200;

/// Everything wrong with `source`, in document order.
///
/// Empty for valid JSON — and empty for an **empty** document, which is not valid JSON but is what
/// every new file is. Shouting at somebody for not having typed anything yet is not diagnosis.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    Checker::new(source).run()
}

/// What the checker is expecting next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// A value, and only a value.
    Value,
    /// A value, or the `]` closing an array that turns out to be empty.
    ValueOrClose,
    /// A key, or the `}` closing an object that turns out to be empty.
    KeyOrClose,
    /// A key, because a comma promised another member.
    Key,
    /// The `:` between a key and its value.
    Colon,
    /// A value has just finished: a comma, a closer, or the end of the document.
    After,
}

/// What a token is, as far as structure is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Open(Bracket),
    Close(Bracket),
    Colon,
    Comma,
    /// A string. Kept apart from the other scalars because an object key must be one, and that is
    /// the only place in JSON where the difference between a string and a number is structural.
    Str,
    /// A number or a literal. Whatever is wrong with its own bytes is reported when it is scanned,
    /// so the state machine only cares that a value appeared.
    Scalar,
    /// A byte that begins nothing JSON has a name for.
    Garbage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bracket {
    Curly,
    Square,
}

struct Checker<'a> {
    source: &'a str,
    bytes: &'a [u8],
    at: usize,
    out: Vec<Diagnostic>,
    /// The containers currently open, with where each was opened, so an unclosed one can be pointed
    /// at rather than merely counted.
    stack: Vec<(Bracket, ByteSpan)>,
}

impl<'a> Checker<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            at: 0,
            out: Vec::new(),
            stack: Vec::new(),
        }
    }

    fn run(mut self) -> Vec<Diagnostic> {
        let mut expect = Expect::Value;
        // Set only by a comma, and cleared by anything else, so a closer can blame the comma that
        // promised something after it rather than blaming itself.
        let mut after_comma: Option<ByteSpan> = None;
        let mut any = false;

        while let Some((span, tok)) = self.next_token() {
            if self.out.len() >= LIMIT {
                return self.out;
            }
            any = true;
            expect = self.step(expect, span, tok, after_comma);
            after_comma = (tok == Tok::Comma).then_some(span);
        }

        self.finish(any);
        // In document order, which is not the order they are found in: an unclosed container is
        // noticed at the end of the file and points at its opener, near the start. "Jump to the
        // first error" has to mean the first one a reader would reach.
        self.out.sort_by_key(|diagnostic| diagnostic.span.start());
        self.out
    }

    /// Consumes one token against `expect` and answers what to expect next.
    fn step(
        &mut self,
        expect: Expect,
        span: ByteSpan,
        tok: Tok,
        after_comma: Option<ByteSpan>,
    ) -> Expect {
        match (expect, tok) {
            (Expect::Value | Expect::ValueOrClose, Tok::Open(bracket)) => self.open(bracket, span),
            (Expect::Value | Expect::ValueOrClose, Tok::Str | Tok::Scalar) => Expect::After,

            (Expect::KeyOrClose | Expect::Key, Tok::Str) => Expect::Colon,
            (Expect::Colon, Tok::Colon) => Expect::Value,

            (Expect::After, Tok::Comma) => {
                if self.stack.is_empty() {
                    self.report(span, "a `,` separates nothing here");
                    return Expect::After;
                }
                self.after_comma_expect()
            }
            (Expect::After | Expect::ValueOrClose | Expect::KeyOrClose, Tok::Close(bracket)) => {
                self.close(bracket, span)
            }

            // A closer where something was still promised. The comma is the culprit when there was
            // one — `[1,]` is a trailing comma, not a missing value. With nothing open, the closer
            // itself is the whole problem and `close` is about to say so.
            (Expect::Value | Expect::Key, Tok::Close(bracket)) => {
                if let Some(comma) = after_comma {
                    self.report(comma, "trailing comma");
                } else if !self.stack.is_empty() {
                    self.report(span, "something was expected before this");
                }
                self.close(bracket, span)
            }

            // Something that is not a string where a key belongs. Treated as the key anyway, because
            // that is plainly what was meant and it keeps the rest of the object checkable.
            (Expect::KeyOrClose | Expect::Key, Tok::Scalar | Tok::Open(_)) => {
                self.report(span, "an object key must be a string");
                match tok {
                    Tok::Open(bracket) => self.open(bracket, span),
                    _ => Expect::Colon,
                }
            }

            // A value where a colon belongs: `{"a" 1}`.
            (Expect::Colon, Tok::Str | Tok::Scalar | Tok::Open(_)) => {
                self.report(span, "a `:` belongs between a key and its value");
                match tok {
                    Tok::Open(bracket) => self.open(bracket, span),
                    _ => Expect::After,
                }
            }

            // Two values with nothing between them: `[1 2]`.
            (Expect::After, Tok::Str | Tok::Scalar | Tok::Open(_)) => {
                if self.stack.is_empty() {
                    self.report(span, "a JSON document holds one value");
                } else {
                    self.report(span, "a `,` belongs between one value and the next");
                }
                match tok {
                    Tok::Open(bracket) => self.open(bracket, span),
                    _ => Expect::After,
                }
            }

            (_, Tok::Garbage) => {
                self.report(span, "not JSON");
                expect
            }

            // Everything else is a separator with nothing to separate: a stray comma or colon, or a
            // closer at the top level. Reported and stepped over, which is the only recovery that
            // does not invent structure.
            (_, Tok::Close(bracket)) => {
                self.report(span, unexpected_close(bracket));
                expect
            }
            (_, Tok::Comma) => {
                self.report(span, "a `,` separates nothing here");
                expect
            }
            (_, Tok::Colon) => {
                self.report(span, "a `:` belongs after an object key");
                expect
            }
        }
    }

    fn open(&mut self, bracket: Bracket, span: ByteSpan) -> Expect {
        self.stack.push((bracket, span));
        match bracket {
            Bracket::Curly => Expect::KeyOrClose,
            Bracket::Square => Expect::ValueOrClose,
        }
    }

    /// Closes the innermost container, reporting a mismatch or a closer that closes nothing.
    fn close(&mut self, bracket: Bracket, span: ByteSpan) -> Expect {
        match self.stack.pop() {
            Some((open, _)) if open == bracket => Expect::After,
            Some((open, at)) => {
                self.report(span, mismatched_close(bracket));
                self.report(at, mismatched_open(open));
                Expect::After
            }
            None => {
                self.report(span, unexpected_close(bracket));
                // Still `After`: at the top level that means a further value is reported as a second
                // one, which is a better description of `1}2` than expecting a value would be.
                Expect::After
            }
        }
    }

    /// What a comma promises, which depends on the container it is in.
    fn after_comma_expect(&self) -> Expect {
        match self.stack.last() {
            Some((Bracket::Curly, _)) => Expect::Key,
            _ => Expect::Value,
        }
    }

    /// Reports what the end of the document leaves unfinished.
    ///
    /// Which is only ever an unclosed container. There is deliberately no "the document ends here"
    /// diagnostic: every other way of stopping part way through a value already has a better-placed
    /// one. A half-typed string reports itself where it opened, a half-typed number where it sits,
    /// and a stray `:` or `,` where it is. Adding an end-of-document message would put a second
    /// diagnostic at the end of the file for a problem the user has already been shown.
    fn finish(&mut self, any: bool) {
        // A document with nothing in it is not valid JSON, and saying so would mean every new file
        // opens with an error on it. Silence is the right answer for the empty case.
        if !any {
            return;
        }

        // Outermost first, which is document order, and each pointed at rather than counted.
        let unclosed: Vec<ByteSpan> = self.stack.iter().map(|(_, span)| *span).collect();
        for span in unclosed {
            self.report(span, "this is never closed");
        }
    }

    fn report(&mut self, span: ByteSpan, message: &'static str) {
        if self.out.len() < LIMIT {
            self.out.push(Diagnostic::new(span, message));
        }
    }

    // ---- scanning ----

    /// The next token, or `None` at the end. Reports anything wrong with the token's own bytes.
    fn next_token(&mut self) -> Option<(ByteSpan, Tok)> {
        self.skip_whitespace();
        let start = self.at;
        let byte = *self.bytes.get(self.at)?;

        let tok = match byte {
            b'{' => self.one(Tok::Open(Bracket::Curly)),
            b'}' => self.one(Tok::Close(Bracket::Curly)),
            b'[' => self.one(Tok::Open(Bracket::Square)),
            b']' => self.one(Tok::Close(Bracket::Square)),
            b':' => self.one(Tok::Colon),
            b',' => self.one(Tok::Comma),
            b'"' => self.string(start),
            // `+` and `.` begin no JSON token at all, but they are what somebody typing `+1` or
            // `.5` meant, and scanning them as a number says "not a number JSON accepts" over the
            // whole thing instead of "not JSON" over one invisible character.
            b'-' | b'+' | b'.' | b'0'..=b'9' => self.number(start),
            byte if byte.is_ascii_alphabetic() => self.literal(start),
            _ => {
                self.advance_one();
                Tok::Garbage
            }
        };
        Some((self.span(start), tok))
    }

    fn one(&mut self, tok: Tok) -> Tok {
        self.at += 1;
        tok
    }

    /// JSON's whitespace, which is four bytes and not `char::is_whitespace`.
    ///
    /// A no-break space is not whitespace to JSON — it is a byte that begins nothing, and reporting
    /// it is the point: it is what a document pasted out of a word processor is full of, and it is
    /// invisible.
    fn skip_whitespace(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// Steps over one whole character, so a multi-byte one is never split — a span that is not on a
    /// character boundary would panic the moment anything sliced or selected it.
    fn advance_one(&mut self) {
        self.at += 1;
        while self.at < self.bytes.len() && !self.source.is_char_boundary(self.at) {
            self.at += 1;
        }
    }

    fn span(&self, start: usize) -> ByteSpan {
        ByteSpan::new(ByteOffset::new(start), ByteOffset::new(self.at))
    }

    fn string(&mut self, start: usize) -> Tok {
        self.at += 1;
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                self.report(self.span(start), "this string is never closed");
                return Tok::Str;
            };
            match byte {
                b'"' => {
                    self.at += 1;
                    return Tok::Str;
                }
                b'\\' => self.escape(),
                // A raw control character has to be written as an escape. This is the one that bites
                // in practice: a literal newline inside a string looks fine and is not JSON.
                0x00..=0x1f => {
                    let at = self.at;
                    self.at += 1;
                    self.report(self.span(at), "a control character must be escaped");
                }
                _ => self.advance_one(),
            }
        }
    }

    /// The escape at `self.at`, which is known to be a backslash.
    fn escape(&mut self) {
        let start = self.at;
        self.at += 1;
        let Some(&byte) = self.bytes.get(self.at) else {
            // The unterminated-string report is coming; a backslash at the very end is that, not a
            // second problem.
            return;
        };
        match byte {
            b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.at += 1,
            b'u' => {
                self.at += 1;
                let hex = self.at;
                while self.at < hex + 4
                    && self.bytes.get(self.at).is_some_and(u8::is_ascii_hexdigit)
                {
                    self.at += 1;
                }
                if self.at != hex + 4 {
                    self.report(self.span(start), "`\\u` needs four hex digits");
                }
            }
            _ => {
                self.advance_one();
                self.report(self.span(start), "not an escape JSON knows");
            }
        }
    }

    /// A number, scanned loosely and then judged strictly.
    ///
    /// Loosely first because `1.2.3` is one wrong number rather than three tokens, and reporting it
    /// once where it sits is what a person can act on.
    fn number(&mut self, start: usize) -> Tok {
        self.at += 1;
        while matches!(
            self.bytes.get(self.at),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        let span = self.span(start);
        if !is_json_number(span.slice(self.source)) {
            self.report(span, "not a number JSON accepts");
        }
        Tok::Scalar
    }

    fn literal(&mut self, start: usize) -> Tok {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_alphabetic) {
            self.at += 1;
        }
        let span = self.span(start);
        if !matches!(span.slice(self.source), "true" | "false" | "null") {
            self.report(
                span,
                "only `true`, `false` and `null` are bare words in JSON",
            );
        }
        Tok::Scalar
    }
}

fn unexpected_close(bracket: Bracket) -> &'static str {
    match bracket {
        Bracket::Curly => "this `}` closes nothing",
        Bracket::Square => "this `]` closes nothing",
    }
}

fn mismatched_close(bracket: Bracket) -> &'static str {
    match bracket {
        Bracket::Curly => "this `}` closes something that is not an object",
        Bracket::Square => "this `]` closes something that is not an array",
    }
}

fn mismatched_open(bracket: Bracket) -> &'static str {
    match bracket {
        Bracket::Curly => "this `{` is closed by the wrong bracket",
        Bracket::Square => "this `[` is closed by the wrong bracket",
    }
}

/// Whether `text` is a number by JSON's grammar: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
///
/// Stricter than most people expect, and deliberately so — `01`, `1.`, `.5` and `+1` are all things
/// other formats accept and JSON does not, and an editor that stayed quiet about them would be
/// letting the user write a file the next tool rejects.
fn is_json_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;

    if bytes.get(at) == Some(&b'-') {
        at += 1;
    }
    // An integer part is required, and a leading zero may not be followed by more digits.
    match bytes.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => {
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
        }
        _ => return false,
    }

    if bytes.get(at) == Some(&b'.') {
        at += 1;
        if !bytes.get(at).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
    }

    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if !bytes.get(at).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
    }

    at == bytes.len()
}
