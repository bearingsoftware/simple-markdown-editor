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
    PositionedGlyph, StyledRun, TextError, TextLayout, TextStyle,
};

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_GLYPH_OFFSET, DWRITE_GLYPH_RUN, DWRITE_GLYPH_RUN_DESCRIPTION, DWRITE_LINE_METRICS,
    DWRITE_MATRIX, DWRITE_MEASURING_MODE, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC, DWRITE_STRIKETHROUGH, DWRITE_TEXT_RANGE,
    DWRITE_TEXTURE_CLEARTYPE_3x1, DWRITE_UNDERLINE, DWriteCreateFactory, IDWriteFactory,
    IDWriteFontFace, IDWriteInlineObject, IDWritePixelSnapping_Impl, IDWriteTextFormat,
    IDWriteTextLayout, IDWriteTextRenderer, IDWriteTextRenderer_Impl,
};
use windows_core::{BOOL, IUnknown, Ref, implement};

/// Families we ask the system for, by role.
///
/// Deliberately common Windows fonts rather than a bundled one: fallback for anything they do not
/// cover is DirectWrite's job, which is the property this implementation exists to get.
const UI_FAMILY: &str = "Segoe UI";
const MONO_FAMILY: &str = "Consolas";
const LOCALE: &str = "en-us";

/// Text layout and glyph rasterization via DirectWrite.
pub struct DirectWriteText {
    factory: IDWriteFactory,
    /// Text formats are immutable and expensive to build, so they are cached per distinct style.
    formats: HashMap<FormatKey, IDWriteTextFormat>,
    /// Font faces seen so far. A [`FontId`] is an index into this, which is what keeps the concrete
    /// `IDWriteFontFace` on this side of the trait boundary.
    faces: Vec<IDWriteFontFace>,
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

        Ok(Self {
            factory,
            formats: HashMap::new(),
            faces: Vec::new(),
        })
    }

    fn format(&mut self, style: TextStyle) -> Result<IDWriteTextFormat, TextError> {
        let key = FormatKey::from(style);
        if let Some(existing) = self.formats.get(&key) {
            return Ok(existing.clone());
        }

        let family = wide(match style.family {
            FontFamily::Ui => UI_FAMILY,
            FontFamily::Mono => MONO_FAMILY,
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
        let (face, em_size, indices, advances) = unsafe {
            let run = &*glyph_run;
            let Some(face) = run.fontFace.as_ref() else {
                return Ok(());
            };
            let count = run.glyphCount as usize;
            (
                face.clone(),
                run.fontEmSize,
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
        });

        Ok(())
    }

    fn DrawUnderline(
        &self,
        _context: *const core::ffi::c_void,
        _baseline_x: f32,
        _baseline_y: f32,
        _underline: *const DWRITE_UNDERLINE,
        _effect: Ref<'_, IUnknown>,
    ) -> windows_core::Result<()> {
        // Markdown has no underline, and emphasis is drawn as a font change rather than a rule.
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
            }

            unit_cursor = unit_cursor.saturating_add(units);
        }

        let collected = Rc::new(RefCell::new(Vec::new()));
        let renderer: IDWriteTextRenderer = RunCollector {
            runs: Rc::clone(&collected),
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

        Ok(assemble(collected, &line_metrics, &source_map, self))
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
    line_metrics: &[DWRITE_LINE_METRICS],
    source_map: &SourceMap,
    text: &mut DirectWriteText,
) -> Layout {
    // DirectWrite reports runs in visual order but says which baseline each sits on, so grouping by
    // baseline recovers the lines without assuming runs arrive line by line.
    let mut lines: Vec<(f32, Vec<PositionedGlyph>)> = Vec::new();

    for run in runs {
        let font = text.font_id(&run.face);
        let size = FontSize::new(run.em_size);

        let mut pen_x = run.baseline_x;
        let mut glyphs = Vec::with_capacity(run.glyphs.len());
        for (index, (glyph_id, advance)) in run.glyphs.iter().enumerate() {
            let unit = run.text_positions.get(index).copied().unwrap_or(0);
            let start = source_map.byte_at(unit);
            // The span covers the cluster's first byte onward; the renderer only needs a position
            // to hang a caret on, not an exact extent.
            let end = ByteOffset::new(start.get());

            glyphs.push(PositionedGlyph {
                key: GlyphKey {
                    font,
                    glyph: GlyphId::new(*glyph_id),
                    size,
                },
                pos: LayoutPos::new(pen_x, run.baseline_y),
                advance: *advance,
                source: ByteSpan::new(start, end),
            });
            pen_x += advance;
        }

        match lines
            .iter_mut()
            .find(|(baseline, _)| (*baseline - run.baseline_y).abs() < f32::EPSILON)
        {
            Some((_, existing)) => existing.extend(glyphs),
            None => lines.push((run.baseline_y, glyphs)),
        }
    }

    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    Layout {
        lines: lines
            .into_iter()
            .enumerate()
            .map(|(index, (baseline, mut glyphs))| {
                glyphs.sort_by(|a, b| {
                    a.pos
                        .x
                        .partial_cmp(&b.pos.x)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                LayoutLine {
                    baseline,
                    height: line_metrics
                        .get(index)
                        .map_or(0.0, |metrics| metrics.height),
                    glyphs,
                }
            })
            .collect(),
    }
}
