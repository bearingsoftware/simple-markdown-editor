//! The text-layout boundary.
//!
//! Shaping text is the one part of this editor that cannot be written from scratch and cannot be
//! taken from a single portable crate. `AGENTS.md` requires it sit behind a narrow trait with two
//! implementations — DirectWrite on Windows, parley elsewhere — because
//! [MDEDIT-5](https://chattersley.atlassian.net/browse/MDEDIT-5) proved on real hardware that
//! `fontique` resolves `script = Hani` to one fallback family and stops, rendering Simplified
//! Chinese as tofu with ten capable fonts installed.
//!
//! This crate is that trait and the vocabulary it speaks. It has no platform dependencies, and
//! **no implementation type may appear in its API** — an `IDWriteFontFace` reaching the editor core
//! is the failure this boundary exists to prevent.
//!
//! # What a caller gets back
//!
//! [`TextLayout::layout`] takes styled runs of text and returns positioned glyphs, grouped into
//! lines. [`TextLayout::rasterize`] turns one glyph into an 8-bit coverage bitmap for an atlas.
//! Layout and rasterization are separate because they cache differently: layout changes on every
//! keystroke, a rasterized glyph is reusable until the font size changes.
//!
//! Positions are [`LayoutPos`] — device-independent pixels, not physical ones.

use std::fmt;

use mdedit_offsets::{ByteOffset, ByteSpan, LayoutPos};

/// Why a layout or rasterization request could not be served.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TextError {
    /// The platform text stack refused a call.
    ///
    /// Carries a rendered message rather than a platform error type, so that a `HRESULT` cannot
    /// reach the editor core by riding inside an error variant.
    #[error("{api} failed: {detail}")]
    Platform {
        /// The API that failed, for locating the call site.
        api: &'static str,
        /// The platform's own description.
        detail: String,
    },
    /// A glyph was requested that this layout never produced.
    #[error("glyph {glyph:?} is not from this layout")]
    UnknownGlyph {
        /// The offending key.
        glyph: GlyphKey,
    },
}

/// Which family a run is drawn in.
///
/// Deliberately not a font name. The editor asks for a role; the implementation maps it to whatever
/// the platform actually has, and fallback for characters outside that font is the implementation's
/// problem — which is the entire point of this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontFamily {
    /// Proportional text. Prose, headings.
    #[default]
    Ui,
    /// Fixed pitch. Code spans, fenced blocks, and concealed syntax while it is revealed.
    Mono,
    /// Monochrome pictograms. The formatting bar's buttons, and nothing else.
    ///
    /// MDEDIT-92, and it is a *role* for the same reason the other two are: the editor asks for
    /// icons and the backend decides which font that means. On Windows that is Segoe Fluent Icons
    /// with Segoe MDL2 Assets behind it; on a platform with neither it resolves to nothing, every
    /// glyph comes back [`.notdef`](GlyphId::is_notdef), and the caller draws a word instead.
    ///
    /// **The code points are private-use, which makes them a promise about a font rather than a
    /// standard.** `U+E8DD` is Bold in Segoe Fluent Icons and is nothing at all in a font that has
    /// not got it, so a caller asking for this family owes its buttons a text fallback. That is why
    /// `.notdef` is on [`GlyphId`] rather than being an implementation detail.
    Icon,
}

/// How heavy a run is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontWeight {
    /// Body text.
    #[default]
    Regular,
    /// Strong emphasis, and headings.
    Bold,
}

/// A font size in device-independent pixels.
///
/// Wraps `f32` so it can key a glyph cache: `Eq` and `Hash` go through the bit pattern, which is
/// exact for the handful of sizes a document actually uses and sidesteps `clippy::float_cmp`.
#[derive(Debug, Clone, Copy)]
pub struct FontSize(f32);

impl FontSize {
    /// A size in device-independent pixels.
    #[must_use]
    pub fn new(px: f32) -> Self {
        Self(px)
    }

    /// The size in device-independent pixels.
    #[must_use]
    pub fn get(self) -> f32 {
        self.0
    }

    /// The bit pattern, which is what equality and hashing are defined over.
    fn bits(self) -> u32 {
        // Normalise the two zeroes so `0.0` and `-0.0` cannot key two atlas entries.
        if self.0 == 0.0 { 0 } else { self.0.to_bits() }
    }
}

impl PartialEq for FontSize {
    fn eq(&self, other: &Self) -> bool {
        self.bits() == other.bits()
    }
}

impl Eq for FontSize {}

impl std::hash::Hash for FontSize {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.bits().hash(state);
    }
}

impl fmt::Display for FontSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}px", self.0)
    }
}

/// How a run of text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextStyle {
    /// Size in device-independent pixels.
    pub size: FontSize,
    /// Family role.
    pub family: FontFamily,
    /// Weight.
    pub weight: FontWeight,
    /// Whether the run is italic.
    pub italic: bool,
    /// Whether the run carries a rule under it.
    ///
    /// The implementation decides where the rule sits and how thick it is, from the font's own
    /// metrics — which is why this is a `bool` here and a [`Rule`] on the way back.
    pub underline: bool,
    /// Whether the run carries a rule through it.
    ///
    /// The same bargain as [`underline`](Self::underline), and the font answers the same two
    /// questions about it. Kept separate rather than folded into one enum because a run can
    /// legitimately be both — a struck-through link is `~~[a](/url)~~`.
    pub strikethrough: bool,
}

impl TextStyle {
    /// Body text at `size`.
    #[must_use]
    pub fn body(size: f32) -> Self {
        Self {
            size: FontSize::new(size),
            family: FontFamily::Ui,
            weight: FontWeight::Regular,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }

    /// This style in bold.
    #[must_use]
    pub fn bold(mut self) -> Self {
        self.weight = FontWeight::Bold;
        self
    }

    /// This style in italic.
    #[must_use]
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// This style underlined.
    #[must_use]
    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// This style struck through.
    #[must_use]
    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    /// This style in the monospace family.
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.family = FontFamily::Mono;
        self
    }

    /// This style at a different size.
    #[must_use]
    pub fn at_size(mut self, size: f32) -> Self {
        self.size = FontSize::new(size);
        self
    }
}

/// A run of text to lay out, with the style it is drawn in.
///
/// `source` is where the run came from in the buffer, carried through so a caller can map a glyph
/// back to the byte the caret addresses. Runs whose text is not in the buffer — there are none yet,
/// but a rendered list bullet would be one — pass an empty span.
#[derive(Debug, Clone, Copy)]
pub struct StyledRun<'a> {
    /// The text of this run.
    pub text: &'a str,
    /// How to draw it.
    pub style: TextStyle,
    /// Where it came from in the buffer.
    pub source: ByteSpan,
}

/// An implementation's handle for a resolved font.
///
/// Opaque by construction. The editor never learns which family this is, only that two glyphs
/// carrying the same `FontId` came from the same face and belong in the same atlas page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(u32);

impl FontId {
    /// Wraps an implementation's font index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The underlying index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A glyph index within a font.
///
/// A *glyph*, not a character: after shaping there may be no correspondence between the two, which
/// is why the editor cannot conflate them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphId(u16);

impl GlyphId {
    /// The glyph every OpenType font reserves for "no glyph for this character" — tofu.
    pub const NOTDEF: Self = Self(0);

    /// Wraps a raw glyph index.
    #[must_use]
    pub const fn new(id: u16) -> Self {
        Self(id)
    }

    /// The raw glyph index.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether this is `.notdef`.
    ///
    /// Glyph 0 is `.notdef` in every OpenType font by definition, so this is a reliable tofu test
    /// across any font and any implementation — which is what makes the fallback assertion in this
    /// crate's tests meaningful rather than a guess.
    #[must_use]
    pub const fn is_notdef(self) -> bool {
        self.0 == Self::NOTDEF.0
    }
}

/// Everything needed to rasterize one glyph, and to cache the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// The face the glyph belongs to.
    pub font: FontId,
    /// The glyph within that face.
    pub glyph: GlyphId,
    /// The size it is drawn at. Part of the key because a bitmap is size-specific.
    pub size: FontSize,
}

/// One glyph, placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    /// Which glyph, and at what size.
    pub key: GlyphKey,
    /// Pen position on the baseline, in device-independent pixels.
    pub pos: LayoutPos,
    /// How far the pen advances after this glyph.
    pub advance: f32,
    /// The bytes of the buffer this glyph came from.
    ///
    /// Not one-to-one with characters: a ligature covers several, and a mark covers none of its
    /// own. Good enough to place a caret, which is all this slice asks of it.
    pub source: ByteSpan,
}

/// Which of the two rules this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    /// Under the text, from [`TextStyle::underline`].
    Under,
    /// Through it, from [`TextStyle::strikethrough`].
    Through,
}

/// A rule under or through a run, placed by the implementation.
///
/// A rectangle rather than a flag on the run, because where a rule sits and how thick it is come
/// from the font's own metrics — which only the implementation has — and because a wrapped run needs
/// one of these per row rather than one for the run.
///
/// **One type for both, distinguished by [`kind`](Self::kind)**, because everything about them is
/// the same: the font decides the offset and the thickness, wrapping splits them per row, and the
/// caller draws a quad. MDEDIT-91 added the second, and the only thing it had to add was the field
/// saying which.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule {
    /// Under the text or through it.
    pub kind: RuleKind,
    /// Left edge, in the same space as [`PositionedGlyph::pos`].
    pub x: f32,
    /// Top edge of the rule, measured from the top of the block like a baseline is.
    pub top: f32,
    /// How far the rule runs.
    pub width: f32,
    /// How thick it is, from the font.
    pub thickness: f32,
    /// A byte the ruled text came from, so the caller can colour the rule like the run.
    pub source: ByteOffset,
}

/// One laid-out line.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutLine {
    /// Distance from the top of the block to this line's baseline.
    pub baseline: f32,
    /// Total line height, ascent to descent plus leading.
    pub height: f32,
    /// The glyphs on this line, in visual order.
    pub glyphs: Vec<PositionedGlyph>,
    /// The rules under and through this line's runs. Empty for almost every line.
    pub rules: Vec<Rule>,
}

/// The result of laying out a sequence of styled runs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// Lines, top to bottom.
    pub lines: Vec<LayoutLine>,
}

impl Layout {
    /// Every glyph in the layout, in line order.
    pub fn glyphs(&self) -> impl Iterator<Item = &PositionedGlyph> {
        self.lines.iter().flat_map(|line| line.glyphs.iter())
    }

    /// Total height of all lines.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.lines.iter().map(|line| line.height).sum()
    }
}

/// A rasterized glyph: 8-bit coverage, ready for an atlas.
///
/// Coverage rather than colour. The renderer multiplies it by the run's colour, so one bitmap
/// serves every colour the glyph is ever drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphImage {
    /// Bitmap width in physical pixels.
    pub width: u32,
    /// Bitmap height in physical pixels.
    pub height: u32,
    /// Horizontal offset from the pen position to the bitmap's left edge.
    pub left: i32,
    /// Vertical offset from the baseline to the bitmap's top edge. Negative is above the baseline,
    /// which is where most of a glyph lives.
    pub top: i32,
    /// Row-major coverage, `width * height` bytes.
    pub coverage: Vec<u8>,
}

/// Shapes text and rasterizes glyphs.
///
/// Implementations are stateful — they cache font handles and their own shaping state — hence
/// `&mut self` throughout.
///
/// # Implementing this
///
/// The contract that matters: **fallback is the implementation's job**. A caller passing text
/// containing 汉 with `FontFamily::Ui` must get back a real glyph if any installed font has one.
/// Returning [`GlyphId::NOTDEF`] when the system could have rendered the character is a defect,
/// and is exactly what disqualified `fontique` on Windows.
pub trait TextLayout {
    /// Lays out `runs` as one block, wrapping at `max_width` device-independent pixels.
    ///
    /// Runs are laid out in order and share a paragraph; a `\n` inside a run starts a new line.
    ///
    /// # Errors
    ///
    /// [`TextError::Platform`] if the platform text stack refuses.
    fn layout(&mut self, runs: &[StyledRun<'_>], max_width: f32) -> Result<Layout, TextError>;

    /// Rasterizes one glyph at `scale_factor` physical pixels per device-independent pixel.
    ///
    /// Returns `Ok(None)` for a glyph with no ink — a space — which is not an error and which the
    /// caller should skip rather than store an empty atlas entry for.
    ///
    /// # Errors
    ///
    /// [`TextError::UnknownGlyph`] if the key did not come from this implementation, or
    /// [`TextError::Platform`] if rasterization fails.
    fn rasterize(
        &mut self,
        key: GlyphKey,
        scale_factor: f32,
    ) -> Result<Option<GlyphImage>, TextError>;

    /// The family name behind a [`FontId`], for diagnostics and tests.
    ///
    /// A `String`, deliberately: it exists so a test can report *which* font fallback chose, not so
    /// the editor can make decisions on the answer.
    fn font_name(&self, font: FontId) -> Option<String>;
}
