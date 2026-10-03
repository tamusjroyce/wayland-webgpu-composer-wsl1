# wayland-webgpu-composer

A **WSL1 Wayland compositor** that lets you run Linux desktop apps *inside Windows* and
presents them through a **WebGPU** window. The compositor runs as a normal Linux process in
**WSL1**, software-composites its Wayland clients into a shared framebuffer, and a native
Windows app renders that framebuffer live with [`wgpu`](https://github.com/gfx-rs/wgpu) while
forwarding mouse/keyboard back to the clients.

> Built on top of [**Smithay**](https://github.com/Smithay/smithay) — the `smallvil` example
> compositor was used as the starting template and Smithay provides the Wayland server,
> `xdg-shell`, `wl_shm`, seat/input, and surface-management plumbing.
>
> This project was written with **GitHub Copilot (Claude Opus 4.8 model)**.

---

## Why WSL1 specifically?

WSL1 runs Linux binaries as a syscall-translation layer on the NT kernel (not a VM like
WSL2). Two WSL1 properties make the bridge cheap and coherent:

- **Shared filesystem / NT pages** — a file on a Windows path (`C:\…`, seen from Linux as
  `/mnt/c/…`) is backed by the *same* physical pages. Memory-mapping it on both sides gives
  **true zero-copy shared memory** for the framebuffer. (On WSL2, `/mnt/c` is proxied over
  virtiofs and isn't coherent in the same way.)
- **Shared loopback** — `127.0.0.1` TCP between a WSL1 process and a Windows process is a
  direct loopback (no NAT), used here as the low-latency **control channel**.

A WSL1 ELF process can't load the Windows WebGPU DLL in-process, so the GPU work lives in a
small native Windows host; the two halves talk over the shared framebuffer + control channel.

```
        WSL1 (Linux)                                Windows (native)
 ┌───────────────────────────┐              ┌──────────────────────────────┐
 │ wsl-compositor (smithay)  │              │ win-host (winit + wgpu)      │
 │  • Wayland server socket  │              │  • single scalable window    │
 │  • wl_shm clients         │              │  • WebGPU device/surface     │
 │  • software compositing ──┼── shared ────┼─▶ maps same file, uploads    │
 │    → mmap'd /mnt/c file    │   memory     │    texture, presents scaled  │
 │  • injects input      ◀───┼── 127.0.0.1 ─┼── forwards pointer/keyboard  │
 └───────────────────────────┘   TCP        └──────────────────────────────┘
```

## Workspace layout

| Crate | Target | Purpose |
|-------|--------|---------|
| `src/wsl-compositor` | Linux (WSL1) | The Smithay-based Wayland compositor; software-composites `wl_shm` clients into the shared framebuffer and injects input. |
| `src/win-host` | Windows | `winit` + `wgpu` app: one scalable window, maps the shared framebuffer, presents it, forwards input over TCP. |
| `src/bridge-protocol` | shared | Shared-memory framebuffer layout + the TCP control-channel wire protocol. Pure `std` Rust, builds on both OSes. |
| `src/fb-dump` | any | Renders the shared framebuffer file to a PNG — headless "visual test". |
| `src/wwc-setup` | Windows | Setup/launcher GUI (`eframe`) for the MSIX: detects a WSL1 Debian/Ubuntu distro, installs the compositor into it, and launches the host. Excluded from the workspace. |
| `smithay/` | — | Vendored Smithay (path dependency). |

`win-host`, `bridge-protocol`, and `fb-dump` form the root Cargo workspace (Windows-buildable).
`wsl-compositor` is excluded and built inside WSL1.

## Prerequisites

### To run a pre-built release
- **Windows 10/11 (x64)** with a `wgpu`-supported GPU driver (Direct3D 12 or Vulkan).
- A **WSL1** distribution — **Ubuntu 22.04** recommended (its glibc 2.35 matches the release
  binary). Verify it is WSL1 with `wsl -l -v` (VERSION column shows `1`).
- Inside the WSL1 distro: `libxkbcommon0` (runtime). Optional: `weston` (demo clients /
  nested desktop) and `dmz-cursor-theme` (cursor icons).

### To build from source
- **Rust** (stable) on both sides — install via <https://rustup.rs>.
- **Windows:** the **MSVC toolchain** Rust links against — install the *Visual Studio Build
  Tools* with the "Desktop development with C++" workload (or full Visual Studio).
- **WSL1 (Ubuntu 22.04):** `build-essential`, `pkg-config`, `libxkbcommon-dev`.
  `scripts/wsl1-install.sh` installs these (plus the Rust toolchain and demo clients) for you.
- **Optional — code coverage:** `cargo install cargo-llvm-cov` and
  `rustup component add llvm-tools-preview`.

## Download & install (pre-built releases)

Each tagged release publishes these assets on the GitHub **Releases** page (built
automatically by GitHub Actions):

| Asset | Contents | For |
|-------|----------|-----|
| `wayland-webgpu-composer-windows-x64.msix` | Installer: Start Menu app + setup wizard (bundles the host binaries and `install.cmd`/`run.cmd`/`run.sh`) | Windows |
| `wayland-webgpu-composer.cer` + `install_cert.cmd` / `install_cert.sh` | Signing certificate + scripts to trust it (beta is self-signed) | Windows |
| `wayland-webgpu-composer-windows-x64.zip` | `win-host.exe`, `fb-dump.exe`, `install.cmd`, `run.cmd`, `run.sh` | Windows (portable) |
| `wayland-webgpu-composer-wsl1-x64.tar.gz` | `wsl-compositor`, `install.sh` (+ `wsl1-install.sh`) | WSL1 (Ubuntu 22.04) |

**Windows — MSIX (recommended).**

1. Download `wayland-webgpu-composer-windows-x64.msix`, `wayland-webgpu-composer.cer`, and
   `install_cert.cmd` into the **same folder**.
2. This beta is **self-signed**, so trust the certificate once: right-click `install_cert.cmd`
   → **Run as administrator** (or run `install_cert.sh` from an elevated Git Bash). This must
   be redone for each release — the self-signed certificate changes per build.
3. Double-click the `.msix` and click **Install** (Publisher now shows
   `wayland-webgpu-composer`).
4. Launch **"Wayland WebGPU Composer"** from the Start Menu. The setup wizard lists your WSL
   distros, enables only **WSL version 1** Debian/Ubuntu distros (others grayed out), installs
   the compositor into the one you pick, and opens the WebGPU window.

**Windows — portable (no install).** Download `…-windows-x64.zip`, extract it anywhere, and
run `install.cmd`. It sets up **both** halves and hands off to `run.cmd` to launch them:

1. Installs the Windows host into `%LOCALAPPDATA%\wayland-webgpu-composer`.
2. Ensures a WSL1 distro named `WWC-WSL1` exists (imports Ubuntu 22.04 as WSL1 on first run).
3. Installs the Linux `wsl-compositor` + a demo desktop (Weston) inside it.
4. `run.cmd` launches the compositor and the WebGPU window.

Re-run `install.cmd` to update and relaunch; run `run.cmd` (or `run.sh` from Git Bash) to just
relaunch. Pass a client to change what runs, e.g. `install.cmd "gnome-calculator"`. (First run
downloads an Ubuntu rootfs, so it takes a few minutes; later runs are quick.)

**WSL1 (manual).** Download `…-wsl1-x64.tar.gz`, extract, and run `install.sh` (or install by
hand):

```bash
tar -xzf wayland-webgpu-composer-wsl1-x64.tar.gz
./install.sh    # installs wsl-compositor + the libxkbcommon0 runtime dep
# ...or by hand:
sudo install -Dm755 wsl-compositor /usr/local/bin/wsl-compositor
sudo apt-get update && sudo apt-get install -y libxkbcommon0 weston  # runtime lib + demo clients
```

The Linux binary is built on Ubuntu 22.04 (glibc 2.35) — use an Ubuntu 22.04 WSL1 distro for a
matching runtime. Then skip to [Run](#run).

## Install (build from source into a WSL1 distro)

If you don't have a WSL1 distro yet:

```powershell
# Ubuntu 22.04 rootfs → import as WSL1 (note --version 1)
curl.exe -L -o C:\temp\jammy-wsl.tar.gz `
  "https://cloud-images.ubuntu.com/wsl/jammy/current/ubuntu-jammy-wsl-amd64-ubuntu22.04lts.rootfs.tar.gz"
wsl --import WWC-WSL1 C:\wsl\WWC-WSL1 C:\temp\jammy-wsl.tar.gz --version 1
```

Then build + install the compositor and Wayland clients inside it:

```powershell
wsl -d WWC-WSL1 -u root -- bash -lc "sed -i 's/\r$//' /mnt/c/path/to/repo/scripts/wsl1-install.sh; bash /mnt/c/path/to/repo/scripts/wsl1-install.sh"
```

That installs build deps + Wayland demo clients (`weston`), the Rust toolchain if needed,
builds the compositor, and installs it to `/usr/local/bin/wsl-compositor`.

## Run

**1. Start the compositor in WSL1** (it auto-launches a client with `-c`):

```bash
XDG_RUNTIME_DIR=/tmp/wwc-xrd wsl-compositor \
  --shm /mnt/c/Users/<you>/wwc/visual.fb \
  -c gnome-calculator
```

The compositor is **headless on WSL1** — it renders into the shared file, not an on-screen
window. The directory part of `--shm` must already exist.

**2. View it on Windows** with the WebGPU host:

```powershell
cargo run -p win-host
```

With no arguments the host auto-discovers the compositor by scanning `127.0.0.1` from port
8335 upward (the compositor binds the first free port in that range). Pass `--host <ip:port>`
to target a specific address, or `--connection-type tcp` to select the transport (TCP is the
default and currently the only option).

A single scalable window appears showing the composited output; your mouse/keyboard in that
window flow back into the Wayland clients. For a headless check, snapshot to PNG instead:

```powershell
cargo run -p fb-dump -- target\visual.fb target\out.png
```

### Running a whole "desktop"

This is itself the compositor, so a full DE (GNOME/KDE) won't run on top of it. For a desktop
experience, run a **nested compositor** (Weston) as a client — it draws its entire desktop
(background, panel, clock, launcher) into one surface:

```bash
XDG_RUNTIME_DIR=/tmp/wwc-desk wsl-compositor \
  --shm /mnt/c/Users/<you>/wwc/desktop.fb \
  --width 1296 --height 824 \
  -c 'weston --use-pixman --width=1280 --height=800'
```

## Tests & coverage

```powershell
cargo test --workspace          # unit tests for protocol, input mapping, blit, snapshot
cargo coverage                  # requires cargo-llvm-cov + llvm-tools-preview
```

The testable crates report ~99% line coverage. GPU init, the event loop, and background
TCP threads are excluded (validated by running the app). The `wsl-compositor` crate is
Linux-only — build/test it inside WSL1.

## Continuous integration & releases

- **CI** ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) — on every push/PR, builds
  and tests the Windows workspace and builds the WSL1 compositor on Ubuntu 22.04.
- **Release** ([`.github/workflows/release.yml`](.github/workflows/release.yml)) — a reusable
  workflow that builds release binaries for Windows and WSL1 and attaches the archives
  (`.zip`, `.tar.gz`, and the signed `.msix` + `.cer`) to a GitHub Release. It runs on a
  pushed `vX.Y.Z` tag, or when called by auto-release.
- **Auto-release** ([`.github/workflows/auto-release.yml`](.github/workflows/auto-release.yml))
  — every push to `main` bumps the patch version from the latest `v*` tag and cuts a release
  automatically. Add `[skip release]` to a commit message to push without releasing. To
  release manually instead, push a tag:

  ```bash
  git tag v0.1.3
  git push origin v0.1.3
  ```

### Security & supply chain

- **CodeQL** ([`codeql.yml`](.github/workflows/codeql.yml)) — static analysis of the Rust
  workspace on every push/PR and weekly.
- **cargo audit** ([`security.yml`](.github/workflows/security.yml)) — checks every committed
  lockfile against the RustSec advisory database.
- **cargo deny** ([`deny.toml`](deny.toml)) — enforces an allow-list of permissive licenses,
  blocks advisories, and restricts dependency sources to crates.io.
- **Reproducible builds** — the toolchain is pinned ([`rust-toolchain.toml`](rust-toolchain.toml))
  and all release builds use `--locked`, so CI builds from the exact committed dependency set.
- **Signed from CI artifacts only** — the MSIX is signed from the artifact CI produced (see
  below), not from a local machine.

### MSIX signing

The MSIX ([`packaging/msix/`](packaging/msix)) is built by the `msix` CI job with `makeappx`
and bundles the setup wizard (`wwc-setup`) plus the host binaries. MSIX must be signed to
install, and the certificate's subject must match the manifest's `Publisher`
(`CN=wayland-webgpu-composer`). CI chooses a signer in this order:

1. **SignPath** (recommended; free for OSS) — if `SIGNPATH_API_TOKEN` is set, CI uploads the
   unsigned MSIX as a build artifact and `signpath/github-action-submit-signing-request`
   signs **that artifact** with a trusted certificate. The result installs with no warning and
   no per-user trust step. Configure on [signpath.io](https://signpath.io): create a project,
   add GitHub Actions as a trusted build system, and set repo secret `SIGNPATH_API_TOKEN` plus
   repo variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`,
   `SIGNPATH_SIGNING_POLICY_SLUG`.
2. **Provided certificate** — set secrets `WWC_PFX_BASE64` (base64 `.pfx`) + `WWC_PFX_PASSWORD`.
3. **Self-signed fallback** (current beta default) — CI self-signs with Microsoft tooling
   (`New-SelfSignedCertificate` + `signtool`) and publishes, as separate downloads:
   `wayland-webgpu-composer.cer` and `install_cert.cmd` / `install_cert.sh`. Users trust it
   once per release — download the `.msix`, the `.cer`, and `install_cert.cmd` into one folder,
   right-click `install_cert.cmd` → **Run as administrator** (or `install_cert.sh` from an
   elevated Git Bash), then open the `.msix`. Equivalent manual commands:

   ```powershell
   Import-Certificate -FilePath wayland-webgpu-composer.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
   Add-AppxPackage wayland-webgpu-composer-windows-x64.msix
   ```

To build locally:

```bat
packaging\msix\build-msix.cmd packaging\msix\bin
```

(with `wwc-setup.exe`, `win-host.exe`, `fb-dump.exe` staged in `packaging\msix\bin`; set
`WWC_PFX`/`WWC_PFX_PASS` to sign).

## Known limitations

- **xdg-shell + `wl_shm` + seat only** — no `linux-dmabuf`/EGL, no `wlr-layer-shell`, no
  server-side decorations yet. Shm-based GTK/Qt apps and Weston demos work.
- **No window management** — every toplevel maps at `(0,0)` and stacks.
- **No GPU in WSL1** — clients render in software (the compositor exports
  `LIBGL_ALWAYS_SOFTWARE=1`, `GSK_RENDERER=cairo`, etc.).
- **`weston --fullscreen` aborts the compositor** on WSL1 — a `rustix` `recvmsg` cmsg
  parsing overflow against a WSL1 syscall quirk. Use size-matching instead (above).

## Acknowledgements & license

- Based on [**Smithay**](https://github.com/Smithay/smithay) (MIT) — the `smallvil` example
  was the template for the compositor.
- Rendering via [`wgpu`](https://github.com/gfx-rs/wgpu); windowing via
  [`winit`](https://github.com/rust-windowing/winit).
- Authored with **GitHub Copilot (Claude Opus 4.8)**.

Licensed under the MIT License.
