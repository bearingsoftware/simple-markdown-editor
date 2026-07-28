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
//! # Two approximations, both from the missing block model
//!
//! [`mdedit_markers`] reports marker spans but not block extents (MDEDIT-12), so:
//!
//! - **Heading size is decided per line.** A `##` marker styles the rest of its own line. Correct
//!   for ATX headings, which is all of them in practice; a setext heading's underline is styled as
//!   a marker but does not resize the line above it.
//! - **Inline styling toggles.** `**` flips bold on, the next `**` flips it off. Markdown's own
//!   nesting rules make that correct for well-formed input; unclosed emphasis would leak to the
//!   end of the line, which is why the state resets at every newline rather than running on.
//!
//! Both are bounded to one line, so the worst case is a mis-styled line rather than a mis-styled
//! document. Revisit when blocks exist.

use mdedit_document::{Document, Selection};
use mdedit_markers::{Marker, SegmentKind};
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
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            body_size: 16.0,
            background: Rgba::rgb(0x1e, 0x1e, 0x1e),
            text: Rgba::rgb(0xe6, 0xe6, 0xe6),
            marker: Rgba::rgb(0x6a, 0x9c, 0x6a),
            caret: Rgba::rgb(0xff, 0xd7, 0x4a),
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

/// The styling state carried while walking segments.
///
/// Split by lifetime, which is the distinction that matters: emphasis and heading size are
/// line-scoped and must not survive a newline, but a fenced code block spans many lines and must.
/// Resetting both at once silently un-monospaces every fence.
#[derive(Debug, Clone, Copy, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these are independent named flags in a state record; packing them into bitflags to satisfy a count would make the code harder to read, not safer"
)]
struct DecorState {
    // Line-scoped. Cleared at every newline.
    bold: bool,
    italic: bool,
    code_span: bool,
    heading: Option<u8>,
    /// An indented code block marks each line's indent separately, so it is per-line too.
    indented_code: bool,

    // Block-scoped. Survives newlines; cleared by the closing fence.
    in_fence: bool,
}

impl DecorState {
    /// Clears the line-scoped half, leaving block state alone.
    fn end_line(&mut self) {
        self.bold = false;
        self.italic = false;
        self.code_span = false;
        self.heading = None;
        self.indented_code = false;
    }

    fn mono(self) -> bool {
        self.code_span || self.in_fence || self.indented_code
    }

    fn style(self, theme: &Theme) -> TextStyle {
        let size = self
            .heading
            .map_or(theme.body_size, |level| theme.heading_size(level));
        let mut style = TextStyle::body(size);
        if self.bold || self.heading.is_some() {
            style = style.bold();
        }
        if self.italic {
            style = style.italic();
        }
        if self.mono() {
            style = style.mono();
        }
        style
    }
}

/// Whether a `CodeBlock` marker is a fence rather than an indent.
///
/// Fences open with three backticks or three tildes; an indented block's marker is whitespace.
fn is_fence(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// How many `#` a heading marker opens with, if it is an ATX opener.
fn atx_level(text: &str) -> Option<u8> {
    let hashes = text.chars().take_while(|c| *c == '#').count();
    // A trailing `###` closing sequence is also a Heading marker but opens nothing, and a setext
    // underline is `=`/`-`. Only a run of hashes at the very start sets the level.
    if hashes == 0 || !text.starts_with('#') {
        return None;
    }
    u8::try_from(hashes.min(6)).ok()
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
                builder.apply(marker, piece);
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
    state: DecorState,
}

impl Builder<'_> {
    /// Folds one marker into the styling state.
    fn apply(&mut self, marker: Marker, piece: &str) {
        match marker {
            Marker::Strong => self.state.bold = !self.state.bold,
            Marker::Emphasis => self.state.italic = !self.state.italic,
            Marker::CodeSpan => self.state.code_span = !self.state.code_span,
            Marker::CodeBlock => {
                if is_fence(piece) {
                    // Opening and closing fences are both `CodeBlock` markers, so toggling is
                    // correct: on at the opener, off at the closer.
                    self.state.in_fence = !self.state.in_fence;
                } else {
                    // An indented block marks the indent of every line, so toggling here would
                    // alternate. It is per-line state instead.
                    self.state.indented_code = true;
                }
            }
            Marker::Heading => {
                if let Some(level) = atx_level(piece) {
                    self.state.heading = Some(level);
                }
            }
            _ => {}
        }
    }

    /// Pushes `piece`, splitting at newlines so line-scoped styling state can be reset.
    ///
    /// The style is recomputed **per line**, not once per segment. `mdedit_markers::segment`
    /// coalesces adjacent runs of the same kind, so a single content segment routinely spans a
    /// heading's text, the blank line after it, and the whole next paragraph. Styling that segment
    /// once meant the heading size leaked into everything that followed it.
    fn push_split(&mut self, piece: &str, span: ByteSpan, role: Role) {
        let base = span.start().get();
        let mut cursor = 0usize;

        while cursor < piece.len() {
            let style = match role {
                // Markers are drawn monospaced at body size even inside a heading: syntax you are
                // editing reads more clearly when it is not proportionally spaced or enlarged.
                Role::Marker => self
                    .state
                    .style(self.theme)
                    .mono()
                    .at_size(self.theme.body_size),
                Role::Content => self.state.style(self.theme),
            };
            let colour = match role {
                Role::Marker => self.theme.marker,
                Role::Content => self.theme.text,
            };

            let rest = &piece[cursor..];
            if let Some(index) = rest.find('\n') {
                let end = cursor + index + 1;
                self.push_one(&piece[cursor..end], base + cursor, style, colour);
                // A newline ends every line-scoped decision: heading size, and any emphasis left
                // unclosed. Without this an unterminated `**` would embolden the rest of the file.
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
