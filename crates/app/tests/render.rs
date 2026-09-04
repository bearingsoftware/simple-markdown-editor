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
//! It is **not** a screenshot test. Nothing here is golden, no hash is committed and no reference
//! image exists. MDEDIT-123 found the pixels were byte-identical across two machines and said
//! plainly that this is evidence rather than a guarantee — a Windows update that reships a font
//! would break a golden test on an unrelated pull request, with a diff explaining nothing.
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

use mdedit_app::gpu::{
    ATLAS_SIZE, Atlas, QuadInstance, SHADER, create_atlas_parts, create_bind_layout,
    create_image_parts, create_pipeline, make_bind_group,
};

/// Small: WARP is a software rasteriser and every pixel is CPU work.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;

/// What the frame is cleared to, so "ink" means "not this".
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
    fn target(&self) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
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
    fn read_back(&self, target: &wgpu::Texture) -> Vec<u8> {
        // `bytes_per_row` must be a multiple of 256 for a texture-to-buffer copy.
        let row = (WIDTH * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row * HEIGHT),
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
                    rows_per_image: Some(HEIGHT),
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
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
        let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            let start = (y * row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (WIDTH * 4) as usize]);
        }
        drop(mapped);
        buffer.unmap();
        pixels
    }
}

/// The pixel at `(x, y)`, as RGBA.
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * WIDTH + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

/// A one-texel opaque white RGBA texture, standing in for a decoded picture.
fn picture_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("picture"),
        size: wgpu::Extent3d {
            width: 1,
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
        &[0x40, 0x80, 0xc0, 0xff],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
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
    let (pictures, picture_sampler, _) = create_image_parts(device, &shader, &bind_layout, format);

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
        size: (size_of::<QuadInstance>() * 2) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &instances,
        0,
        bytemuck::cast_slice(&[picture_quad, glyph_quad]),
    );

    Built {
        quads,
        pictures,
        atlas_binding,
        picture_binding,
        instances,
    }
}

#[test]
fn both_pipelines_build_and_draw_on_a_device_with_no_window() {
    let harness = Harness::new();
    let (device, queue) = (&harness.device, &harness.queue);
    let Built {
        quads,
        pictures,
        atlas_binding,
        picture_binding,
        instances,
    } = build(&harness);

    let target = harness.target();
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
        pass.set_pipeline(&pictures);
        pass.set_bind_group(0, &picture_binding, &[]);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.draw(0..4, 0..1);

        pass.set_pipeline(&quads);
        pass.set_bind_group(0, &atlas_binding, &[]);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.draw(0..4, 1..2);
    }
    queue.submit([encoder.finish()]);

    let pixels = harness.read_back(&target);

    // **Ink, not survival.** Reaching here having drawn nothing is the failure mode that let
    // MDEDIT-146 through by hand, so each pipeline's own region is checked.
    let from_picture = pixel(&pixels, 16, 16);
    let from_glyphs = pixel(&pixels, 48, 16);

    assert_eq!(
        from_picture,
        [0x40, 0x80, 0xc0, 0xff],
        "the image pipeline drew nothing, or drew the wrong texture"
    );
    assert_eq!(
        from_glyphs,
        [0xff, 0x00, 0x00, 0xff],
        "the glyph pipeline drew nothing, or lost the instance's colour"
    );

    // And the frame is not uniformly one colour, which a clear alone would satisfy.
    assert_ne!(
        from_picture, from_glyphs,
        "both regions are identical, so neither pipeline is distinguishable from the clear"
    );
}
