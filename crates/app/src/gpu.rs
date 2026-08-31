//! The quad and glyph pipeline.
//!
//! Hand-written rather than vello, per `docs/research/02-gui-toolkits.md`: vello is alpha and
//! mandates compute shaders, and everything this editor draws is an axis-aligned rectangle — either
//! a glyph sampled from a coverage atlas, or a solid fill for the caret. One pipeline covers both,
//! because a solid fill is a quad sampling a single opaque texel.
//!
//! The atlas packer is a shelf packer: glyphs are placed left to right on rows whose height is set
//! by the first glyph on them. It wastes some space against a true bin packer, and it is a few
//! dozen lines instead of a few hundred. Text is a stream of similar heights, which is the case
//! shelf packing handles well.

use std::collections::HashMap;

use mdedit_text::{GlyphImage, GlyphKey};

/// Side length of the glyph atlas, in physical pixels.
///
/// One page. Running out is handled by reporting it rather than silently dropping glyphs — a
/// document that exhausts 2048² of distinct rasterized glyphs is a case worth seeing, not
/// papering over.
pub const ATLAS_SIZE: u32 = 2048;

/// Padding between packed glyphs, so bilinear sampling cannot bleed a neighbour's coverage in.
const PAD: u32 = 1;

/// Where a glyph lives in the atlas, and how to place it relative to a pen position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasEntry {
    /// Left edge in the atlas, in texels.
    pub u: u32,
    /// Top edge in the atlas, in texels.
    pub v: u32,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Horizontal offset from the pen position to the bitmap's left edge, in physical pixels.
    pub left: i32,
    /// Vertical offset from the baseline to the bitmap's top edge, in physical pixels.
    pub top: i32,
}

/// Why a glyph could not be packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AtlasFull {
    /// No room left on any shelf, and no room for a new one.
    #[error("glyph atlas is full: {ATLAS_SIZE}x{ATLAS_SIZE} exhausted")]
    Exhausted,
}

/// A shelf-packed coverage atlas.
///
/// Owns the CPU-side pixels and the placement map. Uploading is the renderer's job, so this stays
/// testable without a GPU.
#[derive(Debug)]
pub struct Atlas {
    pixels: Vec<u8>,
    entries: HashMap<GlyphKey, AtlasEntry>,
    /// Left edge of the next free slot on the current shelf.
    shelf_x: u32,
    /// Top edge of the current shelf.
    shelf_y: u32,
    /// Height of the current shelf, set by the tallest glyph placed on it.
    shelf_height: u32,
    /// Whether anything has been written since the last upload.
    dirty: bool,
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

impl Atlas {
    /// An empty atlas with the solid texel already placed.
    #[must_use]
    pub fn new() -> Self {
        let mut atlas = Self {
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE) as usize],
            entries: HashMap::new(),
            shelf_x: 0,
            shelf_y: 0,
            shelf_height: 0,
            dirty: true,
        };
        // Texel (0, 0) is fully opaque, so a solid-colour quad is just a glyph quad sampling it.
        // That is what lets the caret and the text share one pipeline and one draw call.
        atlas.pixels[0] = 0xff;
        atlas.shelf_x = 1 + PAD;
        atlas.shelf_height = 1;
        atlas
    }

    /// The atlas pixels, one coverage byte per texel, row-major.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Whether the pixels have changed since [`mark_clean`](Self::mark_clean).
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Records that the current pixels have been uploaded.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// The entry for a glyph already packed.
    #[must_use]
    pub fn get(&self, key: GlyphKey) -> Option<AtlasEntry> {
        self.entries.get(&key).copied()
    }

    /// How many distinct glyphs are packed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether any glyph has been packed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Packs a rasterized glyph, or returns the entry if it is already present.
    ///
    /// # Errors
    ///
    /// [`AtlasFull::Exhausted`] when the page has no room left.
    pub fn insert(&mut self, key: GlyphKey, image: &GlyphImage) -> Result<AtlasEntry, AtlasFull> {
        if let Some(existing) = self.entries.get(&key) {
            return Ok(*existing);
        }

        let (w, h) = (image.width, image.height);
        if w > ATLAS_SIZE || h > ATLAS_SIZE {
            return Err(AtlasFull::Exhausted);
        }

        if self.shelf_x + w > ATLAS_SIZE {
            // Start a new shelf below the tallest glyph on this one.
            self.shelf_y += self.shelf_height + PAD;
            self.shelf_x = 0;
            self.shelf_height = 0;
        }
        if self.shelf_y + h > ATLAS_SIZE {
            return Err(AtlasFull::Exhausted);
        }

        let (u, v) = (self.shelf_x, self.shelf_y);
        for row in 0..h {
            let src = (row * w) as usize;
            let dst = ((v + row) * ATLAS_SIZE + u) as usize;
            self.pixels[dst..dst + w as usize]
                .copy_from_slice(&image.coverage[src..src + w as usize]);
        }

        self.shelf_x += w + PAD;
        self.shelf_height = self.shelf_height.max(h);
        self.dirty = true;

        let entry = AtlasEntry {
            u,
            v,
            width: w,
            height: h,
            left: image.left,
            top: image.top,
        };
        self.entries.insert(key, entry);
        Ok(entry)
    }
}

/// One quad, as the shader consumes it.
///
/// `repr(C)` and `Pod` because it goes straight into a vertex buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct QuadInstance {
    /// Position in physical pixels: x, y, width, height.
    pub rect: [f32; 4],
    /// Atlas region in texels: u, v, width, height.
    pub uv: [f32; 4],
    /// Straight RGBA, 0–1.
    pub colour: [f32; 4],
}

/// The shader. One vertex per corner, generated from the vertex index; no index buffer.
pub const SHADER: &str = r"
struct Globals {
    // Viewport size in physical pixels, and atlas size in texels.
    viewport: vec2<f32>,
    atlas: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) colour: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, instance: Instance) -> VertexOut {
    // Two triangles as a strip: (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));

    let pixel = instance.rect.xy + corner * instance.rect.zw;
    // Pixel space to clip space: y is flipped because text layout grows downward.
    let ndc = vec2<f32>(
        pixel.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.viewport.y * 2.0,
    );

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = (instance.uv.xy + corner * instance.uv.zw) / globals.atlas;
    out.colour = instance.colour;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // The atlas stores coverage, not colour, so one bitmap serves every colour a glyph is drawn in.
    let coverage = textureSample(atlas_tex, atlas_sampler, in.uv).r;
    return vec4<f32>(in.colour.rgb, in.colour.a * coverage);
}

// A picture (MDEDIT-146). Same instance format, same bind-group layout and the same globals — only
// the texture bound and these two entry points differ, which is why it is a second pipeline rather
// than a second renderer.
//
// **The glyph atlas is not widened to hold pictures.** It is `R8Unorm`, and taking it to `Rgba8Unorm`
// would put it from 4 MB to 16 MB against a baseline RSS `AGENTS.md` already calls overrun — and
// would still cap a picture at the 2048-texel page. A picture gets its own texture instead.

@vertex
fn vs_image(@builtin(vertex_index) vertex: u32, instance: Instance) -> VertexOut {
    let corner = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let pixel = instance.rect.xy + corner * instance.rect.zw;
    let ndc = vec2<f32>(
        pixel.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.viewport.y * 2.0,
    );

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    // Not divided by the atlas size: a picture is one texture rather than a region of a shared one,
    // so its instance already carries normalised coordinates.
    out.uv = instance.uv.xy + corner * instance.uv.zw;
    out.colour = instance.colour;
    return out;
}

@fragment
fn fs_image(in: VertexOut) -> @location(0) vec4<f32> {
    // Colour, not coverage. The surface is deliberately configured non-sRGB (see `Renderer::new`),
    // so a `Rgba8Unorm` texture's bytes reach the display unchanged — which is the same treatment
    // the theme's own colours get, and the reason a picture matches the chrome around it.
    return textureSample(atlas_tex, atlas_sampler, in.uv) * in.colour;
}
";
