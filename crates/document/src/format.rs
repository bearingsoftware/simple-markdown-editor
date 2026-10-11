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
//! - **Inside the construct already** — remove exactly the two enclosing marker spans. A caret
//!   standing between the two stars of `**` counts as inside; see [`Document::enclosing`].
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
//! on-then-off property that the rest of this module rests on.
//!
//! # Bytes
//!
//! Every edit goes through [`Document::replace`], so the byte-preservation invariant is untouched:
//! the only bytes that change are the delimiters, which the user asked for.
//!
//! **On and then off restores the original exactly**, and that is property-tested rather than
//! asserted. Off and then *on* is a different question and deliberately not an identity: removing a
//! construct does not record how much content it held, so putting it back wraps the word at the
//! caret — and on `**__x__**` the two presses correctly peel two levels rather than one. What holds
//! in that direction is that one press takes off exactly one construct, which is property-tested
//! against the parse (MDEDIT-98).

use mdedit_language::{BlockKind, Marker, Segment, SegmentKind};
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
        //
        // **Above the code-block refusal below**, deliberately: taking back a pair this module just
        // inserted is the *un*-toggle, not a new toggle. `~~~~` at the start of a line is a tilde
        // fence, so a press that left one there would otherwise be unrepeatable — which is the exact
        // thing the refusal exists to prevent.
        if let Some(pair) = self.empty_pair(what) {
            let _ = self.replace(pair, "");
            return true;
        }
        // Inside a code *block*, fenced or indented, `**` is not markup either — which this used to
        // miss entirely, refusing only inside a code *span*. The first press put delimiters in that
        // the parser reports no markers for, and the second press wrapped them again rather than
        // taking them away. MDEDIT-107.
        if self.in_code_block(caret) {
            return false;
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

    /// The text a link or image dialog should start with, and the span its result replaces.
    ///
    /// MDEDIT-129. The same answer [`target_span`](Self::target_span) has always given the toggles —
    /// the selection, or the word at the caret, or nothing — surfaced so a dialog can pre-fill its
    /// text field from it and then write over exactly what it showed you.
    #[must_use]
    pub fn dialog_target(&self) -> ByteSpan {
        self.target_span()
    }

    /// Replaces [`dialog_target`](Self::dialog_target) with `markdown`, in one edit.
    ///
    /// The caret lands after it, because a dialog has already asked for every part — there is
    /// nothing left to type into, which is the difference from the skeleton this replaced.
    pub fn replace_target(&mut self, markdown: &str) -> bool {
        let target = self.dialog_target();
        let after = target.start().get() + markdown.len();
        self.transact(|doc| {
            let _ = doc.replace(target, markdown);
            doc.set_selection(Selection::caret(ByteOffset::new(after)));
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
        if &text[start..caret] != delimiter || &text[caret..end] != delimiter {
            return None;
        }

        // Narrow is not narrow enough on its own. A caret between the two characters of a live `**`
        // also has one `*` on each side of it, so pressing Ctrl+I there matched this and deleted a
        // strong delimiter the user wrote (MDEDIT-98). `enclosing` cannot rule that out, because the
        // construct in the way is of a different kind from the one being toggled — but the parser
        // can: an empty pair is no construct at all, while a real delimiter belongs to one.
        //
        // **Inline markers only.** An empty pair really can end up inside a *block* marker — `~~~~`
        // typed at the start of a line is a tilde fence, and the four characters the parser then
        // calls a fence are the ones this just put there. Those are exactly the ones to take back.
        let at = self.selection().head();
        let inside_a_delimiter = self.segments().iter().any(|segment| {
            matches!(
                segment.kind(),
                SegmentKind::Marker(
                    Marker::Strong | Marker::Emphasis | Marker::Strikethrough | Marker::CodeSpan
                )
            ) && segment.span().start() < at
                && at < segment.span().end()
        });

        (!inside_a_delimiter).then(|| ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)))
    }

    /// Whether `at` is inside the content of an inline code span.
    fn inside_code_span(&self, at: ByteOffset) -> bool {
        self.enclosing(Inline::Code, at).is_some()
    }

    /// The two marker spans enclosing `at`, if `at` sits inside a construct of this kind.
    ///
    /// # Why this is a stack and not two at a time
    ///
    /// It used to take the markers of one kind two at a time, on the premise that they alternate
    /// open, close, open, close and that a construct never nests inside another of its own kind.
    /// The second half of that is wrong: `**a __b__ c**` is strong inside strong, spelled with the
    /// other delimiter, and is ordinary CommonMark. Four markers arrive in open, open, close, close
    /// order, so the pairing matched the outer opening delimiter with the inner one and MDEDIT-98 was
    /// a toggle that deleted bytes in the middle of a line.
    ///
    /// What makes the nesting legal is exactly what makes it resolvable: CommonMark cannot nest a
    /// construct inside another written with the *same* delimiter, so a marker closes the nearest
    /// open one spelled the same way. That is all the stack keys on, and it is why it needs no parser
    /// of its own.
    ///
    /// # A marker segment is not always one delimiter
    ///
    /// Adjacent markers of one kind are merged into a single segment, so the closing `_` and the
    /// opening `*` of `*_em_*` arrive as one two-byte `Emphasis` marker. Each segment is therefore
    /// walked as runs of one byte, and it is the runs that are paired — which is also what makes
    /// `**__b__**`, where the two delimiters of a nest touch, come apart correctly.
    ///
    /// # A caret in a delimiter is in the construct
    ///
    /// The test is `open.start() < at < close.end()`, not "between the two delimiters". A caret
    /// standing *between the two stars of `**`* is inside the bold text as far as the user is
    /// concerned, and the narrower test sent it to the wrapping path, where it inserted an empty pair
    /// into the middle of a live delimiter. The two outer edges stay excluded, so a caret parked
    /// immediately before or after a construct is still outside it.
    ///
    /// The innermost enclosing pair is the one returned, because the innermost is the first to close.
    fn enclosing(&self, what: Inline, at: ByteOffset) -> Option<(ByteSpan, ByteSpan)> {
        let wanted = what.marker();
        let rope = self.rope();
        let mut open: Vec<((usize, u8), ByteSpan)> = Vec::new();

        for bounds in self
            .segments()
            .iter()
            .filter(|segment| segment.kind() == SegmentKind::Marker(wanted))
            .map(mdedit_language::Segment::span)
        {
            let mut start = bounds.start().get();
            while start < bounds.end().get() {
                let byte = rope.byte(start);
                let mut end = start + 1;
                while end < bounds.end().get() && rope.byte(end) == byte {
                    end += 1;
                }
                let span = ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end));
                let delimiter = (end - start, byte);

                match open
                    .iter()
                    .rposition(|(candidate, _)| *candidate == delimiter)
                {
                    Some(position) => {
                        let (_, opening) = open.remove(position);
                        if opening.start() < at && at < span.end() {
                            return Some((opening, span));
                        }
                    }
                    None => open.push((delimiter, span)),
                }
                // Nothing is open and the walk is past `at`, so every pair still to come opens after
                // it. This is what keeps the per-frame cost of `is_inside` off the document's length.
                if open.is_empty() && span.start() > at {
                    return None;
                }
                start = end;
            }
        }
        None
    }

    /// Removes a construct's two delimiters, leaving its content where it was.
    fn unwrap_construct(&mut self, open: ByteSpan, close: ByteSpan) -> bool {
        let opening = open.end().get() - open.start().get();
        // Where the caret ends up once the opening delimiter has gone. Worked out before anything
        // moves, because afterwards the old offsets mean nothing.
        //
        // Clamped into what will be left of the content, because a caret standing *inside* a
        // delimiter — which `enclosing` now counts as being in the construct — has nowhere else to
        // land once that delimiter is gone. Without this it slid out in front of the content, and a
        // second press then wrapped the whitespace it had landed on instead of the word.
        let content = open.start().get()..(close.start().get() - opening);
        let caret = ByteOffset::new(
            self.selection()
                .head()
                .get()
                .saturating_sub(opening)
                .clamp(content.start, content.end),
        );

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

// ---- block formatting (MDEDIT-51) ----

/// A line-wise construct that can be turned on and off.
///
/// Quote is deliberately not here: it is *orthogonal* to the rest and sits outside them — a quoted
/// list is `> - item` — so it toggles on its own and does not have to be stripped to change a list
/// into a heading. See [`Document::toggle_quote`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// An ATX heading of this level, 1–6.
    Heading(u8),
    /// `- ` in front of each line.
    Bullet,
    /// `1. `, renumbered from one on every application.
    Ordered,
    /// A checkbox on a list item: `[ ] ` after a number the line already has, or `- [ ] ` on any
    /// other line.
    Task,
}

/// A line taken apart into the pieces block formatting rewrites.
///
/// Splitting rather than pattern-matching whole lines is what makes the operations compose: changing
/// a bullet into a heading is a change to one field, and quoting it is a change to another, so the
/// two cannot corrupt each other.
#[derive(Debug, Default, PartialEq, Eq)]
struct Parts {
    /// Leading whitespace. Kept exactly, so an indented continuation line stays where it was.
    indent: String,
    /// The run of `> ` prefixes, however many deep.
    quotes: String,
    /// The heading or list marker, with its trailing space. Empty for a paragraph.
    marker: String,
    /// Everything after it.
    content: String,
}

impl Parts {
    /// Puts the line back together.
    fn joined(&self) -> String {
        format!(
            "{}{}{}{}",
            self.indent, self.quotes, self.marker, self.content
        )
    }

    /// Whether this line already carries `what`.
    fn is(&self, what: Block) -> bool {
        match what {
            Block::Heading(level) => self.marker.trim_end() == "#".repeat(level as usize),
            // A task is a list item whose content starts with a checkbox.
            Block::Task => is_list(&self.marker) && checkbox_len(&self.content).is_some(),
            // **The kind of list ignores the box** (MDEDIT-144). A task is a modifier on an item,
            // so `- [ ] a` is a bullet *and* a task: Bullet lights on it, and Bullet on it clears
            // the list, as it would on `- a`. Until then a task answered to Task alone, which was
            // right while Bullet, Ordered and Task were three peer buttons and wrong once the bar
            // offers a kind and a modifier.
            Block::Bullet => is_bullet(&self.marker),
            Block::Ordered => is_ordered(&self.marker),
        }
    }
}

/// Whether `marker` is a bullet marker.
fn is_bullet(marker: &str) -> bool {
    matches!(marker.trim_end(), "-" | "*" | "+")
}

/// Whether `marker` is an ordered-list marker.
fn is_ordered(marker: &str) -> bool {
    let trimmed = marker.trim_end();
    let Some(digits) = trimmed.strip_suffix(['.', ')']) else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// Whether `marker` is either kind of list marker, which is what a task's box can follow.
fn is_list(marker: &str) -> bool {
    is_bullet(marker) || is_ordered(marker)
}

/// The task checkbox opening a list item, with its trailing space, exactly as written.
fn task_box(parts: &Parts) -> Option<String> {
    if !is_list(&parts.marker) {
        return None;
    }
    checkbox_len(&parts.content).map(|len| parts.content[..len].to_owned())
}

/// How many bytes of `content` are a task checkbox, if it opens with one.
fn checkbox_len(content: &str) -> Option<usize> {
    let rest = content.strip_prefix('[')?;
    let mut chars = rest.chars();
    let state = chars.next()?;
    if !matches!(state, ' ' | 'x' | 'X') {
        return None;
    }
    let after = chars.as_str().strip_prefix(']')?;
    // The trailing space is part of the checkbox: `[ ]text` is not one.
    let space = usize::from(after.starts_with(' '));
    (space == 1).then_some(1 + state.len_utf8() + 1 + space)
}

/// Splits one line's text into [`Parts`].
fn split_line(line: &str) -> Parts {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);

    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let mut rest = &line[indent.len()..];

    // Quotes come outermost and may repeat, so they are taken before anything else.
    let mut quotes = String::new();
    while let Some(after) = rest.strip_prefix('>') {
        quotes.push('>');
        if let Some(after) = after.strip_prefix(' ') {
            quotes.push(' ');
            rest = after;
        } else {
            rest = after;
        }
    }

    let marker = leading_marker(rest);
    Parts {
        indent,
        quotes,
        content: rest[marker.len()..].to_owned(),
        marker,
    }
}

/// The heading or list marker at the start of `rest`, with its trailing space.
fn leading_marker(rest: &str) -> String {
    let hashes: String = rest.chars().take_while(|c| *c == '#').collect();
    if (1..=6).contains(&hashes.len()) && rest[hashes.len()..].starts_with(' ') {
        return format!("{hashes} ");
    }

    if let Some(after) = rest.strip_prefix(['-', '*', '+'])
        && after.starts_with(' ')
    {
        return rest[..2].to_owned();
    }

    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        let after = &rest[digits.len()..];
        if let Some(tail) = after.strip_prefix(['.', ')'])
            && tail.starts_with(' ')
        {
            return rest[..digits.len() + 2].to_owned();
        }
    }

    String::new()
}

impl Document {
    /// Turns `what` on for every line the caret or selection touches, or off if they all have it.
    ///
    /// # Mixed selections apply uniformly
    ///
    /// When some selected lines are already listed and some are not, this makes them **all** listed
    /// rather than toggling each. One more invocation then clears them all. That is what Typora and
    /// VS Code both do, and per-line toggling produces a mess nobody asked for — half a list is not
    /// a thing anyone means.
    ///
    /// # One undo step
    ///
    /// However many lines it rewrites. Prefixing twenty lines is twenty edits, and a single press
    /// must undo as a single press — which is what MDEDIT-63's transaction is for.
    ///
    /// Returns whether anything changed.
    pub fn set_block(&mut self, what: Block) -> bool {
        // A setext heading is two lines — the text and the `===` or `---` under it — and only the
        // block list knows that. Prefixing the first line would leave the underline behind, still
        // making it a heading and now a different one, so the whole construct is replaced instead.
        if let Some(span) = self.enclosing_setext() {
            return self.rewrite_setext(span, what);
        }

        let lines = self.selected_lines();
        if lines.is_empty() {
            return false;
        }

        let text = self.to_string();
        let parsed: Vec<Parts> = lines
            .iter()
            .map(|span| split_line(span.slice(&text)))
            .collect();
        // Off only when every line already has it — see the mixed-selection rule above.
        let clearing = parsed.iter().all(|parts| parts.is(what));

        let mut number = 0_u32;
        let rewritten: Vec<String> = parsed
            .into_iter()
            .map(|mut parts| {
                // **A task is a modifier on a list item, not a third kind of list** (MDEDIT-142), so
                // Task on a numbered item adds or removes the box and keeps the author's number. A
                // bullet is only what Task supplies to a line with no list, and it goes with the box,
                // which keeps paragraph → task → paragraph a round trip.
                let numbered = (what == Block::Task && is_ordered(&parts.marker))
                    .then(|| parts.marker.clone());
                // And so changing the kind keeps the box (MDEDIT-144): `- [x] a` becomes
                // `1. [x] a`, the box's bytes exactly as they were, so `[X]` is not respelled.
                // MDEDIT-142 dropped it, which silently lost a tick the user set; taking the box
                // off is Task's job. Clearing still takes it, since a box outside a list item is
                // only brackets.
                let kept = (!clearing && matches!(what, Block::Bullet | Block::Ordered))
                    .then(|| task_box(&parts))
                    .flatten();
                strip_block_marker(&mut parts);
                if let Some(marker) = &numbered {
                    parts.marker.clone_from(marker);
                }
                if !clearing {
                    number += 1;
                    if numbered.is_none() {
                        parts.marker = marker_for(what, number);
                    }
                    if what == Block::Task {
                        parts.content = format!("[ ] {}", parts.content);
                    }
                }
                if let Some(checkbox) = kept {
                    parts.content.insert_str(0, &checkbox);
                }
                parts.joined()
            })
            .collect();

        self.rewrite_lines(&lines, &rewritten)
    }

    /// Adds or removes one level of `> ` on every line the caret or selection touches.
    ///
    /// Separate from [`set_block`](Self::set_block) because quoting is orthogonal to it: a quoted
    /// list is `> - item`, so making a list quoted must not disturb the list, and unquoting must not
    /// disturb it either.
    pub fn toggle_quote(&mut self) -> bool {
        let lines = self.selected_lines();
        if lines.is_empty() {
            return false;
        }

        let text = self.to_string();
        let parsed: Vec<Parts> = lines
            .iter()
            .map(|span| split_line(span.slice(&text)))
            .collect();
        let quoted = parsed.iter().all(|parts| !parts.quotes.is_empty());

        let rewritten: Vec<String> = parsed
            .into_iter()
            .map(|mut parts| {
                if quoted {
                    // One level, not all of them: a doubly-quoted line becomes singly quoted, which
                    // is what "remove a quote" means.
                    parts.quotes = parts
                        .quotes
                        .strip_prefix("> ")
                        .or_else(|| parts.quotes.strip_prefix('>'))
                        .unwrap_or("")
                        .to_owned();
                } else {
                    parts.quotes.insert_str(0, "> ");
                }
                parts.joined()
            })
            .collect();

        self.rewrite_lines(&lines, &rewritten)
    }

    /// Ticks the task on every line the caret or selection touches, or unticks them if they are all
    /// ticked (MDEDIT-175). Returns whether there was a task to change.
    ///
    /// The keyboard's way to do what pressing a box does. In preview the caret cannot stand in
    /// `[ ]` — the bullet and the box are one concealed run it steps over — so without this a task
    /// can only be ticked with the mouse, or by switching to plain markup and typing the `x`.
    ///
    /// # Mixed selections apply uniformly
    ///
    /// The rule [`set_block`](Self::set_block) follows, for the same reason: if **any** selected box
    /// is unticked, every one is ticked; only when all of them already are does a press untick them.
    /// Toggling each would turn "three done, one not" into "one done, three not", which is a
    /// shuffle nobody asked for, and it would leave no press that reliably finishes a list.
    ///
    /// # One byte per box, and only the boxes that change
    ///
    /// The byte between the brackets, exactly as [`toggle_task`](Self::toggle_task) writes it, so
    /// nothing else about a line is rewritten. A box already in the state asked for is not written
    /// at all — ticking `[X]` along with `[ ]` leaves its capital alone rather than re-spelling it.
    /// All of it in one transaction, so one press is one Ctrl+Z, and the selection is put back
    /// because none of this is a click in the text.
    ///
    /// # A line with no box is left alone
    ///
    /// Obsidian makes such a line a task. This does not, so the chord only ever changes a box's state
    /// and never what a line *is* — making a task is the toolbar's Task button, which already exists.
    /// A selection over prose and tasks ticks the tasks; one over prose alone does nothing.
    ///
    /// The boxes are the parse's, not a pattern over the text, so `[ ]` in a code block or after
    /// prose is not a task and is not touched. That is also why it works in plain markup: concealment
    /// is how a box is drawn, not whether there is one.
    pub fn toggle_selected_tasks(&mut self) -> bool {
        let boxes: Vec<ByteSpan> = self
            .selected_lines()
            .into_iter()
            .filter_map(|line| self.task_box_in(line))
            .map(state_of)
            .collect();
        if boxes.is_empty() {
            return false;
        }

        let ticked = |doc: &Self, state: ByteSpan| doc.text.byte(state.start().get()) != b' ';
        let ticking = boxes.iter().any(|&state| !ticked(self, state));

        let selection = self.selection;
        self.transact(|doc| {
            // Backwards, because a transaction reorders nothing. Every edit here is one byte for one
            // byte, so the order cannot actually move an offset — but that is an accident of today's
            // replacement, and the rule costs nothing to keep.
            for &state in boxes.iter().rev() {
                if ticked(doc, state) != ticking {
                    let _ = doc.replace(state, if ticking { "x" } else { " " });
                }
            }
            doc.set_selection(selection);
        });
        true
    }

    /// The task box on `line`, if it has one.
    ///
    /// A source line holds at most one: GFM allows a box only first in a list item's content, and a
    /// line opens at most one item however deeply it nests. So the first is the only.
    fn task_box_in(&self, line: ByteSpan) -> Option<ByteSpan> {
        let first = self
            .segments
            .partition_point(|segment| segment.span().end() <= line.start());
        self.segments[first..]
            .iter()
            .take_while(|segment| segment.span().start() < line.end())
            .find(|segment| segment.kind() == SegmentKind::Marker(Marker::TaskMarker))
            .map(Segment::span)
    }

    /// The source lines the caret or selection touches, in order.
    ///
    /// A selection ending exactly at the start of a line does not include that line — otherwise
    /// selecting one whole line and pressing a chord would format two.
    fn selected_lines(&self) -> Vec<ByteSpan> {
        let span = self.selection().span();
        let mut lines = Vec::new();
        let mut at = self.source_line(span.start());
        loop {
            lines.push(at);
            if at.end() >= span.end() {
                break;
            }
            let next = self.source_line(at.end());
            if next.start() <= at.start() || next.start() >= ByteOffset::new(self.len_bytes()) {
                break;
            }
            at = next;
        }
        lines
    }

    /// Replaces each of `lines` with the matching entry in `rewritten`, as one undoable operation.
    ///
    /// Backwards, because a transaction deliberately does not reorder anything and each edit shifts
    /// every offset after it.
    fn rewrite_lines(&mut self, lines: &[ByteSpan], rewritten: &[String]) -> bool {
        let text = self.to_string();
        if lines
            .iter()
            .zip(rewritten)
            .all(|(span, new)| line_body(span.slice(&text)) == new)
        {
            return false;
        }

        let selection = self.selection();
        let collapsed = selection.is_collapsed();
        // How much longer the whole run gets. Every offset after the first line moves by this, which
        // is what puts the selection back over the same lines rather than over the same bytes.
        let delta: isize = lines
            .iter()
            .zip(rewritten)
            .map(|(span, new)| {
                new.len().cast_signed() - line_body(span.slice(&text)).len().cast_signed()
            })
            .sum();

        self.transact(|doc| {
            for (span, new) in lines.iter().zip(rewritten).rev() {
                let body = ByteSpan::new(
                    span.start(),
                    ByteOffset::new(span.start().get() + line_body(span.slice(&text)).len()),
                );
                let _ = doc.replace(body, new);
            }

            // **The selection has to survive**, or "one more invocation clears them all" is a rule
            // nobody can reach: the second press would see a caret and format one line. Kept over
            // the same *lines*, since the bytes have moved.
            let len = doc.len_bytes();
            if collapsed {
                // Roughly where it was. The line changed length, so the exact byte is gone — and
                // landing in the right line is what a user notices, not the column.
                let at = shifted(selection.head(), delta, lines, len);
                doc.set_selection(Selection::caret(at));
            } else {
                let anchor = shifted(selection.span().start(), 0, lines, len);
                let head = shifted(selection.span().end(), delta, lines, len);
                doc.set_selection(Selection::new(anchor, head));
            }
        });
        true
    }
}

/// Whether `line` is a fence line — the opening or closing one, rather than code.
fn fence_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// A line without its trailing newline, which block formatting never rewrites.
fn line_body(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// Removes whatever heading or list marker a line has, and any task checkbox with it.
///
/// Called before applying a new one, which is what makes bullet → ordered → task a *swap* rather
/// than a stack of prefixes.
fn strip_block_marker(parts: &mut Parts) {
    if is_list(&parts.marker)
        && let Some(len) = checkbox_len(&parts.content)
    {
        parts.content.drain(..len);
    }
    parts.marker.clear();
}

/// The marker for `what`, for the `number`-th line of the run.
fn marker_for(what: Block, number: u32) -> String {
    match what {
        Block::Heading(level) => format!("{} ", "#".repeat(level.clamp(1, 6) as usize)),
        Block::Bullet | Block::Task => "- ".to_owned(),
        // Renumbered from one on every application, so a list that was edited in the middle comes
        // out consecutive rather than carrying whatever numbers happened to be there.
        Block::Ordered => format!("{number}. "),
    }
}

impl Document {
    /// Wraps the selected lines in a fenced code block, or removes the fence the caret is inside.
    ///
    /// Removing uses the **block extent**, not a search for backticks: `Document::block_at` knows
    /// exactly where the fence starts and ends, which is the same span-precision argument the inline
    /// toggles rest on. A fence containing a line that merely looks like a fence is why guessing
    /// does not work.
    ///
    /// When wrapping, the caret parks on the info string — the first thing anybody types after
    /// making a code block is the language.
    pub fn toggle_fence(&mut self) -> bool {
        if let Some(span) = self.enclosing_fence() {
            return self.unwrap_fence(span);
        }

        let lines = self.selected_lines();
        let Some(first) = lines.first() else {
            return false;
        };
        let last = lines.last().copied().unwrap_or(*first);
        let text = self.to_string();
        // A fence has to start its own line, and the closing one has to end with a newline or the
        // text after it joins the fence.
        let closing = if text[..last.end().get()].ends_with('\n') {
            "```\n".to_owned()
        } else {
            "\n```\n".to_owned()
        };

        self.transact(|doc| {
            let _ = doc.replace(ByteSpan::new(last.end(), last.end()), &closing);
            let _ = doc.replace(ByteSpan::new(first.start(), first.start()), "```\n");
            // On the info string: immediately after the opening backticks, before the newline.
            doc.set_selection(Selection::caret(ByteOffset::new(first.start().get() + 3)));
        });
        true
    }

    /// Puts a thematic break on its own line, with the blank lines CommonMark wants around it.
    ///
    /// `---` directly under a paragraph is a setext heading, not a rule — which is the single
    /// easiest way to get this wrong, and why the blank line above is not decoration.
    pub fn insert_rule(&mut self) -> bool {
        let line = self.source_line(self.selection().head());
        let text = self.to_string();
        let at = line.end();

        let before = if text[..at.get()].ends_with("\n\n") || at.get() == 0 {
            ""
        } else if text[..at.get()].ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        // The blank line *after* only matters when something follows, and CommonMark needs one there
        // for the same reason as above. At the end of the document it would be litter.
        let rest = &text[at.get()..];
        let after = if rest.is_empty() || rest.starts_with('\n') {
            "---\n"
        } else {
            "---\n\n"
        };

        let rule = format!("{before}{after}");
        self.transact(|doc| {
            let _ = doc.replace(ByteSpan::new(at, at), &rule);
            doc.set_selection(Selection::caret(ByteOffset::new(at.get() + rule.len())));
        });
        true
    }

    /// Puts an empty GFM table of `rows` × `columns` below the caret's line.
    ///
    /// `rows` **counts the header**, because that is what the picker showed: three highlighted
    /// squares mean a table with three rows in it, and a picker whose smallest table is two rows is
    /// a picker that lies about its own top-left cell.
    ///
    /// The pipes line up in the source. Every cell is written to the width of the `---` under it, so
    /// a fresh table is already in the shape MDEDIT-128's re-pad will keep it in — and, more to the
    /// point, it is readable in the tools that are not this editor.
    ///
    /// **The blank line above is not decoration.** GFM will not let a table interrupt a paragraph,
    /// so a table written directly under one is not a table at all — the same trap `insert_rule`
    /// documents, where `---` under a paragraph is a setext heading.
    pub fn insert_table(&mut self, rows: usize, columns: usize) -> bool {
        if rows == 0 || columns == 0 {
            return false;
        }

        let line = self.source_line(self.selection().head());
        let at = line.end();

        // Read the two bytes before the insertion point out of the rope rather than out of a copy of
        // the whole document: nothing about deciding a separator needs the other end of the file.
        let prev = self
            .rope()
            .byte_slice(at.get().saturating_sub(2)..at.get())
            .to_string();
        let before = if at.get() == 0 || prev.ends_with("\n\n") {
            ""
        } else if prev.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        // A blank line *after* only matters when something follows. At the end of the document it
        // would be litter.
        let after = if at.get() == self.rope().len_bytes()
            || self.rope().byte_slice(at.get()..=at.get()) == "\n"
        {
            ""
        } else {
            "\n"
        };

        let mut table = String::from(before);
        // The width of `---`, which is what the delimiter row needs and so what every other row is
        // padded to.
        let blank = "|".to_owned() + &"     |".repeat(columns) + "\n";
        table.push_str(&blank);
        table.push('|');
        for _ in 0..columns {
            table.push_str(" --- |");
        }
        table.push('\n');
        for _ in 1..rows {
            table.push_str(&blank);
        }
        table.push_str(after);

        // Inside the first cell, which is one byte past the pipe and the space that follows it.
        let caret = ByteOffset::new(at.get() + before.len() + 2);
        self.transact(|doc| {
            let _ = doc.replace(ByteSpan::new(at, at), &table);
            doc.set_selection(Selection::caret(caret));
        });
        true
    }

    /// The span of the fenced code block the caret is in, if it is in one.
    fn enclosing_fence(&self) -> Option<ByteSpan> {
        let at = self.selection().head();
        self.blocks()
            .iter()
            .filter(|block| {
                block.kind() == BlockKind::CodeBlock
                    && block.span().start() <= at
                    && at <= block.span().end()
            })
            // The innermost, for the same reason `code_block_at` takes the deepest.
            .max_by_key(|block| block.depth())
            .map(mdedit_language::Block::span)
    }

    /// Removes a fence's first and last lines, leaving the code where it was.
    fn unwrap_fence(&mut self, span: ByteSpan) -> bool {
        let text = self.to_string();
        let body = span.slice(&text);
        // An *indented* code block has no fence lines to remove — its marker is the indent — so
        // there is nothing here to do rather than something wrong to do.
        if !body.trim_start().starts_with("```") {
            return false;
        }

        let opening = self.source_line(span.start());
        // The last line of the block, which for a well-formed fence is the closing one.
        let closing_start = ByteOffset::new(
            span.end().get()
                - body
                    .strip_suffix('\n')
                    .unwrap_or(body)
                    .rsplit('\n')
                    .next()
                    .map_or(0, str::len),
        );
        let closing = self.source_line(closing_start);
        if closing.start() <= opening.start() {
            // One line: a fence with nothing in it and no separate closing line to remove.
            return false;
        }

        // **An unclosed fence has no closing line**, and its last line is code. Taking the block's
        // last line on the assumption that it is a fence destroyed that line — MDEDIT-107 — and an
        // unclosed fence is exactly what one looks like halfway through being typed, so this is the
        // common state rather than a pathological one. The line has to *be* a fence to be removed as
        // one, which is the same "point at it rather than guess" rule the inline toggles rest on.
        let closes = fence_line(line_body(closing.slice(&text)));
        let caret = ByteOffset::new(
            self.selection()
                .head()
                .get()
                .saturating_sub(opening.end().get() - opening.start().get()),
        );
        self.transact(|doc| {
            if closes {
                let _ = doc.replace(closing, "");
            }
            let _ = doc.replace(opening, "");
            doc.set_selection(Selection::caret(ByteOffset::new(
                caret.get().min(doc.len_bytes()),
            )));
        });
        true
    }
}

/// Moves an offset by `delta` if it sits at or after the run being rewritten, and clamps it.
///
/// An offset before the first rewritten line has not moved at all; one after has moved by the whole
/// change. Anything inside is approximated to the run's start plus the change, which is close enough
/// to keep a selection over the lines it was over.
fn shifted(at: ByteOffset, delta: isize, lines: &[ByteSpan], len: usize) -> ByteOffset {
    let Some(first) = lines.first() else {
        return ByteOffset::new(at.get().min(len));
    };
    if at <= first.start() {
        return ByteOffset::new(at.get().min(len));
    }
    let moved = at.get().saturating_add_signed(delta);
    ByteOffset::new(moved.min(len))
}

impl Document {
    /// The span of the setext heading the caret is in, if it is in one.
    ///
    /// A setext heading is a heading whose block covers more than the line the text is on: the
    /// underline is part of the construct. An ATX heading (`## like this`) is one line and is
    /// handled by the ordinary prefix path.
    fn enclosing_setext(&self) -> Option<ByteSpan> {
        // Only for a caret or a selection inside one heading. A selection spanning several blocks is
        // the multi-line path, and mixing the two would need a rule nobody has asked for yet.
        let selection = self.selection().span();
        let block = self.block_at(selection.start())?;
        if !matches!(block.kind(), BlockKind::Heading(_)) {
            return None;
        }
        let span = block.span();
        if selection.end() > span.end() {
            return None;
        }
        // One line means ATX, which the prefix path already does correctly.
        (self.source_line(span.start()).end() < span.end()).then_some(span)
    }

    /// Replaces a whole setext heading with `what` applied to its text.
    fn rewrite_setext(&mut self, span: ByteSpan, what: Block) -> bool {
        let text = self.to_string();
        let body = span.slice(&text);
        // The heading's text is everything above the underline — **every** line of it. Keeping only
        // the first threw the rest away, and CommonMark is explicit that a setext heading's content
        // may span lines (MDEDIT-107).
        //
        // Joined with a space, because what it is becoming is one line. That is the faithful
        // rendering rather than a compromise: a setext heading's content is those lines as one run
        // of inline text, and an ATX heading has nowhere else to put them.
        let mut lines: Vec<&str> = body.lines().collect();
        lines.pop();
        let title = lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        let mut parts = split_line(&title);
        strip_block_marker(&mut parts);
        // Whatever it is becoming, it stops being a setext heading — the underline goes with the
        // construct either way. A list also picks up its checkbox here, as the prefix path does.
        parts.marker = marker_for(what, 1);
        if what == Block::Task {
            parts.content = format!("[ ] {}", parts.content);
        }

        // Whether the block ended with a newline, so the document does not lose or gain one.
        let trailing = if body.ends_with('\n') { "\n" } else { "" };
        let replacement = format!("{}{trailing}", parts.joined());

        self.transact(|doc| {
            let _ = doc.replace(span, &replacement);
            doc.set_selection(Selection::caret(ByteOffset::new(
                (span.start().get() + replacement.trim_end().len()).min(doc.len_bytes()),
            )));
        });
        true
    }
}

impl Document {
    /// Whether the caret is inside a construct of this kind.
    ///
    /// A **lookup**, not a scan: it asks the same `enclosing` the toggles ask, against segments the
    /// parse already produced. That is what makes lighting a toolbar button affordable per frame —
    /// see `mdedit_app::toolbar`.
    #[must_use]
    pub fn is_inside(&self, what: Inline) -> bool {
        self.enclosing(what, self.selection().head()).is_some()
    }

    /// Whether the caret's line already carries this block format.
    ///
    /// One line, taken apart by the same `split_line` the toggles use, so what lights and what
    /// would clear are decided by one piece of code rather than two that agree today. Reading a
    /// single line is not the scan the keystroke budget objects to — see `mdedit_app::toolbar`.
    ///
    /// A **setext** heading has no marker on its first line, so it answers `false` here; the block
    /// list is what knows about those, and the caller asks that instead for headings.
    #[must_use]
    pub fn carries(&self, what: Block) -> bool {
        self.caret_parts().is(what)
    }

    /// Whether the caret's line is quoted, at any depth.
    #[must_use]
    pub fn is_quoted(&self) -> bool {
        !self.caret_parts().quotes.is_empty()
    }

    /// The caret's line, taken apart.
    ///
    /// Sliced out of the rope rather than out of `to_string`, which would copy the whole document to
    /// look at one line of it — the difference between a lookup and a scan, and this is asked once
    /// per button per frame.
    fn caret_parts(&self) -> Parts {
        let line = self.source_line(self.selection().head());
        let text = self
            .rope()
            .byte_slice(line.start().get()..line.end().get())
            .to_string();
        split_line(&text)
    }
}

/// The one byte of a task box that says whether it is ticked: the one between `[` and `]`.
fn state_of(task: ByteSpan) -> ByteSpan {
    let at = task.start().get() + 1;
    ByteSpan::new(ByteOffset::new(at), ByteOffset::new(at + 1))
}
