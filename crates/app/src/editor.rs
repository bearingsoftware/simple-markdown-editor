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
use crate::decoding::{Decoder, PictureKey, PictureState};
use crate::scrollbar::{self, Bar};
use mdedit_document::{Document, Selection};
use mdedit_image::Picture;
use mdedit_language::Diagnostic;
use mdedit_markers::BlockKind;
use mdedit_offsets::{Affinity, ByteOffset, ByteSpan};
use mdedit_session::Workspace;
use mdedit_session::recent::Recent;
use mdedit_text::{FontFamily, GlyphKey, Layout, StyledRun, TextLayout, TextStyle};
use winit::application::ApplicationHandler;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{CursorIcon, Window, WindowId};

use crate::blocks;
use crate::clipboard::{Clipboard, HoldNothing};
use crate::config::Config;
use crate::content::{ContentRect, Insets, Offset};
use crate::dialog;
use crate::field;
use crate::file_icon;
use crate::find;
use crate::gpu::{
    ATLAS_SIZE, Atlas, INITIAL_PICTURES, QuadInstance, SHADER, create_atlas_parts,
    create_bind_layout, create_image_parts, create_pipeline, make_bind_group,
};
use crate::hit;
use crate::ime::{Action, Composition};
use crate::latency::Latency;
use crate::list;
use crate::pane;
use crate::pointer::Pointer;
use crate::popup;
use crate::prompt::{
    self, ChangeNothing, Conflict, ConflictPrompt, Conflicting, KeepEverything, Unsaved,
    UnsavedPrompt,
};
use crate::quote;
use crate::scroll::{self, Scroll};
use crate::status;
use crate::table;
use crate::tabs;
use crate::toolbar;
use crate::tooltip;
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
    /// The decode worker has finished a picture (MDEDIT-156).
    ///
    /// Carries nothing: the answers are on the worker's channel, and one wake is enough however
    /// many of them arrived.
    Pictures,
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
    /// This buffer has unsaved work at this revision, and here it is — with the stamp of the file it
    /// was read from, which is what a restore compares against (MDEDIT-100).
    Write(
        u64,
        Option<PathBuf>,
        Option<mdedit_file::Stamp>,
        u64,
        String,
    ),
    /// This buffer is safe on disk; its snapshot should go.
    Forget(u64),
}

/// Whether this key is a modifier and nothing else — a press that cannot have moved the caret.
///
/// Holding one is not an edit and not a motion, so it must not re-arm the caret reveal. See where it
/// is called: MDEDIT-49 found this the hard way for Shift+wheel, where holding Shift undid the pan
/// on the very next frame.
fn modifier_only(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::CapsLock
                | NamedKey::NumLock
                | NamedKey::ScrollLock
                | NamedKey::Fn
                | NamedKey::FnLock
                | NamedKey::Symbol
                | NamedKey::SymbolLock
        )
    )
}

/// Whether a key press carrying these modifiers is a chord rather than text to insert.
///
/// **Ctrl without Alt**, and the `!alt` is the whole of MDEDIT-102 rather than a nicety. Windows
/// reports `AltGr` as Ctrl+Alt, and `AltGr` is how most of Europe types the characters a Markdown
/// editor is least able to do without: on a German layout `@` is `AltGr`+Q, `{`, `[`, `]` and `}`
/// are `AltGr`+7/8/9/0, and `€` is `AltGr`+E. All of them arrive as ordinary `Key::Character` events
/// with `control_key()` true.
///
/// Every chord in this editor is Ctrl-only, so a gate that read Ctrl alone sent those into the chord
/// dispatch, where they matched no arm and were dropped — the character never reached the buffer. A
/// German-layout user could not type `@` or braces at all.
///
/// A free function taking two plain flags because that is the part a test can reach: the gate itself
/// reads a winit `Modifiers`, and a winit `KeyEvent` cannot be constructed by a test. Same seam, and
/// same reason, as [`Editor::apply_chord`].
#[must_use]
pub fn is_chord(control: bool, alt: bool) -> bool {
    control && !alt
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

/// The glyphs a flagged line number is drawn from (MDEDIT-76): bold digits, and the `!` before them.
#[derive(Debug, Clone, Copy)]
struct Flagged {
    digits: [(GlyphKey, f32); 10],
    bang: (GlyphKey, f32),
}

/// One line number to draw, and whether anything is wrong on its line.
#[derive(Debug, Clone, Copy)]
struct LineNumber {
    number: usize,
    /// The row's **baseline**, in document coordinates — not its top edge.
    ///
    /// It used to be the top, and the digits were then placed the way a glyph is: `top` plus the
    /// bitmap's own offset from the baseline, which is negative. So every number was drawn about an
    /// ascent *above* the line it belonged to, and on a real font that is most of a row. Nothing
    /// caught it: `gutter.rs` asserts where numbers sit horizontally — that was MDEDIT-34 — and the
    /// headless stub gives every glyph a one-pixel offset, so the error is one pixel there and
    /// invisible. MDEDIT-76 found it by putting a `!` in the gutter and looking at the window.
    baseline: f32,
    flagged: bool,
    /// Whether the caret is on this line, which brightens it to `text` (MDEDIT-77).
    current: bool,
}

/// One visual row being drawn, gathered so a helper can be given it rather than seven arguments.
#[derive(Clone, Copy)]
struct Row<'a> {
    cached: &'a Cached,
    chunk: blocks::Chunk,
    /// The whole document's text, which is what chunk spans index into.
    text: &'a str,
    /// What bytes each of the chunk's rows covers, as offsets into the chunk's own text.
    ///
    /// **`hit` is the one authority on this** (MDEDIT-183). Three places used to count newlines to
    /// answer it, and two of them were still wrong after the caret was fixed: a selection
    /// highlighted a continuation row of the line above, and the whitespace marks on every row after
    /// a wrap were placed against the wrong bytes. Computed once per chunk and handed down, which is
    /// also what keeps it off the per-row path.
    rows: &'a [(usize, usize)],
    /// Which row of the chunk's layout.
    index: usize,
    /// Where the row's top edge is, in document coordinates.
    top: f32,
    height: f32,
    /// Where the chunk's top edge is, which is what layout positions are relative to.
    chunk_top: f32,
}

impl Row<'_> {
    /// Where this row's text sits in the buffer, from `hit`'s mapping.
    ///
    /// Falls back to the chunk's own span for a row the mapping does not have, which cannot happen
    /// for a row being drawn — the mapping covers every line of the layout — and is a clamp rather
    /// than a panic if it ever does.
    fn bytes(&self) -> (usize, usize) {
        let base = self.chunk.span.start().get();
        self.rows
            .get(self.index)
            .map_or((base, self.chunk.span.end().get()), |&(start, end)| {
                (base + start, base + end)
            })
    }
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

/// A list that is showing: what is in it, and where it landed.
#[derive(Debug, Clone)]
struct OpenMenu {
    /// Which button opened it, so a widened window can close the right one.
    kind: toolbar::Menu,
    /// The rows, in order.
    rows: Vec<toolbar::Action>,
    /// The tabs a [`toolbar::Menu::Tabs`] list offers, in order, where `rows` is empty
    /// (MDEDIT-179). Empty for every other kind.
    tabs: Vec<usize>,
    /// Top-left corner, in window coordinates.
    at: (f32, f32),
    /// Panel width, measured from the widest row when it opened.
    width: f32,
}

impl OpenMenu {
    /// How many rows the list has, whichever kind of row they are.
    fn len(&self) -> usize {
        if self.kind == toolbar::Menu::Tabs {
            self.tabs.len()
        } else {
            self.rows.len()
        }
    }
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

/// One dash of a diagnostic's squiggle, in device-independent pixels.
const SQUIGGLE_DASH: f32 = 2.0;
/// How far the alternate dash sits below the first, which is what makes it a wave.
const SQUIGGLE_RISE: f32 = 2.0;
/// How thick each dash is.
const SQUIGGLE_THICKNESS: f32 = 1.0;
/// How far the wave's top sits above the bottom of the row it marks.
const SQUIGGLE_DROP: f32 = 4.0;

/// How much smaller a formatting-bar label is than body text.
///
/// The bar's cached labels, keyed by the strings at the
/// same size — one cache serves both only while that holds.
const TOOLBAR_TEXT: f32 = 0.62;

/// Icon size on the bar, as a multiple of the body size.
///
/// 22 pixels at the default 16, which is what MDEDIT-92's mock-up drew and what a 48-pixel bar
/// wants. A multiple rather than a constant so it follows the zoom, exactly as every other piece of
/// chrome text does.
const TOOLBAR_ICON: f32 = 1.375;

/// Size of a face that is a letterform **on purpose** — the heading buttons and rows.
///
/// Near the icon size rather than the label size, so `H1` reads as a sibling of the pictures either
/// side of it instead of as a caption between them. Twenty pixels at the default 16, against the
/// icons' 22: two characters at twenty are about as wide as one icon at twenty-two, which is what
/// keeps the button square. See [`toolbar::Action::letters`] for why this is not the same question
/// as the missing-font fallback.
const TOOLBAR_LETTERS: f32 = 1.25;

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
    /// The second pipeline: colour from a texture of its own rather than coverage from the atlas
    /// (MDEDIT-146). Same instance format, same bind-group layout, different entry points.
    image_pipeline: wgpu::RenderPipeline,
    /// Linear, where [`sampler`](Self::sampler) is nearest — a picture is nearly always scaled down.
    image_sampler: wgpu::Sampler,
    image_instances: wgpu::Buffer,
    image_capacity: u64,
    /// A bind group per uploaded picture, by the editor's picture id.
    pictures: HashMap<u64, wgpu::BindGroup>,
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

        let (atlas_texture, sampler) = create_atlas_parts(&device);
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

        let pipeline = create_pipeline(
            &device,
            &shader,
            &bind_layout,
            format,
            ("vs_main", "fs_main"),
        );
        let (image_pipeline, image_sampler, image_instances) =
            create_image_parts(&device, &shader, &bind_layout, format);

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
            image_pipeline,
            image_sampler,
            image_instances,
            // **The same constant the buffer was sized with**, not a second literal. They were 16
            // and 64 for one commit, so `ensure_image_capacity` believed there was room for 64
            // instances in a buffer holding 16 and `write_buffer` would have overrun it at the
            // seventeenth picture on screen. CI has no GPU and could never have seen it.
            image_capacity: INITIAL_PICTURES,
            pictures: HashMap::new(),
        })
    }

    /// Uploads any picture on screen this frame that has not been uploaded already.
    ///
    /// Keyed on the editor's picture id, so the same file in two paragraphs is one texture — and so
    /// a scroll that brings a picture back costs nothing.
    ///
    /// **Never evicted.** A session that opens a hundred distinct images holds a hundred textures,
    /// which is the same bargain `heights` strikes and a smaller one than `layouts`: the editor's own
    /// picture cache is what bounds this, and capping it is the follow-up that measurement should
    /// ask for rather than a guess made now.
    fn upload_pictures(&mut self, sources: &[(u64, Rc<Picture>)]) {
        for (id, picture) in sources {
            if self.pictures.contains_key(id) {
                continue;
            }
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("picture"),
                size: wgpu::Extent3d {
                    width: picture.width,
                    height: picture.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Not `Rgba8UnormSrgb`: the surface is deliberately configured non-sRGB, so the
                // shader's output reaches the display unconverted and a picture's bytes should too.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &picture.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(picture.width * 4),
                    rows_per_image: Some(picture.height),
                },
                wgpu::Extent3d {
                    width: picture.width,
                    height: picture.height,
                    depth_or_array_layers: 1,
                },
            );
            let bind_group = make_bind_group(
                &self.device,
                &self.bind_layout,
                &self.globals,
                &texture,
                &self.image_sampler,
            );
            self.pictures.insert(*id, bind_group);
        }
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

    /// Grows the picture instance buffer. No bind group to rebuild: a picture's is built with its
    /// texture and does not name this buffer.
    fn ensure_image_capacity(&mut self, needed: u64) {
        if needed <= self.image_capacity {
            return;
        }
        self.image_capacity = needed.next_power_of_two();
        self.image_instances = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pictures"),
            size: self.image_capacity * size_of::<QuadInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
    fn draw(&mut self, quads: &[QuadInstance], pictures: &[crate::image::Drawn], background: Rgba) {
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

        // One instance per picture, in the order they will be drawn. `uv` is the whole texture
        // rather than a region of one, and the white `colour` is what `fs_image` multiplies by —
        // the multiply exists so a future tint has somewhere to go, and today it changes nothing.
        let image_quads: Vec<QuadInstance> = pictures
            .iter()
            .map(|drawn| QuadInstance {
                rect: drawn.rect,
                uv: [0.0, 0.0, 1.0, 1.0],
                colour: [1.0, 1.0, 1.0, 1.0],
            })
            .collect();
        if !image_quads.is_empty() {
            self.ensure_image_capacity(image_quads.len() as u64);
            self.queue
                .write_buffer(&self.image_instances, 0, bytemuck::cast_slice(&image_quads));
        }

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
            // **Pictures first, in the same pass.** There is no depth buffer and no scissor, so
            // painter's order is the whole of the answer (`docs/decisions/chrome.md`) — and a
            // picture belongs under everything, because the document's glyphs and then the chrome
            // both have to be able to cover it. One draw apiece, because each samples its own
            // texture and so needs its own bind group.
            pass.set_pipeline(&self.image_pipeline);
            pass.set_vertex_buffer(0, self.image_instances.slice(..));
            for (index, drawn) in pictures.iter().enumerate() {
                let Some(bind_group) = self.pictures.get(&drawn.id) else {
                    continue;
                };
                pass.set_bind_group(0, bind_group, &[]);
                let at = u32::try_from(index).unwrap_or(u32::MAX);
                pass.draw(0..4, at..at + 1);
            }

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

/// What the pointer is resting on, when that is worth a panel.
///
/// Three things now, and they are two different questions. A **button** is explained: the panel says
/// what the icon means, and it appears whether or not the button is wide enough for its own label.
/// A **tab** or a **pane row** is completed: the panel says what did not fit, and only appears when
/// something did not — a tooltip repeating what is already on screen is noise.
///
/// MDEDIT-121 added the second two. `AGENTS.md` recorded for both of them that they truncate a name
/// and have nowhere to put the full one; MDEDIT-85 built the dwell, the second `ControlFlow`
/// deadline and the panel, so this is a hover region and a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hovered {
    /// A formatting-bar button, which says what it does.
    Button(toolbar::Action),
    /// A tab whose name the strip could not draw in full.
    Tab(usize),
    /// A file-pane row whose name the pane could not draw in full.
    Row(usize),
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
    /// Which palette is being drawn (MDEDIT-78).
    ///
    /// Kept beside the theme rather than derived from it: the *choice* lives in the config and what
    /// Windows says can change under it, so the resolved answer is a thing to hold and compare
    /// against — the same shape as `counted_for`.
    appearance: view::Appearance,
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
    /// Where each tab landed in the strip last frame, and what did not fit, for resolving a click.
    strip: tabs::Strip,
    /// Whether the formatting bar is showing. Per-run: MDEDIT-52 left the setting to the settings
    /// story, and Ctrl+B-era per-run chrome is the precedent.
    toolbar: bool,
    /// The bar's buttons as laid out last frame, for the click chain.
    toolbar_buttons: Vec<toolbar::ButtonRect>,
    /// Each button's glyph, laid out once. The fourth cache of this kind — see `crate::toolbar`.
    toolbar_labels: HashMap<String, Label>,
    /// The list a toolbar button has opened, if one is open.
    ///
    /// MDEDIT-116. Holds its rows as well as its rectangle, because the overflow list's contents
    /// depend on how narrow the window was when it opened, and re-deriving them on the click would
    /// be the second computation the ticket's own risk section warns about.
    open_menu: Option<OpenMenu>,
    /// The modal dialog that is asking for a link's or an image's parts, if one is.
    ///
    /// MDEDIT-129. **The only surface in the editor that takes the keyboard away from both the
    /// document and the find bar**, which is why it is checked before either of them in
    /// `window_event` and why the document dims behind it.
    dialog: Option<dialog::Dialog>,
    /// What the bar could not fit last frame, which is exactly what `More` opens.
    ///
    /// Kept rather than recomputed on the click: it and `toolbar_buttons` are two halves of one
    /// partition, and computing them separately is how an action ends up in neither.
    dropped: Vec<toolbar::Action>,
    /// The window's logical width last frame, so a list can be clamped inside it.
    window_width: f32,
    /// And its height, so a modal dialog can be centred in it (MDEDIT-129).
    window_height: f32,
    /// What the pointer is over, if anything with something to say. Hovering, not yet dwelling.
    hovered: Option<Hovered>,
    /// Which row of the open list the pointer is over. Separate from `hovered` because a row is
    /// highlighted the instant the pointer reaches it, where a button waits out a dwell.
    hovered_row: Option<usize>,
    /// Which cell of the open size picker the pointer is over, as a zero-based `(row, column)`.
    ///
    /// Separate from [`hovered_row`](Self::hovered_row) rather than a second meaning laid over it:
    /// only one panel is ever open, so only one of the two is ever `Some`, and a single field
    /// holding "row 3" or "row 3, column 4" depending on what opened it is the shape a later reader
    /// misuses. It is also what the picker *shows* — every cell up to and including this one is lit,
    /// which is the whole of the hover-to-size gesture.
    hovered_cell: Option<(usize, usize)>,
    /// Which button's tooltip is showing. Set when the dwell elapses, cleared by anything at all.
    ///
    /// Separate from [`hovered`](Self::hovered) because the two answer different questions and the
    /// gap between them *is* the dwell: the pointer is over a button for the whole time it takes to
    /// travel the bar, and a panel appears only if it stops.
    tooltip: Option<Hovered>,
    /// Each tooltip's line, laid out once. The eighth cache of this kind — see `set_zoom`.
    tooltip_labels: HashMap<String, Label>,
    /// When the pointer will have rested long enough, if it is resting.
    ///
    /// One of the two deadlines the loop's single next-wake-up has to serve. See
    /// [`next_wake`](Self::next_wake).
    dwell_at: Option<Instant>,
    /// When the whole-document re-parse and everything else on the idle beat comes due.
    ///
    /// The other of the two. This used to be set straight into `ControlFlow::WaitUntil` at the
    /// keystroke, which worked while it was the only deadline there was.
    reconcile_at: Option<Instant>,
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
    /// Where the open files are shown. See [`Chrome`].
    chrome: Chrome,
    /// The pane's visible rows, kept for the next click, as `strip` is for the strip.
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
    /// Whether each icon code point resolved to a glyph this machine has (MDEDIT-174), asked once and
    /// shared by the pane, the strip (MDEDIT-178) and the strip's overflow button (MDEDIT-179).
    ///
    /// Not cleared with the label caches: whether a font has a code point does not change with the
    /// size it is drawn at, and asking per row per frame is a text-engine call on the path a
    /// keystroke takes — which is the trap the gutter, the status bar and the pane have each hit.
    icon_faces: HashMap<char, bool>,
    /// Last known cursor position, in device-independent pixels.
    cursor: (f32, f32),
    /// What shape the window was last told to draw the pointer as (MDEDIT-176). Held so a still
    /// pointer costs nothing and a moving one costs a comparison rather than a call per event.
    pointer: Pointer,
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
    /// Decoded pictures, by the file they came from (MDEDIT-146).
    ///
    /// **A cache of its own rather than part of [`ChunkKey`]**, because that key hashes the chunk's
    /// bytes and a picture's content is not in the buffer at all. Two paragraphs naming the same
    /// file share one entry and one texture, and editing a paragraph beside an image does not
    /// re-read it.
    ///
    /// **A refusal is cached too.** A document naming a file that is not there would otherwise
    /// re-stat it on every layout, and a broken image is the common case in a document being
    /// written — the path is typed before the file exists.
    /// **Three answers, not two** (MDEDIT-156). A picture whose header has been read can be laid
    /// out before its pixels exist, so "sized but not decoded yet" is a state the cache has to be
    /// able to hold.
    pictures: HashMap<PictureKey, PictureState>,
    /// The number the next distinct picture gets, so the renderer can key a texture on it.
    picture_ids: HashMap<PictureKey, u64>,
    /// The thread that decodes pictures, once one has been wanted (MDEDIT-156).
    ///
    /// `None` until then, so a session that never opens a document with an image in it never starts
    /// a thread — which is most sessions, and every test that does not place a picture.
    decoder: Option<Decoder>,
    /// Which pictures the last [`build_frame`](Self::build_frame) placed, and where.
    ///
    /// Rebuilt each frame and read by the renderer afterwards, rather than returned: `build_frame`
    /// returns quads and is called from thirty-odd tests, and widening its signature to carry a
    /// second list would churn all of them to say nothing.
    drawn_pictures: Vec<crate::image::Drawn>,
    /// The bytes each of those pictures stands for (MDEDIT-152).
    ///
    /// **What makes the caret step over an image instead of into it.** Whether a paragraph drew as a
    /// picture is the renderer's answer, not the document's, so [`Self::concealed_at`] takes the
    /// document's concealment and adds these. Rebuilt each frame beside `drawn_pictures`, and read
    /// on the next key press — see that method for what the one-frame lag costs.
    picture_spans: Vec<ByteSpan>,
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
    /// The same in bold, plus the `!`, for a line with a diagnostic on it (MDEDIT-76).
    ///
    /// Laid out the first time an error is on screen rather than at startup: most documents never
    /// have one, and a document that is not checked at all — every Markdown file — never pays for it.
    flagged_digits: Option<Flagged>,
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
    counted_for: Option<u64>,
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
    diagnosed_for: Option<(u64, u64)>,
    /// What the Tools menu was last told, so it is only told again when the answer changes.
    ///
    /// **Compared rather than hooked.** Which document is active changes on a great many paths, and
    /// a list of call sites to update is a list somebody forgets to add to — `counted_for` is the
    /// same shape for the same reason. This one has to push rather than pull,
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
    ///
    /// **Dark to begin with**, whatever Windows is set to: nothing here asks. The question is put in
    /// [`Self::with_config`] and again on the idle beat, so an editor left running with the default
    /// choice does end up following Windows — which means **a test that names colours should pin the
    /// choice**, or it is asserting against whichever palette the machine it runs on prefers. Two
    /// test files learned that from a light-themed CI runner.
    pub fn new(source: &str, text: T) -> Self {
        let mut workspace = Workspace::new();
        // Set here as well as in `with_workspace`, because a file opened later through the menu goes
        // into this one and has to get its language too.
        workspace.with_language_resolver(crate::detect::language_for);
        Self {
            workspace,
            scratch: Document::from(source),
            theme: Theme::default(),
            appearance: view::Appearance::default(),
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
            strip: tabs::Strip::default(),
            toolbar: true,
            toolbar_buttons: Vec::new(),
            toolbar_labels: HashMap::new(),
            open_menu: None,
            dialog: None,
            hovered_row: None,
            hovered_cell: None,
            dropped: Vec::new(),
            window_width: 0.0,
            window_height: 0.0,
            hovered: None,
            tooltip: None,
            tooltip_labels: HashMap::new(),
            dwell_at: None,
            reconcile_at: None,
            find: find::Bar::default(),
            find_open: false,
            find_focused: false,
            find_layout: None,
            chrome: Chrome::Strip,
            row_rects: Vec::new(),
            pane_scroll: 0.0,
            row_labels: HashMap::new(),
            icon_faces: HashMap::new(),
            // Nowhere, until somebody says where. See `with_snapshot_store`, and MDEDIT-68 for what
            // defaulting this to the real directory cost.
            snapshots: None,
            snapshotted: HashMap::new(),
            cursor: (0.0, 0.0),
            pointer: Pointer::Arrow,
            scroll: Scroll::new(),
            viewport: Viewport::default(),
            follow_caret: false,
            heights: HashMap::new(),
            layouts: HashMap::new(),
            pictures: HashMap::new(),
            picture_ids: HashMap::new(),
            decoder: None,
            drawn_pictures: Vec::new(),
            picture_spans: Vec::new(),
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
            flagged_digits: None,
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
        // The palette first, because it replaces the theme wholesale — and then the zoom into it, so
        // a document opens at the size it was last read at. Both are applied here rather than left
        // for the first keypress: an editor that opened dark and turned light on its second frame
        // would be worse than one that took a moment longer to appear.
        self.apply_appearance(
            config
                .theme
                .resolve(mdedit_menu_win::apps_use_light_theme()),
        );
        self.theme.body_size = Theme::default().body_size * config.zoom;
    }

    /// Draws with `appearance` from now on, keeping the zoom.
    ///
    /// # The cache question, answered by test rather than by reasoning
    ///
    /// Nothing is cleared, and MDEDIT-78's acceptance made that an assertion rather than a claim:
    /// `theme_switch.rs` swaps palettes and checks that no quad still carries a colour from the old
    /// one. It holds because colour is deliberately **not** in [`mdedit_text::TextStyle`] and not in
    /// `ChunkKey` — every cache here holds *laid-out glyphs*, and a glyph is the same shape in both
    /// palettes. The colour is chosen at draw time, from `self.theme`, which is what this replaces.
    ///
    /// The one thing that has to be carried across is the zoom. `body_size` is a *scaled* value and
    /// a palette's own is the unscaled one, so the scale is taken from the theme being replaced
    /// rather than from the settings or the config: those two disagree for as long as it takes
    /// `with_config` to run, and a theme switch that quietly reset the text size would be a strange
    /// bug to hunt.
    fn apply_appearance(&mut self, appearance: view::Appearance) {
        let scale = self.theme.body_size / Theme::default().body_size;
        self.appearance = appearance;
        self.theme = Theme::for_appearance(appearance);
        self.theme.body_size *= scale;
        // The menus are the same variable as the document (MDEDIT-79): a light document under a dark
        // menu bar was the known intermediate state at the end of MDEDIT-78, and this is what ends
        // it. Cheap and idempotent — `set_appearance` returns immediately when the mode has not
        // moved, which is most of the times this runs.
        if let Some(menu) = self.menu.as_ref() {
            menu.set_appearance(appearance == view::Appearance::Dark);
        }
        self.refresh_view_menu();
    }

    /// Which palette is being drawn.
    #[must_use]
    pub fn appearance(&self) -> view::Appearance {
        self.appearance
    }

    /// Chooses a palette, or chooses to follow Windows, and remembers it.
    ///
    /// Returns whether anything changed, so a caller can skip a redraw — and so the tests can say
    /// that choosing what is already chosen is not a change.
    pub fn set_theme_choice(&mut self, choice: view::ThemeChoice) -> bool {
        if self.config.theme == choice && self.appearance == Self::resolved(choice) {
            return false;
        }
        self.config.theme = choice;
        self.settings.update(|config| config.theme = choice);
        self.apply_appearance(Self::resolved(choice));
        true
    }

    /// What a choice means right now, asking Windows if the choice says to.
    fn resolved(choice: view::ThemeChoice) -> view::Appearance {
        choice.resolve(mdedit_menu_win::apps_use_light_theme())
    }

    /// Tells the renderer the window is a different size, and asks for a frame at it.
    fn on_resized(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size.width, size.height);
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Takes Windows' word that something changed, and asks the registry what it changed to.
    ///
    /// **The event is the notification, not the answer** (MDEDIT-78). winit raises
    /// `WindowEvent::ThemeChanged` off `WM_SETTINGCHANGE`: it does not filter on `ImmersiveColorSet`
    /// but simply re-evaluates and speaks only when its own answer moved. That answer comes from
    /// `ShouldAppsUseDarkMode`, which `AGENTS.md` says not to trust — so the theme it carries is
    /// ignored and [`Self::system_appearance_changed`] reads the registry instead.
    ///
    /// It arrives at all only because nothing calls winit's `with_theme`: a window built with a
    /// preferred theme is never sent this event, which would be a quiet way to lose live switching.
    fn follow_system_appearance(&mut self) {
        if self.system_appearance_changed()
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }

    /// Windows' light/dark setting changed while the editor was running.
    ///
    /// Called from `WindowEvent::ThemeChanged`, which winit raises off `WM_SETTINGCHANGE` — see the
    /// event loop for the division of labour there: **winit is the notification, the registry is the
    /// answer**, because winit decides its own value with `ShouldAppsUseDarkMode` and `AGENTS.md`
    /// says not to trust that.
    ///
    /// Does nothing when the palette is pinned, which is the whole point of pinning it. Returns
    /// whether the editor now draws differently.
    pub fn system_appearance_changed(&mut self) -> bool {
        let wanted = Self::resolved(self.config.theme);
        if wanted == self.appearance {
            return false;
        }
        self.apply_appearance(wanted);
        true
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
        // The palette is asked about here as well as on `ThemeChanged`, and for the same reason the
        // disk is: the user went away, changed something, and came back. MDEDIT-78 found that the
        // event alone cannot be relied on — see `follow_system_appearance` — and coming back to the
        // window is exactly when somebody who has just flipped Windows' setting is looking at it.
        let repaint = self.check_disk() | self.system_appearance_changed();
        if repaint && let Some(window) = self.window.as_ref() {
            window.request_redraw();
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
            // **And so is the word count**, for exactly the reason `reopen_with` gives and for
            // exactly as long as it was the inconsistent twin of it (MDEDIT-109): a count is left to
            // age because a stale one is nearly right, and after a file has been rewritten
            // underneath it is not stale but *wrong*. Nothing schedules an idle tick on a focus
            // change, so the bar showed the old file's number until the next keystroke.
            self.words = None;
            self.counted_for = None;
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
        // Debounced onto this timer rather than written on the keystroke that changed it, so a run
        // of rapid toggles costs one write. Not on exit — see `crate::config`.
        self.settings.flush();

        // The sixth thing on this timer, and the cheapest by three orders of magnitude: one registry
        // read, microseconds, against a whole-document re-parse. It is here because winit's
        // `ThemeChanged` cannot be relied on — see `follow_system_appearance` — and this is the
        // event-driven beat the editor already has. **Not a poll**: nothing schedules it, the loop
        // runs it when it is about to wait after handling input, so an idle editor asks nothing.
        let repalette = self.system_appearance_changed();

        reparsed
            || before != self.words
            || repalette
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
                // The stamp of the bytes this buffer was *read* from, not of the file as it is now.
                // MDEDIT-100: taking it here would record a rewrite that happened since the load as
                // though the buffer already knew about it, and the restore would then not ask.
                open.file.as_ref().and_then(mdedit_file::LoadedFile::stamp),
                revision,
                open.document.to_string(),
            ));
        }

        for item in todo {
            match item {
                Snapshotting::Write(id, path, stamp, revision, text) => {
                    if let Err(error) = store.write(id, path.as_deref(), stamp, &text) {
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
        // ordinary quit is **the feature working** — since MDEDIT-82 that is what leaving the
        // program does, with nothing asked on the way out — so it is noted and not announced. The
        // same text appearing after the process was killed is worth saying out loud, because the
        // user did not choose it and may be wondering what survived.
        if after_clean_exit {
            crate::log_error!(
                "restored {count} unsaved buffer(s) left open at the last ordinary quit"
            );
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
                if let Some(store) = self.snapshots.as_mut()
                    && let Err(error) = store.forget(entry.id)
                {
                    crate::log_error!("could not remove the stored edit: {error}");
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

    /// Drops what the last frame knew about pictures, so this one rebuilds it.
    ///
    /// Both lists together because they are two halves of one answer — where each picture went, and
    /// which bytes it stands for — and a frame that rebuilt one without the other would draw a
    /// picture the caret does not step over, or step over one that is not drawn.
    fn forget_last_frames_pictures(&mut self) {
        self.drawn_pictures.clear();
        self.picture_spans.clear();
    }

    /// Where the last [`build_frame`](Self::build_frame) put each picture (MDEDIT-146).
    ///
    /// The renderer's other half of a frame, and a headless test's only view of one: CI cannot open
    /// a window, so this is where "the picture is 900 pixels wide and starts at the margin" is
    /// checkable at all.
    #[must_use]
    pub fn drawn_pictures(&self) -> &[crate::image::Drawn] {
        &self.drawn_pictures
    }

    /// The pixels behind a [`Drawn::id`](crate::image::Drawn::id), for the renderer to upload.
    ///
    /// Looked up rather than carried on the `Drawn` so that a picture drawn twice — the same file in
    /// two paragraphs — is one texture and one upload.
    ///
    /// `None` while the decode is still running (MDEDIT-156). The rectangle is already placed and
    /// the row is already the right height; only the pixels are late.
    #[must_use]
    pub fn picture(&self, id: u64) -> Option<&Picture> {
        let key = self
            .picture_ids
            .iter()
            .find(|(_, assigned)| **assigned == id)
            .map(|(key, _)| key)?;
        self.pictures.get(key)?.pixels().map(Rc::as_ref)
    }

    /// The distinct pictures behind `drawn`, ready to hand to the renderer.
    ///
    /// Deduplicated, so the same file in two paragraphs is offered once. A picture still being
    /// decoded is simply absent, and `Gpu::draw` skips an id it has no texture for — which is the
    /// whole of what "a blank row that fills in later" needed (MDEDIT-156).
    ///
    /// **Public for the same reason [`Self::drawn_pictures`] is**: uploading needs a device, so this
    /// is the last point a headless test can see what the renderer will be handed. Returning an
    /// empty list here draws no pictures at all and no test noticed until one was made to.
    #[must_use]
    pub fn picture_sources(&self, drawn: &[crate::image::Drawn]) -> Vec<(u64, Rc<Picture>)> {
        let mut sources: Vec<(u64, Rc<Picture>)> = Vec::new();
        for id in drawn.iter().map(|picture| picture.id) {
            if sources.iter().any(|(seen, _)| *seen == id) {
                continue;
            }
            let found = self
                .picture_ids
                .iter()
                .find(|(_, assigned)| **assigned == id)
                .and_then(|(key, _)| self.pictures.get(key))
                .and_then(PictureState::pixels)
                .map(Rc::clone);
            if let Some(picture) = found {
                sources.push((id, picture));
            }
        }
        sources
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
        if self.workspace.active_path() == Some(path)
            && let Some(index) = self.workspace.active()
        {
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
            Command::SetAppearance(choice) => self.set_theme_choice(match choice {
                mdedit_menu_win::AppearanceChoice::System => view::ThemeChoice::System,
                mdedit_menu_win::AppearanceChoice::Dark => view::ThemeChoice::Dark,
                mdedit_menu_win::AppearanceChoice::Light => view::ThemeChoice::Light,
            }),
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
        // A let-chain, which the floor allows as of MDEDIT-189 — it was two nested `if let`s and a
        // comment explaining why it had to be.
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(win32) = handle.as_raw()
        {
            mdedit_menu_win::set_window_icon(win32.hwnd.get());
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
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(win32) = handle.as_raw()
        {
            // In the editor's palette, not Windows' — MDEDIT-79. They are the same by default
            // and deliberately not the same when the config pins one.
            menu.attach(win32.hwnd.get(), self.appearance == view::Appearance::Dark);
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
        if let Some(store) = self.snapshots.as_mut()
            && let Err(error) = store.forget(id)
        {
            crate::log_error!("could not remove a stored buffer: {error}");
        }
    }

    /// Says whether the editor may close, and makes sure it can.
    ///
    /// # Nothing is asked
    ///
    /// MDEDIT-82. Closing the program with unsaved work closes it, and the work comes back on the
    /// next launch — which is what MDEDIT-54 and MDEDIT-66 already built, and what MDEDIT-64 then
    /// put a dialog in front of. Every dirty buffer, titled or untitled, is already copied beside
    /// the session file, and those copies already survive a clean quit; Save All / Keep for now /
    /// Cancel was asking the user to decide something the editor had already handled.
    ///
    /// **Closing a *tab* still asks.** That is where a document is deliberately discarded and the
    /// user is looking at it. This is about leaving the program.
    ///
    /// # It flushes first, and that is not optional
    ///
    /// The copy on disk is whatever the idle timer last wrote. An edit made and the window closed
    /// inside that interval would come back a few keystrokes short — which is worse than the dialog
    /// was, because it looks like the feature working. So the snapshot is taken **synchronously**
    /// here, before anything says yes.
    ///
    /// # Why it still returns a bool
    ///
    /// This is the seam `main` and the tests drive, and a reason to refuse would live here — a save
    /// in flight, a modal the platform owns. There is none today, and a caller that ignored the
    /// answer would also skip the flush.
    pub fn may_close(&mut self) -> bool {
        self.snapshot_dirty_buffers();
        true
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

    /// Applies a Ctrl chord. Returns whether anything changed and a redraw is needed.
    ///
    /// Separate from the rest of the key handling because they are two different things: a chord is
    /// a command, and everything else edits or moves.
    fn handle_chord(&mut self, event: &KeyEvent) -> bool {
        match &event.logical_key {
            Key::Named(NamedKey::Home) => {
                return self.press_document_edge(false, self.modifiers.shift_key());
            }
            Key::Named(NamedKey::End) => {
                return self.press_document_edge(true, self.modifiers.shift_key());
            }
            // Tab is a *named* key, not a character — which `find_key`'s own decode already relied
            // on — so the `"\t"` arm below was unreachable from the keyboard and `cycle_tab` could
            // not be reached at all: there is no menu item for it either. MDEDIT-101, and MDEDIT-65's
            // class exactly, a label and a binding disagreeing with nothing to catch it.
            Key::Named(NamedKey::Tab) => {
                return self.apply_chord("\t", self.modifiers.shift_key());
            }
            // Enter is named too, and handed on as `"\n"` for the same reason: so a test can press
            // it (MDEDIT-175). Until then Ctrl+Enter reached here and was dropped.
            Key::Named(NamedKey::Enter) => {
                return self.apply_chord("\n", self.modifiers.shift_key());
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
            //
            // **Both spellings read `shift` explicitly**, which the first version did not: `"z" if
            // !shift` then `"z" | "Z" | …` meant Caps Lock — which delivers `"Z"` with `shift`
            // false — turned Ctrl+Z into Redo. The s/S, x/X and e/E pairs were already immune
            // because each reads the flag; only this one leaned on the letter's case to carry it
            // (MDEDIT-109).
            "z" | "Z" if !shift => self.document_mut().undo(),
            "z" | "Z" | "y" | "Y" => self.document_mut().redo(),
            "w" | "W" => self.close_tab(),
            "	" => self.cycle_tab(!shift),
            // Ctrl+Enter ticks the task on the caret's line (MDEDIT-175): Obsidian's default for the
            // same thing, and unbound here until now. Not with Shift — Ctrl+Shift+Enter is "insert a
            // line above" in VS Code and Word, and a chord nobody has claimed is cheaper to keep free
            // than to take back.
            "\n" if !shift => self.document_mut().toggle_selected_tasks(),
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

    /// Applies one key press to the document.
    ///
    /// Returns whether anything changed and a redraw is needed.
    fn handle_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }

        // Typing means the pointer is not what the user is doing, whatever it is resting on.
        // Unconditional, unlike the caret-reveal below: a bare modifier press is still a person at
        // the keyboard rather than at the mouse.
        let tooltip_gone = self.dismiss_panels();

        // **The dialog takes every key, before anything else looks at one** (MDEDIT-129). It is the
        // only modal surface in the editor: the find panel shares the keyboard with the document and
        // takes only the clicks on itself, and this takes both outright and says so by dimming what
        // is behind it. Above F3 as well, because a modal panel that let a find shortcut
        // through would be a panel the document could scroll out from under.
        if self.dialog.is_some() {
            return self.dialog_key(event) || tooltip_gone;
        }

        // A key press should leave the caret on screen — but only a press that can move it.
        //
        // This used to arm unconditionally, and MDEDIT-109 is the two things that cost. Typing into
        // the **find field** yanked the view back to the document's caret, which is the focus model
        // leaking in the direction the field was built to prevent. And a bare **modifier** press
        // armed it while doing nothing at all, so the next redraw from any source snapped a
        // deliberately-scrolled view back — which is the same hazard MDEDIT-49 documented for
        // Shift+wheel and repaired only in `pan_by`. Its rule was "anything that moves the viewport
        // deliberately has to say so", and this was the half that did not.
        //
        // Still **before** dispatch: the two keys that scroll without moving the caret clear it
        // again themselves below, and arming afterwards would undo that.
        if !self.find_focused() && !modifier_only(&event.logical_key) {
            self.follow_caret = true;
        }

        // F3 works wherever focus is, which is the point of having it as well as Enter — it is how
        // you step through matches while typing in the document.
        if matches!(&event.logical_key, Key::Named(NamedKey::F3)) {
            return tooltip_gone
                | if self.modifiers.shift_key() {
                    self.find_previous()
                } else {
                    self.find_next()
                };
        }

        // Everything else goes to whichever of the two has focus, and the find field having it is
        // the only case where the document does not. That is the whole focus model.
        if self.find_focused() {
            return self.find_key(event) || tooltip_gone;
        }

        // Chords first, so a modified key never inserts its own letter. Deliberately **above** the
        // dismissal: Ctrl+B is one of the bar's own actions arriving by keyboard, and taking the bar
        // away only to have the idle timer put it back is a flicker for no rule.
        if is_chord(self.modifiers.control_key(), self.modifiers.alt_key()) {
            return self.handle_chord(event) || tooltip_gone;
        }

        let dismissed = tooltip_gone;
        if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
            // Esc closes the find bar even when focus has gone back to the document — the bar is
            // still taking room and still lighting matches, so leaving it open would make Esc do
            // half of what it looks like it does.
            return self.close_find() || dismissed;
        }

        // Shift with a motion key drags the selection along instead of collapsing it.
        let extend = self.modifiers.shift_key();

        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => self.press_backspace(),
            Key::Named(NamedKey::Delete) => self.press_delete(),
            Key::Named(NamedKey::ArrowLeft) => self.press_left(extend),
            Key::Named(NamedKey::ArrowRight) => self.press_right(extend),
            Key::Named(NamedKey::ArrowUp) => self.move_vertically(false, extend),
            Key::Named(NamedKey::ArrowDown) => self.move_vertically(true, extend),
            // The row, not the document (MDEDIT-192). Until then these two did what Ctrl+Home and
            // Ctrl+End do, so two of the four bindings were the same command twice.
            Key::Named(NamedKey::Home) => self.press_home(extend),
            Key::Named(NamedKey::End) => self.press_end(extend),
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
            Key::Named(NamedKey::Enter) => self.insert_newline(),
            Key::Named(NamedKey::Space) => self.insert(" "),
            Key::Character(text) => self.insert(text),
            _ => false,
        }
    }

    /// A key, while the dialog is up.
    ///
    /// **Everything is swallowed**, whether it did something or not — that is what modal means, and
    /// a key falling through to the document while a panel covers it is the failure this shape
    /// exists to prevent. A chord that is not one of the field's own is dropped rather than
    /// performed: Ctrl+S while a dialog is open should not save.
    fn dialog_key(&mut self, event: &KeyEvent) -> bool {
        let shift = self.modifiers.shift_key();

        // The same gate the find field uses, and for the same reason: a URL box that could not be
        // pasted into would be useless, and one that could not be typed `@` into would be
        // MDEDIT-102 with a smaller blast radius.
        if is_chord(self.modifiers.control_key(), self.modifiers.alt_key()) {
            let Key::Character(character) = &event.logical_key else {
                return true;
            };
            return match character.as_str() {
                "a" | "A" => self.press_dialog(&field::Press::SelectAll, false),
                "v" | "V" => {
                    let Some(text) = self.clipboard.read() else {
                        return true;
                    };
                    // One line only. A multi-line paste into a URL box is a paste of something that
                    // is not a URL, and putting its newlines in would write a construct that spans
                    // lines and parses as neither.
                    let line = text.lines().next().unwrap_or_default().to_owned();
                    self.press_dialog(&field::Press::Text(line), false)
                }
                _ => true,
            };
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.cancel_dialog();
                true
            }
            Key::Named(NamedKey::Enter) => {
                // A refusal leaves it open, which is the whole of the validation: there is nothing
                // to write without a destination, and closing on a press that did nothing would
                // look like the construct went in somewhere.
                self.accept_dialog();
                true
            }
            Key::Named(NamedKey::Tab) => self.tab_dialog(!shift),
            Key::Named(NamedKey::Backspace) => self.press_dialog(&field::Press::Backspace, shift),
            Key::Named(NamedKey::Delete) => self.press_dialog(&field::Press::Delete, shift),
            Key::Named(NamedKey::ArrowLeft) => self.press_dialog(&field::Press::Left, shift),
            Key::Named(NamedKey::ArrowRight) => self.press_dialog(&field::Press::Right, shift),
            Key::Named(NamedKey::Home) => self.press_dialog(&field::Press::Home, shift),
            Key::Named(NamedKey::End) => self.press_dialog(&field::Press::End, shift),
            // Up and Down move between fields as well as Tab, because a form is a column and that
            // is what a column invites.
            Key::Named(NamedKey::ArrowUp) => self.tab_dialog(false),
            Key::Named(NamedKey::ArrowDown) => self.tab_dialog(true),
            // **Space is a *named* key in winit, not a character**, so without this arm the catch-all
            // below swallowed it and a URL box could not hold a space — which is exactly the input
            // the angle-bracket escaping exists for. `find_key` has the same arm for the same reason.
            Key::Named(NamedKey::Space) => {
                self.press_dialog(&field::Press::Text(" ".to_owned()), false)
            }
            Key::Character(character) => {
                self.press_dialog(&field::Press::Text(character.to_string()), false)
            }
            // A modifier on its own, a function key, anything else: swallowed and ignored.
            _ => true,
        }
    }

    /// Routes one key press to the find field.
    ///
    /// Chords are **not** swallowed. Ctrl+S saves the document while the caret is in the search box,
    /// which is what anyone would expect; only the three that mean something to a text field —
    /// select all, copy, paste and cut — are taken here, and the rest go where they always went.
    fn find_key(&mut self, event: &KeyEvent) -> bool {
        let shift = self.modifiers.shift_key();
        // The same gate as the document's, and for the same reason: a search box that could not be
        // typed `@` into would be the MDEDIT-102 bug with a smaller blast radius.
        if is_chord(self.modifiers.control_key(), self.modifiers.alt_key()) {
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

    /// Opens the link dialog, pre-filled from the clipboard and the selection.
    ///
    /// # This used to write the link without asking
    ///
    /// It used the clipboard when it held something URL-shaped and left the destination empty
    /// otherwise, and its docs called that "the case that needs no dialog at all — copy a link,
    /// select some words, press Ctrl+K, done". MDEDIT-129 reverses that: a link has **two** parts,
    /// and inserting a skeleton for you to fill in only helps when you already know the shape.
    ///
    /// **The fast path survives as one extra keystroke.** The clipboard's URL and the selection are
    /// both pre-filled, so copy-select-Ctrl+K becomes copy-select-Ctrl+K-Enter — and you see what it
    /// is about to write before it writes it.
    pub fn link(&mut self) -> bool {
        self.open_dialog(dialog::Kind::Link)
    }

    /// Opens the image dialog, pre-filled from the selection.
    ///
    /// No clipboard read, which is where it differs from [`link`](Self::link) and always has: a URL
    /// on the clipboard is very often the link you meant, and a *path* on the clipboard is very
    /// often not the image you meant.
    pub fn image(&mut self) -> bool {
        self.open_dialog(dialog::Kind::Image)
    }

    /// Opens `kind`, pre-filled from what the document and the clipboard already know.
    fn open_dialog(&mut self, kind: dialog::Kind) -> bool {
        let mut open = dialog::Dialog::new(kind);

        // The text the construct will wrap: the selection, or the word at the caret. Taken from the
        // document rather than from the selection directly, so the dialog shows exactly the span its
        // result will replace.
        let target = self.document().dialog_target();
        let text = self.document().to_string();
        let existing = target.slice(&text).to_owned();
        if !existing.trim().is_empty() {
            open.prefill(kind.text(), &existing);
        }

        if kind == dialog::Kind::Link
            && let Some(url) = self
                .clipboard
                .read()
                .filter(|text| format::looks_like_url(text))
        {
            open.prefill(kind.destination(), url.trim());
        }

        // The first empty field, so the caret starts where there is something to type. With a URL
        // and a selection both present that is neither, and it stays on the destination — which is
        // the field you would correct if the guess was wrong.
        let first_empty = open
            .fields()
            .iter()
            .position(crate::field::Field::is_empty)
            .unwrap_or(kind.destination());
        open.focus(first_empty);

        self.dialog = Some(open);
        true
    }

    /// The dialog that is open, for a test to look at.
    #[must_use]
    pub fn dialog(&self) -> Option<&dialog::Dialog> {
        self.dialog.as_ref()
    }

    /// Writes what the open dialog describes, and closes it.
    ///
    /// **A dialog with an empty destination refuses**, which is the whole of the validation: a link
    /// with no URL says nothing. It stays open rather than closing on a press that did nothing.
    ///
    /// Returns whether the frame needs redrawing.
    pub fn accept_dialog(&mut self) -> bool {
        let Some(open) = self.dialog.as_ref() else {
            return false;
        };
        let Some(markdown) = open.markdown() else {
            return false;
        };
        self.dialog = None;
        self.follow_caret = true;
        self.document_mut().replace_target(&markdown)
    }

    /// Closes the dialog without touching the document.
    ///
    /// Returns whether there was one to close.
    pub fn cancel_dialog(&mut self) -> bool {
        self.dialog.take().is_some()
    }

    /// A key, while the dialog has the keyboard.
    ///
    /// Split out to be callable by a test, for the reason [`apply_chord`](Self::apply_chord) gives:
    /// a winit `KeyEvent` carries a private platform field and cannot be constructed.
    ///
    /// Returns whether the frame needs redrawing.
    pub fn press_dialog(&mut self, press: &field::Press, shift: bool) -> bool {
        let Some(open) = self.dialog.as_mut() else {
            return false;
        };
        open.focused_field().apply(press, shift);
        true
    }

    /// Tab, or Shift+Tab, while the dialog has the keyboard.
    ///
    /// Returns whether there was a dialog to move within.
    pub fn tab_dialog(&mut self, forwards: bool) -> bool {
        let Some(open) = self.dialog.as_mut() else {
            return false;
        };
        open.focus_next(forwards);
        true
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
    ///
    /// **Asks for the caret to be revealed**, because this is the tests' stand-in for typing and
    /// typing does — `handle_key` sets the same flag before it dispatches anything. Without it the
    /// seam was subtly unfaithful, and MDEDIT-49 is where that mattered: a test typing at the end of
    /// a long line saw no horizontal reveal and reported the feature broken when it was the test
    /// that had not asked for one.
    pub fn insert_text(&mut self, text: &str) -> bool {
        self.follow_caret = true;
        // The second of the two places a tooltip is dismissed by the keyboard, and it is not a
        // duplicate of the one in `handle_key`: that one covers the keys which do not insert —
        // arrows, Esc, Backspace — and this one is the funnel every *character* comes through,
        // including an IME commit and a paste. Text arriving in the document means the user is not
        // pointing at a button, however it got there.
        self.dismiss_panels();
        self.insert(text)
    }

    /// Moves both ends of the selection past any concealed marker they are inside (MDEDIT-194).
    ///
    /// **A caret inside a concealed marker writes into its syntax**: at byte 1 of `# Title` it draws
    /// before the `T`, and typing gave `#x Title`. Plain markup lets the caret stand there, and it
    /// survives into preview by more routes than one — the toggle, another tab (the mode is the
    /// editor's, the selection each tab's), undo restoring a plain-mode caret, a session, find and
    /// go-to-line. So rather than chase each, every character is checked where it arrives, in
    /// [`Self::insert`] — keys, IME commits, paste and Enter all come through it — and the toggle
    /// checks too so a selection is right before anything is deleted. The anchor moves as well, so replacing a selection cannot strand half a marker.
    ///
    /// Untouched when nothing moves, so a caret keeps the affinity that says which row it is on. In
    /// plain markup nothing moves, because `past_concealed` asks [`Self::concealed_at`].
    fn leave_concealed(&mut self) {
        let selection = self.document().selection();
        let anchor = self.past_concealed(selection.anchor());
        let head = self.past_concealed(selection.head());
        if (anchor, head) != (selection.anchor(), selection.head()) {
            self.document_mut()
                .set_selection(Selection::new(anchor, head));
        }
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
        self.leave_concealed();
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

    /// Backspace against the document.
    ///
    /// **A concealed marker goes whole.** With nothing revealed (MDEDIT-118) the bytes of `## ` are
    /// not on screen, so removing one of them is a keystroke with no visible effect and then, two
    /// presses later, a heading that suddenly is not one. The only meaning Backspace has against
    /// bytes nobody can see is "this is not a heading any more" — one `delete`, so one Ctrl+Z puts
    /// every byte back.
    ///
    /// Split out of the key handler to be callable by a test, for the reason [`Self::apply_chord`]
    /// gives: a winit `KeyEvent` carries a private platform field and cannot be constructed. Without
    /// this the whole of MDEDIT-118's editing half would be unreachable from a test.
    ///
    /// Returns whether the frame needs redrawing.
    pub fn press_backspace(&mut self) -> bool {
        // With something selected, Backspace removes the selection rather than the character before
        // it — which is also what makes it one edit and one undo step.
        if self.delete_selection() {
            return true;
        }
        let caret = self.document().selection().head();
        if caret == ByteOffset::ZERO {
            return false;
        }
        if let Some(marker) = self.concealed_before(caret) {
            return self.delete_marker(marker);
        }
        // `delete` already leaves the caret at the start of what it removed.
        let span = ByteSpan::new(self.previous_boundary(caret), caret);
        self.document_mut().delete(span).is_ok()
    }

    /// Forward delete, whole concealed markers included.
    ///
    /// The same rule as [`Self::press_backspace`] and for the same reason: half of an invisible `**`
    /// left behind becomes two asterisks that appear from nowhere.
    ///
    /// Returns whether the frame needs redrawing.
    pub fn press_delete(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let caret = self.document().selection().head();
        if let Some(marker) = self.concealed_after(caret) {
            return self.delete_marker(marker);
        }
        let span = ByteSpan::new(caret, self.next_boundary(caret));
        !span.is_empty() && self.document_mut().delete(span).is_ok()
    }

    /// Left arrow, with `extend` for Shift.
    ///
    /// Returns whether the frame needs redrawing.
    pub fn press_left(&mut self, extend: bool) -> bool {
        let target = self.caret_before(self.document().selection().head());
        self.select(target, extend);
        true
    }

    /// Right arrow, with `extend` for Shift.
    ///
    /// Returns whether the frame needs redrawing.
    pub fn press_right(&mut self, extend: bool) -> bool {
        let target = self.caret_after(self.document().selection().head());
        self.select(target, extend);
        true
    }

    /// Home, with `extend` for Shift: the start of the **visual row** the caret is on (MDEDIT-192).
    ///
    /// Visual rather than source, as Up and Down are: a wrapped paragraph is several rows, and each
    /// continuation has a start of its own. It is the point gesture a click far left of the row
    /// would be, so it reaches the row through the same hit test and cannot disagree with one.
    ///
    /// **In preview it stops after a concealed marker**, so Home on `- item` or `# Title` lands
    /// before the text rather than before the syntax. That is not a choice of style: a caret inside
    /// or before a concealed marker draws at the same x as its far edge, and `past_concealed` is
    /// the invariant every motion keeps. Plain markup conceals nothing, so there Home reaches the
    /// row's true start. There is no second press that toggles between the two.
    ///
    /// Returns whether the frame needs redrawing, and `false` before the first frame, when there are
    /// no rows to be on.
    pub fn press_home(&mut self, extend: bool) -> bool {
        let Some((offset, _)) = self.caret_row_edge(false) else {
            return false;
        };
        // In plain markup this is the offset itself: `concealed_at` asks the mode.
        let offset = self.past_concealed(offset);
        self.select(offset, extend);
        true
    }

    /// End, with `extend` for Shift: the end of the visual row the caret is on (MDEDIT-192).
    ///
    /// **The caret stays drawn on that row**, which is MDEDIT-188's affinity set from the keyboard.
    /// Where a line wraps, the end of one row and the start of the next are one byte, and only the
    /// gesture knows which it meant. A click past the row's end already says `Upstream` there, and
    /// End is the same point gesture, so typing after End appends to the row it ended.
    pub fn press_end(&mut self, extend: bool) -> bool {
        let Some((offset, affinity)) = self.caret_row_edge(true) else {
            return false;
        };
        self.select_with(offset, extend, affinity);
        true
    }

    /// Ctrl+Home or Ctrl+End: the start or end of the whole document, with `extend` for Shift.
    ///
    /// Where every editor puts them. The caret moves, so the scroll follows without saying so.
    pub fn press_document_edge(&mut self, end: bool, extend: bool) -> bool {
        let offset = if end {
            ByteOffset::new(self.document().len_bytes())
        } else {
            ByteOffset::ZERO
        };
        self.select(offset, extend);
        true
    }

    /// The byte a point beyond the left or right edge of the caret's row addresses, and its affinity.
    ///
    /// Aimed at the row's vertical middle, so the hit test cannot pick its neighbour whatever the
    /// row's height, and at an `x` no glyph can be beyond.
    fn caret_row_edge(&self, end: bool) -> Option<(ByteOffset, Affinity)> {
        let (index, row, _) = self.caret_place()?;
        let frame = self.last_frame.as_ref()?;
        let cached = self.chunk_layout(frame, index)?;
        let target = cached.as_hit(frame.chunks[index], &frame.text);
        let height = cached
            .layout
            .lines
            .get(row)
            .map_or(self.theme.body_size, |line| line.height);
        let middle = frame.tops[index] + hit::row_top(&target, row) + height / 2.0;
        let x = if end { f32::MAX } else { f32::MIN };
        self.offset_in_document(x, middle)
    }

    /// `offset`, moved past any concealed marker it is standing in.
    ///
    /// # The invariant the caret keeps
    ///
    /// With nothing revealed (MDEDIT-118) the bytes of `# ` draw nowhere, so offsets 0, 1 and 2 in
    /// `# Title` are all the same x. **A position inside a concealed marker is therefore not a place
    /// the caret may be**, and every motion normalises through here before and after it steps —
    /// otherwise Right from the line's start would land on the marker's far edge, which is the same
    /// pixel it started at, and read as a key that did nothing.
    ///
    /// The loop is a loop because two markers can abut with no content between them: `> ` and the
    /// backtick in ``> `code` `` are separate segments.
    fn past_concealed(&self, offset: ByteOffset) -> ByteOffset {
        let mut at = offset;
        while let Some(span) = self.concealed_at(at) {
            at = span.end();
        }
        at
    }

    /// What draws nowhere at `offset`: a concealed marker, or a picture (MDEDIT-152).
    ///
    /// `Document::concealed_at` answers the first. It cannot answer the second, and should not be
    /// asked to: whether a paragraph drew as a **picture** depends on a file existing on disk and
    /// decoding, which is the renderer's business. So the two are combined here, at the layer that
    /// knows both — the same shape as MDEDIT-126, where a grid hands `hit.rs` its row spans because
    /// the renderer knows something the navigation model does not.
    ///
    /// # Why the whole construct and not just the alt text
    ///
    /// An image with no alt text is already **one** `Marker::Image` segment, so the document alone
    /// steps over `![](x.png)` correctly and always did. Give it alt text and it becomes three
    /// segments — `![`, the alt, `](x.png)` — and the alt is `Content`, which is a legitimate caret
    /// stop right up until the moment a picture draws over it. Returning the construct's whole span
    /// makes the two spellings behave alike, which is the point.
    ///
    /// # The one-frame lag, which is real and bounded
    ///
    /// `picture_spans` is what the **last** frame drew, so a picture the caret is about to reach
    /// that has not been on screen yet is not in it, and the first Right into it stops in the alt
    /// text once. The next frame scrolls the caret into view, the picture is laid out, and the press
    /// after that steps over it.
    ///
    /// Living with that rather than recomputing per key press, because the alternative is resolving
    /// and stat-ing a path on every arrow — file I/O on the keystroke path, which is what MDEDIT-151
    /// exists to get *off* the UI thread. A once-per-image imprecision at the edge of the viewport
    /// is the cheaper error.
    ///
    /// # A picture is asked about first, and that ordering is load-bearing
    ///
    /// Its span **contains** the two `Marker::Image` segments inside it. Asking the document first
    /// answers `](cat.png)` for the last byte of the construct — a true answer about a smaller thing
    /// — and Backspace then hands that to `delete_marker`, which pairs it with the `![` and takes
    /// both, leaving the alt text behind as prose. That is precisely the stranding `delete_marker`
    /// exists to prevent, arrived at through the front door. The outer span is the whole truth here,
    /// so it goes first.
    /// # A span that no longer fits the buffer is ignored
    ///
    /// The lag above has a sharp edge: an edit between two key presses can shorten the document
    /// under a span, and `past_concealed` would then hand the caret an offset past the end. Nothing
    /// panics on one today — the selection tolerates it and the next frame corrects it — but an
    /// out-of-range offset is a loaded gun for whoever next slices with one, and it costs a
    /// comparison to unload.
    ///
    /// # Plain markup conceals nothing (MDEDIT-194)
    ///
    /// So the answer there is always `None`. `Document::concealed_at` answers by **language** —
    /// plain markup is a way of drawing the parse, not a different one — and asked unfiltered it
    /// made every drawn marker an uncrossable step: Left from the `T` of `# Title` did nothing and
    /// Right from byte 0 jumped to 3, while a click could land between. Plain draws no picture
    /// either, so the picture spans go too, rather than trusting the last frame to have emptied
    /// them. Every motion, Home, Backspace and Delete ask here, so one test of the mode covers all.
    fn concealed_at(&self, offset: ByteOffset) -> Option<ByteSpan> {
        if self.mode == view::Mode::Plain {
            return None;
        }
        let end = self.document().len_bytes();
        self.picture_spans
            .iter()
            .find(|span| span.start() <= offset && offset < span.end() && span.end().get() <= end)
            .copied()
            .or_else(|| self.document().concealed_at(offset))
    }

    /// Where the caret goes from `offset` on Right.
    fn caret_after(&self, offset: ByteOffset) -> ByteOffset {
        let from = self.past_concealed(offset);
        self.past_concealed(self.next_boundary(from))
    }

    /// Where the caret goes from `offset` on Left.
    ///
    /// The mirror of [`Self::caret_after`], with one asymmetry worth stating: a marker's *start* is
    /// as unreachable as its middle, because it draws at the same x as its end. Going left from the
    /// first letter of `# Title` therefore lands **at the end of the previous line**, not at byte 0,
    /// which is what a reader means by pressing Left at the start of a heading.
    ///
    /// When there is no previous line — the marker starts the document — there is nowhere further
    /// left to go, so Left is a no-op. That is the answer Left gives at the start of any document;
    /// it just arrives at it two bytes early.
    fn caret_before(&self, offset: ByteOffset) -> ByteOffset {
        let from = self.past_concealed(offset);
        let mut at = self.previous_boundary(from);
        while let Some(span) = self.concealed_at(at) {
            if span.start() == ByteOffset::ZERO {
                return from;
            }
            at = self.previous_boundary(span.start());
        }
        at
    }

    /// Removes a concealed marker, **and the other half of its construct** if it has one.
    ///
    /// MDEDIT-118 took a marker whole, on the argument that removing one byte of something invisible
    /// is a keystroke with no visible effect. It missed that the other half is invisible too:
    /// deleting a link's `[` left `](https://example.com)` behind, and a stranded marker stops being
    /// a marker and **becomes visible text** — a URL nobody could see, appearing mid-sentence, from a
    /// keystroke that looked like it should have taken a letter.
    ///
    /// Both spans in one transaction, so one Ctrl+Z restores the link rather than half of it. The
    /// later span goes first, because removing the earlier one would shift it.
    ///
    /// See `Document::paired_marker` for which constructs pair and why the delimiter runs do not.
    fn delete_marker(&mut self, marker: ByteSpan) -> bool {
        let Some(pair) = self.document().paired_marker(marker) else {
            return self.document_mut().delete(marker).is_ok();
        };
        let (first, second) = if marker.start() < pair.start() {
            (marker, pair)
        } else {
            (pair, marker)
        };
        let mut removed = false;
        self.document_mut().transact(|doc| {
            removed = doc.delete(second).is_ok() && doc.delete(first).is_ok();
            doc.set_selection(Selection::caret(first.start()));
        });
        removed
    }

    /// The concealed marker the caret is standing immediately after, if any.
    ///
    /// What Backspace deletes instead of one character. Its whole span, in one edit and one undo
    /// step, because the bytes are invisible and taking them off one at a time would be a key that
    /// does nothing until it suddenly does something.
    fn concealed_before(&self, offset: ByteOffset) -> Option<ByteSpan> {
        if offset == ByteOffset::ZERO {
            return None;
        }
        self.concealed_at(self.previous_boundary(offset))
    }

    /// The concealed marker the caret is standing immediately before, if any.
    ///
    /// Delete's counterpart to [`Self::concealed_before`], and it is here for the same reason: a
    /// forward delete that took one byte of an invisible `**` would leave the other one behind as
    /// text, which is a worse answer than either taking all of it or none.
    fn concealed_after(&self, offset: ByteOffset) -> Option<ByteSpan> {
        self.concealed_at(offset)
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

    /// The glyph atlas the last frame's quads address.
    ///
    /// Public for the same reason [`build_frame`](Self::build_frame) is, one step further along: a
    /// test that draws those quads has to upload the texture they were packed into, and packing is
    /// this editor's, not the renderer's (MDEDIT-134). Read-only, so nothing outside can pack into
    /// it — the quads and the atlas would then disagree about where a glyph is.
    #[must_use]
    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// Lays the document out and turns it into quads.
    ///
    /// `width` and `height` are the window's, in physical pixels. Public so a headless test can
    /// assert what gets drawn without a GPU: nothing here touches the renderer.
    pub fn build_frame(&mut self, width: f32, height: f32, scale: f32) -> Vec<QuadInstance> {
        let mut quads = Vec::new();
        // Rebuilt from scratch: what was on screen last frame says nothing about this one, and a
        // picture scrolled off has to stop being drawn.
        self.forget_last_frames_pictures();
        // Everything the chrome has not taken. One place decides it; see `crate::content`.
        self.window_height = height / scale;
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
        if following && let Some((index, caret)) = self.caret_position(&frame) {
            let caret_top = frame.tops[index] + caret.top;
            self.scroll
                .reveal(caret_top, caret_top + caret.height, frame.content, visible);
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
        if following
            && !self.wrap
            && let Some((_, caret)) = self.caret_position(&frame)
        {
            self.pan.reveal(
                (caret.x - PAN_MARGIN).max(0.0),
                caret.x + PAN_MARGIN,
                self.widest,
                content.width(),
            );
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
        self.draw_find(&mut quads, scale);
        self.draw_pane(&mut quads, height, scale);
        self.draw_status_bar(&mut quads, width, height, scale);
        self.draw_scrollbar(&mut quads, width, scale);
        // Genuinely last. `AGENTS.md`'s painter-order rule is that
        // chrome drawn first gets painted over, and a tooltip is the one thing that must be on top
        // of everything — including the bar it describes, which it does not overlap, and the
        // document, which it does.
        self.draw_tooltip(&mut quads, width, scale);
        // After even the tooltip. A list is the only thing that can be open *and* hit-tested, so it
        // is the top of the z-order in both directions — see `Editor::click`.
        self.draw_menu(&mut quads, scale);
        // And after even that: the dialog is modal, so nothing may be drawn over it (MDEDIT-129).
        self.draw_dialog(&mut quads, width, height, scale);

        self.finish_frame(frame);
        quads
    }

    /// Keeps what a frame measured, and gives the pointer its shape from it.
    ///
    /// The frame is kept for the next click and the next Up or Down: both need to know where the
    /// lines ended up, and this is the only place that knows.
    ///
    /// **The pointer has not moved, and what is under it has** (MDEDIT-176): a scroll, a panel, a
    /// tab switch or Ctrl+/ all change the answer with no `CursorMoved` to prompt it. It goes after
    /// the frame is kept, because that is what the shape over the document is read from.
    fn finish_frame(&mut self, frame: Frame) {
        self.last_frame = Some(frame);
        let _ = self.refresh_pointer();
    }

    /// Draws the modal dialog, and the dim over everything behind it (MDEDIT-129).
    ///
    /// **The dim is the modality**, not decoration. It is the only thing on screen saying that the
    /// document is not taking keys. The find panel floats over unchanged text too (MDEDIT-177) and
    /// says what it is with a border and a panel colour — which is enough for a surface that takes
    /// only its own clicks, and is not enough for one that takes every click in the window.
    ///
    /// Drawn after every other piece of chrome including the tooltip, which is what makes it the
    /// topmost thing; `click` asks about it first, which is the same z-order read the other way.
    fn draw_dialog(&mut self, quads: &mut Vec<QuadInstance>, width: f32, height: f32, scale: f32) {
        let Some(open) = self.dialog.clone() else {
            return;
        };
        let kind = open.kind();
        let labels = kind.labels();
        let (x, y) = self.dialog_at();
        let panel_height = dialog::height_of(labels.len());

        // The dim, over the whole window. `selection` is the only translucent colour in the palette
        // and is far too blue for this, so it is the page colour at a third — which darkens a dark
        // theme and lightens nothing, and reads as "behind glass" in both.
        quads.push(Self::solid(
            [0.0, 0.0, width, height],
            view::Rgba {
                a: 0.55,
                ..self.theme.background
            },
        ));

        // A hairline edge under the ground, as the tooltip and the menus have.
        quads.push(Self::solid(
            [
                (x - 1.0) * scale,
                (y - 1.0) * scale,
                (dialog::WIDTH + 2.0) * scale,
                (panel_height + 2.0) * scale,
            ],
            self.theme.line,
        ));
        quads.push(Self::solid(
            [
                x * scale,
                y * scale,
                dialog::WIDTH * scale,
                panel_height * scale,
            ],
            self.theme.panel,
        ));

        let style = TextStyle::body(self.theme.body_size);
        let drop = self.theme.body_size * 0.35;
        let baseline = move |top: f32, box_height: f32| top + box_height / 2.0 + drop;

        self.draw_label(
            quads,
            kind.title(),
            style,
            x + dialog::PADDING,
            y + dialog::PADDING + baseline(0.0, dialog::TITLE_HEIGHT),
            self.theme.text,
            scale,
            dialog::WIDTH - dialog::PADDING * 2.0,
        );

        for (index, label) in labels.iter().enumerate() {
            self.draw_dialog_field(quads, &open, index, label, (x, y), style, scale);
        }

        self.draw_dialog_buttons(quads, &open, (x, y), style, scale);
    }

    /// One labelled field: its name, its box, its text and its caret.
    ///
    /// Split out of `draw_dialog` only because that crossed a hundred lines, which is a real limit
    /// here. The seam is the natural one — everything below is about one row.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_dialog_field(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        open: &dialog::Dialog,
        index: usize,
        label: &str,
        at: (f32, f32),
        style: TextStyle,
        scale: f32,
    ) {
        let (x, y) = at;
        let drop = self.theme.body_size * 0.35;
        let baseline = move |top: f32, box_height: f32| top + box_height / 2.0 + drop;
        let (box_x, box_y, box_width, box_height) = dialog::field_rect(index, (x, y));

        self.draw_label(
            quads,
            label,
            style,
            x + dialog::PADDING,
            baseline(box_y, box_height),
            self.theme.marker,
            scale,
            dialog::LABEL_WIDTH - 8.0,
        );

        // The focused box takes the raised surface and a brighter edge, which is the same two
        // signals a lit toolbar button uses. Colour alone would fail the pack's contract.
        let focused = open.focused() == index;
        quads.push(Self::solid(
            [
                (box_x - 1.0) * scale,
                (box_y - 1.0) * scale,
                (box_width + 2.0) * scale,
                (box_height + 2.0) * scale,
            ],
            if focused {
                self.theme.caret
            } else {
                self.theme.line
            },
        ));
        quads.push(Self::solid(
            [
                box_x * scale,
                box_y * scale,
                box_width * scale,
                box_height * scale,
            ],
            self.theme.background,
        ));

        self.draw_dialog_text(
            quads,
            &open.fields()[index],
            (box_x, box_y, box_width, box_height),
            style,
            (focused, baseline(box_y, box_height)),
            scale,
        );
    }

    /// What is inside one box: the selection behind, the glyphs, the caret in front.
    ///
    /// Split out of [`Self::draw_dialog_field`] only because the two together are past the line
    /// limit; the seam is the box's chrome against its contents.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_dialog_text(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        field: &field::Field,
        box_rect: (f32, f32, f32, f32),
        style: TextStyle,
        focus: (bool, f32),
        scale: f32,
    ) {
        let (box_x, box_y, box_width, box_height) = box_rect;
        let (focused, baseline) = focus;
        {
            let text = field.text().to_owned();
            let caret_at = field.caret();
            let (from, to) = field.selection();

            let label = (!text.is_empty())
                .then(|| self.lay_out_label(&text, style))
                .flatten();

            // Where a byte sits along the laid-out text: the glyph that starts at or after it, or
            // past the last one. **The same rule the find field uses**, and it has to be the same
            // one — a caret placed by measuring a prefix instead sits half a glyph away from the
            // highlight, which is what this drew before.
            let x_of = |offset: usize| -> f32 {
                label.as_ref().map_or(0.0, |label| {
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

            // **The box scrolls to keep the caret in it**, which the find field does not need to and
            // this does: a URL is longer than the box far more often than a search term is, and a
            // field you cannot see what you are typing into is not a field.
            let inner = box_width - 12.0;
            let caret_x = x_of(caret_at);
            let scroll = (caret_x - inner + CARET_WIDTH).max(0.0);
            let left = box_x + 6.0 - scroll;

            if from < to && focused {
                let (start, end) = (x_of(from), x_of(to));
                // Clamped to the box, so a selection running off either edge stops at it rather
                // than painting over the label column or the panel's border.
                let start = (left + start).max(box_x + 6.0);
                let end = (left + end).min(box_x + box_width - 6.0);
                if end > start {
                    quads.push(Self::solid(
                        [
                            start * scale,
                            (box_y + 4.0) * scale,
                            (end - start) * scale,
                            (box_height - 8.0) * scale,
                        ],
                        self.theme.selection,
                    ));
                }
            }

            if let Some(label) = &label {
                // Clipped at both edges, because scrolling means glyphs can now fall off the left.
                let glyphs: Vec<_> = label
                    .glyphs
                    .iter()
                    .filter(|glyph| {
                        let at = left + glyph.pos.x;
                        at + glyph.advance > box_x + 6.0 && at < box_x + box_width - 6.0
                    })
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
                        (left + caret_x) * scale,
                        (box_y + 4.0) * scale,
                        CARET_WIDTH * scale,
                        (box_height - 8.0) * scale,
                    ],
                    self.theme.caret,
                ));
            }
        }
    }

    /// Cancel and Insert, at the foot of the panel.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_dialog_buttons(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        open: &dialog::Dialog,
        at: (f32, f32),
        style: TextStyle,
        scale: f32,
    ) {
        let drop = self.theme.body_size * 0.35;
        let baseline = move |top: f32, box_height: f32| top + box_height / 2.0 + drop;

        let [cancel, accept] = dialog::button_rects(open.fields().len(), at);
        // Accept is dimmed when there is nothing to write, which is the validation made visible
        // rather than only felt when Enter does nothing.
        let can_accept = open.markdown().is_some();
        for (rect, text, lit) in [(cancel, "Cancel", false), (accept, "Insert", can_accept)] {
            quads.push(Self::solid(
                [
                    rect.0 * scale,
                    rect.1 * scale,
                    rect.2 * scale,
                    rect.3 * scale,
                ],
                if lit {
                    self.theme.tab_active
                } else {
                    self.theme.tab_inactive
                },
            ));
            let colour = if lit || text == "Cancel" {
                self.theme.text
            } else {
                self.theme.gutter
            };
            let width_of = self.measure_label(text, style);
            self.draw_label(
                quads,
                text,
                style,
                rect.0 + (rect.2 - width_of) / 2.0,
                baseline(rect.1, rect.3),
                colour,
                scale,
                rect.2,
            );
        }
    }

    /// Draws the list a toolbar button opened, if one is open.
    ///
    /// A row is its icon in a fixed column and then its words, so the words line up down the list
    /// whether or not each row's icon resolved. Both come out of `toolbar_labels`, which by now
    /// holds the button faces, the tooltips and these — one cache of laid-out chrome text, cleared
    /// by `set_zoom` in one place.
    ///
    /// The lit row is the one the caret is already in, which for the heading list is the level being
    /// shown on the button. It uses `tab_active`, as a lit button on the bar does.
    fn draw_menu(&mut self, quads: &mut Vec<QuadInstance>, scale: f32) {
        let Some(menu) = self.open_menu.clone() else {
            return;
        };
        let (x, y) = menu.at;
        let height = if menu.kind == toolbar::Menu::TableSize {
            popup::grid::height()
        } else {
            popup::height_of(menu.len())
        };

        quads.push(Self::solid(
            [
                (x - 1.0) * scale,
                (y - 1.0) * scale,
                (menu.width + 2.0) * scale,
                (height + 2.0) * scale,
            ],
            self.theme.line,
        ));
        quads.push(Self::solid(
            [x * scale, y * scale, menu.width * scale, height * scale],
            self.theme.panel,
        ));

        if menu.kind == toolbar::Menu::TableSize {
            self.draw_size_picker(quads, scale, menu.at);
        } else if menu.kind == toolbar::Menu::Tabs {
            self.draw_tab_menu_rows(quads, scale, &menu);
        } else {
            self.draw_menu_rows(quads, scale, &menu);
        }
    }

    /// Draws the rows of the list of hidden tabs (MDEDIT-179): each file's type icon in the icon
    /// column, as its tab and its pane row draw it, and its name beside it.
    fn draw_tab_menu_rows(&mut self, quads: &mut Vec<QuadInstance>, scale: f32, menu: &OpenMenu) {
        let (x, y) = menu.at;
        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        let baseline_offset = popup::ROW_HEIGHT / 2.0 + self.theme.body_size * 0.22;

        for (row, index) in menu.tabs.clone().into_iter().enumerate() {
            let top = y + popup::row_top(row);
            if self.hovered_row == Some(row) {
                quads.push(Self::solid(
                    [
                        (x + 2.0) * scale,
                        top * scale,
                        (menu.width - 4.0) * scale,
                        popup::ROW_HEIGHT * scale,
                    ],
                    self.theme.selection,
                ));
            }

            let (face, face_style) = self.row_icon_face(self.icon_for(index), style);
            self.draw_label(
                quads,
                &face,
                face_style,
                x + popup::PADDING,
                top + baseline_offset,
                self.theme.text,
                scale,
                popup::ICON_COLUMN - popup::PADDING,
            );

            let name = self.workspace.tabs()[index].display_name();
            self.draw_label(
                quads,
                &name,
                style,
                x + popup::ICON_COLUMN,
                top + baseline_offset,
                self.theme.text,
                scale,
                (menu.width - popup::ICON_COLUMN - popup::PADDING).max(0.0),
            );
        }
    }

    /// Draws a list panel's rows into the frame `draw_menu` has already put behind them.
    fn draw_menu_rows(&mut self, quads: &mut Vec<QuadInstance>, scale: f32, menu: &OpenMenu) {
        let (x, y) = menu.at;
        let icon_style = TextStyle {
            family: mdedit_text::FontFamily::Icon,
            ..TextStyle::body(self.theme.body_size * TOOLBAR_ICON)
        };
        let letters_style = TextStyle::body(self.theme.body_size * TOOLBAR_LETTERS);
        let text_style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);

        for (index, action) in menu.rows.clone().into_iter().enumerate() {
            let top = y + popup::row_top(index);
            // **Two grounds, and a row can have both.** Lit is "you are already in this one" and
            // uses the raised surface a lit button on the bar does; hovered is "this is what you
            // would press" and uses the selection tint, which is translucent and stacks over it —
            // the same two-tints-on-one-row arrangement MDEDIT-77 made the caret line and the
            // selection. A menu whose only feedback was the lit row would look inert on every
            // document that is not already a heading.
            let mut ground = |colour| {
                quads.push(Self::solid(
                    [
                        (x + 2.0) * scale,
                        top * scale,
                        (menu.width - 4.0) * scale,
                        popup::ROW_HEIGHT * scale,
                    ],
                    colour,
                ));
            };
            if self.is_lit(action) {
                ground(self.theme.tab_active);
            }
            if self.hovered_row == Some(index) {
                ground(self.theme.selection);
            }

            // The column holds the row's **face**: its icon, or — for the heading levels, which have
            // none by design — its letterform. That is what makes the heading list read H1 to H6
            // down its own column rather than being six lines of prose, and it is the same face the
            // button shows.
            let (face, style, centred) = match action.icon() {
                Some(icon) => (
                    icon.to_string(),
                    icon_style,
                    self.theme.body_size * TOOLBAR_ICON * 0.41,
                ),
                None => (
                    action.label(),
                    letters_style,
                    self.theme.body_size * TOOLBAR_LETTERS * 0.35,
                ),
            };
            {
                let glyph = face;
                if !self.toolbar_labels.contains_key(&glyph)
                    && let Some(label) = self.lay_out_label(&glyph, style)
                {
                    // Only kept when it resolved. A `.notdef` box in a list that also has words
                    // is worse than a blank column, and the words carry the row regardless.
                    if !label.glyphs.iter().any(|g| g.key.glyph.is_notdef()) {
                        self.toolbar_labels.insert(glyph.clone(), label);
                    }
                }
                if let Some(label) = self.toolbar_labels.get(&glyph) {
                    let where_ = LabelAt {
                        // **Centred in the column, not indented into it.** Left-aligning at
                        // `PADDING` left whatever air the face did not use on the right, so a
                        // heading list — which is the column and nothing else — had ten pixels down
                        // one side and seven down the other, and read as shoved right.
                        x: x + (popup::ICON_COLUMN - label.width) / 2.0,
                        // Off the **face's own** size, not the body size. Using the body size put
                        // the icon's ink about three pixels high in the row, which reads as the
                        // words beside it sitting low. Same ratio `face_baseline` uses on the bar.
                        baseline: top + popup::ROW_HEIGHT / 2.0 + centred,
                        scale,
                        colour: self.theme.text,
                    };
                    let glyphs = label.glyphs.clone();
                    self.draw_label_glyphs(quads, &glyphs, where_);
                }
            }

            let Some(text) = popup::text_for(action) else {
                continue;
            };
            if !self.toolbar_labels.contains_key(&text)
                && let Some(label) = self.lay_out_label(&text, text_style)
            {
                self.toolbar_labels.insert(text.clone(), label);
            }
            let Some(label) = self.toolbar_labels.get(&text) else {
                continue;
            };
            let where_ = LabelAt {
                x: x + popup::ICON_COLUMN,
                baseline: top + popup::ROW_HEIGHT / 2.0 + self.theme.body_size * 0.22,
                scale,
                colour: self.theme.text,
            };
            let glyphs = label.glyphs.clone();
            self.draw_label_glyphs(quads, &glyphs, where_);
        }
    }

    /// Draws the rows × columns grid the Table button opens, and the line naming what it would
    /// insert (MDEDIT-127).
    ///
    /// **Every cell up to and including the hovered one is filled**, which is the gesture: the lit
    /// block *is* the table, so its shape is read rather than counted. Lighting only the cell under
    /// the pointer would make the caption the sole feedback, and a caption is a thing you read where
    /// this is a thing you see.
    ///
    /// Two quads a cell — an outline and its interior — for eighty cells, on a panel that only
    /// exists while it is open.
    fn draw_size_picker(&mut self, quads: &mut Vec<QuadInstance>, scale: f32, at: (f32, f32)) {
        let (x, y) = at;
        let lit = self.hovered_cell;

        for row in 0..popup::grid::ROWS {
            for column in 0..popup::grid::COLUMNS {
                let (cell_x, cell_y) = popup::grid::cell_at(row, column);
                let (cell_x, cell_y) = (x + cell_x, y + cell_y);
                let side = popup::grid::CELL;
                quads.push(Self::solid(
                    [cell_x * scale, cell_y * scale, side * scale, side * scale],
                    self.theme.line,
                ));
                let inside =
                    lit.is_some_and(|(hot_row, hot_column)| row <= hot_row && column <= hot_column);
                quads.push(Self::solid(
                    [
                        (cell_x + 1.0) * scale,
                        (cell_y + 1.0) * scale,
                        (side - 2.0) * scale,
                        (side - 2.0) * scale,
                    ],
                    if inside {
                        self.theme.selection
                    } else {
                        self.theme.panel
                    },
                ));
            }
        }

        // **Named rather than blank when nothing is hovered.** The strip's height is reserved either
        // way so the panel does not change size under the pointer, and an empty strip along the
        // bottom of a panel reads as something that failed to draw.
        let caption = lit.map_or_else(
            || "Table size".to_owned(),
            |(row, column)| popup::grid::caption(row + 1, column + 1),
        );
        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        if !self.toolbar_labels.contains_key(&caption)
            && let Some(label) = self.lay_out_label(&caption, style)
        {
            self.toolbar_labels.insert(caption.clone(), label);
        }
        let Some(label) = self.toolbar_labels.get(&caption) else {
            return;
        };
        let where_ = LabelAt {
            x: x + (popup::grid::width() - label.width) / 2.0,
            baseline: y + popup::grid::caption_middle() + self.theme.body_size * 0.22,
            scale,
            colour: self.theme.text,
        };
        let glyphs = label.glyphs.clone();
        self.draw_label_glyphs(quads, &glyphs, where_);
    }

    /// What the panel will actually say, laid out and in the label cache when this returns.
    ///
    /// A button's tooltip is a few words and is drawn as it is. **A path can be wider than the
    /// window**, and `tooltip::place` clamps a panel to the edge without shortening it, so a deep
    /// path would be drawn off the screen (MDEDIT-173). The candidates come from
    /// [`tooltip::elisions`] longest first, and this takes the first that fits — measuring each,
    /// because only the text engine knows how wide a string is.
    ///
    /// **It stops at the last candidate rather than failing**: in a window too narrow even for
    /// `C:\…\plan.md` the shortest is drawn and `place` keeps its left edge, which is the rule that
    /// module already follows for a panel wider than its window.
    ///
    /// Measuring costs a layout per candidate, once, on a hover that has already waited out the
    /// 400 ms dwell — and the cache is keyed by the string, so the frames after this one measure
    /// nothing.
    ///
    /// **The candidates that did not fit stay in the cache on purpose.** `draw_tooltip` calls this
    /// on *every* frame a panel is showing, so evicting them would re-lay-out each loser once a
    /// frame for as long as the pointer rests — recurring frame time, against a p99 the performance
    /// targets treat as a commitment, to save memory that is already bounded: at most one label per
    /// interior component per tab hovered since the last `set_zoom`.
    fn tooltip_line(&mut self, over: Hovered, full: String, available: f32) -> String {
        // The candidates are worked out first, so the measuring below borrows `self` alone: a button
        // has one candidate and a path has one per interior component.
        let candidates = match over {
            Hovered::Tab(index) | Hovered::Row(index) => self
                .workspace
                .tabs()
                .get(index)
                .and_then(mdedit_session::Tab::path)
                .map_or_else(|| vec![full.clone()], tooltip::elisions),
            Hovered::Button(_) => vec![full.clone()],
        };

        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        let last = candidates.len().saturating_sub(1);
        for (at, candidate) in candidates.into_iter().enumerate() {
            if !self.tooltip_labels.contains_key(&candidate)
                && let Some(label) = self.lay_out_label(&candidate, style)
            {
                self.tooltip_labels.insert(candidate.clone(), label);
            }
            let fits = self
                .tooltip_labels
                .get(&candidate)
                .is_some_and(|label| tooltip::width_of(label.width) <= available);
            if fits || at == last {
                return candidate;
            }
        }
        full
    }

    /// Draws the panel that says what the hovered button does, or where the hovered file is.
    ///
    /// Its line is laid out **once** and kept, like every other piece of chrome text here — the
    /// eighth cache of that kind, and `set_zoom` clears it with the rest. Which line that is comes
    /// from [`tooltip_line`](Self::tooltip_line), because a path may not fit the window.
    ///
    /// **Never hit-tested.** There is no `click_tooltip`, deliberately: the panel hangs over the
    /// document, and a click landing on it has to reach the text underneath. "It is only a tooltip"
    /// is exactly how a control ends up unpressable.
    fn draw_tooltip(&mut self, quads: &mut Vec<QuadInstance>, width: f32, scale: f32) {
        let Some(over) = self.tooltip else {
            return;
        };
        // Where to point, and what to say. Both can come back `None` for the same reason: the frame
        // that recorded this hover is not the frame being drawn. The bar may have been re-laid-out
        // narrower and lost the button; a tab may have been closed. Nothing to point at, so nothing
        // to draw.
        let Some((centre, below)) = self.tooltip_anchor(over) else {
            return;
        };
        let Some(line) = self.tooltip_text() else {
            return;
        };
        let line = self.tooltip_line(over, line, width / scale);
        let Some(label) = self.tooltip_labels.get(&line) else {
            return;
        };

        let panel_width = tooltip::width_of(label.width);
        let (x, y) = tooltip::place(panel_width, centre, below, width / scale);

        // A hairline edge under the ground, one pixel proud on each side. `theme.line` has had no
        // consumer since MDEDIT-71 put it there; this is it.
        quads.push(Self::solid(
            [
                (x - 1.0) * scale,
                (y - 1.0) * scale,
                (panel_width + 2.0) * scale,
                (tooltip::HEIGHT + 2.0) * scale,
            ],
            self.theme.line,
        ));
        quads.push(Self::solid(
            [
                x * scale,
                y * scale,
                panel_width * scale,
                tooltip::HEIGHT * scale,
            ],
            self.theme.panel,
        ));

        let where_ = LabelAt {
            x: x + tooltip::PADDING,
            baseline: y + tooltip::HEIGHT / 2.0 + self.theme.body_size * 0.3,
            scale,
            colour: self.theme.text,
        };
        let glyphs = label.glyphs.clone();
        self.draw_label_glyphs(quads, &glyphs, where_);
    }

    /// What a tooltip points at: the horizontal centre of the thing, and the edge to sit below.
    ///
    /// Three different pieces of chrome at three different heights, and all three place the same
    /// way — under what they describe, never under the pointer, which is [`tooltip::place`]'s rule
    /// and its reason. A pane row's "below" is the row's own bottom rather than the pane's, because
    /// a pane is as tall as the window and a panel at the foot of it points at nothing.
    fn tooltip_anchor(&self, over: Hovered) -> Option<(f32, f32)> {
        match over {
            Hovered::Button(action) => self
                .toolbar_buttons
                .iter()
                .find(|button| button.action == action)
                .map(|button| (button.x + button.width / 2.0, toolbar::HEIGHT)),
            Hovered::Tab(index) => {
                self.strip
                    .tabs
                    .iter()
                    .find(|rect| rect.index == index)
                    .map(|rect| {
                        (
                            rect.x + rect.width / 2.0,
                            self.toolbar_height() + tabs::STRIP_HEIGHT,
                        )
                    })
            }
            Hovered::Row(index) => {
                self.row_rects
                    .iter()
                    .find(|rect| rect.index == index)
                    .map(|rect| {
                        (
                            pane::PANE_WIDTH / 2.0,
                            self.toolbar_height() + rect.bottom(),
                        )
                    })
            }
        }
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

        for action in &actions {
            self.button_face(*action);
        }

        let widths: Vec<f32> = actions
            .iter()
            .map(|action| {
                self.toolbar_labels
                    .get(&self.face_key(*action))
                    .map_or(0.0, |label| label.width)
            })
            .collect();
        let logical_width = width / scale;
        self.window_width = logical_width;
        // MDEDIT-116: `overflowing` rather than `layout`, and the difference is that what does not
        // fit is reported rather than dropped on the floor.
        let row = toolbar::overflowing(&actions, &widths, logical_width);
        self.toolbar_buttons = row.buttons;
        self.dropped = row.dropped;
        if self.dropped.is_empty() {
            // The window was widened while the overflow list was open, so the list is about buttons
            // that are now on the bar. Closing it is the only honest answer — leaving it would offer
            // the same action twice, and re-pointing it at nothing would leave an empty panel.
            if self
                .open_menu
                .as_ref()
                .is_some_and(|open| open.kind == toolbar::Menu::Overflow)
            {
                self.open_menu = None;
            }
        } else {
            // The overflow button has a face like any other, and it only exists on some frames.
            self.button_face(toolbar::Action::More);
        }

        // MDEDIT-84: the bar is the topmost chrome, and the tab strip is drawn below it. See
        // `build_tab_strip` for the other half, and `click_toolbar` and `click_strip` for the two
        // hit tests that have to agree with both.
        let top = 0.0;
        quads.push(Self::solid(
            [0.0, top * scale, width, toolbar::HEIGHT * scale],
            self.theme.strip,
        ));

        // Both answers are collected before the loop, because drawing borrows `self` mutably and
        // asking mid-loop would not compile. `is_lit` is a state and `is_enabled` is a capability;
        // a button can be neither, either or — for a heading the caret is already in, on a document
        // with nothing to save — both.
        let states: Vec<(bool, bool)> = self
            .toolbar_buttons
            .iter()
            .map(|button| (self.is_lit(button.action), self.is_enabled(button.action)))
            .collect();
        for (button, (active, enabled)) in self.toolbar_buttons.clone().into_iter().zip(states) {
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
            let key = self.face_key(button.action);
            let Some(label) = self.toolbar_labels.get(&key) else {
                continue;
            };
            let where_ = LabelAt {
                x: button.x + (button.width - label.width) / 2.0,
                baseline: top + self.face_baseline(label, &key, button.action),
                scale,
                // `gutter` is the palette's role for "present but not the thing you are reading",
                // and MDEDIT-72 chose its value so that it clears 4.5:1 on a panel as well as on the
                // page. The bar's ground is `strip`, so `contrast.rs` asserts that pair too — a
                // dimmed button has to be legible, not merely faint.
                colour: if enabled {
                    self.theme.text
                } else {
                    self.theme.gutter
                },
            };
            let glyphs = label.glyphs.clone();
            self.draw_label_glyphs(quads, &glyphs, where_);
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
            toolbar::Action::Heading(level) => {
                self.caret_block_kind() == Some(BlockKind::Heading(level))
            }
            // A task lights its kind as well as Task since MDEDIT-144: the kind ignores the box.
            toolbar::Action::Bullet => doc.carries(Block::Bullet),
            toolbar::Action::Ordered => doc.carries(Block::Ordered),
            // Lit in any list, as Headings is in any heading; which kind is the face's job.
            toolbar::Action::Lists => doc.carries(Block::Bullet) || doc.carries(Block::Ordered),
            toolbar::Action::Task => doc.carries(Block::Task),
            toolbar::Action::Quote => doc.is_quoted(),
            toolbar::Action::Fence => self.caret_block_kind() == Some(BlockKind::CodeBlock),
            // Neither is a state. A rule is inserted and a link is made; there is no "inside a
            // horizontal rule" for a button to reflect.
            //
            // Nor is any of the standard set: Save and Paste are things you do, not modes the caret
            // can be in. Whether they can be done at all is `is_enabled`, which is a different
            // question with a different colour.
            // The heading button lights for *any* level, because it stands for all six. Which one
            // is shown by its face rather than by the light — see `face_key`.
            toolbar::Action::Headings => {
                matches!(self.caret_block_kind(), Some(BlockKind::Heading(_)))
            }
            // Nothing about the caret makes an overflow list more or less applicable either.
            toolbar::Action::More
            | toolbar::Action::Rule
            | toolbar::Action::Link
            // Nor is MDEDIT-87's. An image is something you insert; there is no "inside an image"
            // the caret can be in that this button would light up for.
            | toolbar::Action::Image
            // "Inside a table" *is* a state, unlike the two above, and the button now answers it by
            // opening a different menu rather than by lighting: what it opens *is* the state. The
            // rows of that menu are actions on the table, and an action is not a mode to be in.
            | toolbar::Action::Table
            | toolbar::Action::RowAbove
            | toolbar::Action::RowBelow
            | toolbar::Action::ColumnLeft
            | toolbar::Action::ColumnRight
            | toolbar::Action::DeleteRow
            | toolbar::Action::DeleteColumn
            | toolbar::Action::DeleteTable
            | toolbar::Action::SelectRow
            | toolbar::Action::SelectTable
            | toolbar::Action::New
            | toolbar::Action::Open
            | toolbar::Action::Save
            | toolbar::Action::Undo
            | toolbar::Action::Redo
            | toolbar::Action::Cut
            | toolbar::Action::Copy
            | toolbar::Action::Paste => false,
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
        if self.words.is_none() || self.counted_for != self.active_document_id() {
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
            .is_none_or(|(tab, _)| Some(tab) != self.active_document_id())
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
        if let Some(label) = &self.status_labels[slot]
            && label.text == text
        {
            return label.width;
        }

        let run = StyledRun::from_buffer(text, style, ByteSpan::empty_at(ByteOffset::ZERO));
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
    /// Lays out a button's face once and keeps it: its icon if the font has one, its word if not.
    ///
    /// Returns the cache key, which is the button's **word** whether or not that is what got drawn.
    /// One key per action either way, so a caller asking twice gets the same entry and a machine
    /// without the icon font uses the same cache the icons would have.
    ///
    /// **The `.notdef` check is the fallback, and it has to be here rather than in a table.**
    /// `Action::icon` returns private-use code points, which are a promise about a font: `U+E8DD` is
    /// Bold in Segoe Fluent Icons and nothing at all in a font that has not got it. Asking the text
    /// layer and looking at what came back is the only way to know, and it is the same oracle
    /// MDEDIT-5's Han test uses. A machine with neither icon font draws the bar it had before this
    /// story rather than twenty-two empty boxes.
    /// What a button's face is keyed by: the label of the action it draws.
    fn face_key(&self, action: toolbar::Action) -> String {
        self.face_action(action).label()
    }

    /// The action whose face a button draws, which is its own except for the two that are stateful.
    ///
    /// The heading button stands for six levels and says which one the caret is in — `H2` inside a
    /// level-two heading, `H1` outside any. The list button (MDEDIT-144) stands for two kinds and
    /// draws the one the caret is in, the bullet outside any list. That makes their faces depend on
    /// the document, so it makes their *cache keys* depend on it too, which is fine: there are seven
    /// heading faces and two list faces, and each is laid out once.
    ///
    /// **An action, not a label**, since MDEDIT-144. The heading faces are letterforms, so swapping
    /// the word was enough; the list faces are icons, and an icon is found from an action. Resolving
    /// to `Bullet` or `Ordered` is what lets [`button_face`](Self::button_face) draw that action's
    /// icon, and its word on a machine without the font, with no case of its own.
    fn face_action(&self, action: toolbar::Action) -> toolbar::Action {
        use mdedit_document::format::Block;

        match action {
            toolbar::Action::Headings => match self.caret_block_kind() {
                Some(BlockKind::Heading(level)) => toolbar::Action::Heading(level),
                _ => action,
            },
            toolbar::Action::Lists if self.document().carries(Block::Ordered) => {
                toolbar::Action::Ordered
            }
            toolbar::Action::Lists => toolbar::Action::Bullet,
            _ => action,
        }
    }

    /// What kind of block the caret is in, counting the document's very end as inside the last one.
    ///
    /// **`Document::block_at` is half-open, and a caret can sit on the closing edge.** A block's span
    /// ends where the next one begins, so `offset < end` is the right rule for the renderer asking
    /// which block owns a byte — every byte has exactly one owner and nothing is drawn twice. A
    /// *caret* is not a byte: it sits between them, and the position after the last character of the
    /// last block is a legitimate place to be.
    ///
    /// It only bites at the end of the document, because everywhere else the block after it starts
    /// there and answers instead. A file ending `# Title` with no trailing newline therefore had a
    /// caret in no block at all — so the heading button read `H1` while the caret was in an `H3`, and
    /// it did not light. Adding a newline fixed it, which is what made it look like a parser bug
    /// rather than an off-by-one at the boundary.
    fn caret_block_kind(&self) -> Option<BlockKind> {
        let doc = self.document();
        let caret = doc.selection().head();
        if let Some(block) = doc.block_at(caret) {
            return Some(block.kind());
        }
        // Only the end of the document reaches here, and only when nothing follows the last block.
        if caret == ByteOffset::ZERO {
            return None;
        }
        doc.block_at(self.previous_boundary(caret))
            .map(mdedit_language::Block::kind)
    }

    fn button_face(&mut self, action: toolbar::Action) -> Option<String> {
        let action = self.face_action(action);
        let key = action.label();
        if self.toolbar_labels.contains_key(&key) {
            return Some(key);
        }

        let drawn = action.icon().and_then(|icon| {
            let style = TextStyle {
                family: mdedit_text::FontFamily::Icon,
                ..TextStyle::body(self.theme.body_size * TOOLBAR_ICON)
            };
            self.lay_out_label(&icon.to_string(), style)
                .filter(|label| !label.glyphs.iter().any(|g| g.key.glyph.is_notdef()))
        });

        let label = if let Some(label) = drawn {
            label
        } else {
            // A letterform that is *meant* to be one is set near the icon size; a word the missing
            // icon font left behind is set small, because "Paste" at that size would not fit.
            let ratio = if action.letters() {
                TOOLBAR_LETTERS
            } else {
                TOOLBAR_TEXT
            };
            let style = TextStyle::body(self.theme.body_size * ratio);
            self.lay_out_label(&key, style)?
        };
        self.toolbar_labels.insert(key.clone(), label);
        Some(key)
    }

    /// How far below the bar's top a button's face sits, so its ink is centred in the row.
    ///
    /// An icon and a word need different numbers because their ink sits differently in the em: an
    /// icon fills most of it, where a 10-pixel word is a short cap height near the baseline. The two
    /// are told apart by comparing the laid-out text against the cache key — a face that drew its
    /// icon holds one private-use character, and one that fell back holds the word the key *is*.
    /// That is why [`Label`] keeps its text; nothing else reads it.
    fn face_baseline(&self, label: &Label, key: &str, action: toolbar::Action) -> f32 {
        let drew_icon = label.text != key;
        let size = if drew_icon {
            self.theme.body_size * TOOLBAR_ICON * 0.41
        } else if action.letters() {
            // A deliberate letterform, set near the icon size, so it needs its own drop rather than
            // the small word's.
            self.theme.body_size * TOOLBAR_LETTERS * 0.35
        } else {
            self.theme.body_size * 0.3
        };
        toolbar::HEIGHT / 2.0 + size
    }

    /// How wide `text` is in `style`, without keeping the glyphs.
    ///
    /// For the dialog's caret and its centred button captions (MDEDIT-129). Measured rather than
    /// counted, because a proportional face makes a character count the wrong answer — the same
    /// reason the find field measures its own caret.
    fn measure_label(&mut self, text: &str, style: TextStyle) -> f32 {
        self.lay_out_label(text, style)
            .map_or(0.0, |label| label.width)
    }

    fn lay_out_label(&mut self, text: &str, style: TextStyle) -> Option<Label> {
        let run = StyledRun::from_buffer(text, style, ByteSpan::empty_at(ByteOffset::ZERO));
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
        self.counted_for = self.active_document_id();
    }

    /// Which document, at which revision, the diagnostics would describe if recomputed now.
    fn diagnosis_stamp(&self) -> Option<(u64, u64)> {
        Some((self.active_document_id()?, self.document().revision()))
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
        let run = [StyledRun::from_buffer(
            &text,
            style,
            ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(text.len())),
        )];
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
    /// A rendering mode, not a document mode: the buffer and the parse are untouched, and so is the
    /// caret's byte, with one exception.
    ///
    /// **Entering preview moves a caret out of a marker it is now inside** (MDEDIT-194). Plain
    /// markup lets the caret stand anywhere, but in preview a caret at byte 1 of `# Title` draws
    /// just before the `T`, and typing there wrote `#x Title` — a heading silently turned into a
    /// paragraph by a keystroke that looked safe. So both ends go past any concealed marker, the
    /// rule every motion keeps. See `leave_concealed`, which typing asks too.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            view::Mode::Preview => view::Mode::Plain,
            view::Mode::Plain => view::Mode::Preview,
        };
        self.leave_concealed();
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
            // The strip and the formatting bar sit above the document, so the two add.
            //
            // **The find bar is deliberately not a third** (MDEDIT-177). It floats over the text at
            // the top right, so opening it, closing it, or turning Replace on takes no height and
            // moves no line — which is the whole of that ticket, and the reason the term the code
            // used to add here is gone rather than zero.
            top: self.strip_height() + self.toolbar_height(),
            // The pane and the gutter both take from the left, and they add rather than compete —
            // which is exactly the arithmetic `ContentRect` exists to keep in one place.
            left: self.pane_width() + self.gutter_width(),
            bottom: status::BAR_HEIGHT,
            ..Insets::default()
        }
    }

    /// Height the formatting bar occupies, which is nothing only when it is switched off.
    ///
    /// It used to be nothing whenever the *language* had no buttons, which is how a JSON tab got no
    /// bar. MDEDIT-84's standard set removed that case: with the bar on there is always something in
    /// it, so the row is a fixed thing — which is what the tab strip below it needs, since a row
    /// that came and went would move the strip on every tab switch.
    fn toolbar_height(&self) -> f32 {
        if self.toolbar { toolbar::HEIGHT } else { 0.0 }
    }

    /// The buttons the bar shows: the standard set, then the visible document's language's.
    ///
    /// Keyed off the language **here**, in the app, rather than through `mdedit_language::Language`.
    /// Actions are UI vocabulary and the trait is parsing vocabulary; a parser has no business
    /// knowing what a toolbar is.
    ///
    /// Returns an owned list rather than a `&'static` slice, because the two halves are separate
    /// constants and Rust cannot concatenate slices in a const. It is called **once per frame**,
    /// from `draw_toolbar` alone — `toolbar_height` deliberately no longer asks, since the answer
    /// stopped depending on the language.
    fn toolbar_actions(&self) -> Vec<toolbar::Action> {
        if !self.toolbar {
            return Vec::new();
        }
        let language = match self.workspace.active_open() {
            // Markdown is the only language with actions of its own so far. JSON's Format and
            // Validate would be its set, and belong to whichever ticket gives it one; until then it
            // gets the standard buttons and no more, because a button that does nothing is worse
            // than no button.
            Some(open) if open.document.language().name() == "Markdown" => toolbar::MARKDOWN,
            // The scratch document, before anything is open, is Markdown too.
            None => toolbar::MARKDOWN,
            Some(_) => &[],
        };
        [toolbar::STANDARD, language].concat()
    }

    /// Whether a button can do anything right now, which decides both its colour and its clicks.
    ///
    /// Save with nothing to save, Undo with no history, Copy with no selection: all ordinary, and
    /// drawing them live and having them do nothing is the failure MDEDIT-52 named. The precedent is
    /// Tools → Format Document, which is greyed rather than hidden.
    ///
    /// **Paste is never dimmed, which is a deliberate departure from MDEDIT-84's plan.** Deciding
    /// needs the system clipboard, and there are only two ways to have that answer: read it as the
    /// frame is drawn, which means opening the platform clipboard sixty times a second — the thing
    /// [`crate::clipboard`]'s docs say not to do, and the API whose misuse MDEDIT-67 was — or cache
    /// it and refresh on some event, which means that when the cache is stale the button is dimmed
    /// while the clipboard has text, and a *wrong* dim loses the user a paste. A live Paste that
    /// inserts nothing is exactly what Ctrl+V already is, and nobody calls that broken.
    ///
    /// The language's own actions are always live: they act on the caret's line or the selection,
    /// and each does something sensible with either.
    #[must_use]
    pub fn is_enabled(&self, action: toolbar::Action) -> bool {
        match action {
            toolbar::Action::Save => self.is_modified(),
            toolbar::Action::Undo => self.document().can_undo(),
            toolbar::Action::Redo => self.document().can_redo(),
            toolbar::Action::Cut | toolbar::Action::Copy => {
                !self.document().selection().span().is_empty()
            }
            _ => true,
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
        self.strip = tabs::Strip::default();
        if self.chrome != Chrome::Strip || self.workspace.tabs().is_empty() {
            self.close_tab_menu_if(|_| true);
            return Vec::new();
        }

        let logical_width = width / scale;
        let label_style = TextStyle::body(13.0);

        let names: Vec<String> = self
            .workspace
            .tabs()
            .iter()
            .map(mdedit_session::Tab::display_name)
            .collect();
        // Measured only when `overflowing` asks, which is for about as many tabs as fit: with two
        // hundred open, shaping every label to draw eight would be the same mistake the document
        // renderer makes, and there is no reason to repeat it here.
        let text = &mut self.text;
        self.strip = tabs::overflowing(
            names.len(),
            self.workspace.active(),
            logical_width,
            |index| {
                let run = StyledRun::from_buffer(
                    &names[index],
                    label_style,
                    ByteSpan::empty_at(ByteOffset::ZERO),
                );
                text.layout(&[run], f32::MAX).map_or(60.0, |layout| {
                    layout
                        .glyphs()
                        .map(|glyph| glyph.pos.x + glyph.advance)
                        .fold(0.0_f32, f32::max)
                })
            },
        );
        // A list of hidden tabs that no longer matches what is hidden — the window was widened, a
        // tab closed, another became active — is offering the wrong files. The toolbar's overflow
        // closes for the same reason, and closing is the only honest answer here too.
        let hidden = self.strip.hidden.clone();
        self.close_tab_menu_if(|listed| listed != hidden.as_slice());

        // MDEDIT-84: the formatting bar is above the strip now, so the strip starts where the bar
        // ends. `tabs.rs` is untouched by that — it lays out within a row whose origin the caller
        // supplies, which is exactly so this is the only place that has to know.
        let top = self.toolbar_height();
        let mut quads = vec![Self::solid(
            [0.0, top * scale, width, tabs::STRIP_HEIGHT * scale],
            self.theme.strip,
        )];

        for rect in self.strip.tabs.clone() {
            self.draw_tab(&mut quads, rect, names.get(rect.index), label_style, scale);
        }
        if let Some(more) = self.strip.more {
            self.draw_strip_more(&mut quads, more, label_style, scale);
        }

        quads
    }

    /// Draws the strip's overflow button (MDEDIT-179): the formatting bar's `More` face, lit while
    /// its list is open, as a lit button on the bar is.
    fn draw_strip_more(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        more: f32,
        label_style: TextStyle,
        scale: f32,
    ) {
        let top = self.toolbar_height();
        if self.menu_kind() == Some(toolbar::Menu::Tabs) {
            quads.push(Self::solid(
                [
                    more * scale,
                    top * scale,
                    tabs::MORE_WIDTH * scale,
                    tabs::STRIP_HEIGHT * scale,
                ],
                self.theme.tab_active,
            ));
        }
        let (face, style) = self.icon_face(
            toolbar::Action::More.icon().unwrap_or('\u{E712}'),
            &toolbar::Action::More.label(),
            label_style,
        );
        let width = self
            .lay_out_label(&face, style)
            .map_or(tabs::MORE_WIDTH, |label| label.width);
        self.draw_label(
            quads,
            &face,
            style,
            more + (tabs::MORE_WIDTH - width) / 2.0,
            top + tabs::STRIP_HEIGHT * 0.68,
            self.theme.text,
            scale,
            tabs::MORE_WIDTH,
        );
    }

    /// Draws one tab: its ground, its close affordance and its name.
    ///
    /// Split out of [`build_tab_strip`](Self::build_tab_strip) only because that function reached a
    /// hundred lines, which is a real limit here. The seam is the natural one — everything below is
    /// about one tab and reads nothing about the strip except where the strip begins.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_tab(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        rect: tabs::TabRect,
        name: Option<&String>,
        label_style: TextStyle,
        scale: f32,
    ) {
        let top = self.toolbar_height();
        let is_active = self.workspace.active() == Some(rect.index);
        let background = if is_active {
            self.theme.tab_active
        } else {
            self.theme.tab_inactive
        };
        // A one-pixel gap on the right separates neighbours without a border pass.
        quads.push(Self::solid(
            [
                rect.x * scale,
                top * scale,
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
            quads,
            glyph,
            label_style,
            close_x + close_width / 2.0 - 4.0,
            // Placed so the label sits centred in the strip.
            top + tabs::STRIP_HEIGHT * 0.68,
            colour,
            scale,
            close_width,
        );

        let ink = if is_active {
            self.theme.text
        } else {
            self.theme.marker
        };

        // The same icon the tab's pane row draws (MDEDIT-178), from the same answer and through the
        // same fallback, so the two presentations cannot disagree about what a file is.
        let icon = self.icon_for(rect.index);
        let (face, face_style) = self.row_icon_face(icon, label_style);
        self.draw_label(
            quads,
            &face,
            face_style,
            rect.icon_x(),
            top + tabs::STRIP_HEIGHT * 0.68,
            ink,
            scale,
            tabs::ICON_WIDTH,
        );

        if let Some(name) = name {
            self.draw_label(
                quads,
                name,
                label_style,
                rect.label_x(),
                top + tabs::STRIP_HEIGHT * 0.68,
                ink,
                scale,
                rect.label_width(),
            );
        }
    }

    /// Draws one short string of chrome, clipped to `max_width`.
    ///
    /// `baseline` is in window pixels, so the same call serves the strip at the top and the status
    /// bar at the bottom.
    ///
    /// It used to **return whether anything was clipped off**, because MDEDIT-121's rule was that
    /// only a truncated tab or row got a tooltip, and whether a name fitted is knowable only here —
    /// it depends on the width the strip gave that tab, and the glyphs are laid out by this point.
    /// MDEDIT-173 retired that rule: every tab with a file behind it shows its path, so nothing
    /// asked any more and the answer went with the two lists that carried it.
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
        let run = StyledRun::from_buffer(text, style, ByteSpan::empty_at(ByteOffset::ZERO));
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

            // The icon before the name, in the column `label_x` leaves for it (MDEDIT-174). Drawn in
            // the row's own colour: it says what the file is, not how it is doing.
            let icon = self.icon_for(rect.index);
            let (face, face_style) = self.row_icon_face(icon, style);
            self.draw_row_label(
                quads,
                &face,
                face_style,
                rect.icon_x(),
                baseline,
                if is_active {
                    self.theme.text
                } else {
                    self.theme.marker
                },
                scale,
                pane::ICON_WIDTH,
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

    /// Which icon a tab's file gets, in the pane (MDEDIT-174) and the strip (MDEDIT-178).
    ///
    /// **Asked of the path, not of the document.** A tab is a lightweight handle until it is shown,
    /// so most rows have no document to ask — and `detect::language_for` is the same answer the
    /// highlighting uses, which is what stops an icon disagreeing with the colours in the file it
    /// names. A buffer with no path has no extension either, so it comes out plain text's.
    fn icon_for(&self, index: usize) -> file_icon::Icon {
        self.workspace.tabs()[index]
            .path()
            .map_or(file_icon::Icon::Text, |path| {
                file_icon::Icon::of(crate::detect::language_for(path))
            })
    }

    /// The glyph to draw for an icon, or its badge on a machine whose fonts have not got it.
    ///
    /// The same `.notdef` check the formatting bar makes (MDEDIT-92), and for the same reason: a
    /// tofu box in a column of file names is a bug report, where two letters are merely plain.
    fn row_icon_face(&mut self, icon: file_icon::Icon, text: TextStyle) -> (String, TextStyle) {
        self.icon_face(icon.glyph(), icon.badge(), text)
    }

    /// `glyph` in the icon font, or `fallback` in `text` on a machine that has not got it.
    ///
    /// Asked of the font once per code point and remembered, because chrome is redrawn every frame
    /// and the answer cannot change with the size it is drawn at.
    fn icon_face(&mut self, glyph: char, fallback: &str, text: TextStyle) -> (String, TextStyle) {
        let style = TextStyle {
            family: FontFamily::Icon,
            ..text
        };
        let face = glyph.to_string();
        let drawn = if let Some(known) = self.icon_faces.get(&glyph) {
            *known
        } else {
            let resolved = self
                .lay_out_label(&face, style)
                .is_some_and(|label| !label.glyphs.iter().any(|g| g.key.glyph.is_notdef()));
            self.icon_faces.insert(glyph, resolved);
            resolved
        };
        if drawn {
            (face, style)
        } else {
            (fallback.to_owned(), text)
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
            let run = StyledRun::from_buffer(text, style, ByteSpan::empty_at(ByteOffset::ZERO));
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
                // than a row is wide, and an ellipsis needs its own measurement pass. Whether it
                // happened was the answer MDEDIT-121's tooltip wanted; MDEDIT-173 retired that rule,
                // so nothing asks and nothing is returned.
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
        self.flagged_digits = None;
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
            appearance: match self.config.theme {
                view::ThemeChoice::System => mdedit_menu_win::AppearanceChoice::System,
                view::ThemeChoice::Dark => mdedit_menu_win::AppearanceChoice::Dark,
                view::ThemeChoice::Light => mdedit_menu_win::AppearanceChoice::Light,
            },
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

    /// What shape the pointer takes over a point in the window (MDEDIT-176).
    ///
    /// **The order is [`click`](Self::click)'s**, and for the same reason: a panel that takes the
    /// click takes the shape. Two copies of that order is how the pointer would come to say one
    /// thing while a click at the same point did another.
    ///
    /// **Only the panels that overlay the document need an arm here**, which is fewer than `click`
    /// has to ask. The formatting bar, the tab strip, the file pane, the status bar and the gutter
    /// all sit *outside* [`ContentRect`], so the arrow they want is the arrow every point outside
    /// the content already gets; an arm apiece would be branches no test could tell from their
    /// absence. **The scrollbar is outside it too**, which its own doc comment does not say: it
    /// overlays the *window's* right margin rather than the text, so a point on the bar is already
    /// outside the content. What is left is the dialog, the open list and the find panel — all
    /// three drawn over the text, and the last of them with fields that want the I-beam.
    ///
    /// The find panel joined that list on MDEDIT-177 and had not needed an arm before it: while the
    /// bar was a band above the document it was outside [`ContentRect`] like the rest of the chrome,
    /// and the arrow fell out of the same `contains` check. Floating is what makes it a question.
    ///
    /// A gesture in progress outranks the pixel underneath: a selection being dragged keeps the
    /// I-beam wherever the pointer goes, and a held scrollbar thumb keeps the arrow, because what
    /// the pointer is doing is no longer a question about what it is over.
    #[must_use]
    pub fn pointer_at(&self, x: f32, y: f32) -> Pointer {
        if self.thumb_grab.is_some() {
            return Pointer::Arrow;
        }
        if self.dragging {
            return Pointer::Text;
        }

        // **A dialog is modal, so every point in the window is the dialog's** — inside it a click
        // acts or is swallowed, and outside it cancels (MDEDIT-129). Neither reaches the document,
        // so nothing outside its fields is worth pointing at.
        if let Some(open) = self.dialog.as_ref() {
            let fields = open.fields().len();
            let at = self.dialog_at();
            return match dialog::hit_test(fields, x, y, at) {
                Some(dialog::Hit::Field(_)) => Pointer::Text,
                _ => Pointer::Arrow,
            };
        }
        // **An open list takes every point, not just its own panel.** `click_menu` swallows a click
        // anywhere while one is open — on a row it acts, off the panel it closes — so nothing
        // reaches the document and nothing under it is worth pointing at. Asking `popup::contains`
        // instead looked right and was wrong twice: it let the pointer promise an I-beam a click
        // would not honour, and it measured a panel of `rows.len()` rows, which is **zero** for the
        // table picker, whose rows are its geometry rather than a list of actions.
        if self.open_menu.is_some() {
            return Pointer::Arrow;
        }
        if let Some(layout) = self.find_layout.as_ref()
            && layout.contains(x, y)
        {
            return match layout.over_field(x, y) {
                Some(_) => Pointer::Text,
                None => Pointer::Arrow,
            };
        }
        // Everything else the chrome owns is outside the content: the bar and the strip above it,
        // the pane beside it, the status bar below, and the gutter — which is left of the text and
        // selects nothing, so it is not an I-beam and not the reversed arrow that would promise a
        // line selection this editor does not do.
        if !self.content.contains(x, y) {
            return Pointer::Arrow;
        }
        // The one thing in a document that is pressed rather than typed into (MDEDIT-142), found
        // with the same two lookups the press itself uses so the two cannot disagree.
        if self.task_box_at_point(x, y).is_some() {
            return Pointer::Hand;
        }
        Pointer::Text
    }

    /// Notes where the pointer is and gives it its shape. Returns whether the shape changed.
    ///
    /// Public so a test can move the pointer without a window, as [`click_at`](Self::click_at) is:
    /// the window's own handler does this and then [`on_pointer_moved`](Self::on_pointer_moved),
    /// which is the half that drags and hovers.
    pub fn move_pointer(&mut self, x: f32, y: f32) -> bool {
        self.cursor = (x, y);
        self.refresh_pointer()
    }

    /// What shape the pointer is wearing.
    #[must_use]
    pub fn pointer(&self) -> Pointer {
        self.pointer
    }

    /// Tells the window what shape to draw, when that has changed since the last time it was told.
    ///
    /// Called from every pointer move and from every frame: scrolling, opening a panel, switching
    /// tab or toggling plain markup all change what is under a pointer that has not moved, and a
    /// pointer that stays an I-beam over a toolbar that has just appeared under it is the tell.
    ///
    /// **Compared before it is set**, so a pointer crossing one region costs a comparison per event
    /// rather than a call into the window per event. Returns whether the window was told.
    fn refresh_pointer(&mut self) -> bool {
        let wanted = self.pointer_at(self.cursor.0, self.cursor.1);
        if self.pointer == wanted {
            return false;
        }
        self.pointer = wanted;
        if let Some(window) = self.window.as_ref() {
            window.set_cursor(match wanted {
                Pointer::Arrow => CursorIcon::Default,
                Pointer::Text => CursorIcon::Text,
                Pointer::Hand => CursorIcon::Pointer,
            });
        }
        true
    }

    /// A middle-click anywhere in the window: it closes the tab under the pointer, as every editor
    /// with tabs does, and does nothing anywhere else.
    ///
    /// **Here rather than in the event handler so that a test can reach it** (MDEDIT-181). It used
    /// to be a `match` arm on a winit `MouseButton`, which no test can construct, and deleting that
    /// arm outright failed nothing: `cargo-mutants` found it on #173. The left-click path was
    /// already split out for the same reason, which is what [`click`](Self::click) says.
    ///
    /// The strip and the pane are asked in that order for the reason `click` gives — only one of the
    /// two is ever showing. The overflow button (MDEDIT-179) closes nothing: it is not a tab, and a
    /// middle-click on it should no more close something than one on the empty strip beside it.
    ///
    /// **A dialog or an open list stops it, which `click`'s own order says and this used to skip.**
    /// A modal swallows a click even where it has no control (MDEDIT-129), and closing a tab under
    /// one is worse than letting a caret through: `close_tab_at` activates another document, and the
    /// dialog then acts on a file the user never opened it against. Nothing is dismissed here,
    /// though — a middle-click is not how either is cancelled, and inventing that is not this
    /// method's business.
    pub fn middle_click(&mut self, x: f32, y: f32) -> bool {
        if self.dialog.is_some() || self.open_menu.is_some() {
            return false;
        }
        match (self.strip_hit(x, y), self.pane_hit(x, y)) {
            (Some(tabs::Hit::Select(index) | tabs::Hit::Close(index)), _)
            | (_, Some(pane::Hit::Select(index) | pane::Hit::Close(index))) => {
                // The pointer was on chrome, so the click run ends — a tab closed between two
                // clicks in the document must not make them a double-click, which is the rule
                // `click` follows for every piece of chrome it hits.
                self.clicks.reset();
                self.close_tab_at(index)
            }
            (Some(tabs::Hit::More) | None, None) => false,
        }
    }

    /// A left-click anywhere in the window.
    ///
    /// Chrome first, because both presentations sit over an edge of the document; only one of the
    /// two is ever showing, so at most one can match. The order lives here rather than in the event
    /// handler so that a test clicks the same way the window does — a test that reached past this
    /// into [`click_at`](Self::click_at) would be exercising a dispatch the app does not use.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        // Before the chain, because every click dismisses a tooltip — including the one on the
        // button the tooltip is describing. The button still acts; the panel goes.
        //
        // Note what is *not* here: the tooltip is never hit-tested. It is drawn over the document
        // and asking it about a click would make the text under it unclickable, which is how "it is
        // only a tooltip" ends in a control nobody can press.
        let dismissed = self.dismiss_tooltip();

        // **The dialog is above everything, and swallows a click even where it has no control**
        // (MDEDIT-129). That last part is what makes it modal rather than merely topmost: a click on
        // its own background reaching the document would move the caret out from under the text the
        // dialog is about to replace. A click *outside* it falls through deliberately — dismissing
        // on an outside click is what every other panel here does, and a dialog that trapped the
        // pointer as well as the keyboard would need a reason.
        if let Some(acted) = self.click_dialog(x, y) {
            return acted || dismissed;
        }

        // **An open list is above everything else**, and so is asked next. This function's order is
        // the whole of the z-order, in both directions: drawn last, asked first.
        if let Some(acted) = self.click_menu(x, y) {
            return acted || dismissed;
        }
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
        // **And nothing below the content is a click in the document.** The status bar is the only
        // thing down there, and the press fell through to `click_at`, whose `offset_at_point` clamps
        // an out-of-band y to the last chunk: clicking the word count moved the caret to the end of
        // the document and started a drag from there (MDEDIT-109). See `ContentRect::is_below` for
        // why this is not `contains`.
        if self.content.is_below(y) {
            self.clicks.reset();
            return dismissed;
        }
        // Clicking into the text is how focus comes back without closing the bar, which is what
        // makes "find it, then edit it there" one gesture rather than two.
        self.find_focused = false;
        self.click_at(x, y) || dismissed
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
        // **The find panel first, because it is the one piece of chrome inside the content rect**
        // (MDEDIT-177). While the bar was a band above the document, `contains` below declined a
        // right-click on it for free; a panel that floats is inside the rect, so without this a
        // right-click on the search field placed the caret in the text *underneath* the panel,
        // collapsed the current match's selection and popped the document's menu over the panel.
        if self
            .find_layout
            .as_ref()
            .is_some_and(|layout| layout.contains(x, y))
        {
            return false;
        }
        // Chrome has no context menu of its own yet, and a document menu popping up over the tab
        // strip would be worse than nothing. Deliberately not the same as the left-button chain:
        // there is nothing here for the scrollbar or the strip to do.
        if !self.content.contains(x, y) {
            return false;
        }
        let Some(offset) = self.offset_only_at_point(x, y) else {
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
            // A drag is not a rest. Both early exits here are also the answer to "should a tooltip
            // appear while the pointer is held down", which is no.
            return;
        }

        if self.hover(point.0, point.1)
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }

    /// Notes where the pointer is, and restarts the dwell.
    ///
    /// Returns whether anything visible changed — which is only ever a tooltip being taken *away*,
    /// since one appearing is [`on_dwell`](Self::on_dwell)'s job a few hundred milliseconds later.
    ///
    /// **The dwell restarts per button, not per pixel.** Moving inside the button you are already on
    /// leaves the deadline where it is, so resting still counts as resting while a hand shakes;
    /// moving to a different button, or off the bar, starts it again. A dwell that restarted on
    /// every `CursorMoved` would never elapse for anybody without a very steady hand.
    ///
    /// Public because a test cannot deliver a `CursorMoved`, and because the dwell it schedules is
    /// what `next_wake` is then asked about.
    pub fn hover(&mut self, x: f32, y: f32) -> bool {
        // **An open list takes the pointer, and takes it immediately.** No dwell: a menu highlights
        // the row under the pointer at once, because the highlight *is* the feedback that the row is
        // a target — where a tooltip is extra information about a button you can already see. The
        // list is also above everything, so nothing behind it can be hovered while it is open, which
        // is the same rule `click` follows one function along.
        if let Some(menu) = self.open_menu.as_ref() {
            let moved = if menu.kind == toolbar::Menu::TableSize {
                let cell = popup::grid::hit_test(x, y, menu.at);
                let moved = cell != self.hovered_cell;
                self.hovered_cell = cell;
                moved
            } else {
                let row = popup::hit_test(menu.len(), x, y, menu.at, menu.width);
                let moved = row != self.hovered_row;
                self.hovered_row = row;
                moved
            };
            self.hovered = None;
            self.dwell_at = None;
            return moved | self.tooltip.take().is_some();
        }
        self.hovered_row = None;
        self.hovered_cell = None;

        let over = self.hoverable(x, y);

        if over == self.hovered {
            return false;
        }
        self.hovered = over;
        self.dwell_at = over.map(|_| Instant::now() + tooltip::DWELL);
        // Whatever was showing belonged to the button we have just left.
        self.tooltip.take().is_some()
    }

    /// What is under `(x, y)` that has something to say, if anything.
    ///
    /// **A tab or a row counts when it has a file behind it** — see [`has_path`](Self::has_path),
    /// which is where the reasoning for that lives and what it replaced (MDEDIT-173). This is still
    /// not simply the hit test the click path uses, because two things under the pointer are
    /// deliberately silent: the overflow button, and a tab with nothing saved.
    fn hoverable(&self, x: f32, y: f32) -> Option<Hovered> {
        if !self.toolbar_buttons.is_empty() && (0.0..toolbar::HEIGHT).contains(&y) {
            return toolbar::hit_test(&self.toolbar_buttons, x, y).map(Hovered::Button);
        }
        if let Some(hit) = self.strip_hit(x, y) {
            // The overflow button explains itself by being pressed; a tooltip naming it would
            // cover the list it is about to open.
            let (tabs::Hit::Select(index) | tabs::Hit::Close(index)) = hit else {
                return None;
            };
            return self.has_path(index).then_some(Hovered::Tab(index));
        }
        if let Some(pane::Hit::Select(index) | pane::Hit::Close(index)) = self.pane_hit(x, y) {
            return self.has_path(index).then_some(Hovered::Row(index));
        }
        None
    }

    /// Whether tab `index` has a file behind it, which is what it takes to have a path to show.
    ///
    /// **This replaced MDEDIT-121's rule, and the replacement is the ticket** (MDEDIT-173). A tab or
    /// row used to be hoverable only when its name had not fitted, on the ground that a tooltip
    /// repeating what is already on screen is noise. A *path* is not a repeat — it adds where the
    /// file is, and it subsumes the truncated-name case because a path ends in the name. So the
    /// measurement of what fitted stopped being consulted here, and the two lists that carried it
    /// from the last frame went with it.
    ///
    /// An untitled buffer has no path and gets nothing. The alternative was a panel saying it has not
    /// been saved, which tells the reader what the tab already says — `Untitled 1` — and is the kind
    /// of noise the rule above was written against.
    fn has_path(&self, index: usize) -> bool {
        self.workspace
            .tabs()
            .get(index)
            .is_some_and(|tab| tab.path().is_some())
    }

    /// Which row of the open list the pointer is over, if any.
    #[must_use]
    pub fn hovered_row(&self) -> Option<usize> {
        self.hovered_row
    }

    /// Which cell of the open size picker the pointer is over, as a zero-based `(row, column)`.
    #[must_use]
    pub fn hovered_cell(&self) -> Option<(usize, usize)> {
        self.hovered_cell
    }

    /// Which panel is open, if one is.
    #[must_use]
    pub fn menu_kind(&self) -> Option<toolbar::Menu> {
        self.open_menu.as_ref().map(|menu| menu.kind)
    }

    /// Where the open list's top-left corner is, for a test.
    #[must_use]
    pub fn menu_at(&self) -> Option<(f32, f32)> {
        self.open_menu.as_ref().map(|menu| menu.at)
    }

    /// How wide the open list is, for a test.
    #[must_use]
    pub fn menu_width(&self) -> Option<f32> {
        self.open_menu.as_ref().map(|menu| menu.width)
    }

    /// The pointer has now rested long enough. Shows the tooltip, if there is still a button under
    /// it.
    ///
    /// Public for the same reason [`on_idle`](Self::on_idle) is: a test cannot deliver a
    /// `StartCause::ResumeTimeReached`, and sleeping for the dwell in a unit test would be four
    /// hundred milliseconds nobody gets back.
    pub fn on_dwell(&mut self) -> bool {
        let Some(over) = self.hovered else {
            return false;
        };
        if self.tooltip == Some(over) {
            return false;
        }
        self.tooltip = Some(over);
        true
    }

    /// What the showing tooltip belongs to, if one is showing.
    #[must_use]
    pub fn tooltip(&self) -> Option<Hovered> {
        self.tooltip
    }

    /// The line the showing tooltip reads, if one is showing.
    ///
    /// A button's is its own description; **a tab's or a row's is its file's path, not its name**
    /// (MDEDIT-173), so two open files called `README.md` in different folders are told apart
    /// without opening either.
    ///
    /// The *whole* path, whatever the window's width. What is drawn may be shorter — a deep path has
    /// its middle elided to fit, see [`tooltip::elisions`] — and that is the drawing's business
    /// rather than this answer's.
    ///
    /// **Reported as stored, with nothing asked of the filesystem.** A tab restored from a session
    /// whose file has since moved or been deleted still shows the path it was opened from, which is
    /// the useful fact when the file is missing; and a hover is the last place that should be
    /// touching the disk.
    #[must_use]
    pub fn tooltip_text(&self) -> Option<String> {
        match self.tooltip? {
            Hovered::Button(action) => Some(action.tooltip()),
            Hovered::Tab(index) | Hovered::Row(index) => self
                .workspace
                .tabs()
                .get(index)
                .and_then(mdedit_session::Tab::path)
                .map(|path| path.display().to_string()),
        }
    }

    /// The rows of the list that is open, if one is.
    #[must_use]
    pub fn menu_rows(&self) -> Option<&[toolbar::Action]> {
        self.open_menu.as_ref().map(|menu| menu.rows.as_slice())
    }

    /// The tabs the open list offers, if it is the strip's list of hidden tabs (MDEDIT-179).
    #[must_use]
    pub fn menu_tabs(&self) -> Option<&[usize]> {
        self.open_menu
            .as_ref()
            .filter(|menu| menu.kind == toolbar::Menu::Tabs)
            .map(|menu| menu.tabs.as_slice())
    }

    /// The strip as last drawn: its tabs, what did not fit, and where the overflow button is.
    #[must_use]
    pub fn strip(&self) -> &tabs::Strip {
        &self.strip
    }

    /// Which menu a button opens, given where the caret is (MDEDIT-128).
    ///
    /// [`toolbar::Action::opens`] answers the structural half — *this button opens a panel* — and
    /// cannot answer this half, because an `Action` is a button rather than a place in a document.
    /// One button's answer depends on both: **inside a table, Table opens the edit list instead of
    /// the size picker.** GFM has no nested table, so the picker has nothing to offer there.
    ///
    /// This is why a group of nine controls needed no room on a bar with room for one.
    fn menu_for(&self, action: toolbar::Action) -> Option<toolbar::Menu> {
        match action.opens()? {
            toolbar::Menu::TableSize if self.document().caret_cell().is_some() => {
                Some(toolbar::Menu::TableEdit)
            }
            kind => Some(kind),
        }
    }

    /// Selects the caret's table row, from its first cell to its last (MDEDIT-128).
    ///
    /// The row's *cells*, not its source line: the pipes are syntax the grid draws for you, and a
    /// selection that took them in would put the table's scaffolding on the clipboard.
    fn select_table_row(&mut self) -> bool {
        let Some((row, _)) = self.document().caret_cell() else {
            return false;
        };
        let head = self.document().selection().head();
        let Some(table) = self.document().table_at(head) else {
            return false;
        };
        let Some(span) = table.row_span(row) else {
            return false;
        };
        self.document_mut()
            .set_selection(Selection::new(span.start(), span.end()));
        true
    }

    /// Selects the whole table, its trailing newline excluded.
    fn select_whole_table(&mut self) -> bool {
        let head = self.document().selection().head();
        let Some(table) = self.document().table_at(head) else {
            return false;
        };
        self.document_mut()
            .set_selection(Selection::new(table.span.start(), table.span.end()));
        true
    }

    /// Opens the list `button` stands for, or closes it if it is already that one.
    ///
    /// **Clicking an open menu's own button closes it**, which is what every menu does and what
    /// stops the button being a one-way door.
    fn toggle_menu(&mut self, button: toolbar::ButtonRect, kind: toolbar::Menu) -> bool {
        let rows: Vec<toolbar::Action> = match kind {
            toolbar::Menu::Headings => toolbar::HEADINGS.to_vec(),
            toolbar::Menu::Lists => toolbar::LISTS.to_vec(),
            // **Whatever the bar dropped, taken from the row it drew** rather than recomputed. The
            // two are one partition and deriving them twice is how an action ends up in neither.
            toolbar::Menu::Overflow => self.dropped.clone(),
            toolbar::Menu::TableEdit => toolbar::TABLE_EDIT.to_vec(),
            // MDEDIT-127's grid has no rows of actions — its contents are its geometry — so it is
            // never empty and the check below does not apply to it. The strip's list of hidden tabs
            // (MDEDIT-179) is opened by `toggle_tab_menu`, never by a button on this bar, and an
            // empty list is refused just below, so a bar that somehow asked opens nothing.
            toolbar::Menu::TableSize | toolbar::Menu::Tabs => Vec::new(),
        };
        if kind != toolbar::Menu::TableSize && rows.is_empty() {
            return false;
        }
        if self.open_menu.is_some() {
            return self.close_menu();
        }

        let width = if kind == toolbar::Menu::TableSize {
            popup::grid::width()
        } else {
            self.list_width(&rows)
        };
        let at = popup::place(width, button.x, toolbar::HEIGHT, self.window_width);
        self.open_menu = Some(OpenMenu {
            kind,
            rows,
            tabs: Vec::new(),
            at,
            width,
        });
        // A menu and a tooltip about the same button at once is two panels saying different things.
        self.tooltip = None;
        true
    }

    /// Opens the list of tabs the strip had no room for, or closes it (MDEDIT-179).
    ///
    /// **The rows are the strip's `hidden`, taken from the layout it drew** rather than recomputed —
    /// the toolbar's rule for its own overflow, and for the same reason: what is drawn and what is
    /// listed are one partition, and deriving them twice is how a tab ends up in neither.
    fn toggle_tab_menu(&mut self) -> bool {
        let (Some(more), false) = (self.strip.more, self.strip.hidden.is_empty()) else {
            return false;
        };
        if self.open_menu.is_some() {
            return self.close_menu();
        }

        let tabs = self.strip.hidden.clone();
        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        let mut widest = 0.0_f32;
        for &index in &tabs {
            let name = self.workspace.tabs()[index].display_name();
            if let Some(label) = self.lay_out_label(&name, style) {
                widest = widest.max(label.width);
            }
        }
        // Never wider than the window: a name that long is clipped in its row, as it is on a tab,
        // and MDEDIT-121's tooltip is what shows the rest of it there.
        let width = popup::width_of(widest).min(self.window_width);
        let at = popup::place(
            width,
            more,
            self.toolbar_height() + tabs::STRIP_HEIGHT,
            self.window_width,
        );
        self.open_menu = Some(OpenMenu {
            kind: toolbar::Menu::Tabs,
            rows: Vec::new(),
            tabs,
            at,
            width,
        });
        self.tooltip = None;
        true
    }

    /// Closes the list of hidden tabs if it is open and `stale` says what it lists is out of date.
    fn close_tab_menu_if(&mut self, stale: impl FnOnce(&[usize]) -> bool) {
        if self
            .open_menu
            .as_ref()
            .is_some_and(|open| open.kind == toolbar::Menu::Tabs && stale(&open.tabs))
        {
            self.close_menu();
        }
    }

    /// How wide a list of rows has to be, laying out any words it needs on the way.
    fn list_width(&mut self, rows: &[toolbar::Action]) -> f32 {
        let style = TextStyle::body(self.theme.body_size * TOOLBAR_TEXT);
        let mut widest = 0.0_f32;
        for action in rows {
            let Some(text) = popup::text_for(*action) else {
                continue;
            };
            if !self.toolbar_labels.contains_key(&text)
                && let Some(label) = self.lay_out_label(&text, style)
            {
                self.toolbar_labels.insert(text.clone(), label);
            }
            if let Some(label) = self.toolbar_labels.get(&text) {
                widest = widest.max(label.width);
            }
        }

        // A list of nothing but letterforms is the icon column and its padding, and no more.
        if popup::any_text(rows) {
            popup::width_of(widest)
        } else {
            popup::ICON_COLUMN
        }
    }

    /// Closes whatever panel is open.
    fn close_menu(&mut self) -> bool {
        self.hovered_row = None;
        self.hovered_cell = None;
        self.open_menu.take().is_some()
    }

    /// Resolves a click while the dialog is up.
    ///
    /// `None` when there is no dialog, or when the click was outside it — an outside click closes
    /// it and is **not** swallowed, which is what every other panel here does. Inside it, everything
    /// is the dialog's, including a click on its own background: that is what makes it modal, and
    /// letting one through would move the caret out from under the text it is about to replace.
    fn click_dialog(&mut self, x: f32, y: f32) -> Option<bool> {
        let open = self.dialog.as_ref()?;
        let fields = open.fields().len();
        let at = self.dialog_at();

        if !dialog::contains(fields, x, y, at) {
            self.cancel_dialog();
            return Some(true);
        }

        match dialog::hit_test(fields, x, y, at) {
            Some(dialog::Hit::Field(index)) => {
                self.dialog.as_mut()?.focus(index);
                Some(true)
            }
            Some(dialog::Hit::Accept) => Some(self.accept_dialog()),
            Some(dialog::Hit::Cancel) => Some(self.cancel_dialog()),
            // Inside the panel but on nothing. Swallowed, and nothing changed.
            None => Some(false),
        }
    }

    /// Where the dialog's top-left corner is, in the window the last frame measured.
    fn dialog_at(&self) -> (f32, f32) {
        let fields = self.dialog.as_ref().map_or(0, |open| open.fields().len());
        dialog::place(
            self.window_width,
            self.window_height,
            dialog::height_of(fields),
        )
    }

    /// Resolves a click while a list is open.
    ///
    /// **Asked before everything else in [`click`](Self::click)**, because it is drawn after
    /// everything else, and `click`'s order is the whole of the z-order in both directions.
    ///
    /// Returns whether the click was the menu's. A click **anywhere** while a menu is open belongs
    /// to the menu: on a row it acts, off the panel it closes and is swallowed. That is what every
    /// menu does, and the alternative — a click that both dismisses and lands — is how somebody
    /// closing a menu accidentally moves their caret.
    fn click_menu(&mut self, x: f32, y: f32) -> Option<bool> {
        let menu = self.open_menu.clone()?;
        if menu.kind == toolbar::Menu::TableSize {
            let cell = popup::grid::hit_test(x, y, menu.at);
            self.close_menu();
            // The cell is zero-based and a table of no rows is not a table: the top-left square is
            // one row by one column, which is a header and nothing under it.
            return Some(match cell {
                Some((row, column)) => self.document_mut().insert_table(row + 1, column + 1),
                None => true,
            });
        }
        let hit = popup::hit_test(menu.len(), x, y, menu.at, menu.width);
        self.close_menu();
        if menu.kind == toolbar::Menu::Tabs {
            // Choosing a hidden tab shows it, and the next frame's strip moves to include it —
            // `overflowing` keeps the active tab in view, which is what makes the list a way *to*
            // the tab rather than a second, separate place to switch from.
            return Some(match hit.and_then(|row| menu.tabs.get(row).copied()) {
                Some(index) => match self.workspace.activate(index) {
                    Ok(_) => true,
                    Err(error) => {
                        crate::log_error!("could not open tab: {error}");
                        true
                    }
                },
                None => true,
            });
        }
        match hit.and_then(|row| menu.rows.get(row).copied()) {
            // **A displaced dropdown opens its own panel** (MDEDIT-193). The overflow list holds the
            // buttons the bar dropped, verbatim, and some of those open a panel rather than edit
            // anything — so sending every row to `perform` made Headings and Table dead rows, the
            // list closing on a click that did nothing. Asked through `menu_for`, as the bar asks, so
            // Table still opens the edit list inside a table and the next button to open a panel is
            // covered without anyone remembering it here.
            Some(action) => Some(match self.menu_for(action) {
                Some(kind) => self.open_in_place_of_overflow(kind),
                None => self.perform(action),
            }),
            None => Some(true),
        }
    }

    /// Opens `kind` where the overflow list just was, under the `More` button that list hung from.
    ///
    /// **In place of it, not beside it**: the overflow list is already closed by the time this runs,
    /// so there is still one panel open at a time, which is the rule every other part of the popup
    /// code — drawing, hit testing, the pointer — is written against. A nested submenu would have
    /// been a second open panel and a second z-order for the sake of a narrow window.
    ///
    /// Returns true either way: the list closing is itself a change to draw.
    fn open_in_place_of_overflow(&mut self, kind: toolbar::Menu) -> bool {
        let more = self
            .toolbar_buttons
            .iter()
            .find(|button| button.action == toolbar::Action::More)
            .copied();
        if let Some(more) = more {
            self.toggle_menu(more, kind);
        }
        true
    }

    /// Takes the tooltip away, and stops one being on its way.
    ///
    /// Called from everything that means the user has moved on: a click, a keystroke, a wheel notch,
    /// a tab switch, the pointer leaving the window. Deliberately blunt — a tooltip that lingered
    /// over a document the user is now typing into is worse than one that is a little too eager to
    /// go.
    fn dismiss_tooltip(&mut self) -> bool {
        self.dwell_at = None;
        self.hovered = None;
        self.tooltip.take().is_some()
    }

    /// Takes the tooltip **and** any open list away.
    ///
    /// For everything that is not a click: a keystroke, a wheel notch, the pointer leaving the
    /// window. Every reason to dismiss one is a reason to dismiss the other.
    ///
    /// **A click must not use this**, and getting that wrong is what shipped a heading menu whose
    /// rows could not be clicked: `Editor::click` dismissed the tooltip first, which closed the menu
    /// with it, so by the time `click_menu` looked there was nothing open and the click fell through
    /// to the document. The two are separate functions now so the click path cannot reach the wrong
    /// one — it was a comment claiming the right order, above code that did not have it.
    fn dismiss_panels(&mut self) -> bool {
        let menu = self.close_menu();
        self.dismiss_tooltip() || menu
    }

    /// When the loop should wake up next, out of everything waiting on a clock.
    ///
    /// **There is one `ControlFlow` and two things want it**, which is the whole of MDEDIT-85's
    /// named risk. The reconcile timer owned it outright until this story: the keystroke handler set
    /// `WaitUntil(now + RECONCILE_AFTER)` directly, and `new_events` set `Wait` again afterwards.
    /// Adding a second deadline to that arrangement breaks it in both directions — a dwell set while
    /// a reconcile is pending pushes the reconcile out, and a reconcile set while a dwell is pending
    /// swallows the tooltip.
    ///
    /// The fix is not clever: keep both and take the earliest. It gets a test rather than a comment
    /// because the failure is silent and intermittent — the same shape as MDEDIT-49's `follow_caret`
    /// bug, where two features quietly fought over one field.
    #[must_use]
    pub fn next_wake(&self) -> Option<Instant> {
        match (self.reconcile_at, self.dwell_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (deadline, None) | (None, deadline) => deadline,
        }
    }

    /// Arms the idle beat, which is what a keystroke owes.
    ///
    /// Public for the same reason `on_idle` is: the keystroke that would set it arrives as a winit
    /// `KeyEvent`, which a test cannot construct.
    pub fn schedule_reconcile(&mut self) {
        self.reconcile_at = Some(Instant::now() + RECONCILE_AFTER);
    }

    /// Points the loop at whichever deadline comes first, or lets it sleep.
    ///
    /// The **only** place `ControlFlow` is set after start-up, which is what keeps
    /// [`next_wake`](Self::next_wake) the single answer rather than one opinion among several.
    fn reschedule(&self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(
            self.next_wake()
                .map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
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
        // The seventh, and MDEDIT-76's: the bold digits and the `!` a flagged line is drawn with.
        self.flagged_digits = None;
        self.status_labels = [None, None, None];
        self.row_labels.clear();
        self.toolbar_labels.clear();
        // MDEDIT-85's, and the eighth. A tooltip is a line of laid-out text like every other piece
        // of chrome here, and the text size is not in its key either.
        self.tooltip_labels.clear();
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

    /// What a window position means to the file pane, if anything.
    ///
    /// The rects are in the pane's own coordinates and the pane starts below the formatting bar, so
    /// the window's y has to come back to the pane's first. Three callers need that — a click, a
    /// wheel notch and a middle-click — and doing it in three places is how two of them end up
    /// disagreeing.
    fn pane_hit(&self, x: f32, y: f32) -> Option<pane::Hit> {
        pane::hit_test(&self.row_rects, x, y - self.toolbar_height())
    }

    /// What a window position means to the tab strip, if anything.
    ///
    /// The strip's twin of [`pane_hit`](Self::pane_hit), and it exists for the same reason that one
    /// does: since MDEDIT-84 the strip starts below the formatting bar, so the window's y has to
    /// come back to the strip's — and there are **two** callers, a left click and a middle click.
    /// Doing the subtraction in both places is how one of them ends up not doing it, which is
    /// exactly what happened here when only the left click was moved.
    fn strip_hit(&self, x: f32, y: f32) -> Option<tabs::Hit> {
        self.strip.hit_test(x, y - self.toolbar_height())
    }

    /// Resolves a click in the formatting bar.
    ///
    /// **Returns true for any click inside the bar's height**, including the gaps between buttons
    /// and including a dimmed button. A click on chrome must never reach the document, and neither
    /// "it fell between two buttons" nor "that one cannot act" is a reason to start a selection in
    /// the text underneath.
    ///
    /// The dimmed check is here rather than in [`perform`](Self::perform), and that is the same
    /// second-door rule the whole bar follows: `perform` is what a chord reaches too, and Ctrl+S on
    /// an unmodified document is allowed to be a no-op. What a *dimmed button* means is that this
    /// particular door is shut, which is a fact about the click.
    fn click_toolbar(&mut self, x: f32, y: f32) -> bool {
        if self.toolbar_buttons.is_empty() {
            return false;
        }
        // The bar is the topmost chrome, so its own top is the window's. `click_strip` starts where
        // this ends, and `the_two_bands_abut_exactly` is what says they still do.
        let top = 0.0;
        if !(top..top + toolbar::HEIGHT).contains(&y) {
            return false;
        }

        if let Some(action) = toolbar::hit_test(&self.toolbar_buttons, x, y - top) {
            // The buttons that open a panel rather than edit anything. One lookup rather than
            // two special cases, so a third menu-opener cannot be forgotten here.
            if let Some(kind) = self.menu_for(action) {
                let button = self
                    .toolbar_buttons
                    .iter()
                    .find(|button| button.action == action)
                    .copied();
                if let Some(button) = button {
                    self.toggle_menu(button, kind);
                }
            } else if self.is_enabled(action) {
                self.perform(action);
            }
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
            toolbar::Action::New => {
                self.new_file();
                true
            }
            toolbar::Action::Open => self.open_prompted(),
            toolbar::Action::Save => {
                self.save();
                false
            }
            toolbar::Action::Undo => self.document_mut().undo(),
            toolbar::Action::Redo => self.document_mut().redo(),
            toolbar::Action::Cut => self.cut(),
            toolbar::Action::Copy => {
                self.copy();
                false
            }
            toolbar::Action::Paste => self.paste(),
            // **The four that are not a second door.** They open a panel, and `click_toolbar` deals
            // with them before it gets here, as `click_menu` does for one chosen from the overflow
            // list — a chord never reaches them, because `Action::chord` gives them none. Reaching
            // this arm means somebody called `perform` directly with one, which is a caller bug
            // rather than a user action, so it does nothing rather than guessing which row, or which
            // size, was meant. The overflow list was exactly that caller until MDEDIT-193.
            toolbar::Action::Headings
            | toolbar::Action::Lists
            | toolbar::Action::More
            | toolbar::Action::Table => false,
            // MDEDIT-128. Each answers `false` when the caret is not in a table, or when the edit
            // would leave something that is not one — the header row, the last column — which is
            // what `Document` refuses rather than what this decides.
            toolbar::Action::RowAbove => self.document_mut().insert_table_row(false),
            toolbar::Action::RowBelow => self.document_mut().insert_table_row(true),
            toolbar::Action::ColumnLeft => self.document_mut().insert_table_column(false),
            toolbar::Action::ColumnRight => self.document_mut().insert_table_column(true),
            toolbar::Action::DeleteRow => self.document_mut().delete_table_row(),
            toolbar::Action::DeleteColumn => self.document_mut().delete_table_column(),
            toolbar::Action::DeleteTable => self.document_mut().delete_table(),
            toolbar::Action::SelectRow => self.select_table_row(),
            toolbar::Action::SelectTable => self.select_whole_table(),
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
            // MDEDIT-87 added the button; MDEDIT-129 made both of them ask before they write.
            toolbar::Action::Image => self.image(),
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
        // **No cache clear, since MDEDIT-177.** The panel floats, so the wrapping width is the same
        // width it was a moment ago and every cached layout is still the right one. It used to take
        // height out of the content rect, and a bar that changed the rect made every chunk's layout
        // stale — so opening Find re-laid out the whole visible document, which is what a cache
        // keyed on width is for avoiding.
        true
    }

    /// Closes the find bar and gives the document the keyboard back.
    ///
    /// Returns whether it was open, which is what lets Esc fall through when it was not.
    pub fn close_find(&mut self) -> bool {
        if !self.find_open {
            return false;
        }
        self.find_open = false;
        self.find_focused = false;
        self.find_layout = None;
        // Nothing to clear here either, and for the reason `open_bar` gives.
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
    /// answer, and for the same reason, as `counted_for`.
    fn find_document(&self) -> u64 {
        self.document_id()
    }

    /// Which document is showing, by an identity that survives another tab closing.
    ///
    /// **Not the index.** `Workspace::active` is a position in a `Vec`, and positions are reused:
    /// closing tab 0 makes the old tab 1 into tab 0. Two documents nobody has typed in are both at
    /// revision 0 — the common case — so every `(document, revision)` staleness stamp keyed on the
    /// index compared *equal* across a close, and the survivor inherited the closed tab's find
    /// highlights, diagnostics, error count and word count until something moved. MDEDIT-105.
    ///
    /// `Tab::snapshot_id` counts up and is never reused, which is exactly the property the recovery
    /// store already relies on for the same reason.
    fn active_document_id(&self) -> Option<u64> {
        let index = self.workspace.active()?;
        self.workspace
            .tabs()
            .get(index)
            .map(mdedit_session::Tab::snapshot_id)
    }

    /// [`Self::active_document_id`], with a value no document can have when there is no tab.
    fn document_id(&self) -> u64 {
        self.active_document_id().unwrap_or(u64::MAX)
    }

    /// Where the floating panel's top-left corner goes, in device-independent pixels (MDEDIT-177).
    ///
    /// **Top right of the document, and it moves out of the caret's way.** Top right is the
    /// convention — VS Code, Chrome and Word all put it there — and the right is where a
    /// left-to-right reader's eye is least often. It stops at the content's right edge rather than
    /// the window's, so it never covers the scrollbar; the scrollbar overlays the window's right
    /// margin, which is outside [`ContentRect`], and a panel over the thumb would be a panel the
    /// scrollbar's own hit test wins from underneath.
    ///
    /// **The one case where it moves is the caret behind it**, which is the case the ticket's
    /// acceptance names: the match you have just navigated to is selected, so the caret is on it,
    /// and a panel over the thing you were looking for is worse than one on the wrong side. VS Code
    /// shifts its widget left for the same reason. It shifts only when the left actually helps — in
    /// a window too narrow for the panel to move anywhere the two overlap whatever is chosen, and
    /// the convention is the better of two bad answers.
    fn find_origin(&self, panel_width: f32, panel_height: f32) -> (f32, f32) {
        let top = self.content.top_edge();
        let left_edge = self.content.left_edge();
        let right = (left_edge + self.content.width() - panel_width).max(left_edge);
        let clear = |left: f32| match self.caret_area {
            Some((x, y, height)) => {
                x + CARET_WIDTH <= left
                    || x >= left + panel_width
                    || y + height <= top
                    || y >= top + panel_height
            }
            None => true,
        };
        match (clear(right), clear(left_edge)) {
            (false, true) => (left_edge, top),
            _ => (right, top),
        }
    }

    /// Draws the find panel over the top right of the document, and remembers where its parts went.
    fn draw_find(&mut self, quads: &mut Vec<QuadInstance>, scale: f32) {
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
            if !self.toolbar_labels.contains_key(&glyph)
                && let Some(label) = self.lay_out_label(&glyph, style)
            {
                self.toolbar_labels.insert(glyph.clone(), label);
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

        // Wanted, then what there is room for — and then a floor, because a panel narrower than its
        // own buttons draws them over the document where no click can reach them. See
        // `find::least_width`.
        let panel_width = find::width(mode)
            .min(self.content.width())
            .max(find::least_width(mode, &widths));
        let panel_height = find::height(mode);
        let (left, top) = self.find_origin(panel_width, panel_height);
        let layout = find::layout(mode, (left, top), panel_width, count_width, &widths);

        // A border and then the panel inside it, as an open list draws itself: over prose a panel
        // with no edge reads as a bite taken out of the text. The docked bar never needed one — it
        // sat on the chrome, where the document's background *was* the edge.
        quads.push(Self::solid(
            [
                (left - 1.0) * scale,
                (top - 1.0) * scale,
                (panel_width + 2.0) * scale,
                (panel_height + 2.0) * scale,
            ],
            self.theme.line,
        ));
        quads.push(Self::solid(
            [
                left * scale,
                top * scale,
                panel_width * scale,
                panel_height * scale,
            ],
            self.theme.panel,
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
            // The layout's parts are measured from the panel's corner, so everything drawn from
            // here down is placed relative to it. See `find::Layout`.
            let box_ = (left + box_.0, box_.1);
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
                x: left + layout.count,
                baseline: baseline_of(0.0),
                scale,
                colour: self.theme.marker,
            };
            self.draw_label_glyphs(quads, &label.glyphs, where_);
        }

        self.draw_find_buttons(quads, &layout, baseline_drop, scale);
        self.find_layout = Some(layout);
    }

    /// Draws the panel's buttons: the lit ground under Match Case when it is on, then each glyph
    /// centred in its cell.
    ///
    /// Its own method because [`draw_find`](Self::draw_find) was over the line limit with it inline,
    /// and this is the part of the panel that has nothing to do with where the panel is — every
    /// rect it needs is already in the layout, measured from the panel's corner, and `baseline_drop`
    /// is the panel's first row's baseline.
    fn draw_find_buttons(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        layout: &find::Layout,
        baseline_drop: f32,
        scale: f32,
    ) {
        let (left, top) = layout.origin;
        let case = self.find.case_sensitive;
        // Not cloned, unlike when this was inline in `draw_find`: `layout` is a parameter now rather
        // than a local of the same `&mut self` borrow, so the loop borrows nothing its body touches.
        // It is a heap allocation a frame, on the path the latency harness measures.
        for rect in &layout.buttons {
            if rect.button == find::Button::Case && case {
                quads.push(Self::solid(
                    [
                        (left + rect.x) * scale,
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
                x: left + rect.x + (rect.width - label.width) / 2.0,
                baseline: baseline_drop + rect.y,
                scale,
                colour: self.theme.text,
            };
            let glyphs = label.glyphs.clone();
            self.draw_label_glyphs(quads, &glyphs, where_);
        }
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
        if !layout.contains(x, y) {
            return false;
        }

        if let Some(which) = layout.over_field(x, y) {
            self.find_focused = true;
            self.find.focus = which;
            return true;
        }
        match layout.hit_test(x, y) {
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

    /// Where the strip's tabs are, in the strip's own coordinates, for a test.
    ///
    /// The strip's origin is not in here — it is the formatting bar's height, and a caller turning
    /// a window position into a hit should use `strip_hit` rather than doing the arithmetic again.
    #[must_use]
    pub fn tab_rects(&self) -> &[tabs::TabRect] {
        &self.strip.tabs
    }

    /// Where the file pane's rows landed, for a test. The pane's twin of
    /// [`tab_rects`](Self::tab_rects), and it is `pane_hit` that turns a window position into one.
    #[must_use]
    pub fn row_rects(&self) -> &[pane::RowRect] {
        &self.row_rects
    }

    /// What a button actually drew — its icon, or the word it fell back to — for a test.
    ///
    /// The distinction is invisible from outside otherwise, and it is the whole of MDEDIT-92's
    /// fallback: a bar of words on a machine with no icon font is the old bar, and a bar of empty
    /// boxes is a bug report. Only a frame decides which, so this reads what the last one cached.
    #[must_use]
    pub fn button_face_drawn(&self, action: toolbar::Action) -> Option<&str> {
        self.toolbar_labels
            .get(&self.face_key(action))
            .map(|label| label.text.as_str())
    }

    /// Where the bar's buttons are, for a test.
    #[must_use]
    pub fn toolbar_buttons(&self) -> &[toolbar::ButtonRect] {
        &self.toolbar_buttons
    }

    /// Resolves a click in the tab strip.
    fn click_strip(&mut self, x: f32, y: f32) -> bool {
        let Some(hit) = self.strip_hit(x, y) else {
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
            tabs::Hit::More => self.toggle_tab_menu(),
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
    fn offset_at_point(&self, x: f32, y: f32) -> Option<(ByteOffset, Affinity)> {
        // The same rectangle the glyphs were placed with, run backwards. Neither side computes an
        // offset of its own, so the two cannot drift apart.
        let (x, y) = self.content.to_document(x, y, self.viewport_offset());
        let (offset, affinity) = self.offset_in_document(x, y)?;
        // **A task's box is not a caret position** (MDEDIT-142). A point on it addresses its `[`,
        // which in preview sits between concealed markers — the bullet and the box — so typing there
        // would write into the list's syntax and break the item. Past them is the nearest place a
        // caret can be. Plain mode draws the brackets as they are, and before one is fine.
        //
        // The affinity goes with it: this is a *different* offset from the one the point addressed,
        // moved forward past concealed syntax, so what row the point was on says nothing about which
        // row this one belongs to.
        if self.mode == view::Mode::Preview && self.document().task_box_at(offset).is_some() {
            return Some((self.past_concealed(offset), Affinity::Downstream));
        }
        Some((offset, affinity))
    }

    /// The task box drawn under a point in the window, as the offset of its `[`.
    ///
    /// The glyph is asked first and the document second: a bullet is substituted too, so "a
    /// substitute is under the pointer" is not yet "a box is". Plain mode substitutes nothing, so it
    /// never has a box to find.
    fn task_box_at_point(&self, x: f32, y: f32) -> Option<ByteOffset> {
        let (x, y) = self.content.to_document(x, y, self.viewport_offset());
        let frame = self.last_frame.as_ref()?;
        let index = frame.tops.partition_point(|top| *top <= y).checked_sub(1)?;
        let chunk = *frame.chunks.get(index)?;
        let cached = self.chunk_layout(frame, index)?;
        let target = cached.as_hit(chunk, &frame.text);
        let at = hit::substitute_at(&target, cached.drawn_lines(), y - frame.tops[index], x)?;
        self.document().task_box_at(at).map(ByteSpan::start)
    }

    /// The byte at a point in the document's own coordinates, and the row that point was on.
    ///
    /// The [`Affinity`] rides along because only a point knows it — see [`hit::offset_at`].
    fn offset_in_document(&self, x: f32, y: f32) -> Option<(ByteOffset, Affinity)> {
        let frame = self.last_frame.as_ref()?;
        if frame.chunks.is_empty() {
            return None;
        }
        if y < 0.0 {
            return Some((ByteOffset::ZERO, Affinity::Downstream));
        }

        let index = frame
            .tops
            .partition_point(|top| *top <= y)
            .saturating_sub(1)
            .min(frame.chunks.len() - 1);
        let chunk = frame.chunks[index];
        let cached = self.chunk_layout(frame, index)?;

        let target = cached.as_hit(chunk, &frame.text);
        Some(hit::offset_at(
            &target,
            cached.drawn_lines(),
            y - frame.tops[index],
            x,
        ))
    }

    /// The byte a point addresses, when the caller has no use for which row addressed it.
    fn offset_only_at_point(&self, x: f32, y: f32) -> Option<ByteOffset> {
        self.offset_at_point(x, y).map(|(offset, _)| offset)
    }

    /// Which chunk and visual line the caret is on, and how far along it.
    fn caret_place(&self) -> Option<(usize, usize, f32)> {
        let frame = self.last_frame.as_ref()?;
        let selection = self.document().selection();
        let caret = selection.head();
        let index = frame
            .chunks
            .iter()
            .position(|chunk| chunk.span.start() <= caret && caret < chunk.span.end())
            .or_else(|| frame.chunks.len().checked_sub(1))?;
        let chunk = frame.chunks[index];
        let cached = self.chunk_layout(frame, index)?;

        let target = cached.as_hit(chunk, &frame.text);
        let (row, x) = cached.place_of(&target, caret, selection.affinity())?;
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
        let target = cached.as_hit(chunk, &frame.text);
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
        // **The affinity comes from the aimed-at row, not from the row being left** (MDEDIT-188).
        // This is a point gesture like a click: aiming past the end of a wrapped row lands on that
        // row's last byte and must stay drawn there, or Down out of it would appear not to move.
        let Some((offset, affinity)) = self.offset_in_document(aim, edge + step) else {
            return false;
        };

        self.select_with(offset, extend, affinity);
        // Restored after `select` clears it: this is the one motion that keeps its column.
        self.desired_x = Some(aim);
        true
    }

    /// Puts the caret at `offset`, or drags the selection to it.
    ///
    /// The caret gets no affinity, which is [`Affinity::Downstream`] — right for every gesture that
    /// names an offset without knowing which visual row it meant. A gesture that *does* know calls
    /// [`select_with`](Self::select_with); there are only the ones driven by a point.
    fn select(&mut self, offset: ByteOffset, extend: bool) {
        self.select_with(offset, extend, Affinity::Downstream);
    }

    /// Puts the caret at `offset` on the row `affinity` names, or drags the selection to it.
    fn select_with(&mut self, offset: ByteOffset, extend: bool, affinity: Affinity) {
        let selection = if extend {
            Selection::new(self.document().selection().anchor(), offset)
        } else {
            Selection::caret(offset)
        };
        self.document_mut()
            .set_selection(selection.with_affinity(affinity));
        self.desired_x = None;
    }

    /// The horizontal extent of `selected` on one visual row, if it covers any of it.
    ///
    /// A row wholly inside the selection runs to the end of its text plus a little, so that a
    /// selection spanning lines reads as covering the line breaks rather than as a ragged stack of
    /// separate highlights.
    fn row_selection(row: Row<'_>, selected: ByteSpan) -> Option<(f32, f32)> {
        let Row { cached, index, .. } = row;
        let line = cached.layout.lines.get(index)?;
        let rebase = |offset: ByteOffset| {
            ByteOffset::new(row.chunk.span.start().get() + offset.get().saturating_sub(cached.base))
        };

        // Where this row's text sits in the buffer, asked of `hit`'s mapping (MDEDIT-183). It used
        // to count newlines here, which made a wrapped row's bounds the whole source line's — and
        // the comment called that "a little generous". It was not generous, it was the wrong row: a
        // selection on the line *after* a wrap was drawn on a continuation row of the line above,
        // because row 1 is that line's continuation rather than source line 1.
        let (row_start, row_end) = row.bytes();

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
        // **A task's box is the first thing in a document that is pressed rather than placed in**
        // (MDEDIT-142). It needs no z-order of its own: `click` has asked every panel before reaching
        // here, so a box is only pressed where nothing is drawn over it. Shift still extends a
        // selection, which is what it means everywhere else in the text.
        if !self.modifiers.shift_key()
            && let Some(task) = self.task_box_at_point(x, y)
        {
            // Not part of a run: a second press on the box unticks it rather than selecting a
            // word, and a click in the text after it is a first click.
            self.clicks.reset();
            return self.document_mut().toggle_task(task);
        }

        let Some((offset, affinity)) = self.offset_at_point(x, y) else {
            return false;
        };

        // Shift+click extends the existing selection and is not part of a run. Counting it would
        // make shift-clicking twice in a place select a word, which is not what either gesture
        // means.
        if self.modifiers.shift_key() {
            self.clicks.reset();
            self.granularity = Granularity::Character;
            self.anchor_span = None;
            self.select_with(offset, true, affinity);
            self.dragging = true;
            return true;
        }

        self.granularity = Granularity::of(self.clicks.press(at, x, y));
        let span = self.unit_at(offset);
        self.anchor_span = Some(span);

        match self.granularity {
            // Unchanged from before this existed, deliberately: a slow click must behave exactly as
            // it always did, and the counting wraps the old path rather than replacing it.
            Granularity::Character => self.select_with(offset, false, affinity),
            // A word or a line is a *span*, and its head is the unit's end rather than the byte
            // under the pointer — so which row the pointer was on says nothing about it. The one
            // place a word can end on a wrap boundary is where the wrap fell mid-word, and there the
            // selection visibly covers both rows, which answers the question the affinity would.
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
        let Some((offset, affinity)) = self.offset_at_point(x, y) else {
            return false;
        };
        let Some(anchor) = self
            .anchor_span
            .filter(|_| !matches!(self.granularity, Granularity::Character))
        else {
            // Character granularity, so the head is the byte under the pointer and the affinity
            // applies exactly as it does to a click. The word and line arms below take a unit's
            // end instead, and no row said where that is.
            self.select_with(offset, true, affinity);
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
        let language = self.document().language().name();
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
                    language,
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
            content, offset, ..
        } = at;
        let (band_top, band_bottom) = content.visible(offset);
        let mut line_height = self.theme.body_size;
        // The source line the caret is on, for MDEDIT-77's highlight and for the brighter number
        // beside it. Taken once per frame rather than per row: it is a walk of the rope's line index.
        let caret_line = self
            .document()
            .line_and_column(frame.selection.head())
            .0
            .get();

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
            let runs =
                view::build_runs_in(self.document(), self.decoration(), chunk.span, &frame.text);
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
            let wrong = self.diagnostics_touching(chunk.span);
            // What bytes each row covers, from `hit` and **once per chunk** (MDEDIT-183). Every
            // consumer below takes it off `Row`: the selection, the find highlights, the squiggles
            // and the whitespace marks each used to work it out again from a newline count, which is
            // both the defect this fixes and, at one count per row per span, the slowest way to get
            // the wrong answer.
            let row_bytes = cached.as_hit(chunk, &frame.text).row_bytes();
            let mut row_top = chunk_top;
            self.draw_chunk_ground(quads, &cached, chunk, chunk_top, at);
            // The breaks inside a quote or list (MDEDIT-168), by the source line each is on.
            let nested = self.nested_break_lines(&cached, chunk);

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

                let current = source_line == caret_line;

                if self.config.line_numbers && starts_a_line {
                    // Only the row a source line *starts* on is numbered, so that is the row the
                    // gutter mark belongs on too — a wrapped continuation of a bad line is not a
                    // second error.
                    let flagged = wrong.iter().any(|span| {
                        self.document().line_and_column(span.start()).0.get() == source_line
                    });
                    self.draw_line_number(
                        quads,
                        LineNumber {
                            number: source_line + 1,
                            baseline,
                            flagged,
                            current,
                        },
                        at,
                    );
                }

                let here = Row {
                    cached: &cached,
                    chunk,
                    text: &frame.text,
                    rows: &row_bytes,
                    index: row,
                    top: top_of_row,
                    height: line.height,
                    chunk_top,
                };
                // First of everything on the row, so the selection and the search highlights sit on
                // top of it and the text sits on top of them.
                if current {
                    self.draw_caret_line(quads, here, at);
                }
                let rule = nested
                    .iter()
                    .find(|(at_line, _)| starts_a_line && *at_line == source_line);
                if let Some(&(_, indent)) = rule {
                    self.draw_nested_break(quads, here, indent, at);
                }
                self.draw_matches(quads, &found, here, at);
                self.draw_row_selection(quads, selected, here, at);
                if self.whitespace {
                    self.draw_marks(quads, here, baseline, at);
                }

                self.draw_rules(quads, &runs, here, at);
                self.draw_glyphs(quads, &runs, here, at);

                // **After the glyphs, deliberately.** Everything else drawn under a row is a ground
                // the text sits on; a squiggle is a mark *about* the text, and a descender crossing
                // it must not be able to hide it.
                self.draw_squiggles(quads, &wrong, here, at);
            }
        }
        line_height
    }

    /// Tints the row the caret is on (MDEDIT-77).
    ///
    /// # Three decisions, all deliberate
    ///
    /// **Every row of a wrapped line is lit, not only the row the caret sits on.** The highlight
    /// answers "which line am I editing", and a line is a source line — the gutter numbers it once
    /// and the status bar counts it once. Lighting one row of three would be answering a different
    /// question, and it is not what any editor does.
    ///
    /// **It stays lit while the find field has focus.** The document's caret has not moved; it is
    /// where Esc will put you back. Blinking the highlight off for the duration of a search would be
    /// motion with no information in it.
    ///
    /// **It stays lit under a selection**, so the two tints stack — `contrast.rs` asserts the text
    /// still clears the floor through both, which is the pairing the design pack does not model.
    ///
    /// It spans the document's width including its margins, but not the gutter: the gutter has its
    /// own answer to the same question, which is the brighter number.
    fn draw_caret_line(&mut self, quads: &mut Vec<QuadInstance>, row: Row<'_>, at: Placement) {
        let (_, y) = at.content.to_window(0.0, row.top, at.offset);
        // In window coordinates and not panned with the text: a highlight that slid sideways with a
        // horizontal pan would leave the rest of the row untinted.
        let left = at.content.left_edge() - MARGIN;
        let width = at.content.width() + MARGIN * 2.0;
        quads.push(Self::solid(
            [
                left * at.scale,
                y * at.scale,
                width * at.scale,
                row.height * at.scale,
            ],
            self.theme.caret_line,
        ));
    }

    /// Draws one visual row's glyphs, each in its run's colour.
    fn draw_glyphs(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        runs: &[view::ViewRun<'_>],
        row: Row<'_>,
        at: Placement,
    ) {
        for glyph in &row.cached.layout.lines[row.index].glyphs {
            let Some(entry) = self.entry_for(glyph.key, at.scale) else {
                continue;
            };
            let colour = view::colour_at(
                runs,
                row.cached.rebase(glyph, row.chunk.span),
                self.theme.text,
            );
            let (x, y) = at
                .content
                .to_window(glyph.pos.x, glyph.pos.y + row.chunk_top, at.offset);
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
    }

    /// Draws the wavy rule under whatever is wrong on one visual row (MDEDIT-76).
    ///
    /// # Why a row of quads rather than a shape
    ///
    /// There is one pipeline here and it draws rectangles, so the wave is made of them: a step of
    /// short dashes alternating between two heights. At a body size that reads as a squiggle and not
    /// as a dashed line, and it needs no shader, no second pipeline and no glyph.
    ///
    /// The geometry comes from [`Self::row_selection`] — the same helper the selection and the find
    /// highlights use — so a diagnostic spanning several rows is clipped to each of them by the code
    /// that already knows how, and one that is off-screen is never asked about.
    ///
    /// An empty span draws nothing: some checks point *at* a position rather than at bytes, and a
    /// zero-width squiggle would be a smudge with no relationship to anything. The gutter mark is
    /// the cue in that case.
    fn draw_squiggles(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        wrong: &[ByteSpan],
        row: Row<'_>,
        at: Placement,
    ) {
        for span in wrong {
            let Some((from, to)) = Self::row_selection(row, *span) else {
                continue;
            };
            if to <= from {
                continue;
            }

            let (x, y) = at.content.to_window(from, row.top, at.offset);
            // Along the bottom of the row, inside it: a rule below the row would land on the line
            // beneath at a tight line height, and read as that line's error.
            let bottom = y + row.height - SQUIGGLE_DROP;
            let mut cursor = 0.0f32;
            let width = to - from;
            let mut up = true;
            while cursor < width {
                let dash = SQUIGGLE_DASH.min(width - cursor);
                let top = if up { bottom } else { bottom + SQUIGGLE_RISE };
                quads.push(Self::solid(
                    [
                        (x + cursor) * at.scale,
                        top * at.scale,
                        dash * at.scale,
                        (SQUIGGLE_THICKNESS * at.scale).max(1.0),
                    ],
                    self.theme.span_error,
                ));
                cursor += dash;
                up = !up;
            }
        }
    }

    /// Rules a table: its outer border, the line between every row and column, and a wash behind the
    /// header (MDEDIT-126).
    ///
    /// Drawn once per chunk and before any row, so every ground a row draws — the caret line, a
    /// search match, the selection — lands on top of the wash, and the glyphs on top of those.
    /// Painter's order is the whole of the answer here as it is for the rest of the chrome.
    ///
    /// **`Rule` could not be reused.** That type is horizontal, is measured from a baseline, and
    /// takes its colour from the run it underlines — three things a table border is not. It carries
    /// an underline; this carries a box.
    ///
    /// The wash and the rules are both [`Theme::line`], the one translucent hairline role, which
    /// lands correctly on either ground without a second colour to keep in step. `panel` would not:
    /// it is pure white in light mode, which is the page.
    fn draw_table(
        &self,
        quads: &mut Vec<QuadInstance>,
        cached: &Cached,
        chunk_top: f32,
        at: Placement,
    ) {
        let Some(grid) = cached.grid.as_ref() else {
            return;
        };
        // The rows the grid covers. The trailing empty line is the blank line *after* the table, so
        // ruling it would draw a box around nothing.
        let heights: Vec<f32> = cached
            .layout
            .lines
            .iter()
            .take(grid.rows.len())
            .map(|line| line.height)
            .collect();
        if heights.is_empty() {
            return;
        }

        let width = grid.width();
        let total: f32 = heights.iter().sum();
        // A rule that rounds to nothing is a line that is not there, so it is floored at one
        // physical pixel — the same clamp `draw_rules` puts on an underline from a thin font.
        let hairline = table::RULE.max(1.0 / at.scale);
        let mut solid = |x: f32, y: f32, w: f32, h: f32| {
            let (wx, wy) = at.content.to_window(x, y + chunk_top, at.offset);
            quads.push(Self::solid(
                [wx * at.scale, wy * at.scale, w * at.scale, h * at.scale],
                self.theme.line,
            ));
        };

        // The header's ground, first, so a rule drawn over it stays visible.
        solid(0.0, 0.0, width, heights[0]);

        let mut y = 0.0;
        for height in &heights {
            solid(0.0, y, width, hairline);
            y += height;
        }
        solid(0.0, y, width, hairline);

        for x in grid.column_edges() {
            solid(x, 0.0, hairline, total);
        }
    }

    /// A block quote's bars, one per level (MDEDIT-145).
    ///
    /// In `theme.line`, the document's hairline colour, drawn [`quote::BAR`] wide so it reads as a
    /// stroke rather than a rule. **They move with the text when panned**, unlike a thematic break:
    /// a bar marks where quoted text is, so it belongs to the text's position rather than to the
    /// window's. A level's bar is one quad per band, and adjacent bands meet, so a quote whose depth
    /// changes part-way still draws its outer bar as one unbroken stroke.
    fn draw_quote(
        &self,
        quads: &mut Vec<QuadInstance>,
        cached: &Cached,
        chunk_top: f32,
        at: Placement,
    ) {
        let bar = quote::BAR.max(1.0 / at.scale);
        for band in &cached.quote {
            for level in 0..band.depth {
                let (x, y) = at.content.to_window(
                    band.x + quote::bar_x(level),
                    chunk_top + band.top,
                    at.offset,
                );
                quads.push(Self::solid(
                    [
                        x * at.scale,
                        y * at.scale,
                        bar * at.scale,
                        band.height * at.scale,
                    ],
                    self.theme.line,
                ));
            }
        }
    }

    /// A thematic break's rule (MDEDIT-143).
    ///
    /// **`---` is only its marker**, so concealing the marker in preview used to remove the whole
    /// construct: a divider in plain markup and nothing at all in preview. A list at least leaves its
    /// text behind; a break left a blank row.
    ///
    /// A quad rather than a MDEDIT-141 substitution, because a stand-in glyph cannot span the column.
    /// Drawn from the chunk's kind and the mode, and never cached, so `ChunkKey` carries nothing new
    /// — the MDEDIT-118 trap of a layout that varies with something its key does not hold does not
    /// arise. Same colour and thickness as a table's rules: both are the document's hairline.
    ///
    /// **Across the visible band, not from document x 0.** With word wrap off the view can pan, and
    /// everything in the document moves left with it. A divider has no width of its own to scroll,
    /// so it starts at the pan and spans the content rectangle — drawn from x 0 it would slide out of
    /// view under the gutter.
    fn draw_thematic_break(
        &self,
        quads: &mut Vec<QuadInstance>,
        cached: &Cached,
        chunk_top: f32,
        at: Placement,
    ) {
        if self.decoration().mode == view::Mode::Plain {
            return;
        }
        // The break's own line, which is the chunk's first: any blank lines after it belong to the
        // chunk too, and a rule through the middle of those would sit below the break.
        let Some(row) = cached.layout.lines.first() else {
            return;
        };
        let hairline = table::RULE.max(1.0 / at.scale);
        let y = chunk_top + (row.height - hairline) / 2.0;
        let (x, y) = at.content.to_window(at.offset.x, y, at.offset);
        quads.push(Self::solid(
            [
                x * at.scale,
                y * at.scale,
                at.content.width() * at.scale,
                hairline * at.scale,
            ],
            self.theme.line,
        ));
    }

    /// What a chunk draws as a whole rather than row by row: a table's rules, a quote's bars, a
    /// picture, a top-level break's rule. Before any row, so each row's text sits on top of them.
    #[allow(clippy::too_many_arguments, reason = "a draw call needs its geometry")]
    fn draw_chunk_ground(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        cached: &Cached,
        chunk: blocks::Chunk,
        chunk_top: f32,
        at: Placement,
    ) {
        self.draw_table(quads, cached, chunk_top, at);
        self.draw_quote(quads, cached, chunk_top, at);
        self.place_picture(cached, chunk, chunk_top, at);
        if chunk.kind == Some(BlockKind::ThematicBreak) {
            self.draw_thematic_break(quads, cached, chunk_top, at);
        }
    }

    /// The breaks nested in this chunk (MDEDIT-168), as the source line each is on and its indent.
    ///
    /// Asked once per chunk, and matched to a row by the same `line_of_row` the gutter uses: a
    /// wrapped row above a break only ever adds rows, so the break's row is the one its line starts
    /// on.
    /// What is wrong with the bytes in `span` (MDEDIT-76), for the squiggles on one chunk's rows.
    ///
    /// Filtered rather than binary-searched because MDEDIT-47 caps the list at 200: nobody is fixing
    /// the 201st, so the whole list is shorter than one screen of matches and a scan of it costs
    /// nothing.
    ///
    /// From [`diagnostics`](Self::diagnostics), which is empty while an edit is newer than the check
    /// — so an edit takes the squiggles away and the idle timer brings them back, with no hook on
    /// any edit path. Nothing here re-runs the check.
    fn diagnostics_touching(&self, span: ByteSpan) -> Vec<ByteSpan> {
        self.diagnostics()
            .iter()
            .filter(|diagnostic| touches(diagnostic.span, span))
            .map(|diagnostic| diagnostic.span)
            .collect()
    }

    fn nested_break_lines(&self, cached: &Cached, chunk: blocks::Chunk) -> Vec<(usize, f32)> {
        cached
            .breaks
            .iter()
            .map(|nested| {
                let offset =
                    (chunk.span.start().get() + nested.at).min(self.document().len_bytes());
                (self.document().rope().byte_to_line(offset), nested.indent)
            })
            .collect()
    }

    /// A thematic break's rule, inside a quote or a list item (MDEDIT-168).
    ///
    /// The same hairline as a top-level break's, across the middle of the break's own row, but
    /// **from its container's indent rather than from the column's edge**: a rule in a quote that
    /// crossed the quote's bars would read as the quote ending, and one in a list item that ran
    /// under the bullet would read as a break between items rather than within one.
    ///
    /// It starts at a document x, so like the bars it moves with the text when panned, and it runs
    /// to the right-hand edge of the visible band as a top-level rule does. Nothing in plain mode:
    /// there the quote and list layouts are not used, so no break is recorded, and the dashes draw.
    fn draw_nested_break(
        &self,
        quads: &mut Vec<QuadInstance>,
        row: Row<'_>,
        indent: f32,
        at: Placement,
    ) {
        let hairline = table::RULE.max(1.0 / at.scale);
        let y = row.top + (row.height - hairline) / 2.0;
        let (x, y) = at.content.to_window(indent, y, at.offset);
        let width = (at.content.width() - (indent - at.offset.x)).max(0.0);
        quads.push(Self::solid(
            [
                x * at.scale,
                y * at.scale,
                width * at.scale,
                hairline * at.scale,
            ],
            self.theme.line,
        ));
    }

    /// Records where this chunk's picture goes, if it has one (MDEDIT-146).
    ///
    /// **Recorded rather than drawn**, and into a list of its own rather than into `quads`. A
    /// picture samples a texture of its own, so it cannot share the glyph atlas's draw call; and
    /// keeping it as data is what lets a test assert the geometry without a GPU, which is the only
    /// automated check this feature can have.
    ///
    /// No culling of its own. `draw_chunks` already only walks the visible chunks, and a picture
    /// hanging past the edge of the document area is covered by chrome, which draws after — the
    /// painter's order `docs/decisions/chrome.md` says is the whole of the answer.
    fn place_picture(
        &mut self,
        cached: &Cached,
        chunk: blocks::Chunk,
        chunk_top: f32,
        at: Placement,
    ) {
        let Some(fitted) = cached.picture else {
            return;
        };
        // Rebased from the chunk, not taken from the cache as-is: `Fitted` is content-keyed and the
        // same image in two paragraphs shares one entry. See `Fitted::span`.
        let base = chunk.span.start().get();
        self.picture_spans.push(ByteSpan::new(
            ByteOffset::new(base + fitted.span.start().get()),
            ByteOffset::new(base + fitted.span.end().get()),
        ));
        let (x, y) = at.content.to_window(0.0, chunk_top, at.offset);
        self.drawn_pictures.push(crate::image::Drawn {
            rect: [
                x * at.scale,
                y * at.scale,
                fitted.width * at.scale,
                fitted.height * at.scale,
            ],
            id: fitted.id,
        });
    }

    /// Draws the rules under and through one visual row's runs (MDEDIT-73, MDEDIT-91).
    ///
    /// Where each rule sits and how thick it is came from the font, through the text layer — the
    /// renderer turns the rectangle into a quad and decides nothing about it but the colour, which
    /// is the run's own so a rule cannot disagree with the text it belongs to.
    ///
    /// **A strikethrough needs no arm of its own here.** It arrives as the same rectangle with a
    /// different `top`, because DirectWrite measures both offsets from the baseline — which is why
    /// MDEDIT-91 added a field to `Rule` and not a branch to this loop.
    fn draw_rules(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        runs: &[view::ViewRun<'_>],
        row: Row<'_>,
        at: Placement,
    ) {
        for rule in &row.cached.layout.lines[row.index].rules {
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

        self.draw_row_marks(quads, row, chunk_top, at);

        // The line ending, past the end of the row. An empty line has no glyphs, so its terminator
        // is found from where the row starts rather than from where it ends.
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

                // **Whatever the layout declined to shape, which on DirectWrite is the trailing
                // tab.** MDEDIT-113 measured it: `"ab \r\ncd"` shapes the space, `"ab\t\ncd"` does
                // not shape the tab. So a trailing tab has no glyph to stamp a mark into and would
                // be the one invisible character this feature could not show.
                //
                // The condition is "has a mark and was not shaped" rather than a list of
                // characters, because which ones a backend skips is the backend's business and the
                // set is not the same on both. Walking from past the last glyph is what keeps a
                // character that *did* get one from being marked twice, and `mark_for` returning
                // `None` for `\r` and `\n` is what stops the walk at the terminator.
                while let Some(character) = row.text[after..].chars().next() {
                    let Some(mark) = view::mark_for(character) else {
                        break;
                    };
                    if let Some(drawn) = self.mark(mark) {
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

                (view::ending_at(row.text, after), pen)
            }
            None => (view::ending_at(row.text, Self::row_start(row)), 0.0),
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

    /// Marks every invisible character on a row that the layout gave a place to (MDEDIT-124).
    ///
    /// **The characters come from the text and their positions from the layout.** Iterating the
    /// glyphs alone misses everything the layout declined to shape, and on DirectWrite that is an
    /// interior tab and every C0 control: a tab is an *advance to a stop* rather than a glyph, and a
    /// control is dropped outright. A character with no glyph has no cell to stamp into — but its
    /// place is recoverable, because it lies in the gap between the glyph before it and the glyph
    /// after.
    ///
    /// Before this the feature's headline mark was invisible in the case people actually hit: only a
    /// *trailing* tab was marked, by the walk past the last glyph in [`draw_marks`](Self::draw_marks)
    /// which reads characters out of the text.
    ///
    /// Nothing here re-shapes anything. A mark is stamped into space the layout already decided on,
    /// so no glyph moves and no click lands anywhere new — MDEDIT-46's standing rule, and the reason
    /// this could not be fixed by giving the character a glyph.
    ///
    /// **Bidi:** the gap is read between neighbours in *source* order, which is their visual order
    /// only in a left-to-right run. A tab inside a right-to-left run is out of scope, as it was
    /// before — it was not marked at all then.
    fn draw_row_marks(
        &mut self,
        quads: &mut Vec<QuadInstance>,
        row: Row<'_>,
        chunk_top: f32,
        at: Placement,
    ) {
        let line = &row.cached.layout.lines[row.index];
        let baseline = line.baseline + chunk_top;
        let mut glyphs = line.glyphs.iter().peekable();
        let mut pen = 0.0_f32;
        let mut offset = Self::row_start(row);

        while let Some(character) = row.text.get(offset..).and_then(|rest| rest.chars().next()) {
            // A glyph may cover several characters — a ligature does — so anything already behind
            // the walk is not a second cell.
            while glyphs
                .peek()
                .is_some_and(|glyph| row.cached.rebase(glyph, row.chunk.span).get() < offset)
            {
                glyphs.next();
            }

            let Some(next) = glyphs.peek().copied() else {
                // Past the last glyph. What remains on the row belongs to the trailing walk.
                break;
            };
            let source = row.cached.rebase(next, row.chunk.span).get();
            let shaped = source == offset;
            let (x, width) = if shaped {
                (next.pos.x, next.advance)
            } else {
                // The gap this character was given: from where the last glyph ended to where the
                // next one starts. A tab's stop makes that generous; a control takes no advance at
                // all and so leaves nothing, which is what the width check below reads.
                //
                // **Shared between every unshaped character in the run**, because two tabs in a row
                // sit between the same two glyphs — and a gap read whole would stack both their
                // arrows at its middle.
                let sharing = row
                    .text
                    .get(offset..source)
                    .map_or(1, |run| run.chars().count());
                let sharing = u16::try_from(sharing).unwrap_or(u16::MAX).max(1);
                (pen, (next.pos.x - pen) / f32::from(sharing))
            };

            if let Some(drawn) = view::mark_for(character).and_then(|mark| self.mark(mark)) {
                // **Only where it fits.** A control has no advance, so drawing its picture would put
                // it over the character beside it. Showing one needs room the layout never reserved,
                // which is a different feature from this one — recorded on MDEDIT-124.
                if drawn.advance <= width {
                    self.stamp(
                        quads,
                        drawn,
                        MarkAt {
                            pos: (x + (width - drawn.advance) / 2.0, baseline),
                            colour: self.theme.marker,
                            at,
                        },
                    );
                }
            }

            if shaped {
                pen = next.pos.x + next.advance;
                glyphs.next();
            } else {
                // Its share of the gap, so the next unshaped character in the run starts after it.
                pen += width;
            }
            offset += character.len_utf8();
        }
    }

    /// Where a row's text starts in the buffer.
    ///
    /// **Was its own newline count, and its defence was false** (MDEDIT-183). It claimed to be
    /// reached only for a row with no glyphs, where being wrong about wrapping would not matter —
    /// but `draw_row_marks` calls it for *every* row, as the offset its character walk starts from.
    /// On any row after a wrap the count returned the next line's start, so the walk paired this
    /// row's glyphs with another line's bytes, exited immediately, and left every tab and control
    /// character after the first wrapped row unmarked. That is the defect MDEDIT-124 was raised for,
    /// reintroduced one function away from its own test.
    fn row_start(row: Row<'_>) -> usize {
        row.bytes().0
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
        let run = StyledRun::from_buffer(
            character.encode_utf8(&mut text),
            TextStyle::body(self.theme.body_size),
            ByteSpan::empty_at(ByteOffset::ZERO),
        );
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

    /// What Enter does: inserts the terminator **this file already uses**, not always `\n`.
    ///
    /// MDEDIT-59 — Enter used to add an LF to a CRLF document, which is how a file becomes Mixed
    /// without anybody pasting anything into it.
    ///
    /// Split out of the key handler for the reason [`Self::apply_chord`] is: a winit `KeyEvent`
    /// cannot be constructed by a test, so a test that wanted to check this had to re-derive what to
    /// insert — and a test that asks the editor what its newline is and then inserts that would pass
    /// against the bug it was written for (MDEDIT-110). This *is* the arm's body.
    pub fn insert_newline(&mut self) -> bool {
        let ending = self.newline().as_str();
        self.insert(ending)
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
        let Some((from, to)) = Self::row_selection(row, selected) else {
            return;
        };
        let (x, y) = at.content.to_window(from, row.top, at.offset);
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
            let Some((from, to)) = Self::row_selection(row, *span) else {
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
        let run = [StyledRun::from_buffer(
            "0123456789",
            style,
            ByteSpan::empty_at(ByteOffset::ZERO),
        )];
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

    /// The ten digits and a `!`, in bold, for a line something is wrong on (MDEDIT-76).
    ///
    /// A second set rather than a reuse of [`digits`](Self::digits), because the theme's rule for a
    /// flagged line is the `error` colour **and** bold **and** a `!` — three cues, of which colour is
    /// the one a reader may not have. It costs one layout, the first time an error is on screen, and
    /// is cleared by [`Self::set_zoom`] with every other cache of laid-out text.
    fn flagged_digits(&mut self) -> Option<Flagged> {
        if let Some(flagged) = self.flagged_digits {
            return Some(flagged);
        }
        let style = TextStyle::body(self.theme.body_size).bold();
        let run = [StyledRun::from_buffer(
            "0123456789!",
            style,
            ByteSpan::empty_at(ByteOffset::ZERO),
        )];
        let layout = self.text.layout(&run, f32::MAX).ok()?;

        let measured: Vec<(GlyphKey, f32)> = layout
            .glyphs()
            .map(|glyph| (glyph.key, glyph.advance))
            .collect();
        // Eleven glyphs for eleven characters, unless a backend did something surprising — in which
        // case fall back to the plain digits rather than mis-render.
        let [
            zero,
            one,
            two,
            three,
            four,
            five,
            six,
            seven,
            eight,
            nine,
            bang,
        ] = <[(GlyphKey, f32); 11]>::try_from(measured).ok()?;
        let flagged = Flagged {
            digits: [zero, one, two, three, four, five, six, seven, eight, nine],
            bang,
        };
        self.flagged_digits = Some(flagged);
        Some(flagged)
    }

    /// Draws one line number, right-aligned in the gutter.
    ///
    /// Composed from the cached digits rather than laid out: see [`digits`](Self::digits).
    ///
    /// [`LineNumber::flagged`] is MDEDIT-76's gutter mark: the number goes bold and rose with a `!`
    /// before it, so a line with a diagnostic on it is findable by shape as well as by colour — which
    /// is what the design pack's contract asks for and what a squiggle alone does not give.
    fn draw_line_number(&mut self, quads: &mut Vec<QuadInstance>, line: LineNumber, at: Placement) {
        let LineNumber {
            number,
            baseline,
            flagged,
            current,
        } = line;
        let marked = flagged.then(|| self.flagged_digits()).flatten();
        // Three states, and the order between them is the point: something wrong outranks where you
        // are, and where you are outranks scaffolding.
        let colour = if marked.is_some() {
            self.theme.span_error
        } else if current {
            // MDEDIT-77, and the pack's rule for the role: "the current line steps up to `text`".
            // It is the gutter's second cue — the caret-line tint is the first, and a reader who
            // cannot see a 7% tint still sees which number is brighter.
            self.theme.text
        } else {
            self.theme.gutter
        };
        let Some(digits) = marked.map_or_else(|| self.digits(), |flagged| Some(flagged.digits))
        else {
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
        // The row's baseline, so the digits sit on the same line the text does — each glyph is then
        // placed from it by its own bitmap offset, exactly as a document glyph is.
        let (_, y) = at.content.to_window(0.0, baseline, at.offset);
        // Right-aligned against the *gutter's* right edge, less its padding — not against where the
        // text starts, which is what MDEDIT-34 was. `left_edge` is the gutter plus the document's
        // margin, so aligning to it drew the number past the gutter entirely and hard up against the
        // text.
        //
        // The width is a whole number of digit columns plus a padding either side, so a number that
        // fills it sits exactly centred, and a shorter one is right-aligned within that column —
        // which keeps the ones digits in line as the document passes ten and a hundred. Centring
        // each number on its own would break that.
        //
        // **Measured from the pane's right edge, not the window's** (MDEDIT-117). The gutter is the
        // second thing on the left when the file pane is showing — `insets.left` is
        // `pane_width() + gutter_width()` — and this used to start at zero regardless, which put
        // every number inside the pane's own band. They were drawn and then painted over, because
        // `draw_pane` runs after `draw_chunks`; the give-away was the empty gutter-width strip
        // between the pane and the text, reserved and holding nothing.
        let right = self.pane_width() + self.gutter_width() - GUTTER_PADDING;
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
                colour: [colour.r, colour.g, colour.b, colour.a],
            });
        }

        // The `!`, to the left of the number rather than in place of a digit: the numbers stay in
        // their column, so a flagged line does not push its neighbours' digits out of line.
        // Not a let-chain, and the floor is no longer the reason (MDEDIT-189): the `let … else`
        // inside **returns**, which abandons the whole mark rather than falling past this `if`.
        if let Some(flagged) = marked {
            let (key, bang_advance) = flagged.bang;
            let Some(entry) = self.entry_for(key, at.scale) else {
                return;
            };
            let x = right - width - bang_advance;
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
    }

    /// Lays a chunk out, from the cache when it is there and from the text engine when it is not.
    ///
    /// # The caret's own chunk is cached like any other, and used not to be
    ///
    /// It was laid out fresh every frame because what it drew depended on where in it the caret sat
    /// — reveal-on-caret put the marker bytes back into the text — and the caret is not in the key,
    /// so a cached entry would have served the wrong glyphs to the next keystroke.
    ///
    /// MDEDIT-118 removed that dependency: what is drawn is now a function of the bytes, the kind
    /// and the width, which is exactly what the key already is. So the chunk being typed in — the
    /// one chunk that is laid out on every single keystroke — is the one this now stops re-laying
    /// out.
    fn lay_out(&mut self, index: usize, frame: &Frame) -> Option<Rc<Cached>> {
        if let Some(cached) = frame.fresh_at(index) {
            return Some(cached);
        }
        let chunk = frame.chunks[index];
        let key = frame.keys[index];
        if let Some(cached) = self.layouts.get(&key) {
            return Some(Rc::clone(cached));
        }

        let body = chunk.span.slice(&frame.text);
        // MDEDIT-126. A table is the one chunk that is not a run of lines: its cells are laid out
        // apart and placed into columns. `None` means this block did not parse into cells after all,
        // and a malformed table is then drawn as the pipe rows it is rather than as an empty grid.
        let grid = match chunk.kind {
            Some(BlockKind::Table) => self.lay_out_table(chunk, frame),
            _ => None,
        };
        // MDEDIT-147. A list is the other chunk that is not a plain run of lines: its items are laid
        // out apart so a wrapped one hangs under its own text rather than under its bullet. It needs
        // no `Grid` — nothing about a list drops a source line, so `hit` goes on counting newlines.
        let hung = match chunk.kind {
            Some(BlockKind::List) if grid.is_none() => self.lay_out_list(chunk, frame),
            _ => None,
        };
        // MDEDIT-146. A lone image is the third chunk that is not a run of lines, and the only one
        // with no glyphs at all: it lays out as a single empty line as tall as the picture.
        // MDEDIT-145. A block quote is the fourth: each run of lines at one depth is laid out at the
        // width its indent leaves and placed, so a wrapped row stays clear of the bars.
        let quoted = match chunk.kind {
            Some(BlockKind::BlockQuote) => self.lay_out_quote(chunk, frame),
            _ => None,
        };
        let picture = if grid.is_none() && hung.is_none() && quoted.is_none() {
            self.lay_out_picture(chunk, body, frame)
        } else {
            None
        };
        let mut bands = Vec::new();
        let mut breaks = Vec::new();
        let (mut layout, grid) = if let Some((layout, grid)) = grid {
            (layout, Some(grid))
        } else if let Some((layout, quoted, nested)) = hung {
            bands = quoted;
            breaks = nested;
            (layout, None)
        } else if let Some((layout, quoted, nested)) = quoted {
            bands = quoted;
            breaks = nested;
            (layout, None)
        } else if let Some(fitted) = picture {
            (picture_layout(fitted, body), None)
        } else {
            let runs =
                view::build_runs_in(self.document(), self.decoration(), chunk.span, &frame.text);
            let styled = view::as_styled(&runs);
            (self.text.layout(&styled, frame.width).ok()?, None)
        };
        // **An empty document is a document with one empty line**, and a text engine handed nothing
        // to shape does not agree: `DirectWriteText::layout` returns `Layout::default()`, with no
        // rows at all. Everything downstream is keyed off rows, so a blank page — File → New, or a
        // zero-byte file, which is the first thing anybody sees — drew no caret, no line number and
        // no caret-line tint. Keystrokes worked and nothing said where they would land (MDEDIT-81).
        //
        // Repaired here rather than in each of those, because "one empty line" is what every editor
        // shows for a blank page and one `LayoutLine` with no glyphs is the whole of saying so. The
        // height is the nominal one, which is what an unmeasured chunk already gets.
        if layout.lines.is_empty() {
            let height = (self.theme.body_size * ESTIMATED_LINE_RATIO).max(1.0);
            layout.lines.push(mdedit_text::LayoutLine {
                baseline: height,
                height,
                glyphs: Vec::new(),
                rules: Vec::new(),
            });
        }

        let mut cached = Cached::new(layout, grid, picture, &chunk, body, key.last);
        cached.quote = bands;
        cached.breaks = breaks;
        let cached = Rc::new(cached);
        // What this chunk contributes to the horizontal clamp. Recorded here because this is where a
        // chunk stops being an estimate and becomes a measurement — the same moment its height
        // becomes real. Only ever grows within a layout width; `set_wrap` and `set_zoom` reset it
        // along with the caches whose answers it was derived from.
        self.widest = self.widest.max(cached.width);
        // Layouts are the memory-hungry half — a glyph apiece — so they are capped and the whole lot
        // dropped when the cap is hit. Heights are a float each and worth keeping: they are what
        // stops the document's length jumping around as it is scrolled.
        if self.layouts.len() >= MAX_CACHED_LAYOUTS {
            self.layouts.clear();
        }
        self.layouts.insert(key, Rc::clone(&cached));
        self.heights.insert(key, cached.height);
        Some(cached)
    }

    /// Decodes the picture this chunk is, and works out how big it is drawn (MDEDIT-146).
    ///
    /// `None` for every chunk that is not a lone image, and for one whose file cannot be read or
    /// decoded — a broken image then draws its alt text, which is what the editor did before
    /// pictures existed and is never a lie about the document.
    ///
    /// **Plain mode never comes here**, for the reason `lay_out_table` gives: `Ctrl+/` exists to
    /// show the bytes, and the bytes of an image are its syntax. A picture there would be the mode
    /// failing to do the one thing it is for.
    fn lay_out_picture(
        &mut self,
        chunk: blocks::Chunk,
        body: &str,
        frame: &Frame,
    ) -> Option<Fitted> {
        if self.decoration().mode == view::Mode::Plain {
            return None;
        }
        // **The cheap guard, and it is load-bearing.** `image::lone` filters the document's whole
        // segment list, so calling it for every paragraph would be quadratic in a long document.
        // A lone image begins `![` after at most some whitespace, and that is O(this chunk).
        if !body.trim_start().starts_with("![") {
            return None;
        }
        let found = crate::image::lone(self.document(), chunk.span, &frame.text)?;
        let destination = found.destination.slice(&frame.text);

        // Everything about what a destination may name lives in `mdedit_image::resolve`, including
        // the rule that keeps a UNC path from leaking an NTLM hash. None of it is repeated here.
        let path = match mdedit_image::resolve(destination, self.document_directory().as_deref()) {
            Ok(path) => path,
            Err(refused) => {
                Self::note_picture_refused(&refused);
                return None;
            }
        };
        let (id, declared_width, declared_height) = self.picture_at(path)?;
        let (width, height) = crate::image::fit((declared_width, declared_height), frame.width);
        let base = chunk.span.start().get();
        Some(Fitted {
            id,
            width,
            height,
            after: self.body_line_height(),
            span: ByteSpan::new(
                ByteOffset::new(found.span.start().get() - base),
                ByteOffset::new(found.span.end().get() - base),
            ),
        })
    }

    /// How tall one ordinary line of body text is, measured rather than estimated.
    ///
    /// A picture's chunk needs it for the blank line that follows the image, and that line has to be
    /// the same height as the blank line after a paragraph or an image sits flush against whatever
    /// comes next. `body_size * ESTIMATED_LINE_RATIO` is close and not equal — it is the estimate
    /// used for a chunk that has not been through the engine yet — so this asks the engine.
    ///
    /// A space rather than the empty string, because `layout` hands back no lines at all for nothing
    /// to shape; that is the MDEDIT-81 case `lay_out` repairs downstream.
    fn body_line_height(&mut self) -> f32 {
        let style = TextStyle::body(self.theme.body_size);
        let run = StyledRun::from_buffer(" ", style, ByteSpan::empty_at(ByteOffset::new(0)));
        self.text
            .layout(&[run], f32::MAX)
            .ok()
            .and_then(|layout| layout.lines.first().map(|line| line.height))
            .unwrap_or((self.theme.body_size * ESTIMATED_LINE_RATIO).max(1.0))
    }

    /// The directory a relative destination resolves against: the visible tab's.
    ///
    /// `None` for an untitled or generated buffer, which has no directory of its own — and a
    /// relative destination is then refused rather than resolved against the working directory,
    /// which is whatever the editor happened to be launched from.
    fn document_directory(&self) -> Option<PathBuf> {
        self.workspace
            .active_path()?
            .parent()
            .map(Path::to_path_buf)
    }

    /// How big the picture at `path` is, reading its header the first time only (MDEDIT-156).
    ///
    /// **The header, not the picture.** This is on the layout path, which is the thread that draws,
    /// and decoding a 4K PNG there costs more than the whole first-paint budget. So the size comes
    /// from `mdedit_image::measure` — a few dozen bytes — and the pixels are asked of the worker,
    /// which answers by waking the loop for another frame.
    ///
    /// `None` when the file cannot be read, is not a usable PNG, or declares a surface past the
    /// caps. Cached as [`PictureState::Unusable`] so a document naming a file that is not there
    /// costs one attempt rather than one per layout.
    fn picture_at(&mut self, path: PathBuf) -> Option<(u64, u32, u32)> {
        let key = PictureKey::of(path)?;
        let next = self.picture_ids.len() as u64;
        let id = *self.picture_ids.entry(key.clone()).or_insert(next);

        if let Some(known) = self.pictures.get(&key) {
            return known.size().map(|(width, height)| (id, width, height));
        }

        match mdedit_image::measure(&key.path) {
            Ok((width, height)) => {
                self.pictures.insert(
                    key.clone(),
                    PictureState::Known {
                        width,
                        height,
                        pixels: None,
                    },
                );
                self.decoder().request(key);
                Some((id, width, height))
            }
            Err(error) => {
                // Once, on the frame the file is first looked at, rather than every frame after —
                // which is what caching the refusal below buys, and a broken image in a document
                // being written would otherwise fill the log.
                crate::log_error!("{}: {error}", key.path.display());
                self.pictures.insert(key, PictureState::Unusable);
                None
            }
        }
    }

    /// The decode worker, started if this is the first picture to want one.
    fn decoder(&mut self) -> &Decoder {
        // Cloned out first: `get_or_insert_with` holds a mutable borrow of `self.decoder` for as
        // long as its closure runs, so the closure cannot reach `self` for the proxy.
        let wake = self.menu_proxy.clone();
        self.decoder.get_or_insert_with(|| Decoder::spawn(wake))
    }

    /// Takes whatever the decode worker has finished, without waiting (MDEDIT-156).
    ///
    /// `true` when something arrived, which is the caller's cue to draw another frame. Called on
    /// [`Wake::Pictures`], and harmless at any other time.
    pub fn absorb_pictures(&mut self) -> bool {
        let answers = self
            .decoder
            .as_mut()
            .map(Decoder::collect)
            .unwrap_or_default();
        self.take_pictures(answers)
    }

    /// Blocks for up to `patience` for the worker's next answer, then takes it (MDEDIT-156).
    ///
    /// **For a caller with no event loop to be woken by**, which means a headless test. It waits on
    /// the answer rather than on a clock, so a broken worker fails the test at the timeout instead
    /// of turning it flaky.
    ///
    /// **Waiting decides when to look and nothing else** — the answers then come through
    /// [`Self::absorb_pictures`], the same call the event loop makes. A test that reached the cache
    /// by its own route would leave the production path as the one nothing exercises.
    #[must_use]
    pub fn await_pictures(&mut self, patience: std::time::Duration) -> bool {
        if let Some(decoder) = self.decoder.as_mut() {
            decoder.wait_for_one(patience);
        }
        self.absorb_pictures()
    }

    /// Files the worker's answers into the picture cache.
    ///
    /// A key that is no longer in the cache is dropped rather than re-inserted: the only way that
    /// happens is the file changing under a running editor, and the answer is then about bytes
    /// nothing is asking to draw.
    fn take_pictures(
        &mut self,
        answers: Vec<(PictureKey, Result<Picture, mdedit_image::Undecodable>)>,
    ) -> bool {
        let mut arrived = false;
        for (key, decoded) in answers {
            match decoded {
                Ok(picture) => {
                    if let Some(PictureState::Known { pixels, .. }) = self.pictures.get_mut(&key) {
                        *pixels = Some(Rc::new(picture));
                        arrived = true;
                    }
                }
                Err(error) => {
                    // **Logged here rather than on the worker**, so one thread writes the log and
                    // the lines stay in an order that matches the frames.
                    crate::log_error!("{}: {error}", key.path.display());
                    // A header that measured and pixels that would not decode: a truncated file, or
                    // one being written while it is read. The row keeps the height the header gave
                    // it, because moving it now is the reflow this change exists to avoid.
                    if let Some(PictureState::Known { pixels, .. }) = self.pictures.get_mut(&key) {
                        *pixels = None;
                    }
                }
            }
        }
        arrived
    }

    /// Notes a destination this editor will not read.
    ///
    /// Not logged when it is merely absent: a path typed before the file exists is the ordinary
    /// state of a document being written, and saying so on every layout is noise. A destination
    /// **refused on a rule** is worth one line, because the alternative is a picture silently not
    /// appearing and no way to find out why.
    fn note_picture_refused(refused: &mdedit_image::Refused) {
        if !matches!(refused, mdedit_image::Refused::NotAFile(_)) {
            crate::log_error!("image not drawn: {refused}");
        }
    }

    /// Lays a table out as a grid: each cell on its own, then placed into columns (MDEDIT-126).
    ///
    /// `None` when there is nothing here that parses into cells, and the caller then lays the chunk
    /// out as ordinary lines. That is the honest answer for a malformed table, and it is also what
    /// keeps this from being a second renderer with its own idea of what a block is.
    ///
    /// **Plain mode never comes here.** `Ctrl+/` exists to show the bytes, and the bytes of a table
    /// are pipe rows; a grid there would be the mode failing to do the one thing it is for. It is
    /// also the answer to the widget's fifth question — the grid is a preview-mode thing only.
    ///
    /// Cells are laid out at an unbounded width, so nothing inside one wraps and a table wider than
    /// the window pans rather than reflows. That is a stated limit rather than an oversight: a
    /// column whose width depends on the viewport would re-lay the whole table on every resize, and
    /// the cache key does not carry a column width.
    fn lay_out_table(
        &mut self,
        chunk: blocks::Chunk,
        frame: &Frame,
    ) -> Option<(Layout, table::Grid)> {
        if self.decoration().mode == view::Mode::Plain {
            return None;
        }
        let cells = table::cells(self.document(), chunk.span);
        if cells.is_empty() {
            return None;
        }
        // **An empty cell still has a line.** A text engine handed nothing returns a layout with no
        // rows at all — the same answer that made a blank document draw no caret in MDEDIT-81 — and
        // a row of them came out six pixels tall, which is what a freshly inserted row looked like.
        // The height a cell contributes cannot depend on whether anyone has typed in it yet.
        let nominal = (self.theme.body_size * ESTIMATED_LINE_RATIO).max(1.0);

        let mut rows = Vec::with_capacity(cells.len());
        for row in &cells {
            let mut laid = Vec::with_capacity(row.len());
            for cell in row {
                let runs =
                    view::build_runs_in(self.document(), self.decoration(), *cell, &frame.text);
                let styled = view::as_styled(&runs);
                let mut layout = self.text.layout(&styled, f32::MAX).ok()?;
                if layout.lines.is_empty() {
                    layout.lines.push(mdedit_text::LayoutLine {
                        baseline: nominal,
                        height: nominal,
                        glyphs: Vec::new(),
                        rules: Vec::new(),
                    });
                }
                laid.push(layout);
            }
            rows.push(laid);
        }

        let body = chunk.span.slice(&frame.text);
        Some(table::assemble(&rows, &cells, body.ends_with('\n')))
    }

    /// A block quote, as runs of lines at one depth, each placed past its bars (MDEDIT-145).
    ///
    /// `None` in plain mode, where the markers are the text and nothing is indented, and for a
    /// quote chunk that turns out to have no quoted line in it at all — the honest answer is then
    /// to lay it out as ordinary lines, as `lay_out_list` does for a list with no marker.
    ///
    /// **A list inside the quote is laid out as a list** (MDEDIT-169), at the width the quote leaves,
    /// so a wrapped item hangs under its own text rather than under its bullet. Which part of a run
    /// is a list is the block tree's answer ([`list_pieces`]): a paragraph before it or after it in
    /// the same quote is not an item, and is not hung as one.
    fn lay_out_quote(
        &mut self,
        chunk: blocks::Chunk,
        frame: &Frame,
    ) -> Option<(Layout, Vec<quote::Band>, Vec<Nested>)> {
        if self.decoration().mode == view::Mode::Plain {
            return None;
        }
        let units = quote::units(self.document(), chunk.span, &frame.text);
        if units.iter().all(|unit| unit.depth == 0) {
            return None;
        }

        let mut runs = Vec::with_capacity(units.len());
        let mut breaks = Vec::new();
        for unit in units {
            let indent = quote::indent(unit.depth);
            let width = (frame.width - indent).max(1.0);
            for (piece, is_list) in list_pieces(self.document(), unit.span, &frame.text) {
                let listed = if is_list {
                    self.lay_out_items(piece, chunk.span.start(), width, &frame.text)
                } else {
                    None
                };
                let layout = if let Some((layout, _, nested)) = listed {
                    // The items' breaks hang from the items' text; the quote moves all of it right.
                    breaks.extend(nested.into_iter().map(|nested| Nested {
                        indent: nested.indent + indent,
                        ..nested
                    }));
                    layout
                } else {
                    // A break in a quote starts past its bars (MDEDIT-168), so the rule does not
                    // cross them.
                    breaks.extend(nested_breaks(
                        self.document(),
                        piece,
                        chunk.span.start(),
                        indent,
                    ));
                    self.lay_out_run(piece, width, &frame.text)?
                };
                runs.push((unit.depth, layout));
            }
        }

        let (layout, bands) = quote::assemble(&runs);
        Some((layout, bands, breaks))
    }

    /// `span` laid out as ordinary lines at `width`, never with no rows at all.
    ///
    /// **A run can have nothing left to shape.** `>` alone on the last line of the file is all marker
    /// and no newline, and a text engine handed nothing answers with no rows — the MDEDIT-81 shape,
    /// one run down. The whole-chunk repair only fires when the *chunk* is empty, so a quote ending
    /// that way would lose a line the caret can reach.
    fn lay_out_run(&mut self, span: ByteSpan, width: f32, text: &str) -> Option<Layout> {
        let styled = view::build_runs_in(self.document(), self.decoration(), span, text);
        let mut layout = self.text.layout(&view::as_styled(&styled), width).ok()?;
        if layout.lines.is_empty() {
            let height = (self.theme.body_size * ESTIMATED_LINE_RATIO).max(1.0);
            layout.lines.push(mdedit_text::LayoutLine {
                baseline: height,
                height,
                glyphs: Vec::new(),
                rules: Vec::new(),
            });
        }
        Some(layout)
    }

    /// Lays a list out item by item, so a wrapped item hangs under its text (MDEDIT-147).
    ///
    /// `None` when the chunk holds no list marker after all, and the caller then lays it out as
    /// ordinary lines — the same bargain [`Self::lay_out_table`] strikes with a malformed table.
    ///
    /// **Plain mode never comes here**, for the reason a table does not: `Ctrl+/` exists to show the
    /// bytes, and the indent hangs off a marker that plain mode draws in full. There is nothing to
    /// substitute, so there is nothing to hang from.
    fn lay_out_list(
        &mut self,
        chunk: blocks::Chunk,
        frame: &Frame,
    ) -> Option<(Layout, Vec<quote::Band>, Vec<Nested>)> {
        if self.decoration().mode == view::Mode::Plain {
            return None;
        }
        self.lay_out_items(chunk.span, chunk.span.start(), frame.width, &frame.text)
    }

    /// The items in `span`, each hung under its own text at `width`, and stacked.
    ///
    /// Two layout calls an item — the marker alone to measure the indent, then the rest at the width
    /// it leaves. `list`'s own docs say why one call cannot do it.
    ///
    /// **A quote inside an item is laid out as a quote** (MDEDIT-169): its lines are cut into runs by
    /// depth, placed past their bars within the item's text, and the bars come back with the layout
    /// to be drawn under the item. Depth is measured from the item's first line, so it is a quote
    /// *inside* the item that counts. That is also what keeps this one level deep: `lay_out_quote`
    /// only hands over lines of a single depth, so a list inside a quote never finds one here.
    fn lay_out_items(
        &mut self,
        span: ByteSpan,
        chunk_start: ByteOffset,
        width: f32,
        text: &str,
    ) -> Option<(Layout, Vec<quote::Band>, Vec<Nested>)> {
        let units = list::units(self.document(), span, text);
        if units.is_empty() {
            return None;
        }

        let mut items = Vec::with_capacity(units.len());
        let mut breaks = Vec::new();
        for unit in units {
            let depths = quote::units(self.document(), unit, text);
            let base = depths.first().map_or(0, |run| run.depth);
            let has_quote = depths.iter().any(|run| run.depth != base);
            // The part holding the marker: the whole item, or its lines before the quote starts.
            let head = if has_quote { depths[0].span } else { unit };

            let runs = view::build_runs_in(self.document(), self.decoration(), head, text);
            // The substituted marker is the run the indent is measured from, and it is always the
            // first: `units` cuts at the marker, so nothing precedes it inside the unit. A task's box
            // can be first too, when the bullet it replaced had no indent to leave behind, and it is
            // content rather than the marker (MDEDIT-142).
            let split = usize::from(
                runs.first()
                    .is_some_and(|run| run.substituted && !view::is_task_box(run)),
            );
            let (marker, rest) = runs.split_at(split);

            let marker = self.text.layout(&view::as_styled(marker), f32::MAX).ok()?;
            let indent = list::width_of(&marker);
            let width = (width - indent).max(1.0);
            let first = self.text.layout(&view::as_styled(rest), width).ok()?;
            // A break in an item hangs where the item's text does, not under its bullet (MDEDIT-168).
            breaks.extend(nested_breaks(self.document(), head, chunk_start, indent));

            if !has_quote {
                items.push(list::Item {
                    marker,
                    text: first,
                    bands: Vec::new(),
                });
                continue;
            }
            let mut stacked = vec![(0, first)];
            for run in &depths[1..] {
                let depth = run.depth.saturating_sub(base);
                let inset = quote::indent(depth);
                stacked.push((
                    depth,
                    self.lay_out_run(run.span, (width - inset).max(1.0), text)?,
                ));
                breaks.extend(nested_breaks(
                    self.document(),
                    run.span,
                    chunk_start,
                    indent + inset,
                ));
            }
            let (text_layout, bands) = quote::assemble(&stacked);
            items.push(list::Item {
                marker,
                text: text_layout,
                bands,
            });
        }

        let (layout, bands) = list::assemble(&items);
        Some((layout, bands, breaks))
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
        let selection = self.document().selection();
        let caret = selection.head();
        let index = frame
            .chunks
            .iter()
            .position(|chunk| chunk.span.start() <= caret && caret < chunk.span.end())
            .or_else(|| frame.chunks.len().checked_sub(1))?;
        let chunk = frame.chunks[index];

        let cached = frame
            .fresh_at(index)
            .or_else(|| self.layouts.get(&frame.keys[index]).map(Rc::clone))?;

        // **Where the caret is drawn and where a click lands must be the same answer**, so both come
        // from `hit`. This counted its own newlines until MDEDIT-126, which was a third copy of the
        // rule that a visual row is a source line — and a grid is the one layout that breaks it, so
        // the caret sat on the trailing empty row for every table row below the header.
        let target = cached.as_hit(chunk, &frame.text);
        let (row, x) = cached.place_of(&target, caret, selection.affinity())?;
        let line = cached.layout.lines.get(row)?;
        let top: f32 = cached.layout.lines[..row].iter().map(|l| l.height).sum();

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
        // The `occurrence`-th chunk with this content, not the first — see [`Anchor::occurrence`].
        // A document whose block count changed above the anchor may not have that many any more, and
        // then there is nothing to correct against: leaving the offset alone is what this function
        // already did when it could not find the content at all.
        let Some(index) = frame
            .keys
            .iter()
            .enumerate()
            .filter(|(_, key)| key.content == anchor.content)
            .nth(anchor.occurrence)
            .map(|(index, _)| index)
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
            occurrence: frame.keys[..index]
                .iter()
                .filter(|earlier| earlier.content == key.content)
                .count(),
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

        // Gathered before the renderer is borrowed, because it lives in `self` and reading the
        // picture cache through `&self` while holding `&mut self.renderer` is two borrows. Both are
        // cheap: a `Drawn` is `Copy` and a source is an `Rc` clone, and there are usually none.
        let pictures = self.drawn_pictures.clone();
        let sources = self.picture_sources(&pictures);

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if self.atlas.is_dirty() {
            renderer.upload_atlas(&self.atlas);
            self.atlas.mark_clean();
        }
        renderer.upload_pictures(&sources);
        renderer.draw(&quads, &pictures, self.theme.background);
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
            Wake::Pictures => {
                // Only if something actually landed: a wake whose answers a previous one already
                // collected must not cost a frame (MDEDIT-156).
                if self.absorb_pictures()
                    && let Some(window) = self.window.as_ref()
                {
                    window.request_redraw();
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

        // Whichever deadlines have come due, and only those. This used to be an unconditional
        // `set_control_flow(Wait)` followed by `on_idle`, which was right while the reconcile timer
        // was the only thing on the clock — and would have cancelled a pending dwell the first time
        // it was not. See `next_wake`.
        let now = Instant::now();
        let mut redraw = false;
        if self.reconcile_at.is_some_and(|at| at <= now) {
            self.reconcile_at = None;
            redraw |= self.on_idle();
        }
        if self.dwell_at.is_some_and(|at| at <= now) {
            self.dwell_at = None;
            redraw |= self.on_dwell();
        }
        self.reschedule(event_loop);

        if redraw && let Some(window) = self.window.as_ref() {
            window.request_redraw();
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
            // The value it carries is deliberately ignored. See `follow_system_appearance`.
            WindowEvent::ThemeChanged(_) => self.follow_system_appearance(),
            WindowEvent::Resized(size) => self.on_resized(size),
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
                // The shape first, and before the drag arms inside `on_pointer_moved` return early:
                // a drag holds its own shape, and `pointer_at` is what knows that.
                let _ = self.move_pointer(point.0, point.1);
                self.on_pointer_moved(point);
            }
            // New with MDEDIT-85, and the window handler had no arm for it before: without one, a
            // tooltip raised on the last button before the pointer left the window stays up over an
            // editor nobody is pointing at.
            WindowEvent::CursorLeft { .. } => {
                if self.dismiss_panels()
                    && let Some(window) = self.window.as_ref()
                {
                    window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.dismiss_panels();
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
                    MouseButton::Middle => self.middle_click(x, y),
                    _ => false,
                };
                if changed && let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::Ime(ime) => {
                if self.handle_ime(ime)
                    && let Some(window) = self.window.as_ref()
                {
                    window.request_redraw();
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
                //
                // Recorded rather than set straight into `ControlFlow`, since MDEDIT-85: a dwell may
                // be pending and come sooner. See `next_wake`.
                self.schedule_reconcile();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }

        // **Every arm, unconditionally, and that is the point of it being here rather than in the
        // arms that arm a deadline.** Two of them do — a keystroke records `reconcile_at`, a pointer
        // move records `dwell_at` — and a deadline that nothing turns into a `ControlFlow` is a
        // deadline that never arrives: `new_events` only runs on `ResumeTimeReached`, so the loop
        // sleeps on `Wait` for ever and neither the idle beat nor the dwell fires.
        //
        // MDEDIT-85 shipped that bug to the window. The keystroke arm used to call
        // `set_control_flow` itself and stopped when the deadline became a field; nothing replaced
        // it, so the whole-document re-parse, the word count, the recovery snapshots and the theme
        // check all went quiet, and no tooltip ever appeared. **No test here could see it** — the
        // handler needs an `ActiveEventLoop`, which cannot be constructed, and the tooltip tests
        // drive `on_dwell` directly. Found by opening the editor and resting on a button.
        self.reschedule(event_loop);
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
    #[allow(
        clippy::too_many_arguments,
        reason = "a cache key is the sum of what it identifies"
    )]
    fn of(
        chunk: &blocks::Chunk,
        body: &str,
        last: bool,
        width: f32,
        mode: view::Mode,
        language: &'static str,
    ) -> Self {
        let mut hasher = DefaultHasher::new();
        chunk.kind.hash(&mut hasher);
        body.hash(&mut hasher);
        // **The same bytes lay out differently in two languages**, so the language is part of what
        // identifies a layout — for the reason the mode above is, and following it rather than
        // clearing the cache when a document is re-languaged (MDEDIT-93). Saving an untitled buffer
        // as `data.json` changes no byte and every colour, and the alternative is a clear at each of
        // the paths that could re-language a tab, which is a list somebody forgets to add to.
        //
        // **No pair of today's languages can actually collide here**, and that is worth saying
        // rather than implying: `chunk.kind` is above, a plain document has no blocks at all, and
        // every other language produces some — so the hash already differs. Removing this line
        // breaks no test. It is here because "the key identifies what was laid out" is the property,
        // and relying on a second field to happen to differ is relying on an accident.
        //
        // The name rather than the pointer: `&'static dyn Language` is a fat pointer whose vtable
        // address is not guaranteed unique, and a language's name is already required to identify it
        // to a status bar.
        language.hash(&mut hasher);
        Self {
            content: hasher.finish(),
            width: width.to_bits(),
            last,
            mode,
        }
    }
}

/// A picture as a chunk draws it: which one, and how big after fitting the column.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fitted {
    id: u64,
    /// Logical pixels, so a denser display draws the same picture larger rather than smaller.
    width: f32,
    height: f32,
    /// How tall the blank line after the image is — see [`picture_layout`].
    after: f32,
    /// The `![alt](dest)` this stands for, **relative to its chunk's start** (MDEDIT-152).
    ///
    /// Relative because `Cached` is keyed by content: the same bytes elsewhere in the document reuse
    /// this entry, so an absolute span would be right for the first occurrence and wrong for every
    /// other. The same reason `Cached::base` exists, and it is rebased the same way.
    span: ByteSpan,
}

/// A thematic break inside a quote or a list item, as its container laid it out (MDEDIT-168).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Nested {
    /// Where the break starts, **from the start of its chunk**. Not a buffer offset: the layout
    /// cache is keyed by content, so the same quote further down the document reuses this entry,
    /// and only an offset relative to the chunk still means the same bytes there.
    at: usize,
    /// How far from the text's left edge the rule starts: past the quote's bars, or under the list
    /// item's text rather than its bullet.
    indent: f32,
}

/// The thematic breaks inside `span`, each as its offset from `chunk_start` with `indent`.
///
/// From the block tree, as `quote_depth` is and for the reason it gives: the `---` of a break
/// inside `> ` arrives as one marker with the quote's, and the tree is what says which is which.
fn nested_breaks(
    doc: &Document,
    span: ByteSpan,
    chunk_start: ByteOffset,
    indent: f32,
) -> Vec<Nested> {
    let blocks = doc.blocks();
    let first = blocks.partition_point(|block| block.span().start() < span.start());
    blocks[first..]
        .iter()
        .take_while(|block| block.span().start() < span.end())
        .filter(|block| block.kind() == BlockKind::ThematicBreak)
        .map(|block| Nested {
            at: block.span().start().get() - chunk_start.get(),
            indent,
        })
        .collect()
}

/// `span`, a run of whole lines, cut where a list inside it starts and ends (MDEDIT-169).
///
/// Each piece is whole lines, marked `true` if it is a list. From the block tree rather than the
/// markers, because the markers cannot say where a list *ends*: a paragraph after it in the same
/// quote has no marker of its own, and `list::units` would attach it to the last item and hang it
/// there.
///
/// **A list's span starts and ends mid-line** inside a quote, and both are cut back to a line
/// start. It starts after the `> ` of its own first line, and it ends after the `> ` of the line
/// *following* it — where the next block's content begins. Rounding that end forward pulled the next
/// paragraph into the list, which is the defect this function exists to avoid. The one end rounded
/// forward is a list's last line with no newline, at the end of the file.
fn list_pieces(doc: &Document, span: ByteSpan, text: &str) -> Vec<(ByteSpan, bool)> {
    let line_start = |at: usize| text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let line_end = |at: usize, start: usize| {
        if at == 0 || text.as_bytes()[at - 1] == b'\n' {
            at
        } else if line_start(at) > start {
            line_start(at)
        } else {
            text[at..]
                .find('\n')
                .map_or(text.len(), |newline| at + newline + 1)
        }
    };

    let mut pieces = Vec::new();
    let mut at = span.start().get();
    let blocks = doc.blocks();
    let first = blocks.partition_point(|block| block.span().start() < span.start());
    for block in blocks[first..]
        .iter()
        .take_while(|block| block.span().start() < span.end())
    {
        // Outermost lists only: a nested one starts inside the piece already taken.
        if block.kind() != BlockKind::List || block.span().start().get() < at {
            continue;
        }
        let start = line_start(block.span().start().get()).max(at);
        let end = line_end(block.span().end().get(), start).min(span.end().get());
        if start > at {
            pieces.push((
                ByteSpan::new(ByteOffset::new(at), ByteOffset::new(start)),
                false,
            ));
        }
        if end > start {
            pieces.push((
                ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
                true,
            ));
        }
        at = end.max(start);
    }
    if at < span.end().get() || pieces.is_empty() {
        pieces.push((ByteSpan::new(ByteOffset::new(at), span.end()), false));
    }
    pieces
}

/// A chunk that has been through the text engine.
#[derive(Debug)]
struct Cached {
    layout: Layout,
    /// The boxes a table is ruled with, and the bytes each of its rows covers (MDEDIT-126).
    ///
    /// `None` for every other chunk, and that `None` is what says "this is not a table" to the two
    /// things that need to know: `hit`, which otherwise finds a row's bytes by counting newlines,
    /// and `draw_chunks`, which otherwise rules nothing.
    grid: Option<table::Grid>,
    /// The picture this chunk is, if it is one (MDEDIT-146).
    ///
    /// `None` for every chunk that is not a lone image — which, like `grid`'s `None`, is what says
    /// "draw this as lines" to `draw_chunks`. A chunk with a picture has one empty layout line whose
    /// height is the picture's, so everything keyed off rows goes on working.
    picture: Option<Fitted>,
    /// Where a block quote's bars go, and how many at each height (MDEDIT-145).
    ///
    /// Empty for every chunk that is not a quote, and empty is what says "no bars" to
    /// `draw_quote` — the same role `grid`'s and `picture`'s `None` play.
    quote: Vec<quote::Band>,
    /// The thematic breaks nested inside this quote or list, and how far each one's rule is indented
    /// (MDEDIT-168).
    ///
    /// Empty for every other chunk. A top-level break is a chunk of its own and is drawn from the
    /// chunk's kind; one inside a container is a block *within* a chunk, and only the layout that
    /// placed its container knows where the container's text starts.
    breaks: Vec<Nested>,
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
    #[allow(
        clippy::too_many_arguments,
        reason = "what a cached chunk is made of, and each of the six is needed to make it"
    )]
    fn new(
        layout: Layout,
        grid: Option<table::Grid>,
        picture: Option<Fitted>,
        chunk: &blocks::Chunk,
        body: &str,
        is_last: bool,
    ) -> Self {
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
            grid,
            picture,
            quote: Vec::new(),
            breaks: Vec::new(),
            base: chunk.span.start().get(),
            drops_last,
            height,
            // A picture is as wide as it is drawn. Without this the pan clamp measures its glyphs,
            // of which it has none, and a picture wider than the window could not be scrolled to.
            width: picture.map_or(width, |fitted| fitted.width),
        }
    }

    /// Where `offset` sits on this chunk — [`hit::place_of`], with one correction for block quotes.
    ///
    /// **A blank quoted row has no glyph to hang the caret on** (MDEDIT-145). `hit::place_of` then
    /// answers x 0, which for a quote is its first bar, and the first keystroke jumped the caret right
    /// to where the text goes. A row with no glyphs inside a quote's band goes to that band's indent
    /// instead — the same repair `hit` makes for an empty table cell, made here because the bands
    /// live on `Cached` and every caller of this already holds one.
    ///
    /// Both callers — where the caret is drawn and where up and down aim from — come through here,
    /// so they cannot disagree about it.
    fn place_of(
        &self,
        target: &hit::Chunk<'_>,
        offset: ByteOffset,
        affinity: Affinity,
    ) -> Option<(usize, f32)> {
        let (row, x) = hit::place_of(target, self.drawn_lines(), offset, affinity)?;
        let blank = self
            .layout
            .lines
            .get(row)
            .is_some_and(|line| line.glyphs.is_empty());
        if !blank {
            return Some((row, x));
        }
        let top = hit::row_top(target, row);
        let indent = self
            .quote
            .iter()
            .find(|band| top >= band.top && top < band.top + band.height)
            .map_or(x, |band| band.x + quote::indent(band.depth));
        Some((row, indent))
    }

    /// This chunk as [`hit`] sees it: its layout, its bytes, and where both sit.
    ///
    /// Six call sites built this literal, identically, which is one more copy than a struct with a
    /// rule in it should have — and the rule is exactly the one MDEDIT-183 found in three places at
    /// once. `text` is the document's, since a chunk span indexes into it.
    fn as_hit<'a>(&'a self, chunk: blocks::Chunk, text: &'a str) -> hit::Chunk<'a> {
        hit::Chunk {
            layout: &self.layout,
            text: chunk.span.slice(text),
            start: chunk.span.start(),
            base: ByteOffset::new(self.base),
            grid: self.grid.as_ref(),
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
    /// How many chunks with this same content come before it.
    ///
    /// **The disambiguator.** `ChunkKey::of` hashes a chunk's kind and its bytes, so identical
    /// top-level blocks share a content hash — repeated `---` breaks, a run of duplicated
    /// paragraphs, a log of similar entries. Resolving by first match snapped the viewport toward
    /// the first of them on any redraw, from the fortieth of fifty identical entries to the first
    /// (MDEDIT-108).
    ///
    /// An occurrence count rather than an index into `frame.keys`, because the index is what the
    /// height corrections this whole mechanism exists to absorb are allowed to move.
    occurrence: usize,
    /// How far into that chunk the viewport's top edge sits.
    into_chunk: f32,
    /// The scroll offset when this was taken. If it has changed, the user has scrolled and the
    /// anchor is stale.
    offset: f32,
}

/// The layout a picture's chunk gets: one empty line as tall as the picture (MDEDIT-146).
///
/// **No glyphs, and that is the whole trick.** MDEDIT-81 already made "one `LayoutLine` with no
/// glyphs" the shape of an empty document, so every consumer downstream — `blocks::stack`, the
/// scrollbar, the anchor, `hit`, the caret — handles it without a branch. A picture is a tall empty
/// line, and the rectangle over it is drawn separately.
///
/// The trailing line is the same bargain `table::assemble` strikes: a chunk whose bytes end in a
/// newline gets one from the text engine and `Cached` drops it again through
/// `blocks::drops_last_line`, so a layout assembled without one loses the picture's own row instead.
///
/// **The picture replaces the chunk's first line and no others.** A chunk laid out linearly gets one
/// line per newline plus one, and `Cached` then drops the last through `blocks::drops_last_line`; a
/// paragraph's `X\n\n` is therefore three lines and two drawn rows — the text and the blank line
/// after it. So this shape has to be the same, with the first row as tall as the picture and the
/// rest an ordinary line each.
///
/// Getting that wrong is invisible in every obvious way. The first attempt emitted one trailing line
/// of height zero, which made the chunk exactly picture-tall: the blank line every other block gets
/// after it disappeared and an image sat flush against the next paragraph, while the picture's own
/// rectangle, its size and its position were all still correct. See
/// `a_picture_is_followed_by_the_same_blank_line_a_paragraph_is`.
fn picture_layout(fitted: Fitted, body: &str) -> Layout {
    let mut lines = vec![mdedit_text::LayoutLine {
        baseline: fitted.height,
        height: fitted.height,
        glyphs: Vec::new(),
        rules: Vec::new(),
    }];
    let mut baseline = fitted.height;
    for _ in body.bytes().filter(|byte| *byte == b'\n') {
        baseline += fitted.after;
        lines.push(mdedit_text::LayoutLine {
            baseline,
            height: fitted.after,
            glyphs: Vec::new(),
            rules: Vec::new(),
        });
    }
    Layout { lines }
}

/// Whether two spans overlap or meet. A caret at a chunk's edge belongs to both sides of it.
fn touches(span: ByteSpan, other: ByteSpan) -> bool {
    span.start() <= other.end() && other.start() <= span.end()
}
