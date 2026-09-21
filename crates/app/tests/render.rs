//! One real frame, through both real pipelines, on a device with no window (MDEDIT-153).
//!
//! # Why this exists
//!
//! MDEDIT-146 shipped an abort to a human. It added a second pipeline for pictures with a `Linear`
//! sampler and reused the glyph pipeline's bind-group layout, which declares `NonFiltering`; wgpu
//! refuses that pairing, and the first frame that actually uploaded a picture killed the process:
//!
//! ```text
//! In Device::create_bind_group, label = 'globals'
//!   Sampler binding 2 expects filtering = false, but given a sampler with filtering = true
//! ```
//!
//! Twelve CI jobs passed. Sixteen hundred tests passed. Two review passes found four other defects
//! and not that one. `AGENTS.md` said rendering could only be checked by hand — but
//! `docs/research/07-rendering-in-ci.md` (MDEDIT-123, measured) had already shown the premise was
//! wrong: **rendering needs a device, not a window**, and a hosted runner has WARP.
//!
//! # What it does and does not check
//!
//! It builds the **real** layout, samplers, pipelines and bind groups — `crate::gpu`'s, not a
//! replica — and draws one frame into an offscreen texture. It then asserts that ink landed where
//! each pipeline was told to draw.
//!
//! **And then the editor's own frame** (MDEDIT-134). The quads above are this file's; those are
//! `Editor::build_frame`'s, drawn against the atlas the editor packed — the two halves that have to
//! agree about where a glyph is, and the half a stub can never check. Three claims, each of which
//! was made to fail before it was believed: ink where the caret is, ink inside the document
//! letter's own box **against the row beside it**, and the theme's background on the row below.
//!
//! The comparisons are where the review of #182 found this file's own failure mode twice over. The
//! glyph was compared against the theme's page colour, which its row does not have — the caret-line
//! tint covers it — so the assertion held whether or not the glyph drew, and an atlas that packed
//! nothing passed. And the frame was cleared to this file's black rather than the theme's
//! background, so "the page is the page" compared a constant this file owns against a colour the
//! editor never put there.
//!
//! It is **not** a screenshot test. Nothing here is golden, no hash is committed and no reference
//! image exists. MDEDIT-123 found the pixels were byte-identical across two machines and said
//! plainly that this is evidence rather than a guarantee — a Windows update that reships a font
//! would break a golden test on an unrelated pull request, with a diff explaining nothing.
//!
//! **It does not reach `Renderer` itself, and MDEDIT-146's *other* defect lived there.** The
//! instance buffer sized for sixteen pictures while `image_capacity` claimed sixty-four was
//! bookkeeping inside a struct that owns a surface, and a surface needs a window. What is reachable
//! is everything a surface is not required for — which was enough for the defect that aborted, and
//! is not everything. Widening `Renderer` so its non-surface half can be built headlessly would
//! close the rest, and is not attempted here.
//!
//! # It has to be able to fail for the reason it exists
//!
//! MDEDIT-146's by-hand check ran twice and passed twice **without ever drawing a picture**, because
//! session restore won the active tab. "It did not crash" was satisfied by a process that never
//! reached the code under test. So this asserts ink rather than survival, and it **requires** an
//! adapter rather than skipping when one is absent: WARP ships with Windows, so no adapter means
//! something is wrong, not that there is nothing to test.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_text_dwrite::DirectWriteText;

use mdedit_app::gpu::{
    ATLAS_SIZE, Atlas, QuadInstance, SHADER, create_atlas_parts, create_bind_layout,
    create_image_parts, create_pipeline, make_bind_group,
};

/// Small: WARP is a software rasteriser and every pixel is CPU work.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;

/// What the frame is cleared to, as bytes — "ink" means "not this".
const BACKGROUND_RGBA: [u8; 4] = [0, 0, 0, 0xff];

/// The same, as the clear takes it.
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// Everything built once, because the first draw costs ~550 ms and every one after it ~2.5 ms.
///
/// MDEDIT-123 measured that: the driver compiles the shader on the first draw. One fixture and many
/// assertions is the shape that follows from it, and it is why this file has one `#[test]`.
struct Harness {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Harness {
    /// A device on the software adapter, which is the one CI has.
    ///
    /// `force_fallback_adapter` asks for WARP specifically, so a developer's machine exercises the
    /// same adapter the runner will. Panics rather than skipping — see the module docs.
    fn new() -> Self {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(descriptor);

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: true,
            ..Default::default()
        }))
        .expect("WARP ships with Windows, so a fallback adapter is always available");

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mdedit-153"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        }))
        .expect("the fallback adapter gives a device");

        Self { device, queue }
    }

    /// An `Rgba8Unorm` target, matching the non-sRGB surface `Renderer::new` deliberately picks.
    fn target(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// The frame's pixels, without the row padding a copy imposes.
    fn read_back(&self, target: &wgpu::Texture) -> Pixels {
        let (width, height) = (target.width(), target.height());
        // `bytes_per_row` must be a multiple of 256 for a texture-to-buffer copy.
        let row = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the queue drains");

        let mapped = slice.get_mapped_range().expect("the buffer maps");
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let start = (y * row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Pixels {
            data: pixels,
            width,
            height,
        }
    }
}

/// A frame read back from the GPU, which knows its own size.
///
/// **The dimensions come from the texture**, not from the caller. `read_back` used to take them,
/// which left them free to disagree with the texture being copied: a 240-wide frame read as 64 wide
/// answers plausible nonsense, with nothing in the signature to stop it (review of #182).
struct Pixels {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl Pixels {
    /// The pixel at `(x, y)`, as RGBA.
    ///
    /// Bounds-checked, because the index arithmetic would otherwise fold an `x` past the right edge
    /// into the next row and answer a plausible wrong colour rather than failing.
    fn at(&self, (x, y): (u32, u32)) -> [u8; 4] {
        assert!(
            x < self.width && y < self.height,
            "({x}, {y}) is outside the {}x{} frame",
            self.width,
            self.height,
        );
        let at = ((y * self.width + x) * 4) as usize;
        [
            self.data[at],
            self.data[at + 1],
            self.data[at + 2],
            self.data[at + 3],
        ]
    }
}

/// Whether `found` is nearer `wanted` than `other`, summed over the channels.
///
/// **Nearness rather than equality, because the picture sampler is `Linear` and that is the one
/// under test.** No pixel centre lands exactly on a texel centre, so a two-texel texture always
/// comes back a few units blended — the first version of this asserted `[64, 128, 192]` and got
/// `[68, 126, 187]`. Which texel a point *resolves to* is the property; the exact blend is the
/// filter's business and asserting it would be asserting the rasteriser.
fn nearer(found: [u8; 4], wanted: [u8; 4], other: [u8; 4]) -> bool {
    let distance = |to: [u8; 4]| -> i32 {
        found
            .iter()
            .zip(to)
            .map(|(a, b)| i32::from(*a).abs_diff(i32::from(b)).cast_signed())
            .sum()
    };
    distance(wanted) < distance(other)
}

/// The two texels of the stand-in picture: left and right.
const LEFT_TEXEL: [u8; 4] = [0x40, 0x80, 0xc0, 0xff];
const RIGHT_TEXEL: [u8; 4] = [0xc0, 0x40, 0x20, 0xff];

/// A **two**-texel RGBA texture, standing in for a decoded picture.
///
/// **Two rather than one, and that is the point.** `vs_image` uses the instance's `uv` directly
/// while `vs_main` divides it by the atlas size — and against a 1×1 texture those are
/// indistinguishable, because every sample lands on the only texel there is. The image pipeline
/// could be wired to the glyph vertex shader and nothing would notice.
///
/// With two texels the quad's left half samples one and its right half the other under `vs_image`,
/// where `vs_main` would compress the whole 0..1 range into a 1/2048 sliver of the first. Found by
/// mis-wiring the pipeline on purpose and watching the test pass.
fn picture_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("picture"),
        size: wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[LEFT_TEXEL, RIGHT_TEXEL].concat(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// A quad covering the given rectangle, sampling the atlas's solid texel at `colour`.
fn solid(rect: [f32; 4], colour: [f32; 4]) -> QuadInstance {
    QuadInstance {
        rect,
        uv: [0.0, 0.0, 1.0, 1.0],
        colour,
    }
}

/// Both pipelines and both bind groups, built the way `Renderer::new` builds them.
struct Built {
    quads: wgpu::RenderPipeline,
    pictures: wgpu::RenderPipeline,
    atlas_binding: wgpu::BindGroup,
    picture_binding: wgpu::BindGroup,
    instances: wgpu::Buffer,
    picture_instances: wgpu::Buffer,
    /// Kept so the editor frame can bind its own atlas to the layout these pipelines were built
    /// against, rather than compiling a second shader for it (MDEDIT-134).
    bind_layout: wgpu::BindGroupLayout,
}

/// Apart from the test body, so the test reads as draw-then-assert.
///
/// **The interesting failure is in here**, not in the drawing: `make_bind_group` is where a sampler
/// and a layout that disagree abort the process.
fn build(harness: &Harness) -> Built {
    let (device, queue) = (&harness.device, &harness.queue);
    let format = wgpu::TextureFormat::Rgba8Unorm;

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quads"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });

    // The real ones. A replica of this wiring would have been written correctly and proved nothing.
    let bind_layout = create_bind_layout(device);
    let (atlas_texture, atlas_sampler) = create_atlas_parts(device);
    let quads = create_pipeline(
        device,
        &shader,
        &bind_layout,
        format,
        ("vs_main", "fs_main"),
    );
    // The buffer is the production one too, not a local: `Renderer::draw` keeps a separate instance
    // buffer per pipeline, and a test that shared one would be a shape the editor never runs.
    let (pictures, picture_sampler, picture_instances) =
        create_image_parts(device, &shader, &bind_layout, format);

    let globals = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("globals"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    #[allow(
        clippy::cast_precision_loss,
        reason = "viewport and atlas dimensions are far below f32's exact-integer limit"
    )]
    queue.write_buffer(
        &globals,
        0,
        bytemuck::cast_slice(&[
            WIDTH as f32,
            HEIGHT as f32,
            ATLAS_SIZE as f32,
            ATLAS_SIZE as f32,
        ]),
    );

    // The atlas's solid texel, so a quad drawn through the glyph pipeline is opaque.
    let atlas = Atlas::new();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &atlas_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        atlas.pixels(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS_SIZE),
            rows_per_image: Some(ATLAS_SIZE),
        },
        wgpu::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
    );

    // **The call MDEDIT-146 aborted on.** A `Linear` sampler in a layout that declared
    // `NonFiltering` is a validation error, raised here rather than at pipeline creation — so
    // nothing short of actually building this bind group finds it.
    let picture = picture_texture(device, queue);
    let atlas_binding = make_bind_group(
        device,
        &bind_layout,
        &globals,
        &atlas_texture,
        &atlas_sampler,
    );
    let picture_binding =
        make_bind_group(device, &bind_layout, &globals, &picture, &picture_sampler);

    // Left half a picture, right half a glyph-pipeline fill, so each pipeline owns a region and a
    // silent failure in either one is a region that stayed background.
    let picture_quad = solid([0.0, 0.0, 32.0, 32.0], [1.0, 1.0, 1.0, 1.0]);
    let glyph_quad = solid([32.0, 0.0, 32.0, 32.0], [1.0, 0.0, 0.0, 1.0]);
    let instances = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: size_of::<QuadInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&instances, 0, bytemuck::cast_slice(&[glyph_quad]));
    queue.write_buffer(&picture_instances, 0, bytemuck::cast_slice(&[picture_quad]));

    Built {
        quads,
        pictures,
        atlas_binding,
        picture_binding,
        instances,
        picture_instances,
        bind_layout,
    }
}

/// The window the editor's own frame is drawn at. Small, because WARP rasterises on the CPU, and
/// still tall enough for the bar, a row of text and the status bar.
const EDITOR_WIDTH: u32 = 240;
const EDITOR_HEIGHT: u32 = 160;

/// A quad's colour as eight bits a channel.
fn bytes_of(colour: [f32; 4]) -> [u8; 4] {
    colour.map(|channel| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a quad's channel is 0..=1, so this lands in 0..=255"
        )]
        {
            (channel * 255.0).round() as u8
        }
    })
}

/// A theme colour as eight bits a channel, which is what an opaque quad lands on.
///
/// The pipeline blends `ALPHA_BLENDING` over `Rgba8Unorm`, so an opaque quad over the target writes
/// exactly these bytes and a translucent one writes a blend of them — no colour management and no
/// rounding to allow for. Where a comparison here is [`nearer`] rather than equality, it is because
/// something *translucent* is expected over it, not because the bytes drift.
fn bytes(colour: mdedit_app::view::Rgba) -> [u8; 4] {
    [colour.r, colour.g, colour.b, colour.a].map(|channel| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a theme channel is 0..=1, so this lands in 0..=255"
        )]
        {
            (channel * 255.0).round() as u8
        }
    })
}

/// The editor's own frame, drawn through the glyph pipeline into `target` (MDEDIT-134).
///
/// **The quads and the atlas both come from the editor**, which is the whole of the point: the
/// glyphs a frame addresses were packed by `Editor`, and a test that packed its own would be
/// asserting against a texture the product never draws. Pictures are not involved — a document with
/// no image in it emits no picture quads — so this is the quad pipeline alone.
///
/// **Cleared to the theme's background, because that is where the page colour comes from.**
/// `build_frame` emits no full-page quad; `Renderer::draw` clears with `theme.background` and the
/// document is drawn over it. A test that cleared to its own black would be asserting the page
/// against a constant it owns — which is what the first version of this did, and the review of #182
/// caught.
///
/// The pipeline, shader and layout are the ones `build` already made: the driver compiles the shader
/// on the first draw at ~550 ms, and a second pipeline pays that again for nothing.
fn draw_editor_frame(
    harness: &Harness,
    target: &wgpu::Texture,
    built: &Built,
    quads: &[QuadInstance],
    atlas: &Atlas,
) {
    let (device, queue) = (&harness.device, &harness.queue);
    let (atlas_texture, atlas_sampler) = create_atlas_parts(device);

    let globals = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("globals"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    #[allow(
        clippy::cast_precision_loss,
        reason = "viewport and atlas dimensions are far below f32's exact-integer limit"
    )]
    queue.write_buffer(
        &globals,
        0,
        bytemuck::cast_slice(&[
            EDITOR_WIDTH as f32,
            EDITOR_HEIGHT as f32,
            ATLAS_SIZE as f32,
            ATLAS_SIZE as f32,
        ]),
    );
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &atlas_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        atlas.pixels(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS_SIZE),
            rows_per_image: Some(ATLAS_SIZE),
        },
        wgpu::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
    );

    let binding = make_bind_group(
        device,
        &built.bind_layout,
        &globals,
        &atlas_texture,
        &atlas_sampler,
    );
    let instances = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("editor instances"),
        size: size_of_val(quads) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&instances, 0, bytemuck::cast_slice(quads));

    let page = mdedit_app::view::Theme::default().background;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("editor frame"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("editor quads"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(page.r),
                        g: f64::from(page.g),
                        b: f64::from(page.b),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&built.quads);
        pass.set_bind_group(0, &binding, &[]);
        pass.set_vertex_buffer(0, instances.slice(..));
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a frame's quads are thousands at most"
        )]
        pass.draw(0..4, 0..quads.len() as u32);
    }
    queue.submit([encoder.finish()]);
}

/// The second half of the one test (MDEDIT-134): the editor's own quads, against its own atlas.
///
/// A function rather than a `#[test]` of its own, for the reason the module docs give — nextest
/// gives every test its own process and the driver compiles the shader on each process's first
/// draw, at ~550 ms against ~2.5 ms a frame after it.
///
/// **The document has a letter in it**, and that is the half a stub cannot check. The caret samples
/// the atlas's origin texel, which `Atlas::new` seeds before anything is packed — so a caret proves
/// the pipeline draws and nothing about packing. A glyph's quad addresses a region `Editor` packed,
/// so ink inside one is the quads and the texture agreeing about where that glyph is.
fn the_editors_own_frame_has_ink_where_it_drew(harness: &Harness, built: &Built) {
    let mut editor = Editor::new(
        "x",
        DirectWriteText::new().expect("DirectWrite is available on any supported Windows"),
    );
    #[allow(
        clippy::cast_precision_loss,
        reason = "a window a few hundred pixels across is exact in f32"
    )]
    let quads = editor.build_frame(EDITOR_WIDTH as f32, EDITOR_HEIGHT as f32, 1.0);
    let page = bytes(mdedit_app::view::Theme::default().background);
    let caret_colour = bytes(mdedit_app::view::Theme::default().caret);
    // **The caret, and not something else the same colour.** `Theme::dark` gives `tab_modified` the
    // same bytes, so the colour alone would let a modified tab's dot answer for a caret that had
    // stopped being drawn — which is the regression MDEDIT-81 is about. A caret is the solid texel
    // at the atlas origin and is taller than it is wide; a dot is neither.
    let caret = quads
        .iter()
        .find(|quad| {
            bytes_of(quad.colour) == caret_colour
                && quad.uv == [0.0, 0.0, 1.0, 1.0]
                && quad.rect[3] > quad.rect[2]
        })
        .copied()
        .expect("a blank page draws a caret (MDEDIT-81)");
    // **The document's own letter**, which is a quad addressing a region `Editor` packed rather
    // than the seeded origin texel — and inside the content rectangle, because a chrome glyph sits
    // on the bar's own fill and "not the page colour" would be true of it whether it drew or not.
    let content = editor.content_rect();
    // Its *row*, not `ContentRect::contains`: the letter's ink starts a pixel left of the column's
    // edge — a negative left side bearing, which is real and which `contains` rejects.
    let glyph = quads
        .iter()
        .find(|quad| {
            quad.uv != [0.0, 0.0, 1.0, 1.0]
                && quad.rect[1] >= content.top_edge()
                && quad.rect[1] < content.top_edge() + content.height()
        })
        .copied()
        .expect("a document with a letter in it draws a glyph");

    let target = harness.target(EDITOR_WIDTH, EDITOR_HEIGHT);
    draw_editor_frame(harness, &target, built, &quads, editor.atlas());
    let drawn = harness.read_back(&target);

    // The caret's middle, the glyph's, and a point on the page a row below both — outside the
    // caret-line tint, which spans the caret's whole row and is *not* the page colour.
    let on_the_caret = drawn.at(middle(caret.rect));
    let on_the_glyph = drawn.at(middle(glyph.rect));
    let (page_x, page_y) = middle(caret.rect);
    let below = drawn.at((page_x, page_y + row_below(caret.rect)));
    // **Beside the glyph, on its own row**, which is the page the glyph is drawn on. Comparing the
    // glyph against the *theme's* page colour instead was the first version and proved nothing: the
    // glyph's row carries the caret-line tint, so the two differ whether or not the glyph drew.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a glyph is a few pixels across, inside a window a few hundred wide"
    )]
    let beside_the_glyph = drawn.at((
        (glyph.rect[0] + glyph.rect[2]) as u32 + 2,
        middle(glyph.rect).1,
    ));

    assert!(
        nearer(on_the_caret, caret_colour, page),
        "no ink where the caret is: {on_the_caret:?}, wanted {caret_colour:?}"
    );
    assert_ne!(
        on_the_glyph, beside_the_glyph,
        "the glyph's own box is the same colour as the row beside it, so nothing was drawn from \
         the atlas the quad addresses"
    );
    assert_eq!(
        below, page,
        "the page below the caret's row is not the theme's background"
    );
}

/// The middle of a quad's rectangle, as whole pixels.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "every rect here is inside a window a few hundred pixels across"
)]
fn middle(rect: [f32; 4]) -> (u32, u32) {
    (
        (rect[0] + rect[2] / 2.0) as u32,
        (rect[1] + rect[3] / 2.0) as u32,
    )
}
#[test]
fn both_pipelines_build_and_draw_on_a_device_with_no_window() {
    let harness = Harness::new();
    let (device, queue) = (&harness.device, &harness.queue);
    let built = build(&harness);
    let Built {
        quads,
        pictures,
        atlas_binding,
        picture_binding,
        instances,
        picture_instances,
        bind_layout: _,
    } = &built;

    let target = harness.target(WIDTH, HEIGHT);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("frame"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quads"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(BACKGROUND),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // Pictures first, then the quad pass — the order `Renderer::draw` uses.
        pass.set_pipeline(pictures);
        pass.set_bind_group(0, picture_binding, &[]);
        pass.set_vertex_buffer(0, picture_instances.slice(..));
        pass.draw(0..4, 0..1);

        pass.set_pipeline(quads);
        pass.set_bind_group(0, atlas_binding, &[]);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.draw(0..4, 0..1);
    }
    queue.submit([encoder.finish()]);

    let pixels = harness.read_back(&target);

    // **Ink, not survival.** Reaching here having drawn nothing is the failure mode that let
    // MDEDIT-146 through by hand, so each pipeline's own region is checked.
    //
    // The picture quad spans x 0..32, so a quarter and three quarters across it are the centres of
    // its two texels — the points a `Linear` sampler resolves to pure colours rather than a blend.
    let picture_left = pixels.at((8, 16));
    let picture_right = pixels.at((24, 16));
    let from_glyphs = pixels.at((48, 16));

    assert!(
        nearer(picture_left, LEFT_TEXEL, BACKGROUND_RGBA),
        "the image pipeline drew nothing, or drew the wrong texture: {picture_left:?}"
    );
    assert_eq!(
        from_glyphs,
        [0xff, 0x00, 0x00, 0xff],
        "the glyph pipeline drew nothing, or lost the instance's colour"
    );

    // **The half that tells the two vertex shaders apart.** `vs_main` divides `uv` by the atlas
    // size, so under it the whole quad samples the *first* texel and both halves come back
    // `LEFT_TEXEL`. Only `vs_image`'s undivided `uv` reaches the second one. Verified by mis-wiring
    // the pipeline to `vs_main` on purpose: without this assertion the test passed.
    assert!(
        nearer(picture_right, RIGHT_TEXEL, LEFT_TEXEL),
        "the picture's right half resolved to the left texel — the image pipeline is on the glyph \
         vertex shader, or its uv mapping is wrong: {picture_right:?}"
    );

    // And the frame is not uniformly one colour, which a clear alone would satisfy.
    assert_ne!(
        picture_left, from_glyphs,
        "both regions are identical, so neither pipeline is distinguishable from the clear"
    );
    the_editors_own_frame_has_ink_where_it_drew(&harness, &built);
}

/// How far below a quad's middle the next row starts, from the quad's own height.
///
/// The caret is a row tall, so one and a half of it clears the caret-line tint that spans its row —
/// which is what the point below has to be outside of to be the page at all.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a row is tens of pixels"
)]
fn row_below(rect: [f32; 4]) -> u32 {
    (rect[3] * 1.5).round() as u32
}
