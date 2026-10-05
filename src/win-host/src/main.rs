//! Native Windows WebGPU host (binary entry point).
//!
//! Opens a single scalable window backed by a WebGPU (`wgpu`) context, maps the shared
//! framebuffer produced by the WSL1 compositor, and presents it scaled to the window.
//! Window input and resize events are forwarded to the compositor over a `127.0.0.1`
//! control channel. All reusable logic lives in the `win_host` library crate.

use bridge_protocol::{Backend, ClientMessage, ServerMessage};
use memmap2::Mmap;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, MouseScrollDelta, WindowEvent};
use winit::event_loop::EventLoopBuilder;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, WindowBuilder};

use win_host::backend::{self, RenderOutcome, Renderer};
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
    // Title reflects the actual host renderer so it is clear which backend is live.
    let backend_label = match parsed.backend {
        Backend::Webgpu => "WebGPU (wgpu)",
        Backend::Vulkan => "Vulkan (ash + gpu-allocator)",
    };
    let window = std::sync::Arc::new(
        WindowBuilder::new()
            .with_title(format!(
                "WSL1 Wayland → {backend_label}  (F11 fullscreen · F10 minimize)"
            ))
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .with_decorations(true)
            .with_resizable(true)
            .build(&event_loop)
            .expect("create window"),
    );

    let mut gpu: Box<dyn Renderer> = backend::create(parsed.backend, window.clone());

    let proxy = event_loop.create_proxy();
    let tx = bridge::spawn(args::candidate_addrs(&parsed.host), proxy);

    let mut shm: Option<Mmap> = None;
    // The backend actually in use. Starts from `--backend`; the compositor handshake may
    // override it so a single parameter on either side stays consistent.
    let mut active_backend = parsed.backend;
    // Output framebuffer size from the handshake, needed to size a GpuScene composite.
    let mut fb_size = (0u32, 0u32);
    // Host-side window state (F11 toggles borderless fullscreen).
    let mut fullscreen = false;

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(winit::event_loop::ControlFlow::Wait);
            match event {
                Event::UserEvent(UserEvent::Server(msg)) => {
                    handle_server_message(
                        msg,
                        &mut shm,
                        gpu.as_mut(),
                        &mut active_backend,
                        parsed.backend,
                        &mut fb_size,
                    );
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
                        RenderOutcome::Presented | RenderOutcome::Error => {}
                        RenderOutcome::Lost => {
                            let (w, h) = gpu.window_size();
                            gpu.resize(w, h);
                        }
                        RenderOutcome::OutOfMemory => elwt.exit(),
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
                        // Host-side window controls, handled locally (not forwarded to sway).
                        if event.state == ElementState::Pressed {
                            match event.physical_key {
                                PhysicalKey::Code(KeyCode::F11) => {
                                    fullscreen = !fullscreen;
                                    gpu.window().set_fullscreen(
                                        fullscreen.then(|| Fullscreen::Borderless(None)),
                                    );
                                    return;
                                }
                                PhysicalKey::Code(KeyCode::F10) => {
                                    gpu.window().set_minimized(true);
                                    return;
                                }
                                _ => {}
                            }
                        }
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

fn handle_server_message(
    msg: ServerMessage,
    shm: &mut Option<Mmap>,
    gpu: &mut dyn Renderer,
    active_backend: &mut Backend,
    requested_backend: Backend,
    fb_size: &mut (u32, u32),
) {
    match msg {
        ServerMessage::FrameConfig {
            width,
            height,
            shm_size,
            backend,
            host_path,
        } => {
            log::info!(
                "frame config: {width}x{height}, {shm_size} bytes, backend={}, path={host_path}",
                backend.name()
            );
            *fb_size = (width, height);
            if backend != requested_backend {
                log::warn!(
                    "compositor advertised backend '{}' but host was launched with '{}'; \
                     the compositor's choice is authoritative. Restart win-host with \
                     --backend {} for a matching host renderer.",
                    backend.name(),
                    requested_backend.name(),
                    backend.name()
                );
            }
            *active_backend = backend;
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
        // GPU-composite scene path: CPU-composite the pool quads (back-to-front) into a frame
        // and upload it through the normal present path. Backend-agnostic (wgpu or vulkan);
        // the on-GPU per-quad path is a later optimization (plan.md Phase 7/8).
        ServerMessage::GpuScene { seq: _, surfaces } => {
            let (w, h) = *fb_size;
            if w == 0 || h == 0 {
                return;
            }
            if let Some(map) = shm.as_ref() {
                let pixels = bridge_protocol::composite_scene(w, h, &map[..], &surfaces);
                gpu.upload(w, h, &pixels);
                gpu.window().request_redraw();
            }
        }
    }
}
