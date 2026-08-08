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

use mdedit_document::Document;
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

    /// A translucent colour, for a tint that has to work over whatever is beneath it.
    ///
    /// Kept as real alpha rather than pre-blended against a known background, which is what a
    /// two-palette theme would otherwise force: the same tint over two different grounds is two
    /// different opaque colours, and one of them would be wrong the moment a mode changed.
    #[must_use]
    pub const fn translucent(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a,
        }
    }
}

/// Which palette the editor is drawing with.
///
/// Both are reachable since MDEDIT-78: the config key chooses one or says to follow Windows, the
/// View menu sets it, and `WindowEvent::ThemeChanged` re-reads the registry while the editor runs.
/// It exists as a value with two variants rather than a `Default` impl with one because of MDEDIT-71,
/// which is what made the light palette expressible at all.
///
/// Deliberately **not** called `Mode`: that is already taken by how the *document* is drawn
/// (preview against plain markup), and the two are independent — plain markup is still dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    /// The palette the editor has always had.
    #[default]
    Dark,
    /// Nocturne's light palette.
    Light,
}

/// Which palette the user asked for, which is not the same as which one is drawn (MDEDIT-78).
///
/// [`Appearance`] is the answer; this is the question. `System` resolves against Windows and changes
/// with it; the other two pin it. Stored in the config as `"system" | "dark" | "light"`, following
/// the [`Chrome`] precedent — **never a path**, per the rule at the top of [`crate::config`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Whatever Windows is set to, and change when it changes. The default.
    #[default]
    System,
    /// Dark, whatever Windows is set to.
    Dark,
    /// Light, whatever Windows is set to.
    Light,
}

impl ThemeChoice {
    /// Which palette this choice means, given what Windows says.
    ///
    /// `light` is `None` when Windows does not say — an old build, or a user who has never opened
    /// the personalisation page. **Dark is the answer then**, which is a decision about this editor
    /// rather than about Windows: it is what the editor has always drawn, and every value in the
    /// dark palette has been looked at on real hardware.
    #[must_use]
    pub fn resolve(self, light: Option<bool>) -> Appearance {
        match self {
            Self::Light => Appearance::Light,
            Self::System if light == Some(true) => Appearance::Light,
            // `Dark` and a `System` that Windows answered dark, or did not answer at all.
            Self::Dark | Self::System => Appearance::Dark,
        }
    }
}

/// Sizes and colours.
///
/// # Two palettes, both reachable
///
/// MDEDIT-71 made the palette a value with two of them; MDEDIT-72 made the dark one Nocturne, and
/// MDEDIT-78 made the light one reachable — a `theme` config key, three View menu items, and a
/// re-read of the registry when Windows says its setting changed. `Default` is dark, which is what
/// an editor with no config and a Windows that will not say still opens as.
///
/// # Roles nothing reads yet
///
/// Seven fields arrived with the design pack ahead of the stories that consume them, and each says
/// which one below. They are here rather than added one at a time because every story after this
/// would otherwise re-open this struct, and because the light values have to be chosen against each
/// other to be checked against each other.
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
    /// The diagnostic count in the status bar (MDEDIT-47).
    ///
    /// Amber, and **deliberately not the same colour as [`Theme::span_error`]**. Red in a status bar
    /// reads as "something has gone wrong with the editor"; these are facts about a file the user is
    /// deliberately part way through writing, and half of them are transient. The design pack asked
    /// for one rose `error` role covering both; MDEDIT-69 kept two, because the two arguments are
    /// each correct about a different thing.
    pub status_error: Rgba,
    /// A diagnostic's span in the document — the squiggle, once MDEDIT-76 draws one.
    ///
    /// Rose, from the pack, where [`Theme::status_error`] is amber. This one *is* pointing at the
    /// bytes that are wrong, which is the case a warning colour is for. Read by nothing yet.
    pub span_error: Rgba,
    /// A soft advisory: tabs in YAML indentation, an over-long line. Read by nothing yet.
    pub warn: Rgba,
    /// A link destination. MDEDIT-75 colours one; MDEDIT-73 gives it the underline that stops it
    /// being colour alone. Read by nothing yet.
    pub link: Rgba,
    /// Secondary text — blockquotes, and chrome that should recede. MDEDIT-75. Read by nothing yet.
    pub muted: Rgba,
    /// Line numbers in the gutter.
    ///
    /// The gutter draws in [`Theme::marker`] today — "the same dimmed colour as revealed syntax:
    /// scaffolding, not content", which was a good call while there was one dim colour and is not
    /// once there are several. MDEDIT-77 switches it and lights the caret's line. Read by nothing yet.
    pub gutter: Rgba,
    /// Behind the line the caret is on. MDEDIT-77. Read by nothing yet.
    ///
    /// Translucent on purpose — see [`Rgba::translucent`].
    pub caret_line: Rgba,
    /// A hairline separator. Translucent, for the same reason. Read by nothing yet.
    pub line: Rgba,
    /// A raised surface. The file pane is one, and draws in it since MDEDIT-72; before that it
    /// borrowed the tab strip's colour, because the strip's was the only dark chrome there was.
    pub panel: Rgba,
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
    /// Dark, because that is the only palette the editor can currently reach.
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Nocturne's dark palette, from the MDEDIT-69 design pack.
    ///
    /// MDEDIT-72. Four hue families over one neutral ramp: lavender for structure, sage for
    /// literals, sand for numerics, steel for constants, and rose kept for errors alone.
    ///
    /// # The values the pack does not supply
    ///
    /// Six roles have no counterpart in it, and each is derived from its ramp rather than invented:
    ///
    /// - `find_match` — the pack has no search highlight at all. Sand, and **translucent**, so it is
    ///   warm where the selection is cool: at 0.30 over the page it is within 1.2× of the selection's
    ///   *brightness*, which is the point. Two tints told apart by hue survive peripheral vision and
    ///   colour blindness; two told apart by lightness do not.
    /// - `strip`, `tab_active`, `tab_inactive` — three surfaces from the pack's two. The strip is
    ///   `chrome`, the active tab is `panel` (it is the raised one), and the inactive tab is a step
    ///   of the ramp between the page and the panel, so the active/inactive distinction survives.
    /// - `tab_modified` — the accent, which is the caret's lavender, matching [`Theme::light`].
    /// - `token_comment` — the pack sends comments to `muted`, losing a distinction the editor
    ///   already makes. A dimmed sage keeps comments in the literals family and out of the way.
    ///
    /// # Two deliberate deviations
    ///
    /// `gutter` is `#8b8fa4`, one step lighter than the pack's `#82869a`. The pack measures every
    /// role against the page alone, and against `panel` its gutter is **4.22:1** — under its own
    /// floor, on the surface a file pane is. This clears it at 4.75:1 there and 5.50:1 on the page.
    ///
    /// `status_error` stays amber and is **not** the pack's rose, which is [`Theme::span_error`].
    /// The reason is on that field.
    #[must_use]
    pub fn dark() -> Self {
        Self {
            body_size: 16.0,
            background: Rgba::rgb(0x16, 0x18, 0x26),
            text: Rgba::rgb(0xe9, 0xe9, 0xed),
            marker: Rgba::rgb(0x93, 0x97, 0xab),
            caret: Rgba::rgb(0x91, 0x84, 0xd9),
            selection: Rgba::translucent(0x91, 0x84, 0xd9, 0.32),
            // Warm where the selection is cool, so the two are told apart by hue rather than by
            // brightness — the one distinction that survives being looked at out of the corner of
            // an eye, and the one a colour-blind reader keeps.
            find_match: Rgba::translucent(0xd6, 0xb4, 0x8f, 0.30),
            // Amber rather than the pack's rose. Red in a status bar reads as "something has gone
            // wrong with the editor"; these are facts about the file the user is deliberately part
            // way through writing, and half of them are transient. It is already in the sand family's
            // hue, carried over more saturated because it is a signal rather than a literal.
            status_error: Rgba::rgb(0xe0, 0x9b, 0x3c),
            span_error: Rgba::rgb(0xe0, 0x91, 0x9b),
            warn: Rgba::rgb(0xd9, 0xbb, 0x8a),
            link: Rgba::rgb(0x9f, 0xb8, 0xe0),
            muted: Rgba::rgb(0x93, 0x97, 0xab),
            gutter: Rgba::rgb(0x8b, 0x8f, 0xa4),
            caret_line: Rgba::translucent(0x91, 0x84, 0xd9, 0.07),
            line: Rgba::translucent(0xe9, 0xe9, 0xed, 0.10),
            panel: Rgba::rgb(0x23, 0x25, 0x32),
            strip: Rgba::rgb(0x12, 0x13, 0x1f),
            tab_active: Rgba::rgb(0x23, 0x25, 0x32),
            tab_inactive: Rgba::rgb(0x1a, 0x1c, 0x2a),
            tab_modified: Rgba::rgb(0x91, 0x84, 0xd9),
            token_key: Rgba::rgb(0xa9, 0x9c, 0xf0),
            token_string: Rgba::rgb(0xa8, 0xc9, 0xb6),
            token_number: Rgba::rgb(0xd6, 0xb4, 0x8f),
            token_literal: Rgba::rgb(0x8f, 0xb6, 0xd9),
            // A step below `marker` on the same ramp: braces and commas are structure the eye
            // should skip over, and the pack's own map collapses them into `marker` entirely.
            token_punctuation: Rgba::rgb(0x82, 0x86, 0x9a),
            token_comment: Rgba::rgb(0x8a, 0xa8, 0x96),
            // Light enough to find against the page, dark enough not to compete with the text it
            // overlays. Deliberately no track colour: the track is the background, so a bar over a
            // short line looks like nothing rather than like a gutter.
            scrollbar_thumb: Rgba::rgb(0x45, 0x49, 0x65),
        }
    }

    /// Nocturne's light palette, from the MDEDIT-69 design pack.
    ///
    /// Reachable since MDEDIT-78. It was filled in ahead of that because the values have to be chosen
    /// against one another to be *checked* against one another, and `contrast.rs` checks them.
    ///
    /// Six roles have no counterpart in the pack — it has no search highlight at all, and offers two
    /// chrome colours where the tab strip needs four. Those are derived here from the pack's ramp
    /// rather than invented: the selection is its lavender tint, so the find highlight is taken from
    /// sand to stay warm against it, which is the property the dark one was chosen for.
    #[must_use]
    pub fn light() -> Self {
        Self {
            body_size: 16.0,
            background: Rgba::rgb(0xf7, 0xf8, 0xfd),
            text: Rgba::rgb(0x29, 0x2b, 0x31),
            marker: Rgba::rgb(0x59, 0x5d, 0x6c),
            caret: Rgba::rgb(0x5d, 0x52, 0x94),
            selection: Rgba::translucent(0x5d, 0x52, 0x94, 0.22),
            find_match: Rgba::translucent(0xc9, 0x8a, 0x2e, 0.30),
            status_error: Rgba::rgb(0x7a, 0x5a, 0x1e),
            span_error: Rgba::rgb(0x8f, 0x2f, 0x3c),
            warn: Rgba::rgb(0x7a, 0x5a, 0x1e),
            link: Rgba::rgb(0x3d, 0x55, 0x90),
            muted: Rgba::rgb(0x59, 0x5d, 0x6c),
            gutter: Rgba::rgb(0x63, 0x68, 0x7a),
            caret_line: Rgba::translucent(0x5d, 0x52, 0x94, 0.06),
            line: Rgba::translucent(0x29, 0x2b, 0x31, 0.12),
            panel: Rgba::rgb(0xff, 0xff, 0xff),
            strip: Rgba::rgb(0xec, 0xee, 0xf8),
            tab_active: Rgba::rgb(0xff, 0xff, 0xff),
            tab_inactive: Rgba::rgb(0xe2, 0xe5, 0xf2),
            tab_modified: Rgba::rgb(0x5d, 0x52, 0x94),
            token_key: Rgba::rgb(0x5d, 0x52, 0x94),
            token_string: Rgba::rgb(0x3f, 0x6b, 0x52),
            token_number: Rgba::rgb(0x7a, 0x54, 0x26),
            token_literal: Rgba::rgb(0x3a, 0x5f, 0x80),
            token_punctuation: Rgba::rgb(0x59, 0x5d, 0x6c),
            token_comment: Rgba::rgb(0x59, 0x6b, 0x52),
            scrollbar_thumb: Rgba::rgb(0xb4, 0xb8, 0xc8),
        }
    }

    /// The palette for an appearance.
    #[must_use]
    pub fn for_appearance(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Dark => Self::dark(),
            Appearance::Light => Self::light(),
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent inline states, each genuinely on or off. Packing them into flags \
              would be working around the lint rather than saying anything about the code"
)]
struct DecorState {
    bold: bool,
    italic: bool,
    code_span: bool,
    /// Whether the walk is between a link's `[` and its `](`, so its text can take the `key` colour.
    ///
    /// Here rather than derived from the block list, unlike MDEDIT-74's quote: a link is *inline*, so
    /// it pairs within a line like the emphasis delimiters beside it, and there is no block to ask.
    /// The cost is that a link whose text is broken across a source line loses the colour on the
    /// second line, which is the same bargain `bold` and `italic` already make.
    link_text: bool,
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

/// Builds the runs to draw for `doc`.
///
/// Concealed markers are absent from the output entirely — not empty runs, not spaces. That is what
/// makes the rendered text read as prose rather than as source.
///
/// **This does not depend on where the caret is**, and used to. MDEDIT-118 took reveal-on-caret out,
/// so what is drawn is now a function of the document and the mode alone. The `Selection` parameter
/// went with it rather than being left unread, because a parameter nothing looks at reads as a
/// dependency that is not there — and this one has a consequence: a chunk holding the caret no
/// longer draws differently from the same chunk without it.
#[must_use]
pub fn build_runs<'t>(doc: &Document, theme: &Theme, text: &'t str) -> Vec<ViewRun<'t>> {
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(doc.len_bytes()));
    let decoration = Decoration {
        theme,
        mode: Mode::Preview,
    };
    build_runs_in(doc, decoration, whole, text)
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
        // In plain mode nothing is concealed, so there is nothing to ask. In preview the answer no
        // longer depends on the caret at all (MDEDIT-118) — it is the language's, not the
        // selection's.
        let revealed = decoration.mode == Mode::Plain || doc.is_revealed(segment);

        match segment.kind() {
            SegmentKind::Marker(marker) => {
                // The state change happens whether or not the marker is drawn: concealing `**`
                // must still embolden what follows, which is the entire point.
                builder.apply(marker);
                builder.note_link(marker, piece);
                if revealed {
                    builder.push_split(piece, span, Role::Marker(marker));
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
    /// Syntax, carrying which construct it belongs to.
    ///
    /// It carries the [`Marker`] since MDEDIT-75: all thirteen kinds used to collapse to one colour,
    /// so a heading's `#`, a bullet and a link's `](` were the same green.
    Marker(Marker),
    Content,
    /// A link's destination — the bytes between `](` and `)`.
    ///
    /// Not a [`Marker`] variant, because `mdedit_markers` does not emit one: `pulldown-cmark` reports
    /// a link as one construct and subtracts its *text*, so the destination arrives inside the same
    /// marker segment as the brackets around it. Splitting that segment is this renderer's business
    /// rather than the locator's — nothing about the tiling invariant changes.
    Destination,
    /// Syntax that is coloured rather than concealed — see [`mdedit_language::Token`].
    Token(Token),
}

/// Whether a role is `true`, `false` or `null`, which the theme draws in italic.
///
/// MDEDIT-74. The pack's `constant` role is "italic — never colour alone", and this is the one place
/// a [`Token`] reaches [`TextStyle`] rather than only a colour. A quoted `"true"` is
/// [`Token::String`] and is deliberately unaffected: the classification already knows the difference,
/// and italicising on the text would be ignoring it.
fn is_constant(role: Role) -> bool {
    role == Role::Token(Token::Literal)
}

/// Whether this is a link's or an image's syntax, which is the one segment drawn in two colours.
fn is_link_syntax(role: Role) -> bool {
    matches!(role, Role::Marker(Marker::Link | Marker::Image))
}

/// The characters that bracket a link, as opposed to the destination they bracket.
///
/// A classification of bytes rather than a parse, deliberately: the alternative is re-deriving where
/// the destination starts, which means handling angle brackets, titles, and reference labels — a
/// second link parser living in the renderer. A reference label takes the destination's colour under
/// this rule, which is the right answer for the same reason: it is where the link points.
fn brackets_a_link(character: char) -> bool {
    matches!(character, '[' | ']' | '(' | ')' | '!' | '<' | '>')
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

    /// Notes whether a link's syntax opened or closed, so its text can be told from prose.
    ///
    /// Decided from the marker's own bytes rather than from a count: `[` and `![` open one and
    /// everything else — `](url)`, `][ref]`, a bare `]` — closes it. A count would have to know how
    /// many segments a link produces, which depends on the form it is written in.
    fn note_link(&mut self, marker: Marker, piece: &str) {
        if matches!(marker, Marker::Link | Marker::Image) {
            self.state.link_text = piece.ends_with('[');
        }
    }

    /// How text at `offset` is drawn: its style and its colour.
    ///
    /// One method rather than two because both need the same answer from [`Document::block_at`] —
    /// whether this byte is inside a fence decides the monospacing *and*, since MDEDIT-75, the
    /// colour. Asking twice would double the largest per-run cost MDEDIT-37 found.
    fn decorate(&self, offset: ByteOffset, role: Role) -> (TextStyle, Rgba) {
        // Innermost is what governs: code content has no child blocks, so a byte inside a fence
        // resolves to the code block, and a heading's bytes resolve to the heading.
        let kind = self.doc.block_at(offset).map(Block::kind);
        let heading = match kind {
            Some(BlockKind::Heading(level)) => Some(level),
            _ => None,
        };
        let in_code_block = kind == Some(BlockKind::CodeBlock);
        let colour = self.colour(role, self.state.code_span || in_code_block);

        // Plain mode is one size and one weight, all the way down. Anything else — a heading still
        // large, a fence still monospaced — would be preview wearing its markers, which is not what
        // was asked for and not what the file looks like. **Colours stay**: tinting syntax does not
        // misrepresent what is there. So does a destination's underline, for the same reason and
        // because the pack's contract is that `link` is never colour alone in either mode.
        if self.mode == Mode::Plain {
            let plain = TextStyle::body(self.theme.body_size);
            return match role {
                Role::Destination => (plain.underline(), colour),
                _ => (plain, colour),
            };
        }

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
        if self.state.italic || self.is_quoted(offset, role, in_code_block) || is_constant(role) {
            style = style.italic();
        }
        if self.state.code_span
            || in_code_block
            || matches!(role, Role::Marker(_) | Role::Destination)
        {
            style = style.mono();
        }
        // MDEDIT-73's first consumer, and the pack's own rule: `link` is "underlined", so the
        // destination is never colour alone.
        if role == Role::Destination {
            style = style.underline();
        }
        (style, colour)
    }

    /// What colour a piece is drawn in.
    ///
    /// The per-kind marker mapping is MDEDIT-75's, from the design pack's markdown scope map. Only
    /// two kinds leave the `marker` colour, and both are landmarks rather than scaffolding: a list
    /// bullet, which the pack sends to `key`, and a link's destination, which is `link`.
    fn colour(&self, role: Role, in_code: bool) -> Rgba {
        match role {
            Role::Marker(Marker::ListItem) => self.theme.token_key,
            Role::Marker(_) => self.theme.marker,
            Role::Destination => self.theme.link,
            // A link's *text* is content — `pulldown-cmark` subtracts it from the construct — so it
            // is told from surrounding prose by where the walk is rather than by what it says.
            Role::Content if self.state.link_text => self.theme.token_key,
            // Code content is content too, and the pack draws it as a string. `code_span` is already
            // tracked for the monospacing; a fence comes from the block list.
            Role::Content if in_code => self.theme.token_string,
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
        }
    }

    /// Whether the content at `offset` is quoted prose, which the theme draws in italic.
    ///
    /// MDEDIT-74, and the theme's "never colour alone" contract: `muted` is the blockquote's colour
    /// and italic is its second cue, so a reader who cannot tell the two greys apart still sees which
    /// text is quoted.
    ///
    /// **Asked of the block list rather than tracked in [`DecorState`].** That state resets at every
    /// newline, because inline delimiters pair within a line — and a quote is the opposite shape: it
    /// spans lines by definition, so a toggle would be cleared by the first one. The block list knows
    /// the extent, which is what makes a quote ending mid-paragraph end the italics with it.
    ///
    /// Markers are not included: a revealed `>` is syntax, and slanting it would say it is prose.
    /// Nor is code inside a quote — a fenced block is quoted text nobody wants slanted, and italic
    /// monospace is where alignment goes to die.
    fn is_quoted(&self, offset: ByteOffset, role: Role, in_code_block: bool) -> bool {
        role == Role::Content && !in_code_block && self.doc.in_block_quote(offset)
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
            let rest = &piece[cursor..];
            if let Some(index) = rest.find('\n') {
                let end = cursor + index + 1;
                self.push_line(&piece[cursor..end], base + cursor, role);
                self.state.end_line();
                cursor = end;
            } else {
                self.push_line(rest, base + cursor, role);
                cursor = piece.len();
            }
        }
    }

    /// One line's worth of a segment, at one style — except a link's, which is two.
    ///
    /// `[text](url)` reaches here as `[` and `](url)`, because the parser subtracts the text and
    /// leaves the destination inside the closing marker. Drawing that whole segment in one colour is
    /// what made a URL indistinguishable from the brackets holding it, so the delimiters are told
    /// from what they bracket by [`brackets_a_link`] and the two halves are pushed separately.
    fn push_line(&mut self, line: &'t str, start: usize, role: Role) {
        if !is_link_syntax(role) {
            let (style, colour) = self.decorate(ByteOffset::new(start), role);
            self.push_one(line, start, style, colour);
            return;
        }

        let mut at = 0usize;
        while at < line.len() {
            let rest = &line[at..];
            // Non-empty by the loop condition, and `char_indices` is what keeps the split on a
            // character boundary — a destination is as likely to hold a `é` as a document is.
            let bracket = rest.chars().next().is_some_and(brackets_a_link);
            let len = rest
                .char_indices()
                .find(|(_, character)| brackets_a_link(*character) != bracket)
                .map_or(rest.len(), |(index, _)| index);
            let part_role = if bracket { role } else { Role::Destination };
            let (style, colour) = self.decorate(ByteOffset::new(start + at), part_role);
            self.push_one(&rest[..len], start + at, style, colour);
            at += len;
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

/// What is drawn in the cell of an invisible character when whitespace is shown (MDEDIT-46).
///
/// `None` for anything that is already visible, which is nearly every character. The mark is drawn
/// *in* the character's existing cell at its existing advance — the layout is not consulted and not
/// changed — so this never moves a glyph, a wrap point or a hit test.
///
/// # Why the Control Pictures block rather than a drawn box
///
/// U+2400..U+241F exist for exactly this: one codepoint per C0 control, already shaped like the
/// boxed abbreviation everyone draws by hand. One glyph through the same engine as everything else
/// beats a bespoke box-and-hex renderer, and a font without them shows a tofu box, which is close
/// enough to the convention to be readable rather than wrong.
///
/// Space and tab keep the traditional middot and arrow instead, because those are what people
/// actually recognise and neither is a control picture anybody would read.
#[must_use]
pub fn mark_for(character: char) -> Option<char> {
    match character {
        ' ' => Some('\u{b7}'),
        '\t' => Some('\u{2192}'),
        // Handled per line rather than per character - a line ending is one mark for what may be
        // two bytes, and it is drawn past the end of the row rather than in a cell.
        '\n' | '\r' => None,
        // The rest of C0. These are the ones that are invisible *and* usually a mistake.
        control if control < ' ' => char::from_u32(0x2400 + control as u32),
        '\u{7f}' => Some('\u{2421}'),
        _ => None,
    }
}

/// How a line ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// One `\n`.
    Lf,
    /// A `\r\n` pair.
    CrLf,
    /// A `\r` on its own - a classic-Mac ending, or far more often a mistake.
    Cr,
}

impl Ending {
    /// The marks drawn for it, in order.
    #[must_use]
    pub fn marks(self) -> &'static [char] {
        match self {
            Self::Lf => &['\u{240a}'],
            Self::CrLf => &['\u{240d}', '\u{240a}'],
            Self::Cr => &['\u{240d}'],
        }
    }

    /// Whether this one should be drawn in the warning colour.
    ///
    /// **Only a lone `\r`.** LF and CRLF are both ordinary answers and a document full of either
    /// should read as calm; a bare CR is almost always damage, and MDEDIT-46 asks for it to be
    /// flagged loudly rather than merely distinguished.
    #[must_use]
    pub fn is_suspect(self) -> bool {
        matches!(self, Self::Cr)
    }
}

/// The line ending starting at `at`, if there is one there.
///
/// # No backend gives a terminator a glyph
///
/// This used to be reached through an `ending_before`, which backed up over any `\r` and `\n` before
/// the offset because DirectWrite was believed to give `\n` "a glyph of its own" and the stub was
/// known not to. MDEDIT-113 measured it: `"ab\ncd"`, `"ab\rcd"` and `"ab\r\ncd"` each lay out to two
/// lines and four glyphs, none of them the terminator. The stub now agrees, so the offset past a
/// row's last glyph is always the offset *of* its ending and the backing-up had nothing left to do.
#[must_use]
pub fn ending_at(text: &str, at: usize) -> Option<Ending> {
    let rest = text.get(at..)?;
    if rest.starts_with("\r\n") {
        Some(Ending::CrLf)
    } else if rest.starts_with('\n') {
        Some(Ending::Lf)
    } else if rest.starts_with('\r') {
        Some(Ending::Cr)
    } else {
        None
    }
}
