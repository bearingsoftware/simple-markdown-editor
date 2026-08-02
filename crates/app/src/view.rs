//! Turning a document into styled runs — the decoration step.
//!
//! This is where the live-preview illusion is actually assembled. [`mdedit_document`] says which
//! bytes are syntax and whether the caret is near them; this decides what that *looks* like:
//! concealed markers are dropped from the text entirely, revealed ones are shown dimmed, and
//! content picks up the styling its surrounding markers imply.
//!
//! Deliberately free of GPU and platform types so it can be tested without a window, which is most
//! of what makes the renderer's behaviour assertable at all.
//!
//! # Where the styling comes from
//!
//! **Block structure is looked up, not inferred.** Heading size and code-block monospacing come
//! from [`Document::block_at`], which knows the answer for any byte. They used to be accumulated
//! while walking segments, and that produced two bugs in a row: a heading's size leaking into the
//! paragraph after it, then — when that was fixed by resetting at newlines — every fence losing its
//! monospacing, because a fence spans lines by definition.
//!
//! **Inline emphasis still toggles.** `**` flips bold on, the next `**` flips it off. Markdown's
//! own rules pair delimiters within a line, so the state resets at every newline; without that an
//! unterminated `**` would embolden the rest of the file.

use mdedit_document::{Document, Selection};
use mdedit_language::{Block, BlockKind, Marker, SegmentKind, Token};
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{StyledRun, TextStyle};

/// A straight RGBA colour, 0–1 per channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Alpha.
    pub a: f32,
}

impl Rgba {
    /// An opaque colour from 8-bit channels.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }
}

/// Sizes and colours. One dark theme; there is no settings surface yet to choose another.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Body text size in device-independent pixels.
    pub body_size: f32,
    /// Background.
    pub background: Rgba,
    /// Ordinary content.
    pub text: Rgba,
    /// Syntax, while revealed. Dimmed rather than hidden, so it reads as scaffolding.
    pub marker: Rgba,
    /// The caret.
    pub caret: Rgba,
    /// Behind selected text. Drawn under the glyphs, so it tints rather than covers.
    pub selection: Rgba,
    /// Behind a search match that is not the current one. Drawn under the selection as well as under
    /// the glyphs, so the match the caret is on is simply the one wearing the selection colour.
    pub find_match: Rgba,
    /// Background of the tab strip behind the tabs.
    pub strip: Rgba,
    /// Background of the tab being shown.
    pub tab_active: Rgba,
    /// Background of a tab that is not showing.
    pub tab_inactive: Rgba,
    /// The dot marking a tab with unsaved changes.
    pub tab_modified: Rgba,
    /// A key in a data language.
    pub token_key: Rgba,
    /// A quoted string used as a value.
    pub token_string: Rgba,
    /// A numeric literal.
    pub token_number: Rgba,
    /// `true`, `false`, `null`.
    pub token_literal: Rgba,
    /// Braces, brackets, commas, colons. Dimmed: structure the eye skips over.
    pub token_punctuation: Rgba,
    /// The scrollbar thumb. No hover or pressed variant — there is no hover affordance in the
    /// editor, and inventing one for this is not what MDEDIT-56 was for.
    pub scrollbar_thumb: Rgba,
    /// A comment, in the dialects that permit one.
    pub token_comment: Rgba,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            body_size: 16.0,
            background: Rgba::rgb(0x1e, 0x1e, 0x1e),
            text: Rgba::rgb(0xe6, 0xe6, 0xe6),
            marker: Rgba::rgb(0x6a, 0x9c, 0x6a),
            caret: Rgba::rgb(0xff, 0xd7, 0x4a),
            selection: Rgba::rgb(0x2d, 0x4f, 0x6b),
            // Warm where the selection is cool, so the two are told apart by hue rather than by
            // brightness — the one distinction that survives being looked at out of the corner of
            // an eye, and the one a colour-blind reader keeps.
            find_match: Rgba::rgb(0x6b, 0x53, 0x1e),
            strip: Rgba::rgb(0x14, 0x14, 0x14),
            tab_active: Rgba::rgb(0x2d, 0x2d, 0x2d),
            tab_inactive: Rgba::rgb(0x1a, 0x1a, 0x1a),
            tab_modified: Rgba::rgb(0xff, 0xd7, 0x4a),
            // Conventional enough to be unsurprising in any editor, and picked to sit on the same
            // dark background the prose does.
            token_key: Rgba::rgb(0x9c, 0xdc, 0xfe),
            token_string: Rgba::rgb(0xce, 0x91, 0x78),
            token_number: Rgba::rgb(0xb5, 0xce, 0xa8),
            token_literal: Rgba::rgb(0x56, 0x9c, 0xd6),
            token_punctuation: Rgba::rgb(0x9a, 0x9a, 0x9a),
            token_comment: Rgba::rgb(0x6a, 0x99, 0x55),
            // Light enough to find against the page, dark enough not to compete with the text it
            // overlays. Deliberately no track colour: the track is the background, so a bar over a
            // short line looks like nothing rather than like a gutter.
            scrollbar_thumb: Rgba::rgb(0x50, 0x50, 0x50),
        }
    }
}

impl Theme {
    /// Size for a heading of `level`, 1–6.
    ///
    /// A geometric-ish ramp rather than CSS's, which is tuned for documents rather than editors and
    /// makes `h1` large enough to dominate a viewport.
    #[must_use]
    pub fn heading_size(&self, level: u8) -> f32 {
        let scale = match level {
            1 => 1.9,
            2 => 1.6,
            3 => 1.35,
            4 => 1.2,
            5 => 1.1,
            _ => 1.0,
        };
        self.body_size * scale
    }
}

/// How the document is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Syntax concealed unless the caret is near it, headings sized, emphasis applied. The editor's
    /// reason for existing.
    #[default]
    Preview,
    /// Every byte on screen, at one size and weight: the file as it actually is.
    ///
    /// Not "preview with the markers left in" — that is what the caret already does locally, and it
    /// would still lie about the sizes. Plain means plain: no heading sizes, no bold, no italics, no
    /// monospacing. Colours stay, because tinting syntax does not misrepresent what is there.
    Plain,
}

/// Everything the decoration step needs to know about how to draw, other than the document.
///
/// A pair rather than two arguments because they travel together everywhere and the alternative is a
/// six-argument function.
#[derive(Debug, Clone, Copy)]
pub struct Decoration<'a> {
    /// Sizes and colours.
    pub theme: &'a Theme,
    /// Concealed, or shown as it is.
    pub mode: Mode,
}

/// One run of text to draw, with its colour.
///
/// Colour is deliberately not part of [`mdedit_text::TextStyle`]: it does not affect shaping, and
/// putting it there would make two identically-shaped runs miss the layout cache for no reason.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewRun<'a> {
    /// The text of the run.
    ///
    /// Borrowed from the buffer. Every run is a contiguous slice of it — concealing a marker drops a
    /// whole run rather than splicing one, so it is only the *concatenation* of runs that is not
    /// contiguous, which is what the previous comment here had wrong.
    ///
    /// Owning it cost a `String` allocation per run per frame. Removing that turned out **not** to be
    /// measurable (MDEDIT-37 — `build_runs_in` on a JSON chunk went 0.064 ms to 0.068, which is
    /// noise); the cost was `Document::block_at`. It stays because it is strictly less work and the
    /// comment it replaced was wrong, not because it made anything faster.
    pub text: &'a str,
    /// How it is shaped.
    pub style: TextStyle,
    /// Where it came from in the buffer.
    pub source: ByteSpan,
    /// What colour to draw it.
    pub colour: Rgba,
}

/// Inline styling carried while walking segments.
///
/// Only inline emphasis lives here now. Heading size and code-block monospacing used to be tracked
/// the same way and were the source of two bugs — a heading's size leaking into the next paragraph,
/// then a fence losing its monospacing when that was fixed. Both are now read from the block
/// structure (MDEDIT-12), which knows the answer rather than inferring it from what came before.
#[derive(Debug, Clone, Copy, Default)]
struct DecorState {
    bold: bool,
    italic: bool,
    code_span: bool,
}

impl DecorState {
    /// Clears inline state at a newline.
    ///
    /// Still needed: Markdown's own rules pair delimiters within a line, so an unterminated `**`
    /// would otherwise embolden the rest of the file. Block state needs no such guard because it is
    /// looked up per byte rather than accumulated.
    fn end_line(&mut self) {
        *self = Self::default();
    }
}

/// Builds the runs to draw for `doc` with the caret at `selection`.
///
/// Concealed markers are absent from the output entirely — not empty runs, not spaces. That is what
/// makes the rendered text read as prose rather than as source.
#[must_use]
pub fn build_runs<'t>(
    doc: &Document,
    selection: Selection,
    theme: &Theme,
    text: &'t str,
) -> Vec<ViewRun<'t>> {
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(doc.len_bytes()));
    let decoration = Decoration {
        theme,
        mode: Mode::Preview,
    };
    build_runs_in(doc, selection, decoration, whole, text)
}

/// The runs to draw for the part of `doc` covered by `within`.
///
/// `text` is the whole buffer, not the slice: segment spans are absolute, and so are the spans the
/// runs carry, so a caller that has already materialized the buffer once should not do it again
/// per chunk.
///
/// Inline styling starts clean at `within`, which is sound because chunk boundaries fall on line
/// boundaries and Markdown pairs inline delimiters within a line.
#[must_use]
pub fn build_runs_in<'t>(
    doc: &Document,
    selection: Selection,
    decoration: Decoration<'_>,
    within: ByteSpan,
    text: &'t str,
) -> Vec<ViewRun<'t>> {
    let mut builder = Builder {
        runs: Vec::new(),
        theme: decoration.theme,
        mode: decoration.mode,
        doc,
        state: DecorState::default(),
    };

    // Segments are sorted and there are as many of them as the whole document has, so a chunk finds
    // its first one rather than walking past every earlier segment. Without this the cost of a frame
    // is the number of visible chunks times the length of the document.
    let first = doc
        .segments()
        .partition_point(|segment| segment.span().end() <= within.start());

    for segment in &doc.segments()[first..] {
        let Some(span) = clip(segment.span(), within) else {
            // In order, so the first one past the end means there is nothing left.
            if segment.span().start() >= within.end() {
                break;
            }
            continue;
        };
        let piece = span.slice(text);
        // Asked of the real selection, not a chunk-local one: whether a marker is revealed is a
        // property of the document, and a caret just outside this chunk can still reveal one in it.
        // In plain mode nothing is concealed, so there is nothing to ask.
        let revealed = decoration.mode == Mode::Plain || doc.is_revealed(segment, selection);

        match segment.kind() {
            SegmentKind::Marker(marker) => {
                // The state change happens whether or not the marker is drawn: concealing `**`
                // must still embolden what follows, which is the entire point.
                builder.apply(marker);
                if revealed {
                    builder.push_split(piece, span, Role::Marker);
                }
            }
            // Never concealed, whatever the caret is doing: the whole point of the colour-only
            // model is that every byte stays on screen.
            SegmentKind::Token(token) => builder.push_split(piece, span, Role::Token(token)),
            // Content, and any kind this renderer has not learned yet — whose bytes are drawn as
            // ordinary text rather than dropped, the same bargain `Marker` already makes.
            _ => builder.push_split(piece, span, Role::Content),
        }
    }

    builder.runs
}

/// The part of `span` inside `within`, or `None` if they do not overlap.
///
/// Needed because segments do not respect chunk boundaries: `mdedit_markers` coalesces adjacent
/// content, so one segment routinely spans a heading, the blank line after it, and the paragraph
/// below — three chunks.
fn clip(span: ByteSpan, within: ByteSpan) -> Option<ByteSpan> {
    let start = span.start().max(within.start());
    let end = span.end().min(within.end());
    (start < end).then(|| ByteSpan::new(start, end))
}

/// Whether a piece is syntax or content, which decides its colour and whether it is monospaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Marker,
    Content,
    /// Syntax that is coloured rather than concealed — see [`mdedit_language::Token`].
    Token(Token),
}

/// Accumulates runs while walking the document's segments.
struct Builder<'a, 't> {
    runs: Vec<ViewRun<'t>>,
    theme: &'a Theme,
    mode: Mode,
    doc: &'a Document,
    state: DecorState,
}

impl<'t> Builder<'_, 't> {
    /// Folds one marker into the inline styling state.
    ///
    /// Block markers do nothing here: what a heading's size is, and whether a byte is inside a code
    /// block, are answered by [`Document::block_at`] at the point of use.
    fn apply(&mut self, marker: Marker) {
        match marker {
            Marker::Strong => self.state.bold = !self.state.bold,
            Marker::Emphasis => self.state.italic = !self.state.italic,
            Marker::CodeSpan => self.state.code_span = !self.state.code_span,
            _ => {}
        }
    }

    /// The style for text at `offset`, combining block structure with inline state.
    fn style_at(&self, offset: ByteOffset, role: Role) -> TextStyle {
        // Plain mode is one size and one weight, all the way down. Anything else — a heading still
        // large, a fence still monospaced — would be preview wearing its markers, which is not what
        // was asked for and not what the file looks like.
        if self.mode == Mode::Plain {
            return TextStyle::body(self.theme.body_size);
        }

        // Innermost is what governs: code content has no child blocks, so a byte inside a fence
        // resolves to the code block, and a heading's bytes resolve to the heading.
        let kind = self.doc.block_at(offset).map(Block::kind);
        let heading = match kind {
            Some(BlockKind::Heading(level)) => Some(level),
            _ => None,
        };
        let in_code_block = kind == Some(BlockKind::CodeBlock);

        // Markers are drawn monospaced at body size even inside a heading: syntax you are editing
        // reads more clearly when it is not proportionally spaced or enlarged.
        let size = match heading {
            // Markers stay at body size even inside a heading: syntax you are editing reads more
            // clearly when it is not enlarged along with the text it marks up.
            Some(level) if role == Role::Content => self.theme.heading_size(level),
            _ => self.theme.body_size,
        };

        let mut style = TextStyle::body(size);
        if self.state.bold || (heading.is_some() && role == Role::Content) {
            style = style.bold();
        }
        if self.state.italic {
            style = style.italic();
        }
        if self.state.code_span || in_code_block || role == Role::Marker {
            style = style.mono();
        }
        style
    }

    /// Pushes `piece`, splitting at newlines so line-scoped styling state can be reset.
    ///
    /// The style is looked up **per line**, not once per segment. `mdedit_markers::segment`
    /// coalesces adjacent runs of the same kind, so a single content segment routinely spans a
    /// heading's text, the blank line after it, and the whole next paragraph — three different
    /// blocks. Styling that segment once drew all of it at heading size.
    fn push_split(&mut self, piece: &'t str, span: ByteSpan, role: Role) {
        let base = span.start().get();
        let mut cursor = 0usize;

        while cursor < piece.len() {
            let style = self.style_at(ByteOffset::new(base + cursor), role);
            let colour = match role {
                Role::Marker => self.theme.marker,
                Role::Content => self.theme.text,
                Role::Token(token) => match token {
                    Token::Key => self.theme.token_key,
                    Token::String => self.theme.token_string,
                    Token::Number => self.theme.token_number,
                    Token::Literal => self.theme.token_literal,
                    Token::Punctuation => self.theme.token_punctuation,
                    Token::Comment => self.theme.token_comment,
                    // A class added to the vocabulary since this was written.
                    _ => self.theme.text,
                },
            };

            let rest = &piece[cursor..];
            if let Some(index) = rest.find('\n') {
                let end = cursor + index + 1;
                self.push_one(&piece[cursor..end], base + cursor, style, colour);
                self.state.end_line();
                cursor = end;
            } else {
                self.push_one(rest, base + cursor, style, colour);
                cursor = piece.len();
            }
        }
    }

    fn push_one(&mut self, text: &'t str, start: usize, style: TextStyle, colour: Rgba) {
        if text.is_empty() {
            return;
        }
        self.runs.push(ViewRun {
            text,
            style,
            source: ByteSpan::new(ByteOffset::new(start), ByteOffset::new(start + text.len())),
            colour,
        });
    }
}

/// Borrows the runs as [`StyledRun`]s for the layout engine.
#[must_use]
pub fn as_styled<'a>(runs: &[ViewRun<'a>]) -> Vec<StyledRun<'a>> {
    runs.iter()
        .map(|run| StyledRun {
            text: run.text,
            style: run.style,
            source: run.source,
        })
        .collect()
}

/// The colour to draw a glyph that came from `offset`.
///
/// Layout hands back glyphs keyed by source byte, not by run, so the mapping has to be recovered.
/// Runs are in source order and non-overlapping, so this is a binary search.
#[must_use]
pub fn colour_at(runs: &[ViewRun<'_>], offset: ByteOffset, fallback: Rgba) -> Rgba {
    let found = runs.binary_search_by(|run| {
        if offset < run.source.start() {
            std::cmp::Ordering::Greater
        } else if offset >= run.source.end() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    });
    found.map_or(fallback, |index| runs[index].colour)
}

/// Where the list of open files is shown.
///
/// One or the other, never both: they are two presentations of the same list, and showing both would
/// be two places to click for one action and two pieces of chrome taking room from the document.
///
/// Here rather than in `editor` because it is a setting as well as a view state — `crate::config`
/// persists it, and the config module should not have to reach into the editor to name it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chrome {
    /// A strip of tabs across the top. The default, and what MDEDIT-15 built.
    #[default]
    Strip,
    /// A vertical list down the left.
    Pane,
}
