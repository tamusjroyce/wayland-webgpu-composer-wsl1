//! Setup/launcher wizard for wayland-webgpu-composer.
//!
//! Detects installed WSL distros, shows which are usable (WSL **version 1** and
//! Debian/Ubuntu-based), lets the user pick one, installs the Linux compositor into it,
//! and launches the compositor + the WebGPU host. The MSIX package wires this executable
//! to the "Wayland WebGPU Composer" Start Menu entry.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;

/// Don't flash a console window when invoking `wsl.exe`.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const WSL_TARBALL_URL: &str = "https://github.com/tamusjroyce/wayland-webgpu-composer-wsl1/releases/latest/download/wayland-webgpu-composer-wsl1-x64.tar.gz";

/// A WSL distro and whether it can host the compositor.
#[derive(Clone)]
struct Distro {
    name: String,
    version: u8,
    compatible: bool,
    reason: String,
}

fn wsl_cmd(args: &[&str]) -> Command {
    let mut c = Command::new("wsl.exe");
    c.args(args);
    c.env("WSL_UTF8", "1"); // UTF-8 output instead of UTF-16
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// Is the distro Debian/Ubuntu based? Probed via `/etc/os-release`. Only call for WSL1
/// distros to avoid cold-starting unrelated distros.
fn is_debian_like(name: &str) -> bool {
    match wsl_cmd(&["-d", name, "-u", "root", "--", "cat", "/etc/os-release"]).output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).to_lowercase();
            s.contains("debian") || s.contains("ubuntu")
        }
        Err(_) => false,
    }
}

/// Enumerate distros via `wsl --list --verbose` and classify each.
fn list_distros() -> Vec<Distro> {
    let output = match wsl_cmd(&["--list", "--verbose"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut distros = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches('*').trim();
        if line.is_empty() || line.starts_with("NAME") {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        let name = cols[0].to_string();
        let version: u8 = cols[cols.len() - 1].parse().unwrap_or(0);

        let (compatible, reason) = if version != 1 {
            (false, format!("Not WSL1 (currently WSL{version})"))
        } else if !is_debian_like(&name) {
            (false, "WSL1 but not Debian/Ubuntu based".to_string())
        } else {
            (true, "WSL1, Debian/Ubuntu — supported".to_string())
        };
        distros.push(Distro {
            name,
            version,
            compatible,
            reason,
        });
    }
    distros
}

fn win_to_wsl(p: &str) -> String {
    let drive = p.chars().next().unwrap_or('c').to_ascii_lowercase();
    let rest = p.get(2..).unwrap_or("").replace('\\', "/");
    format!("/mnt/{drive}{rest}")
}

fn log_line(log: &Arc<Mutex<String>>, line: impl AsRef<str>) {
    if let Ok(mut g) = log.lock() {
        g.push_str(line.as_ref());
        g.push('\n');
    }
}

struct SetupApp {
    distros: Vec<Distro>,
    selected: Option<usize>,
    log: Arc<Mutex<String>>,
    busy: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl SetupApp {
    fn new() -> Self {
        let distros = list_distros();
        let selected = distros.iter().position(|d| d.compatible);
        SetupApp {
            distros,
            selected,
            log: Arc::new(Mutex::new(String::new())),
            busy: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
        }
    }

    fn any_compatible(&self) -> bool {
        self.distros.iter().any(|d| d.compatible)
    }

    fn start_install(&self, idx: usize) {
        let distro = self.distros[idx].name.clone();
        let log = self.log.clone();
        let busy = self.busy.clone();
        let finished = self.finished.clone();
        busy.store(true, Ordering::SeqCst);
        thread::spawn(move || {
            log_line(&log, format!("Installing compositor into '{distro}'..."));
            let setup = format!(
                "set -e; cd /tmp; curl -fL '{WSL_TARBALL_URL}' -o wwc.tgz; tar -xzf wwc.tgz; \
                 install -Dm755 wsl-compositor /usr/local/bin/wsl-compositor; \
                 export DEBIAN_FRONTEND=noninteractive; \
                 apt-get -o APT::Sandbox::User=root update -qq || true; \
                 apt-get -o APT::Sandbox::User=root install -y libxkbcommon0 weston dmz-cursor-theme >/dev/null 2>&1 || true; \
                 rm -f wwc.tgz"
            );
            let status = wsl_cmd(&["-d", &distro, "-u", "root", "--", "bash", "-lc", &setup]).status();
            if !matches!(status, Ok(s) if s.success()) {
                log_line(&log, "ERROR: failed to install the compositor into the distro.");
                busy.store(false, Ordering::SeqCst);
                return;
            }

            // Shared framebuffer lives under %LOCALAPPDATA% so both sides can map it.
            let local = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "C:\\".into());
            let fb_dir = format!("{local}\\wayland-webgpu-composer\\fb");
            let _ = std::fs::create_dir_all(&fb_dir);
            let fb_win = format!("{fb_dir}\\desktop.fb");
            let fb_wsl = win_to_wsl(&fb_win);

            log_line(&log, "Starting compositor...");
            let run = format!(
                "pkill -9 -x wsl-compositor 2>/dev/null; mkdir -p /tmp/wwc-desk; \
                 XDG_RUNTIME_DIR=/tmp/wwc-desk /usr/local/bin/wsl-compositor \
                 --shm '{fb_wsl}' --width 1440 --height 900 \
                 -c 'weston --use-pixman --width=1280 --height=800'"
            );
            let _ = wsl_cmd(&["-d", &distro, "-u", "root", "--", "bash", "-lc", &run]).spawn();

            log_line(&log, "Starting WebGPU window...");
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    let host = dir.join("win-host.exe");
                    let mut c = Command::new(host);
                    c.creation_flags(CREATE_NO_WINDOW);
                    let _ = c.spawn();
                }
            }

            log_line(&log, "Running. You can close this window.");
            finished.store(true, Ordering::SeqCst);
            busy.store(false, Ordering::SeqCst);
        });
    }
}

impl eframe::App for SetupApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let busy = self.busy.load(Ordering::SeqCst);
        let finished = self.finished.load(Ordering::SeqCst);
        let any_compatible = self.any_compatible();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Wayland WebGPU Composer");
            ui.label("Run Linux desktop apps inside Windows through a WebGPU window.");
            ui.add_space(8.0);
            ui.label("Choose the WSL distro to host the compositor. Only WSL version 1 \
                      Debian/Ubuntu distros are supported.");
            ui.add_space(8.0);

            if self.distros.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 80, 80),
                    "No WSL distros found. Install WSL and a WSL1 Ubuntu/Debian distro, then reopen.",
                );
            }

            egui::Grid::new("distros").num_columns(2).striped(true).show(ui, |ui| {
                for (i, d) in self.distros.iter().enumerate() {
                    let label = format!("{}  (WSL{})", d.name, d.version);
                    if d.compatible {
                        ui.radio_value(&mut self.selected, Some(i), label);
                    } else {
                        // Grayed-out, non-selectable entry for unsupported distros.
                        ui.add_enabled(false, egui::RadioButton::new(false, label));
                    }
                    ui.label(&d.reason);
                    ui.end_row();
                }
            });

            ui.add_space(12.0);

            ui.horizontal(|ui| {
                let can_install = !busy
                    && self
                        .selected
                        .map(|i| self.distros.get(i).is_some_and(|d| d.compatible))
                        .unwrap_or(false);
                if ui
                    .add_enabled(can_install, egui::Button::new("Install & Run"))
                    .clicked()
                {
                    if let Some(i) = self.selected {
                        self.start_install(i);
                    }
                }

                // Per spec: Cancel is only enabled when there is no usable WSL1 OS.
                let can_cancel = !busy && !any_compatible;
                if ui
                    .add_enabled(can_cancel, egui::Button::new("Cancel"))
                    .clicked()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.add_space(12.0);
            let log = self.log.lock().map(|g| g.clone()).unwrap_or_default();
            if !log.is_empty() {
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    ui.monospace(log);
                });
            }

            if finished {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(80, 170, 80), "Launched.");
            }
        });

        if busy {
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 500.0])
            .with_title("Wayland WebGPU Composer"),
        ..Default::default()
    };
    eframe::run_native(
        "Wayland WebGPU Composer",
        options,
        Box::new(|_cc| Ok(Box::new(SetupApp::new()))),
    )
}
