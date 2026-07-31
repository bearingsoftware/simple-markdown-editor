//! The window, the frame loop, and the keystroke path.
//!
//! Generic over [`TextLayout`] rather than hardcoding DirectWrite, so this whole module still
//! compiles on Linux CI and the portable backend needs no changes here when it arrives. Only
//! `main.rs` picks a backend.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use mdedit_document::{Document, Selection};
use mdedit_markers::BlockKind;
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_session::Workspace;
use mdedit_text::{GlyphKey, Layout, StyledRun, TextLayout, TextStyle};
use winit::application::ApplicationHandler;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId};

use crate::blocks;
use crate::clipboard::{Clipboard, System};
use crate::config::Config;
use crate::content::{ContentRect, Insets};
use crate::gpu::{ATLAS_SIZE, Atlas, QuadInstance, SHADER};
use crate::hit;
use crate::ime::{Action, Composition};
use crate::latency::Latency;
use crate::pane;
use crate::prompt::{KeepEverything, Unsaved, UnsavedPrompt};
use crate::scroll::{self, Scroll};
use crate::status;
use crate::tabs;

/// Where the list of open files is shown.
///
/// One or the other, never both: they are two presentations of the same list, and showing both would
/// be two places to click for one action and two pieces of chrome taking room from the document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Chrome {
    /// A strip of tabs across the top. The default, and what MDEDIT-15 built.
    #[default]
    Strip,
    /// A vertical list down the left.
    Pane,
}

/// The status bar's right-hand fields.
const RIGHT: usize = 0;
/// The status bar's breadcrumb.
const TRAIL: usize = 1;

/// A short string of chrome, laid out and kept until it changes.
#[derive(Debug, Clone)]
struct Label {
    text: String,
    glyphs: Vec<mdedit_text::PositionedGlyph>,
    width: f32,
}
use crate::view::{self, Rgba, Theme};
use mdedit_menu_win::{Command, MenuBar};

/// Asks the user where to save something. Given the current name as a suggestion; `None` if they
/// decline, or if there is nobody to ask.
pub type SavePathPicker = Box<dyn Fn(Option<&str>) -> Option<PathBuf>>;

/// Left and top margin, in device-independent pixels.
const MARGIN: f32 = 24.0;
/// Fewest digits the gutter is sized for, so a short file does not visibly resize its gutter the
/// moment it grows past nine lines.
const GUTTER_MIN_DIGITS: usize = 2;
/// Space either side of the numbers in the gutter.
const GUTTER_PADDING: f32 = 8.0;
/// Caret width, in device-independent pixels.
const CARET_WIDTH: f32 = 2.0;
/// Thickness of the line under text an input method is still composing.
const UNDERLINE_WIDTH: f32 = 1.0;
/// How much width a selected line break shows as, so a selection spanning lines reads as one shape
/// rather than as a ragged stack of separate highlights.
const EMPTY_ROW_SELECTION: f32 = 6.0;
/// What a line of text is assumed to cost before it has been measured, as a multiple of its font
/// size. Only ever applied to chunks that are off screen.
const ESTIMATED_LINE_RATIO: f32 = 1.4;
/// How long the typing has to stop before the whole document is re-parsed. Long enough not to fire
/// between keystrokes at any plausible typing speed, short enough that nobody notices the wait.
const RECONCILE_AFTER: std::time::Duration = std::time::Duration::from_millis(150);
/// How many laid-out chunks to keep. At a few hundred glyphs each this is tens of megabytes at the
/// cap, and it is only reached by scrolling through a very large document.
const MAX_CACHED_LAYOUTS: usize = 512;

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

        // `Instance::default()` enables every backend wgpu was built with, and each one initialises
        // its own driver stack — Vulkan and OpenGL get loaded and paid for even though nothing here
        // will ever use them. Measured on an empty window with none of this project's code in it:
        // all backends 346 MB, DX12 alone 124 MB. See MDEDIT-19 and `spikes/gpu-baseline`.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            // Nothing runs off Windows yet, but pinning DX12 there would mean no adapter at all
            // rather than an honest failure.
            wgpu::Backends::PRIMARY
        };
        let instance = wgpu::Instance::new(descriptor);

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
    /// Every open tab. The active one supplies the buffer being edited.
    workspace: Workspace,
    /// Used only when no file is open, for the built-in sample.
    scratch: Document,
    theme: Theme,
    text: T,
    atlas: Atlas,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    latency: Latency,
    /// When the keystroke currently in flight was received, if any. Cleared once presented.
    pending_keystroke: Option<Instant>,
    /// Keystroke to quads-ready, which is the part of a frame this process spends CPU on.
    build: Latency,
    /// When the first frame was presented, for the file-open target.
    first_paint: Option<Instant>,
    /// Set if the GPU could not be brought up, for `main` to report on exit.
    gpu_error: Option<GpuError>,
    /// Held so Ctrl+S can be told apart from a plain `s`.
    modifiers: ModifiersState,
    /// Where each tab landed in the strip last frame, for resolving a click.
    tab_rects: Vec<tabs::TabRect>,
    /// Where the open files are shown. See [`Chrome`].
    chrome: Chrome,
    /// The pane's visible rows, kept for the next click, as `tab_rects` is for the strip.
    row_rects: Vec<pane::RowRect>,
    /// How far the pane's list has been scrolled. Its own, not the document's — the two scroll
    /// independently, which is the point of a pane that can hold more files than fit.
    pane_scroll: f32,
    /// Laid-out row labels, keyed by the text.
    ///
    /// The pane redraws every frame and a forty-file pane shows sixteen rows, so laying each name
    /// out per frame put thirty-odd text-engine calls on the keystroke path — measured, and the same
    /// trap the gutter and the status bar each hit. A file name changes when a tab is renamed or
    /// saved somewhere new, which is rare enough that keying on the text is the whole cache
    /// invalidation this needs. Bounded by the session's tab limit.
    row_labels: HashMap<String, Label>,
    /// Last known cursor position, in device-independent pixels.
    cursor: (f32, f32),
    /// Where the viewport sits over the document.
    scroll: Scroll,
    /// What the last frame measured, so the wheel and the page keys have something to clamp to.
    viewport: Viewport,
    /// Set by a key press that should bring the caret back into view on the next frame.
    ///
    /// Not every frame: scrolling with the wheel would snap straight back to the caret if the
    /// document followed it unconditionally.
    follow_caret: bool,
    /// What each chunk measured, by content. Kept for the whole session: it is one float per
    /// distinct chunk, and it is what stops the document's length jumping about as it is scrolled.
    heights: HashMap<ChunkKey, f32>,
    /// Laid-out chunks, by content. Capped — see [`MAX_CACHED_LAYOUTS`].
    layouts: HashMap<ChunkKey, Rc<Cached>>,
    /// What the top of the viewport is resting on, so measuring a chunk does not move the page.
    anchor: Option<Anchor>,
    /// What an input method is composing, which is deliberately not in the buffer. See [`crate::ime`].
    composition: Composition,
    /// Where the caret was drawn last frame, in device-independent pixels from the window's corner.
    ///
    /// Kept so the candidate window can be put next to it. An input method that is not told where
    /// the caret is puts its candidate list at the window's origin, which is step 2 of the script.
    caret_area: Option<(f32, f32, f32)>,
    /// What the last frame measured. Clicking and vertical motion both ask where the lines went,
    /// and this is the only thing that knows.
    last_frame: Option<Frame>,
    /// The column Up and Down are aiming for, in device-independent pixels.
    ///
    /// Remembered across a run of them so that passing through a short line does not drag the caret
    /// to its end and leave it there. Any horizontal movement or edit clears it.
    desired_x: Option<f32>,
    /// Whether the pointer is dragging out a selection.
    dragging: bool,
    /// Asks about unsaved work when a tab is closing. See [`crate::prompt`].
    prompt: Box<dyn UnsavedPrompt>,
    /// Asks where to save something that has nowhere to go. Declines by default.
    pick_save_path: SavePathPicker,
    /// Concealed, or shown as it is. See [`crate::view::Mode`].
    mode: view::Mode,
    /// What the user chose. Read once at startup; see [`crate::config`].
    config: Config,
    /// The ten digits and their advance, laid out once. See [`Self::digits`].
    digits: Option<[(GlyphKey, f32); 10]>,
    /// The last word count, and `None` until one has finished.
    ///
    /// Counted on the idle timer rather than per frame — see [`crate::status`] — so this is what the
    /// status bar shows, deliberately a moment behind while typing.
    words: Option<usize>,
    /// The status bar's two strings, laid out.
    ///
    /// Chrome is redrawn every frame and the text engine is the expensive part of drawing it, so
    /// each side is laid out only when what it says changes. The breadcrumb changes when the caret
    /// crosses a heading; the right-hand fields change when it moves at all — which is once per
    /// keystroke rather than once per frame, and that is the difference worth having.
    status_labels: [Option<Label>; 2],
    /// Which tab [`Self::words`] was counted from.
    ///
    /// Tracked rather than cleared at each of the nine places that activate a tab, because a count
    /// that belongs to the tab you just left is not stale, it is wrong — and one forgotten call site
    /// is a bug nobody would see until they switched files.
    counted_for: Option<usize>,
    /// The native menu bar, once there is a window to hang it on.
    menu: Option<MenuBar>,
    /// How menu commands get from muda's channel into this loop. Set before the window exists.
    menu_proxy: Option<EventLoopProxy<Command>>,
    /// Where text goes when it is copied, and comes from when it is pasted.
    ///
    /// Boxed rather than a second type parameter: `Editor` is already generic over the text
    /// backend, and threading a second one through every signature would cost more than it buys.
    clipboard: Box<dyn Clipboard>,
    /// Where the document was allowed to draw last frame.
    ///
    /// Kept because a click arrives between frames and has to be mapped back through exactly the
    /// rectangle the glyphs were placed with. See [`crate::content`].
    content: ContentRect,
}

/// What the last laid-out frame measured, in device-independent pixels.
///
/// Scroll limits depend on the layout, and the layout only exists during a frame — so a wheel event
/// arriving between frames clamps against these. Zero before the first paint, which means a wheel
/// event that beats the first frame does nothing rather than guessing.
#[derive(Debug, Clone, Copy, Default)]
struct Viewport {
    /// Height of the whole laid-out document.
    content: f32,
    /// Height of the area the document is drawn into, below the tab strip.
    height: f32,
    /// A representative line height, for the wheel notch and the page keys.
    line: f32,
}

impl<T: TextLayout> std::fmt::Debug for Editor<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The text backend and the GPU resources have no useful representation, and requiring
        // `T: Debug` would put a bound on the trait for the sake of a derive.
        f.debug_struct("Editor")
            .field("bytes", &self.document().len_bytes())
            .field("caret", &self.document().selection().head())
            .field("glyphs_packed", &self.atlas.len())
            .field("latency_samples", &self.latency.len())
            .finish_non_exhaustive()
    }
}

impl<T: TextLayout> Editor<T> {
    /// An editor over `source`, drawn with `text`.
    pub fn new(source: &str, text: T) -> Self {
        let mut workspace = Workspace::new();
        // Set here as well as in `with_workspace`, because a file opened later through the menu goes
        // into this one and has to get its language too.
        workspace.with_language_resolver(crate::detect::language_for);
        Self {
            workspace,
            scratch: Document::from(source),
            theme: Theme::default(),
            text,
            atlas: Atlas::new(),
            window: None,
            renderer: None,
            latency: Latency::new(),
            pending_keystroke: None,
            build: Latency::new(),
            first_paint: None,
            gpu_error: None,
            modifiers: ModifiersState::empty(),
            tab_rects: Vec::new(),
            chrome: Chrome::Strip,
            row_rects: Vec::new(),
            pane_scroll: 0.0,
            row_labels: HashMap::new(),
            cursor: (0.0, 0.0),
            scroll: Scroll::new(),
            viewport: Viewport::default(),
            follow_caret: false,
            heights: HashMap::new(),
            layouts: HashMap::new(),
            anchor: None,
            composition: Composition::new(),
            caret_area: None,
            last_frame: None,
            desired_x: None,
            dragging: false,
            content: ContentRect::default(),
            clipboard: Box::new(System),
            prompt: Box::new(KeepEverything),
            pick_save_path: Box::new(|_| None),
            mode: view::Mode::default(),
            config: Config::default(),
            digits: None,
            words: None,
            status_labels: [None, None],
            counted_for: None,
            menu: None,
            menu_proxy: None,
        }
    }

    /// The latency samples gathered so far.
    #[must_use]
    pub fn latency(&self) -> &Latency {
        &self.latency
    }

    /// The CPU half of those samples: keystroke to quads ready, before the GPU is involved.
    #[must_use]
    pub fn build_latency(&self) -> &Latency {
        &self.build
    }

    /// When the first frame reached the screen.
    ///
    /// The file-open target is stated as *first paint*, not as first window, and the two are not
    /// close: a window appears before anything has been laid out.
    #[must_use]
    pub fn first_paint(&self) -> Option<Instant> {
        self.first_paint
    }

    /// Why the editor could not start, if it could not.
    #[must_use]
    pub fn gpu_error(&self) -> Option<&GpuError> {
        self.gpu_error.as_ref()
    }

    /// An editor over a restored workspace.
    ///
    /// Nothing is read until the active tab is displayed, which is the point of the whole type.
    pub fn with_workspace(mut workspace: Workspace, text: T) -> Self {
        workspace.with_language_resolver(crate::detect::language_for);
        let mut editor = Self::new("", text);
        editor.workspace = workspace;
        editor
    }

    /// Applies settings read from the config file.
    pub fn with_config(&mut self, config: Config) {
        self.config = config;
    }

    /// What the user chose.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config
    }

    /// Where the document was allowed to draw last frame.
    ///
    /// For a test that needs to know where the text starts — which moves when chrome appears, and is
    /// exactly what a click has to agree with.
    #[must_use]
    pub fn content_rect(&self) -> ContentRect {
        self.content
    }

    /// Uses a different clipboard.
    ///
    /// So a test can assert what copy put there and what paste took, without depending on whatever
    /// the machine happens to have on its real clipboard — or racing another test doing the same.
    pub fn with_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.clipboard = clipboard;
    }

    /// Answers the unsaved-changes question differently.
    ///
    /// So a test can close a modified tab without a dialog — and, on Windows, without linking one:
    /// see [`crate::prompt`] for why merely linking `rfd` stops a test binary from starting.
    pub fn with_prompt(&mut self, prompt: Box<dyn UnsavedPrompt>) {
        self.prompt = prompt;
    }

    /// The tabs, for a caller that wants to report on them.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// The tabs, to change. For a caller driving the editor without a window.
    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    /// Displays whichever tab the session had active, or the first one.
    ///
    /// This is the moment a handle becomes a document, and at startup it is the only one.
    pub fn activate_saved_tab(&mut self) {
        let index = self.workspace.active().unwrap_or(0);
        if let Err(error) = self.workspace.activate(index) {
            crate::log_error!("could not open tab: {error}");
        }
    }

    /// Writes the tab list, for a caller shutting down.
    ///
    /// Handles only, never content — that is what makes restoring one cheap.
    pub fn save_session(&self, path: &Path) {
        if let Err(error) = self.workspace.save_session(path) {
            crate::log_error!("could not save session: {error}");
        }
    }

    /// The buffer being edited: the active tab's, or the scratch document if no file is open.
    #[must_use]
    pub fn document(&self) -> &Document {
        self.workspace
            .active_open()
            .map_or(&self.scratch, |open| &open.document)
    }

    /// The text engine, for a test that wants to know what was asked of it.
    #[must_use]
    pub fn text_backend(&self) -> &T {
        &self.text
    }

    /// The buffer being edited, to change.
    pub fn document_mut(&mut self) -> &mut Document {
        match self.workspace.active_open_mut() {
            Some(open) => &mut open.document,
            None => &mut self.scratch,
        }
    }

    /// Whether the buffer differs from what is on disk.
    ///
    /// Asks the tab rather than comparing here: an untitled buffer has no disk to differ from, and
    /// deciding what "modified" means for one belongs with the type that knows it has no file. This
    /// used to be a second copy of the comparison, which is one copy too many.
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.workspace
            .active()
            .and_then(|index| self.workspace.tabs().get(index))
            .is_some_and(mdedit_session::Tab::is_modified)
    }

    /// Writes the buffer back to the file it came from.
    ///
    /// Reloads afterwards so the in-memory original matches what is now on disk — otherwise the
    /// next save would compare against stale bytes and take the wrong path.
    pub fn save(&mut self) {
        let Some(path) = self.workspace.active_path().map(Path::to_path_buf) else {
            // Either the built-in sample, which has nowhere to go, or an untitled buffer, which needs
            // somewhere chosen first. Neither is an error.
            return;
        };
        self.write_active_to(&path);
    }

    /// Writes the visible tab to `path` and points the tab at it.
    ///
    /// The Save As half of the File menu, with the choosing left to the caller — so the command is
    /// testable without a file dialog, and the dialog stays where the platform code is.
    ///
    /// Byte preservation still holds: a tab that came from a file and has not been edited writes its
    /// original bytes verbatim, even to a new name, so copying a Shift-JIS file somewhere else does
    /// not silently re-encode it.
    pub fn save_as(&mut self, path: &Path) -> bool {
        if !self.write_active_to(path) {
            return false;
        }
        let Some(index) = self.workspace.active() else {
            return false;
        };
        match mdedit_file::load(path) {
            Ok(file) => self
                .workspace
                .bind_to_path(index, path.to_path_buf(), file)
                .is_ok(),
            // The bytes are on disk. Only our record of the tab is stale, which matters for the next
            // save rather than for this one.
            Err(error) => {
                crate::log_error!("saved, but could not re-read {}: {error}", path.display());
                false
            }
        }
    }

    /// Writes the visible tab's buffer to `path`. Returns whether it got there.
    fn write_active_to(&mut self, path: &Path) -> bool {
        let Some(open) = self.workspace.active_open() else {
            return false;
        };

        let text = open.document.to_string();
        // A buffer that has never been on disk has no bytes to preserve, so the encoding is chosen
        // rather than inherited. One that has goes through the byte-preserving path.
        let written = match &open.file {
            Some(file) => mdedit_file::save(path, &text, file),
            None => mdedit_file::save_new(path, &text),
        };
        if let Err(error) = written {
            crate::log_error!("could not save {}: {error}", path.display());
            return false;
        }

        // Re-read so the modified comparison is against what is now on disk. Without this the next
        // save would compare against stale bytes and take the re-encoding path for nothing.
        let reloaded = match &open.file {
            Some(file) => mdedit_file::reload(path, file),
            None => mdedit_file::load(path),
        };
        match reloaded {
            Ok(file) => self.workspace.set_active_file(file),
            Err(error) => {
                crate::log_error!("saved, but could not re-read {}: {error}", path.display());
            }
        }
        true
    }

    /// Asks where to put the visible tab, then puts it there.
    ///
    /// The asking is somebody else's: [`with_file_picker`](Self::with_file_picker) supplies it, and
    /// the default declines — so an editor with no dialog does nothing rather than writing somewhere
    /// nobody chose.
    pub fn save_as_prompted(&mut self) -> bool {
        let suggestion = self
            .workspace
            .active()
            .and_then(|index| self.workspace.tabs().get(index))
            .map(mdedit_session::Tab::display_name);
        let Some(path) = (self.pick_save_path)(suggestion.as_deref()) else {
            return false;
        };
        self.save_as(&path)
    }

    /// Does what a menu item asked for. Returns whether the frame needs redrawing.
    ///
    /// Separate from the event handler so the whole menu is testable without one — a native menu
    /// cannot be clicked by a test, but this can be called by one, and every arm is a command that
    /// already exists. The menu is a surface, not a second implementation.
    pub fn apply_command(&mut self, command: Command) -> bool {
        match command {
            Command::New => {
                self.new_file();
                true
            }
            Command::Save => {
                self.save();
                true
            }
            Command::SaveAs => self.save_as_prompted(),
            Command::SaveAll => {
                self.save_all();
                true
            }
            Command::Close => self.close_tab(),
            Command::CloseAll => self.close_all(),
            Command::Undo => self.document_mut().undo(),
            Command::Redo => self.document_mut().redo(),
            Command::Cut => self.cut(),
            Command::Copy => {
                self.copy();
                false
            }
            Command::Paste => self.paste(),
        }
    }

    /// Builds the menu bar and puts it on the window.
    ///
    /// Quiet about failure: an editor with no menu bar still edits, and every command it offers has
    /// a chord. Losing the menu is a worse day, not a lost document.
    fn attach_menu(&mut self, window: &Window) {
        let Some(menu) = MenuBar::new() else {
            return;
        };
        if let Some(proxy) = self.menu_proxy.clone() {
            menu.forward_to(move |command| {
                // The loop has gone away, which happens while the editor is shutting down. Nothing
                // to deliver it to and nothing to do about it.
                let _ = proxy.send_event(command);
            });
        }

        // Getting the handle is safe; only attaching to it is not, and that lives in the platform
        // crate where `AGENTS.md` permits it.
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                menu.attach(win32.hwnd.get());
            }
        }
        self.menu = Some(menu);
    }

    /// Carries menu commands into this editor's event loop.
    ///
    /// Held until there is a menu to attach it to, which is when the window appears.
    pub fn forward_menu(&mut self, proxy: EventLoopProxy<Command>) {
        self.menu_proxy = Some(proxy);
    }

    /// Supplies the "where should this go?" dialog.
    ///
    /// Kept out of the library for the same reason the unsaved-changes prompt is: `rfd` cannot be
    /// linked into a test binary. See [`crate::prompt`].
    pub fn with_file_picker(&mut self, picker: SavePathPicker) {
        self.pick_save_path = picker;
    }

    /// Opens an empty buffer that has never been saved, and shows it.
    pub fn new_file(&mut self) {
        let index = self.workspace.add_untitled();
        if let Err(error) = self.workspace.activate(index) {
            crate::log_error!("could not open a new file: {error}");
        }
    }

    /// Saves every tab that has somewhere to go, reporting the ones that did not.
    ///
    /// Carries on past a failure rather than stopping at the first: the point of Save All is that
    /// afterwards everything that could be saved has been.
    pub fn save_all(&mut self) -> usize {
        let mut saved = 0;
        let was_active = self.workspace.active();

        for index in 0..self.workspace.tabs().len() {
            let Some(tab) = self.workspace.tabs().get(index) else {
                continue;
            };
            // An untitled tab needs a path chosen, which is a question rather than a save. Nothing
            // to do here, and nothing to complain about either.
            if !tab.is_modified() || tab.is_untitled() {
                continue;
            }
            let Some(path) = tab.path().map(Path::to_path_buf) else {
                continue;
            };
            // Saving works on the visible tab, because that is the one whose bytes are in hand.
            if self.workspace.activate(index).is_err() {
                continue;
            }
            if self.write_active_to(&path) {
                saved += 1;
            }
        }

        if let Some(index) = was_active {
            let _ = self.workspace.activate(index);
        }
        saved
    }

    /// Closes every tab, asking about each one that has unsaved work.
    ///
    /// Stops on the first Cancel rather than closing the rest anyway — a cancelled prompt means "I
    /// did not mean this", not "skip that one".
    pub fn close_all(&mut self) -> bool {
        while !self.workspace.tabs().is_empty() {
            if !self.close_tab_at(0) {
                return false;
            }
        }
        true
    }

    /// Shows the next or previous tab, wrapping.
    ///
    /// Unsigned arithmetic throughout: adding `count - 1` is the same as subtracting one, modulo
    /// `count`, and avoids a signed cast that clippy is right to flag.
    fn cycle_tab(&mut self, forward: bool) -> bool {
        let count = self.workspace.tabs().len();
        if count < 2 {
            return false;
        }
        let current = self.workspace.active().unwrap_or(0);
        let step = if forward { 1 } else { count - 1 };
        let next = (current + step) % count;
        if let Err(error) = self.workspace.activate(next) {
            crate::log_error!("could not open tab: {error}");
            return false;
        }
        true
    }

    /// Closes the visible tab.
    ///
    /// Refuses to discard unsaved work silently. There is no dialog yet, so it says so and declines
    /// rather than losing the edit — a prompt arrives with the tab strip.
    fn close_tab(&mut self) -> bool {
        match self.workspace.active() {
            Some(index) => self.close_tab_at(index),
            None => false,
        }
    }

    /// Closes a specific tab, asking first if it has unsaved work.
    ///
    /// **Returns whether the tab was closed** — not whether something happened. The difference
    /// matters: [`close_all`](Self::close_all) walks tabs until one refuses, and a "yes, but it is
    /// still there" would spin on the same tab for ever. It was that, until a test for an untitled
    /// buffer that cannot be saved found it.
    ///
    /// The prompt is a native dialog rather than something drawn in the window. `AGENTS.md` already
    /// names `rfd` for dialogs, and a modal built from our own quads would need focus handling,
    /// keyboard traversal and a screen-reader story that none of the rest of the editor has yet.
    fn close_tab_at(&mut self, index: usize) -> bool {
        let modified = self
            .workspace
            .tabs()
            .get(index)
            .is_some_and(mdedit_session::Tab::is_modified);

        if modified {
            let name = self
                .workspace
                .tabs()
                .get(index)
                .map_or_else(|| "this file".to_owned(), mdedit_session::Tab::display_name);

            match self.prompt.ask(&name) {
                Unsaved::Save => {
                    // Saving the tab being closed means showing it first: `save` works on the
                    // active tab, which is the only one whose bytes we hold.
                    if self.workspace.activate(index).is_err() {
                        return false;
                    }
                    self.save();
                    // A failed save leaves the buffer dirty, and closing then would lose it. That
                    // includes an untitled buffer, where `save` has nowhere to write and does
                    // nothing at all.
                    if self.is_modified() {
                        crate::log_error!("not closing {name}: the save did not succeed");
                        return false;
                    }
                }
                Unsaved::Discard => {}
                Unsaved::Cancel => return false,
            }
        }

        match self.workspace.close(index) {
            Ok(_) => {
                if let Some(active) = self.workspace.active() {
                    let _ = self.workspace.activate(active);
                }
                true
            }
            Err(error) => {
                crate::log_error!("could not close tab: {error}");
                false
            }
        }
    }

    /// The window title: the file name, marked when there are unsaved changes, with a tab count.
    pub fn title(&self) -> String {
        // The tab's own name, so an untitled buffer shows as "Untitled 1" rather than falling back
        // to the bare product name as if nothing were open.
        let Some(name) = self
            .workspace
            .active()
            .and_then(|index| self.workspace.tabs().get(index))
            .map(mdedit_session::Tab::display_name)
        else {
            return "Simple Markdown Editor".to_owned();
        };
        let marker = if self.is_modified() { "•" } else { "—" };
        let count = self.workspace.tabs().len();
        if count > 1 {
            let position = self.workspace.active().unwrap_or(0) + 1;
            format!("{name} {marker} {position}/{count} — Simple Markdown Editor")
        } else {
            format!("{name} {marker} Simple Markdown Editor")
        }
    }

    /// Applies one key press to the document.
    ///
    /// Returns whether anything changed and a redraw is needed.
    /// Applies a Ctrl chord. Returns whether anything changed and a redraw is needed.
    ///
    /// Separate from the rest of the key handling because they are two different things: a chord is
    /// a command, and everything else edits or moves.
    fn handle_chord(&mut self, event: &KeyEvent) -> bool {
        // Ctrl+Home and Ctrl+End go to the ends of the document, which is where every editor puts
        // them. The caret moves, so the scroll follows without either arm saying so.
        match &event.logical_key {
            Key::Named(NamedKey::Home) => {
                self.select(ByteOffset::ZERO, self.modifiers.shift_key());
                return true;
            }
            Key::Named(NamedKey::End) => {
                let end = ByteOffset::new(self.document().len_bytes());
                self.select(end, self.modifiers.shift_key());
                return true;
            }
            _ => {}
        }

        let Key::Character(character) = &event.logical_key else {
            return false;
        };
        match character.as_str() {
            "s" | "S" => {
                self.save();
                false
            }
            // Shift+Ctrl+Z is the other conventional redo, and costs one arm to support.
            "z" if !self.modifiers.shift_key() => self.document_mut().undo(),
            "z" | "Z" | "y" | "Y" => self.document_mut().redo(),
            "w" | "W" => self.close_tab(),
            "	" => self.cycle_tab(!self.modifiers.shift_key()),
            "c" | "C" => {
                self.copy();
                false
            }
            "x" | "X" => self.cut(),
            "v" | "V" => self.paste(),
            "a" | "A" => {
                self.select_all();
                true
            }
            // Typora's binding for the same idea, which is the closest thing to a convention a
            // WYSIWYG Markdown editor has.
            "/" => {
                self.toggle_mode();
                true
            }
            // What VS Code, Sublime and Zed all use for showing and hiding the sidebar. There is no
            // menu item, because there is still no View menu to put one in.
            "b" | "B" => {
                self.toggle_chrome();
                true
            }
            _ => false,
        }
    }

    fn handle_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }

        // A key press should leave the caret on screen. The two keys that scroll without moving it
        // say so themselves, below.
        self.follow_caret = true;

        // Chords first, so a modified key never inserts its own letter.
        if self.modifiers.control_key() {
            return self.handle_chord(event);
        }

        let caret = self.document().selection().head();
        // Shift with a motion key drags the selection along instead of collapsing it.
        let extend = self.modifiers.shift_key();

        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                // With something selected, Backspace removes the selection rather than the character
                // before it — which is also what makes it one edit and one undo step.
                if self.delete_selection() {
                    return true;
                }
                if caret == ByteOffset::ZERO {
                    return false;
                }
                let previous = self.previous_boundary(caret);
                if self
                    .document_mut()
                    .delete(ByteSpan::new(previous, caret))
                    .is_err()
                {
                    return false;
                }
                // `delete` already leaves the caret at the start of what it removed.
                true
            }
            Key::Named(NamedKey::Delete) if self.delete_selection() => true,
            Key::Named(NamedKey::Delete) => {
                let next = self.next_boundary(caret);
                if next == caret
                    || self
                        .document_mut()
                        .delete(ByteSpan::new(caret, next))
                        .is_err()
                {
                    return false;
                }
                true
            }
            Key::Named(NamedKey::ArrowLeft) => {
                let target = self.previous_boundary(caret);
                self.select(target, extend);
                true
            }
            Key::Named(NamedKey::ArrowRight) => {
                let target = self.next_boundary(caret);
                self.select(target, extend);
                true
            }
            Key::Named(NamedKey::ArrowUp) => self.move_vertically(false, extend),
            Key::Named(NamedKey::ArrowDown) => self.move_vertically(true, extend),
            Key::Named(NamedKey::Home) => {
                self.select(ByteOffset::ZERO, extend);
                true
            }
            Key::Named(NamedKey::End) => {
                let end = ByteOffset::new(self.document().len_bytes());
                self.select(end, extend);
                true
            }
            // Paging moves the view, not the caret — so the caret may end up off screen, and must
            // not be dragged back the moment the frame is built.
            Key::Named(NamedKey::PageUp) => {
                self.follow_caret = false;
                self.page(-1.0);
                true
            }
            Key::Named(NamedKey::PageDown) => {
                self.follow_caret = false;
                self.page(1.0);
                true
            }
            Key::Named(NamedKey::Enter) => self.insert("\n"),
            Key::Named(NamedKey::Space) => self.insert(" "),
            Key::Character(text) => self.insert(text),
            _ => false,
        }
    }

    /// Applies one input-method event. Returns whether a redraw is needed.
    ///
    /// The composition itself lives in [`crate::ime`], which owns the rule that matters: the
    /// pre-edit never touches the buffer, so a commit is one edit and a cancel is none.
    fn handle_ime(&mut self, ime: winit::event::Ime) -> bool {
        let action = match ime {
            winit::event::Ime::Preedit(text, cursor) => self.composition.preedit(text, cursor),
            // One edit for the whole composition, however many keystrokes built it.
            winit::event::Ime::Commit(text) => self.composition.commit(text),
            // Focus moved away, or the composition was abandoned. There is nothing to undo: the
            // pre-edit was never in the buffer, and anything the input method meant to keep it has
            // already sent as a commit.
            winit::event::Ime::Disabled => self.composition.cancel(),
            winit::event::Ime::Enabled => Action::Nothing,
        };

        match action {
            Action::Nothing => false,
            Action::Redraw => true,
            Action::Insert(text) => {
                let inserted = self.insert(&text);
                self.follow_caret = true;
                inserted
            }
        }
    }

    /// The selected text, or nothing when the selection is collapsed.
    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        let span = self.document().selection().span();
        (!span.is_empty()).then(|| span.slice(&self.document().to_string()).to_owned())
    }

    /// Puts the selection on the clipboard, leaving the buffer alone.
    pub fn copy(&mut self) {
        if let Some(text) = self.selected_text() {
            self.clipboard.write(&text);
        }
    }

    /// Puts the selection on the clipboard and removes it. One edit.
    pub fn cut(&mut self) -> bool {
        let Some(text) = self.selected_text() else {
            return false;
        };
        self.clipboard.write(&text);
        self.follow_caret = true;
        self.delete_selection()
    }

    /// Replaces the selection with whatever is on the clipboard, as one edit.
    ///
    /// Multi-line text included: a paste is one insertion however many lines it contains, so undoing
    /// it takes one step rather than one per line.
    pub fn paste(&mut self) -> bool {
        let Some(text) = self.clipboard.read() else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        self.follow_caret = true;
        self.insert(&text)
    }

    /// Selects the whole buffer.
    pub fn select_all(&mut self) {
        let end = ByteOffset::new(self.document().len_bytes());
        self.document_mut()
            .set_selection(Selection::new(ByteOffset::ZERO, end));
        self.desired_x = None;
    }

    /// Removes what is selected, if anything is. One edit, so one undo step.
    ///
    /// Returns whether there was a selection to remove, which is how the callers tell "handled" from
    /// "carry on and do the single-character thing".
    pub fn delete_selected(&mut self) -> bool {
        self.delete_selection()
    }

    /// Inserts text at the caret, replacing the selection if there is one.
    pub fn insert_text(&mut self, text: &str) -> bool {
        self.insert(text)
    }

    /// Removes what is selected, if anything is. One edit, so one undo step.
    fn delete_selection(&mut self) -> bool {
        let span = self.document().selection().span();
        if span.is_empty() {
            return false;
        }
        // `delete` leaves the caret at the start of what it removed, which is where the selection
        // began — the same place every editor leaves it.
        self.document_mut().delete(span).is_ok()
    }

    /// Inserts `text` at the caret, replacing the selection if there is one.
    ///
    /// One edit either way. Typing over a selection used to insert *beside* it, which left the
    /// selected text in the buffer and the highlight covering something the caret was no longer in.
    fn insert(&mut self, text: &str) -> bool {
        let span = self.document().selection().span();
        if !span.is_empty() {
            return self.document_mut().replace(span, text).is_ok();
        }
        let caret = span.start();
        if self.document_mut().insert(caret, text).is_err() {
            return false;
        }
        // `insert` leaves the caret after what it inserted.
        true
    }

    /// The character boundary before `offset`, or the start of the buffer.
    fn previous_boundary(&self, offset: ByteOffset) -> ByteOffset {
        let text = self.document().to_string();
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
        let text = self.document().to_string();
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

    /// Moves the view by whole pages, against whatever the last frame measured.
    pub fn page(&mut self, pages: f32) {
        self.scroll.page(
            pages,
            self.viewport.line,
            self.viewport.content,
            self.viewport.height,
        );
    }

    /// Moves the view by device-independent pixels, positive being further down the document.
    ///
    /// Clamped against the last frame's measurements, so nothing here can scroll past either end.
    pub fn scroll_by(&mut self, delta: f32) {
        self.scroll
            .by(delta, self.viewport.content, self.viewport.height);
    }

    /// How far down the document the view has moved.
    #[must_use]
    pub fn scroll_offset(&self) -> f32 {
        self.scroll.offset()
    }

    /// Brings the caret back into view on the next frame.
    pub fn follow_caret(&mut self) {
        self.follow_caret = true;
    }

    /// Lays the document out and turns it into quads.
    ///
    /// `width` and `height` are the window's, in physical pixels. Public so a headless test can
    /// assert what gets drawn without a GPU: nothing here touches the renderer.
    pub fn build_frame(&mut self, width: f32, height: f32, scale: f32) -> Vec<QuadInstance> {
        let mut quads = Vec::new();

        // Everything the chrome has not taken. One place decides it; see `crate::content`.
        self.content = ContentRect::new(width / scale, height / scale, MARGIN, self.insets());
        let content = self.content;
        let visible = content.height();
        let mut frame = self.measure(content.width());

        self.hold_anchor(&frame, visible);
        if self.follow_caret {
            if let Some((index, caret)) = self.caret_position(&frame) {
                let caret_top = frame.tops[index] + caret.top;
                self.scroll
                    .reveal(caret_top, caret_top + caret.height, frame.content, visible);
            }
        }
        self.follow_caret = false;
        // Editing and resizing both change what there is to scroll through, so the offset from the
        // last frame may now be past the end.
        self.scroll
            .set(self.scroll.offset(), frame.content, visible);
        let offset = self.scroll.offset();

        // Measure what is about to be drawn *before* deciding where it goes. Estimates are fine for
        // chunks nobody can see; a chunk on screen placed by an estimate would be drawn in the wrong
        // place and only corrected on the next frame, which there may not be one of.
        let warmed = blocks::visible(&frame.tops, &frame.heights, offset, visible);
        self.warm(&mut frame, warmed);
        self.scroll.set(offset, frame.content, visible);
        let offset = self.scroll.offset();

        let range = blocks::visible(&frame.tops, &frame.heights, offset, visible);
        let at = Placement {
            content,
            scroll: offset,
            scale,
        };
        let line = self.draw_chunks(&mut quads, &frame, range.clone(), at);

        self.viewport = Viewport {
            content: frame.content,
            height: visible,
            line,
        };
        self.take_anchor(&frame, offset);

        let caret = self
            .caret_position(&frame)
            .filter(|(index, _)| range.contains(index))
            .map(|(index, caret)| CaretRect {
                top: frame.tops[index] + caret.top,
                ..caret
            });

        let placed = caret.map(|caret| content.to_window(caret.x, caret.top, offset));
        self.caret_area = placed
            .zip(caret)
            .map(|((x, y), caret)| (x, y, caret.height));

        if let (Some((x, y)), Some(caret)) = (placed, caret) {
            quads.push(QuadInstance {
                rect: [
                    x * scale,
                    y * scale,
                    CARET_WIDTH * scale,
                    caret.height * scale,
                ],
                // The opaque texel at the atlas origin: a solid fill is a glyph quad that samples it.
                uv: [0.0, 0.0, 1.0, 1.0],
                colour: [
                    self.theme.caret.r,
                    self.theme.caret.g,
                    self.theme.caret.b,
                    self.theme.caret.a,
                ],
            });

            if self.composition.is_active() {
                self.draw_preedit(&mut quads, (x, y), caret.height, scale);
            }
        }

        // ALL CHROME DRAWS AFTER THE DOCUMENT, and that ordering is a rule rather than a
        // preference: there is no depth buffer and no scissor, so a later quad simply covers an
        // earlier one. The row cull deliberately keeps up to a line-height of overshoot above the
        // viewport — it is what makes a partly-scrolled row look cut off at the top edge instead of
        // popping in — and the strip used to be built first, so those rows painted straight over it
        // and it read as transparent (MDEDIT-39).
        //
        // Anything added here goes after the document too, or it will be drawn on.
        quads.append(&mut self.build_tab_strip(width, scale));
        self.draw_pane(&mut quads, height, scale);
        self.draw_status_bar(&mut quads, width, height, scale);

        // Kept for the next click and the next Up or Down: both need to know where the lines
        // ended up, and this is the only place that knows.
        self.last_frame = Some(frame);
        quads
    }

    /// Draws the status bar across the foot of the window.
    ///
    /// The breadcrumb goes left and gets whatever room the right-hand fields leave, because it is
    /// the only part whose length the document decides. Everything on the right is short and
    /// bounded, so it is measured and placed from the right edge.
    fn draw_status_bar(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        width: f32,
        height: f32,
        scale: f32,
    ) {
        let logical_height = height / scale;
        let logical_width = width / scale;
        let top = logical_height - status::BAR_HEIGHT;

        quads.push(Self::solid(
            [
                0.0,
                top * scale,
                width,
                status::BAR_HEIGHT.mul_add(scale, 1.0),
            ],
            self.theme.strip,
        ));

        let style = TextStyle::body(self.theme.body_size * 0.85);
        let baseline = top + status::BAR_HEIGHT * 0.68;

        // The one time a count happens on a drawing path: a document nobody has typed in yet has
        // never set the idle timer, so without this it would show no count until first edited. It
        // costs one walk of the buffer, once per document — a keystroke leaves a count in place, so
        // it cannot recur on the path this is kept off.
        if self.words.is_none() || self.counted_for != self.workspace.active() {
            self.recount_words();
        }

        let head = self.document().selection().head();
        let (line, column) = self.document().line_and_column(head);
        let position = status::position(line.get(), column.get());

        let format = self
            .workspace
            .active_open()
            .and_then(|open| open.file.as_ref().map(mdedit_file::LoadedFile::format));
        let details = status::details(
            format.map_or("UTF-8", |format| format.encoding()),
            format.map_or_else(
                // An untitled buffer has no file to have detected anything from, so it will be
                // written with whatever `Newline::default` is until it is saved somewhere.
                mdedit_file::Newline::default,
                |format| format.newline(),
            ),
            self.words,
        );

        // TWO CONTAINERS THAT CANNOT OVERLAP, which is the whole of MDEDIT-38. The two sides are not
        // symmetrical: the right-hand fields are short and bounded, the breadcrumb is neither, so
        // the fields get the room they ask for up to a share of the bar and the breadcrumb takes
        // what is left. Both clip to their own container rather than running into the other.
        let right_text = format!("{position}   {details}");
        let wanted = self.status_label(RIGHT, &right_text, style);
        let right_room = wanted.min(status::right_share(logical_width));
        let right_x = (logical_width - status::PADDING - right_room).max(status::PADDING);
        self.draw_status_label(
            quads,
            RIGHT,
            right_x,
            baseline,
            self.theme.marker,
            scale,
            right_room,
        );

        let steps: Vec<String> = self
            .document()
            .heading_trail(head)
            .iter()
            .map(|heading| self.document().heading_text(heading))
            .collect();
        let trail = status::breadcrumb(&steps);
        self.status_label(TRAIL, &trail, style);
        if !trail.is_empty() {
            // Up to the right container's edge, less a gap so the two never touch even when both
            // are full.
            let trail_room = (right_x - status::GAP - status::PADDING).max(0.0);
            self.draw_status_label(
                quads,
                TRAIL,
                status::PADDING,
                baseline,
                self.theme.text,
                scale,
                trail_room,
            );
        }
    }

    /// Lays a status-bar string out if it has changed, and returns its width.
    fn status_label(&mut self, slot: usize, text: &str, style: TextStyle) -> f32 {
        if let Some(label) = &self.status_labels[slot] {
            if label.text == text {
                return label.width;
            }
        }

        let run = StyledRun {
            text,
            style,
            source: ByteSpan::empty_at(ByteOffset::ZERO),
        };
        let Ok(layout) = self.text.layout(&[run], f32::MAX) else {
            self.status_labels[slot] = None;
            return 0.0;
        };
        let glyphs: Vec<_> = layout.glyphs().copied().collect();
        let width = glyphs
            .iter()
            .map(|glyph| glyph.pos.x + glyph.advance)
            .fold(0.0_f32, f32::max);

        self.status_labels[slot] = Some(Label {
            text: text.to_owned(),
            glyphs,
            width,
        });
        width
    }

    /// Draws a slot that [`Self::status_label`] has already laid out.
    ///
    /// The label is moved out and put back rather than cloned, because rasterising needs `&mut self`
    /// and a per-frame copy of the glyphs would give back some of what the cache just saved.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_status_label(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        slot: usize,
        x: f32,
        baseline: f32,
        colour: Rgba,
        scale: f32,
        max_width: f32,
    ) {
        let Some(label) = self.status_labels[slot].take() else {
            return;
        };
        for glyph in &label.glyphs {
            // Truncated rather than ellipsised, as the strip's and the pane's labels are: an
            // ellipsis needs its own measurement pass. This parameter is the fix for MDEDIT-38 —
            // it existed on `draw_label`, and was lost when this function was written to draw from
            // the cache.
            if glyph.pos.x > max_width {
                break;
            }
            let Some(entry) = self.entry_for(glyph.key, scale) else {
                continue;
            };
            quads.push(QuadInstance {
                rect: [
                    (x + glyph.pos.x) * scale + px_i(entry.left),
                    baseline * scale + px_i(entry.top),
                    px(entry.width),
                    px(entry.height),
                ],
                uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                colour: [colour.r, colour.g, colour.b, colour.a],
            });
        }
        self.status_labels[slot] = Some(label);
    }

    /// Recounts the words. Called on the idle timer, never on the keystroke path.
    fn recount_words(&mut self) {
        self.words = Some(status::count_words(&self.document().rope().to_string()));
        self.counted_for = self.workspace.active();
    }

    /// Draws what the input method is composing, at the caret.
    ///
    /// On its own background rather than blended with the text underneath: the pre-edit is not in
    /// the buffer, so the line does not reflow around it, and drawing it over the following words
    /// would leave two strings on top of each other. Covering them reads as the overlay it is.
    ///
    /// Underlined, because that is what every platform's convention says uncommitted text looks
    /// like, and it is the difference between "this is being composed" and "this has been typed".
    fn draw_preedit(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        at: (f32, f32),
        height: f32,
        scale: f32,
    ) {
        let text = self.composition.text().to_owned();
        let style = TextStyle::body(self.theme.body_size);
        let run = [StyledRun {
            text: &text,
            style,
            source: ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(text.len())),
        }];
        let Ok(layout) = self.text.layout(&run, f32::MAX) else {
            return;
        };

        let width: f32 = layout
            .glyphs()
            .map(|glyph| glyph.advance)
            .sum::<f32>()
            .max(1.0);
        // Already in window coordinates: the caller placed the caret, and this hangs off it.
        let (x, y) = at;

        quads.push(Self::solid(
            [x * scale, y * scale, width * scale, height * scale],
            self.theme.background,
        ));

        for glyph in layout.glyphs() {
            let Some(entry) = self.entry_for(glyph.key, scale) else {
                continue;
            };
            quads.push(QuadInstance {
                rect: [
                    (glyph.pos.x + x) * scale + px_i(entry.left),
                    // The layout's own baseline is at the top of its box; the caret's box is where
                    // this has to sit, so the glyphs hang from the caret's baseline instead.
                    (glyph.pos.y + y) * scale + px_i(entry.top),
                    px(entry.width),
                    px(entry.height),
                ],
                uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                colour: [
                    self.theme.text.r,
                    self.theme.text.g,
                    self.theme.text.b,
                    self.theme.text.a,
                ],
            });
        }

        quads.push(Self::solid(
            [
                x * scale,
                (y + height - UNDERLINE_WIDTH) * scale,
                width * scale,
                UNDERLINE_WIDTH * scale,
            ],
            self.theme.marker,
        ));
    }

    /// Switches between the previewed view and the raw markup.
    ///
    /// A rendering mode, not a document mode: the buffer, the parse and the caret's byte are
    /// untouched. Only what is drawn changes.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            view::Mode::Preview => view::Mode::Plain,
            view::Mode::Plain => view::Mode::Preview,
        };
        // The caret is on the same byte, but that byte is somewhere else on screen now — concealed
        // markers have appeared, and every heading has changed size. Bring it back into view rather
        // than leaving the reader hunting for it.
        self.follow_caret = true;
    }

    /// Which view the document is drawn in.
    #[must_use]
    pub fn mode(&self) -> view::Mode {
        self.mode
    }

    /// How the document should be drawn right now.
    fn decoration(&self) -> view::Decoration<'_> {
        view::Decoration {
            theme: &self.theme,
            mode: self.mode,
        }
    }

    /// What the chrome has taken out of the window.
    ///
    /// The tab strip is the only one so far. A menu bar, a status bar, a file pane and a
    /// line-number gutter each add to this and nothing else changes — which is the point.
    fn insets(&mut self) -> Insets {
        // The menu bar is deliberately absent. A native Windows menu is part of the window frame
        // rather than of the client area, so Windows has already taken its height off what
        // `inner_size` reports — accounting for it here would take it off twice and leave a strip of
        // the document permanently below the fold.
        Insets {
            top: self.strip_height(),
            // The pane and the gutter both take from the left, and they add rather than compete —
            // which is exactly the arithmetic `ContentRect` exists to keep in one place.
            left: self.pane_width() + self.gutter_width(),
            bottom: status::BAR_HEIGHT,
            ..Insets::default()
        }
    }

    /// Width the file pane occupies, which is nothing unless it is the chrome in use.
    fn pane_width(&self) -> f32 {
        if self.chrome == Chrome::Pane && !self.workspace.tabs().is_empty() {
            pane::PANE_WIDTH
        } else {
            0.0
        }
    }

    /// How wide the line-number gutter is, or nothing when it is switched off.
    ///
    /// Wide enough for the largest number the document can show, with a floor of two digits so a
    /// short file's gutter does not visibly resize the moment it grows past nine lines. It changes
    /// only when the document crosses a power of ten.
    fn gutter_width(&mut self) -> f32 {
        if !self.config.line_numbers {
            return 0.0;
        }
        let lines = self.document().rope().len_lines().max(1);
        let digits = lines.ilog10() as usize + 1;
        let digit = self.digit_width();
        #[allow(
            clippy::cast_precision_loss,
            reason = "a document with more digits than an f32 can count is not reachable"
        )]
        let columns = digits.max(GUTTER_MIN_DIGITS) as f32;
        digit.mul_add(columns, GUTTER_PADDING * 2.0)
    }

    /// How wide a digit is.
    ///
    /// Measured rather than assumed, because it is what the gutter's width is built from and a guess
    /// would show as numbers crowding the text or floating away from it. Digits are the same width
    /// as each other in every face worth setting a UI in, which is the assumption the gutter's
    /// right-alignment rests on as well.
    fn digit_width(&mut self) -> f32 {
        self.digits()
            .map_or(self.theme.body_size * 0.6, |digits| digits[0].1)
    }

    /// Height the tab strip occupies, which is nothing when there is nothing to show.
    fn strip_height(&self) -> f32 {
        if self.chrome == Chrome::Strip && !self.workspace.tabs().is_empty() {
            tabs::STRIP_HEIGHT
        } else {
            0.0
        }
    }

    /// A filled rectangle, in physical pixels.
    ///
    /// Samples the opaque texel at the atlas origin, so solid fills and glyphs share one pipeline.
    fn solid(rect: [f32; 4], colour: Rgba) -> QuadInstance {
        QuadInstance {
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            colour: [colour.r, colour.g, colour.b, colour.a],
        }
    }

    /// Draws the strip, and remembers where each tab landed so a click can be resolved.
    fn build_tab_strip(&mut self, width: f32, scale: f32) -> Vec<QuadInstance> {
        self.tab_rects.clear();
        if self.chrome != Chrome::Strip || self.workspace.tabs().is_empty() {
            return Vec::new();
        }

        let logical_width = width / scale;
        let label_style = TextStyle::body(13.0);

        // Measured for as many tabs as could possibly fit, not for all of them: with two hundred
        // open, shaping every label to draw eight would be the same mistake the document renderer
        // makes, and there is no reason to repeat it here.
        let names: Vec<String> = self
            .workspace
            .tabs()
            .iter()
            .map(mdedit_session::Tab::display_name)
            .collect();
        let mut label_widths = Vec::with_capacity(names.len());
        for name in &names {
            let run = StyledRun {
                text: name,
                style: label_style,
                source: ByteSpan::empty_at(ByteOffset::ZERO),
            };
            let measured = self.text.layout(&[run], f32::MAX).map_or(60.0, |layout| {
                layout
                    .glyphs()
                    .map(|glyph| glyph.pos.x + glyph.advance)
                    .fold(0.0_f32, f32::max)
            });
            label_widths.push(measured);
            // One more than fits is enough to know where the edge is.
            if label_widths
                .iter()
                .sum::<f32>()
                .gt(&(logical_width + tabs::STRIP_HEIGHT))
            {
                break;
            }
        }

        self.tab_rects = tabs::layout(&label_widths, logical_width);

        let active = self.workspace.active();
        let mut quads = vec![Self::solid(
            [0.0, 0.0, width, tabs::STRIP_HEIGHT * scale],
            self.theme.strip,
        )];

        for rect in self.tab_rects.clone() {
            let is_active = active == Some(rect.index);
            let background = if is_active {
                self.theme.tab_active
            } else {
                self.theme.tab_inactive
            };
            // A one-pixel gap on the right separates neighbours without a border pass.
            quads.push(Self::solid(
                [
                    rect.x * scale,
                    0.0,
                    (rect.width - 1.0) * scale,
                    tabs::STRIP_HEIGHT * scale,
                ],
                background,
            ));

            let modified = self
                .workspace
                .tabs()
                .get(rect.index)
                .is_some_and(mdedit_session::Tab::is_modified);

            // The close affordance: a dot when there is unsaved work, an x otherwise. Same target
            // either way, so the click does not move under the pointer when a file becomes dirty.
            let (close_x, close_width) = rect.close_region();
            let glyph = if modified { "•" } else { "×" };
            let colour = if modified {
                self.theme.tab_modified
            } else {
                self.theme.marker
            };
            self.draw_label(
                &mut quads,
                glyph,
                label_style,
                close_x + close_width / 2.0 - 4.0,
                // Placed so the label sits centred in the strip.
                tabs::STRIP_HEIGHT * 0.68,
                colour,
                scale,
                close_width,
            );

            if let Some(name) = names.get(rect.index) {
                self.draw_label(
                    &mut quads,
                    name,
                    label_style,
                    rect.label_x(),
                    tabs::STRIP_HEIGHT * 0.68,
                    if is_active {
                        self.theme.text
                    } else {
                        self.theme.marker
                    },
                    scale,
                    rect.label_width(),
                );
            }
        }

        quads
    }

    /// Draws one short string of chrome, clipped to `max_width`.
    ///
    /// `baseline` is in window pixels, so the same call serves the strip at the top and the status
    /// bar at the bottom.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_label(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        text: &str,
        style: TextStyle,
        x: f32,
        baseline: f32,
        colour: Rgba,
        scale: f32,
        max_width: f32,
    ) {
        let run = StyledRun {
            text,
            style,
            source: ByteSpan::empty_at(ByteOffset::ZERO),
        };
        let Ok(layout) = self.text.layout(&[run], f32::MAX) else {
            return;
        };

        let glyphs: Vec<_> = layout.glyphs().copied().collect();
        for glyph in glyphs {
            if glyph.pos.x > max_width {
                // Truncated rather than ellipsised: an ellipsis needs its own measurement pass and
                // buys little at this size.
                break;
            }
            let Some(entry) = self.entry_for(glyph.key, scale) else {
                continue;
            };
            quads.push(QuadInstance {
                rect: [
                    (x + glyph.pos.x) * scale + px_i(entry.left),
                    baseline * scale + px_i(entry.top),
                    px(entry.width),
                    px(entry.height),
                ],
                uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                colour: [colour.r, colour.g, colour.b, colour.a],
            });
        }
    }

    /// Draws the file pane down the left, and remembers where its rows landed.
    ///
    /// Drawn after the document rather than before it, so that a row and a glyph cannot disagree
    /// about which is on top — the pane's inset keeps the text out of its column, and this covers
    /// the seam.
    fn draw_pane(&mut self, quads: &mut Vec<QuadInstance>, height: f32, scale: f32) {
        self.row_rects.clear();
        if self.chrome != Chrome::Pane || self.workspace.tabs().is_empty() {
            return;
        }

        let logical_height = height / scale;
        // The pane runs the full height, under nothing: there is no strip when it is showing, and
        // the status bar is drawn after it.
        let visible = (logical_height - status::BAR_HEIGHT).max(0.0);
        let count = self.workspace.tabs().len();

        self.pane_scroll = self
            .pane_scroll
            .clamp(0.0, pane::max_scroll(count, visible));
        self.row_rects = pane::layout(count, visible, self.pane_scroll);

        quads.push(Self::solid(
            [0.0, 0.0, pane::PANE_WIDTH * scale, visible * scale],
            self.theme.strip,
        ));

        let names: Vec<String> = self
            .row_rects
            .iter()
            .map(|rect| self.workspace.tabs()[rect.index].display_name())
            .collect();
        let active = self.workspace.active();
        let style = TextStyle::body(self.theme.body_size * 0.85);

        for (rect, name) in self.row_rects.clone().into_iter().zip(names) {
            let is_active = active == Some(rect.index);
            if is_active {
                quads.push(Self::solid(
                    [
                        0.0,
                        rect.y * scale,
                        pane::PANE_WIDTH * scale,
                        pane::ROW_HEIGHT * scale,
                    ],
                    self.theme.tab_active,
                ));
            }

            let modified = self
                .workspace
                .tabs()
                .get(rect.index)
                .is_some_and(mdedit_session::Tab::is_modified);
            let (close_x, close_width) = rect.close_region();
            let baseline = rect.y + pane::ROW_HEIGHT * 0.7;

            self.draw_row_label(
                quads,
                if modified { "•" } else { "×" },
                style,
                close_x + close_width / 2.0 - 4.0,
                baseline,
                if modified {
                    self.theme.tab_modified
                } else {
                    self.theme.marker
                },
                scale,
                close_width,
            );

            self.draw_row_label(
                quads,
                &name,
                style,
                rect.label_x(),
                baseline,
                if is_active {
                    self.theme.text
                } else {
                    self.theme.marker
                },
                scale,
                rect.label_width(),
            );
        }
    }

    /// Turns a wheel notch into a scroll of whatever is under the pointer.
    fn wheel(&mut self, delta: MouseScrollDelta) {
        // Wheel-up is positive and moves the view towards the top, which is a smaller offset —
        // hence the negation on both arms.
        let delta = match delta {
            MouseScrollDelta::LineDelta(_, lines) => {
                -lines * scroll::LINES_PER_NOTCH * self.viewport.line
            }
            // A precision touchpad reports pixels directly, already at the rate the user moved
            // their fingers. Truncation is fine: these are gesture-sized deltas.
            #[allow(
                clippy::cast_possible_truncation,
                reason = "scroll deltas are small pixel counts"
            )]
            MouseScrollDelta::PixelDelta(position) => -(position.y as f32),
        };

        // Over the pane, the wheel scrolls the file list rather than the document — which is what
        // every editor with a sidebar does, and the reason the pane owns an offset of its own.
        let (x, y) = self.cursor;
        if pane::hit_test(&self.row_rects, x, y).is_some() {
            self.scroll_pane(delta);
        } else {
            self.scroll_by(delta);
        }
    }

    /// Draws one row label, laying it out only if this text has not been seen before.
    ///
    /// The pane's equivalent of [`Self::status_label`], keyed by text rather than by slot because
    /// the rows are a list rather than a fixed pair. Taken out of the map and put back rather than
    /// cloned, since rasterising needs `&mut self` and a per-frame copy of the glyphs would give
    /// back some of what the cache just saved.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_row_label(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        text: &str,
        style: TextStyle,
        x: f32,
        baseline: f32,
        colour: Rgba,
        scale: f32,
        max_width: f32,
    ) {
        let label = if let Some(label) = self.row_labels.remove(text) {
            label
        } else {
            let run = StyledRun {
                text,
                style,
                source: ByteSpan::empty_at(ByteOffset::ZERO),
            };
            let Ok(layout) = self.text.layout(&[run], f32::MAX) else {
                return;
            };
            let glyphs: Vec<_> = layout.glyphs().copied().collect();
            let width = glyphs
                .iter()
                .map(|glyph| glyph.pos.x + glyph.advance)
                .fold(0.0_f32, f32::max);
            Label {
                text: text.to_owned(),
                glyphs,
                width,
            }
        };

        for glyph in &label.glyphs {
            if glyph.pos.x > max_width {
                // Truncated rather than ellipsised, as the strip's labels are: file names are longer
                // than a row is wide, and an ellipsis needs its own measurement pass.
                break;
            }
            let Some(entry) = self.entry_for(glyph.key, scale) else {
                continue;
            };
            quads.push(QuadInstance {
                rect: [
                    (x + glyph.pos.x) * scale + px_i(entry.left),
                    baseline * scale + px_i(entry.top),
                    px(entry.width),
                    px(entry.height),
                ],
                uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                colour: [colour.r, colour.g, colour.b, colour.a],
            });
        }

        self.row_labels.insert(label.text.clone(), label);
    }

    /// Scrolls the file pane's list. Returns whether it moved.
    pub fn scroll_pane(&mut self, by: f32) -> bool {
        if self.chrome != Chrome::Pane {
            return false;
        }
        let before = self.pane_scroll;
        // Clamped against the last known viewport; the next frame clamps again with a fresh one,
        // which is what makes a resize while scrolled land somewhere sensible.
        let visible = (self.viewport.height + status::BAR_HEIGHT).max(0.0);
        self.pane_scroll = (self.pane_scroll + by)
            .clamp(0.0, pane::max_scroll(self.workspace.tabs().len(), visible));
        #[allow(
            clippy::float_cmp,
            reason = "asking whether the clamp moved it, where exact equality is the question"
        )]
        let moved = self.pane_scroll != before;
        moved
    }

    /// Which presentation the open files are in.
    #[must_use]
    pub fn chrome(&self) -> Chrome {
        self.chrome
    }

    /// Swaps the tab strip for the file pane, or back.
    ///
    /// The wrapping width changes with it, so every cached layout is invalidated — correctly and by
    /// itself, because a chunk's cache key includes the width. Toggling on a large document
    /// therefore re-lays-out the viewport, which is expected rather than a bug.
    pub fn toggle_chrome(&mut self) {
        self.chrome = match self.chrome {
            Chrome::Strip => Chrome::Pane,
            Chrome::Pane => Chrome::Strip,
        };
        self.pane_scroll = 0.0;
    }

    /// A left-click anywhere in the window.
    ///
    /// Chrome first, because both presentations sit over an edge of the document; only one of the
    /// two is ever showing, so at most one can match. The order lives here rather than in the event
    /// handler so that a test clicks the same way the window does — a test that reached past this
    /// into [`click_at`](Self::click_at) would be exercising a dispatch the app does not use.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        self.click_strip(x, y) || self.click_pane(x, y) || self.click_at(x, y)
    }

    /// Resolves a click in the file pane.
    fn click_pane(&mut self, x: f32, y: f32) -> bool {
        let Some(hit) = pane::hit_test(&self.row_rects, x, y) else {
            return false;
        };
        match hit {
            pane::Hit::Select(index) => {
                if let Err(error) = self.workspace.activate(index) {
                    crate::log_error!("could not open tab: {error}");
                    return false;
                }
                true
            }
            pane::Hit::Close(index) => self.close_tab_at(index),
        }
    }

    /// Resolves a click in the tab strip.
    fn click_strip(&mut self, x: f32, y: f32) -> bool {
        let Some(hit) = tabs::hit_test(&self.tab_rects, x, y) else {
            return false;
        };
        match hit {
            tabs::Hit::Select(index) => {
                if let Err(error) = self.workspace.activate(index) {
                    crate::log_error!("could not open tab: {error}");
                    return false;
                }
                true
            }
            tabs::Hit::Close(index) => self.close_tab_at(index),
        }
    }

    /// The laid-out chunk at `index`, from whichever half of the cache has it.
    fn chunk_layout(&self, frame: &Frame, index: usize) -> Option<Rc<Cached>> {
        frame
            .fresh_at(index)
            .or_else(|| self.layouts.get(frame.keys.get(index)?).map(Rc::clone))
    }

    /// The byte a point in the window addresses, if there is a laid-out document under it.
    ///
    /// `x` and `y` are in device-independent pixels from the window's corner. Returns `None` before
    /// the first frame, when there is nothing to hit-test against and guessing would be worse than
    /// doing nothing.
    fn offset_at_point(&self, x: f32, y: f32) -> Option<ByteOffset> {
        // The same rectangle the glyphs were placed with, run backwards. Neither side computes an
        // offset of its own, so the two cannot drift apart.
        let (x, y) = self.content.to_document(x, y, self.scroll.offset());
        self.offset_in_document(x, y)
    }

    /// The byte at a point in the document's own coordinates.
    fn offset_in_document(&self, x: f32, y: f32) -> Option<ByteOffset> {
        let frame = self.last_frame.as_ref()?;
        if frame.chunks.is_empty() {
            return None;
        }
        if y < 0.0 {
            return Some(ByteOffset::ZERO);
        }

        let index = frame
            .tops
            .partition_point(|top| *top <= y)
            .saturating_sub(1)
            .min(frame.chunks.len() - 1);
        let chunk = frame.chunks[index];
        let cached = self.chunk_layout(frame, index)?;

        let target = hit::Chunk {
            layout: &cached.layout,
            text: chunk.span.slice(&frame.text),
            start: chunk.span.start(),
            base: ByteOffset::new(cached.base),
        };
        Some(hit::offset_at(
            &target,
            cached.drawn_lines(),
            y - frame.tops[index],
            x,
        ))
    }

    /// Which chunk and visual line the caret is on, and how far along it.
    fn caret_place(&self) -> Option<(usize, usize, f32)> {
        let frame = self.last_frame.as_ref()?;
        let caret = self.document().selection().head();
        let index = frame
            .chunks
            .iter()
            .position(|chunk| chunk.span.start() <= caret && caret < chunk.span.end())
            .or_else(|| frame.chunks.len().checked_sub(1))?;
        let chunk = frame.chunks[index];
        let cached = self.chunk_layout(frame, index)?;

        let target = hit::Chunk {
            layout: &cached.layout,
            text: chunk.span.slice(&frame.text),
            start: chunk.span.start(),
            base: ByteOffset::new(cached.base),
        };
        let (row, x) = hit::place_of(&target, cached.drawn_lines(), caret)?;
        Some((index, row, x))
    }

    /// Moves the caret one visual line up or down.
    ///
    /// Visual, not source: a wrapped paragraph is several rows, and Down in the middle of one has to
    /// reach the next row of the same paragraph rather than skipping the rest of it.
    ///
    /// The step lands on the very edge of the neighbouring row rather than a line-height away, so a
    /// heading or a blank line — which are not the same height as what precedes them — cannot be
    /// stepped over.
    fn move_vertically(&mut self, down: bool, extend: bool) -> bool {
        let Some((index, row, x)) = self.caret_place() else {
            return false;
        };
        let Some(frame) = self.last_frame.as_ref() else {
            return false;
        };
        let Some(cached) = self.chunk_layout(frame, index) else {
            return false;
        };

        let chunk = frame.chunks[index];
        let target = hit::Chunk {
            layout: &cached.layout,
            text: chunk.span.slice(&frame.text),
            start: chunk.span.start(),
            base: ByteOffset::new(cached.base),
        };
        let row_top = frame.tops[index] + hit::row_top(&target, row);
        let height = cached
            .layout
            .lines
            .get(row)
            .map_or(self.theme.body_size, |line| line.height);

        let aim = self.desired_x.unwrap_or(x);
        let edge = if down { row_top + height } else { row_top };
        // A hair past the boundary, so the answer is the neighbouring row whatever its height.
        let step = if down { 0.5 } else { -0.5 };
        let Some(offset) = self.offset_in_document(aim, edge + step) else {
            return false;
        };

        self.select(offset, extend);
        // Restored after `select` clears it: this is the one motion that keeps its column.
        self.desired_x = Some(aim);
        true
    }

    /// Puts the caret at `offset`, or drags the selection to it.
    fn select(&mut self, offset: ByteOffset, extend: bool) {
        let selection = if extend {
            Selection::new(self.document().selection().anchor(), offset)
        } else {
            Selection::caret(offset)
        };
        self.document_mut().set_selection(selection);
        self.desired_x = None;
    }

    /// The horizontal extent of `selected` on one visual row, if it covers any of it.
    ///
    /// A row wholly inside the selection runs to the end of its text plus a little, so that a
    /// selection spanning lines reads as covering the line breaks rather than as a ragged stack of
    /// separate highlights.
    fn row_selection(
        cached: &Cached,
        chunk: blocks::Chunk,
        text: &str,
        row: usize,
        selected: ByteSpan,
    ) -> Option<(f32, f32)> {
        let line = cached.layout.lines.get(row)?;
        let rebase = |offset: ByteOffset| {
            ByteOffset::new(chunk.span.start().get() + offset.get().saturating_sub(cached.base))
        };

        // Where this row's text sits in the buffer. Wrapping adds rows without adding newlines, so
        // a wrapped row's bounds are the whole source line's — the highlight is then a little
        // generous on a wrapped line, which is better than absent.
        let body = chunk.span.slice(text);
        let base = chunk.span.start().get();
        let row_start = base
            + body
                .split_inclusive('\n')
                .take(row)
                .map(str::len)
                .sum::<usize>();
        let row_end = row_start
            + body[row_start - base..]
                .find('\n')
                .unwrap_or(body.len() - (row_start - base));

        if selected.end().get() <= row_start || selected.start().get() > row_end {
            return None;
        }

        // Where a byte sits along the row: the glyph that starts at or after it, or past the last.
        let x_at = |offset: usize| -> f32 {
            line.glyphs
                .iter()
                .find(|glyph| rebase(glyph.source.start()).get() >= offset)
                .map_or_else(
                    || {
                        line.glyphs
                            .last()
                            .map_or(0.0, |glyph| glyph.pos.x + glyph.advance)
                    },
                    |glyph| glyph.pos.x,
                )
        };

        let from = if selected.start().get() <= row_start {
            0.0
        } else {
            x_at(selected.start().get())
        };
        // Past the end of the row means the line break is selected too, and a selection spanning
        // lines reads as one shape rather than a ragged stack only if that shows as width.
        let to = if selected.end().get() > row_end {
            x_at(usize::MAX) + EMPTY_ROW_SELECTION
        } else {
            x_at(selected.end().get())
        };
        Some((from, to))
    }

    /// Puts the caret at a point in the window, and begins dragging a selection from it.
    ///
    /// Shift extends the existing selection instead, which is how every editor lets you correct a
    /// selection whose far end you got wrong. Returns whether anything was under the pointer.
    /// Public so a headless test can point at the document without a mouse.
    pub fn click_at(&mut self, x: f32, y: f32) -> bool {
        let Some(offset) = self.offset_at_point(x, y) else {
            return false;
        };
        self.select(offset, self.modifiers.shift_key());
        self.dragging = true;
        true
    }

    /// Drags the far end of the selection to a point in the window.
    pub fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some(offset) = self.offset_at_point(x, y) else {
            return false;
        };
        self.select(offset, true);
        true
    }

    /// Moves the caret one visual line, extending the selection if asked.
    pub fn move_caret(&mut self, down: bool, extend: bool) -> bool {
        self.move_vertically(down, extend)
    }

    /// Packs a glyph if it is not already in the atlas.
    fn entry_for(&mut self, key: GlyphKey, scale: f32) -> Option<crate::gpu::AtlasEntry> {
        if let Some(entry) = self.atlas.get(key) {
            return Some(entry);
        }
        let image = self.text.rasterize(key, scale).ok()??;
        self.atlas.insert(key, &image).ok()
    }

    /// Works out where every chunk sits, laying out only the one holding the caret.
    ///
    /// Everything else is answered from what it measured last time, or estimated if it has never
    /// been measured. That is the whole of virtualization: the cost of a frame follows the number
    /// of chunks, not the number of glyphs, until something has to be drawn.
    fn measure(&mut self, width: f32) -> Frame {
        let text = self.document().to_string();
        let selection = self.document().selection();
        let chunks = blocks::split(self.document(), &text);
        let last = chunks.len().saturating_sub(1);
        let keys: Vec<ChunkKey> = chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                ChunkKey::of(
                    chunk,
                    chunk.span.slice(&text),
                    index == last,
                    width,
                    self.mode,
                )
            })
            .collect();

        let mut frame = Frame {
            chunks,
            keys,
            fresh: Vec::new(),
            heights: Vec::new(),
            tops: Vec::new(),
            content: 0.0,
            text,
            selection,
            width,
        };

        // The chunk the caret is in is laid out every frame: concealment depends on where in it the
        // caret sits, so two frames over the same bytes can want different glyphs. It happens before
        // the height table because its height feeds that table, and because following the caret
        // needs its position whether or not it is on screen.
        for index in 0..frame.chunks.len() {
            if !touches(frame.chunks[index].span, selection.span()) {
                continue;
            }
            if let Some(cached) = self.lay_out(index, &frame) {
                frame.fresh.push((index, cached));
            }
        }

        frame.heights = (0..frame.chunks.len())
            .map(|index| {
                frame
                    .fresh_at(index)
                    .map(|cached| cached.height)
                    .or_else(|| self.heights.get(&frame.keys[index]).copied())
                    .unwrap_or_else(|| self.estimate(&frame, index))
            })
            .collect();
        frame.tops = blocks::stack(&frame.heights);
        // A bottom margin to match the top one, so the last line is not flush against the edge.
        frame.content = frame.tops.last().copied().unwrap_or(0.0)
            + frame.heights.last().copied().unwrap_or(0.0)
            + MARGIN;
        frame
    }

    /// Lays out the chunks in `range` and puts their real heights into the table.
    ///
    /// The stacking is redone afterwards, so what gets drawn is placed by measurement rather than by
    /// the estimate that found it.
    fn warm(&mut self, frame: &mut Frame, range: Range<usize>) {
        let mut measured = false;
        for index in range {
            if frame.fresh_at(index).is_some() || self.heights.contains_key(&frame.keys[index]) {
                continue;
            }
            if self.lay_out(index, frame).is_some() {
                measured = true;
            }
        }
        if !measured {
            return;
        }

        for (index, height) in frame.heights.iter_mut().enumerate() {
            if let Some(measured) = self.heights.get(&frame.keys[index]) {
                *height = *measured;
            }
        }
        frame.tops = blocks::stack(&frame.heights);
        frame.content = frame.tops.last().copied().unwrap_or(0.0)
            + frame.heights.last().copied().unwrap_or(0.0)
            + MARGIN;
    }

    /// Turns the chunks in `range` into quads, laying out any that have not been laid out yet.
    ///
    /// Returns a line height for the page keys to step by — measured, not assumed, because a
    /// document that opens on a heading has a different line height from one that opens on prose.
    fn draw_chunks(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        frame: &Frame,
        range: Range<usize>,
        at: Placement,
    ) -> f32 {
        let Placement {
            content,
            scroll,
            scale,
        } = at;
        let (band_top, band_bottom) = content.visible(scroll);
        let mut line_height = self.theme.body_size;

        for index in range {
            let Some(cached) = self.lay_out(index, frame) else {
                continue;
            };
            let chunk = frame.chunks[index];
            if let Some(first) = cached.layout.lines.first() {
                line_height = first.height.max(1.0);
            }

            // Runs are rebuilt rather than stored with the layout: they are cheap for one chunk, and
            // keeping them would mean holding a `String` per run for every chunk ever measured.
            let runs = view::build_runs_in(
                self.document(),
                frame.selection,
                self.decoration(),
                chunk.span,
                &frame.text,
            );
            let chunk_top = frame.tops[index];
            let drawn = cached.drawn_lines();
            quads.reserve(
                cached.layout.lines[..drawn]
                    .iter()
                    .map(|line| line.glyphs.len())
                    .sum(),
            );

            let selected = frame.selection.span();
            let mut row_top = chunk_top;
            // Which source line the row above was on, so a wrapped row can be told from a new one.
            // `usize::MAX` for "there was no row above", which no line number can collide with.
            let mut previous_line = usize::MAX;

            for (row, line) in cached.layout.lines[..drawn].iter().enumerate() {
                // Culled in the document's own coordinates rather than the window's, so the check
                // is in the same space the layout is in.
                let baseline = line.baseline + chunk_top;
                let top_of_row = row_top;
                row_top += line.height;

                // Worked out before the cull, because a row scrolled off the top still tells the
                // row below it whether it is a continuation.
                let source_line = self.line_of_row(&cached, chunk, line, previous_line);
                let starts_a_line = source_line != previous_line;
                previous_line = source_line;

                if baseline - line.height > band_bottom || baseline < band_top - line.height {
                    continue;
                }

                if self.config.line_numbers && starts_a_line {
                    self.draw_line_number(quads, source_line + 1, top_of_row, at);
                }

                // Behind the glyphs, so the text stays readable through it.
                if !selected.is_empty() {
                    if let Some((from, to)) =
                        Self::row_selection(&cached, chunk, &frame.text, row, selected)
                    {
                        let (x, y) = content.to_window(from, top_of_row, scroll);
                        quads.push(Self::solid(
                            [
                                x * scale,
                                y * scale,
                                (to - from).max(1.0) * scale,
                                line.height * scale,
                            ],
                            self.theme.selection,
                        ));
                    }
                }

                for glyph in &line.glyphs {
                    let Some(entry) = self.entry_for(glyph.key, scale) else {
                        continue;
                    };
                    let colour =
                        view::colour_at(&runs, cached.rebase(glyph, chunk.span), self.theme.text);
                    let (x, y) = content.to_window(glyph.pos.x, glyph.pos.y + chunk_top, scroll);
                    quads.push(QuadInstance {
                        rect: [
                            x * scale + px_i(entry.left),
                            y * scale + px_i(entry.top),
                            px(entry.width),
                            px(entry.height),
                        ],
                        uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                        colour: [colour.r, colour.g, colour.b, colour.a],
                    });
                }
            }
        }
        line_height
    }

    /// Which source line a visual row belongs to.
    ///
    /// Asked of the glyphs rather than by counting newlines, and that distinction is the whole
    /// point: a wrapped paragraph is one source line over several rows, so counting newlines would
    /// number every row of it and the numbers would stop matching the file — which is the only
    /// reason to show them.
    ///
    /// A row with no glyphs is a blank line, which is never wrapped, so it is one line on from
    /// whatever was above it.
    fn line_of_row(
        &self,
        cached: &Cached,
        chunk: blocks::Chunk,
        line: &mdedit_text::LayoutLine,
        previous: usize,
    ) -> usize {
        match line.glyphs.first() {
            Some(glyph) => {
                let at = cached.rebase(glyph, chunk.span);
                self.document()
                    .rope()
                    .byte_to_line(at.get().min(self.document().len_bytes()))
            }
            None if previous == usize::MAX => self
                .document()
                .rope()
                .byte_to_line(chunk.span.start().get().min(self.document().len_bytes())),
            None => previous + 1,
        }
    }

    /// The ten digits, laid out once.
    ///
    /// Laying out a number per visible row is thirty-odd trips through the text engine every frame —
    /// on Windows, thirty `CreateTextLayout` calls — and it measured at p50 2.03 → 2.22 ms with p99
    /// 5.10 → 6.10 on a 245 KB document. Inside budget, but a fifth of the tail for something that
    /// draws at most five characters. There are only ten digits, and UI faces make them the same
    /// width, so one layout answers every number the editor will ever show.
    fn digits(&mut self) -> Option<[(GlyphKey, f32); 10]> {
        if let Some(digits) = self.digits {
            return Some(digits);
        }
        let style = TextStyle::body(self.theme.body_size);
        let run = [StyledRun {
            text: "0123456789",
            style,
            source: ByteSpan::empty_at(ByteOffset::ZERO),
        }];
        let layout = self.text.layout(&run, f32::MAX).ok()?;

        let measured: Vec<(GlyphKey, f32)> = layout
            .glyphs()
            .map(|glyph| (glyph.key, glyph.advance))
            .collect();
        // Ten glyphs for ten characters, unless a backend did something surprising — in which case
        // fall back rather than mis-render, since drawing numbers is not worth a panic.
        let digits: [(GlyphKey, f32); 10] = measured.try_into().ok()?;
        self.digits = Some(digits);
        Some(digits)
    }

    /// Draws one line number, right-aligned in the gutter.
    ///
    /// Composed from the cached digits rather than laid out: see [`digits`](Self::digits).
    fn draw_line_number(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        number: usize,
        top: f32,
        at: Placement,
    ) {
        let Some(digits) = self.digits() else {
            return;
        };

        // Written back to front into a fixed buffer, so a line number costs no allocation. Twenty
        // digits is more than a usize can have.
        let mut buffer = [0usize; 20];
        let mut count = 0;
        let mut left = number;
        loop {
            buffer[count] = left % 10;
            count += 1;
            left /= 10;
            if left == 0 {
                break;
            }
        }

        let advance = digits[0].1;
        let (_, y) = at.content.to_window(0.0, top, at.scroll);
        // Right-aligned against the *gutter's* right edge, less its padding — not against where the
        // text starts, which is what MDEDIT-34 was. `left_edge` is the gutter plus the document's
        // margin, so aligning to it drew the number past the gutter entirely and hard up against the
        // text.
        //
        // The width is a whole number of digit columns plus a padding either side, so a number that
        // fills it sits exactly centred, and a shorter one is right-aligned within that column —
        // which keeps the ones digits in line as the document passes ten and a hundred. Centring
        // each number on its own would break that.
        let right = self.gutter_width() - GUTTER_PADDING;
        #[allow(
            clippy::cast_precision_loss,
            reason = "a line number with more digits than an f32 can count is not reachable"
        )]
        let width = advance * count as f32;

        for (place, digit) in buffer[..count].iter().enumerate() {
            let (key, _) = digits[*digit];
            let Some(entry) = self.entry_for(key, at.scale) else {
                continue;
            };
            #[allow(
                clippy::cast_precision_loss,
                reason = "a line number with more digits than an f32 can count is not reachable"
            )]
            let from_right = (count - place - 1) as f32;
            let x = right - width + advance * from_right;
            quads.push(QuadInstance {
                rect: [
                    x * at.scale + px_i(entry.left),
                    y * at.scale + px_i(entry.top),
                    px(entry.width),
                    px(entry.height),
                ],
                uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                // The same dimmed colour as revealed syntax: scaffolding, not content.
                colour: [
                    self.theme.marker.r,
                    self.theme.marker.g,
                    self.theme.marker.b,
                    self.theme.marker.a,
                ],
            });
        }
    }

    /// Lays a chunk out, from the cache when it is there and from the text engine when it is not.
    ///
    /// A chunk holding the caret is laid out fresh and not stored: what it draws depends on where in
    /// it the caret sits, which is not in the key, so caching it would serve the wrong glyphs to the
    /// next keystroke.
    fn lay_out(&mut self, index: usize, frame: &Frame) -> Option<Rc<Cached>> {
        if let Some(cached) = frame.fresh_at(index) {
            return Some(cached);
        }
        let chunk = frame.chunks[index];
        let key = frame.keys[index];
        let revealed = touches(chunk.span, frame.selection.span());
        if !revealed {
            if let Some(cached) = self.layouts.get(&key) {
                return Some(Rc::clone(cached));
            }
        }

        let body = chunk.span.slice(&frame.text);
        let runs = view::build_runs_in(
            self.document(),
            frame.selection,
            self.decoration(),
            chunk.span,
            &frame.text,
        );
        let styled = view::as_styled(&runs);
        let layout = self.text.layout(&styled, frame.width).ok()?;

        let cached = Rc::new(Cached::new(layout, &chunk, body, key.last));
        if !revealed {
            // Layouts are the memory-hungry half — a glyph apiece — so they are capped and the
            // whole lot dropped when the cap is hit. Heights are a float each and worth keeping:
            // they are what stops the document's length jumping around as it is scrolled.
            if self.layouts.len() >= MAX_CACHED_LAYOUTS {
                self.layouts.clear();
            }
            self.layouts.insert(key, Rc::clone(&cached));
            self.heights.insert(key, cached.height);
        }
        Some(cached)
    }

    /// What a chunk's height is before it has been measured.
    ///
    /// Counts the lines it must occupy and multiplies by what a line of its kind costs. Wrapping
    /// makes this an under-estimate for a long paragraph, which is why it is only ever used for
    /// chunks that are off screen: an estimate is never what anybody looks at.
    fn estimate(&self, frame: &Frame, index: usize) -> f32 {
        let chunk = frame.chunks[index];
        let body = chunk.span.slice(&frame.text);
        let is_last = index + 1 == frame.chunks.len();
        let size = match chunk.kind {
            Some(BlockKind::Heading(level)) => self.theme.heading_size(level),
            _ => self.theme.body_size,
        };
        #[allow(
            clippy::cast_precision_loss,
            reason = "a line count large enough to lose precision is a document nobody has"
        )]
        let lines = blocks::line_count(body, is_last) as f32;
        lines * size * ESTIMATED_LINE_RATIO
    }

    /// Which chunk the caret is in, and where in that chunk it sits.
    ///
    /// The line is found by counting newlines rather than by hunting for a glyph, so a caret on a
    /// blank line lands on the blank line instead of jumping to the next thing with ink on it.
    fn caret_position(&self, frame: &Frame) -> Option<(usize, CaretRect)> {
        let caret = self.document().selection().head();
        let index = frame
            .chunks
            .iter()
            .position(|chunk| chunk.span.start() <= caret && caret < chunk.span.end())
            .or_else(|| frame.chunks.len().checked_sub(1))?;
        let chunk = frame.chunks[index];

        let cached = frame
            .fresh_at(index)
            .or_else(|| self.layouts.get(&frame.keys[index]).map(Rc::clone))?;

        let body = chunk.span.slice(&frame.text);
        let local = caret
            .get()
            .saturating_sub(chunk.span.start().get())
            .min(body.len());
        let row = body[..local].bytes().filter(|byte| *byte == b'\n').count();
        let line = cached.layout.lines.get(row)?;

        let top: f32 = cached.layout.lines[..row].iter().map(|l| l.height).sum();
        let x = line
            .glyphs
            .iter()
            .find(|glyph| glyph.source.start() >= caret)
            .map(|glyph| glyph.pos.x)
            .or_else(|| line.glyphs.last().map(|glyph| glyph.pos.x + glyph.advance))
            .unwrap_or(0.0);

        Some((
            index,
            CaretRect {
                x,
                top,
                height: line.height.max(1.0),
            },
        ))
    }

    /// Keeps the view still when a chunk above it turns out to be taller than its estimate.
    ///
    /// Only applied when nothing has moved the offset since the anchor was taken. Scrolling is the
    /// user's, and correcting for a measurement must not undo it.
    fn hold_anchor(&mut self, frame: &Frame, visible: f32) {
        let Some(anchor) = self.anchor else { return };
        if (self.scroll.offset() - anchor.offset).abs() > f32::EPSILON {
            return;
        }
        let Some(index) = frame
            .keys
            .iter()
            .position(|key| key.content == anchor.content)
        else {
            return;
        };
        self.scroll.set(
            frame.tops[index] + anchor.into_chunk,
            frame.content,
            visible,
        );
    }

    /// Records what the top of the viewport is resting on, for the next frame to hold it there.
    fn take_anchor(&mut self, frame: &Frame, offset: f32) {
        let index = frame
            .tops
            .partition_point(|top| *top <= offset)
            .saturating_sub(1);
        self.anchor = frame.keys.get(index).map(|key| Anchor {
            content: key.content,
            into_chunk: offset - frame.tops[index],
            offset,
        });
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

        let quads = self.build_frame(px(size.width), px(size.height), scale);

        // Told every frame rather than once: the caret moves, and an input method that is not told
        // where it is puts its candidate list at the window's origin.
        if let Some((x, y, height)) = self.caret_area {
            window.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(x, y),
                winit::dpi::LogicalSize::new(CARET_WIDTH, height),
            );
        }

        // Sampled before the GPU is touched. Under `Fifo`, `present` waits for the next vblank, so
        // once a frame is fast the number below is a refresh interval however little work was done —
        // it says nothing about whether the work got cheaper. This one does.
        if let Some(started) = self.pending_keystroke {
            self.build.record(started.elapsed());
        }

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if self.atlas.is_dirty() {
            renderer.upload_atlas(&self.atlas);
            self.atlas.mark_clean();
        }
        renderer.draw(&quads, self.theme.background);
        self.first_paint.get_or_insert_with(Instant::now);

        // Sampled after `present` returns, which is the last moment this process controls. See
        // `latency.rs` for exactly what that does and does not include.
        if let Some(started) = self.pending_keystroke.take() {
            self.latency.record(started.elapsed());
        }
    }
}

impl<T: TextLayout> ApplicationHandler<Command> for Editor<T> {
    /// A menu item was chosen.
    ///
    /// The menu delivers on a channel of its own, forwarded into this loop — see
    /// [`mdedit_menu_win::MenuBar::forward_to`]. Every arm is a command that already exists and is
    /// already tested, which is what makes the menu a surface rather than a second implementation.
    fn user_event(&mut self, event_loop: &ActiveEventLoop, command: Command) {
        if self.apply_command(command) {
            self.follow_caret = true;
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }

        // Closing the last tab leaves nothing to edit, which is what the window was for. Decided
        // here rather than in `apply_command`, because only the loop can end itself.
        if command == Command::CloseAll && self.workspace.tabs().is_empty() {
            event_loop.exit();
        }
    }

    /// Brings the parse up to date once the typing has stopped.
    ///
    /// The keystroke path re-parses only the region around an edit, which is what makes a keystroke
    /// in a large document cost what a keystroke in a small one does. The whole-document parse it
    /// skips happens here instead, on the timer the last keystroke set — off the path where anyone
    /// is waiting for it.
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if !matches!(cause, StartCause::ResumeTimeReached { .. }) {
            return;
        }
        event_loop.set_control_flow(ControlFlow::Wait);

        let reparsed = self.document_mut().reconcile();
        if reparsed {
            // The parse changed its mind about something, so anything derived from it is suspect.
            // Rare — see `Document::reconcile` — and the alternative is drawing a stale answer.
            self.layouts.clear();
            self.heights.clear();
        }

        // The other thing too expensive to do per keystroke. It shares this timer rather than having
        // its own, so typing schedules one wake-up rather than two.
        let before = self.words;
        self.recount_words();

        if reparsed || before != self.words {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }

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
        // Without this winit never delivers `WindowEvent::Ime` at all, and a Japanese, Chinese or
        // Korean keyboard types nothing into the editor.
        window.set_ime_allowed(true);
        self.attach_menu(&window);
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
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor());
                // Stored in device-independent pixels, which is the space the strip is laid out in.
                // Truncation is fine: these are screen coordinates, orders of magnitude inside
                // what f32 represents exactly.
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "cursor positions are small screen coordinates"
                )]
                let point = ((position.x / scale) as f32, (position.y / scale) as f32);
                self.cursor = point;

                // Dragging extends the selection from wherever the press landed.
                if self.dragging {
                    if let Some(offset) = self.offset_at_point(point.0, point.1) {
                        self.select(offset, true);
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel(delta);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.dragging = false,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let (x, y) = self.cursor;
                let changed = match button {
                    MouseButton::Left => self.click(x, y),
                    // Middle-click closes, as every editor with tabs does.
                    MouseButton::Middle => {
                        match (
                            tabs::hit_test(&self.tab_rects, x, y),
                            pane::hit_test(&self.row_rects, x, y),
                        ) {
                            (Some(tabs::Hit::Select(index) | tabs::Hit::Close(index)), _)
                            | (_, Some(pane::Hit::Select(index) | pane::Hit::Close(index))) => {
                                self.close_tab_at(index)
                            }
                            (None, None) => false,
                        }
                    }
                    _ => false,
                };
                if changed {
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::Ime(ime) => {
                if self.handle_ime(ime) {
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
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
                // An edit into a large document re-parses only the region around it, which cannot
                // see what the rest of the document does to that region. Come back when the typing
                // stops and parse the whole thing.
                if self.document().is_scoped() {
                    event_loop
                        .set_control_flow(ControlFlow::WaitUntil(Instant::now() + RECONCILE_AFTER));
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
}

/// The caret's place in the laid-out document, in device-independent pixels.
#[derive(Debug, Clone, Copy)]
struct CaretRect {
    x: f32,
    top: f32,
    height: f32,
}

/// Where a chunk lands on screen: how far down from the document origin, and at what device scale.
#[derive(Debug, Clone, Copy)]
struct Placement {
    /// Where the document may draw, and the only thing that converts between its coordinates and
    /// the window's.
    content: ContentRect,
    /// How far down the document the viewport has moved.
    ///
    /// With the rectangle, this gives the band of the document that is on screen — which is what
    /// lines are culled against. A chunk can be far taller than the window, since a document with
    /// no blank line in it is a single chunk, so per-line culling is needed as well as per-chunk.
    scroll: f32,
    scale: f32,
}

/// One frame's worth of measurement: what the chunks are and where each one sits.
///
/// Built once per frame and passed around rather than recomputed, because every step needs the same
/// answers and the buffer is only worth materializing once.
struct Frame {
    chunks: Vec<blocks::Chunk>,
    keys: Vec<ChunkKey>,
    /// Chunks laid out this frame rather than taken from the cache — the caret's, always.
    fresh: Vec<(usize, Rc<Cached>)>,
    heights: Vec<f32>,
    tops: Vec<f32>,
    /// The whole document's height, including the bottom margin.
    content: f32,
    text: String,
    selection: Selection,
    /// The wrapping width.
    width: f32,
}

impl Frame {
    fn fresh_at(&self, index: usize) -> Option<Rc<Cached>> {
        self.fresh
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, cached)| Rc::clone(cached))
    }
}

/// What identifies a laid-out chunk.
///
/// Deliberately **not** [`mdedit_offsets::BlockId`], and not the chunk's position: both are indices
/// into one analysis, so inserting a character in the first paragraph would renumber every block
/// after it and the cache would hand back the wrong layout for the whole document below an edit.
/// The content is the identity — the same bytes, of the same kind, at the same width, lay out the
/// same way wherever they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ChunkKey {
    content: u64,
    /// The wrapping width, as bits: a resize invalidates every layout, and rightly.
    width: u32,
    /// Whether this is the document's last chunk, which decides whether its trailing empty line is
    /// a line of its own or the start of the next chunk.
    last: bool,
    /// The same bytes lay out differently concealed and revealed, so the mode is part of what
    /// identifies a layout. In the key rather than clearing the cache on a toggle, so switching back
    /// and forth is free after the first time.
    mode: view::Mode,
}

impl ChunkKey {
    fn of(chunk: &blocks::Chunk, body: &str, last: bool, width: f32, mode: view::Mode) -> Self {
        let mut hasher = DefaultHasher::new();
        chunk.kind.hash(&mut hasher);
        body.hash(&mut hasher);
        Self {
            content: hasher.finish(),
            width: width.to_bits(),
            last,
            mode,
        }
    }
}

/// A chunk that has been through the text engine.
#[derive(Debug)]
struct Cached {
    layout: Layout,
    /// Where in the buffer the text was when it was laid out. Glyphs carry absolute source spans,
    /// and the same bytes elsewhere in the document reuse this entry, so the difference has to be
    /// added back before a span means anything.
    base: usize,
    /// Whether the last line belongs to the next chunk rather than to this one.
    drops_last: bool,
    height: f32,
}

impl Cached {
    fn new(layout: Layout, chunk: &blocks::Chunk, body: &str, is_last: bool) -> Self {
        let drops_last = blocks::drops_last_line(body, is_last);
        let height = layout.height()
            - if drops_last {
                layout.lines.last().map_or(0.0, |line| line.height)
            } else {
                0.0
            };
        Self {
            layout,
            base: chunk.span.start().get(),
            drops_last,
            height,
        }
    }

    /// How many of the laid-out lines this chunk actually draws.
    fn drawn_lines(&self) -> usize {
        let lines = self.layout.lines.len();
        if self.drops_last {
            lines.saturating_sub(1)
        } else {
            lines
        }
    }

    /// A glyph's source offset, moved to where this chunk actually sits in the buffer.
    fn rebase(&self, glyph: &mdedit_text::PositionedGlyph, span: ByteSpan) -> ByteOffset {
        ByteOffset::new(span.start().get() + glyph.source.start().get().saturating_sub(self.base))
    }
}

/// Where the top of the viewport is resting, so a measurement below it does not move the page.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    /// The chunk's *content*, not its whole key.
    ///
    /// Deliberately: the key carries the drawing mode, so anchoring on it would lose the anchor at
    /// exactly the moment it is most wanted — switching between preview and plain changes every
    /// chunk's height at once, and without an anchor the reader would be thrown somewhere else in
    /// the document.
    content: u64,
    /// How far into that chunk the viewport's top edge sits.
    into_chunk: f32,
    /// The scroll offset when this was taken. If it has changed, the user has scrolled and the
    /// anchor is stale.
    offset: f32,
}

/// Whether two spans overlap or meet. A caret at a chunk's edge belongs to both sides of it.
fn touches(span: ByteSpan, other: ByteSpan) -> bool {
    span.start() <= other.end() && other.start() <= span.end()
}
