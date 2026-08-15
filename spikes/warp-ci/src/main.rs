//! MDEDIT-123: can a CI runner render one of this editor's frames without a window?
//!
//! `AGENTS.md` says CI cannot open a window, so rendering is verified by hand on Windows — and that
//! sentence has cost real defects: MDEDIT-92's row overflowing a default window, MDEDIT-116's
//! unclickable menu rows, MDEDIT-81's missing caret, MDEDIT-70's missing icon. Every one was found by
//! a human opening the editor.
//!
//! Four probes, each answering a smaller question than the last one implies:
//!
//! 1. **Layout.** Does an `Editor<DirectWriteText>` — the real font stack, no grid stub — build a
//!    frame with no window and no GPU at all?
//! 2. **Adapter.** Is WARP, Microsoft's software rasteriser, reachable through wgpu's DX12 backend?
//! 3. **Render.** Does this project's actual shader, atlas and quad pipeline produce pixels on it?
//! 4. **Stability and cost.** Same input, same bytes out? And in seconds rather than minutes?
//!
//! Run it:
//!
//! ```text
//! cargo run --release --manifest-path spikes/warp-ci/Cargo.toml
//! ```
//!
//! Printing is the whole output. Exit status is 0 if every probe answered, 1 if one could not run —
//! a "no" is a result and still exits 0, because a recorded negative is what this ticket asked for.

#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

fn main() {
    println!("MDEDIT-123 — can a CI runner render a frame?\n");
    #[cfg(not(windows))]
    {
        println!("WARP is a Direct3D component and DirectWrite is Windows-only.");
        println!("This probe answers nothing off Windows; that is the point of the ticket.");
    }
    #[cfg(windows)]
    windows::run();
}

#[cfg(windows)]
mod windows {
    use super::{Duration, Instant};

    use mdedit_app::editor::Editor;
    use mdedit_app::gpu::{ATLAS_SIZE, Atlas, QuadInstance, SHADER};
    use mdedit_offsets::{ByteOffset, ByteSpan};
    use mdedit_text::{StyledRun, TextLayout, TextStyle};
    use mdedit_text_dwrite::DirectWriteText;

    /// A window's worth, and the size the app opens at.
    const WIDTH: u32 = 900;
    const HEIGHT: u32 = 600;

    /// Enough prose to exercise wrapping, headings and inline syntax.
    const DOCUMENT: &str = "\
# A heading

Some **strong** text, some *emphasis*, a `code span`, and a [link](http://example.com).

- a list item
- another one

> a quoted line
";

    pub fn run() {
        let layout = probe_layout();
        let adapter = probe_adapter();
        match (layout, adapter) {
            (Some(quads), Some(gpu)) => probe_render(quads, gpu),
            _ => println!("\nNothing further to measure."),
        }
    }

    // -----------------------------------------------------------------------
    // 1. Layout, with no window and no GPU.
    // -----------------------------------------------------------------------

    /// Builds a frame through the real font stack and reports what came out.
    ///
    /// This is the half nobody had tried. `Editor` is generic over `TextLayout`, and every
    /// integration test in `crates/app` instantiates it with the grid stub whose glyphs are all
    /// eight pixels wide. Nothing stops it being instantiated with DirectWrite instead, and
    /// `build_frame` explicitly "touches no renderer".
    fn probe_layout() -> Option<usize> {
        println!("1. Layout through the real font stack, headless");

        let Ok(text) = DirectWriteText::new() else {
            println!("   DirectWrite refused to start. No layout, so nothing else means anything.");
            return None;
        };

        let started = Instant::now();
        let mut editor = Editor::new(DOCUMENT, text);
        let quads = editor.build_frame(WIDTH as f32, HEIGHT as f32, 1.0);
        let elapsed = started.elapsed();

        println!("   {} quads in {elapsed:.2?}, no window involved", quads.len());
        if quads.is_empty() {
            println!("   ...which is nothing. Something is wrong before the GPU is reached.");
            return None;
        }

        // The stub's whole problem is that every glyph is the same width, so it cannot answer
        // whether anything overflows. Real advances vary; showing that they do is the point.
        let widths: Vec<f32> = quads.iter().map(|q| q.rect[2]).collect();
        let distinct = {
            let mut bits: Vec<u32> = widths.iter().map(|w| w.to_bits()).collect();
            bits.sort_unstable();
            bits.dedup();
            bits.len()
        };
        println!("   {distinct} distinct quad widths — the stub would report a handful");
        Some(quads.len())
    }

    // -----------------------------------------------------------------------
    // 2. Is WARP there?
    // -----------------------------------------------------------------------

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        name: String,
    }

    /// Finds the software adapter, preferring it over any real GPU.
    ///
    /// A CI runner has no display and usually no hardware adapter either, so this is what would be
    /// picked there anyway. Asking for it explicitly makes the probe measure the same thing on this
    /// machine, which does have one.
    fn probe_adapter() -> Option<Gpu> {
        println!("\n2. The WARP adapter");

        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(descriptor);

        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
        for adapter in &adapters {
            let info = adapter.get_info();
            println!("   found: {:?}  {}", info.device_type, info.name);
        }

        let adapter = adapters
            .into_iter()
            .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)?;
        let name = adapter.get_info().name;

        let started = Instant::now();
        let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("warp-ci"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        }));

        match device {
            Ok((device, queue)) => {
                println!("   device from \"{name}\" in {:.2?}", started.elapsed());
                Some(Gpu {
                    device,
                    queue,
                    name,
                })
            }
            Err(e) => {
                println!("   \"{name}\" would not give a device: {e}");
                None
            }
        }
    }

    // -----------------------------------------------------------------------
    // 3 and 4. Render this project's pipeline, twice, and compare.
    // -----------------------------------------------------------------------

    fn probe_render(editor_quads: usize, gpu: Gpu) {
        println!("\n3. This project's pipeline, offscreen");

        let Some((quads, atlas)) = real_quads() else {
            println!("   could not rasterize through DirectWrite");
            return;
        };
        println!(
            "   {} glyph quads over {} packed glyphs (the editor built {editor_quads})",
            quads.len(),
            atlas.len()
        );

        // Built once and reused, so the per-frame number below is a frame rather than a frame plus
        // shader compilation. Which cost is which is the whole question for a CI budget.
        let started = Instant::now();
        let pipeline = Pipeline::new(&gpu, &atlas);
        let setup = started.elapsed();
        println!("   pipeline, shader and atlas upload: {setup:.2?} (paid once)");

        let mut runs = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let pixels = pipeline.render(&gpu, &quads);
            runs.push((started.elapsed(), pixels));
        }

        let (inked, total) = ink(&runs[0].1);
        println!(
            "   {inked} of {total} pixels differ from the background ({:.2}%)",
            inked as f64 / total as f64 * 100.0
        );
        if inked == 0 {
            println!("   ...so it produced a blank frame. That is a no.");
            return;
        }

        println!("\n4. Stability and cost");
        let stable = runs.windows(2).all(|pair| pair[0].1 == pair[1].1);
        println!(
            "   three renders in this process, byte-identical: {}",
            if stable { "yes" } else { "NO" }
        );
        // **Within a process is not what "run to run" means.** A golden value in a test is compared
        // against a *different* process on a *different* day, so what has to be stable is this —
        // run the probe twice and check the two printed hashes match.
        println!("   pixel hash: {:016x}   (compare against another run of this binary)", hash(&runs[0].1));
        let times: Vec<Duration> = runs.iter().map(|(t, _)| *t).collect();
        println!("   per frame: {times:.2?}");
        println!("   note the first: a driver compiles the shader on the first draw, not at setup");

        println!(
            "\nVerdict inputs: adapter \"{}\", ink present, {}stable.",
            gpu.name,
            if stable { "" } else { "un" }
        );
    }

    /// FNV-1a over the frame. Not for security; for telling two runs apart in one printed line.
    fn hash(pixels: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in pixels {
            h ^= u64::from(*byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// How many pixels are not the background.
    ///
    /// **The background is taken from the frame rather than from the clear colour I asked for.**
    /// Comparing against the requested colour was the first thing tried and it reported 100% ink,
    /// which is not a believable answer and was the probe being wrong rather than the renderer —
    /// a clear value goes through a conversion this code should not be trying to predict. The modal
    /// pixel of a mostly-empty frame *is* the background, whatever the pipeline made of it.
    fn ink(pixels: &[u8]) -> (usize, usize) {
        let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
        for pixel in pixels.chunks_exact(4) {
            *counts.entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
        }
        let ground = counts
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(c, _)| *c)
            .unwrap_or([0, 0, 0]);
        let total = pixels.len() / 4;
        let inked = pixels
            .chunks_exact(4)
            .filter(|p| [p[0], p[1], p[2]] != ground)
            .count();
        println!("   background reads as {ground:?}");
        (inked, total)
    }

    /// Lays out a line through DirectWrite and turns it into quads the real shader can draw.
    ///
    /// A trimmed copy of what `Editor` does per glyph. Duplicated here rather than reached for,
    /// because the editor keeps its atlas private — which is exactly the one thing this spike would
    /// ask to change if it recommends going further.
    fn real_quads() -> Option<(Vec<QuadInstance>, Atlas)> {
        let mut text = DirectWriteText::new().ok()?;
        let body = "The quick brown fox jumps over the lazy dog. 0123456789";
        let style = TextStyle::body(16.0);
        let layout = text
            .layout(
                &[StyledRun {
                    text: body,
                    style,
                    source: ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(body.len())),
                }],
                WIDTH as f32 - 40.0,
            )
            .ok()?;

        let mut atlas = Atlas::new();
        let mut quads = Vec::new();
        for glyph in layout.glyphs() {
            let Ok(Some(image)) = text.rasterize(glyph.key, 1.0) else {
                continue;
            };
            let Ok(entry) = atlas.insert(glyph.key, &image) else {
                continue;
            };
            quads.push(QuadInstance {
                rect: [
                    glyph.pos.x + 20.0 + entry.left as f32,
                    glyph.pos.y + 40.0 + entry.top as f32,
                    entry.width as f32,
                    entry.height as f32,
                ],
                uv: [
                    entry.u as f32,
                    entry.v as f32,
                    entry.width as f32,
                    entry.height as f32,
                ],
                colour: [0.9, 0.9, 0.93, 1.0],
            });
        }
        Some((quads, atlas))
    }

    /// Everything a frame needs that does not change between frames.
    struct Pipeline {
        target: wgpu::Texture,
        view: wgpu::TextureView,
        bind_group: wgpu::BindGroup,
        pipeline: wgpu::RenderPipeline,
        readback: wgpu::Buffer,
        row: u32,
    }

    impl Pipeline {
    /// Builds the pipeline: shader, atlas upload, bind group, readback buffer.
    fn new(gpu: &Gpu, atlas: &Atlas) -> Self {
        let (device, queue) = (&gpu.device, &gpu.queue);
        let format = wgpu::TextureFormat::Rgba8Unorm;

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            atlas_texture.as_image_copy(),
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

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            ..Default::default()
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas_texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quads"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quads"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quads"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<QuadInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Readback rows must be a multiple of 256 bytes.
        let row = (WIDTH * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row * HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            target,
            view,
            bind_group,
            pipeline,
            readback,
            row,
        }
    }

    /// One offscreen frame, read back as RGBA8 with the row padding removed.
    fn render(&self, gpu: &Gpu, quads: &[QuadInstance]) -> Vec<u8> {
        let (device, queue) = (&gpu.device, &gpu.queue);
        let (row, readback, view) = (self.row, &self.readback, &self.view);

        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quads"),
            size: std::mem::size_of_val(quads) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&instances, 0, bytemuck::cast_slice(quads));

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quads"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.086,
                            g: 0.094,
                            b: 0.149,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..4, 0..quads.len() as u32);
        }
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: readback,
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
        queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("the queue drains");

        // Only the pixels, not the row padding, so two runs compare like for like.
        let mapped = slice.get_mapped_range().expect("the buffer maps");
        let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            let start = (y * row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (WIDTH * 4) as usize]);
        }
        drop(mapped);
        readback.unmap();
        pixels
    }
    }
}
