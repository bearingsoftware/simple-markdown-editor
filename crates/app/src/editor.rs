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

use crate::clicks::{Clicks, Granularity};
use crate::scrollbar::{self, Bar};
use mdedit_document::{Document, Selection};
use mdedit_language::Diagnostic;
use mdedit_markers::BlockKind;
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_session::Workspace;
use mdedit_session::recent::Recent;
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
use crate::clipboard::{Clipboard, HoldNothing};
use crate::config::Config;
use crate::content::{ContentRect, Insets, Offset};
use crate::field;
use crate::find;
use crate::gpu::{ATLAS_SIZE, Atlas, QuadInstance, SHADER};
use crate::hit;
use crate::ime::{Action, Composition};
use crate::latency::Latency;
use crate::pane;
use crate::prompt::{
    self, ChangeNothing, Conflict, ConflictPrompt, Conflicting, KeepEverything, Unsaved,
    UnsavedPrompt,
};
use crate::scroll::{self, Scroll};
use crate::status;
use crate::tabs;
use crate::toolbar;
use crate::view::Chrome;
use mdedit_document::format::{self, Inline};

/// Something that arrived from outside the event loop and has to be handled inside it.
///
/// Two things wake this loop without a key or a mouse behind them, and both arrive on winit's user
/// channel: a menu item, which muda raises on a channel of its own, and an Explorer activation,
/// which arrives on a COM call from `mdedit-shell-win` (MDEDIT-61).
///
/// **A second variant rather than a second `Command`.** A file list is not a menu item, and putting
/// one into `mdedit_menu_win::Command` would put paths into the crate that draws menus. The loop
/// carries the union; each half stays in the crate it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wake {
    /// The user chose a menu item.
    Menu(Command),
    /// Explorer handed over a selection: open every one of these, in this order.
    Open(Vec<PathBuf>),
}

/// The width chunks are laid out at when word wrap is off (MDEDIT-49).
///
/// A large finite number rather than infinity. It has to be finite because it goes into `ChunkKey`
/// and is compared and hashed there, and because a text engine handed an infinite maximum is
/// entitled to do arithmetic with it; it has to be large enough that nothing real reaches it. A
/// hundred thousand pixels is about twelve thousand characters on one line, which is past the point
/// where a line is a data file rather than prose.
const UNWRAPPED: f32 = 100_000.0;

/// How far one Shift+wheel notch pans sideways.
///
/// In pixels rather than characters, because a proportional font has no column to count. Roughly
/// three characters of body text, which matches the three lines a vertical notch moves.
const PAN_PER_NOTCH: f32 = 48.0;

/// How much room the horizontal reveal leaves either side of the caret.
///
/// A caret revealed hard against the right edge has nothing after it to read, which is exactly the
/// position somebody typing at the end of a long line is in.
const PAN_MARGIN: f32 = 40.0;

/// The status bar's right-hand fields.
const RIGHT: usize = 0;
/// The status bar's breadcrumb.
const TRAIL: usize = 1;
/// The status bar's diagnostic count, which is its own slot because it is its own colour.
const COUNT: usize = 2;

/// One thing the recovery snapshotter decided to do, gathered before any of it is done.
///
/// Gathered first because writing borrows the store and the bookkeeping mutably while the tab list
/// is still borrowed to read from.
/// What checking a tab against its file concluded. See [`Editor::check_one`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checked {
    /// The file is as it was read, or there was no file to check.
    Nothing,
    /// The buffer now holds what is on disk.
    Reloaded,
    /// The file changed and the buffer was kept. On a save, that means not writing.
    KeptMine,
    /// The file changed and the user chose to write over it anyway.
    Overwrite,
}

enum Snapshotting {
    /// This buffer has unsaved work at this revision, and here it is.
    Write(u64, Option<PathBuf>, u64, String),
    /// This buffer is safe on disk; its snapshot should go.
    Forget(u64),
}

/// Where one of the find bar's text boxes is being drawn.
#[derive(Clone, Copy)]
struct TermAt {
    /// The box: left edge, then width.
    box_: (f32, f32),
    /// Which of the bar's fields it holds, which is also what decides whether it shows a caret.
    which: find::Focus,
    baseline: f32,
}

/// One visual row being drawn, gathered so a helper can be given it rather than seven arguments.
#[derive(Clone, Copy)]
struct Row<'a> {
    cached: &'a Cached,
    chunk: blocks::Chunk,
    /// The whole document's text, which is what chunk spans index into.
    text: &'a str,
    /// Which row of the chunk's layout.
    index: usize,
    /// Where the row's top edge is, in document coordinates.
    top: f32,
    height: f32,
    /// Where the chunk's top edge is, which is what layout positions are relative to.
    chunk_top: f32,
}

/// One whitespace mark, laid out once so it can be stamped wherever it is needed.
///
/// The advance is kept so the mark can be *centred* in the cell of the character it stands for,
/// which is what makes a middot look like it belongs to the space rather than to the letter beside
/// it. It is never used to advance a pen: the layout decides where everything goes.
#[derive(Debug, Clone, Copy)]
struct Mark {
    key: mdedit_text::GlyphKey,
    advance: f32,
}

/// Where a mark goes, and in what.
///
/// One parameter rather than four, for the reason [`LabelAt`] gives: they always travel together,
/// and a row of positional `f32`s is the shape a caller silently transposes.
#[derive(Debug, Clone, Copy)]
struct MarkAt {
    /// In the document's coordinates, before the content rect has its say.
    pos: (f32, f32),
    colour: Rgba,
    at: Placement,
}

/// A short string of chrome, laid out and kept until it changes.
#[derive(Debug, Clone)]
struct Label {
    text: String,
    glyphs: Vec<mdedit_text::PositionedGlyph>,
    width: f32,
}

/// Where a laid-out label goes, and in what.
///
/// One parameter rather than four, because they always travel together and four positional `f32`s
/// in a row is the shape a caller silently transposes.
#[derive(Debug, Clone, Copy)]
struct LabelAt {
    x: f32,
    baseline: f32,
    scale: f32,
    colour: Rgba,
}
use crate::view::{self, Rgba, Theme};
use mdedit_menu_win::{Command, MenuBar};

/// Asks the user where to save something. Given the current name as a suggestion; `None` if they
/// decline, or if there is nobody to ask.
pub type SavePathPicker = Box<dyn Fn(Option<&str>) -> Option<PathBuf>>;

/// Asks which files to open. Returns `None` when the dialog was dismissed, and an empty list is the
/// same thing said differently.
pub type OpenPathPicker = Box<dyn Fn() -> Option<Vec<PathBuf>>>;

/// Left and top margin, in device-independent pixels.
const MARGIN: f32 = 24.0;
/// Fewest digits the gutter is sized for, so a short file does not visibly resize its gutter the
/// moment it grows past nine lines.
const GUTTER_MIN_DIGITS: usize = 2;
/// Space either side of the numbers in the gutter.
const GUTTER_PADDING: f32 = 8.0;
/// Caret width, in device-independent pixels.
const CARET_WIDTH: f32 = 2.0;

/// How much smaller a formatting-bar label is than body text.
///
/// Shared by the fixed bar and the floating one so their cached labels are the same strings at the
/// same size — one cache serves both only while that holds.
const TOOLBAR_TEXT: f32 = 0.62;

/// How thick the floating bar's outline is.
const FLOAT_BORDER: f32 = 1.0;

/// How far the find bar's text box is inset from the bar around it.
const FIND_INSET: f32 = 4.0;

/// The longest selection Ctrl+F carries into the search box.
///
/// A whole paragraph flattened onto one line is not a search term, and putting one there means the
/// first thing the chord does is something the user has to undo. Above this the previous term is
/// kept instead.
const FIND_SEED_LIMIT: usize = 120;
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "the window's state, and its flags are genuinely independent of each other — whether a \
              drag is in progress says nothing about whether the formatting bar is showing. The \
              state machine the lint suggests would have to enumerate their product."
)]
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
    /// Whether the formatting bar is showing. Per-run: MDEDIT-52 left the setting to the settings
    /// story, and Ctrl+B-era per-run chrome is the precedent.
    toolbar: bool,
    /// The bar's buttons as laid out last frame, for the click chain.
    toolbar_buttons: Vec<toolbar::ButtonRect>,
    /// Each button's glyph, laid out once. The fourth cache of this kind — see `crate::toolbar`.
    toolbar_labels: HashMap<String, Label>,
    /// Whether the bar that follows a selection is wanted.
    ///
    /// Wanted, not shown: it appears only when there is also a selection to hang it on and that
    /// selection's first row is on screen. The two are separate because the *rules* are about
    /// wanting — a drag clears it, a settled selection sets it — and the geometry is about whether
    /// there is anywhere to put it.
    floating: bool,
    /// The floating bar's buttons as placed last frame, in window coordinates. Empty when it is not
    /// showing, which is also what makes the click chain a no-op then.
    floating_buttons: Vec<toolbar::ButtonRect>,
    /// Where the floating bar was drawn last frame and how wide, for the click chain.
    floating_at: Option<(f32, f32, f32)>,
    /// Which tab the bar settled on.
    ///
    /// Switching tabs has to take it away — the new document has its own selection, and a bar that
    /// survived the switch would be pointing at the last one. Held rather than hooked into every
    /// path that activates a tab, of which there are eight: a comparison here cannot be forgotten
    /// by the ninth.
    floating_for: Option<usize>,
    /// The first *visible* row of the selection, in window coordinates, recorded while drawing it.
    ///
    /// The selection's rectangles are otherwise transient — `row_selection` returns x-extents that
    /// go straight into a quad — so the one the floating bar anchors to is kept here as `caret_area`
    /// is kept for the IME. `None` when nothing is selected or the selection has scrolled off, and
    /// the bar hides in both cases.
    selection_anchor: Option<toolbar::Anchor>,
    /// The find bar, when it is open. `None` is closed, and there is no other flag for it.
    ///
    /// Kept on the editor rather than per tab: a search is a thing you are doing, not a property of
    /// a document, and every editor lets you carry a term from one file to the next. What *is* per
    /// tab is the match list, which is why switching tab invalidates it.
    find: find::Bar,
    /// Whether it is showing. Separate from the bar itself so closing it keeps the term and the
    /// case flag — Esc then Ctrl+F then Enter repeats the last search, which is what every editor
    /// does and what a bar thrown away on close cannot do.
    find_open: bool,
    /// Whether keys go to the find field rather than to the document.
    ///
    /// The whole focus model. Exactly one of the two has it, and with the bar closed it is always
    /// the document — so this is only ever consulted alongside `find`.
    find_focused: bool,
    /// Where the bar's parts landed last frame, for the click chain. `None` when it is closed.
    find_layout: Option<find::Layout>,
    /// Whether the selection was made by landing on a search match rather than by the user.
    ///
    /// The floating formatting bar arms on a settled selection, and a match is one by that rule — so
    /// without this every press of Find Next put a bar over the prose. Held on the editor rather
    /// than keyed off whether the find bar is open, because the selection outlives the bar: Esc
    /// closes it and leaves the match selected, and a bar appearing *then* would be chrome arriving
    /// from the gesture that dismisses chrome.
    found_selection: bool,
    /// Where the open files are shown. See [`Chrome`].
    chrome: Chrome,
    /// The pane's visible rows, kept for the next click, as `tab_rects` is for the strip.
    row_rects: Vec<pane::RowRect>,
    /// How far the pane's list has been scrolled. Its own, not the document's — the two scroll
    /// independently, which is the point of a pane that can hold more files than fit.
    pane_scroll: f32,
    /// Where unsaved work is kept against a crash. `None` when there is nowhere to put it.
    snapshots: Option<mdedit_session::snapshot::Store>,
    /// The document revision each snapshot was taken at, so an unchanged buffer is not rewritten.
    snapshotted: HashMap<u64, u64>,
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
    /// Where the scrollbar was last frame, or `None` when the document fits. See
    /// [`crate::scrollbar`].
    scrollbar: Option<Bar>,
    /// How far into the thumb the pointer was when a thumb drag started, so the thumb does not jump
    /// to centre itself under the pointer on the first press.
    thumb_grab: Option<f32>,
    /// Counts presses into double- and triple-clicks. See [`crate::clicks`].
    clicks: Clicks,
    /// What one click currently selects: a position, a word, or a line.
    granularity: Granularity,
    /// The unit the drag started from, so extending grows by the same unit in both directions.
    anchor_span: Option<ByteSpan>,
    /// Whether the pointer is dragging out a selection.
    dragging: bool,
    /// Asks what to do when a file changed on disk under a modified buffer. See [`crate::prompt`].
    conflict: Box<dyn ConflictPrompt>,
    /// Asks about unsaved work when a tab is closing. See [`crate::prompt`].
    prompt: Box<dyn UnsavedPrompt>,
    /// Asks where to save something that has nowhere to go. Declines by default.
    pick_save_path: SavePathPicker,
    /// Asks which files to open. Opens nothing by default.
    pick_open_paths: OpenPathPicker,
    /// Shows the About dialog. Does nothing by default — the dialog lives in the binary.
    show_about: Box<dyn Fn()>,
    /// Tells the user something they asked for cannot be done, and why. Says nothing by default.
    ///
    /// Only for a refusal that is a direct answer to something the user just chose from a menu —
    /// Format Document on a document that is not valid JSON is the whole of it so far. Anything the
    /// editor merely *noticed* belongs in the log, which is where every other soft error goes.
    say: Box<dyn FnMut(&str)>,
    /// The files opened lately, for the Open Recent submenu.
    recent: Recent,
    /// Whether spaces, tabs, control characters and line endings are drawn (MDEDIT-46).
    ///
    /// **Per-run, and deliberately not remembered**, for the same reason the drawing mode is not:
    /// it is a way of looking at a document for a moment — where did that trailing space come from,
    /// why is this line CRLF — rather than a preference. An editor that opened showing middots
    /// because you checked something last week would be wrong.
    whitespace: bool,
    /// The marks, laid out once each.
    ///
    /// A fifth thing holding laid-out text, so it is a sixth thing `set_zoom` has to clear. The
    /// *layout* caches need nothing, which is the point of drawing in place: see `draw_marks`.
    marks: HashMap<char, Option<Mark>>,
    /// Whether long lines wrap, or run off to the right (MDEDIT-49).
    ///
    /// **Per-run, like the drawing mode**, and for the same reason: it is a way of looking at a
    /// document rather than a preference about it.
    wrap: bool,
    /// How far right the viewport has panned. Always zero while [`Self::wrap`] is on.
    ///
    /// A second [`Scroll`], not a second kind of arithmetic: every one of its methods takes the
    /// content and viewport extents as parameters, so the type never knew which axis it was on.
    pan: Scroll,
    /// The widest row laid out so far, for the horizontal clamp.
    ///
    /// **Measured, not estimated, and therefore incomplete.** Only chunks that have been laid out
    /// have a width to contribute, so a document nobody has scrolled to the bottom of may have a
    /// longer line further down — exactly the position the vertical axis is in with its estimated
    /// heights. It grows as chunks are measured, and is cleared with the layout caches.
    widest: f32,
    /// Concealed, or shown as it is. See [`crate::view::Mode`].
    mode: view::Mode,
    /// What the user chose. Read once at startup; see [`crate::config`].
    config: Config,
    /// The settings file and the write path onto it. See [`crate::config`].
    settings: crate::config::Settings,
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
    status_labels: [Option<Label>; 3],
    /// Which tab [`Self::words`] was counted from.
    ///
    /// Tracked rather than cleared at each of the nine places that activate a tab, because a count
    /// that belongs to the tab you just left is not stale, it is wrong — and one forgotten call site
    /// is a bug nobody would see until they switched files.
    counted_for: Option<usize>,
    /// What is wrong with the active document, as of the last idle timer (MDEDIT-47).
    ///
    /// Empty for every language but JSON, and empty for valid JSON.
    diagnostics: Vec<Diagnostic>,
    /// Which document, at which revision, [`Self::diagnostics`] describes.
    ///
    /// **This pair is how the diagnostics are cleared on an edit**, and it is deliberately not a
    /// hook on the edit paths. Same shape and same reason as `find`'s match list and `counted_for`:
    /// a revision alone would not do, because two tabs nobody has typed in are both at revision
    /// zero. A stale set is not merely out of date — the menu item moves the caret to one of these
    /// spans, so a span from before an edit points somewhere that has moved.
    diagnosed_for: Option<(usize, u64)>,
    /// What the Tools menu was last told, so it is only told again when the answer changes.
    ///
    /// **Compared rather than hooked.** Which document is active changes on a great many paths, and
    /// a list of call sites to update is a list somebody forgets to add to — `counted_for` and
    /// `floating_for` are the same shape for the same reason. This one has to push rather than pull,
    /// because the menu is a native object, so the comparison is what keeps the pushing rare.
    tools_state: Option<mdedit_menu_win::ToolsState>,
    /// What the Line Endings submenu was last told, for the same reason.
    endings_state: Option<mdedit_menu_win::EndingsState>,
    /// What the encoding submenus were last told, for the same reason.
    encoding_state: Option<mdedit_menu_win::EncodingState>,
    /// The native menu bar, once there is a window to hang it on.
    menu: Option<MenuBar>,
    /// How menu commands get from muda's channel into this loop. Set before the window exists.
    menu_proxy: Option<EventLoopProxy<Wake>>,
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
            toolbar: true,
            toolbar_buttons: Vec::new(),
            toolbar_labels: HashMap::new(),
            floating: false,
            floating_buttons: Vec::new(),
            floating_at: None,
            floating_for: None,
            selection_anchor: None,
            find: find::Bar::default(),
            find_open: false,
            find_focused: false,
            find_layout: None,
            found_selection: false,
            chrome: Chrome::Strip,
            row_rects: Vec::new(),
            pane_scroll: 0.0,
            row_labels: HashMap::new(),
            // Nowhere, until somebody says where. See `with_snapshot_store`, and MDEDIT-68 for what
            // defaulting this to the real directory cost.
            snapshots: None,
            snapshotted: HashMap::new(),
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
            scrollbar: None,
            thumb_grab: None,
            clicks: Clicks::default(),
            granularity: Granularity::default(),
            anchor_span: None,
            dragging: false,
            content: ContentRect::default(),
            clipboard: Box::new(HoldNothing),
            prompt: Box::new(KeepEverything),
            conflict: Box::new(ChangeNothing),
            pick_save_path: Box::new(|_| None),
            pick_open_paths: Box::new(|| None),
            show_about: Box::new(|| {}),
            say: Box::new(|_| {}),
            recent: Recent::default(),
            wrap: true,
            pan: Scroll::new(),
            widest: 0.0,
            whitespace: false,
            marks: HashMap::new(),
            mode: view::Mode::default(),
            config: Config::default(),
            settings: crate::config::Settings::nowhere(),
            digits: None,
            words: None,
            status_labels: [None, None, None],
            counted_for: None,
            diagnostics: Vec::new(),
            diagnosed_for: None,
            tools_state: None,
            endings_state: None,
            encoding_state: None,
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
        self.chrome = config.chrome;
        // Applied here rather than left for the first zoom press, so a document opens at the size it
        // was last read at.
        self.theme.body_size = Theme::default().body_size * config.zoom;
    }

    /// Uses these settings, and applies what they say.
    ///
    /// The settings own the write path, so this is how a change made in the editor ever reaches the
    /// file. Pointing it at a temporary one is also how a test avoids touching the user's.
    pub fn with_settings(&mut self, settings: crate::config::Settings) {
        self.with_config(settings.config());
        self.settings = settings;
    }

    /// The settings, for a caller that wants to know what is stored. For tests.
    #[must_use]
    pub fn settings(&self) -> &crate::config::Settings {
        &self.settings
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

    /// Checks every open file for an external change, and redraws if one turned up.
    fn on_focus(&mut self) {
        if self.check_disk() {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }

    /// Answers the file-changed-on-disk question. See [`with_prompt`](Self::with_prompt).
    pub fn with_conflict_prompt(&mut self, conflict: Box<dyn ConflictPrompt>) {
        self.conflict = conflict;
    }

    /// Notices files that changed on disk while the editor was not looking.
    ///
    /// Called on regaining window focus, which is when an external edit has almost always happened —
    /// somebody alt-tabbed away, ran a formatter or pulled a branch, and came back. Polling on the
    /// event rather than watching the filesystem is a deliberate limit: `notify` brings threads and
    /// four platform backends for a case that, in a Markdown editor, happens between focus changes.
    /// A file rewritten *while* the window stays focused is not noticed until the next save, which
    /// is the check that actually prevents data loss.
    ///
    /// Every open tab is checked, not only the visible one, because a background tab quietly holding
    /// stale bytes is how the wrong thing gets saved later.
    ///
    /// Returns whether anything changed, so the caller knows to redraw.
    pub fn check_disk(&mut self) -> bool {
        let mut changed = false;
        for index in 0..self.workspace.tabs().len() {
            changed |= self.check_one(index, Conflicting::Found) == Checked::Reloaded;
        }
        if changed {
            // Every cached layout is keyed on bytes that are no longer in the buffer.
            self.layouts.clear();
            self.heights.clear();
        }
        changed
    }

    /// Checks one tab against its file, asking if it has to.
    ///
    /// One function for both callers rather than two that drift, which is why the answer is richer
    /// than either needs on its own: the focus check acts on `Reloaded` and ignores the rest, while
    /// the save path treats `Nothing` and `Overwrite` as permission to write.
    fn check_one(&mut self, index: usize, why: Conflicting) -> Checked {
        let Some(tab) = self.workspace.tabs().get(index) else {
            return Checked::Nothing;
        };
        // Deferred or untitled: nothing has been read, so nothing can have gone stale under it.
        let (Some(path), Some(open)) = (tab.path(), tab.open()) else {
            return Checked::Nothing;
        };
        let Some(file) = open.file.as_ref() else {
            return Checked::Nothing;
        };

        let modified = tab.is_modified();
        let name = tab.display_name();
        match mdedit_file::disk_state(path, file) {
            mdedit_file::DiskState::Unchanged => Checked::Nothing,
            mdedit_file::DiskState::Gone => {
                // Not an error, and specifically not a reload: the buffer is now the only copy of
                // itself. Dropping the record of what was on disk makes `Tab::is_modified` fall
                // through to its untitled rule — anything in the buffer counts as unsaved — so the
                // tab behaves like an untitled one that happens to know where it wants to go, and
                // saving it writes the file back.
                self.workspace.forget_file(index);
                crate::log_error!("{name} has been deleted; its buffer is now the only copy of it");
                Checked::Nothing
            }
            mdedit_file::DiskState::Changed if !modified => {
                // Nothing to lose, so nothing to ask. What VS Code does, and right: with no local
                // edits the buffer is only a view of the file, and the file moved.
                self.reload(index, &name)
            }
            mdedit_file::DiskState::Changed => match self.conflict.ask(&name, why) {
                Conflict::Reload => self.reload(index, &name),
                Conflict::Overwrite => Checked::Overwrite,
                Conflict::Cancel => Checked::KeptMine,
            },
        }
    }

    /// Re-reads a tab, reporting a failure as having done nothing rather than as having reloaded.
    fn reload(&mut self, index: usize, name: &str) -> Checked {
        match self.workspace.reload_tab(index) {
            Ok(()) => {
                // Recorded even when it discarded nothing. A buffer changing under the user without
                // their asking is the sort of thing that gets reported as "it lost my work", and the
                // log is the only place that question can be answered afterwards.
                crate::log_error!("{name} changed on disk and was reloaded");
                Checked::Reloaded
            }
            Err(error) => {
                crate::log_error!("could not reload {name}: {error}");
                // The buffer is untouched and the stamp is unchanged, so the next check asks again.
                Checked::KeptMine
            }
        }
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

    /// Says where unsaved work is kept.
    ///
    /// **An editor is built with nowhere to keep it, and `main` installs the real store** — the same
    /// wiring as the clipboard, the prompts, the file pickers and the recent-files list, and for the
    /// same reason. MDEDIT-68 is what the other default cost: every test that edited a buffer and ran
    /// the idle timer wrote a snapshot into the user's own recovery directory, and MDEDIT-66 restored
    /// it into their editor on the next launch.
    pub fn with_snapshot_store(&mut self, store: mdedit_session::snapshot::Store) {
        self.snapshots = Some(store);
    }

    /// Whether this editor has anywhere to keep unsaved work.
    ///
    /// False until somebody says where, which is the whole of MDEDIT-68. Exists so a test can assert
    /// the *default* rather than go looking in the user's own directory for evidence of absence — a
    /// test that reads `%LOCALAPPDATA%` passes on a machine that has never run the editor, which
    /// makes it no test at all.
    #[must_use]
    pub fn stores_unsaved_work(&self) -> bool {
        self.snapshots.is_some()
    }

    /// The work that waits for typing to stop, and whether any of it changed what is drawn.
    ///
    /// Four deferred jobs share one timer rather than having four: the whole-document re-parse a
    /// scoped edit owes (MDEDIT-18), the word count, the recovery snapshot (MDEDIT-54) and the
    /// strict language check (MDEDIT-47). Public
    /// because a test cannot deliver a [`StartCause::ResumeTimeReached`] — the event loop does that
    /// — and every one of these is a claim worth testing.
    pub fn on_idle(&mut self) -> bool {
        let reparsed = self.document_mut().reconcile();
        if reparsed {
            // The parse changed its mind about something, so anything derived from it is suspect.
            // Rare — see `Document::reconcile` — and the alternative is drawing a stale answer.
            self.layouts.clear();
            self.heights.clear();
        }

        let before = self.words;
        self.recount_words();
        let endings = self.newline();
        self.recheck_newline();
        let diagnosed = self.diagnostic_count();
        self.revalidate();
        self.snapshot_dirty_buffers();
        // A selection extended with Shift and then left alone has settled. This timer already means
        // "the typing stopped", which is exactly the beat the bar is waiting for — a fourth thing on
        // it rather than a debounce of its own.
        let settled = self.settle_selection();
        // Debounced onto this timer rather than written on the keystroke that changed it, so a run
        // of rapid toggles costs one write. Not on exit — see `crate::config`.
        self.settings.flush();

        reparsed
            || before != self.words
            || settled
            || diagnosed != self.diagnostic_count()
            || endings != self.newline()
    }

    /// Re-detects the active document's line endings. On the idle timer, never on the keystroke path.
    ///
    /// # Why this has to be recomputed rather than remembered
    ///
    /// The endings were detected once, when the file was read, and then believed for ever. Two things
    /// make that a lie. **Undo**: MDEDIT-59's conversion sets the format, and undoing it puts the
    /// bytes back without putting the format back, so the bar went on claiming LF over a document
    /// that was Mixed again. And **pasting**, which is the older of the two — the editor deliberately
    /// preserves pasted CRLF in an LF file, and the bar has been saying "LF" about the result since
    /// MDEDIT-38.
    ///
    /// Deriving it from the buffer makes both self-correcting, at the cost of a walk of the buffer —
    /// which is why it is here, beside the word count, and not on any path a keystroke reaches.
    fn recheck_newline(&mut self) {
        let detected = mdedit_file::Newline::detect(&self.document().rope().to_string());
        if let Some(file) = self
            .workspace
            .active_open_mut()
            .and_then(|open| open.file.as_mut())
        {
            file.set_newline(detected);
        }
        self.refresh_endings_menu();
    }

    /// Writes a recovery snapshot for every buffer with unsaved work in it.
    ///
    /// On the idle timer, never on the keystroke path: this copies whole buffers to disk. A buffer
    /// whose revision has not moved since its last snapshot is skipped, which is the common case —
    /// the timer fires when typing *stops*, so usually exactly one buffer has changed.
    ///
    /// Caret and undo history are deliberately not saved. What a crash gives back is the text; a
    /// recovery that pretended more had survived than did would be worse than an honest one.
    fn snapshot_dirty_buffers(&mut self) {
        let Some(store) = self.snapshots.as_mut() else {
            return;
        };

        // Gathered first, because writing borrows the store and the bookkeeping mutably while the
        // tab list is borrowed to read. Only buffers that actually need writing are materialised,
        // which is the point of the revision check.
        let mut todo = Vec::new();

        for tab in self.workspace.tabs() {
            let id = tab.snapshot_id();
            let Some(open) = tab.open() else {
                continue;
            };

            if !tab.is_modified() {
                // Saved or reverted since the last tick: the snapshot now describes work that is
                // safely on disk, and leaving it would offer to restore it on the next start.
                if self.snapshotted.contains_key(&id) {
                    todo.push(Snapshotting::Forget(id));
                }
                continue;
            }

            let revision = open.document.revision();
            if self.snapshotted.get(&id) == Some(&revision) {
                continue;
            }
            todo.push(Snapshotting::Write(
                id,
                tab.path().map(Path::to_path_buf),
                revision,
                open.document.to_string(),
            ));
        }

        for item in todo {
            match item {
                Snapshotting::Write(id, path, revision, text) => {
                    if let Err(error) = store.write(id, path.as_deref(), &text) {
                        crate::log_error!("could not write a recovery snapshot: {error}");
                        continue;
                    }
                    self.snapshotted.insert(id, revision);
                }
                Snapshotting::Forget(id) => {
                    self.snapshotted.remove(&id);
                    if let Err(error) = store.forget(id) {
                        crate::log_error!("could not clear a recovery snapshot: {error}");
                    }
                }
            }
        }
    }

    /// Restores whatever the last run left behind, and says what it could not.
    ///
    /// Each recovered buffer replaces the text of its tab — opening it if it was only a handle — and
    /// arrives *modified*, so the editor's ordinary unsaved-changes machinery protects it and the
    /// user decides whether to keep it. Untitled buffers come back as untitled tabs.
    pub fn restore_unsaved_work(&mut self, after_clean_exit: bool) {
        let Some(store) = self.snapshots.as_mut() else {
            return;
        };
        let (recovered, skipped) = store.read_all();
        for reason in skipped {
            crate::log_error!("stored buffer skipped: {reason}");
        }
        if recovered.is_empty() {
            return;
        }

        let count = recovered.len();
        for entry in recovered {
            self.restore_one(&entry);
        }

        // The two cases deserve different words. Coming back to your own unsaved edits after an
        // ordinary quit is the feature working and needs no announcement; the same text appearing
        // after the process was killed is worth saying out loud, because the user did not choose it
        // and may be wondering what survived.
        if after_clean_exit {
            crate::log_error!("restored {count} unsaved buffer(s) from the last session");
        } else {
            crate::log_error!(
                "recovered {count} unsaved buffer(s) from a previous run that did not exit cleanly"
            );
        }
    }

    /// Puts one recovered buffer back, into the tab it belongs to.
    ///
    /// Recovery runs after the session has been restored, so the file it is recovering is usually
    /// *already* open. `Workspace::add` returns the existing tab for a path it already holds, which
    /// is what stops a recovered file appearing twice — once showing the recovered text and once
    /// showing what is on disk. Relied on rather than re-implemented here; pinned by a test, because
    /// it is a guarantee of `add` that this depends on rather than an accident.
    fn restore_one(&mut self, entry: &mdedit_session::snapshot::Recovered) {
        // A stored edit is only about the bytes it was taken from. If something else has written the
        // file since, reinstating it silently would put an edit back on top of somebody's work — the
        // same hazard MDEDIT-55 answers when the editor is running, answered here the same way.
        if !self.stored_edit_is_wanted(entry) {
            return;
        }

        let index = match &entry.path {
            Some(path) => {
                let index = self.workspace.add(path.clone());
                if self.workspace.activate(index).is_err() {
                    // The file has gone since. The text still matters, so it comes back as an
                    // untitled buffer rather than being dropped — which is what MDEDIT-55 already
                    // does for a file deleted under a live tab.
                    self.workspace.add_untitled()
                } else {
                    index
                }
            }
            None => self.workspace.add_untitled(),
        };

        if self.workspace.activate(index).is_err() {
            crate::log_error!("could not open a tab to recover into");
            return;
        }
        let Some(open) = self.workspace.active_open_mut() else {
            return;
        };
        let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(open.document.len_bytes()));
        if let Err(error) = open.document.replace(whole, &entry.text) {
            crate::log_error!("could not restore a recovered buffer: {error}");
        }
    }

    /// Whether a stored edit should be put back, asking if the file has moved on since.
    ///
    /// Three cases, and only the middle one asks:
    ///
    /// - **Nothing to compare against** — a scratch buffer, or a copy stored by a build older than
    ///   the stamp. Restored. A scratch buffer has no file to conflict with, and refusing to restore
    ///   an old copy would lose work over a missing field.
    /// - **The file has changed** — ask. Reinstating silently would put an edit back on top of
    ///   whatever else wrote it, which is the hazard MDEDIT-55 exists to prevent while the editor is
    ///   running; a copy that has been sitting on disk since the last session is if anything more
    ///   likely to be stale.
    /// - **Unchanged, or gone** — restored. Gone is not a conflict: the stored text is then the only
    ///   copy, and `restore_one` brings it back as a scratch buffer that remembers its path.
    fn stored_edit_is_wanted(&mut self, entry: &mdedit_session::snapshot::Recovered) -> bool {
        let (Some(path), Some(stamp)) = (entry.path.as_deref(), entry.stamp) else {
            return true;
        };
        if mdedit_file::matches_stamp(path, stamp) != mdedit_file::DiskState::Changed {
            return true;
        }

        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        // `Found` rather than `Saving`: nothing is being written, and the question is which version
        // to look at — which is exactly what that wording says.
        match self.conflict.ask(&name, Conflicting::Found) {
            // Take the stored edit, which is what restoring is.
            Conflict::Cancel => true,
            // Take what is on disk. The stored copy goes, or the same question would be asked on
            // every launch from now on — which is how a safeguard becomes a nuisance and then
            // becomes something people click through without reading.
            Conflict::Reload | Conflict::Overwrite => {
                crate::log_error!(
                    "{name} changed on disk; keeping the file rather than the stored edit"
                );
                if let Some(store) = self.snapshots.as_mut() {
                    if let Err(error) = store.forget(entry.id) {
                        crate::log_error!("could not remove the stored edit: {error}");
                    }
                }
                false
            }
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

    /// Writes the tab list and records that this run ended on purpose.
    ///
    /// Only the exit path calls this. The marker is what stops the next launch describing an
    /// ordinary restore as a crash recovery — see [`mdedit_session::last_run_ended_cleanly`].
    pub fn save_session_on_exit(&self, path: &Path) {
        if let Err(error) = self.workspace.save_session_on_exit(path) {
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

    /// The text size the theme is currently drawing at.
    ///
    /// For a test that cannot see pixels: the stub backend is a fixed grid and ignores the size it
    /// is handed, so the *effect* of a zoom is invisible to it and this is what can be asserted.
    #[must_use]
    pub fn body_size(&self) -> f32 {
        self.theme.body_size
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
            Ok(file) => {
                let bound = self
                    .workspace
                    .bind_to_path(index, path.to_path_buf(), file)
                    .is_ok();
                if bound {
                    // A tab now lives somewhere it has not been before, which is an open as far as
                    // the recent list is concerned.
                    self.remember(path);
                }
                bound
            }
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
        // The check that actually prevents data loss. The focus check keeps the buffer fresh and is
        // a convenience; this one stands between someone else's work and being written over, and it
        // is here rather than at the call sites so that every route to a save passes it — Ctrl+S,
        // the menu, Save All, and the save arm of the close prompt.
        //
        // Only when saving over the tab's *own* file: Save As to a different path is not this
        // situation, and a Save As onto an existing file is an overwrite the user just chose in a
        // dialog that asked.
        if self.workspace.active_path() == Some(path) {
            if let Some(index) = self.workspace.active() {
                match self.check_one(index, Conflicting::Saving) {
                    Checked::Nothing | Checked::Overwrite => {}
                    Checked::Reloaded => {
                        // The buffer now holds what is on disk, so there is nothing left to write —
                        // and writing would put the discarded edits back.
                        self.layouts.clear();
                        self.heights.clear();
                        return false;
                    }
                    Checked::KeptMine => return false,
                }
            }
        }

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
            // **Said, not only logged.** A save that fails silently is the worst outcome in this
            // editor: the user believes their work is on disk. MDEDIT-42 makes it reachable in the
            // ordinary course of things — choose an encoding that cannot hold the text and the save
            // is refused — so the seam MDEDIT-48 added for Format Document's refusal earns its keep
            // for every failure, not just that one.
            (self.say)(&format!("Could not save {}.\n\n{error}", path.display()));
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
            Command::Open => self.open_prompted(),
            Command::OpenRecent(index) => self.open_recent(index),
            Command::PlainMarkup => {
                self.toggle_mode();
                true
            }
            Command::FilePane => {
                self.toggle_chrome();
                true
            }
            Command::LineNumbers => {
                self.toggle_line_numbers();
                true
            }
            Command::Toolbar => {
                self.toggle_toolbar();
                true
            }
            Command::Find => self.open_find(),
            Command::FindNext => self.find_next(),
            Command::FindPrevious => self.find_previous(),
            Command::Replace => self.open_replace(),
            Command::GoToLine => self.open_goto(),
            Command::ZoomIn => self.zoom_by(1),
            Command::ZoomOut => self.zoom_by(-1),
            Command::ResetZoom => self.reset_zoom(),
            Command::Whitespace => self.toggle_whitespace(),
            Command::WordWrap => self.toggle_wrap(),
            Command::ReopenWith(index) => self.reopen_with(index),
            Command::SaveWith(index) => self.save_with(index),
            Command::ConvertToLf => self.set_newline(mdedit_file::Newline::Lf),
            Command::ConvertToCrLf => self.set_newline(mdedit_file::Newline::CrLf),
            Command::Validate => self.go_to_first_diagnostic(),
            Command::Format => self.format_document(),
            Command::Shortcuts => self.show_shortcuts(),
            Command::About => {
                (self.show_about)();
                false
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
            Command::SelectAll => {
                self.select_all();
                true
            }
        }
    }

    /// Puts the executable's embedded icon on the window (MDEDIT-70).
    ///
    /// Embedding the resource is not enough on its own: a window takes its icon from its class or
    /// from `WM_SETICON`, and winit sets neither, so without this the title bar and Alt-Tab draw a
    /// system fallback while the icon sits unused inside the binary. See
    /// [`mdedit_menu_win::set_window_icon`], which is where the Win32 call belongs.
    fn set_window_icon(window: &Window) {
        // Getting the handle is safe; only using it is not, and that lives in the platform crate.
        // Nested rather than a let-chain: those stabilised in 1.88 and this crate's floor is 1.87.
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                mdedit_menu_win::set_window_icon(win32.hwnd.get());
            }
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
                let _ = proxy.send_event(Wake::Menu(command));
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
        // The encoding submenus are empty until told what to list, and the list is `mdedit-file`'s
        // rather than the menu crate's — see `MenuBar::set_encodings`. Once, here, because it never
        // changes.
        let labels: Vec<&str> = mdedit_file::choices()
            .iter()
            .map(|choice| choice.label)
            .collect();
        if let Some(menu) = self.menu.as_mut() {
            menu.set_encodings(&labels);
        }
        // These were all decided before the window existed — the recent list from its file, the view
        // toggles from the settings — so this is the first chance any of them has had to reach the
        // menu.
        //
        // The three compared refreshes are forced by clearing what they think the menu was told.
        // They record their answer whether or not there is a menu to push it to, so if a frame were
        // ever drawn before the menu existed they would go on believing it had been told. Today the
        // menu is attached first and it does not arise; this makes that ordering not matter rather
        // than something to remember.
        self.refresh_recent_menu();
        self.refresh_view_menu();
        self.encoding_state = None;
        self.endings_state = None;
        self.tools_state = None;
        self.refresh_encodings_menu();
        self.refresh_endings_menu();
        self.refresh_tools_menu();
    }

    /// Carries menu commands into this editor's event loop.
    ///
    /// Held until there is a menu to attach it to, which is when the window appears.
    pub fn forward_menu(&mut self, proxy: EventLoopProxy<Wake>) {
        self.menu_proxy = Some(proxy);
    }

    /// Supplies the "where should this go?" dialog.
    ///
    /// Kept out of the library for the same reason the unsaved-changes prompt is: `rfd` cannot be
    /// linked into a test binary. See [`crate::prompt`].
    pub fn with_file_picker(&mut self, picker: SavePathPicker) {
        self.pick_save_path = picker;
    }

    /// Supplies the "which files?" dialog. See [`with_file_picker`](Self::with_file_picker).
    pub fn with_open_picker(&mut self, picker: OpenPathPicker) {
        self.pick_open_paths = picker;
    }

    /// Points the recent-files list somewhere other than beside the session file. For tests.
    pub fn with_recent(&mut self, recent: Recent) {
        self.recent = recent;
        self.refresh_recent_menu();
    }

    /// The files opened lately, most recent first.
    #[must_use]
    pub fn recent(&self) -> &Recent {
        &self.recent
    }

    /// Opens an empty buffer that has never been saved, and shows it.
    pub fn new_file(&mut self) {
        let index = self.workspace.add_untitled();
        if let Err(error) = self.workspace.activate(index) {
            crate::log_error!("could not open a new file: {error}");
        }
    }

    /// Asks which files to open, then opens them.
    ///
    /// The asking is somebody else's, as saving's is: an editor with no dialog opens nothing rather
    /// than guessing. Returns whether anything was opened.
    pub fn open_prompted(&mut self) -> bool {
        let Some(paths) = (self.pick_open_paths)() else {
            return false;
        };
        self.open_paths(&paths)
    }

    /// Opens each of `paths` in its own tab, showing the last one that worked.
    ///
    /// A path already open activates its existing tab rather than making a second one, which
    /// `Workspace::add` does by returning the index it already has.
    ///
    /// Every path goes through [`path::from_command_line`], not `path::validate`: these came from a
    /// person choosing in a dialog or clicking their own recent list, which is the same trust as a
    /// command-line argument. `validate` guards the files *this process wrote* — see
    /// [`mdedit_session::path`] for why they are different functions.
    ///
    /// [`path::from_command_line`]: mdedit_session::path::from_command_line
    pub fn open_paths(&mut self, paths: &[PathBuf]) -> bool {
        let mut opened = false;
        for path in paths {
            let resolved = match mdedit_session::path::from_command_line(path) {
                Ok(resolved) => resolved,
                Err(reason) => {
                    // A recent entry whose file has been deleted lands here. Dropped from the list
                    // rather than left to fail again every time the menu is opened.
                    crate::log_error!("could not open: {reason}");
                    self.recent.forget(path);
                    continue;
                }
            };
            let index = self.workspace.add(resolved.clone());
            if let Err(error) = self.workspace.activate(index) {
                crate::log_error!("could not open {}: {error}", resolved.display());
                continue;
            }
            self.recent.record(&resolved);
            opened = true;
        }

        if opened {
            // A different tab is showing, so the caret and the view belong to a different document.
            self.follow_caret = true;
            self.scroll.to_top();
        }
        self.refresh_recent_menu();
        opened
    }

    /// Records that `path` was opened, without opening it.
    ///
    /// For the routes into a document that do not go through [`open_paths`](Self::open_paths): the
    /// command line, Explorer, and Save As, which puts a tab somewhere it has not been before. A
    /// recent list that only knew about the Open dialog would be empty for the way most people start
    /// this editor.
    pub fn remember(&mut self, path: &Path) {
        self.recent.record(path);
        self.refresh_recent_menu();
    }

    /// Supplies the About dialog. Does nothing by default, as every other dialog seam does.
    pub fn with_about(&mut self, show: Box<dyn Fn()>) {
        self.show_about = show;
    }

    /// Supplies somewhere to tell the user why something they chose cannot be done.
    ///
    /// Inert by default, like every other collaborator that reaches the machine — see MDEDIT-67 and
    /// MDEDIT-68 for what the two exceptions to that rule cost.
    pub fn with_messages(&mut self, say: Box<dyn FnMut(&str)>) {
        self.say = say;
    }

    /// Reformats the active document, or says why it will not.
    ///
    /// # The rule this is the single exception to
    ///
    /// The byte-preservation invariant forbids the editor rewriting bytes the user did not ask it to
    /// rewrite. This is the user asking. It is one `replace` of the whole buffer inside MDEDIT-63's
    /// transaction, so **one press of Ctrl+Z puts every byte back**, and it is reachable only from
    /// the menu — never from a save, and never from anything on a timer.
    ///
    /// Refuses, with a message, when the document is not valid: formatting a half-written file would
    /// mean inventing the structure the user has not finished typing. Refuses silently when the
    /// language does not format at all, which is Markdown and YAML, and is why the menu item is
    /// greyed there rather than left to fail.
    ///
    /// A document that is already formatted is left alone entirely — no edit, so no undo step that
    /// appears to do nothing when pressed.
    pub fn format_document(&mut self) -> bool {
        let source = self.document().rope().to_string();
        let Some(result) = self.document().language().format(&source) else {
            return false;
        };

        let formatted = match result {
            Ok(formatted) => formatted,
            Err(diagnostic) => {
                // Named by line and column rather than by byte offset, because the message is for a
                // person looking at a document rather than at a hex dump. Tools → Validate Document
                // is one item away and puts the caret on it.
                let (line, column) = self.document().line_and_column(diagnostic.span.start());
                let message = format!(
                    "Cannot format: {} at line {}, column {}.",
                    diagnostic.message,
                    line.get() + 1,
                    column.get() + 1
                );
                (self.say)(&message);
                return false;
            }
        };

        if formatted == source {
            return false;
        }

        let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len()));
        let replaced = self.document_mut().transact(|document| {
            if let Err(error) = document.replace(whole, &formatted) {
                crate::log_error!("could not format the document: {error}");
                return false;
            }
            true
        });
        if replaced {
            self.follow_caret = true;
        }
        replaced
    }

    /// Whether Format Document would do anything here, for the menu item's enabled state.
    ///
    /// Asks the *language* rather than comparing its name, and asks only whether it formats at all —
    /// not whether this particular document is valid. A document part way through being typed is
    /// invalid most of the time, and an item that greyed itself out on every other keystroke would
    /// be worse than one that explains itself when pressed.
    ///
    /// The empty string is a probe, not a document: whether a language formats is a property of the
    /// language, so any input answers it, and that one costs nothing to reach a decision on.
    #[must_use]
    pub fn can_format(&self) -> bool {
        self.document().language().format("").is_some()
    }

    /// Shows the keyboard-shortcuts reference in a tab.
    ///
    /// Opening it twice shows the one already open rather than making a second, which is what a
    /// menu item people click twice needs to do.
    pub fn show_shortcuts(&mut self) -> bool {
        let existing = self
            .workspace
            .tabs()
            .iter()
            .position(|tab| tab.display_name() == crate::shortcuts::TITLE);
        let index = existing.unwrap_or_else(|| {
            self.workspace
                .add_generated(crate::shortcuts::TITLE, &crate::shortcuts::document())
        });
        if let Err(error) = self.workspace.activate(index) {
            crate::log_error!("could not show the shortcuts: {error}");
            return false;
        }
        self.follow_caret = true;
        self.scroll.to_top();
        true
    }

    /// Opens the recent-list entry at `index`, if it is still there.
    ///
    /// The index is read back against the same list the menu was built from, which is what makes an
    /// index safe to send through a menu event at all.
    pub fn open_recent(&mut self, index: usize) -> bool {
        let Some(path) = self.recent.paths().get(index).cloned() else {
            return false;
        };
        self.open_paths(&[path])
    }

    /// Rewrites the Open Recent submenu from the current list.
    ///
    /// Full paths rather than file names. A recent menu of six `README.md` is no menu at all, and
    /// there is nowhere to put the directory except in the label — muda has no second column and no
    /// tooltip.
    fn refresh_recent_menu(&mut self) {
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let labels: Vec<String> = self
            .recent
            .paths()
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        menu.set_recent(&labels);
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
            let (name, scratch) = self.workspace.tabs().get(index).map_or_else(
                || ("this file".to_owned(), false),
                |tab| (tab.display_name(), tab.is_untitled()),
            );
            let what = if scratch {
                prompt::Closing::Scratch
            } else {
                prompt::Closing::File
            };

            match self.prompt.ask(&name, what) {
                Unsaved::Save => {
                    // Saving the tab being closed means showing it first: `save` works on the
                    // active tab, which is the only one whose bytes we hold.
                    if self.workspace.activate(index).is_err() {
                        return false;
                    }
                    // A scratch buffer has nowhere to go, so Save means asking where. Before
                    // MDEDIT-66 this called `save`, which declined silently — the user pressed Save,
                    // nothing happened, and the only evidence was a line in the log.
                    if scratch {
                        self.save_as_prompted();
                    } else {
                        self.save();
                    }
                    // A failed save leaves the buffer dirty, and closing then would lose it —
                    // including a Save As the user declined.
                    if self.is_modified() {
                        crate::log_error!("not closing {name}: the save did not succeed");
                        return false;
                    }
                }
                Unsaved::Discard => {}
                // Keep is not offered here. Closing one tab is the place a user says what should
                // happen to *this* document, and "keep it but close it" would leave stored work with
                // no tab to see it in — which is how a store becomes a place things go missing.
                Unsaved::Keep | Unsaved::Cancel => return false,
            }
        }

        // Before the tab goes, because afterwards its id is gone from the workspace and nothing
        // could name the snapshot to remove it. Nothing did this until MDEDIT-66: a leak that did
        // not matter while snapshots were cleared on every clean exit, and matters a great deal now
        // they are permanent — a closed tab's stored text would come back for ever.
        self.forget_snapshot(index);

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

    /// Removes the stored copy of tab `index`, if it has one.
    fn forget_snapshot(&mut self, index: usize) {
        let Some(id) = self
            .workspace
            .tabs()
            .get(index)
            .map(mdedit_session::Tab::snapshot_id)
        else {
            return;
        };
        self.snapshotted.remove(&id);
        if let Some(store) = self.snapshots.as_mut() {
            if let Err(error) = store.forget(id) {
                crate::log_error!("could not remove a stored buffer: {error}");
            }
        }
    }

    /// Asks about every unsaved buffer, once, and says whether the editor may close.
    ///
    /// # Why this exists at all
    ///
    /// `WindowEvent::CloseRequested` used to call `event_loop.exit()` and nothing else, so Alt+F4
    /// or the close button lost every unsaved buffer without a word — and then the clean-exit path
    /// cleared the crash-recovery snapshots that would have got it back. A *crash* was survivable
    /// and a *close* was not, which is exactly backwards. MDEDIT-64.
    ///
    /// # One dialog
    ///
    /// Three modified tabs must not mean three consecutive dialogs. That is a thing people dismiss
    /// without reading, which turns a safeguard into a formality — so the files are named in one
    /// message and the answer applies to all of them.
    ///
    /// # What each answer does
    ///
    /// **Save** writes every file that has somewhere to go, and asks where to put anything untitled.
    /// If anything is still unsaved afterwards — a save that failed, a location the user declined to
    /// choose — the close is **cancelled**, because exiting then would lose exactly the work they
    /// just asked to keep.
    ///
    /// **Discard** closes. **Cancel** does not, and a cancelled exit is not a clean one: the
    /// recovery snapshots stay where they are, which `main` relies on by only clearing them once
    /// this has said yes.
    pub fn may_close(&mut self) -> bool {
        let names: Vec<String> = self
            .workspace
            .tabs()
            .iter()
            .filter(|tab| tab.is_modified())
            .map(mdedit_session::Tab::display_name)
            .collect();

        if names.is_empty() {
            return true;
        }

        match self.prompt.ask_all(&names) {
            // The snapshots are already on disk — the idle timer wrote them as the user typed — so
            // "keep" is a matter of *not* clearing them, and the work is already done.
            Unsaved::Keep => true,
            // Not offered by the window-level dialog, and meaningless if it ever were: it would
            // close the editor and leave the stored copies, which is Keep with worse words.
            Unsaved::Discard | Unsaved::Cancel => false,
            Unsaved::Save => {
                self.save_all();
                // An untitled buffer has nowhere to go, so `save_all` skipped it; it needs a
                // location chosen, which is a question rather than a save.
                self.save_untitled_buffers();

                let unsaved = self.workspace.has_unsaved_changes();
                if unsaved {
                    // Not an error to report and dismiss — the editor stays open, which is the
                    // message. Logged because the reason (a full disk, a declined dialog) is not
                    // otherwise recoverable afterwards.
                    crate::log_error!("not closing: some files are still unsaved");
                }
                !unsaved
            }
        }
    }

    /// Asks where to put each untitled buffer that has anything in it.
    ///
    /// One dialog each, which is the one place a file-at-a-time question is right: each needs its
    /// own answer, and there is nothing to batch.
    fn save_untitled_buffers(&mut self) {
        let was_active = self.workspace.active();

        for index in 0..self.workspace.tabs().len() {
            let needs_a_home = self
                .workspace
                .tabs()
                .get(index)
                .is_some_and(|tab| tab.is_untitled() && tab.is_modified());
            if !needs_a_home {
                continue;
            }
            if self.workspace.activate(index).is_err() {
                continue;
            }
            // The answer is deliberately not checked. Declining, or a write that failed, leaves the
            // tab modified — which is exactly what `may_close` reads as "do not close", so there is
            // nothing to do here but carry on and ask about the next one.
            let _ = self.save_as_prompted();
        }

        if let Some(index) = was_active {
            let _ = self.workspace.activate(index);
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
        self.apply_chord(character.as_str(), self.modifiers.shift_key())
    }

    /// Does what Ctrl plus `character` means, with `shift` said explicitly.
    ///
    /// Split out of [`Self::handle_chord`] to be callable by a test, which cannot construct a winit
    /// [`KeyEvent`] — it carries a private platform-specific field, which is why no test in this
    /// crate drives a key. That gap is not academic: MDEDIT-65 was Ctrl+Shift+S landing in the
    /// plain-save arm, labelled "Save As" in the menu and saving over the file instead, and nothing
    /// could have caught it. Every chord is reachable from here, so now something can.
    ///
    /// Returns whether the frame needs redrawing.
    pub fn apply_chord(&mut self, character: &str, shift: bool) -> bool {
        match character {
            // Split on Shift, as the undo/redo arms are: winit reports Shift+S as "S", so without
            // this the shifted chord fell into the plain-save arm. The menu has always labelled
            // Ctrl+Shift+S as Save As, and the label was the thing telling the truth.
            "s" | "S" if shift => self.save_as_prompted(),
            "s" | "S" => {
                self.save();
                false
            }
            "o" | "O" => self.open_prompted(),
            "n" | "N" => {
                self.new_file();
                true
            }
            // Shift+Ctrl+Z is the other conventional redo, and costs one arm to support.
            "z" if !shift => self.document_mut().undo(),
            "z" | "Z" | "y" | "Y" => self.document_mut().redo(),
            "w" | "W" => self.close_tab(),
            "	" => self.cycle_tab(!shift),
            "c" | "C" => {
                self.copy();
                false
            }
            // Before the cut arm, and it has to be: winit reports Ctrl+Shift+X as "X", which `"x" |
            // "X"` below would otherwise swallow. Exactly the class of mistake MDEDIT-65 was.
            "x" | "X" if shift => self.document_mut().toggle_inline(Inline::Strikethrough),
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
            // Bold and italic are not negotiable: in every editor an end user has formatted text in,
            // these are what these two keys do. MDEDIT-50 took Ctrl+B off the file pane for exactly
            // that reason — a Markdown editor where Ctrl+B moves a sidebar is wrong in a way people
            // notice within a minute.
            "b" | "B" => self.document_mut().toggle_inline(Inline::Bold),
            "i" | "I" => self.document_mut().toggle_inline(Inline::Italic),
            // Inline code. The ticket named chords for bold, italic, strikethrough and link and left
            // this one open; Ctrl+E is the choice, because it is free here and is what several
            // Markdown comment editors use. Weaker ground than the two above, and marked as such.
            "e" | "E" if !shift => self.document_mut().toggle_inline(Inline::Code),
            "k" | "K" => self.link(),
            "f" | "F" => self.open_find(),
            "h" | "H" => self.open_replace(),
            "g" | "G" => self.open_goto(),
            // Ctrl+= is the practical spelling of "Ctrl and plus": `+` needs Shift on most layouts,
            // so an editor that only bound `+` would answer to a chord nobody presses. Both work.
            "=" | "+" => self.zoom_by(1),
            "-" | "_" => self.zoom_by(-1),
            "0" => self.reset_zoom(),
            // Typora's convention. Typora also uses Ctrl+0 for "clear to paragraph", but that key
            // belongs to zoom here and zoom's claim on it is the wider convention — so clearing is
            // done by invoking the level a line already has, which is what `set_block` does.
            "1" | "2" | "3" | "4" | "5" | "6" => {
                let level = character.as_bytes().first().map_or(1, |byte| byte - b'0');
                self.document_mut().set_block(format::Block::Heading(level))
            }
            // The file pane, moved here from Ctrl+B. VS Code's chord for its explorer, which is the
            // convention people arriving from anywhere else already have.
            "e" | "E" => {
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

        // F3 works wherever focus is, which is the point of having it as well as Enter — it is how
        // you step through matches while typing in the document.
        if matches!(&event.logical_key, Key::Named(NamedKey::F3)) {
            return if self.modifiers.shift_key() {
                self.find_previous()
            } else {
                self.find_next()
            };
        }

        // Everything else goes to whichever of the two has focus, and the find field having it is
        // the only case where the document does not. That is the whole focus model.
        if self.find_focused() {
            return self.find_key(event);
        }

        // Chords first, so a modified key never inserts its own letter. Deliberately **above** the
        // dismissal: Ctrl+B is one of the bar's own actions arriving by keyboard, and taking the bar
        // away only to have the idle timer put it back is a flicker for no rule.
        if self.modifiers.control_key() {
            return self.handle_chord(event);
        }

        // Any other key takes the floating bar away. It is put back by the idle timer when the key
        // was a shift-motion and the selection has stopped growing — which is the whole of the
        // "settles" rule for the keyboard, and why this can be blunt.
        let dismissed = self.dismiss_floating();
        if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
            // Esc closes the find bar even when focus has gone back to the document — the bar is
            // still taking room and still lighting matches, so leaving it open would make Esc do
            // half of what it looks like it does.
            return self.close_find() || dismissed;
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
            // What the file already uses, rather than always `\n` — MDEDIT-59. Enter used to add an
            // LF to a CRLF document, which is how a file becomes Mixed without anybody pasting
            // anything.
            Key::Named(NamedKey::Enter) => {
                let ending = self.newline().as_str();
                self.insert(ending)
            }
            Key::Named(NamedKey::Space) => self.insert(" "),
            Key::Character(text) => self.insert(text),
            _ => false,
        }
    }

    /// Routes one key press to the find field.
    ///
    /// Chords are **not** swallowed. Ctrl+S saves the document while the caret is in the search box,
    /// which is what anyone would expect; only the three that mean something to a text field —
    /// select all, copy, paste and cut — are taken here, and the rest go where they always went.
    fn find_key(&mut self, event: &KeyEvent) -> bool {
        let shift = self.modifiers.shift_key();
        if self.modifiers.control_key() {
            let Key::Character(character) = &event.logical_key else {
                return self.handle_chord(event);
            };
            return match character.as_str() {
                "a" | "A" => self.press_find(&field::Press::SelectAll, false),
                "c" | "C" => {
                    self.copy_find();
                    false
                }
                "x" | "X" if !shift => {
                    self.copy_find();
                    self.press_find(&field::Press::Backspace, false)
                }
                "v" | "V" => self.paste_find(),
                _ => self.handle_chord(event),
            };
        }

        let decoded = match &event.logical_key {
            Key::Named(NamedKey::Escape) => find::Key::Close,
            Key::Named(NamedKey::Tab) => find::Key::NextField,
            Key::Named(NamedKey::Enter) if shift => find::Key::Previous,
            Key::Named(NamedKey::Enter) => find::Key::Next,
            Key::Named(NamedKey::Backspace) => find::Key::Edit(field::Press::Backspace),
            Key::Named(NamedKey::Delete) => find::Key::Edit(field::Press::Delete),
            Key::Named(NamedKey::ArrowLeft) => find::Key::Edit(field::Press::Left),
            Key::Named(NamedKey::ArrowRight) => find::Key::Edit(field::Press::Right),
            Key::Named(NamedKey::Home) => find::Key::Edit(field::Press::Home),
            Key::Named(NamedKey::End) => find::Key::Edit(field::Press::End),
            Key::Named(NamedKey::Space) => find::Key::Edit(field::Press::Text(" ".to_owned())),
            Key::Character(text) => find::Key::Edit(field::Press::Text(text.to_string())),
            // Up, Down and the paging keys do nothing here. Deliberately *nothing* rather than
            // falling through to the document: a key that scrolled the text behind a focused search
            // box would be the focus model leaking.
            _ => return false,
        };
        self.apply_find_key(&decoded, shift)
    }

    /// Does what one key press means to the find bar, with Shift said explicitly.
    ///
    /// The testable half of [`Self::find_key`] — see [`find::Key`] for why the two are separate.
    pub fn apply_find_key(&mut self, key: &find::Key, shift: bool) -> bool {
        match key {
            find::Key::Edit(press) => self.press_find(press, shift),
            // Enter means "do the thing this mode is for". In Go to Line that is the jump, and it is
            // also what closes the bar afterwards: unlike a search, there is nothing to step through.
            find::Key::Next if self.find.mode == find::Mode::GoTo => {
                let went = self.go_to_line();
                self.close_find();
                went
            }
            find::Key::Next => self.find_next(),
            find::Key::Previous => self.find_previous(),
            find::Key::Close => self.close_find(),
            find::Key::NextField => self.find.next_field(),
        }
    }

    /// Puts the field's selection on the clipboard.
    fn copy_find(&mut self) {
        let Some(text) = self.find.field.selected_text().map(str::to_owned) else {
            return;
        };
        self.clipboard.write(&text);
    }

    /// Types the clipboard into the field.
    fn paste_find(&mut self) -> bool {
        let Some(text) = self.clipboard.read() else {
            return false;
        };
        self.press_find(&field::Press::Text(text), false)
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

        // Pasting a URL over selected text makes a link of it rather than replacing it. Small, and
        // the thing people are most delighted to find works — it is what every chat client and
        // note-taking app they use already does, so its absence reads as a missing feature rather
        // than as a deliberate simplicity.
        //
        // Only over a *selection*: pasting a URL into empty space is a paste, and turning it into
        // `[](url)` would be inventing link text nobody wrote.
        if !self.document().selection().is_collapsed() && format::looks_like_url(&text) {
            return self.document_mut().make_link(Some(text.trim()));
        }
        self.insert(&text)
    }

    /// Makes a link of the selection, or of the word at the caret.
    ///
    /// Uses the clipboard when it holds something URL-shaped, which is the case that needs no
    /// dialog at all — copy a link, select some words, press Ctrl+K, done. Otherwise the destination
    /// is left empty with the caret in it, ready to type into.
    pub fn link(&mut self) -> bool {
        let url = self
            .clipboard
            .read()
            .filter(|text| format::looks_like_url(text));
        self.follow_caret = true;
        self.document_mut().make_link(url.as_deref().map(str::trim))
    }

    /// Selects the whole buffer.
    pub fn select_all(&mut self) {
        self.found_selection = false;
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
    ///
    /// **Asks for the caret to be revealed**, because this is the tests' stand-in for typing and
    /// typing does — `handle_key` sets the same flag before it dispatches anything. Without it the
    /// seam was subtly unfaithful, and MDEDIT-49 is where that mattered: a test typing at the end of
    /// a long line saw no horizontal reveal and reported the feature broken when it was the test
    /// that had not asked for one.
    pub fn insert_text(&mut self, text: &str) -> bool {
        self.follow_caret = true;
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
        // Hidden, not moved. Repositioning against a viewport that is still moving is jitter, and a
        // bar that slides about as the wheel turns reads as broken.
        self.dismiss_floating();
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
        // Recorded again by `draw_chunks` if the selection's first row is drawn this frame. Cleared
        // rather than kept because a selection scrolled out of view has no anchor, and a stale one
        // would leave the floating bar hanging over the wrong text.
        self.selection_anchor = None;

        // Everything the chrome has not taken. One place decides it; see `crate::content`.
        self.content = ContentRect::new(width / scale, height / scale, MARGIN, self.insets());
        let content = self.content;
        let visible = content.height();
        let mut frame = self.measure(self.wrap_width());

        // The one place the document's contiguous text already exists, so searching it costs no
        // second `to_string`. Rescans only when the needle, the case flag or the revision has moved
        // — see `find::Bar::refresh` — so a still frame with the bar open costs three comparisons.
        let document = self.find_document();
        let revision = self.document().revision();
        if self.find_open {
            self.find.refresh(&frame.text, document, revision);
        }

        // Which document is showing may have changed since the last frame, and both of these are
        // facts about that document. Compared before they push, so a still frame costs three bools.
        self.refresh_tools_menu();
        self.refresh_endings_menu();
        self.refresh_encodings_menu();

        self.hold_anchor(&frame, visible);
        let following = self.follow_caret;
        if following {
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

        // **The horizontal reveal happens here rather than beside the vertical one**, and the reason
        // is `widest`. It is measured, so it only knows about chunks that have been laid out — and
        // before `warm` the chunk the caret is in may be one of the ones that have not, which is
        // always the case for the chunk that was just edited. Revealing there clamped against a
        // width of zero and did nothing at all: typing at the end of a long line left the caret off
        // the right edge. A little margin either side, so the caret does not sit hard against the
        // edge with nothing after it to read.
        if following && !self.wrap {
            if let Some((_, caret)) = self.caret_position(&frame) {
                self.pan.reveal(
                    (caret.x - PAN_MARGIN).max(0.0),
                    caret.x + PAN_MARGIN,
                    self.widest,
                    content.width(),
                );
            }
        }
        // Re-clamped against what is now known to be the widest measured row, which may have grown
        // this frame as more chunks were laid out. Same shape as the vertical clamp above, and the
        // same reason: an offset that was legal last frame can be past the end of this one.
        self.pan
            .set(self.pan.offset(), self.widest, content.width());
        let viewport = self.viewport_offset();

        let range = blocks::visible(&frame.tops, &frame.heights, offset, visible);
        let at = Placement {
            content,
            offset: viewport,
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

        let placed = caret.map(|caret| content.to_window(caret.x, caret.top, viewport));
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
        self.draw_toolbar(&mut quads, width, scale);
        self.draw_find(&mut quads, width, scale);
        self.draw_pane(&mut quads, height, scale);
        self.draw_status_bar(&mut quads, width, height, scale);
        self.draw_scrollbar(&mut quads, width, scale);
        // Genuinely last, and that is the whole of its z-order: it floats over the document *and*
        // over any chrome it happens to overlap, which is what "floating" means with no depth
        // buffer to say so.
        self.draw_floating(&mut quads, scale);

        // Kept for the next click and the next Up or Down: both need to know where the lines
        // ended up, and this is the only place that knows.
        self.last_frame = Some(frame);
        quads
    }

    /// Draws the formatting bar below the tab strip, and remembers where its buttons went.
    ///
    /// Each glyph is laid out **once** and kept. The gutter, the status bar and the file pane each
    /// paid for laying text out every frame before it was noticed; a fourth would be indefensible,
    /// and `a_still_frame_lays_out_no_toolbar_labels` is the guard.
    fn draw_toolbar(&mut self, quads: &mut Vec<QuadInstance>, width: f32, scale: f32) {
        self.toolbar_buttons.clear();
        let actions = self.toolbar_actions();
        if actions.is_empty() {
            return;
        }

        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        for action in actions {
            let glyph = action.glyph();
            if !self.toolbar_labels.contains_key(&glyph) {
                if let Some(label) = self.lay_out_label(&glyph, style) {
                    self.toolbar_labels.insert(glyph, label);
                }
            }
        }

        let widths: Vec<f32> = actions
            .iter()
            .map(|action| {
                self.toolbar_labels
                    .get(&action.glyph())
                    .map_or(0.0, |label| label.width)
            })
            .collect();
        let logical_width = width / scale;
        self.toolbar_buttons = toolbar::layout(actions, &widths, logical_width);

        let top = self.strip_height();
        quads.push(Self::solid(
            [0.0, top * scale, width, toolbar::HEIGHT * scale],
            self.theme.strip,
        ));

        let lit: Vec<bool> = self
            .toolbar_buttons
            .iter()
            .map(|button| self.is_lit(button.action))
            .collect();
        for (button, active) in self.toolbar_buttons.clone().into_iter().zip(lit) {
            if active {
                quads.push(Self::solid(
                    [
                        button.x * scale,
                        (top + 3.0) * scale,
                        button.width * scale,
                        (toolbar::HEIGHT - 6.0) * scale,
                    ],
                    self.theme.tab_active,
                ));
            }
            let Some(label) = self.toolbar_labels.get(&button.action.glyph()) else {
                continue;
            };
            let where_ = LabelAt {
                x: button.x + (button.width - label.width) / 2.0,
                baseline: top + toolbar::HEIGHT / 2.0 + self.theme.body_size * 0.3,
                scale,
                colour: self.theme.text,
            };
            let glyphs = label.glyphs.clone();
            self.draw_label_glyphs(quads, &glyphs, where_);
        }
    }

    /// Draws the bar that follows a selection, if one is wanted and there is somewhere to put it.
    ///
    /// Shares the fixed bar's label cache: the two draw the same glyphs, and a second cache of the
    /// same strings would be the per-frame-layout trap paid for a fifth time with extra steps.
    fn draw_floating(&mut self, quads: &mut Vec<QuadInstance>, scale: f32) {
        self.floating_buttons.clear();
        self.floating_at = None;

        let actions = self.floating_actions();
        let Some(anchor) = self.selection_anchor else {
            return;
        };
        if actions.is_empty() {
            return;
        }

        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        for action in actions {
            let glyph = action.glyph();
            if !self.toolbar_labels.contains_key(&glyph) {
                if let Some(label) = self.lay_out_label(&glyph, style) {
                    self.toolbar_labels.insert(glyph, label);
                }
            }
        }

        let widths: Vec<f32> = actions
            .iter()
            .map(|action| {
                self.toolbar_labels
                    .get(&action.glyph())
                    .map_or(0.0, |label| label.width)
            })
            .collect();
        // Laid out against the content's width rather than the window's, since that is the room it
        // is allowed to occupy; `place` then clamps it inside the same edges.
        let cells = toolbar::layout(actions, &widths, self.content.width());
        if cells.is_empty() {
            return;
        }

        let bar_width = toolbar::width_of(&cells);
        let left = self.content.left_edge();
        let (x, y) = toolbar::place(
            bar_width,
            anchor,
            left,
            left + self.content.width(),
            self.content.top_edge(),
        );
        self.floating_buttons = toolbar::shifted(&cells, x);
        self.floating_at = Some((x, y, bar_width));

        // An outline, then the fill inside it. The fixed bar needs none — it has the window's edge
        // and the tab strip either side of it — but this one is a panel over prose, and a fill close
        // to the background with no border reads as a hole in the text rather than as something in
        // front of it.
        quads.push(Self::solid(
            [
                (x - FLOAT_BORDER) * scale,
                (y - FLOAT_BORDER) * scale,
                (bar_width + FLOAT_BORDER * 2.0) * scale,
                (toolbar::HEIGHT + FLOAT_BORDER * 2.0) * scale,
            ],
            self.theme.marker,
        ));
        quads.push(Self::solid(
            [
                x * scale,
                y * scale,
                bar_width * scale,
                toolbar::HEIGHT * scale,
            ],
            self.theme.tab_active,
        ));

        let lit: Vec<bool> = self
            .floating_buttons
            .iter()
            .map(|button| self.is_lit(button.action))
            .collect();
        for (button, active) in self.floating_buttons.clone().into_iter().zip(lit) {
            if active {
                quads.push(Self::solid(
                    [
                        button.x * scale,
                        (y + 3.0) * scale,
                        button.width * scale,
                        (toolbar::HEIGHT - 6.0) * scale,
                    ],
                    self.theme.selection,
                ));
            }
            let Some(label) = self.toolbar_labels.get(&button.action.glyph()) else {
                continue;
            };
            let where_ = LabelAt {
                x: button.x + (button.width - label.width) / 2.0,
                baseline: y + toolbar::HEIGHT / 2.0 + self.theme.body_size * 0.22,
                scale,
                colour: self.theme.text,
            };
            let glyphs = label.glyphs.clone();
            self.draw_label_glyphs(quads, &glyphs, where_);
        }
    }

    /// The selection has settled, so the bar may appear.
    ///
    /// **Settled, not made.** A bar that appeared while the pointer was still down would chase the
    /// drag across the screen, which is the classic bad implementation of this pattern and the
    /// reason the rule is written as its own method rather than left to the event arm. Called on
    /// mouse release, and on the idle timer for a selection extended with the keyboard.
    pub fn settle_selection(&mut self) -> bool {
        if self.dragging || self.document().selection().is_collapsed() {
            return false;
        }
        // A search-made selection is not one to hang a bar on. Every match landed on is a settled
        // selection by this method.s own rule, so without this Find Next put a formatting bar over
        // the prose on every press, and Esc — which closes the find bar — raised one as a parting
        // gift. Both found by eye in MDEDIT-44, and both invisible to a test that only ever asks
        // where the selection is.
        if self.found_selection {
            return false;
        }
        let was = self.floating;
        self.floating = true;
        self.floating_for = self.workspace.active();
        !was
    }

    /// The bar goes away.
    ///
    /// Every path that could invalidate where it is pointing calls this: typing, Esc, scrolling,
    /// collapsing the selection, changing tab. **Hidden on scroll rather than moved** — following a
    /// moving viewport is jitter, and a bar that jumps as the wheel turns is worse than one that
    /// waits to be asked again.
    pub fn dismiss_floating(&mut self) -> bool {
        let was = self.floating;
        self.floating = false;
        was
    }

    /// Whether the floating bar is currently wanted.
    #[must_use]
    pub fn floating_shown(&self) -> bool {
        self.floating
    }

    /// Where the floating bar was placed last frame, and how wide, for a test.
    #[must_use]
    pub fn floating_at(&self) -> Option<(f32, f32, f32)> {
        self.floating_at
    }

    /// The floating bar's buttons where they were placed, for a test.
    #[must_use]
    pub fn floating_buttons(&self) -> &[toolbar::ButtonRect] {
        &self.floating_buttons
    }

    /// The buttons the floating bar gets, or nothing if it should not be showing at all.
    ///
    /// Markdown only, and only with a selection that has settled. JSON and YAML get nothing for the
    /// reason they get no fixed bar either: their actions are document-level, and a bar that follows
    /// a selection offering to reformat the whole file is a different operation in the same clothes.
    fn floating_actions(&self) -> &'static [toolbar::Action] {
        if !self.floating
            || self.floating_for != self.workspace.active()
            || self.document().selection().is_collapsed()
        {
            return &[];
        }
        match self.workspace.active_open() {
            Some(open) if open.document.language().name() == "Markdown" => toolbar::SELECTION,
            None => toolbar::SELECTION,
            Some(_) => &[],
        }
    }

    /// Whether the caret is already inside what `action` would make, so the button lights.
    ///
    /// Lookups against the segments and blocks the parse already produced, not scans — that is the
    /// line the keystroke budget cares about, and why lighting a toolbar is affordable at all.
    pub fn is_lit(&self, action: toolbar::Action) -> bool {
        use mdedit_document::format::{Block, Inline};

        let doc = self.document();
        match action {
            toolbar::Action::Bold => doc.is_inside(Inline::Bold),
            toolbar::Action::Italic => doc.is_inside(Inline::Italic),
            toolbar::Action::Strikethrough => doc.is_inside(Inline::Strikethrough),
            toolbar::Action::Code => doc.is_inside(Inline::Code),
            // Asked of the block list rather than of the line, because a setext heading's first line
            // carries no marker and would otherwise never light.
            toolbar::Action::Heading(level) => doc
                .block_at(doc.selection().head())
                .is_some_and(|block| block.kind() == BlockKind::Heading(level)),
            toolbar::Action::Bullet => doc.carries(Block::Bullet),
            toolbar::Action::Ordered => doc.carries(Block::Ordered),
            toolbar::Action::Task => doc.carries(Block::Task),
            toolbar::Action::Quote => doc.is_quoted(),
            toolbar::Action::Fence => doc
                .block_at(doc.selection().head())
                .is_some_and(|block| block.kind() == BlockKind::CodeBlock),
            // Neither is a state. A rule is inserted and a link is made; there is no "inside a
            // horizontal rule" for a button to reflect.
            toolbar::Action::Rule | toolbar::Action::Link => false,
        }
    }

    /// Draws the scrollbar down the right edge, and remembers where it went.
    ///
    /// Last of all the chrome, so it overlays the status bar's right end as well as the text — the
    /// bar is the thing you reach for without looking, and it should not have a gap at the bottom
    /// where something else was drawn later.
    ///
    /// Nothing is drawn, and nothing is stored, when the document fits: an always-present bar with a
    /// full-height thumb is a decoration that costs a click target.
    fn draw_scrollbar(&mut self, quads: &mut Vec<QuadInstance>, width: f32, scale: f32) {
        let logical_width = width / scale;
        // The track starts under whatever chrome is above the document and stops above the status
        // bar, so it measures the part of the window the document actually occupies.
        let top = self.content.top_edge() - MARGIN;
        self.scrollbar = scrollbar::layout(
            logical_width,
            top.max(0.0),
            self.viewport.height,
            self.viewport.content,
            self.scroll.offset(),
        );

        let Some(bar) = self.scrollbar else {
            return;
        };
        quads.push(Self::solid(
            [
                bar.x * scale,
                bar.thumb_top * scale,
                scrollbar::WIDTH * scale,
                bar.thumb_height * scale,
            ],
            self.theme.scrollbar_thumb,
        ));
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

        // And the same for the diagnostics, for the same reason and with one difference that
        // matters. The idle timer is set by a *key event*, so a document that was merely opened
        // never reaches it — a file with three errors in it showed none until you typed, which is
        // the state it is most useful in. This is the once-per-document catch-up.
        //
        // **Keyed on the tab, not on the stamp.** A stamp comparison here would re-run the whole
        // check on every frame while somebody is typing, which is exactly the 1.3 ms the idle timer
        // exists to keep off this path. Once per document, then the timer owns it.
        if self
            .diagnosed_for
            .is_none_or(|(tab, _)| Some(tab) != self.workspace.active())
        {
            self.revalidate();
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
        let share = status::right_share(logical_width);

        // The count asks for its room first among the right-hand fields, which inverts the priority
        // order of the fields beside it. Deliberate: it is short and bounded, and a diagnostic that
        // vanishes when the window narrows is a diagnostic you do not know about.
        let count = status::errors(self.diagnostic_count());
        let count_room = match &count {
            Some(text) => self.status_label(COUNT, text, style).min(share),
            None => 0.0,
        };
        let gap = if count.is_some() {
            status::COUNT_GAP
        } else {
            0.0
        };

        let wanted = self.status_label(RIGHT, &right_text, style);
        let right_room = wanted.min((share - count_room - gap).max(0.0));
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

        let count_x = (right_x - gap - count_room).max(status::PADDING);
        if count.is_some() {
            self.draw_status_label(
                quads,
                COUNT,
                count_x,
                baseline,
                self.theme.status_error,
                scale,
                count_room,
            );
        }

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
            // are full. Measured against the count when there is one, because the count is now the
            // leftmost thing in that container.
            let trail_room = (count_x - status::GAP - status::PADDING).max(0.0);
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

    /// Lays out one short string of chrome, ready to be cached.
    ///
    /// The gutter, the status bar and the file pane each grew their own copy of this before there
    /// were four of them. Shared from here on; the other three are left alone because moving them
    /// would be a refactor of three working caches inside a feature ticket.
    fn lay_out_label(&mut self, text: &str, style: TextStyle) -> Option<Label> {
        let run = StyledRun {
            text,
            style,
            source: ByteSpan::empty_at(ByteOffset::ZERO),
        };
        let layout = self.text.layout(&[run], f32::MAX).ok()?;
        let glyphs: Vec<_> = layout.glyphs().copied().collect();
        let width = glyphs
            .iter()
            .map(|glyph| glyph.pos.x + glyph.advance)
            .fold(0.0_f32, f32::max);
        Some(Label {
            text: text.to_owned(),
            glyphs,
            width,
        })
    }

    /// Draws glyphs a cache has already laid out.
    fn draw_label_glyphs(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        glyphs: &[mdedit_text::PositionedGlyph],
        where_: LabelAt,
    ) {
        for glyph in glyphs {
            let Some(entry) = self.entry_for(glyph.key, where_.scale) else {
                continue;
            };
            quads.push(QuadInstance {
                rect: [
                    (where_.x + glyph.pos.x) * where_.scale + px_i(entry.left),
                    where_.baseline * where_.scale + px_i(entry.top),
                    px(entry.width),
                    px(entry.height),
                ],
                uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
                colour: [
                    where_.colour.r,
                    where_.colour.g,
                    where_.colour.b,
                    where_.colour.a,
                ],
            });
        }
    }

    /// The word count the status bar is showing, or `None` before the first one has finished.
    ///
    /// Deliberately the *shown* number rather than a fresh count: what a test wants to know is what
    /// the user is being told, including when that is a moment behind.
    #[must_use]
    pub fn words(&self) -> Option<usize> {
        self.words
    }

    /// Recounts the words. Called on the idle timer, never on the keystroke path.
    fn recount_words(&mut self) {
        self.words = Some(status::count_words(&self.document().rope().to_string()));
        self.counted_for = self.workspace.active();
    }

    /// Which document, at which revision, the diagnostics would describe if recomputed now.
    fn diagnosis_stamp(&self) -> Option<(usize, u64)> {
        Some((self.workspace.active()?, self.document().revision()))
    }

    /// Re-runs the language's strict check. Called on the idle timer, never on the keystroke path.
    ///
    /// It walks the whole buffer, which is the entire reason it lives here beside the whole-document
    /// re-parse and the word count rather than anywhere a keystroke can reach.
    fn revalidate(&mut self) {
        let stamp = self.diagnosis_stamp();
        let source = self.document().rope().to_string();
        self.diagnostics = self.document().language().validate(&source);
        self.diagnosed_for = stamp;
    }

    /// What is wrong with the active document, or nothing while an edit is newer than the check.
    ///
    /// **Nothing, rather than the previous answer**, once the buffer has moved on. The word count
    /// shows a stale number because a stale number is nearly right and costs nothing; a stale
    /// diagnostic points at bytes that have moved, and the menu item puts the caret on it.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        if self.diagnosed_for == self.diagnosis_stamp() {
            &self.diagnostics
        } else {
            &[]
        }
    }

    /// How many problems to show. Zero while an edit is newer than the check.
    #[must_use]
    pub fn diagnostic_count(&self) -> usize {
        self.diagnostics().len()
    }

    /// Recomputes now and goes to the first problem, selecting it.
    ///
    /// Recomputing first is the point of the menu item: the count on the bar is at most one idle
    /// interval old, but "take me to it" has to act on the document as it is this instant, or it
    /// moves the caret using offsets from before the last keystroke.
    ///
    /// Returns whether there was anything to go to.
    pub fn go_to_first_diagnostic(&mut self) -> bool {
        self.revalidate();
        let Some(first) = self.diagnostics.first() else {
            return false;
        };
        let span = first.span;
        self.document_mut()
            .set_selection(Selection::new(span.start(), span.end()));
        // The same reveal every other jump uses, so a diagnostic below the fold materialises the
        // chunk it is in without anything new being written for it.
        self.follow_caret = true;
        // A search's selection suppresses the floating bar and this is the same kind of selection —
        // the editor put it there, the user did not sweep it out.
        self.found_selection = true;
        true
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
        self.refresh_view_menu();
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
            // The bars sit below the strip and above the document, so the three add.
            top: self.strip_height() + self.toolbar_height() + self.find_height(),
            // The pane and the gutter both take from the left, and they add rather than compete —
            // which is exactly the arithmetic `ContentRect` exists to keep in one place.
            left: self.pane_width() + self.gutter_width(),
            bottom: status::BAR_HEIGHT,
            ..Insets::default()
        }
    }

    /// Height the formatting bar occupies, which is nothing when it is off or has nothing to show.
    ///
    /// **An empty bar is worse than none**, so a language with no button set takes no room rather
    /// than drawing a blank strip. That is also what happens with no tabs open at all.
    fn toolbar_height(&self) -> f32 {
        if self.toolbar_actions().is_empty() {
            0.0
        } else {
            toolbar::HEIGHT
        }
    }

    /// The buttons the visible document's language gets, or nothing.
    ///
    /// Keyed off the language **here**, in the app, rather than through `mdedit_language::Language`.
    /// Actions are UI vocabulary and the trait is parsing vocabulary; a parser has no business
    /// knowing what a toolbar is.
    fn toolbar_actions(&self) -> &'static [toolbar::Action] {
        if !self.toolbar {
            return &[];
        }
        match self.workspace.active_open() {
            // Markdown is the only language with actions so far. JSON's Format and Validate arrive
            // with MDEDIT-47 and MDEDIT-48; until then it gets no bar, because a button that does
            // nothing is worse than no button.
            Some(open) if open.document.language().name() == "Markdown" => toolbar::MARKDOWN,
            // The scratch document, before anything is open, is Markdown too.
            None => toolbar::MARKDOWN,
            Some(_) => &[],
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
        // The pane runs from under the formatting bar to above the status bar. There is no tab strip
        // when it is showing, and the status bar is drawn after it — but the bar spans the whole
        // window width, as the strip does, so the pane starts below it rather than under it.
        let top = self.toolbar_height();
        let visible = (logical_height - status::BAR_HEIGHT - top).max(0.0);
        let count = self.workspace.tabs().len();

        self.pane_scroll = self
            .pane_scroll
            .clamp(0.0, pane::max_scroll(count, visible));
        self.row_rects = pane::layout(count, visible, self.pane_scroll);

        // `panel`, not `strip`: MDEDIT-72's palette has a raised surface, and the pane is one. The
        // strip is the ground *behind* tabs, which is a different thing that happened to be the
        // only dark chrome colour there was.
        quads.push(Self::solid(
            [0.0, top * scale, pane::PANE_WIDTH * scale, visible * scale],
            self.theme.panel,
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
                        (top + rect.y) * scale,
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
            let baseline = top + rect.y + pane::ROW_HEIGHT * 0.7;

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
        // A wheel scroll stops the view following the caret, exactly as dragging the thumb does: the
        // user has said where they want to look. `pan_by` does the same for the sideways branch
        // below — see there for what it cost to find out.
        self.follow_caret = false;

        // **Shift turns the wheel sideways**, which is the convention everywhere and the only way to
        // pan without a horizontal scrollbar. Taken before the vertical arithmetic rather than after,
        // because a Shift+wheel must not also scroll down.
        if self.modifiers.shift_key() && !self.wrap {
            let sideways = match delta {
                MouseScrollDelta::LineDelta(_, lines) => -lines * PAN_PER_NOTCH,
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "scroll deltas are small pixel counts"
                )]
                MouseScrollDelta::PixelDelta(position) => -(position.y as f32),
            };
            self.pan_by(sideways);
            return;
        }

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
        if self.pane_hit(x, y).is_some() {
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
        // Remembered, so the next launch opens on whichever was chosen. Written on the idle timer
        // rather than here — see `crate::config`.
        let chrome = self.chrome;
        self.settings.update(|config| config.chrome = chrome);
        self.refresh_view_menu();
    }

    /// Shows or hides the line-number gutter, and remembers which.
    ///
    /// The setting existed from MDEDIT-22 and could only be changed by editing the file and
    /// restarting. MDEDIT-60 gave it somewhere to be written and this gives it somewhere to be
    /// clicked.
    pub fn toggle_line_numbers(&mut self) {
        self.config.line_numbers = !self.config.line_numbers;
        let on = self.config.line_numbers;
        self.settings.update(|config| config.line_numbers = on);
        // The gutter changes the document's width, so every chunk wraps differently — and a layout
        // cached at the old width would draw the old wrapping.
        self.layouts.clear();
        self.heights.clear();
        self.digits = None;
        self.refresh_view_menu();
    }

    /// Makes the View menu's ticks match the editor.
    ///
    /// Called from every path that changes one, **including the chords** — a menu that only knew
    /// about its own clicks would show the wrong state the moment anybody pressed Ctrl+/. That is
    /// the cost of check items, and the reason they are worth it: a chord can toggle something but
    /// cannot tell you which way it is now.
    fn refresh_view_menu(&self) {
        let Some(menu) = self.menu.as_ref() else {
            return;
        };
        menu.set_view(mdedit_menu_win::ViewState {
            plain_markup: self.mode == view::Mode::Plain,
            file_pane: self.chrome == Chrome::Pane,
            line_numbers: self.config.line_numbers,
            toolbar: self.toolbar,
            whitespace: self.whitespace,
            word_wrap: self.wrap,
        });
    }

    /// Greys the Tools items that do not apply to the document being shown.
    ///
    /// Called every frame, and cheap because it compares first: the native call only happens when
    /// the answer has actually changed, which is on a language change and nowhere else.
    fn refresh_tools_menu(&mut self) {
        let wanted = mdedit_menu_win::ToolsState {
            can_format: self.can_format(),
        };
        if self.tools_state == Some(wanted) {
            return;
        }
        self.tools_state = Some(wanted);
        if let Some(menu) = self.menu.as_ref() {
            menu.set_tools(wanted);
        }
    }

    /// A left-click anywhere in the window.
    ///
    /// Chrome first, because both presentations sit over an edge of the document; only one of the
    /// two is ever showing, so at most one can match. The order lives here rather than in the event
    /// handler so that a test clicks the same way the window does — a test that reached past this
    /// into [`click_at`](Self::click_at) would be exercising a dispatch the app does not use.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        // The floating bar before anything else, because it is drawn over everything else. Order in
        // this one function is the whole of the z-order, in both directions: drawn last, asked
        // first.
        if self.click_floating(x, y) {
            return true;
        }
        // Anything else is a click that is not on the bar, and takes it away. Done before the rest
        // of the chain rather than after, so a click that lands on a tab or in the text has already
        // dismissed it by the time that click is resolved.
        self.dismiss_floating();
        // The scrollbar next, because it *overlays* the text rather than taking width from it —
        // a press on the bar must never also land in the document underneath.
        if let Some(hit) = scrollbar::hit_test(self.scrollbar, x, y) {
            self.clicks.reset();
            return self.click_scrollbar(hit, y);
        }
        if self.click_find(x, y)
            || self.click_toolbar(x, y)
            || self.click_strip(x, y)
            || self.click_pane(x, y)
        {
            // The pointer was on chrome, not on text. Ending the run stops a click on a tab
            // followed by one in the document from counting as a double-click on the document.
            self.clicks.reset();
            return true;
        }
        // Clicking into the text is how focus comes back without closing the bar, which is what
        // makes "find it, then edit it there" one gesture rather than two.
        self.find_focused = false;
        self.click_at(x, y)
    }

    /// Decides where the caret goes for a right-click at `(x, y)`, and returns whether to show a
    /// menu.
    ///
    /// **The one subtle rule in the context menu.** A right-click *inside* the current selection
    /// leaves it alone, so Cut and Copy act on what the user can see is selected — moving the caret
    /// there would collapse the selection and grey out the two entries they were reaching for. A
    /// right-click *outside* it places the caret first, as every editor does, because otherwise the
    /// menu would act on something the pointer is nowhere near.
    ///
    /// Separate from showing the menu because the menu is a native popup that cannot be driven by a
    /// test, and this rule can be.
    pub fn right_click_at(&mut self, x: f32, y: f32) -> bool {
        // Chrome has no context menu of its own yet, and a document menu popping up over the tab
        // strip would be worse than nothing. Deliberately not the same as the left-button chain:
        // there is nothing here for the scrollbar or the strip to do.
        if !self.content.contains(x, y) {
            return false;
        }
        let Some(offset) = self.offset_at_point(x, y) else {
            return false;
        };

        let selection = self.document().selection();
        let span = selection.span();
        let inside = span.start() <= offset && offset < span.end();
        if !inside {
            self.select(offset, false);
            // A right-click is not the start of a drag, and a run of them is not a double-click.
            self.clicks.reset();
            self.granularity = Granularity::Character;
            self.anchor_span = None;
        }
        true
    }

    /// Places the caret if it has to, then shows the native context menu at the pointer.
    ///
    /// The popup runs its own modal message loop, so this does not return until the menu closes —
    /// and whatever was chosen has by then already been delivered through the same channel the menu
    /// bar's commands use. There is nothing to return but "the frame changed", which it did if the
    /// caret moved.
    fn popup_context_menu(&mut self, x: f32, y: f32) -> bool {
        if !self.right_click_at(x, y) {
            return false;
        }
        let state = self.context_state();

        let (Some(window), Some(menu)) = (self.window.as_ref(), self.menu.as_mut()) else {
            // No window means no popup, and the caret has still been placed — which is the useful
            // half of a right-click and what a test sees.
            return true;
        };
        let scale = window.scale_factor();
        let Ok(handle) = window.window_handle() else {
            return true;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return true;
        };
        // Physical pixels: the popup is positioned by Windows, which knows nothing about the
        // device-independent space the rest of this file works in.
        menu.show_context_menu(
            win32.hwnd.get(),
            f64::from(x) * scale,
            f64::from(y) * scale,
            state,
        );
        true
    }

    /// What the context menu's entries should be able to do, given what is selected.
    #[must_use]
    pub fn context_state(&mut self) -> mdedit_menu_win::ContextState {
        mdedit_menu_win::ContextState {
            has_selection: !self.document().selection().is_collapsed(),
            // Asked of the clipboard rather than assumed. A paste with nothing there is harmless,
            // but a Paste that is greyed when there *is* something is the kind of small wrongness
            // that makes a menu feel broken.
            can_paste: self.clipboard.read().is_some_and(|text| !text.is_empty()),
        }
    }

    /// Acts on a press that landed on the scrollbar.
    ///
    /// The caret does not move for any of this. Scrolling is a view movement, exactly as the page
    /// keys already are, and a scrollbar that moved the caret would lose the user's place every time
    /// they looked somewhere else in the document.
    fn click_scrollbar(&mut self, hit: scrollbar::Hit, y: f32) -> bool {
        match hit {
            scrollbar::Hit::Thumb => {
                // Where in the thumb the press landed, kept so dragging moves the thumb *with* the
                // pointer rather than jumping it so its top is under the pointer.
                self.thumb_grab = self.scrollbar.map(|bar| y - bar.thumb_top);
                // Not `self.dragging`: that one extends a text selection, and this must not.
                true
            }
            scrollbar::Hit::PageUp => {
                self.page(-1.0);
                true
            }
            scrollbar::Hit::PageDown => {
                self.page(1.0);
                true
            }
        }
    }

    /// Moves the view to follow a thumb being dragged, if one is.
    ///
    /// Returns whether anything moved, so the caller knows whether the press belonged to the bar.
    ///
    /// The mapping is against the *current* content height, not the one the drag started with. As a
    /// drag reaches parts of the document nobody has measured, chunks are laid out and the estimated
    /// total corrects — so the thumb shifts slightly under the pointer. That is the design working;
    /// see [`crate::scrollbar`] for why it must not be smoothed away.
    fn drag_thumb(&mut self, y: f32) -> bool {
        let (Some(grab), Some(bar)) = (self.thumb_grab, self.scrollbar) else {
            return false;
        };
        let offset = scrollbar::offset_for_thumb(&bar, self.viewport.content, y - grab);
        self.scroll
            .set(offset, self.viewport.content, self.viewport.height);
        // The caret is wherever it was, and must not be dragged back on the next frame.
        self.follow_caret = false;
        true
    }

    /// Continues whichever drag is in progress, if either is.
    ///
    /// The thumb first, and exclusively: the two cannot both be happening, and testing the text
    /// drag first would let a thumb drag that wandered over the document start selecting it.
    fn on_pointer_moved(&mut self, point: (f32, f32)) {
        if self.drag_thumb(point.1) {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
            return;
        }

        if self.dragging && self.drag_to(point.0, point.1) {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }

    /// Whether the pointer is dragging out a text selection.
    ///
    /// A right-click must not start one — the pointer moving afterwards should do nothing, not
    /// select. There is no other way to observe that, since the guard lives in the winit handler.
    #[must_use]
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// Whether a scrollbar thumb is being dragged.
    #[must_use]
    pub fn dragging_thumb(&self) -> bool {
        self.thumb_grab.is_some()
    }

    /// Where the scrollbar is, or `None` when the document fits. For a test.
    #[must_use]
    pub fn scrollbar(&self) -> Option<Bar> {
        self.scrollbar
    }

    /// Steps the text size up or down. Negative steps shrink it.
    ///
    /// A multiplicative ladder rather than a fixed number of pixels: the difference between 16 and
    /// 17 px matters far more than between 39 and 40, and a ratio is what makes each press feel like
    /// the same amount of change.
    pub fn zoom_by(&mut self, steps: i32) -> bool {
        /// One press. 1.1 gives about seven presses across the useful range, which is enough to land
        /// where you meant without being enough to lose count.
        const STEP: f32 = 1.1;

        #[expect(
            clippy::cast_precision_loss,
            reason = "steps is a handful of key presses, not a count of anything"
        )]
        let factor = STEP.powf(steps as f32);
        self.set_zoom(self.settings.config().zoom * factor)
    }

    /// Puts the text back to its normal size.
    pub fn reset_zoom(&mut self) -> bool {
        self.set_zoom(1.0)
    }

    /// How much larger or smaller than normal the text currently is.
    #[must_use]
    pub fn zoom(&self) -> f32 {
        self.settings.config().zoom
    }

    /// Rescales the text, and throws away everything that was laid out at the old size.
    ///
    /// # The cache discipline *is* this feature
    ///
    /// Text sizes come from the theme at layout time and are **not** in [`ChunkKey`] — which carries
    /// a content hash, a width, the drawing mode and whether the chunk holds the caret. So changing
    /// the size without clearing leaves every cached chunk drawing at the old one.
    ///
    /// The choice was to put the size in every key or to clear on zoom. **Clear**, because it is
    /// correct by construction and because a size in every key is a per-frame cost paid for ever, to
    /// serve an event that happens perhaps twice in a session.
    ///
    /// Five things hold laid-out text and four of them are cleared here: the chunk layouts, the
    /// measured heights, the gutter's digits, the status bar's two labels, and the file pane's row
    /// labels. The tab strip's labels are measured every frame and so need nothing — which is worth
    /// stating, because "it is already fine" and "it was forgotten" look identical from here.
    ///
    /// # And the viewport must not jump
    ///
    /// The anchor holds a chunk's content hash and how far *in pixels* the viewport's top sits into
    /// it. Those pixels mean something different at a new size, so both they and the scroll offset
    /// are scaled by the same ratio the text was. Without that, zooming halfway down a document
    /// throws the reader somewhere else in it.
    fn set_zoom(&mut self, zoom: f32) -> bool {
        let previous = self.settings.config().zoom;
        let zoom = zoom.clamp(crate::config::MIN_ZOOM, crate::config::MAX_ZOOM);
        if (zoom - previous).abs() < f32::EPSILON {
            return false;
        }

        self.settings.update(|config| config.zoom = zoom);
        // Read back rather than assumed: the settings clamp too, and the theme must follow what was
        // actually stored.
        let zoom = self.settings.config().zoom;
        self.theme.body_size = Theme::default().body_size * zoom;

        self.layouts.clear();
        self.heights.clear();
        self.digits = None;
        self.status_labels = [None, None, None];
        self.row_labels.clear();
        self.toolbar_labels.clear();
        // The sixth thing holding laid-out text. The layout caches above are cleared because the
        // text size is not in their key; this one is cleared for the same reason.
        self.marks.clear();
        // Every row is a different width at a different text size, so the widest is no longer known.
        self.widest = 0.0;

        let ratio = zoom / previous;
        if let Some(anchor) = self.anchor.as_mut() {
            anchor.into_chunk *= ratio;
            anchor.offset *= ratio;
        }
        self.scroll.set(
            self.scroll.offset() * ratio,
            self.viewport.content * ratio,
            self.viewport.height,
        );
        // The pane's rows are a fixed height, so its scroll is unaffected — but its clamp is against
        // a row count that has not changed either, and re-clamping costs nothing.
        self.pane_scroll = self.pane_scroll.max(0.0);
        true
    }

    /// Sets the double-click interval, which on Windows is a system setting.
    ///
    /// Supplied by the binary rather than read here: it is `GetDoubleClickTime`, and this crate is
    /// not where Win32 calls live. See [`crate::clicks`] for why it is a setting worth honouring.
    pub fn with_double_click_interval(&mut self, interval: std::time::Duration) {
        self.clicks = Clicks::with_interval(interval);
    }

    /// Resolves a click in the file pane.
    fn click_pane(&mut self, x: f32, y: f32) -> bool {
        let Some(hit) = self.pane_hit(x, y) else {
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

    /// Resolves a click on the bar that follows the selection.
    ///
    /// **The selection survives it.** That is what lets somebody bold a run and then italicise it
    /// without selecting twice, and it is why the bar stays up afterwards with its lit states
    /// changed rather than vanishing on first use. A press outside it is somebody else's click and
    /// dismisses the bar on its way past — handled by the caller, not here.
    fn click_floating(&mut self, x: f32, y: f32) -> bool {
        let Some((left, top, width)) = self.floating_at else {
            return false;
        };
        // The bar's whole rectangle, not just its buttons: a press in the padding between two of
        // them is still a press on the bar, and must not fall through to the text underneath.
        if !(top..top + toolbar::HEIGHT).contains(&y) || !(left..left + width).contains(&x) {
            return false;
        }

        if let Some(action) = toolbar::hit_test(&self.floating_buttons, x, y - top) {
            self.perform(action);
        }
        true
    }

    /// What a window position means to the file pane, if anything.
    ///
    /// The rects are in the pane's own coordinates and the pane starts below the formatting bar, so
    /// the window's y has to come back to the pane's first. Three callers need that — a click, a
    /// wheel notch and a middle-click — and doing it in three places is how two of them end up
    /// disagreeing.
    fn pane_hit(&self, x: f32, y: f32) -> Option<pane::Hit> {
        pane::hit_test(&self.row_rects, x, y - self.toolbar_height())
    }

    /// Resolves a click in the formatting bar.
    ///
    /// **Returns true for any click inside the bar's height**, including the gaps between buttons.
    /// A click on chrome must never reach the document, and "it fell between two buttons" is not a
    /// reason to start a selection in the text underneath.
    fn click_toolbar(&mut self, x: f32, y: f32) -> bool {
        if self.toolbar_buttons.is_empty() {
            return false;
        }
        let top = self.strip_height();
        if !(top..top + toolbar::HEIGHT).contains(&y) {
            return false;
        }

        if let Some(action) = toolbar::hit_test(&self.toolbar_buttons, x, y - top) {
            self.perform(action);
        }
        true
    }

    /// Does what a toolbar button says, which is always what its chord already does.
    ///
    /// A second door, never a second implementation — every arm here is a method a keystroke also
    /// reaches, so there is no behaviour that exists only when clicked.
    pub fn perform(&mut self, action: toolbar::Action) -> bool {
        use mdedit_document::format::{Block, Inline};

        match action {
            toolbar::Action::Bold => self.document_mut().toggle_inline(Inline::Bold),
            toolbar::Action::Italic => self.document_mut().toggle_inline(Inline::Italic),
            toolbar::Action::Strikethrough => {
                self.document_mut().toggle_inline(Inline::Strikethrough)
            }
            toolbar::Action::Code => self.document_mut().toggle_inline(Inline::Code),
            toolbar::Action::Heading(level) => self.document_mut().set_block(Block::Heading(level)),
            toolbar::Action::Bullet => self.document_mut().set_block(Block::Bullet),
            toolbar::Action::Ordered => self.document_mut().set_block(Block::Ordered),
            toolbar::Action::Task => self.document_mut().set_block(Block::Task),
            toolbar::Action::Quote => self.document_mut().toggle_quote(),
            toolbar::Action::Fence => self.document_mut().toggle_fence(),
            toolbar::Action::Rule => self.document_mut().insert_rule(),
            toolbar::Action::Link => self.link(),
        }
    }

    /// Shows or hides the formatting bar.
    ///
    /// Per-run. MDEDIT-52 left persistence to the settings story, and the bar changes the content
    /// rect's height — which invalidates every cached layout width, exactly as the pane toggle does.
    /// Expected rather than chased.
    pub fn toggle_toolbar(&mut self) {
        self.toolbar = !self.toolbar;
        self.layouts.clear();
        self.heights.clear();
        self.refresh_view_menu();
    }

    /// Whether the formatting bar is showing.
    #[must_use]
    pub fn toolbar_shown(&self) -> bool {
        self.toolbar
    }

    /// Opens the find bar, or refocuses it if it is already open.
    ///
    /// A selection in the document is carried into the field, which is what every editor does with
    /// Ctrl+F over a highlighted word and is the difference between the chord being useful and being
    /// a box you then have to type into. Multi-line selections come in flattened rather than
    /// refused — see [`Field::set_text`](crate::field::Field::set_text).
    ///
    /// The bar takes room out of the content rect, so every cached layout width is now wrong. Cleared
    /// rather than chased, exactly as the pane and the formatting bar do it.
    pub fn open_find(&mut self) -> bool {
        self.open_bar(find::Mode::Find)
    }

    /// Opens the bar with a replacement field as well. Ctrl+H.
    pub fn open_replace(&mut self) -> bool {
        self.open_bar(find::Mode::Replace)
    }

    /// Opens the bar as a line-number box. Ctrl+G.
    ///
    /// The line field starts empty rather than pre-filled with where the caret is: a box already
    /// holding "42" invites you to edit it, and typing a line number from scratch is one gesture.
    pub fn open_goto(&mut self) -> bool {
        self.find.line.clear();
        self.open_bar(find::Mode::GoTo)
    }

    /// The three chords' shared body.
    fn open_bar(&mut self, mode: find::Mode) -> bool {
        let seed = (mode != find::Mode::GoTo)
            .then(|| self.selected_text())
            .flatten()
            .filter(|text| !text.trim().is_empty() && text.len() <= FIND_SEED_LIMIT);

        self.find_open = true;
        self.find.mode = mode;
        // Switching from Replace back to Find leaves focus on a field that is no longer drawn.
        self.find.focus = find::Focus::Primary;
        match seed {
            Some(text) => self.find.field.set_text(&text),
            // Reopened with nothing selected: keep the previous term and select it, so Ctrl+F then
            // Enter repeats the last search and Ctrl+F then typing starts a new one.
            None => self.find.active_mut().select_all(),
        }
        self.find_focused = true;
        self.dismiss_floating();
        self.layouts.clear();
        self.heights.clear();
        true
    }

    /// Closes the find bar and gives the document the keyboard back.
    ///
    /// Returns whether it was open, which is what lets Esc fall through to the floating bar's
    /// dismissal when it was not.
    pub fn close_find(&mut self) -> bool {
        if !self.find_open {
            return false;
        }
        self.find_open = false;
        self.find_focused = false;
        self.find_layout = None;
        self.layouts.clear();
        self.heights.clear();
        true
    }

    /// Whether the find bar is open.
    #[must_use]
    pub fn find_open(&self) -> bool {
        self.find_open
    }

    /// Whether keys are going to the find field rather than to the document.
    #[must_use]
    pub fn find_focused(&self) -> bool {
        self.find_open && self.find_focused
    }

    /// The find bar, for a test to look at what it found and what it says.
    #[must_use]
    pub fn find_bar(&self) -> Option<&find::Bar> {
        self.find_open.then_some(&self.find)
    }

    /// Where the bar's parts were drawn last frame.
    #[must_use]
    pub fn find_layout(&self) -> Option<&find::Layout> {
        self.find_layout.as_ref()
    }

    /// Sends one key press to the find field, and searches again if it changed the term.
    ///
    /// Reachable without a winit event on purpose — see [`field::Press`].
    pub fn press_find(&mut self, press: &field::Press, shift: bool) -> bool {
        if !self.find_open {
            return false;
        }
        self.find.press(press, shift);
        // Rescanning is [`find::Bar::refresh`]'s job and happens on the way to drawing, because the
        // document's text is not held here — `measure` is what has it. So this only has to say the
        // frame is stale.
        true
    }

    /// Replaces the current match, then moves to the next one.
    ///
    /// Returns whether anything was replaced. **The caret has to be on a match already** — pressing
    /// Replace with the caret elsewhere moves to a match rather than replacing whatever the caret
    /// happens to be near, which is the behaviour that loses work.
    pub fn replace_one(&mut self) -> bool {
        self.refresh_find();
        if !self.find_open || self.find.mode != find::Mode::Replace {
            return false;
        }

        let selected = self.document().selection().span();
        if find::index_of(self.find.matches(), selected).is_none() {
            // Not on one yet. Landing on the next match is what the user meant, and the second press
            // then replaces it.
            return self.find_next();
        }

        let replacement = self.find.replace.text().to_owned();
        if self.document_mut().replace(selected, &replacement).is_err() {
            return false;
        }
        // Past the replacement rather than at its start, so a replacement that still contains the
        // search term does not match itself on the way to the next one.
        self.refresh_find();
        self.find_next();
        true
    }

    /// Replaces every match, as one undo step. Returns how many.
    ///
    /// The transaction is [`Document::replace_all`]'s, which MDEDIT-63 built and this ticket was
    /// waiting for: without it a 500-match replace would cost 500 presses of Ctrl+Z.
    pub fn replace_all(&mut self) -> usize {
        self.refresh_find();
        if !self.find_open || self.find.mode != find::Mode::Replace {
            return 0;
        }
        let sites = self.find.matches().to_vec();
        let replacement = self.find.replace.text().to_owned();
        let done = self.document_mut().replace_all(&sites, &replacement);
        if done > 0 {
            self.follow_caret = true;
        }
        done
    }

    /// Moves the caret to the line the Go to Line field names.
    ///
    /// Counts from 1, as the status bar displays, and clamps past the end rather than refusing: a
    /// number larger than the document has lines means "the end", which is what it looks like it
    /// should mean.
    pub fn go_to_line(&mut self) -> bool {
        if !self.find_open || self.find.mode != find::Mode::GoTo {
            return false;
        }
        let Some(line) = self.find.line_number() else {
            return false;
        };
        let rope = self.document().rope();
        let last = rope.len_lines().saturating_sub(1);
        let at = rope.line_to_byte(line.saturating_sub(1).min(last));

        self.document_mut()
            .set_selection(Selection::caret(ByteOffset::new(at)));
        // Not a search result, so the floating bar's rule does not apply — and it is a caret rather
        // than a selection anyway, which that rule already declines.
        self.found_selection = false;
        self.follow_caret = true;
        true
    }

    /// Selects the next match, wrapping at the end. Returns whether the caret moved.
    pub fn find_next(&mut self) -> bool {
        self.go_to_match(true)
    }

    /// Selects the previous match, wrapping at the start.
    pub fn find_previous(&mut self) -> bool {
        self.go_to_match(false)
    }

    /// Moves the selection to the match on one side of it.
    ///
    /// Searching from the *far* end of the current selection in each direction is what stops Find
    /// Next landing on the match it is already sitting on: forward from its end, back from its
    /// start.
    fn go_to_match(&mut self, forward: bool) -> bool {
        // The match list is refreshed while drawing, so a chord pressed before the first frame after
        // a change would navigate a stale list. Refreshing here costs a comparison in the ordinary
        // case and is the difference between correct and nearly.
        self.refresh_find();

        if !self.find_open {
            return false;
        }
        let bar = &self.find;
        let selected = self.document().selection().span();
        let at = if forward {
            find::next(bar.matches(), selected.end())
        } else {
            find::previous(bar.matches(), selected.start())
        };
        let Some(span) = at.map(|at| bar.matches()[at]) else {
            return false;
        };

        // Anchor first, head last, so the head is the end going forwards — which is what `go_to_match`
        // reads back on the next press, and what puts the caret where the reveal wants it.
        self.document_mut()
            .set_selection(Selection::new(span.start(), span.end()));
        self.found_selection = true;
        self.follow_caret = true;
        true
    }

    /// Turns case-sensitivity on or off.
    pub fn toggle_find_case(&mut self) -> bool {
        if !self.find_open {
            return false;
        }
        self.find.case_sensitive = !self.find.case_sensitive;
        true
    }

    /// Brings the match list up to date with the document, the needle and the case flag.
    ///
    /// Costs a `to_string` of the buffer when something has moved and three comparisons when nothing
    /// has. With the bar closed it is not reached at all.
    fn refresh_find(&mut self) {
        if !self.find_open {
            return;
        }
        let source = self.document().to_string();
        let document = self.find_document();
        let revision = self.document().revision();
        self.find.refresh(&source, document, revision);
    }

    /// Which document the match list's offsets are into.
    ///
    /// The active tab, or a value no tab index can be for the scratch buffer. Held as part of the
    /// match list's key rather than hooked into the eight paths that activate a tab — the same
    /// answer, and for the same reason, as the floating bar's `floating_for`.
    fn find_document(&self) -> usize {
        self.workspace.active().unwrap_or(usize::MAX)
    }

    /// Height the find bar occupies, which is nothing when it is closed and two rows for Replace.
    fn find_height(&self) -> f32 {
        if self.find_open {
            find::height(self.find.mode)
        } else {
            0.0
        }
    }

    /// Draws the find bar below whatever chrome is above it, and remembers where its parts went.
    fn draw_find(&mut self, quads: &mut Vec<QuadInstance>, width: f32, scale: f32) {
        self.find_layout = None;
        if !self.find_open {
            return;
        }

        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        let mode = self.find.mode;
        // The button glyphs share the formatting bar's cache: they are short, fixed strings laid out
        // at the same size, and a second cache of the same kind would be the per-frame layout trap
        // paid for a sixth time. `set_zoom` already clears it.
        let mut widths = Vec::with_capacity(find::buttons(mode).len());
        for button in find::buttons(mode) {
            let glyph = button.glyph().to_owned();
            if !self.toolbar_labels.contains_key(&glyph) {
                if let Some(label) = self.lay_out_label(&glyph, style) {
                    self.toolbar_labels.insert(glyph.clone(), label);
                }
            }
            widths.push(self.toolbar_labels.get(&glyph).map_or(0.0, |l| l.width));
        }

        let selection = self.document().selection().span();
        let count = self.find.count_label(selection);
        // Laid out per frame rather than cached, because it changes with every keystroke and every
        // navigation — a cache keyed on "3 of 17" would hold a new entry per match in the document.
        // One short string a frame is what the tab strip's labels already cost.
        let count_label = count
            .as_deref()
            .and_then(|text| self.lay_out_label(text, style));
        let count_width = count_label.as_ref().map_or(0.0, |label| label.width);

        let logical_width = width / scale;
        let layout = find::layout(mode, logical_width, count_width, &widths);
        let top = self.strip_height() + self.toolbar_height();

        quads.push(Self::solid(
            [0.0, top * scale, width, find::height(mode) * scale],
            self.theme.strip,
        ));

        // The offset from a row's top to the text's baseline, taken as a number rather than as a
        // closure over `self.theme` — the closure would hold a borrow across every draw call below.
        let baseline_drop = top + find::HEIGHT / 2.0 + self.theme.body_size * 0.3;
        let baseline_of = move |row: f32| baseline_drop + row;
        // Each field gets a box drawn behind it, so it reads as somewhere to type rather than as a
        // gap in the bar.
        for (row, box_, which) in [
            Some((0.0, layout.field, find::Focus::Primary)),
            layout
                .replace
                .map(|box_| (find::HEIGHT, box_, find::Focus::Secondary)),
        ]
        .into_iter()
        .flatten()
        {
            quads.push(Self::solid(
                [
                    box_.0 * scale,
                    (top + row + FIND_INSET) * scale,
                    box_.1 * scale,
                    (find::HEIGHT - FIND_INSET * 2.0) * scale,
                ],
                self.theme.background,
            ));
            self.draw_find_term(
                quads,
                TermAt {
                    box_,
                    which,
                    baseline: baseline_of(row),
                },
                scale,
            );
        }

        if let Some(label) = count_label {
            let where_ = LabelAt {
                x: layout.count,
                baseline: baseline_of(0.0),
                scale,
                colour: self.theme.marker,
            };
            self.draw_label_glyphs(quads, &label.glyphs, where_);
        }

        let case = self.find.case_sensitive;
        for rect in &layout.buttons.clone() {
            if rect.button == find::Button::Case && case {
                quads.push(Self::solid(
                    [
                        rect.x * scale,
                        (top + rect.y + 3.0) * scale,
                        rect.width * scale,
                        (find::HEIGHT - 6.0) * scale,
                    ],
                    self.theme.tab_active,
                ));
            }
            let Some(label) = self.toolbar_labels.get(rect.button.glyph()) else {
                continue;
            };
            let where_ = LabelAt {
                x: rect.x + (rect.width - label.width) / 2.0,
                baseline: baseline_of(rect.y),
                scale,
                colour: self.theme.text,
            };
            let glyphs = label.glyphs.clone();
            self.draw_label_glyphs(quads, &glyphs, where_);
        }

        self.find_layout = Some(layout);
    }

    /// Draws what has been typed into the field, its selection, and its caret.
    ///
    /// Clipped to the box by dropping glyphs past its right edge — truncated rather than scrolled.
    /// A field that scrolled with the caret needs an offset of its own and a measurement pass to
    /// place it; a search term long enough to need one is rare enough to leave until it is asked
    /// for.
    fn draw_find_term(&mut self, quads: &mut Vec<QuadInstance>, at: TermAt, scale: f32) {
        let TermAt {
            box_,
            which,
            baseline,
        } = at;
        let field = match (self.find.mode, which) {
            (find::Mode::GoTo, _) => &self.find.line,
            (_, find::Focus::Secondary) => &self.find.replace,
            _ => &self.find.field,
        };
        let text = field.text().to_owned();
        let caret_at = field.caret();
        let (from, to) = field.selection();
        // Only the focused field of the focused bar shows a caret and a selection. Two carets on
        // screen at once is the classic way a focus model that is not really one shows itself.
        let focused = self.find_focused && self.find.focus == which;

        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        let label = (!text.is_empty())
            .then(|| self.lay_out_label(&text, style))
            .flatten();

        let left = box_.0 + FIND_INSET * 2.0;
        let right = box_.0 + box_.1 - FIND_INSET;
        // Where a byte sits along the laid-out term: the glyph that starts at or after it, or past
        // the last one. The same rule `row_selection` uses, and it has to be, or the caret would sit
        // half a glyph away from the highlight.
        let x_of = |label: Option<&Label>, offset: usize| -> f32 {
            label.map_or(0.0, |label| {
                label
                    .glyphs
                    .iter()
                    .find(|glyph| glyph.source.start().get() >= offset)
                    .map_or_else(
                        || {
                            label
                                .glyphs
                                .last()
                                .map_or(0.0, |glyph| glyph.pos.x + glyph.advance)
                        },
                        |glyph| glyph.pos.x,
                    )
            })
        };

        if from < to && focused {
            let (start, end) = (x_of(label.as_ref(), from), x_of(label.as_ref(), to));
            quads.push(Self::solid(
                [
                    (left + start) * scale,
                    (baseline - self.theme.body_size * 0.7) * scale,
                    (end - start).max(1.0) * scale,
                    (self.theme.body_size * 0.95) * scale,
                ],
                self.theme.selection,
            ));
        }

        if let Some(label) = &label {
            let glyphs: Vec<_> = label
                .glyphs
                .iter()
                .take_while(|glyph| left + glyph.pos.x < right)
                .copied()
                .collect();
            let where_ = LabelAt {
                x: left,
                baseline,
                scale,
                colour: self.theme.text,
            };
            self.draw_label_glyphs(quads, &glyphs, where_);
        }

        if focused {
            quads.push(Self::solid(
                [
                    (left + x_of(label.as_ref(), caret_at)) * scale,
                    (baseline - self.theme.body_size * 0.7) * scale,
                    CARET_WIDTH * scale,
                    (self.theme.body_size * 0.95) * scale,
                ],
                self.theme.caret,
            ));
        }
    }

    /// Resolves a click on the find bar. Returns whether it was one.
    fn click_find(&mut self, x: f32, y: f32) -> bool {
        let Some(layout) = self.find_layout.clone() else {
            return false;
        };
        let top = self.strip_height() + self.toolbar_height();
        let inside = y - top;
        if !(0.0..find::height(layout.mode)).contains(&inside) {
            return false;
        }

        if let Some(which) = layout.over_field(x, inside) {
            self.find_focused = true;
            self.find.focus = which;
            return true;
        }
        match layout.hit_test(x, inside) {
            Some(find::Button::Case) => {
                self.toggle_find_case();
            }
            Some(find::Button::Previous) => {
                self.find_previous();
            }
            Some(find::Button::Next) => {
                self.find_next();
            }
            Some(find::Button::Replace) => {
                self.replace_one();
            }
            Some(find::Button::ReplaceAll) => {
                let _ = self.replace_all();
            }
            Some(find::Button::Close) => {
                self.close_find();
            }
            // The bar's own background. Swallowed rather than let through, or a click on the strip
            // between two buttons would put the caret in the document underneath it.
            None => {}
        }
        true
    }

    /// Shows the tab at `index`, as a click on the strip would.
    ///
    /// The strip and the pane both reach `Workspace::activate` through their own hit-testing, and a
    /// test that wants a different tab should not have to synthesise a click in whichever chrome is
    /// currently showing to get one.
    pub fn show_tab(&mut self, index: usize) -> bool {
        if let Err(error) = self.workspace.activate(index) {
            crate::log_error!("could not open tab: {error}");
            return false;
        }
        true
    }

    /// Where the bar's buttons are, for a test.
    #[must_use]
    pub fn toolbar_buttons(&self) -> &[toolbar::ButtonRect] {
        &self.toolbar_buttons
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
        let (x, y) = self.content.to_document(x, y, self.viewport_offset());
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
        self.found_selection = false;
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
        self.click_at_time(x, y, Instant::now())
    }

    /// [`click_at`](Self::click_at) with the press time said explicitly.
    ///
    /// A test cannot wait half a second between clicks and should not have to, so the moment a press
    /// happened is a parameter rather than a call to the clock. See [`crate::clicks`].
    pub fn click_at_time(&mut self, x: f32, y: f32, at: Instant) -> bool {
        self.found_selection = false;
        let Some(offset) = self.offset_at_point(x, y) else {
            return false;
        };

        // Shift+click extends the existing selection and is not part of a run. Counting it would
        // make shift-clicking twice in a place select a word, which is not what either gesture
        // means.
        if self.modifiers.shift_key() {
            self.clicks.reset();
            self.granularity = Granularity::Character;
            self.anchor_span = None;
            self.select(offset, true);
            self.dragging = true;
            return true;
        }

        self.granularity = Granularity::of(self.clicks.press(at, x, y));
        let span = self.unit_at(offset);
        self.anchor_span = Some(span);

        match self.granularity {
            // Unchanged from before this existed, deliberately: a slow click must behave exactly as
            // it always did, and the counting wraps the old path rather than replacing it.
            Granularity::Character => self.select(offset, false),
            Granularity::Word | Granularity::Line => self.select_span(span),
        }
        self.dragging = true;
        true
    }

    /// Drags the far end of the selection to a point in the window.
    ///
    /// The unit the drag started with sticks: after a double-click the selection grows by whole
    /// words, after a triple-click by whole lines. That is the part casual implementations leave
    /// out, and the part people notice immediately, because dragging out of the word they just
    /// double-clicked otherwise snaps back to a character selection.
    pub fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some(offset) = self.offset_at_point(x, y) else {
            return false;
        };
        let Some(anchor) = self
            .anchor_span
            .filter(|_| !matches!(self.granularity, Granularity::Character))
        else {
            self.select(offset, true);
            return true;
        };

        // Both ends of the unit under the pointer, so dragging backwards takes the whole of the word
        // the pointer is in rather than the part of it after the pointer.
        let reached = self.unit_at(offset);
        if reached.start() < anchor.start() {
            self.select_between(anchor.end(), reached.start());
        } else {
            self.select_between(anchor.start(), reached.end());
        }
        true
    }

    /// The word or line at `offset`, according to the drag's current unit.
    fn unit_at(&self, offset: ByteOffset) -> ByteSpan {
        match self.granularity {
            Granularity::Character => ByteSpan::new(offset, offset),
            Granularity::Word => self.document().word_at(offset),
            Granularity::Line => self.document().source_line(offset),
        }
    }

    /// Selects a span, with the caret at its end.
    fn select_span(&mut self, span: ByteSpan) {
        self.select_between(span.start(), span.end());
    }

    /// Selects from `anchor` to `head`, leaving the caret at `head`.
    fn select_between(&mut self, anchor: ByteOffset, head: ByteOffset) {
        self.document_mut()
            .set_selection(Selection::new(anchor, head));
        self.desired_x = None;
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
            offset,
            scale,
        } = at;
        let (band_top, band_bottom) = content.visible(offset);
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
            // The matches this chunk covers, taken once per chunk rather than once per row, and
            // binary-searched out of the list rather than filtered. Both matter for the same reason:
            // a document with ten thousand matches must cost what is on screen, not what is in it.
            let found: Vec<ByteSpan> = if self.find_open {
                find::within(self.find.matches(), chunk.span).to_vec()
            } else {
                Vec::new()
            };
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

                let here = Row {
                    cached: &cached,
                    chunk,
                    text: &frame.text,
                    index: row,
                    top: top_of_row,
                    height: line.height,
                    chunk_top,
                };
                self.draw_matches(quads, &found, here, at);
                self.draw_row_selection(quads, selected, here, at);
                if self.whitespace {
                    self.draw_marks(quads, here, baseline, at);
                }

                self.draw_underlines(quads, &runs, here, at);

                for glyph in &line.glyphs {
                    let Some(entry) = self.entry_for(glyph.key, scale) else {
                        continue;
                    };
                    let colour =
                        view::colour_at(&runs, cached.rebase(glyph, chunk.span), self.theme.text);
                    let (x, y) = content.to_window(glyph.pos.x, glyph.pos.y + chunk_top, offset);
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

    /// Draws the rules under one visual row's underlined runs (MDEDIT-73).
    ///
    /// Where each rule sits and how thick it is came from the font, through the text layer — the
    /// renderer turns the rectangle into a quad and decides nothing about it but the colour, which
    /// is the run's own so a rule cannot disagree with the text above it.
    fn draw_underlines(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        runs: &[view::ViewRun<'_>],
        row: Row<'_>,
        at: Placement,
    ) {
        for rule in &row.cached.layout.lines[row.index].underlines {
            let colour = view::colour_at(
                runs,
                row.cached.rebase_offset(rule.source, row.chunk.span),
                self.theme.text,
            );
            let (x, y) = at
                .content
                .to_window(rule.x, rule.top + row.chunk_top, at.offset);
            quads.push(Self::solid(
                [
                    x * at.scale,
                    y * at.scale,
                    rule.width * at.scale,
                    // A thin font's rule is under a pixel at 100%, and a rule that rounds to
                    // nothing is a cue that is not there.
                    (rule.thickness * at.scale).max(1.0),
                ],
                colour,
            ));
        }
    }

    /// Draws the whitespace marks for one visual row (MDEDIT-46).
    ///
    /// # Nothing here changes the layout, and that is the whole design
    ///
    /// Each mark is stamped into the cell the invisible character already occupies, at the advance
    /// the layout already gave it, so no glyph moves, no line re-wraps and no hit test shifts by a
    /// pixel. That is why the toggle clears **no layout cache** — MDEDIT-46's plan expected it to,
    /// by analogy with zoom, and the analogy does not hold: zoom changes what the text engine is
    /// asked for, and this changes nothing it is asked for at all.
    ///
    /// The line-ending mark is drawn *past* the last glyph rather than in a cell, because the
    /// terminator has no cell — it is not in the layout. It therefore adds no column the caret can
    /// enter and nothing the hit test can land on: those come from the layout, which has not been
    /// consulted.
    fn draw_marks(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        row: Row<'_>,
        baseline: f32,
        at: Placement,
    ) {
        let chunk_top = baseline - row.cached.layout.lines[row.index].baseline;
        let line = &row.cached.layout.lines[row.index];

        // In-cell marks: one per invisible character the layout gave a position to.
        for glyph in &line.glyphs {
            let source = row.cached.rebase(glyph, row.chunk.span);
            let Some(character) = row.text[source.get()..].chars().next() else {
                continue;
            };
            let Some(mark) = view::mark_for(character) else {
                continue;
            };
            // Centred in the cell the character already has. `glyph.advance` is the layout's answer
            // and is never recomputed from the mark.
            let Some(drawn) = self.mark(mark) else {
                continue;
            };
            let offset = (glyph.advance - drawn.advance) / 2.0;
            self.stamp(
                quads,
                drawn,
                MarkAt {
                    pos: (glyph.pos.x + offset, glyph.pos.y + chunk_top),
                    colour: self.theme.marker,
                    at,
                },
            );
        }

        // The line ending, past the end of the row. An empty line has no glyphs, so its terminator
        // is found from where the row starts rather than from where it ends.
        // Two shapes, and they need different questions asked. A row with glyphs is asked what comes
        // *before* the offset past its last one, because the engine may or may not have given the
        // terminator a glyph. A row with none is an empty line, whose terminator is exactly *at* its
        // start — asking `before` there would walk back into the previous line and mark it twice.
        let (ending, mut pen) = match line.glyphs.last() {
            Some(last) => {
                // Where the row's last character *ends*, taken from the document rather than from
                // the glyph's own span length. A backend is free to report a glyph's source however
                // suits it — a ligature covers several characters, a mark covers none — and asking
                // the text how long the character at that offset is cannot be wrong whatever it
                // reports.
                let start = row.cached.rebase(last, row.chunk.span).get();
                let mut after = start + row.text[start..].chars().next().map_or(0, char::len_utf8);
                let mut pen = last.pos.x + last.advance;

                // **Trailing whitespace, which is the thing people switch this on to find.**
                // DirectWrite does not shape whitespace at the end of a line, so there is no glyph
                // to stamp a mark into and these characters would be the only invisible ones the
                // feature could not show. Walked over from past the last glyph, so nothing that did
                // get a glyph can be marked twice.
                while let Some(character) = row.text[after..].chars().next() {
                    if character != ' ' && character != '\t' {
                        break;
                    }
                    if let Some(drawn) = view::mark_for(character).and_then(|mark| self.mark(mark))
                    {
                        self.stamp(
                            quads,
                            drawn,
                            MarkAt {
                                pos: (pen, line.baseline + chunk_top),
                                colour: self.theme.marker,
                                at,
                            },
                        );
                        pen += drawn.advance;
                    }
                    after += character.len_utf8();
                }

                (view::ending_before(row.text, after), pen)
            }
            None => (
                view::ending_at(row.text, Self::row_start(row.chunk, row.text, row.index)),
                0.0,
            ),
        };
        let Some(ending) = ending else {
            return;
        };
        let colour = if ending.is_suspect() {
            self.theme.status_error
        } else {
            self.theme.marker
        };
        for mark in ending.marks() {
            let Some(drawn) = self.mark(*mark) else {
                continue;
            };
            self.stamp(
                quads,
                drawn,
                MarkAt {
                    pos: (pen, line.baseline + chunk_top),
                    colour,
                    at,
                },
            );
            pen += drawn.advance;
        }
    }

    /// Where a row's text starts in the buffer, for a row with no glyphs to ask.
    ///
    /// The same arithmetic `row_selection` does, and wrong in the same way for a wrapped row — but a
    /// wrapped row is never empty, and this is only reached for one that is.
    fn row_start(chunk: blocks::Chunk, text: &str, row: usize) -> usize {
        let body = chunk.span.slice(text);
        chunk.span.start().get()
            + body
                .split_inclusive('\n')
                .take(row)
                .map(str::len)
                .sum::<usize>()
    }

    /// Puts one mark on screen at a document position.
    fn stamp(&mut self, quads: &mut Vec<QuadInstance>, mark: Mark, spot: MarkAt) {
        let MarkAt { pos, colour, at } = spot;
        let Some(entry) = self.entry_for(mark.key, at.scale) else {
            return;
        };
        let (x, y) = at.content.to_window(pos.0, pos.1, at.offset);
        quads.push(QuadInstance {
            rect: [
                x * at.scale + px_i(entry.left),
                y * at.scale + px_i(entry.top),
                px(entry.width),
                px(entry.height),
            ],
            uv: [px(entry.u), px(entry.v), px(entry.width), px(entry.height)],
            colour: [colour.r, colour.g, colour.b, colour.a],
        });
    }

    /// One mark, laid out on first use and kept until the zoom changes.
    ///
    /// `None` is cached too: a font with no Control Pictures should be asked once, not once per
    /// control character per frame.
    fn mark(&mut self, character: char) -> Option<Mark> {
        if let Some(known) = self.marks.get(&character) {
            return *known;
        }
        let mut text = [0_u8; 4];
        let run = StyledRun {
            text: character.encode_utf8(&mut text),
            style: TextStyle::body(self.theme.body_size),
            source: ByteSpan::empty_at(ByteOffset::ZERO),
        };
        let found = self.text.layout(&[run], f32::MAX).ok().and_then(|layout| {
            layout.glyphs().next().map(|glyph| Mark {
                key: glyph.key,
                advance: glyph.advance,
            })
        });
        self.marks.insert(character, found);
        found
    }

    /// Whether the encoding submenus apply here.
    ///
    /// False for an untitled buffer: there is nothing on disk to reopen, and Save As is where its
    /// encoding gets decided. Greyed rather than left to fail, like Format Document.
    ///
    /// **A path is the whole test, deliberately.** It does *not* require the tab to have opened —
    /// a windows-1252 file is not valid UTF-8, so it fails to open, and that is precisely the tab a
    /// user needs this menu for. Requiring an open document would grey the submenu on the one file
    /// it exists to rescue.
    #[must_use]
    pub fn can_choose_encoding(&self) -> bool {
        self.workspace.active_path().is_some()
    }

    /// Which of [`mdedit_file::choices`] the active document already uses, if any.
    ///
    /// `None` for an untitled buffer, and for a file in an encoding that is not on the list — which
    /// cannot happen through the menu but can through the command line, and ticking nothing is the
    /// honest answer either way.
    #[must_use]
    pub fn encoding_choice(&self) -> Option<usize> {
        let format = self
            .workspace
            .active_open()?
            .file
            .as_ref()
            .map(mdedit_file::LoadedFile::format)?;
        mdedit_file::choices()
            .iter()
            .position(|choice| choice.matches(format))
    }

    /// Re-reads the active document as a different encoding (MDEDIT-42).
    ///
    /// **Asks before discarding unsaved edits**, through the same prompt Close uses rather than a
    /// second one of its own — reopening is a discard, and the user should be asked in the words
    /// they already know.
    ///
    /// A file that is not valid in the chosen encoding leaves the buffer exactly as it was and says
    /// why: the read happens before anything is replaced, so a wrong guess costs nothing. Trying
    /// several is how somebody finds out what a file actually is.
    pub fn reopen_with(&mut self, index: usize) -> bool {
        let Some(choice) = mdedit_file::choices().get(index).copied() else {
            return false;
        };
        let Some(tab) = self.workspace.active() else {
            return false;
        };
        if !self.can_choose_encoding() {
            return false;
        }

        // Reopening throws away whatever is in the buffer, which is exactly what Close asks about —
        // so it asks with the same prompt, in words the user already knows, rather than growing a
        // second dialog that means the same thing.
        //
        // Only `Discard` goes ahead. `Save` would write the buffer out in the *old* encoding and
        // then immediately re-read it in the new one, which is a strange thing to have asked for;
        // Cancel plainly means stop.
        let modified = self
            .workspace
            .tabs()
            .get(tab)
            .is_some_and(mdedit_session::Tab::is_modified);
        if modified {
            let name = self
                .workspace
                .tabs()
                .get(tab)
                .map_or_else(String::new, mdedit_session::Tab::display_name);
            if self.prompt.ask(&name, crate::prompt::Closing::File) != Unsaved::Discard {
                return false;
            }
        }

        if let Err(error) = self.workspace.reopen_tab_as(tab, choice) {
            (self.say)(&format!(
                "Cannot read this file as {}: {error}",
                choice.label
            ));
            return false;
        }

        // Every cached layout is keyed on bytes that are no longer in the buffer.
        self.layouts.clear();
        self.heights.clear();
        // And so is the word count. It is normally left to age because a stale count is nearly
        // right; after a reopen it is not stale but *wrong*, and the commonest reopen is of a tab
        // that never opened at all — so the bar would read "0 words" over a document full of them
        // until the next keystroke. Clearing it makes the drawing path's catch-up recount.
        self.words = None;
        self.counted_for = None;
        self.follow_caret = true;
        true
    }

    /// Changes the encoding the active document will be saved in (MDEDIT-42).
    ///
    /// **The one deliberate exception to reproducing a file's bytes**, and the same shape as the
    /// other two: the invariant forbids the editor rewriting bytes the user did not ask it to, and
    /// this is the user asking. Nothing is written here — the choice takes effect at the next save,
    /// which is where content the encoding cannot hold is refused.
    pub fn save_with(&mut self, index: usize) -> bool {
        let Some(choice) = mdedit_file::choices().get(index).copied() else {
            return false;
        };
        let changed = self
            .workspace
            .active_open_mut()
            .and_then(|open| open.file.as_mut())
            .is_some_and(|file| file.set_encoding(choice));
        if changed {
            self.refresh_encodings_menu();
        }
        changed
    }

    /// Ticks whichever encoding the active document uses, and greys the submenus for a buffer that
    /// has none.
    ///
    /// Compared before it pushes, like the other two menu refreshes and for the same reason.
    fn refresh_encodings_menu(&mut self) {
        let wanted = mdedit_menu_win::EncodingState {
            enabled: self.can_choose_encoding(),
            current: self.encoding_choice(),
        };
        if self.encoding_state == Some(wanted) {
            return;
        }
        self.encoding_state = Some(wanted);
        if let Some(menu) = self.menu.as_ref() {
            menu.set_encoding_state(wanted);
        }
    }

    /// What the active document's line endings are.
    ///
    /// An untitled buffer has no file to have detected anything from, so it answers with the
    /// default until it is saved somewhere — the same fallback the status bar uses, and it has to be
    /// the same one or Enter and the bar would disagree.
    #[must_use]
    pub fn newline(&self) -> mdedit_file::Newline {
        self.workspace
            .active_open()
            .and_then(|open| open.file.as_ref())
            .map_or_else(mdedit_file::Newline::default, |file| {
                file.format().newline()
            })
    }

    /// Rewrites every line terminator in the active document to `to` (MDEDIT-59).
    ///
    /// # Why this is allowed to change bytes
    ///
    /// The same reason Format Document is, and the reasoning is MDEDIT-48's: the byte-preservation
    /// invariant forbids the editor rewriting bytes *the user did not ask it to*. This is the user
    /// asking. One `replace` inside a transaction, so one Ctrl+Z puts every byte back, and **save
    /// never does this on its own** — the whole point of the invariant is that a file's endings
    /// change only when somebody says so.
    ///
    /// Converts **every** terminator, including a lone `\r`, which is the one a user is most likely
    /// to be trying to get rid of and the one nothing else here produces.
    ///
    /// A document that already uses `to` throughout is left completely alone: no edit, so no undo
    /// press that appears to do nothing and no modified dot on a tab nobody changed.
    pub fn set_newline(&mut self, to: mdedit_file::Newline) -> bool {
        let source = self.document().rope().to_string();
        let Some(converted) = mdedit_file::normalise_newlines(&source, to) else {
            return false;
        };

        let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len()));
        let replaced = self.document_mut().transact(|document| {
            if let Err(error) = document.replace(whole, &converted) {
                crate::log_error!("could not convert the line endings: {error}");
                return false;
            }
            true
        });
        if !replaced {
            return false;
        }

        // The other half. Without it the bytes are one thing, the status bar says another, and the
        // next press of Enter puts back what was just taken out.
        if let Some(file) = self
            .workspace
            .active_open_mut()
            .and_then(|open| open.file.as_mut())
        {
            file.set_newline(to);
        }
        self.follow_caret = true;
        self.refresh_endings_menu();
        true
    }

    /// Ticks whichever line ending the active document uses.
    ///
    /// Compared before it pushes, like the Tools menu's enablement and for the same reason: which
    /// document is showing changes on more paths than anybody will remember to update.
    fn refresh_endings_menu(&mut self) {
        let wanted = mdedit_menu_win::EndingsState {
            lf: self.newline() == mdedit_file::Newline::Lf,
            crlf: self.newline() == mdedit_file::Newline::CrLf,
        };
        if self.endings_state == Some(wanted) {
            return;
        }
        self.endings_state = Some(wanted);
        if let Some(menu) = self.menu.as_ref() {
            menu.set_endings(wanted);
        }
    }

    /// Where the viewport sits over the document, on both axes.
    ///
    /// The one place the two scrolls are put together, so no caller can pass one and forget the
    /// other.
    fn viewport_offset(&self) -> Offset {
        Offset {
            x: self.pan.offset(),
            y: self.scroll.offset(),
        }
    }

    /// The width chunks are laid out at: the content width, or [`UNWRAPPED`] when wrap is off.
    ///
    /// One function so the layout, the cache key and the clamp cannot disagree about it — they are
    /// three readings of the same number, and a mismatch between the first two would draw a
    /// document laid out for a width it is not being shown at.
    fn wrap_width(&self) -> f32 {
        if self.wrap {
            self.content.width()
        } else {
            UNWRAPPED
        }
    }

    /// Whether long lines wrap.
    #[must_use]
    pub fn wrap(&self) -> bool {
        self.wrap
    }

    /// How far right the viewport has panned. Always zero while wrapping.
    #[must_use]
    pub fn pan(&self) -> f32 {
        self.pan.offset()
    }

    /// Turns word wrap on or off, and says whether anything changed (MDEDIT-49).
    ///
    /// **Every chunk is laid out at a different width afterwards**, so this clears the layout caches
    /// exactly as a zoom does — and for the same reason, since the width *is* in `ChunkKey` but the
    /// measured heights and the anchor are not. Turning wrap back on also returns the pan to zero:
    /// there is nothing off to the side any more, and leaving the offset would show a document
    /// scrolled past its own right edge.
    pub fn set_wrap(&mut self, wrap: bool) -> bool {
        if self.wrap == wrap {
            return false;
        }
        self.wrap = wrap;
        self.layouts.clear();
        self.heights.clear();
        self.widest = 0.0;
        if wrap {
            self.pan.to_top();
        }
        // The caret is the thing the reader is looking at, and every row it could be on has just
        // changed shape. Keeping it on screen is what makes the toggle feel like a view change
        // rather than a jump.
        self.follow_caret = true;
        self.refresh_view_menu();
        true
    }

    /// Toggles word wrap.
    pub fn toggle_wrap(&mut self) -> bool {
        self.set_wrap(!self.wrap)
    }

    /// Pans sideways by `delta` pixels, clamped. Does nothing while wrapping.
    ///
    /// # A pan stops the view following the caret
    ///
    /// Without that line this did nothing at all in the window, while every test of it passed.
    /// Shift+wheel is the only way to pan, and **Shift is a key press**, so holding it re-armed
    /// `follow_caret`; the pan landed, and the frame it requested then revealed the caret — which on
    /// a document nobody has clicked into is at the left edge — and put the offset straight back to
    /// zero. The vertical axis has the same hazard and hides it, because a plain wheel is not
    /// preceded by a keystroke and so nothing had re-armed the flag.
    pub fn pan_by(&mut self, delta: f32) -> bool {
        if self.wrap {
            return false;
        }
        self.follow_caret = false;
        let before = self.pan.offset();
        self.pan.by(delta, self.widest, self.content.width());
        // Compared with a tolerance rather than for equality: these are accumulated `f32` offsets,
        // and "did the view move" is a question about pixels rather than about bit patterns.
        (before - self.pan.offset()).abs() > f32::EPSILON
    }

    /// Whether whitespace and line endings are being drawn.
    #[must_use]
    pub fn whitespace(&self) -> bool {
        self.whitespace
    }

    /// Shows or hides them, and says whether anything changed.
    ///
    /// Clears no cache: nothing about the layout depends on it. See [`Self::draw_marks`].
    pub fn set_whitespace(&mut self, showing: bool) -> bool {
        if self.whitespace == showing {
            return false;
        }
        self.whitespace = showing;
        self.refresh_view_menu();
        true
    }

    /// Toggles them.
    pub fn toggle_whitespace(&mut self) -> bool {
        self.set_whitespace(!self.whitespace)
    }

    /// Draws the selection's highlight on one visual row, and records the bar's anchor from it.
    ///
    /// Behind the glyphs, so the text stays readable through it.
    fn draw_row_selection(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        selected: ByteSpan,
        row: Row<'_>,
        at: Placement,
    ) {
        if selected.is_empty() {
            return;
        }
        let Some((from, to)) =
            Self::row_selection(row.cached, row.chunk, row.text, row.index, selected)
        else {
            return;
        };
        let (x, y) = at.content.to_window(from, row.top, at.offset);
        // The first row reached is the first visible row of the selection, and the only one the
        // floating bar wants. Kept here because it is the one place that knows: the highlight is
        // otherwise pushed and forgotten.
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(toolbar::Anchor {
                x,
                y,
                width: to - from,
                height: row.height,
            });
        }
        quads.push(Self::solid(
            [
                x * at.scale,
                y * at.scale,
                (to - from).max(1.0) * at.scale,
                row.height * at.scale,
            ],
            self.theme.selection,
        ));
    }

    /// Lights the search matches crossing one visual row.
    ///
    /// Drawn **under** the selection as well as under the glyphs, and that is the whole of "the one
    /// you are on looks different": the current match *is* the selection, because `go_to_match` sets
    /// it, so the selection colour lands on top of it without a second colour or an index to keep in
    /// step with anything.
    fn draw_matches(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        found: &[ByteSpan],
        row: Row<'_>,
        at: Placement,
    ) {
        for span in found {
            let Some((from, to)) =
                Self::row_selection(row.cached, row.chunk, row.text, row.index, *span)
            else {
                continue;
            };
            let (x, y) = at.content.to_window(from, row.top, at.offset);
            quads.push(Self::solid(
                [
                    x * at.scale,
                    y * at.scale,
                    (to - from).max(1.0) * at.scale,
                    row.height * at.scale,
                ],
                self.theme.find_match,
            ));
        }
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
        let (_, y) = at.content.to_window(0.0, top, at.offset);
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
        // What this chunk contributes to the horizontal clamp. Recorded here because this is where a
        // chunk stops being an estimate and becomes a measurement — the same moment its height
        // becomes real. Only ever grows within a layout width; `set_wrap` and `set_zoom` reset it
        // along with the caches whose answers it was derived from.
        self.widest = self.widest.max(cached.width);
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

impl<T: TextLayout> ApplicationHandler<Wake> for Editor<T> {
    /// Something arrived from outside the loop: a menu item, or an Explorer selection.
    ///
    /// The menu delivers on a channel of its own, forwarded into this loop — see
    /// [`mdedit_menu_win::MenuBar::forward_to`]. Every arm is a command that already exists and is
    /// already tested, which is what makes the menu a surface rather than a second implementation.
    ///
    /// An Explorer activation is the same shape of thing and gets the same treatment: it lands on
    /// [`Editor::open_paths`], which is what the command line, the Open dialog and the recent list
    /// all use. **Twenty files are one event**, not twenty — the whole point of MDEDIT-61 is that
    /// the selection arrives whole.
    fn user_event(&mut self, event_loop: &ActiveEventLoop, wake: Wake) {
        let command = match wake {
            Wake::Menu(command) => command,
            Wake::Open(paths) => {
                if self.open_paths(&paths) {
                    // The window is very likely behind Explorer, because Explorer is what the user
                    // just clicked in. Nothing else raises it: winit's `focus_window` is the only
                    // way, and an editor that opened files invisibly behind the window in front
                    // would look like it had done nothing at all.
                    if let Some(window) = self.window.as_ref() {
                        window.focus_window();
                        window.request_redraw();
                    }
                }
                return;
            }
        };

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

        if self.on_idle() {
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
        // Before the menu, and not inside it: `attach_menu` gives up when muda cannot build a bar,
        // and a window with no menu should still have an icon.
        Self::set_window_icon(&window);
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
            // Not `event_loop.exit()` on its own, which is what this was and what lost people's
            // work. See `may_close`.
            WindowEvent::CloseRequested => {
                if self.may_close() {
                    event_loop.exit();
                } else if let Some(window) = self.window.as_ref() {
                    // Saving may have written files and changed what the title bar and tab strip
                    // should say, and the window is staying open to show it.
                    window.request_redraw();
                }
            }
            // Coming back to the window is when an external edit has almost always happened: the
            // user went away, something ran, and they returned. See `check_disk`.
            WindowEvent::Focused(true) => self.on_focus(),
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
                self.on_pointer_moved(point);
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
            } => {
                self.dragging = false;
                // A thumb drag captures the pointer for as long as the button is down — the pointer
                // leaves the bar's x almost immediately on any real drag, and letting that end the
                // drag would make the bar unusable. Releasing anywhere ends it.
                self.thumb_grab = None;
                // The selection has stopped moving, which is the moment the floating bar is for.
                if self.settle_selection() {
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let (x, y) = self.cursor;
                let changed = match button {
                    MouseButton::Left => self.click(x, y),
                    MouseButton::Right => self.popup_context_menu(x, y),
                    // Middle-click closes, as every editor with tabs does.
                    MouseButton::Middle => {
                        match (tabs::hit_test(&self.tab_rects, x, y), self.pane_hit(x, y)) {
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
                // Come back when the typing stops. Three things wait on this timer and only one of
                // them is about large documents: the whole-document re-parse a scoped edit owes
                // (MDEDIT-18), the word count, and the recovery snapshot (MDEDIT-54).
                //
                // It used to be scheduled only when the parse had been scoped, which meant that on
                // any document under 16 KB the timer never fired at all — so the word count in the
                // status bar was whatever it had been when the file opened, and MDEDIT-54's
                // snapshots would never have been written for exactly the small, unsaved buffers
                // they most exist to protect.
                event_loop
                    .set_control_flow(ControlFlow::WaitUntil(Instant::now() + RECONCILE_AFTER));
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
    /// How far the viewport has moved over the document, on both axes.
    ///
    /// With the rectangle, its `y` gives the band of the document that is on screen — which is what
    /// lines are culled against. A chunk can be far taller than the window, since a document with
    /// no blank line in it is a single chunk, so per-line culling is needed as well as per-chunk.
    ///
    /// Its `x` is zero unless word wrap is off, which is the only way anything can be off to the
    /// side to reach.
    offset: Offset,
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
    /// The widest row in it, for MDEDIT-49's horizontal clamp.
    ///
    /// Measured from the glyphs rather than taken from the layout's own width, because a layout
    /// asked for an unbounded width may report the width it was *asked* for rather than the width it
    /// used — which would clamp the pan against a hundred thousand pixels of nothing.
    width: f32,
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
        // The right-hand edge of the furthest glyph, over every row. A row's own extent is where its
        // last glyph ends, and the widest row is what the pan may reach.
        let width = layout
            .lines
            .iter()
            .filter_map(|line| line.glyphs.last())
            .map(|glyph| glyph.pos.x + glyph.advance)
            .fold(0.0_f32, f32::max);
        Self {
            layout,
            base: chunk.span.start().get(),
            drops_last,
            height,
            width,
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
        self.rebase_offset(glyph.source.start(), span)
    }

    /// The same arithmetic for an offset that did not come from a glyph — an underline's, which the
    /// layout resolves from the text under the rule.
    fn rebase_offset(&self, offset: ByteOffset, span: ByteSpan) -> ByteOffset {
        ByteOffset::new(span.start().get() + offset.get().saturating_sub(self.base))
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
