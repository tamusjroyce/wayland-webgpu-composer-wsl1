# Plan: WSL1 Wayland Compositor → WebGPU on Windows

Checkbox legend (see `agent.md`):

- `- [ ]` not started
- `- [-]` in progress
- `- [*]` completed

---

## 1. Goal

Run a Wayland compositor **inside WSL1** (the syscall-translation flavor, *not* the WSL2
VM) and present its final composited image on the Windows desktop in **a single, scalable
window backed by a WebGPU context**, using the Rust `wgpu` library.

## 2. Key constraint analysis (WSL1 vs. WSL2, and "share WebGPU directly")

What is and is not possible, and why the chosen design follows:

- **Direct in-process call of the Windows WebGPU DLL from WSL1 is NOT possible.** A WSL1
  process is an ELF (Linux) binary. The NT loader cannot load a PE/`.dll` (e.g. the Windows
  Dawn/D3D12 WebGPU backend) into an ELF process's address space, and there is no supported
  ELF→PE in-process FFI. So the WebGPU device must live in a **native Windows process**.
- **WSL1 shares the Windows network stack.** `127.0.0.1` TCP between a WSL1 process and a
  Windows process is a direct loopback with no NAT/port-proxy hop (unlike WSL2's virtual
  NIC). → Use localhost TCP as the low-latency **control channel**.
- **WSL1 shares the Windows filesystem through the same NT file objects.** A file created on
  a Windows path (e.g. `C:\Users\…\Temp\wwc.fb`, visible from WSL1 as
  `/mnt/c/Users/…/Temp/wwc.fb`) is backed by the *same* physical pages. Memory-mapping it
  with `mmap` (Linux side) and `MapViewOfFile` (Windows side) yields **true shared memory** →
  this is our zero-copy "allocate and share the framebuffer" path.
- **No GPU/DRM inside WSL1.** There is no `/dev/dri`. Therefore the compositor does
  **software compositing** of client `wl_shm` buffers into a single CPU framebuffer; the GPU
  work (upload + scale + present) happens on the Windows side via `wgpu`.

### Resulting architecture (two processes + shared memory bridge)

```
        WSL1 (Linux ELF)                          Windows (native PE)
 ┌───────────────────────────┐            ┌──────────────────────────────┐
 │ wsl-compositor (smithay)  │            │ win-host (winit + wgpu)      │
 │  • Wayland server socket  │            │  • single scalable window    │
 │  • wl_shm clients         │            │  • WebGPU device/surface     │
 │  • software compositing   │            │  • fullscreen scaled quad    │
 │  • writes framebuffer ───────shared────────→ maps same file, uploads │
 │    to mmap'd file         │   memory   │    texture, presents         │
 │  • injects input  ←───────── 127.0.0.1 TCP ──── forwards input/resize │
 └───────────────────────────┘  control   └──────────────────────────────┘
                 ▲ bridge-protocol (shared Rust crate: shm layout + messages)
```

## 3. Repository layout

- `smithay/` — vendored Smithay (Wayland compositor toolkit), used as a path dependency.
- `src/bridge-protocol/` — shared crate: shared-memory header layout + TCP control messages.
  Pure Rust `std`, compiles on both Linux and Windows.
- `src/wsl-compositor/` — Linux-only crate (smithay). Built **inside WSL1**.
- `src/win-host/` — Windows-only crate (winit + wgpu). Built **on Windows**.
- Root `Cargo.toml` — virtual workspace for the Windows-buildable crates
  (`bridge-protocol`, `win-host`); `wsl-compositor` is a standalone crate built in WSL1.

## 4. Task checklist

### Phase 0 — Scaffolding & docs
- [*] Analyze Smithay `smallvil` reference compositor
- [*] Decide WSL1 bridge architecture (shared mmap framebuffer + localhost TCP control)
- [*] Create `agent.md` (always maintain `plan.md`)
- [*] Create `plan.md` (this file)
- [*] Create root workspace `Cargo.toml`

### Phase 1 — Shared protocol
- [*] `bridge-protocol`: shared-memory framebuffer header + double-buffer slot layout
- [*] `bridge-protocol`: control-channel message types (resize, pointer, button, axis, key)
- [*] `bridge-protocol`: length-prefixed binary (de)serialization over any `Read`/`Write`

### Phase 2 — Windows WebGPU host
- [*] `win-host`: open a single scalable `winit` window
- [*] `win-host`: initialize `wgpu` device/queue/surface (WebGPU)
- [*] `win-host`: fullscreen-triangle shader that samples the shared framebuffer texture
- [*] `win-host`: memory-map the shared framebuffer file and upload the active slot
- [*] `win-host`: scale framebuffer to window (letterboxed, aspect-correct)
- [*] `win-host`: TCP control client — connect, handshake, forward input + resize
- [*] `win-host`: verify build on Windows (`cargo build -p win-host`)
- [*] `bridge-protocol`: unit tests pass (`cargo test -p bridge-protocol`)

### Phase 3 — WSL1 compositor
- [*] `wsl-compositor`: smithay state, Wayland socket, xdg-shell, shm, seat (from smallvil)
- [*] `wsl-compositor`: software renderer compositing `wl_shm` surfaces → CPU framebuffer
- [*] `wsl-compositor`: publish framebuffer into the shared mmap file (double-buffered)
- [*] `wsl-compositor`: TCP control server — handshake, receive input/resize, inject to seat
- [*] `wsl-compositor`: drive frame callbacks / repaint loop via calloop timer
- [*] `wsl-compositor`: build inside WSL1 (fixed `InputTime` vs `u32` event times)

### Phase 4 — Integration & validation
- [*] Create a real WSL1 distro and run the compositor on it (see §7)
- [*] Verify Windows reads frames the WSL1 compositor wrote to the `/mnt/c` shared file
- [*] Verify the WSL1↔Windows TCP control channel handshake over shared loopback
- [*] Compose a real Wayland client (`weston-flower`/`weston-simple-shm`) and snapshot it
- [*] Install/visual-test scripts (`scripts/wsl1-install.sh`, `wsl1-visual-test.sh`, `visual-test.ps1`)
- [*] `fb-dump` tool to render the shared framebuffer to PNG for headless visual testing
- [ ] End-to-end with the interactive GUI `win-host` window + input round-trip
- [*] Document run instructions in `README.md`

### Phase 4.5 — Testing & coverage
- [*] Extract pure logic into testable units (input mapping, args, path conversion, blit)
- [*] `bridge-protocol`: unit tests for layout, framebuffer, messages, errors, blit, paths
- [*] `win-host`: unit tests for args, input mapping, shared-memory open
- [*] `fb-dump`: unit tests for BGRA→RGBA and snapshot
- [*] Add `cargo llvm-cov` tooling and a `cargo coverage` alias
- [*] Reach 80%+ code coverage (achieved ~99.9% lines / ~98% regions on testable crates)
- [ ] Add compositor-side tests (requires building in WSL1)

### Phase 5 — Optimizations (future)
- [ ] Damage tracking: only re-upload changed rectangles
- [ ] Shared-memory frame signaling via atomics to avoid TCP wakeups
- [ ] Explore `linux-dmabuf` / GPU-side upload paths if a future WSL gains GPU access

## 5. How to run (target workflow)

1. On **Windows**: `cargo run -p win-host -- --shm C:\Users\<you>\AppData\Local\Temp\wwc.fb`
2. In **WSL1**: `cargo run` inside `src/wsl-compositor` with
   `--shm /mnt/c/Users/<you>/AppData/Local/Temp/wwc.fb --host 127.0.0.1:7777`
3. Point Wayland clients at the compositor: `WAYLAND_DISPLAY=wayland-1 weston-terminal`

## 6. Testing & coverage

- Run tests: `cargo test --workspace`
- Measure coverage: `cargo coverage` (requires `cargo install cargo-llvm-cov` and
  `rustup component add llvm-tools-preview`). An HTML report: `cargo coverage-html`.
- Coverage excludes `gpu.rs` (needs a real GPU), `bridge.rs` (background TCP threads /
  winit proxy) and `main.rs` (event-loop entry) — these are integration glue validated by
  running the app, not unit tests. The testable crates report ~99% line coverage.
- The `wsl-compositor` crate is Linux-only; build and test it inside WSL1
  (`cd src/wsl-compositor && cargo test`).

## 7. WSL1 setup & validation results

A real WSL1 instance was created and the compositor was run on it (verified 2026-10-02).

### Creating a WSL1 instance (non-destructive)

The machine's default `Ubuntu` distro is WSL2 (and is used by Docker Desktop), so a
separate WSL1 distro was imported instead of converting it:

```powershell
# Download the Ubuntu 22.04 WSL rootfs (glibc 2.35, matches the build host)
curl.exe -L -o C:\temp\jammy-wsl.tar.gz `
  "https://cloud-images.ubuntu.com/wsl/jammy/current/ubuntu-jammy-wsl-amd64-ubuntu22.04lts.rootfs.tar.gz"
wsl --import WWC-WSL1 C:\wsl\WWC-WSL1 C:\temp\jammy-wsl.tar.gz --version 1
```

Build deps for the compositor (inside any Ubuntu 22.04): `build-essential pkg-config
libxkbcommon-dev`. Runtime-only needs just `libxkbcommon0`. `uname -a` on WSL1 reports
`4.4.0-...-Microsoft` (the translation-layer kernel), confirming it is not WSL2.

### What was validated on WSL1

- **Builds on Linux**: fixed one real bug the Windows build could not catch — this smithay
  fork uses `InputTime` (not `u32`) for event timestamps.
- **Runs on WSL1**: creates the shared framebuffer, binds the control channel, opens the
  `wayland-1` socket.
- **Shared memory works across the boundary**: the compositor wrote to a `/mnt/c` file and
  Windows read back the header (`version=1, width, height, stride`) and a live
  `frame_seq=171` — i.e. frames written by the WSL1 mmap landed in the NTFS file Windows
  sees.
- **Control channel works across shared loopback**: a Windows TCP client connected to the
  WSL1 compositor and received the correct `FrameConfig` handshake (dimensions, `shm_size`,
  `host_path`).

### Known WSL1 limitation (resolved)

- `memfd` sealing is unimplemented on WSL1, so smithay's `SealedFile` (used for the xkb
  keymap) failed with `Function not implemented (os error 38)`. Fixed by patching
  `smithay/src/utils/sealed_file.rs` to fall back to a read-only `shm_open` object when the
  sealed `memfd` path fails — WSL1 has a working `/dev/shm`, so the keymap is now delivered
  without error.

## 8. Running Wayland clients / "desktop environment"

This project **is** the Wayland compositor (display server). A full DE (GNOME/KDE) ships its
own compositor and will not run *on top* of this one; instead you run individual Wayland
**client** apps against it. Because it implements `xdg-shell` + `wl_shm` + seat (no
dmabuf/EGL, no `wlr-layer-shell`), software/shm clients work (weston demos, many GTK apps).

Install clients + build/install the compositor into the WSL1 distro:

```bash
# one-shot (inside the WSL1 distro): installs libs + weston demo clients, builds + installs
wsl -d WWC-WSL1 -u root -- bash /mnt/c/.../scripts/wsl1-install.sh
# or just the client packages:
sudo apt-get install -y weston libwayland-client0 foot    # weston = demo clients
```

### Scripts

- `scripts/wsl1-install.sh` — installs build deps + Wayland clients, builds the compositor
  with `cargo`, and installs it to `/usr/local/bin/wsl-compositor` (run inside WSL1).
- `scripts/wsl1-visual-test.sh` — starts the compositor + a Wayland client (default
  `weston-simple-shm`) for a visual test.
- `scripts/visual-test.ps1` — Windows orchestrator: launches the WSL1 compositor + client,
  snapshots the composited framebuffer to `target/visual.png` via `fb-dump`, and opens it;
  `-Live` also opens the interactive `win-host` WebGPU window.
- `src/fb-dump` — reads the shared framebuffer file and writes a PNG (headless visual test).

### Visual test result

Running `weston-flower` against the WSL1 compositor and snapshotting from Windows produced a
PNG showing the client (a flower) composited at the top-left of the framebuffer — confirming
the full client → compositor → shared-memory → Windows path works on WSL1.

### Bugs found and fixed during visual testing

- **Re-entrant surface-lock deadlock** (critical): `composite()` called
  `with_renderer_surface_state(surface, …)` *inside* a `with_surface_tree_downward` closure,
  re-locking the same surface state and deadlocking the event loop the instant the first
  window mapped. Fixed by reading `RendererSurfaceStateUserData` from the `states` the
  traversal already provides.
- **Repaint loop**: replaced the calloop `Timer` source with a manual
  `dispatch(ZERO)` + `render()` + `sleep(16ms)` loop (robust and simple on WSL1).
- **Live `/mnt/c` coherence**: the compositor now `msync`s (async `flush_async_range`) the
  published slot + header each frame so the Windows host sees frames live (DrvFs mmap is
  otherwise only flushed on unmap).

