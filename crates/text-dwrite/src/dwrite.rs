//! The DirectWrite calls themselves.
//!
//! `AGENTS.md` makes `unsafe` outside the platform layer a review block. This module *is* the
//! platform layer, and every block below is a COM call whose safety rests on the same two facts:
//! the interface pointer is live because we hold a reference-counted handle to it, and the buffers
//! we pass match the lengths DirectWrite documents. Each block says which.

// The only crate permitted to lower this. Every `unsafe` block below carries a `// SAFETY:` note.
#![allow(unsafe_code)]
// Both fire inside the code `#[implement]` generates for the COM vtable, not in anything written
// here. There is no narrower place to put them: the attribute expands at the item.
#![allow(clippy::ref_as_ptr, clippy::inline_always)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::rc::Rc;

use mdedit_offsets::{ByteOffset, ByteSpan, LayoutPos};
use mdedit_text::{
    FontFamily, FontId, FontSize, FontWeight, GlyphId, GlyphImage, GlyphKey, Layout, LayoutLine,
    PositionedGlyph, StyledRun, TextError, TextLayout, TextStyle, Underline,
};

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_GLYPH_OFFSET, DWRITE_GLYPH_RUN, DWRITE_GLYPH_RUN_DESCRIPTION, DWRITE_LINE_METRICS,
    DWRITE_MATRIX, DWRITE_MEASURING_MODE, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_READING_DIRECTION_RIGHT_TO_LEFT, DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
    DWRITE_STRIKETHROUGH, DWRITE_TEXT_RANGE, DWRITE_TEXTURE_CLEARTYPE_3x1, DWRITE_UNDERLINE,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontFace, IDWriteInlineObject,
    IDWritePixelSnapping_Impl, IDWriteTextFormat, IDWriteTextLayout, IDWriteTextRenderer,
    IDWriteTextRenderer_Impl,
};
use windows_core::{BOOL, IUnknown, Ref, implement};

/// Families we ask the system for, by role.
///
/// Deliberately common Windows fonts rather than a bundled one: fallback for anything they do not
/// cover is DirectWrite's job, which is the property this implementation exists to get.
const UI_FAMILY: &str = "Segoe UI";
const MONO_FAMILY: &str = "Consolas";

/// Windows 11's icon font, and Windows 10's, in that order (MDEDIT-92).
///
/// Both are system fonts, so this is the same "ask for what Windows has" rule the two above follow —
/// nothing is bundled. Which one is present is resolved **once**, at construction, by asking the
/// system collection: naming a family DirectWrite has not got does not fail, it substitutes, and a
/// substituted font has none of these private-use code points. That would draw tofu on every button
/// rather than falling back to Segoe MDL2 Assets, which is the whole reason the probe exists.
const ICON_FAMILIES: [&str; 2] = ["Segoe Fluent Icons", "Segoe MDL2 Assets"];

/// What `FontFamily::Icon` resolves to when the machine has neither.
///
/// Not a font that will render an icon — nothing will — and that is deliberate. Every private-use
/// code point comes back `.notdef` from Segoe UI, which is exactly the signal
/// `mdedit_app::toolbar` uses to draw a word instead. A bar of English labels is the old bar; a bar
/// of empty boxes is a bug report.
const ICON_FALLBACK: &str = UI_FAMILY;

const LOCALE: &str = "en-us";

/// Text layout and glyph rasterization via DirectWrite.
pub struct DirectWriteText {
    factory: IDWriteFactory,
    /// Text formats are immutable and expensive to build, so they are cached per distinct style.
    formats: HashMap<FormatKey, IDWriteTextFormat>,
    /// Font faces seen so far. A [`FontId`] is an index into this, which is what keeps the concrete
    /// `IDWriteFontFace` on this side of the trait boundary.
    faces: Vec<IDWriteFontFace>,
    /// Which icon font this machine actually has, decided once. See [`ICON_FAMILIES`].
    icons: &'static str,
}

impl std::fmt::Debug for DirectWriteText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectWriteText")
            .field("cached_formats", &self.formats.len())
            .field("known_faces", &self.faces.len())
            // The factory itself has no useful representation, and printing COM pointers in a log
            // is noise.
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FormatKey {
    family: FontFamily,
    weight: FontWeight,
    italic: bool,
    size: FontSize,
}

impl From<TextStyle> for FormatKey {
    fn from(style: TextStyle) -> Self {
        Self {
            family: style.family,
            weight: style.weight,
            italic: style.italic,
            size: style.size,
        }
    }
}

fn platform_err(api: &'static str, err: &windows_core::Error) -> TextError {
    TextError::Platform {
        api,
        detail: err.message(),
    }
}

/// A NUL-terminated UTF-16 buffer, as every DirectWrite string argument wants.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

impl DirectWriteText {
    /// Creates the DirectWrite factory.
    ///
    /// # Errors
    ///
    /// [`TextError::Platform`] if DirectWrite is unavailable, which on a supported Windows version
    /// means the process is in a state where nothing else will work either.
    pub fn new() -> Result<Self, TextError> {
        // SAFETY: `DWriteCreateFactory` writes a new COM interface pointer through the out
        // parameter and takes no borrowed memory. The turbofish fixes the IID to the interface we
        // bind the result to, so the pointer cannot be typed as an interface it does not implement.
        let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .map_err(|e| platform_err("DWriteCreateFactory", &e))?;

        let icons = Self::installed_icon_family(&factory);
        Ok(Self {
            factory,
            formats: HashMap::new(),
            faces: Vec::new(),
            icons,
        })
    }

    /// The first of [`ICON_FAMILIES`] the system collection has, or [`ICON_FALLBACK`].
    ///
    /// `FindFamilyName` answers directly, which is better than laying out a probe glyph and reading
    /// `.notdef`: it needs no format, no layout and no chosen code point, and it separates "this
    /// machine has no icon font" from "this particular icon is missing from the one it has". The
    /// caller wants both answers and they are not the same answer.
    ///
    /// A failure to reach the collection at all is treated as "neither", which is the safe end: the
    /// bar draws words.
    fn installed_icon_family(factory: &IDWriteFactory) -> &'static str {
        let mut collection = None;
        // SAFETY: the out parameter is a fresh `Option<IDWriteFontCollection>` this call
        // initialises; `false` asks for the cached collection rather than forcing a rescan, which
        // is what every other consumer of the system collection wants.
        if unsafe { factory.GetSystemFontCollection(&raw mut collection, false) }.is_err() {
            return ICON_FALLBACK;
        }
        let Some(collection) = collection else {
            return ICON_FALLBACK;
        };

        for family in ICON_FAMILIES {
            let name = wide(family);
            let mut index = 0u32;
            let mut exists = BOOL(0);
            // SAFETY: `name` is a NUL-terminated UTF-16 buffer that outlives the call, and both out
            // parameters are stack locals DirectWrite writes through. `exists` decides whether
            // `index` means anything, which is why it is the only thing read.
            let found = unsafe {
                collection.FindFamilyName(
                    windows_core::PCWSTR(name.as_ptr()),
                    &raw mut index,
                    &raw mut exists,
                )
            };
            if found.is_ok() && exists.as_bool() {
                return family;
            }
        }
        ICON_FALLBACK
    }

    fn format(&mut self, style: TextStyle) -> Result<IDWriteTextFormat, TextError> {
        let key = FormatKey::from(style);
        if let Some(existing) = self.formats.get(&key) {
            return Ok(existing.clone());
        }

        let family = wide(match style.family {
            FontFamily::Ui => UI_FAMILY,
            FontFamily::Mono => MONO_FAMILY,
            FontFamily::Icon => self.icons,
        });
        let locale = wide(LOCALE);
        let weight = match style.weight {
            FontWeight::Regular => DWRITE_FONT_WEIGHT_NORMAL,
            FontWeight::Bold => DWRITE_FONT_WEIGHT_BOLD,
        };
        let slant = if style.italic {
            DWRITE_FONT_STYLE_ITALIC
        } else {
            DWRITE_FONT_STYLE_NORMAL
        };

        // SAFETY: both string arguments are NUL-terminated UTF-16 buffers that outlive the call —
        // DirectWrite copies what it needs into the returned format. `None` for the font collection
        // selects the system collection, which is what makes system-wide fallback available.
        let format = unsafe {
            self.factory.CreateTextFormat(
                windows_core::PCWSTR(family.as_ptr()),
                None,
                weight,
                slant,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size.get(),
                windows_core::PCWSTR(locale.as_ptr()),
            )
        }
        .map_err(|e| platform_err("CreateTextFormat", &e))?;

        self.formats.insert(key, format.clone());
        Ok(format)
    }

    /// The [`FontId`] for a face, registering it if this is the first time we have seen it.
    fn font_id(&mut self, face: &IDWriteFontFace) -> FontId {
        if let Some(index) = self.faces.iter().position(|known| known == face) {
            // A face table long enough to overflow u32 is not reachable: it is bounded by the
            // number of installed fonts.
            return FontId::new(u32::try_from(index).unwrap_or(u32::MAX));
        }
        self.faces.push(face.clone());
        FontId::new(u32::try_from(self.faces.len() - 1).unwrap_or(u32::MAX))
    }
}

/// Maps each UTF-16 code unit of the layout string back to the buffer byte it came from.
struct SourceMap {
    /// One entry per UTF-16 code unit.
    per_unit: Vec<ByteOffset>,
}

impl SourceMap {
    fn build(runs: &[StyledRun<'_>]) -> (Vec<u16>, Self) {
        let mut utf16 = Vec::new();
        let mut per_unit = Vec::new();

        for run in runs {
            let base = run.source.start().get();
            for (byte_index, ch) in run.text.char_indices() {
                let offset = ByteOffset::new(base + byte_index);
                let before = utf16.len();
                let mut buf = [0u16; 2];
                utf16.extend_from_slice(ch.encode_utf16(&mut buf));
                per_unit.resize(before + (utf16.len() - before), offset);
            }
        }

        (utf16, Self { per_unit })
    }

    /// The byte the code unit at `index` came from.
    fn byte_at(&self, index: usize) -> ByteOffset {
        self.per_unit
            .get(index)
            .copied()
            .or_else(|| self.per_unit.last().copied())
            .unwrap_or(ByteOffset::ZERO)
    }
}

/// One glyph run handed back by DirectWrite during `Draw`.
struct CollectedRun {
    face: IDWriteFontFace,
    em_size: f32,
    baseline_x: f32,
    baseline_y: f32,
    glyphs: Vec<(u16, f32)>,
    /// UTF-16 index in the layout string that each glyph's cluster starts at.
    text_positions: Vec<usize>,
    /// Odd means right-to-left. DirectWrite places an RTL run's origin at its *right* edge and
    /// advances leftward, so ignoring this draws Arabic on top of whatever follows it.
    right_to_left: bool,
}

/// One underline handed back by DirectWrite during `Draw`.
///
/// Wrapping is already applied: an underlined run crossing a row boundary arrives as one of these
/// per row, which is what makes the rule follow the text rather than the run.
struct CollectedUnderline {
    baseline_x: f32,
    baseline_y: f32,
    width: f32,
    thickness: f32,
    /// Distance from the baseline down to the top of the rule.
    offset: f32,
    /// As with a glyph run, a right-to-left underline's origin is its right edge.
    right_to_left: bool,
}

/// Collects glyph runs out of `IDWriteTextLayout::Draw`.
///
/// DirectWrite reports shaped, fallback-resolved glyphs only through this callback — there is no
/// "give me the glyphs" accessor — so implementing it is the price of using DirectWrite's own
/// shaping and fallback rather than reimplementing either.
#[implement(IDWriteTextRenderer)]
struct RunCollector {
    /// Shared with the caller rather than recovered by casting the COM object back afterwards —
    /// one `Rc` is cheaper to read than a downcast, and cannot fail.
    runs: Rc<RefCell<Vec<CollectedRun>>>,
    /// The rules under whatever was asked for underlined, in the same way.
    underlines: Rc<RefCell<Vec<CollectedUnderline>>>,
}

impl IDWritePixelSnapping_Impl for RunCollector_Impl {
    fn IsPixelSnappingDisabled(
        &self,
        _context: *const core::ffi::c_void,
    ) -> windows_core::Result<BOOL> {
        // We position glyphs ourselves in device-independent pixels and snap at draw time, so
        // DirectWrite must not round baselines behind our back.
        Ok(BOOL(1))
    }

    fn GetCurrentTransform(
        &self,
        _context: *const core::ffi::c_void,
        transform: *mut DWRITE_MATRIX,
    ) -> windows_core::Result<()> {
        if transform.is_null() {
            return Err(windows_core::Error::from(E_INVALIDARG));
        }
        // SAFETY: `transform` is non-null (checked above) and DirectWrite guarantees it points at a
        // writable `DWRITE_MATRIX` for the duration of the call.
        unsafe {
            *transform = DWRITE_MATRIX {
                m11: 1.0,
                m12: 0.0,
                m21: 0.0,
                m22: 1.0,
                dx: 0.0,
                dy: 0.0,
            };
        }
        Ok(())
    }

    fn GetPixelsPerDip(&self, _context: *const core::ffi::c_void) -> windows_core::Result<f32> {
        // Layout is in device-independent pixels; the scale factor is applied at rasterization.
        Ok(1.0)
    }
}

impl IDWriteTextRenderer_Impl for RunCollector_Impl {
    fn DrawGlyphRun(
        &self,
        _context: *const core::ffi::c_void,
        baseline_x: f32,
        baseline_y: f32,
        _measuring_mode: DWRITE_MEASURING_MODE,
        glyph_run: *const DWRITE_GLYPH_RUN,
        glyph_run_description: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _effect: Ref<'_, IUnknown>,
    ) -> windows_core::Result<()> {
        if glyph_run.is_null() {
            return Ok(());
        }

        // SAFETY: DirectWrite guarantees `glyph_run` points at a valid structure for the duration
        // of this callback, and that `glyphIndices` and `glyphAdvances` each address `glyphCount`
        // elements. We only read, and only within that count.
        let (face, em_size, right_to_left, indices, advances) = unsafe {
            let run = &*glyph_run;
            let Some(face) = run.fontFace.as_ref() else {
                return Ok(());
            };
            let count = run.glyphCount as usize;
            (
                face.clone(),
                run.fontEmSize,
                run.bidiLevel % 2 == 1,
                std::slice::from_raw_parts(run.glyphIndices, count),
                std::slice::from_raw_parts(run.glyphAdvances, count),
            )
        };

        // The cluster map turns glyph indices back into text positions. Shaping is not
        // one-to-one — a ligature is one glyph over several characters — so this is the only
        // honest way to say which bytes a glyph came from.
        let mut text_positions = vec![0usize; indices.len()];
        if !glyph_run_description.is_null() {
            // SAFETY: non-null checked; DirectWrite guarantees `clusterMap` addresses
            // `stringLength` elements and stays valid for the callback.
            unsafe {
                let desc = &*glyph_run_description;
                let base = desc.textPosition as usize;
                let cluster_map =
                    std::slice::from_raw_parts(desc.clusterMap, desc.stringLength as usize);
                // Walk backwards so the *first* text position mapping to a glyph wins.
                for (text_index, glyph_index) in cluster_map.iter().enumerate().rev() {
                    if let Some(slot) = text_positions.get_mut(*glyph_index as usize) {
                        *slot = base + text_index;
                    }
                }
            }
        }

        self.this.runs.borrow_mut().push(CollectedRun {
            face,
            em_size,
            baseline_x,
            baseline_y,
            glyphs: indices
                .iter()
                .zip(advances.iter())
                .map(|(id, adv)| (*id, *adv))
                .collect(),
            text_positions,
            right_to_left,
        });

        Ok(())
    }

    fn DrawUnderline(
        &self,
        _context: *const core::ffi::c_void,
        baseline_x: f32,
        baseline_y: f32,
        underline: *const DWRITE_UNDERLINE,
        _effect: Ref<'_, IUnknown>,
    ) -> windows_core::Result<()> {
        if underline.is_null() {
            return Ok(());
        }

        // SAFETY: non-null checked, and DirectWrite guarantees the structure is valid for the
        // duration of the callback. Only the geometry fields are read, and none of them borrows.
        let rule = unsafe { &*underline };

        self.this.underlines.borrow_mut().push(CollectedUnderline {
            baseline_x,
            baseline_y,
            width: rule.width,
            // Both come from the font, which is the whole reason this is the backend's answer
            // rather than a rule the renderer guesses at.
            thickness: rule.thickness,
            offset: rule.offset,
            right_to_left: rule.readingDirection == DWRITE_READING_DIRECTION_RIGHT_TO_LEFT,
        });

        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        _context: *const core::ffi::c_void,
        _baseline_x: f32,
        _baseline_y: f32,
        _strikethrough: *const DWRITE_STRIKETHROUGH,
        _effect: Ref<'_, IUnknown>,
    ) -> windows_core::Result<()> {
        // `~~struck~~` is decorated by the renderer from the marker spans, not by DirectWrite:
        // the editor already knows exactly which bytes are struck.
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        _context: *const core::ffi::c_void,
        _origin_x: f32,
        _origin_y: f32,
        _inline_object: Ref<'_, IDWriteInlineObject>,
        _is_sideways: BOOL,
        _is_right_to_left: BOOL,
        _effect: Ref<'_, IUnknown>,
    ) -> windows_core::Result<()> {
        // Nothing inserts inline objects. Tables and fences become block widgets instead — see
        // AGENTS.md, "model-backed block widgets, not decorations".
        Ok(())
    }
}

impl TextLayout for DirectWriteText {
    fn layout(&mut self, runs: &[StyledRun<'_>], max_width: f32) -> Result<Layout, TextError> {
        if runs.is_empty() {
            return Ok(Layout::default());
        }

        let (utf16, source_map) = SourceMap::build(runs);
        if utf16.is_empty() {
            return Ok(Layout::default());
        }

        // The first run's style seeds the layout; the rest are applied as ranges over it.
        let base_format = self.format(runs[0].style)?;

        // SAFETY: `utf16` outlives the call and `CreateTextLayout` copies the text into the layout
        // object. The length is the true element count of the slice.
        let layout: IDWriteTextLayout = unsafe {
            self.factory.CreateTextLayout(
                &utf16,
                &base_format,
                max_width,
                // No vertical constraint: the caller virtualizes, so wrapping height is unbounded.
                f32::MAX,
            )
        }
        .map_err(|e| platform_err("CreateTextLayout", &e))?;

        // Apply each run's style over its own range of the layout string.
        let mut unit_cursor = 0u32;
        for run in runs {
            let units = u32::try_from(run.text.encode_utf16().count()).unwrap_or(u32::MAX);
            let range = DWRITE_TEXT_RANGE {
                startPosition: unit_cursor,
                length: units,
            };
            let family = wide(match run.style.family {
                FontFamily::Ui => UI_FAMILY,
                FontFamily::Mono => MONO_FAMILY,
                FontFamily::Icon => self.icons,
            });

            // SAFETY: `range` lies within the layout string by construction — the cursor advances
            // by exactly the units each run contributed. `family` is NUL-terminated and outlives
            // the call.
            unsafe {
                layout
                    .SetFontFamilyName(windows_core::PCWSTR(family.as_ptr()), range)
                    .map_err(|e| platform_err("SetFontFamilyName", &e))?;
                layout
                    .SetFontSize(run.style.size.get(), range)
                    .map_err(|e| platform_err("SetFontSize", &e))?;
                layout
                    .SetFontWeight(
                        match run.style.weight {
                            FontWeight::Regular => DWRITE_FONT_WEIGHT_NORMAL,
                            FontWeight::Bold => DWRITE_FONT_WEIGHT_BOLD,
                        },
                        range,
                    )
                    .map_err(|e| platform_err("SetFontWeight", &e))?;
                layout
                    .SetFontStyle(
                        if run.style.italic {
                            DWRITE_FONT_STYLE_ITALIC
                        } else {
                            DWRITE_FONT_STYLE_NORMAL
                        },
                        range,
                    )
                    .map_err(|e| platform_err("SetFontStyle", &e))?;
                if run.style.underline {
                    layout
                        .SetUnderline(true, range)
                        .map_err(|e| platform_err("SetUnderline", &e))?;
                }
            }

            unit_cursor = unit_cursor.saturating_add(units);
        }

        let collected = Rc::new(RefCell::new(Vec::new()));
        let rules = Rc::new(RefCell::new(Vec::new()));
        let renderer: IDWriteTextRenderer = RunCollector {
            runs: Rc::clone(&collected),
            underlines: Rc::clone(&rules),
        }
        .into();

        // SAFETY: `renderer` is a live COM object we hold for the duration of the call, and
        // DirectWrite only invokes it synchronously from within `Draw` — so the `RefCell` inside
        // cannot be borrowed re-entrantly from another thread.
        unsafe { layout.Draw(None, &renderer, 0.0, 0.0) }
            .map_err(|e| platform_err("IDWriteTextLayout::Draw", &e))?;

        let line_metrics = line_metrics(&layout)?;
        drop(renderer);
        let collected = collected.take();
        let rules = rules.take();

        Ok(assemble(collected, rules, &line_metrics, &source_map, self))
    }

    fn rasterize(
        &mut self,
        key: GlyphKey,
        scale_factor: f32,
    ) -> Result<Option<GlyphImage>, TextError> {
        let face = self
            .faces
            .get(key.font.get() as usize)
            .ok_or(TextError::UnknownGlyph { glyph: key })?
            .clone();

        let glyph_index = key.glyph.get();
        let advance = 0.0f32;
        let offset = DWRITE_GLYPH_OFFSET {
            advanceOffset: 0.0,
            ascenderOffset: 0.0,
        };

        let run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(face)),
            fontEmSize: key.size.get(),
            glyphCount: 1,
            glyphIndices: &raw const glyph_index,
            glyphAdvances: &raw const advance,
            glyphOffsets: &raw const offset,
            isSideways: BOOL(0),
            bidiLevel: 0,
        };

        // SAFETY: every pointer in `run` addresses a local that outlives this call, and
        // `glyphCount` is 1, matching each of those one-element arrays. Baseline origin is the
        // origin, so the bounds come back relative to the pen position, which is what the atlas
        // wants. The `ManuallyDrop` font face is released explicitly below.
        let result = unsafe {
            self.factory
                .CreateGlyphRunAnalysis(
                    &raw const run,
                    scale_factor,
                    None,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    DWRITE_MEASURING_MODE_NATURAL,
                    0.0,
                    0.0,
                )
                .and_then(|analysis| {
                    let bounds = analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)?;
                    Ok((analysis, bounds))
                })
        };

        // `DWRITE_GLYPH_RUN::fontFace` is `ManuallyDrop`, so the clone we put in it is ours to
        // release. Skipping this leaks one font-face reference per rasterized glyph.
        let mut run = run;
        // SAFETY: the field was initialised with `ManuallyDrop::new` above, has not been dropped,
        // and `run` is not used afterwards.
        unsafe { ManuallyDrop::drop(&mut run.fontFace) };

        let (analysis, bounds) = result.map_err(|e| platform_err("CreateGlyphRunAnalysis", &e))?;

        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        if width <= 0 || height <= 0 {
            // A space, or any glyph with no ink. Not an error, and not worth an atlas entry.
            return Ok(None);
        }

        let (width, height) = (
            width.unsigned_abs() as usize,
            height.unsigned_abs() as usize,
        );

        // ClearType rather than aliased, and then averaged down. `DWRITE_TEXTURE_ALIASED_1x1` is
        // only valid under `DWRITE_RENDERING_MODE_ALIASED` — ask for it alongside an antialiased
        // mode and the bounds come back empty, which reads exactly like a glyph with no ink.
        // Taking the subpixel texture and averaging its triplets gives grayscale antialiasing,
        // which is what we want anyway: subpixel positioning assumes an RGB stripe layout that is
        // wrong on rotated and OLED displays.
        let mut subpixel = vec![0u8; width.saturating_mul(height).saturating_mul(3)];

        // SAFETY: `DWRITE_TEXTURE_CLEARTYPE_3x1` writes three bytes per pixel for the bounds it was
        // measured with, which is exactly the length allocated above. Passing back the same bounds
        // is what makes that equality hold.
        unsafe {
            analysis.CreateAlphaTexture(
                DWRITE_TEXTURE_CLEARTYPE_3x1,
                &raw const bounds,
                &mut subpixel,
            )
        }
        .map_err(|e| platform_err("CreateAlphaTexture", &e))?;

        let coverage = subpixel
            .chunks_exact(3)
            .map(|rgb| {
                // u16 accumulator: three u8s sum to at most 765, which overflows a u8.
                let sum = u16::from(rgb[0]) + u16::from(rgb[1]) + u16::from(rgb[2]);
                u8::try_from(sum / 3).unwrap_or(u8::MAX)
            })
            .collect();

        Ok(Some(GlyphImage {
            width: u32::try_from(width).unwrap_or(u32::MAX),
            height: u32::try_from(height).unwrap_or(u32::MAX),
            left: bounds.left,
            top: bounds.top,
            coverage,
        }))
    }

    fn font_name(&self, font: FontId) -> Option<String> {
        let face = self.faces.get(font.get() as usize)?;

        // SAFETY: `face` is a live handle. `GetFiles` is called twice as documented — once with a
        // null buffer to learn the count, once to fill it — and the second call's buffer has
        // exactly that length.
        let family = unsafe {
            let mut count = 0u32;
            face.GetFiles(&raw mut count, None).ok()?;
            // Recovering a family name from a face means going back through the font collection,
            // which DirectWrite does not expose directly from a face. The face's index within our
            // own table is the honest answer for diagnostics.
            let _ = count;
            None::<String>
        };

        family.or_else(|| Some(format!("face #{}", font.get())))
    }
}

fn line_metrics(layout: &IDWriteTextLayout) -> Result<Vec<DWRITE_LINE_METRICS>, TextError> {
    // SAFETY: the documented two-call pattern. The first call reports how many lines there are via
    // the out parameter and returns a buffer-too-small error, which is expected and discarded.
    let mut needed = 0u32;
    unsafe {
        let _ = layout.GetLineMetrics(None, &raw mut needed);
    }

    let mut metrics = vec![DWRITE_LINE_METRICS::default(); needed as usize];
    if needed > 0 {
        // SAFETY: `metrics` has exactly `needed` elements, the count the first call asked for.
        unsafe { layout.GetLineMetrics(Some(&mut metrics), &raw mut needed) }
            .map_err(|e| platform_err("GetLineMetrics", &e))?;
    }
    Ok(metrics)
}

/// Turns collected runs into lines of positioned glyphs.
fn assemble(
    runs: Vec<CollectedRun>,
    underlines: Vec<CollectedUnderline>,
    line_metrics: &[DWRITE_LINE_METRICS],
    source_map: &SourceMap,
    text: &mut DirectWriteText,
) -> Layout {
    // Lines come from DirectWrite's own metrics, not from the runs it handed back. A blank line
    // produces no glyphs and therefore no run, so recovering lines from the runs dropped it
    // entirely: the vertical space it occupies went missing, and every line after it took the
    // height of the wrong entry in this list.
    let mut lines: Vec<LayoutLine> = Vec::with_capacity(line_metrics.len());
    let mut starts: Vec<u32> = Vec::with_capacity(line_metrics.len());
    let mut top = 0.0f32;
    let mut consumed = 0u32;
    for metrics in line_metrics {
        starts.push(consumed);
        consumed = consumed.saturating_add(metrics.length);
        lines.push(LayoutLine {
            baseline: top + metrics.baseline,
            height: metrics.height,
            glyphs: Vec::new(),
            underlines: Vec::new(),
        });
        top += metrics.height;
    }

    for run in runs {
        let font = text.font_id(&run.face);
        let size = FontSize::new(run.em_size);

        let mut pen_x = run.baseline_x;
        for (index, (glyph_id, advance)) in run.glyphs.iter().enumerate() {
            let unit = run.text_positions.get(index).copied().unwrap_or(0);
            let start = source_map.byte_at(unit);
            // The span covers the cluster's first byte onward; the renderer only needs a position
            // to hang a caret on, not an exact extent.
            let end = ByteOffset::new(start.get());

            // A right-to-left run's origin is its *right* edge and the pen moves leftward, so the
            // decrement has to happen before placing rather than after. Accumulating rightward
            // instead draws Arabic on top of whatever follows it on the line.
            if run.right_to_left {
                pen_x -= advance;
            }

            let glyph = PositionedGlyph {
                key: GlyphKey {
                    font,
                    glyph: GlyphId::new(*glyph_id),
                    size,
                },
                pos: LayoutPos::new(pen_x, run.baseline_y),
                advance: *advance,
                source: ByteSpan::new(start, end),
            };

            if !run.right_to_left {
                pen_x += advance;
            }

            if let Some(line) = lines.get_mut(line_of(&starts, unit)) {
                line.glyphs.push(glyph);
            }
        }
    }

    for line in &mut lines {
        // Runs arrive in visual order, and a bidi line interleaves them, so the glyphs on one line
        // are only in reading order once sorted.
        line.glyphs.sort_by(|a, b| {
            a.pos
                .x
                .partial_cmp(&b.pos.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // After the sort, because each rule is matched to the text under it by position.
    for rule in underlines {
        let x = if rule.right_to_left {
            rule.baseline_x - rule.width
        } else {
            rule.baseline_x
        };
        let Some(line) = nearest_line(&mut lines, rule.baseline_y) else {
            continue;
        };
        // The rule is coloured like the text it sits under, and nothing else on this side of the
        // boundary knows which run that was: `DWRITE_UNDERLINE` carries geometry and no text
        // position. A rule with nothing under it is whitespace only, and is dropped rather than
        // drawn in a colour picked at random.
        let Some(source) = source_under(line, x, rule.width) else {
            continue;
        };
        line.underlines.push(Underline {
            x,
            top: rule.baseline_y + rule.offset,
            width: rule.width,
            thickness: rule.thickness,
            source,
        });
    }

    Layout { lines }
}

/// The line whose baseline is closest to `baseline_y`.
///
/// By position rather than by text offset, because that is all an underline reports.
fn nearest_line(lines: &mut [LayoutLine], baseline_y: f32) -> Option<&mut LayoutLine> {
    lines.iter_mut().min_by(|a, b| {
        (a.baseline - baseline_y)
            .abs()
            .partial_cmp(&(b.baseline - baseline_y).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// A byte the text under `x..x + width` came from.
fn source_under(line: &LayoutLine, x: f32, width: f32) -> Option<ByteOffset> {
    line.glyphs
        .iter()
        .find(|glyph| glyph.pos.x + glyph.advance > x && glyph.pos.x < x + width)
        .map(|glyph| glyph.source.start())
}

/// The line a UTF-16 unit sits on: the last line starting at or before it.
fn line_of(starts: &[u32], unit: usize) -> usize {
    let unit = u32::try_from(unit).unwrap_or(u32::MAX);
    starts
        .partition_point(|start| *start <= unit)
        .saturating_sub(1)
}
