//! Toggling inline formatting, using the marker spans rather than guessing at them.
//!
//! # Why this is in this crate
//!
//! Because the segments are. Every editor that bolts Markdown buttons onto a text widget does
//! *un*-bolding with a regular expression, and every one of them gets nested emphasis wrong —
//! `**a _b_ c**` with the caret in `b` is where they take out the wrong pair of delimiters. This
//! editor already computes the delimiter spans exactly, property-tested against the CommonMark suite
//! (MDEDIT-12), so removing a construct is a matter of deleting two spans it can already point at.
//!
//! That is the differentiator, and it is only available here.
//!
//! # What a toggle does
//!
//! Four states, and they are the four a caret can be in:
//!
//! - **Inside the construct already** — remove exactly the two enclosing marker spans.
//! - **With a selection** — wrap it.
//! - **Mid-word, no selection** — wrap the word. What Typora's chords do, and what people expect
//!   when they bold a word by standing in it.
//! - **At whitespace, no selection** — insert the pair and park the caret between them, ready to
//!   type into.
//!
//! # Inside a code span, nothing happens
//!
//! The ticket offered a choice: no-op, or insert the delimiters literally as Typora does. It is a
//! no-op, and the reason is stronger than the precedent. `**` typed inside a code span is not
//! markup — the parser will not report it as a marker — so a second press could not find it to take
//! it away. **A toggle that cannot be un-toggled is not a toggle**, and it would break the
//! toggle-twice-is-byte-identical property that the rest of this module rests on.
//!
//! # Bytes
//!
//! Every edit goes through [`Document::replace`], so the byte-preservation invariant is untouched:
//! the only bytes that change are the delimiters, which the user asked for. Toggling twice restores
//! the original exactly, and that is property-tested rather than asserted.

use mdedit_language::{Marker, SegmentKind};
use mdedit_offsets::{ByteOffset, ByteSpan};

use crate::{Document, Selection};

/// Whether `text` looks enough like a URL to use as a link destination without asking.
///
/// Deliberately strict, and deliberately not a parser. This decides whether the editor acts on the
/// clipboard *without asking*, so a false positive turns a paste into a link the user did not want —
/// which is worse than a false negative, where they simply type the destination themselves.
///
/// So: a known scheme, no whitespace, and something after the scheme. `www.example.com` is not
/// accepted, because a bare host is also just a word somebody might have copied.
#[must_use]
pub fn looks_like_url(text: &str) -> bool {
    let text = text.trim();
    if text.split_whitespace().count() != 1 {
        return false;
    }
    ["https://", "http://", "mailto:", "ftp://"]
        .iter()
        .any(|scheme| text.len() > scheme.len() && text.starts_with(scheme))
}

/// An inline construct that can be toggled on and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inline {
    /// `**strong**`.
    Bold,
    /// `*emphasis*`.
    Italic,
    /// `~~struck through~~`.
    Strikethrough,
    /// `` `code` ``.
    Code,
}

impl Inline {
    /// The delimiter this construct is written with.
    ///
    /// One spelling each, on the way *in*. Markdown allows `_` as well as `*`, and this editor
    /// preserves whichever a document already uses — it never rewrites one into the other, because
    /// that would be changing bytes nobody asked to change. What it writes when *adding* emphasis is
    /// `*`, which is the commoner of the two and the one that works inside a word.
    #[must_use]
    pub fn delimiter(self) -> &'static str {
        match self {
            Self::Bold => "**",
            Self::Italic => "*",
            Self::Strikethrough => "~~",
            Self::Code => "`",
        }
    }

    /// The marker kind the parser reports for this construct.
    fn marker(self) -> Marker {
        match self {
            Self::Bold => Marker::Strong,
            Self::Italic => Marker::Emphasis,
            Self::Strikethrough => Marker::Strikethrough,
            Self::Code => Marker::CodeSpan,
        }
    }
}

impl Document {
    /// Turns `what` on or off around the caret or selection.
    ///
    /// Returns whether anything changed. See the module docs for the four cases and the code-span
    /// exception.
    ///
    /// One undo step, whichever case it takes: the two delimiters are two edits, and they go in a
    /// transaction so a single press undoes as a single press.
    pub fn toggle_inline(&mut self, what: Inline) -> bool {
        let caret = self.selection().head();

        // Inside a code span, `**` is not markup and a second press could not find it again. See the
        // module docs — a toggle that cannot be un-toggled is not one.
        if what != Inline::Code && self.inside_code_span(caret) {
            return false;
        }

        if let Some((open, close)) = self.enclosing(what, caret) {
            return self.unwrap_construct(open, close);
        }
        // An empty pair this module put in a moment ago. Not a construct, so `enclosing` cannot see
        // it — see `empty_pair`.
        if let Some(pair) = self.empty_pair(what) {
            let _ = self.replace(pair, "");
            return true;
        }
        let target = self.target_span();
        self.wrap(target, what)
    }

    /// Turns the selection, or the word at the caret, into a link.
    ///
    /// `url` goes in the destination slot. With `None` the slot is left empty and the caret parks
    /// inside it, ready to type or paste into — which is the whole of the "no UI needed" claim: the
    /// editor asks nothing, and the next thing typed is the destination.
    ///
    /// Returns whether anything changed.
    ///
    /// One undo step. Unlike a toggle this is not reversible by pressing again — a link is a
    /// construct with content the user supplied, and "un-linking" is a different operation nobody
    /// has asked for. Ctrl+Z is how it comes off.
    pub fn make_link(&mut self, url: Option<&str>) -> bool {
        let target = self.target_span();
        let destination = url.unwrap_or_default();
        // Built as one string so the closing half is a single edit: `](url)` has no interesting
        // interior, and splitting it would only make the transaction longer.
        let closing = format!("]({destination})");

        self.transact(|doc| {
            // Backwards, as everything here is: the opening edit would shift the closing one.
            let _ = doc.replace(ByteSpan::new(target.end(), target.end()), &closing);
            let _ = doc.replace(ByteSpan::new(target.start(), target.start()), "[");

            // The caret goes where the next thing to type goes. With a destination already supplied
            // that is after the whole construct; without one it is inside the empty parentheses.
            let after_open = target.end().get() + 1;
            let caret = match url {
                Some(_) => after_open + closing.len(),
                None => after_open + 2,
            };
            doc.set_selection(Selection::caret(ByteOffset::new(caret)));
        });
        true
    }

    /// The span a toggle should act on when there is no construct to remove.
    fn target_span(&self) -> ByteSpan {
        if !self.selection().is_collapsed() {
            return self.selection().span();
        }
        let caret = self.selection().head();
        // Mid-word with no selection wraps the word, which is what people expect from standing in a
        // word and pressing the chord.
        //
        // "Word" here means one with letters or digits in it. `word_at` also returns runs of
        // punctuation and whitespace — that is right for a double-click, where selecting `---` is
        // useful, and wrong here: wrapping the `**` of a neighbouring construct in another `**` is
        // nonsense, and it is what a caret sitting just before one would otherwise do. Anything
        // without an alphanumeric in it falls through to the empty-span case below.
        let word = self.word_at(caret);
        let text = self.to_string();
        if word.slice(&text).chars().any(char::is_alphanumeric) {
            return word;
        }
        // Nothing to wrap: the pair goes in and the caret parks between them.
        ByteSpan::new(caret, caret)
    }

    /// Whether the caret sits between a delimiter pair with nothing in it, and where they are.
    ///
    /// The pair a press with no selection leaves behind. It is **not** a construct — CommonMark
    /// needs content between the delimiters — so the parser reports no markers for it and
    /// [`Self::enclosing`] cannot find it. Pressing the chord twice by mistake would otherwise leave
    /// `****` in the document with no way to take it off but backspacing.
    ///
    /// So this one case is matched textually. Deliberately narrow: exactly the delimiter on each
    /// side of the caret and nothing between, which is only ever the state this module just created.
    fn empty_pair(&self, what: Inline) -> Option<ByteSpan> {
        let delimiter = what.delimiter();
        let caret = self.selection().head().get();
        let start = caret.checked_sub(delimiter.len())?;
        let end = caret + delimiter.len();

        let text = self.to_string();
        if !text.is_char_boundary(start) || end > text.len() {
            return None;
        }
        (&text[start..caret] == delimiter && &text[caret..end] == delimiter)
            .then(|| ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)))
    }

    /// Whether `at` is inside the content of an inline code span.
    fn inside_code_span(&self, at: ByteOffset) -> bool {
        self.enclosing(Inline::Code, at).is_some()
    }

    /// The two marker spans enclosing `at`, if `at` sits inside a construct of this kind.
    ///
    /// The markers of one kind alternate open, close, open, close through the document — which is
    /// what makes pairing them a matter of taking them two at a time rather than of matching
    /// anything. Nesting a construct inside another of the *same* kind is not a thing CommonMark
    /// produces, so there is no stack to keep.
    fn enclosing(&self, what: Inline, at: ByteOffset) -> Option<(ByteSpan, ByteSpan)> {
        let wanted = what.marker();
        let markers: Vec<ByteSpan> = self
            .segments()
            .iter()
            .filter(|segment| segment.kind() == SegmentKind::Marker(wanted))
            .map(mdedit_language::Segment::span)
            .collect();

        markers.chunks_exact(2).find_map(|pair| {
            let (open, close) = (pair[0], pair[1]);
            (open.end() <= at && at <= close.start()).then_some((open, close))
        })
    }

    /// Removes a construct's two delimiters, leaving its content where it was.
    fn unwrap_construct(&mut self, open: ByteSpan, close: ByteSpan) -> bool {
        let opening = open.end().get() - open.start().get();
        // Where the caret ends up once the opening delimiter has gone. Worked out before anything
        // moves, because afterwards the old offsets mean nothing.
        let caret = ByteOffset::new(self.selection().head().get().saturating_sub(opening));

        self.transact(|doc| {
            // The closing delimiter first: removing the opening one would shift the closing one's
            // offsets, and a transaction deliberately does not reorder anything.
            let _ = doc.replace(close, "");
            let _ = doc.replace(open, "");
            doc.set_selection(Selection::caret(caret));
        });
        true
    }

    /// Puts a construct's delimiters around `span`.
    fn wrap(&mut self, span: ByteSpan, what: Inline) -> bool {
        let delimiter = what.delimiter();
        let selected = !self.selection().is_collapsed();

        self.transact(|doc| {
            // Backwards, for the reason `unwrap_construct` gives.
            let _ = doc.replace(ByteSpan::new(span.end(), span.end()), delimiter);
            let _ = doc.replace(ByteSpan::new(span.start(), span.start()), delimiter);

            // The caret goes *inside* the content, never between the delimiter pairs — the reveal
            // rule dims markers the caret is touching, and parking between them would show the
            // construct at its least readable the moment it was made.
            let start = ByteOffset::new(span.start().get() + delimiter.len());
            let end = ByteOffset::new(span.end().get() + delimiter.len());
            if selected {
                doc.set_selection(Selection::new(start, end));
            } else {
                doc.set_selection(Selection::caret(end));
            }
        });
        true
    }
}
