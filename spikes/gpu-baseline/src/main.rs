//! MDEDIT-19: what does an empty window cost?
//!
//! The editor's baseline RSS is roughly 345 MB against a 50 MB target, and about 8 MB of that is
//! anything this project explicitly allocates. Before optimising, it is worth knowing how much of
//! the rest is the price of admission for winit plus wgpu on this hardware and how much is ours.
//!
//! So: the same window size the editor uses, a surface, a device, and a render pass that clears to
//! a colour and draws nothing. No text, no atlas, no pipeline, no buffers.
//!
//! Set `BACKENDS=dx12` to restrict the instance to DX12; anything else uses wgpu's default, which
//! enables every backend it was compiled with. Measure the process externally — a program that
//! reports its own memory has to allocate to do it.
//!
//! ```text
//! cargo run --release              # all backends
//! BACKENDS=dx12 cargo run --release
//! ```

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

#[derive(Default)]
struct Probe {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
}

impl ApplicationHandler for Probe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            // Matched to the editor so the comparison is like for like.
            .with_title("gpu-baseline-probe")
            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 700.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("a window on a desktop platform"),
        );

        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        if std::env::var("BACKENDS").as_deref() == Ok("dx12") {
            descriptor.backends = wgpu::Backends::DX12;
        }
        let instance = wgpu::Instance::new(descriptor);

        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("a surface for the window");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("an adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("baseline"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("a device");

        let size = window.inner_size();
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("a surface configuration");
        surface.configure(&device, &config);

        self.gpu = Some(Gpu {
            surface,
            device,
            queue,
        });
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                let Some(gpu) = self.gpu.as_ref() else { return };
                let (wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame)) = gpu.surface.get_current_texture()
                else {
                    return;
                };

                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                }));
                gpu.queue.submit(Some(encoder.finish()));
                gpu.queue.present(frame);
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("an event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut Probe::default())
        .expect("the event loop to run");
}
