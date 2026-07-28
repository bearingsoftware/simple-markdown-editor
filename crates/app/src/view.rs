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
use mdedit_markers::{Block, BlockKind, Marker, SegmentKind};
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
    /// Background of the tab strip behind the tabs.
    pub strip: Rgba,
    /// Background of the tab being shown.
    pub tab_active: Rgba,
    /// Background of a tab that is not showing.
    pub tab_inactive: Rgba,
    /// The dot marking a tab with unsaved changes.
    pub tab_modified: Rgba,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            body_size: 16.0,
            background: Rgba::rgb(0x1e, 0x1e, 0x1e),
            text: Rgba::rgb(0xe6, 0xe6, 0xe6),
            marker: Rgba::rgb(0x6a, 0x9c, 0x6a),
            caret: Rgba::rgb(0xff, 0xd7, 0x4a),
            strip: Rgba::rgb(0x14, 0x14, 0x14),
            tab_active: Rgba::rgb(0x2d, 0x2d, 0x2d),
            tab_inactive: Rgba::rgb(0x1a, 0x1a, 0x1a),
            tab_modified: Rgba::rgb(0xff, 0xd7, 0x4a),
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

/// One run of text to draw, with its colour.
///
/// Colour is deliberately not part of [`mdedit_text::TextStyle`]: it does not affect shaping, and
/// putting it there would make two identically-shaped runs miss the layout cache for no reason.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewRun {
    /// The text of the run. Owned, because concealing markers means the drawn string is not a
    /// contiguous slice of the buffer.
    pub text: String,
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
pub fn build_runs(doc: &Document, selection: Selection, theme: &Theme) -> Vec<ViewRun> {
    let text = doc.to_string();
    let mut builder = Builder {
        runs: Vec::new(),
        theme,
        doc,
        state: DecorState::default(),
    };

    for segment in doc.segments() {
        let span = segment.span();
        let piece = span.slice(&text);
        let revealed = doc.is_revealed(segment, selection);

        match segment.kind() {
            SegmentKind::Marker(marker) => {
                // The state change happens whether or not the marker is drawn: concealing `**`
                // must still embolden what follows, which is the entire point.
                builder.apply(marker);
                if revealed {
                    builder.push_split(piece, span, Role::Marker);
                }
            }
            SegmentKind::Content => builder.push_split(piece, span, Role::Content),
        }
    }

    builder.runs
}

/// Whether a piece is syntax or content, which decides its colour and whether it is monospaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Marker,
    Content,
}

/// Accumulates runs while walking the document's segments.
struct Builder<'a> {
    runs: Vec<ViewRun>,
    theme: &'a Theme,
    doc: &'a Document,
    state: DecorState,
}

impl Builder<'_> {
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
    fn push_split(&mut self, piece: &str, span: ByteSpan, role: Role) {
        let base = span.start().get();
        let mut cursor = 0usize;

        while cursor < piece.len() {
            let style = self.style_at(ByteOffset::new(base + cursor), role);
            let colour = match role {
                Role::Marker => self.theme.marker,
                Role::Content => self.theme.text,
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

    fn push_one(&mut self, text: &str, start: usize, style: TextStyle, colour: Rgba) {
        if text.is_empty() {
            return;
        }
        self.runs.push(ViewRun {
            text: text.to_owned(),
            style,
            source: ByteSpan::new(ByteOffset::new(start), ByteOffset::new(start + text.len())),
            colour,
        });
    }
}

/// Borrows the runs as [`StyledRun`]s for the layout engine.
#[must_use]
pub fn as_styled(runs: &[ViewRun]) -> Vec<StyledRun<'_>> {
    runs.iter()
        .map(|run| StyledRun {
            text: &run.text,
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
pub fn colour_at(runs: &[ViewRun], offset: ByteOffset, fallback: Rgba) -> Rgba {
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
