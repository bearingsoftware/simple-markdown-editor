//! The window, the frame loop, and the keystroke path.
//!
//! Generic over [`TextLayout`] rather than hardcoding DirectWrite, so this whole module still
//! compiles on Linux CI and the portable backend needs no changes here when it arrives. Only
//! `main.rs` picks a backend.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mdedit_document::{Document, Selection};
use mdedit_file::LoadedFile;
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{GlyphKey, Layout, TextLayout};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::gpu::{ATLAS_SIZE, Atlas, QuadInstance, SHADER};
use crate::latency::Latency;
use crate::view::{self, Rgba, Theme};

/// Left and top margin, in device-independent pixels.
const MARGIN: f32 = 24.0;
/// Caret width, in device-independent pixels.
const CARET_WIDTH: f32 = 2.0;

/// GPU resources for one window.
struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: u64,
    atlas_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// Why the GPU could not be brought up.
///
/// A machine with no usable adapter is a real configuration — a stripped VM, a broken driver — and
/// it deserves a message rather than a panic in a graphics callback.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GpuError {
    /// No adapter could present to this window.
    #[error("no usable GPU adapter: {0}")]
    NoAdapter(String),
    /// An adapter exists but would not give us a device.
    #[error("GPU adapter refused a device: {0}")]
    NoDevice(String),
    /// The window could not back a drawing surface.
    #[error("could not create a surface for the window: {0}")]
    NoSurface(String),
}

impl Renderer {
    fn new(window: &Arc<Window>) -> Result<Self, GpuError> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(Arc::clone(window))
            .map_err(|e| GpuError::NoSurface(e.to_string()))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoAdapter(e.to_string()))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mdedit"),
            required_features: wgpu::Features::empty(),
            // The adapter's own limits, not `downlevel_defaults()`: that caps textures at 2048,
            // which is smaller than a maximised window on this machine and rejects the surface
            // outright. It is also exactly the atlas size, leaving no headroom.
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoDevice(e.to_string()))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            // Fifo is the only mode guaranteed present everywhere. It also caps latency at the
            // refresh interval, which the harness has to account for — see `latency.rs`.
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quads"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Coverage, one byte per texel. The colour comes from the instance.
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            // Nearest: glyphs are rasterized at the exact device scale, so any filtering would only
            // blur text that DirectWrite already hinted.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bind_layout = create_bind_layout(&device);

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let instance_capacity = 4096;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quads"),
            size: instance_capacity * size_of::<QuadInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = make_bind_group(&device, &bind_layout, &globals, &atlas_texture, &sampler);

        let pipeline = create_pipeline(&device, &shader, &bind_layout, format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            globals,
            instances,
            instance_capacity,
            atlas_texture,
            bind_group,
            bind_layout,
            sampler,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn upload_atlas(&self, atlas: &Atlas) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas_texture,
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
    }

    fn ensure_capacity(&mut self, needed: u64) {
        if needed <= self.instance_capacity {
            return;
        }
        self.instance_capacity = needed.next_power_of_two();
        self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quads"),
            size: self.instance_capacity * size_of::<QuadInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bind_group = make_bind_group(
            &self.device,
            &self.bind_layout,
            &self.globals,
            &self.atlas_texture,
            &self.sampler,
        );
    }

    /// Submits one frame and presents it.
    fn draw(&mut self, quads: &[QuadInstance], background: Rgba) {
        // A surface is transiently unavailable while a window is resized, occluded or minimised.
        // Skipping the frame is the documented response to each; the next redraw presents.
        // `Suboptimal` still carries a usable texture, so it draws rather than dropping a frame the
        // user would see as a stutter.
        let (wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame)) = self.surface.get_current_texture()
        else {
            return;
        };

        self.ensure_capacity(quads.len() as u64);
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::cast_slice(&[
                px(self.config.width),
                px(self.config.height),
                px(ATLAS_SIZE),
                px(ATLAS_SIZE),
            ]),
        );
        self.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(quads));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(background.r),
                            g: f64::from(background.g),
                            b: f64::from(background.b),
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
            pass.set_vertex_buffer(0, self.instances.slice(..));
            let count = u32::try_from(quads.len()).unwrap_or(u32::MAX);
            pass.draw(0..4, 0..count);
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
    }
}

/// A window or atlas dimension in pixels, as a float.
///
/// Both are bounded by the largest texture a GPU will allocate — tens of thousands at most — which
/// `f32` represents exactly. The cast cannot lose anything here, which is why it is confined to one
/// place that says so rather than repeated at every call site.
#[allow(
    clippy::cast_precision_loss,
    reason = "pixel dimensions are far below f32's 2^24 exact-integer limit"
)]
fn px(value: u32) -> f32 {
    value as f32
}

/// A glyph bitmap offset in pixels, as a float. Bounded for the same reason as [`px`].
#[allow(
    clippy::cast_precision_loss,
    reason = "glyph offsets are bounded by the atlas size"
)]
fn px_i(value: i32) -> f32 {
    value as f32
}

fn create_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            },
        ],
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quads"),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quads"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<QuadInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x4,
                    1 => Float32x4,
                    2 => Float32x4,
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
    })
}

fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    atlas: &wgpu::Texture,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// The editor: a document, a caret, a text engine, and a window to put them in.
pub struct Editor<T: TextLayout> {
    document: Document,
    theme: Theme,
    text: T,
    atlas: Atlas,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    latency: Latency,
    /// When the keystroke currently in flight was received, if any. Cleared once presented.
    pending_keystroke: Option<Instant>,
    /// Set if the GPU could not be brought up, for `main` to report on exit.
    gpu_error: Option<GpuError>,
    /// The file being edited, if this is not the built-in sample.
    file: Option<OpenFile>,
    /// Held so Ctrl+S can be told apart from a plain `s`.
    modifiers: ModifiersState,
}

/// A file on disk, and what it looked like when it was read.
#[derive(Debug)]
struct OpenFile {
    path: PathBuf,
    loaded: LoadedFile,
}

impl<T: TextLayout> std::fmt::Debug for Editor<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The text backend and the GPU resources have no useful representation, and requiring
        // `T: Debug` would put a bound on the trait for the sake of a derive.
        f.debug_struct("Editor")
            .field("bytes", &self.document.len_bytes())
            .field("caret", &self.document.selection().head())
            .field("glyphs_packed", &self.atlas.len())
            .field("latency_samples", &self.latency.len())
            .finish_non_exhaustive()
    }
}

impl<T: TextLayout> Editor<T> {
    /// An editor over `source`, drawn with `text`.
    pub fn new(source: &str, text: T) -> Self {
        Self {
            document: Document::from(source),
            theme: Theme::default(),
            text,
            atlas: Atlas::new(),
            window: None,
            renderer: None,
            latency: Latency::new(),
            pending_keystroke: None,
            gpu_error: None,
            file: None,
            modifiers: ModifiersState::empty(),
        }
    }

    /// The latency samples gathered so far.
    #[must_use]
    pub fn latency(&self) -> &Latency {
        &self.latency
    }

    /// Why the editor could not start, if it could not.
    #[must_use]
    pub fn gpu_error(&self) -> Option<&GpuError> {
        self.gpu_error.as_ref()
    }

    /// An editor over a file read from disk.
    pub fn open(path: PathBuf, loaded: LoadedFile, text: T) -> Self {
        let mut editor = Self::new(loaded.text(), text);
        editor.file = Some(OpenFile { path, loaded });
        editor
    }

    /// Whether the buffer differs from what is on disk.
    ///
    /// Compared by content rather than tracked by a flag: typing a character and deleting it again
    /// leaves a document that does not need saving, and a flag would say otherwise.
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.file
            .as_ref()
            .is_some_and(|file| self.document.to_string() != file.loaded.text())
    }

    /// Writes the buffer back to the file it came from.
    ///
    /// Reloads afterwards so the in-memory original matches what is now on disk — otherwise the
    /// next save would compare against stale bytes and take the wrong path.
    fn save(&mut self) {
        let Some(file) = self.file.as_mut() else {
            // The built-in sample has nowhere to go. Not an error; there is just no file.
            return;
        };

        let text = self.document.to_string();
        match mdedit_file::save(&file.path, &text, &file.loaded) {
            Ok(()) => match mdedit_file::reload(&file.path, &file.loaded) {
                Ok(reloaded) => file.loaded = reloaded,
                Err(error) => {
                    // The bytes are on disk; only our copy of them is stale. Worth saying so,
                    // because the next save will fall back to re-encoding.
                    eprintln!(
                        "saved, but could not re-read {}: {error}",
                        file.path.display()
                    );
                }
            },
            Err(error) => eprintln!("could not save {}: {error}", file.path.display()),
        }
    }

    /// The window title: the file name, marked when there are unsaved changes.
    fn title(&self) -> String {
        let Some(file) = self.file.as_ref() else {
            return "Simple Markdown Editor".to_owned();
        };
        let name = file.path.file_name().map_or_else(
            || file.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        if self.is_modified() {
            format!("{name} • Simple Markdown Editor")
        } else {
            format!("{name} — Simple Markdown Editor")
        }
    }

    /// Applies one key press to the document.
    ///
    /// Returns whether anything changed and a redraw is needed.
    fn handle_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }

        // Chords first, so a modified key never inserts its own letter.
        if self.modifiers.control_key() {
            let Key::Character(character) = &event.logical_key else {
                return false;
            };
            return match character.as_str() {
                "s" | "S" => {
                    self.save();
                    false
                }
                // Shift+Ctrl+Z is the other conventional redo, and costs one arm to support.
                "z" if !self.modifiers.shift_key() => self.document.undo(),
                "z" | "Z" | "y" | "Y" => self.document.redo(),
                _ => false,
            };
        }

        let caret = self.document.selection().head();

        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                if caret == ByteOffset::ZERO {
                    return false;
                }
                let previous = self.previous_boundary(caret);
                if self
                    .document
                    .delete(ByteSpan::new(previous, caret))
                    .is_err()
                {
                    return false;
                }
                // `delete` already leaves the caret at the start of what it removed.
                true
            }
            Key::Named(NamedKey::Delete) => {
                let next = self.next_boundary(caret);
                if next == caret || self.document.delete(ByteSpan::new(caret, next)).is_err() {
                    return false;
                }
                true
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.document
                    .set_selection(Selection::caret(self.previous_boundary(caret)));
                true
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.document
                    .set_selection(Selection::caret(self.next_boundary(caret)));
                true
            }
            Key::Named(NamedKey::Home) => {
                self.document
                    .set_selection(Selection::caret(ByteOffset::ZERO));
                true
            }
            Key::Named(NamedKey::End) => {
                let end = ByteOffset::new(self.document.len_bytes());
                self.document.set_selection(Selection::caret(end));
                true
            }
            Key::Named(NamedKey::Enter) => self.insert("\n"),
            Key::Named(NamedKey::Space) => self.insert(" "),
            Key::Character(text) => self.insert(text),
            _ => false,
        }
    }

    fn insert(&mut self, text: &str) -> bool {
        let caret = self.document.selection().head();
        if self.document.insert(caret, text).is_err() {
            return false;
        }
        // `insert` leaves the caret after what it inserted.
        true
    }

    /// The character boundary before `offset`, or the start of the buffer.
    fn previous_boundary(&self, offset: ByteOffset) -> ByteOffset {
        let text = self.document.to_string();
        let mut index = offset.get().min(text.len());
        loop {
            if index == 0 {
                return ByteOffset::ZERO;
            }
            index -= 1;
            if text.is_char_boundary(index) {
                return ByteOffset::new(index);
            }
        }
    }

    /// The character boundary after `offset`, or the end of the buffer.
    fn next_boundary(&self, offset: ByteOffset) -> ByteOffset {
        let text = self.document.to_string();
        let mut index = offset.get();
        loop {
            if index >= text.len() {
                return ByteOffset::new(text.len());
            }
            index += 1;
            if text.is_char_boundary(index) {
                return ByteOffset::new(index);
            }
        }
    }

    /// Lays the document out and turns it into quads.
    fn build_frame(&mut self, width: f32, scale: f32) -> Vec<QuadInstance> {
        let runs = view::build_runs(&self.document, self.document.selection(), &self.theme);
        let styled = view::as_styled(&runs);

        let available = (width / scale - MARGIN * 2.0).max(1.0);
        let Ok(layout) = self.text.layout(&styled, available) else {
            return Vec::new();
        };

        let mut quads = Vec::with_capacity(layout.lines.iter().map(|l| l.glyphs.len()).sum());
        for glyph in layout.glyphs() {
            let Some(entry) = self.entry_for(glyph.key, scale) else {
                continue;
            };
            let (pen_x, pen_y) = (
                (glyph.pos.x + MARGIN) * scale,
                (glyph.pos.y + MARGIN) * scale,
            );
            let colour = view::colour_at(&runs, glyph.source.start(), self.theme.text);
            quads.push(QuadInstance {
                rect: [
                    pen_x + px_i(entry.left),
                    pen_y + px_i(entry.top),
                    px(entry.width),
                    px(entry.height),
                ],
                uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                colour: [colour.r, colour.g, colour.b, colour.a],
            });
        }

        if let Some(caret) = self.caret_quad(&layout, scale) {
            quads.push(caret);
        }
        quads
    }

    /// Packs a glyph if it is not already in the atlas.
    fn entry_for(&mut self, key: GlyphKey, scale: f32) -> Option<crate::gpu::AtlasEntry> {
        if let Some(entry) = self.atlas.get(key) {
            return Some(entry);
        }
        let image = self.text.rasterize(key, scale).ok()??;
        self.atlas.insert(key, &image).ok()
    }

    /// A quad for the caret, placed at the glyph the caret addresses.
    fn caret_quad(&self, layout: &Layout, scale: f32) -> Option<QuadInstance> {
        let caret = self.document.selection().head();
        let mut best: Option<(&mdedit_text::PositionedGlyph, f32)> = None;
        let mut last: Option<&mdedit_text::PositionedGlyph> = None;

        for line in &layout.lines {
            for glyph in &line.glyphs {
                last = Some(glyph);
                if glyph.source.start() >= caret {
                    // The first glyph at or after the caret is where it sits.
                    best = Some((glyph, glyph.pos.x));
                    break;
                }
            }
            if best.is_some() {
                break;
            }
        }

        // Past the last glyph — a caret at the end of the document — sits after it.
        let (glyph, x) = best.or_else(|| last.map(|g| (g, g.pos.x + g.advance)))?;
        let height = layout
            .lines
            .iter()
            .find(|line| line.glyphs.iter().any(|g| std::ptr::eq(g, glyph)))
            .map_or(self.theme.body_size * 1.4, |line| line.height.max(1.0));

        Some(QuadInstance {
            rect: [
                (x + MARGIN) * scale,
                (glyph.pos.y + MARGIN) * scale - height * scale * 0.8,
                CARET_WIDTH * scale,
                height * scale,
            ],
            // The opaque texel at the atlas origin: a solid fill is a glyph quad that samples it.
            uv: [0.0, 0.0, 1.0, 1.0],
            colour: [
                self.theme.caret.r,
                self.theme.caret.g,
                self.theme.caret.b,
                self.theme.caret.a,
            ],
        })
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };

        // Cheap, and the only place the modified marker can appear without a status bar to put it in.
        let title = self.title();
        window.set_title(&title);
        // winit reports the scale factor as f64; every downstream use is f32 pixel maths.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "scale factors are small ratios"
        )]
        let scale = window.scale_factor() as f32;
        let size = window.inner_size();

        let quads = self.build_frame(px(size.width), scale);

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if self.atlas.is_dirty() {
            renderer.upload_atlas(&self.atlas);
            self.atlas.mark_clean();
        }
        renderer.draw(&quads, self.theme.background);

        // Sampled after `present` returns, which is the last moment this process controls. See
        // `latency.rs` for exactly what that does and does not include.
        if let Some(started) = self.pending_keystroke.take() {
            self.latency.record(started.elapsed());
        }
    }
}

impl<T: TextLayout> ApplicationHandler for Editor<T> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Simple Markdown Editor")
            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 700.0));
        let Ok(window) = event_loop.create_window(attributes) else {
            event_loop.exit();
            return;
        };
        let window = Arc::new(window);
        match Renderer::new(&window) {
            Ok(renderer) => {
                self.renderer = Some(renderer);
                self.window = Some(window);
            }
            Err(err) => {
                // Nothing to draw with. Record it for `main` to report and stop, rather than
                // running an event loop that can only ever paint nothing.
                self.gpu_error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                // Glyphs are rasterized per scale factor, so the atlas is stale. Dropping it is the
                // correct response to WM_DPICHANGED and cheap — it refills on the next frame.
                self.atlas = Atlas::new();
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                // Stamped before any work, so the measurement includes our own handling rather
                // than starting after it.
                let started = Instant::now();
                if self.handle_key(&event) {
                    self.pending_keystroke = Some(started);
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
}
