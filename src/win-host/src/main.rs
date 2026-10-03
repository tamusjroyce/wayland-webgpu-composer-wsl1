//! Native Windows WebGPU host (binary entry point).
//!
//! Opens a single scalable window backed by a WebGPU (`wgpu`) context, maps the shared
//! framebuffer produced by the WSL1 compositor, and presents it scaled to the window.
//! Window input and resize events are forwarded to the compositor over a `127.0.0.1`
//! control channel. All reusable logic lives in the `win_host` library crate.

use bridge_protocol::{ClientMessage, ServerMessage};
use memmap2::Mmap;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, MouseScrollDelta, WindowEvent};
use winit::event_loop::EventLoopBuilder;
use winit::keyboard::PhysicalKey;
use winit::window::WindowBuilder;

use win_host::gpu::GpuState;
use win_host::{args, bridge, input, shared, UserEvent};

fn main() {
    // Default to our own info logging but silence wgpu/naga's very verbose internal logs.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,win_host=info"),
    )
    .init();

    let parsed = args::parse_args(std::env::args().skip(1));
    if parsed.help {
        println!("{}", args::USAGE);
        return;
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event()
        .build()
        .expect("create event loop");
    let window = std::sync::Arc::new(
        WindowBuilder::new()
            .with_title("WSL1 Wayland → WebGPU")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .with_resizable(true)
            .build(&event_loop)
            .expect("create window"),
    );

    let mut gpu = GpuState::new(window.clone());

    let proxy = event_loop.create_proxy();
    let tx = bridge::spawn(args::candidate_addrs(&parsed.host), proxy);

    let mut shm: Option<Mmap> = None;

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(winit::event_loop::ControlFlow::Wait);
            match event {
                Event::UserEvent(UserEvent::Server(msg)) => {
                    handle_server_message(msg, &mut shm, &mut gpu);
                }
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::CloseRequested => {
                        let _ = tx.send(ClientMessage::Close);
                        elwt.exit();
                    }
                    WindowEvent::Resized(size) => {
                        gpu.resize(size.width, size.height);
                        let _ = tx.send(ClientMessage::Resize {
                            width: size.width,
                            height: size.height,
                        });
                        gpu.window().request_redraw();
                    }
                    WindowEvent::RedrawRequested => match gpu.render() {
                        Ok(()) => {}
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            let (w, h) = gpu.window_size();
                            gpu.resize(w, h);
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                        Err(e) => log::warn!("render error: {e:?}"),
                    },
                    WindowEvent::CursorMoved { position, .. } => {
                        if let Some((x, y)) =
                            input::map_pointer(position, gpu.window_size(), gpu.tex_size())
                        {
                            let _ = tx.send(ClientMessage::PointerMotion { x, y });
                        }
                    }
                    WindowEvent::MouseInput { state, button, .. } => {
                        if let Some(code) = input::mouse_button_to_evdev(button) {
                            let _ = tx.send(ClientMessage::PointerButton {
                                button: code,
                                pressed: state == ElementState::Pressed,
                            });
                        }
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let (h, v) = match delta {
                            MouseScrollDelta::LineDelta(x, y) => (x * 15.0, y * 15.0),
                            MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                        };
                        // Wayland axis: positive = down/right; invert the wheel's vertical.
                        let _ = tx.send(ClientMessage::PointerAxis {
                            horizontal: h,
                            vertical: -v,
                        });
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        if let PhysicalKey::Code(code) = event.physical_key {
                            if let Some(evdev) = input::keycode_to_evdev(code) {
                                let _ = tx.send(ClientMessage::Key {
                                    keycode: evdev,
                                    pressed: event.state == ElementState::Pressed,
                                });
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        })
        .expect("event loop");
}

fn handle_server_message(msg: ServerMessage, shm: &mut Option<Mmap>, gpu: &mut GpuState) {
    match msg {
        ServerMessage::FrameConfig {
            width,
            height,
            shm_size,
            host_path,
        } => {
            log::info!("frame config: {width}x{height}, {shm_size} bytes, path={host_path}");
            match shared::open_shared(&host_path, shm_size) {
                Some(map) => *shm = Some(map),
                None => log::error!("failed to map shared framebuffer at {host_path}"),
            }
        }
        ServerMessage::FrameReady {
            seq: _,
            width,
            height,
        } => {
            if let Some(map) = shm.as_ref() {
                if let Some(reader) = bridge_protocol::SharedFramebufferReader::new(&map[..]) {
                    let pixels = reader.active_pixels();
                    gpu.upload(width, height, pixels);
                    gpu.window().request_redraw();
                }
            }
        }
    }
}
