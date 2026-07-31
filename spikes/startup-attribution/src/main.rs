//! MDEDIT-20: where do the ~300 ms before the editor's first frame go?
//!
//! MDEDIT-18 measured launch-to-first-paint at 313 ms for a five-byte file and 571 ms for a 245 KB
//! one, of which loading and parsing the document was 9–10 ms. So roughly 300 ms is fixed cost with
//! nothing to do with the file, and this attributes it.
//!
//! Deliberately a spike rather than instrumentation in the editor: the question is what winit and
//! wgpu cost, and the editor's own work is 10 ms of 571. Doing it here also means the numbers are
//! not confounded by anything the editor does, and it stays re-runnable when wgpu is next bumped.
//!
//! It brings up the same window and the same pipeline the editor does — including a shader compile,
//! which `gpu-baseline` deliberately omits because it was measuring memory rather than time.
//!
//! ```text
//! cargo run --release
//! BACKENDS=dx12 cargo run --release   # restrict the instance to DX12
//! ```

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// The same shader shape the editor compiles: a textured, tinted quad.
const SHADER: &str = r"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(index & 1u) * 2 - 1);
    let y = f32(i32(index >> 1u) * 2 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    out.colour = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    return in.colour;
}
";

static LAUNCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn elapsed_since_launch() -> f64 {
    LAUNCH
        .get()
        .map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0)
}

/// Reports how long `label` took, and how far into the run it finished.
fn step<T>(label: &str, work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = work();
    println!(
        "{label:<24} {:7.1} ms   (done at {:7.1} ms)",
        start.elapsed().as_secs_f64() * 1000.0,
        elapsed_since_launch()
    );
    value
}

#[derive(Default)]
struct Probe {
    window: Option<Arc<Window>>,
    done: bool,
}

impl ApplicationHandler for Probe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("startup-attribution-probe")
            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 700.0));
        let window = step("create_window", || {
            Arc::new(
                event_loop
                    .create_window(attributes)
                    .expect("a window on a desktop platform"),
            )
        });

        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        if std::env::var("BACKENDS").as_deref() == Ok("dx12") {
            descriptor.backends = wgpu::Backends::DX12;
        }
        let instance = step("Instance::new", || wgpu::Instance::new(descriptor));

        let surface = step("create_surface", || {
            instance
                .create_surface(Arc::clone(&window))
                .expect("a surface for the window")
        });

        let adapter = step("request_adapter", || {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: match std::env::var("POWER").as_deref() {
                    Ok("high") => wgpu::PowerPreference::HighPerformance,
                    Ok("none") => wgpu::PowerPreference::None,
                    _ => wgpu::PowerPreference::LowPower,
                },
                compatible_surface: Some(&surface),
                ..Default::default()
            }))
            .expect("an adapter")
        });

        let (device, queue) = step("request_device", || {
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("probe"),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .expect("a device")
        });

        let size = window.inner_size();
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("a surface configuration");
        step("surface.configure", || surface.configure(&device, &config));

        let shader = step("create_shader_module", || {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("probe"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            })
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("probe"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let pipeline = step("create_render_pipeline", || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("probe"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    targets: &[Some(config.format.into())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        });

        // One frame, so "first paint" means the same thing it does in the editor.
        step("first frame", || {
            let (wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame)) = surface.get_current_texture()
            else {
                return;
            };
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.12,
                                g: 0.12,
                                b: 0.12,
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
                pass.set_pipeline(&pipeline);
                pass.draw(0..4, 0..1);
            }
            queue.submit(Some(encoder.finish()));
            queue.present(frame);
        });

        println!("----------------------------------------------");
        println!("launch -> first paint    {:7.1} ms", elapsed_since_launch());

        self.window = Some(window);
        self.done = true;
        event_loop.exit();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if event == WindowEvent::CloseRequested {
            event_loop.exit();
        }
    }
}

fn main() {
    let _ = LAUNCH.set(Instant::now());
    let event_loop = step("EventLoop::new", || {
        EventLoop::new().expect("an event loop")
    });
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut Probe::default())
        .expect("the event loop to run");
}
