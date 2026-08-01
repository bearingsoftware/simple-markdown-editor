//! A Markdown buffer that keeps its marker/content segmentation current across edits.
//!
//! [`mdedit_markers`] partitions a *fixed* string. This crate holds the text that is being edited and
//! keeps that partition true as it changes — the difference between an architecture that works at
//! rest and one that works while someone is typing into it.
//!
//! # The guarantee
//!
//! Everything [`mdedit_markers::segment`] promises still holds after any sequence of edits: the
//! segments tile the buffer exactly, and concatenating them reproduces it byte for byte.
//!
//! ```
//! # use mdedit_document::Document;
//! # use mdedit_offsets::{ByteOffset, ByteSpan};
//! let mut doc = Document::from("# Title\n\nSome **bold** text.\n");
//! doc.replace(
//!     ByteSpan::new(ByteOffset::new(9), ByteOffset::new(13)),
//!     "Other",
//! )?;
//!
//! let text = doc.to_string();
//! let rebuilt: String = doc.segments().iter().map(|s| s.span().slice(&text)).collect();
//! assert_eq!(rebuilt, text);
//! # Ok::<(), mdedit_document::EditError>(())
//! ```
//!
//! # Design notes
//!
//! **Offsets are bytes.** Every public position is a [`ByteOffset`] into the buffer, matching
//! [`mdedit_markers`]. `ropey` indexes by character, so [`CharOffset`] coexists here — the
//! conversion is confined to [`Document::replace`] and appears nowhere else, which is the whole
//! reason the two are different types.
//!
//! **Segmentation is recomputed on edit.** [`Document::segments`] is therefore a plain `&self`
//! read, which matters: a renderer needs the segments and the selection in the same loop, and a
//! lazily-filled cache would borrow the document mutably just to read it.
//!
//! An undo group is the transaction shape MDEDIT-7 said would justify re-parsing once instead of
//! once per edit, and MDEDIT-14 built it: undo and redo splice every edit in a group and then
//! re-parse a single time. Ordinary edits still re-parse each, because each is its own group.
//!
//! **Re-segmentation is a full re-parse up to 16 KB, and a regional one above it.**
//! `docs/research/04-parsing-benchmarks.md` measured a whole-document parse at less than a
//! frame for 60 KB, which is why every edit did one; at 245 KB it is 14 ms, which is four keystroke
//! budgets. So an edit to a large buffer re-parses the block it landed in plus a block of margin
//! either side and splices the result in, and the whole-document parse it skipped happens at
//! [`Document::reconcile`] when the typing stops.
//!
//! The margin is not decoration: typing `===` under a paragraph turns the paragraph *above* into a
//! heading, and lazy continuation lets a line join the paragraph before it. What the margin cannot
//! cover is a construct that changes the meaning of the whole document — an unclosed fence, a link
//! reference definition — and that is exactly what the reconcile is for.

use std::ops::Range;

mod history;
pub mod outline;

use history::History;
use mdedit_language::{Block, BlockKind, Language, Segment, SegmentKind};
use mdedit_offsets::{BlockId, ByteOffset, ByteSpan, CharOffset, LineIndex};
use outline::Heading;
use ropey::Rope;

/// Why an edit was rejected.
///
/// Both variants mean the caller computed a range that does not address this buffer, which is a
/// programmer error rather than a user one — but a text editor should not abort over it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    /// The span extended past the end of the buffer, or started after it ended.
    #[error("byte span {span} is outside the buffer of {len} bytes")]
    OutOfBounds {
        /// The offending span.
        span: ByteSpan,
        /// Buffer length in bytes at the time of the edit.
        len: usize,
    },
    /// An endpoint landed inside a multi-byte character.
    #[error("byte offset {offset} is not a character boundary")]
    NotCharBoundary {
        /// The offending endpoint.
        offset: ByteOffset,
    },
}

/// Appends a segment, joining it to the one before if they are the same kind and meet.
///
/// `mdedit_markers` coalesces adjacent runs of the same kind, so a spliced list has to as well —
/// otherwise the seam between the reused part and the re-parsed part shows up as two content
/// segments where a whole-document parse would have produced one. Nothing downstream reads it
/// wrongly, but "the same answer as parsing the lot" is the property this is all judged against, and
/// a seam is a difference.
fn push_merged(out: &mut Vec<Segment>, segment: Segment) {
    if let Some(last) = out.last_mut() {
        if last.kind() == segment.kind() && last.span().end() == segment.span().start() {
            let joined = ByteSpan::new(last.span().start(), segment.span().end());
            *last = segment.moved_to(joined);
            return;
        }
    }
    out.push(segment);
}

/// Whether a segment is a word rather than a run of punctuation or space.
///
/// Alphanumeric is the test rather than "not punctuation" because it is what decides whether joining
/// this segment to its neighbour would merge two *words*. See [`Document::word_at`].
fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

/// Below this, a whole-document parse is already well inside the keystroke budget, so a small
/// document takes exactly the path it always did — one parse, no splicing, nothing to reconcile.
const WHOLE_DOCUMENT_LIMIT: usize = 16 * 1024;

/// Markdown source, plus the marker/content segmentation derived from it.
#[derive(Debug, Clone)]
pub struct Document {
    text: Rope,
    /// Always current: recomputed by every edit.
    segments: Vec<Segment>,
    /// Block structure, recomputed alongside the segments by the same parse.
    blocks: Vec<Block>,
    /// True when the analysis came from a scoped re-parse rather than a whole-document one.
    scoped: bool,
    /// Where the caret is. Owned here because undo has to put it back where it was.
    selection: Selection,
    history: History,
    /// How many times the bytes have changed. See [`Document::revision`].
    revision: u64,
    /// What language this buffer is in.
    ///
    /// A `&'static dyn` rather than a type parameter: the language comes from a file's extension at
    /// runtime, so making it generic would push that runtime choice up through every type that holds
    /// a document. The cost is one vtable call per parse, against a parse measured in milliseconds.
    language: &'static dyn Language,
}

impl Document {
    /// An empty document.
    #[must_use]
    pub fn new() -> Self {
        Self::in_language(&mdedit_markers::MARKDOWN)
    }

    /// An empty document in a given language.
    ///
    /// [`new`](Self::new) is this with Markdown, which is what every caller wants until a file's
    /// extension says otherwise.
    #[must_use]
    pub fn in_language(language: &'static dyn Language) -> Self {
        Self {
            text: Rope::new(),
            segments: Vec::new(),
            blocks: Vec::new(),
            scoped: false,
            selection: Selection::caret(ByteOffset::ZERO),
            history: History::default(),
            revision: 0,
            language,
        }
    }

    /// A document holding `text`, read as `language`.
    ///
    /// [`From<&str>`](Document::from) is this with Markdown.
    #[must_use]
    pub fn with_text(text: &str, language: &'static dyn Language) -> Self {
        let mut doc = Self {
            text: Rope::from_str(text),
            ..Self::in_language(language)
        };
        doc.resegment();
        doc
    }

    /// What language this buffer is being read as.
    #[must_use]
    pub fn language(&self) -> &'static dyn Language {
        self.language
    }

    /// Length of the buffer in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }

    /// Whether the buffer holds no text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len_bytes() == 0
    }

    /// The underlying rope, for callers that need line indexing or cheap slicing.
    #[must_use]
    pub fn rope(&self) -> &Rope {
        &self.text
    }

    /// The marker/content partition of the current buffer.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The block structure of the current buffer, outermost first, in document order.
    ///
    /// Unlike the segments these do not tile the buffer — the blank line between two paragraphs
    /// belongs to no block.
    #[must_use]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The innermost block containing `offset`, if any.
    ///
    /// Innermost rather than outermost because callers ask in order to decide how to treat the
    /// bytes at that position, and the closest enclosing construct is what governs them: a
    /// paragraph inside a quote inside a list is a paragraph.
    #[must_use]
    /// # Why this is not a scan
    ///
    /// It used to be, and `AGENTS.md` named it as the thing to revisit. MDEDIT-37 is what made it
    /// worth revisiting: the renderer asks this **once per styled run**, so a frame costs the number
    /// of visible runs times the number of blocks in the document. For a 357 KB JSON file — 4,001
    /// blocks, thirty-six runs in each of some twenty visible chunks — that is 2.9 million
    /// comparisons a frame, and it was the largest single cost in drawing one.
    ///
    /// Blocks are in document order with non-decreasing starts, so the candidates are a prefix,
    /// found by binary search. The walk back through that prefix then stops at the first *top-level*
    /// block that ends before `offset`: top-level blocks do not overlap and are ordered, so nothing
    /// earlier can contain it either. What is left to scan is one top-level block's worth of nesting.
    pub fn block_at(&self, offset: ByteOffset) -> Option<&Block> {
        let candidates = self
            .blocks
            .partition_point(|block| block.span().start() <= offset);

        let mut innermost: Option<&Block> = None;
        for block in self.blocks[..candidates].iter().rev() {
            if offset < block.span().end() {
                if innermost.is_none_or(|found| block.depth() > found.depth()) {
                    innermost = Some(block);
                }
            } else if block.depth() == 0 {
                break;
            }
        }
        innermost
    }

    /// The heading trail enclosing `offset`, outermost first. See [`outline::heading_trail`].
    #[must_use]
    pub fn heading_trail(&self, offset: ByteOffset) -> Vec<Heading> {
        outline::heading_trail(&self.blocks, offset)
    }

    /// A heading's text, with its syntax left out.
    ///
    /// Taken from the segments rather than by trimming `#` and whitespace, because the segments are
    /// the one place that knows which bytes are syntax — that is what they are for, and a setext
    /// heading's markers are on a different line from its text, which no amount of trimming the
    /// front would find.
    #[must_use]
    pub fn heading_text(&self, heading: &Heading) -> String {
        let span = heading.span();
        let mut text = String::new();
        // Segments tile the buffer in order, so the first one that reaches into this heading can be
        // found rather than scanned to. Walking from byte zero instead costs the whole document on
        // every call, which on a large file is the difference between a breadcrumb being free and
        // it showing up in a keystroke.
        let first = self
            .segments
            .partition_point(|segment| segment.span().end() <= span.start());
        for segment in &self.segments[first..] {
            if segment.span().start() >= span.end() {
                break;
            }
            if segment.is_marker() {
                continue;
            }
            // Clipped rather than skipped. Segments tile the whole buffer and a block's span ends
            // mid-segment: the content run after `# Guide` carries straight on into the blank line
            // and the paragraph below it, so taking it whole puts the next paragraph in the
            // breadcrumb.
            let start = segment.span().start().max(span.start());
            let end = segment.span().end().min(span.end());
            if start >= end {
                continue;
            }
            text.push_str(&self.slice(ByteSpan::new(start, end)));
        }
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The bytes of a span, as a string.
    fn slice(&self, span: ByteSpan) -> String {
        let start = self.text.byte_to_char(span.start().get());
        let end = self.text.byte_to_char(span.end().get());
        self.text.slice(start..end).to_string()
    }

    /// Which source line an offset is on, and how many characters into it, both counting from one.
    ///
    /// **Characters, not bytes and not columns.** A status bar saying "Col 4" in the middle of an
    /// emoji would be lying, and one that counted a tab as eight would be answering a question about
    /// a terminal rather than about this document. `ByteOffset` and `CharOffset` exist as separate
    /// types precisely so this cannot be fudged.
    #[must_use]
    pub fn line_and_column(&self, offset: ByteOffset) -> (LineIndex, CharOffset) {
        let byte = offset.get().min(self.text.len_bytes());
        let line = self.text.byte_to_line(byte);
        let column = self.text.byte_to_char(byte) - self.text.line_to_char(line);
        (LineIndex::new(line), CharOffset::new(column))
    }

    /// The word `offset` sits in or beside, for a double-click.
    ///
    /// # What counts as a word
    ///
    /// Unicode word boundaries (UAX #29), via `unicode-segmentation`. The alternative — an
    /// alphanumeric-run classifier — is a few lines and is wrong on most of the world's text: it
    /// splits `café` at the accent when the é is decomposed, treats a run of CJK as one enormous
    /// word, and cuts `don't` in half. This is the definition of "word" for the whole editor, so it
    /// is worth a small pure dependency rather than a bespoke approximation in each place that needs
    /// one.
    ///
    /// A position **between** two words belongs to the one on its left when there is one, which is
    /// what puts the caret at the end of a word rather than the start of the next after a click at
    /// its edge.
    ///
    /// Whitespace and punctuation are words too. Double-clicking a run of spaces selects the run,
    /// and a run of `---` selects the run, which is what every editor does and what makes a
    /// double-click never select nothing.
    #[must_use]
    pub fn word_at(&self, offset: ByteOffset) -> ByteSpan {
        use unicode_segmentation::UnicodeSegmentation as _;

        let len = self.text.len_bytes();
        let at = offset.get().min(len);
        // The line rather than the whole document: segmentation is linear in what it is given, and
        // a word never crosses a line break. A double-click in a 300 KB paragraph should not walk
        // 300 KB.
        let line = self.text.byte_to_line(at);
        let start = self.text.line_to_byte(line);
        let text: std::borrow::Cow<'_, str> = self.text.line(line).into();
        let within = at - start;

        // UAX #29 breaks a run of punctuation into one segment per character, so a double-click on
        // `---` would select a single dash. Runs of *non-word* segments are therefore joined, which
        // is what every editor does and what the ticket asked for. Word segments are deliberately
        // left alone: joining those as well would take a whole run of CJK as one word, which is the
        // exact failure Unicode segmentation was chosen to avoid.
        let mut segments = text.split_word_bound_indices().peekable();
        let mut last_end = 0;
        while let Some((offset_in_line, segment)) = segments.next() {
            let mut end = offset_in_line + segment.len();
            if !is_word(segment) {
                while let Some((_, next)) = segments.peek() {
                    if is_word(next) {
                        break;
                    }
                    end += next.len();
                    segments.next();
                }
            }
            // `<` rather than `<=`, so a position sitting exactly on a boundary falls through to the
            // word on its left — settled below by the `last_end` fallback.
            if within < end {
                return ByteSpan::new(
                    ByteOffset::new(start + offset_in_line),
                    ByteOffset::new(start + end),
                );
            }
            last_end = end;
        }
        // Past the last word on the line: the end of the line, which is where a click beyond the
        // text lands. An empty line gives an empty span at the caret, and selecting nothing is the
        // honest answer there.
        ByteSpan::new(
            ByteOffset::new(start + last_end),
            ByteOffset::new(start + last_end),
        )
    }

    /// The whole source line `offset` is on, for a triple-click.
    ///
    /// **Source line, not visual row** — the same thing the gutter numbers. A wrapped paragraph is
    /// one line however many rows it draws, which is what makes triple-click select a paragraph
    /// rather than the arbitrary slice of it that happened to fit the window.
    ///
    /// The trailing newline is **included**, when there is one. That is what VS Code does, and it is
    /// what makes a triple-click followed by a delete remove the line rather than leave a blank one
    /// behind. The last line of a document with no final newline simply has none to include.
    #[must_use]
    pub fn source_line(&self, offset: ByteOffset) -> ByteSpan {
        let len = self.text.len_bytes();
        let at = offset.get().min(len);
        let line = self.text.byte_to_line(at);
        let start = self.text.line_to_byte(line);
        let end = if line + 1 < self.text.len_lines() {
            self.text.line_to_byte(line + 1)
        } else {
            len
        };
        ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end))
    }

    /// The innermost code block containing `offset`, if there is one.
    fn code_block_at(&self, offset: ByteOffset) -> Option<&Block> {
        self.blocks
            .iter()
            .filter(|block| {
                block.kind() == BlockKind::CodeBlock
                    && block.span().start() <= offset
                    && offset < block.span().end()
            })
            .max_by_key(|block| block.depth())
    }

    /// Replaces the bytes in `span` with `replacement`.
    ///
    /// The single editing primitive — [`insert`](Self::insert) and [`delete`](Self::delete) are
    /// spellings of it, so the bounds checking and cache invalidation live in one place.
    ///
    /// # Errors
    ///
    /// [`EditError::OutOfBounds`] if the span does not lie within the buffer, or
    /// [`EditError::NotCharBoundary`] if an endpoint splits a multi-byte character. The buffer is
    /// left untouched in both cases.
    pub fn replace(&mut self, span: ByteSpan, replacement: &str) -> Result<(), EditError> {
        let before = self.selection;
        let removed = self.apply(span, replacement)?;

        // The caret lands after what was inserted, which is where a typist expects it and what
        // makes a run of insertions coalesce into one undo step.
        let after = Selection::caret(ByteOffset::new(span.start().get() + replacement.len()));
        self.selection = after;

        self.history.record(
            history::Edit {
                at: span.start(),
                removed,
                inserted: replacement.to_owned(),
            },
            before,
            after,
        );
        Ok(())
    }

    /// Runs `edits` as one undoable operation.
    ///
    /// # What it is for
    ///
    /// Undo groups ordinarily break when the caret moves — see [`history`], where that rule is
    /// documented and defended. It is right for typing and wrong for everything that edits in
    /// several places at once: Replace All, formatting a multi-line selection, converting a
    /// document's line endings. Each of those moves the caret between edits, so without this each
    /// site becomes its own undo step and undoing a 500-match replace takes 500 presses.
    ///
    /// So the caller says so. Inside the closure, caret movement does not break the group and every
    /// edit lands in one history entry; undo restores the pre-transaction bytes *and* the
    /// pre-transaction caret in one step.
    ///
    /// # And it re-parses once
    ///
    /// Each edit ordinarily re-parses — a region for a large buffer, the whole thing for a small
    /// one. Inside a transaction the parse is deferred to the end, so N edits cost one parse rather
    /// than N. The parse taken at the end is a **whole-document** one: a transaction's edits may be
    /// anywhere, and N scoped regions spliced together is neither cheaper nor obviously correct.
    ///
    /// # What it does not do
    ///
    /// **It does not reorder anything.** Offsets shift as edits land, so a caller replacing several
    /// spans must still work from the end backwards, exactly as it would have to without this.
    ///
    /// # Nesting
    ///
    /// A transaction inside a transaction joins the outer one. Friendlier than refusing, and harder
    /// to misuse — "these edits are one step" composes.
    ///
    /// # Panics in the closure
    ///
    /// Not caught. A panic unwinds out of here with the deferred-parse flag still set, so the
    /// document is left in a state where further edits would not re-parse. That is acceptable
    /// because a panic in this editor is terminal — `mdedit_app::log` shows a dialog and the process
    /// ends — and catching it would need `AssertUnwindSafe` around a `&mut self`, which is a worse
    /// trade than the one it fixes.
    pub fn transact<R>(&mut self, edits: impl FnOnce(&mut Self) -> R) -> R {
        let outermost = !self.history.in_transaction();
        self.history.begin(self.selection);
        let revision = self.revision;

        let result = edits(self);

        if outermost {
            // Whatever the edits did to the analysis, it is stale — every one of them skipped its
            // re-parse. Only worth doing if anything actually changed.
            if self.revision != revision {
                self.resegment();
            }
        }
        self.history.commit();
        result
    }

    /// Applies a replacement and brings the segmentation up to date.
    fn apply(&mut self, span: ByteSpan, replacement: &str) -> Result<String, EditError> {
        // Inside a transaction the parse waits for the end — see `transact`. Measured before the
        // splice either way, because `scope_for` reads the block list as it is *now*.
        if self.history.in_transaction() {
            return self.splice(span, replacement);
        }

        let scope = self.scope_for(span);
        let removed = self.splice(span, replacement)?;
        match scope {
            Some(scope) => self.resegment_scope(scope, removed.len(), replacement.len()),
            None => self.resegment(),
        }
        Ok(removed)
    }

    /// The region a keystroke needs re-parsed, or `None` to re-parse the lot.
    ///
    /// Measured before the edit, in the buffer's current coordinates, because it is the *old* block
    /// list that says where the safe boundaries are.
    ///
    /// A margin of one block on each side, not zero: typing `===` under a paragraph turns the
    /// paragraph above into a heading, and lazy continuation lets a line join the paragraph before
    /// it. One block of slack covers both without costing anything measurable.
    fn scope_for(&self, span: ByteSpan) -> Option<Range<usize>> {
        // Under this, a whole-document parse is already well inside the keystroke budget, and doing
        // it keeps the small-document path exactly as it was.
        if self.len_bytes() <= WHOLE_DOCUMENT_LIMIT {
            return None;
        }

        let starts: Vec<usize> = self
            .blocks
            .iter()
            .filter(|block| block.depth() == 0)
            .map(|block| block.span().start().get())
            .collect();

        let before = starts.partition_point(|start| *start <= span.start().get());
        let start = starts.get(before.saturating_sub(2)).copied().unwrap_or(0);

        let after = starts.partition_point(|start| *start <= span.end().get());
        let end = starts
            .get(after + 1)
            .copied()
            .unwrap_or_else(|| self.len_bytes());

        (start < end).then_some(start..end)
    }

    /// Re-parses `scope` alone and splices the result into the analysis.
    ///
    /// `scope` is in the coordinates the buffer had *before* the edit; everything after it moves by
    /// the difference between what was removed and what was inserted.
    fn resegment_scope(&mut self, scope: Range<usize>, removed: usize, inserted: usize) {
        let delta = inserted.cast_signed() - removed.cast_signed();
        let shift = |offset: usize| -> usize { offset.saturating_add_signed(delta) };

        let text = self.text.to_string();
        let region = ByteSpan::new(
            ByteOffset::new(scope.start),
            ByteOffset::new(shift(scope.end).min(text.len())),
        );
        let Some(fresh) = self.language.analyze_region(&text, region) else {
            // This language has no safe place to restart here — JSON nests without limit, so an edit
            // deep inside an object may have no boundary short of the top. A whole-document parse is
            // the correct answer, merely a slower one, and saying so beats a fast wrong one.
            self.resegment();
            return;
        };

        // Segments tile, so one straddling an edge is trimmed rather than dropped: dropping it would
        // leave those bytes covered by nothing, which is the one thing this crate guarantees against.
        let mut segments: Vec<Segment> = Vec::with_capacity(self.segments.len() + 8);
        for segment in &self.segments {
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
        for segment in &self.segments {
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

        // Blocks need no trimming: the scope's edges are top-level block starts, so no block spans
        // one — a nested block is inside its parent, and its parent is inside the scope or outside
        // it entirely.
        let mut blocks: Vec<Block> = Vec::with_capacity(self.blocks.len() + 4);
        blocks.extend(
            self.blocks
                .iter()
                .filter(|block| block.span().end().get() <= scope.start)
                .copied(),
        );
        blocks.extend(fresh.blocks);
        blocks.extend(
            self.blocks
                .iter()
                .filter(|block| block.span().start().get() >= scope.end)
                .map(|block| {
                    block.moved_to(ByteSpan::new(
                        ByteOffset::new(shift(block.span().start().get())),
                        ByteOffset::new(shift(block.span().end().get())),
                    ))
                }),
        );
        // Ids number a block within one analysis, so a spliced list has to be renumbered or two
        // blocks would answer to the same one.
        for (index, block) in blocks.iter_mut().enumerate() {
            *block = block.with_id(BlockId::new(u32::try_from(index).unwrap_or(u32::MAX)));
        }

        self.segments = segments;
        self.blocks = blocks;
        self.scoped = true;
    }

    /// Whether the analysis came from a scoped re-parse and has not been brought up to date.
    ///
    /// See [`reconcile`](Self::reconcile) for what that costs and why it is worth it.
    #[must_use]
    pub fn is_scoped(&self) -> bool {
        self.scoped
    }

    /// Re-parses the whole buffer if the last edits only re-parsed part of it.
    ///
    /// A scoped parse cannot see what the rest of the document does to the region — an unclosed
    /// fence above it, a link reference definition below it. Both are rare and neither is invisible
    /// forever: this is what a caller runs when the typing stops, and it costs one whole-document
    /// parse, which is what every keystroke used to cost.
    ///
    /// Returns whether it found anything to correct, so a caller holding a cache derived from the
    /// segments knows whether to drop it. Usually it does not: the scoped answer is normally the
    /// same answer.
    pub fn reconcile(&mut self) -> bool {
        if !self.scoped {
            return false;
        }
        let segments = std::mem::take(&mut self.segments);
        // Blocks as well as segments, and that is not belt-and-braces. A JSON edit that closes a
        // container early re-depths every member below it while leaving every token's class alone —
        // so the segments agree, the blocks do not, and comparing only the segments reported that
        // nothing had changed while handing back a corrected block list nobody was told about.
        let blocks = std::mem::take(&mut self.blocks);
        self.resegment();
        self.segments != segments || self.blocks != blocks
    }

    /// Applies a replacement to the buffer *without* re-parsing, returning the bytes it displaced.
    ///
    /// The only path that touches the rope. Undo and redo go through it too, so they cannot drift
    /// from what an ordinary edit does — but they re-parse once for the whole group rather than
    /// once per edit in it. Undoing a five-character word is five splices and one parse; doing it
    /// the other way costs five parses, which at 60 KB is most of the p99 keystroke budget spent
    /// four times over for nothing.
    fn splice(&mut self, span: ByteSpan, replacement: &str) -> Result<String, EditError> {
        self.check(span)?;

        // The only place [`CharOffset`] exists. ropey addresses by character, this crate's API
        // addresses by byte, and mixing the two silently is the bug this containment exists to
        // prevent — now with the type system enforcing that the conversion is written down.
        let char_start = self.byte_to_char(span.start());
        let char_end = self.byte_to_char(span.end());

        let removed = if char_start == char_end {
            String::new()
        } else {
            let text = self
                .text
                .slice(char_start.get()..char_end.get())
                .to_string();
            self.text.remove(char_start.get()..char_end.get());
            text
        };
        if !replacement.is_empty() {
            self.text.insert(char_start.get(), replacement);
        }
        // Every path that changes a byte comes through here, undo and redo included, so this is the
        // one place a revision has to be bumped.
        self.revision = self.revision.wrapping_add(1);

        Ok(removed)
    }

    /// How many times the bytes have changed.
    ///
    /// Not a hash and not a timestamp: a cheap way for something that periodically copies the buffer
    /// — crash recovery, in practice — to ask "is this the same text I last saw?" without walking it
    /// or holding a copy to compare against.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Where the caret is, and what is selected.
    #[must_use]
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Moves the caret or changes the selection.
    ///
    /// Ends the current undo group. Moving away from what you were typing is exactly the moment a
    /// timer-based implementation is trying to detect, and the caret says so without a clock.
    pub fn set_selection(&mut self, selection: Selection) {
        if selection != self.selection {
            self.history.break_group();
        }
        self.selection = selection;
    }

    /// Whether there is anything to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is anything to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Reverses the most recent group of edits, restoring the caret with them.
    ///
    /// Returns whether anything was undone.
    pub fn undo(&mut self) -> bool {
        let Some(group) = self.history.pop_undo() else {
            return false;
        };
        for (span, text) in group.undo_steps() {
            // The recorded spans are exactly what was written, so this cannot be out of bounds
            // unless the history and the buffer have already diverged.
            let _ = self.splice(span, text);
        }
        // Once for the group, not once per edit in it.
        self.resegment();
        self.selection = group.selection_before;
        true
    }

    /// Reapplies the most recently undone group.
    ///
    /// Returns whether anything was redone.
    pub fn redo(&mut self) -> bool {
        let Some(group) = self.history.pop_redo() else {
            return false;
        };
        for (span, text) in group.redo_steps() {
            let _ = self.splice(span, text);
        }
        self.resegment();
        self.selection = group.selection_after;
        true
    }

    /// Inserts `text` at a byte offset.
    ///
    /// # Errors
    ///
    /// As [`replace`](Self::replace).
    pub fn insert(&mut self, at: ByteOffset, text: &str) -> Result<(), EditError> {
        self.replace(ByteSpan::empty_at(at), text)
    }

    /// Removes the bytes in `span`.
    ///
    /// # Errors
    ///
    /// As [`replace`](Self::replace).
    pub fn delete(&mut self, span: ByteSpan) -> Result<(), EditError> {
        self.replace(span, "")
    }

    /// Recomputes the partition from the current buffer.
    fn resegment(&mut self) {
        // One parse for both. `analyze` exists precisely so the block list does not cost a second
        // walk of the parser on every keystroke.
        let analysis = self.language.analyze(&self.text.to_string());
        self.segments = analysis.segments;
        self.blocks = analysis.blocks;
        self.scoped = false;
    }

    /// Rejects a span that does not address this buffer.
    fn check(&self, span: ByteSpan) -> Result<(), EditError> {
        let len = self.len_bytes();
        if span.start() > span.end() || span.end().get() > len {
            return Err(EditError::OutOfBounds { span, len });
        }
        for offset in [span.start(), span.end()] {
            // `Rope::byte_to_char` saturates rather than reporting a split character, so the check
            // has to be explicit or a mid-character edit silently moves the edit point.
            if !self.is_char_boundary(offset) {
                return Err(EditError::NotCharBoundary { offset });
            }
        }
        Ok(())
    }

    /// Whether a byte offset sits between characters.
    fn is_char_boundary(&self, offset: ByteOffset) -> bool {
        if offset == ByteOffset::ZERO || offset.get() == self.len_bytes() {
            return true;
        }
        let char_idx = self.byte_to_char(offset);
        self.text.char_to_byte(char_idx.get()) == offset.get()
    }

    /// The rope character index at a byte offset.
    fn byte_to_char(&self, offset: ByteOffset) -> CharOffset {
        CharOffset::new(self.text.byte_to_char(offset.get()))
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for Document {
    fn from(text: &str) -> Self {
        let mut doc = Self {
            text: Rope::from_str(text),
            ..Self::new()
        };
        doc.resegment();
        doc
    }
}

impl std::str::FromStr for Document {
    type Err = std::convert::Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(text))
    }
}

/// Writes the buffer's text.
///
/// This is what gives [`Document`] a `to_string()`. Segment ranges index into that string, so a
/// caller rendering the document wants to hold one rather than build it per segment.
impl std::fmt::Display for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for chunk in self.text.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

/// Where the caret is, and what is selected.
///
/// Offsets are bytes, as everywhere else in this crate. A caret is a selection whose ends coincide;
/// `anchor` is where the selection started and `head` is where it currently is, so `head` is the end
/// that moves and the two can be in either order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    anchor: ByteOffset,
    head: ByteOffset,
}

impl Selection {
    /// A collapsed selection — a caret at `at`.
    #[must_use]
    pub fn caret(at: ByteOffset) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    /// A selection from `anchor` to `head`. Either order is valid.
    #[must_use]
    pub fn new(anchor: ByteOffset, head: ByteOffset) -> Self {
        Self { anchor, head }
    }

    /// Where the selection started.
    #[must_use]
    pub fn anchor(self) -> ByteOffset {
        self.anchor
    }

    /// The moving end of the selection.
    #[must_use]
    pub fn head(self) -> ByteOffset {
        self.head
    }

    /// The selection as an ordered byte span.
    #[must_use]
    pub fn span(self) -> ByteSpan {
        if self.anchor <= self.head {
            ByteSpan::new(self.anchor, self.head)
        } else {
            ByteSpan::new(self.head, self.anchor)
        }
    }

    /// Whether this is a caret rather than a span.
    #[must_use]
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.head
    }
}

impl Document {
    /// Whether a segment's syntax should be shown, given where the caret is.
    ///
    /// This predicate is the whole live-preview illusion: markers are concealed until the caret is
    /// near them, then revealed so they can be edited. Content is always visible.
    ///
    /// Two rules, because Typora has two and conflating them is the usual mistake:
    ///
    /// - **Inline** markers reveal when the selection meets the marker's own span. A *caret* has no
    ///   width, so adjacency counts: resting immediately either side of `**` reveals it, and you can
    ///   delete a delimiter you have just typed past. A *range* selection must genuinely overlap —
    ///   selecting the word inside `**bold**` should not light up delimiters it does not contain.
    /// - **Block** markers reveal when the selection is on any line the marker's block occupies, so
    ///   `## ` stays visible while you edit the heading text rather than vanishing under the cursor.
    ///
    /// Total: out-of-range selections are clamped rather than rejected, because a stale caret should
    /// not be able to crash a render.
    ///
    /// # Known approximation
    ///
    /// [`mdedit_markers`] reports marker spans but not block extents, so "the block" is approximated
    /// by "the line". That is exact for headings, block quotes and list items, whose markers sit on
    /// every line they affect. It is wrong for a multi-line fenced code block: its fences reveal only
    /// when the caret is on a fence line, where Typora would show them from anywhere inside.
    #[must_use]
    pub fn is_revealed(&self, segment: &Segment, selection: Selection) -> bool {
        // A language that never conceals reveals everything, always. That is the whole of the
        // colour-only rendering model: JSON's braces and quotes *are* the document, and YAML's
        // indentation is load-bearing, so hiding either produces something that cannot be edited
        // back into itself.
        if !self.language.conceals() {
            return true;
        }

        let SegmentKind::Marker(marker) = segment.kind() else {
            return true;
        };

        let seg = segment.span();
        let sel = selection.span();

        if marker.is_block() {
            // A fence is the one block marker that does not repeat on the lines it governs: ``` at
            // the top, ``` at the bottom, and nothing in between. So its reveal extent is the whole
            // block, which is what MDEDIT-12 added block extents for.
            //
            // Every other block marker — `>`, a bullet, an indent — is written on each line it
            // affects, so the line is already the exact extent. Widening those to the block would
            // reveal twenty `>` because the caret is on the first line, which is worse than the
            // approximation it replaced.
            if let Some(block) = self.code_block_at(seg.start()) {
                return sel.touches(block.span());
            }
            let marker_lines = self.line_span(seg);
            let caret_lines = self.line_span(sel);
            marker_lines.start <= caret_lines.end && caret_lines.start <= marker_lines.end
        } else if selection.is_collapsed() {
            // Inclusive, so a caret touching either edge counts as inside.
            sel.touches(seg)
        } else {
            sel.overlaps(seg)
        }
    }

    /// The inclusive range of line indices a byte span covers.
    ///
    /// A `Range<LineIndex>` needs no span type of its own: nothing is ever indexed by a line range,
    /// so the orphan-rule problem that [`ByteSpan`] exists to solve does not arise.
    fn line_span(&self, span: ByteSpan) -> Range<LineIndex> {
        let len = self.len_bytes();
        let start = self.text.byte_to_line(span.start().get().min(len));
        let end = self.text.byte_to_line(span.end().get().min(len));
        LineIndex::new(start)..LineIndex::new(end)
    }
}
