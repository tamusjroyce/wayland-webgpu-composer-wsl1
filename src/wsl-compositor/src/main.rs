//! WSL1 Wayland compositor.
//!
//! Runs a Wayland server, software-composites client `wl_shm` surfaces into a shared
//! memory framebuffer (a memory-mapped file on a Windows-visible path), and exchanges input
//! and frame events with the Windows WebGPU host over a `127.0.0.1` control channel.

mod bridge;
mod handlers;
mod input;
mod protocols;
mod render;
mod shell;
mod state;

use std::error::Error;
use std::fs::OpenOptions;
use std::time::Duration;

use bridge_protocol::{FrameLayout, SharedFramebuffer};
use memmap2::MmapMut;
use smithay::reexports::calloop::channel::{channel, Event as ChannelEvent};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use bridge::Bridge;
use state::State;

/// Target frame interval for the repaint loop (~60 Hz).
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

struct Args {
    /// Path to the shared framebuffer file as seen from WSL1 (e.g. `/mnt/c/.../wwc.fb`).
    shm_path: String,
    /// Path to the same file as the Windows host should open it (e.g. `C:\\...\\wwc.fb`).
    host_path: String,
    /// Explicit `ip:port` for the control channel, or `None` to bind the first free port
    /// scanning upward from [`bridge_protocol::DEFAULT_PORT`].
    listen: Option<String>,
    width: u32,
    height: u32,
    command: Option<String>,
}

fn parse_args() -> Args {
    let mut shm_path = "/mnt/c/Temp/wwc.fb".to_string();
    let mut host_path: Option<String> = None;
    let mut listen: Option<String> = None;
    let mut width = 1280u32;
    let mut height = 720u32;
    let mut command = None;

    let usage = "wsl-compositor [--shm <wsl-path>] [--host-path <windows-path>] \
         [--listen <ip:port>] [--connection-type tcp] [--width N] [--height N] [-c <client-cmd>]\n\
         \n\
         --listen           control channel address. If omitted, bind 127.0.0.1 starting at\n\
         \u{20}                  port 8335 and use the first free port.\n\
         --connection-type  transport to use. Only 'tcp' is supported (the default).";

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--shm" => shm_path = it.next().unwrap_or(shm_path),
            "--host-path" => host_path = it.next(),
            "--listen" => listen = it.next(),
            "--connection-type" => match it.next().as_deref() {
                Some("tcp") => {}
                // Any other (or missing) value is unsupported for now: show help and exit.
                _ => {
                    println!("{usage}");
                    std::process::exit(0);
                }
            },
            "--width" => width = it.next().and_then(|v| v.parse().ok()).unwrap_or(width),
            "--height" => height = it.next().and_then(|v| v.parse().ok()).unwrap_or(height),
            "-c" | "--command" => command = it.next(),
            "-h" | "--help" => {
                println!("{usage}");
                std::process::exit(0);
            }
            other => eprintln!("ignoring unknown argument: {other}"),
        }
    }

    // Derive the Windows path from the WSL `/mnt/<drive>/...` path if not given explicitly.
    let host_path = host_path.unwrap_or_else(|| bridge_protocol::wsl_path_to_windows(&shm_path));

    Args {
        shm_path,
        host_path,
        listen,
        width,
        height,
        command,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    init_logging();
    let args = parse_args();

    // Ensure a usable XDG_RUNTIME_DIR so the Wayland socket can be created even if the shell
    // did not set one (common under `sudo`/WSL1).
    ensure_xdg_runtime_dir();

    let layout = FrameLayout::new(args.width, args.height);

    // Create and size the shared framebuffer file, then map and initialize its header.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&args.shm_path)?;
    file.set_len(layout.total_size())?;
    let mut map = unsafe { MmapMut::map_mut(&file)? };
    SharedFramebuffer::initialize(&mut map, layout);
    tracing::info!(
        "shared framebuffer {}x{} ({} bytes) at {} (host: {})",
        args.width,
        args.height,
        layout.total_size(),
        args.shm_path,
        args.host_path
    );

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;

    // Control channel: background TCP server -> calloop channel of client messages.
    let (ctrl_tx, ctrl_rx) = channel();
    let listen_addrs = match &args.listen {
        Some(a) => vec![a.clone()],
        None => bridge_protocol::default_scan_addrs(),
    };
    let (bridge, bound_addr) = Bridge::spawn(
        listen_addrs,
        args.host_path.clone(),
        layout,
        ctrl_tx,
    );

    let mut state = State::new(&mut event_loop, display, layout, map, bridge);

    event_loop
        .handle()
        .insert_source(ctrl_rx, |event, _, state: &mut State| {
            if let ChannelEvent::Msg(msg) = event {
                state.handle_control(msg);
            }
        })
        .expect("failed to insert control source");

    // Point child Wayland clients at our socket and force toolkits onto the Wayland backend
    // so GTK/Qt apps don't fall back to X11 (which yields "cannot open display:").
    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    std::env::set_var("GDK_BACKEND", "wayland");
    std::env::set_var("QT_QPA_PLATFORM", "wayland");
    std::env::set_var("SDL_VIDEODRIVER", "wayland");
    std::env::set_var("CLUTTER_BACKEND", "wayland");
    // No GPU under WSL1: keep clients on software rendering.
    std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
    std::env::set_var("WLR_RENDERER", "pixman");
    std::env::set_var("GSK_RENDERER", "cairo");
    // Provide a cursor theme so clients don't warn about missing cursors. DMZ-White ships the
    // legacy drag-and-drop cursor names (dnd-move/copy/none) that Weston's toolkit expects.
    if std::env::var_os("XCURSOR_THEME").is_none() {
        std::env::set_var("XCURSOR_THEME", "DMZ-White");
    }
    if std::env::var_os("XCURSOR_SIZE").is_none() {
        std::env::set_var("XCURSOR_SIZE", "24");
    }
    let xrd = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    tracing::info!("WAYLAND_DISPLAY={}", state.socket_name.to_string_lossy());
    tracing::info!(
        "run clients in another shell with: export XDG_RUNTIME_DIR={xrd} WAYLAND_DISPLAY={} GDK_BACKEND=wayland",
        state.socket_name.to_string_lossy()
    );
    // WSL1 has no display: the composited image is in the shared file, viewed on Windows.
    match &bound_addr {
        Some(addr) => tracing::info!(
            "no on-screen window on WSL1 — view the output on Windows with: \
             win-host --host {addr}  (or just run win-host; it auto-discovers port {})",
            bridge_protocol::DEFAULT_PORT
        ),
        None => tracing::error!(
            "control channel failed to bind — the Windows host will not be able to connect"
        ),
    }

    if let Some(cmd) = args.command.clone() {
        spawn_client(&cmd);
    }

    // Manual repaint loop. On WSL1, calloop's timeout mechanism (an internal `timerfd`)
    // stops delivering once a Wayland client socket joins the epoll set, which hangs both a
    // `Timer` source and a timed `dispatch`. So we poll non-blocking (zero timeout, no
    // timerfd) and pace the frame rate with `thread::sleep`.
    while state.running {
        event_loop.dispatch(Some(Duration::ZERO), &mut state)?;
        state.render();
        std::thread::sleep(FRAME_INTERVAL);
    }
    Ok(())
}

fn spawn_client(command: &str) {
    // Run through a shell so commands with arguments work; the child inherits the Wayland
    // environment set above.
    match std::process::Command::new("sh").arg("-c").arg(command).spawn() {
        Ok(_) => tracing::info!("spawned client: {command}"),
        Err(e) => tracing::warn!("failed to spawn client '{command}': {e}"),
    }
}

/// Ensure `XDG_RUNTIME_DIR` points at an existing, private directory. An explicitly-set
/// value is honored (and created if missing); otherwise a per-user temp dir is used.
fn ensure_xdg_runtime_dir() {
    let configured = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| !d.is_empty());
    let dir = match configured {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "default".to_string());
            let d = std::env::temp_dir().join(format!("wwc-runtime-{user}"));
            std::env::set_var("XDG_RUNTIME_DIR", &d);
            tracing::info!("XDG_RUNTIME_DIR was unset; using {}", d.display());
            d
        }
    };
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
