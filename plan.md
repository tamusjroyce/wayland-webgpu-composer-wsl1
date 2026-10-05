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

### Phase 6 — wlroots / Wayland protocol compatibility

Reference compositor: `labwc/` (cloned by `configure`). A protocol is **complete** when its
global is created (+ handler where required) and advertised to clients. Two tests per item:
- **code test** — the compositor builds in WSL1 (`cd src/wsl-compositor && cargo build`).
- **e2e test** — `scripts/protocol-check.sh` runs the compositor and `wayland-info` and the
  global appears in the advertised interface list.

- [*] Protocol e2e harness (`scripts/protocol-check.sh` + `wayland-info`/`weston-info`)
  - [*] code: script starts the compositor, runs the info tool, greps the global list
  - [*] e2e: harness prints the advertised interfaces (36 verified 2026-10-04)

#### Desktop-readiness matrix (sway / KDE Plasma)

Protocols commonly required by full window managers and desktops. Each maps to a concrete
Wayland global; "complete" means the global is advertised and handled per its spec. Items not
yet in smithay are hand-rolled with smithay's `Dispatch2`/`GlobalDispatch2` (bindings re-exported
from `wayland-protocols`/`wayland-protocols-wlr`), using `labwc/` as the behavioural reference.

- [*] `xdg-shell` -> `xdg_wm_base` (windows map + configure + popups)
- [*] `layer-shell` -> `zwlr_layer_shell_v1` (render-integrated: panels/bars)
- [*] `xdg-decoration` -> `zxdg_decoration_manager_v1` (forces client-side decorations)
- [ ] `linux-dmabuf` -> `zwp_linux_dmabuf_v1` - blocked: no GPU/DRM on WSL1; clients fall back to `wl_shm`
- [*] `viewporter` -> `wp_viewporter`
- [*] `fractional-scale` -> `wp_fractional_scale_manager_v1`
- [*] `cursor-shape` -> `wp_cursor_shape_manager_v1`
- [*] `data-control` -> `zwlr_data_control_manager_v1` + `ext_data_control_manager_v1`
- [*] `foreign-toplevel` (list) -> `ext_foreign_toplevel_list_v1`
  - [*] `foreign-toplevel` (management) -> `zwlr_foreign_toplevel_manager_v1`
- [*] `xdg-output` -> `zxdg_output_manager_v1`
- [*] `output-management` -> `zwlr_output_manager_v1` (read-only heads)
- [*] `xdg-activation` -> `xdg_activation_v1`
- [*] `presentation-time` -> `wp_presentation`
- [*] `idle-inhibit` -> `zwp_idle_inhibit_manager_v1`
- [*] `pointer-constraints` -> `zwp_pointer_constraints_v1`
- [*] `relative-pointer` -> `zwp_relative_pointer_manager_v1`
- [*] `pointer-gestures` -> `zwp_pointer_gestures_v1`
- [*] `primary-selection` -> `zwp_primary_selection_device_manager_v1`
- [*] `text-input-v3` -> `zwp_text_input_manager_v3`

Core (smallvil baseline):
- [*] `wl_compositor`
  - [*] code: `CompositorState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wl_subcompositor`
  - [*] code: provided by `CompositorState`
  - [*] e2e: advertised in `wayland-info`
- [*] `wl_shm`
  - [*] code: `ShmState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wl_seat`
  - [*] code: `SeatState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wl_output`
  - [*] code: `Output` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wl_data_device_manager`
  - [*] code: `DataDeviceState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `xdg_wm_base`
  - [*] code: `XdgShellState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zxdg_output_manager_v1`
  - [*] code: `OutputManagerState::new_with_xdg_output` builds
  - [*] e2e: advertised in `wayland-info`

Implemented extensions:
- [*] `zxdg_decoration_manager_v1`
  - [*] code: `XdgDecorationState` + handler (CSD) builds
  - [*] e2e: advertised in `wayland-info`
- [*] `xdg_activation_v1`
  - [*] code: `XdgActivationState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wp_viewporter`
  - [*] code: `ViewporterState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wp_fractional_scale_manager_v1`
  - [*] code: `FractionalScaleManagerState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wp_single_pixel_buffer_manager_v1`
  - [*] code: `SinglePixelBufferState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wp_presentation`
  - [*] code: `PresentationState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wp_content_type_manager_v1`
  - [*] code: `ContentTypeState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wp_alpha_modifier_v1`
  - [*] code: `AlphaModifierState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `wp_cursor_shape_manager_v1`
  - [*] code: `CursorShapeManagerState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_primary_selection_device_manager_v1`
  - [*] code: `PrimarySelectionState` + handler + focus wiring builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_keyboard_shortcuts_inhibit_manager_v1`
  - [*] code: `KeyboardShortcutsInhibitState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_relative_pointer_manager_v1`
  - [*] code: `RelativePointerManagerState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_pointer_constraints_v1`
  - [*] code: `PointerConstraintsState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_pointer_gestures_v1`
  - [*] code: `PointerGesturesState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_text_input_manager_v3`
  - [*] code: `TextInputManagerState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_tablet_manager_v2`
  - [*] code: `TabletManagerState` + `TabletSeatHandler` builds
  - [*] e2e: advertised in `wayland-info`

Newly wired (supported by smithay):
- [*] `zwp_idle_inhibit_manager_v1`
  - [*] code: `IdleInhibitManagerState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `ext_idle_notifier_v1`
  - [*] code: `IdleNotifierState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwlr_data_control_manager_v1`
  - [*] code: `DataControlState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `ext_data_control_manager_v1`
  - [*] code: ext data-control global + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `ext_foreign_toplevel_list_v1`
  - [*] code: `ForeignToplevelListState` + handler builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_virtual_keyboard_manager_v1`
  - [*] code: `VirtualKeyboardManagerState` global builds
  - [*] e2e: advertised in `wayland-info`
- [*] `zwp_input_method_manager_v2`
  - [*] code: `InputMethodManagerState` + handler builds
  - [*] e2e: advertised in `wayland-info`

Pending — global is easy but needs compositing of their surfaces to be useful:
- [*] `zwlr_layer_shell_v1`
  - [*] code: `WlrLayerShellState` + `WlrLayerShellHandler` build (new/destroy layer, commit arrange + initial configure)
  - [*] e2e: advertised in `wayland-info` (via `scripts/protocol-check.sh`)
  - [*] render: layer surfaces composited into the framebuffer (background/bottom under windows, top/overlay over; positioned via `layer_map` geometry; frame callbacks sent)

Hand-rolled via smithay `Dispatch2`/`GlobalDispatch2` (bindings re-exported from
`wayland-protocols`/`wayland-protocols-wlr`; `labwc/` used as the behavioural reference).
All four verified advertised + crash-free with a real client via `scripts/protocol-check.sh`
(release binary; the debug build trips a WSL1-only rustix cmsg overflow check in the receive path):
- [*] `wp_tearing_control_manager_v1` (labwc `src/tearing.c`)
  - [*] code: manager + `wp_tearing_control_v1` child; accept and ignore the presentation hint (no tearing on the shm path)
  - [*] e2e: advertised in `wayland-info`
- [*] `ext_session_lock_manager_v1` (labwc `src/session-lock.c`)
  - [*] code: `lock` stores the lock; `get_lock_surface` sends `configure`; first buffer commit sends `locked`; `unlock_and_destroy` clears the lock
  - [*] e2e: advertised in `wayland-info`
  - [*] render: output blanked and the lock surface composited over it; keyboard focus routed to the lock surface and normal-client focus changes suppressed while locked
- [*] `zwlr_foreign_toplevel_manager_v1` (labwc `src/foreign-toplevel/`)
  - [*] code: emit `toplevel` + title/app_id/state/done per window; `activate` raises+focuses, `close` sends `xdg_toplevel.close`; maximize/minimize/fullscreen/rectangle accepted
  - [*] e2e: advertised in `wayland-info`
  - [*] lifecycle: handles track map (`new_toplevel`), title/app-id changes (commit), and `closed` on unmap (`toplevel_destroyed`)
- [*] `zwlr_output_manager_v1` (labwc `src/output-state.c`, `output-virtual.c`)
  - [*] code: advertise one head + mode (name/description/size/refresh/enabled/current_mode/position/transform/scale/make/model) then `done`
  - [*] e2e: advertised in `wayland-info`
  - [*] config: `create_configuration` -> `test`/`apply` reply `failed` (single fixed virtual output)

Blocked (impossible on WSL1 or needs a model the bridge compositor lacks):
- [ ] `zwp_linux_dmabuf_v1` — blocked: no GPU/DRM in WSL1 (software `wl_shm` only)
- [ ] `zwp_linux_explicit_synchronization_v1` — blocked: GPU fence sync; N/A on WSL1
- [ ] `ext_workspace_manager_v1` — deferred: needs a real workspace model not present in this bridge compositor

#### Nested desktop / compositor bring-up (run on top of WWC)

Full compositors run as nested Wayland clients of this bridge compositor: they connect to
`WAYLAND_DISPLAY` and present into the shared framebuffer. Install them with
`install-desktops.sh` (Linux/WSL, or Git Bash which re-dispatches into WSL) or
`install-desktops.cmd` (Windows -> `WWC-WSL1`). Check a box once the compositor launches and
renders at least one frame through WWC. Availability depends on the distro release (several are
only packaged on newer Ubuntu/Debian).

Verified nested on `Ubuntu-Latest-WSL1` (Ubuntu 24.04 "noble"), 2026-10-04. Launch any of them
with the chooser `run-desktop.sh` (writes configs + starts WWC), then `win-host` on Windows.
Software rendering only (`WLR_RENDERER=pixman`, `KWIN_COMPOSE=Q`); a D-Bus session is started
via `dbus-run-session` so panels (waybar) work. WSL1 lacks `memfd_create`, so `run-desktop.sh`
builds and `LD_PRELOAD`s a shim ([shims/memfd_shim.c](shims/memfd_shim.c)) that falls back to an
unlinked temp file — this unlocks KWin, `foot`, and GTK4/Qt apps.

- [*] Sway (`sway`) — full desktop: waybar panel, `☰ Apps` menu (wofi), floating/movable windows; apps verified (galculator, thunar, xfce4-terminal+htop, mousepad)
- [*] Labwc (`labwc`) — floating WM + right-click app menu + waybar panel; renders through WWC
- [*] Weston (`weston`) — nested `--backend=wayland --use-pixman`; own desktop shell/panel
- [*] Cage (`cage`) — kiosk (one fullscreen app); renders through WWC
- [*] KDE / KWin (`kwin-wayland`) — works via `KWIN_COMPOSE=Q` (software) + the memfd shim; `kwin_wayland` manages windows with decorations, renders through WWC (full `plasmashell` panel not started — heavy; Qt/KDE *apps* run under any desktop)
- [ ] Wayfire (`wayfire`) — installed but **infeasible on WSL1**: requires a DRM render node (`Failed to get DRM file descriptor`); no software-compositing mode
- [ ] Hyprland (`hyprland`) — not packaged on noble, and like wayfire needs DRM/GPU GL (no software mode) — infeasible on WSL1 regardless
- [ ] dwl (`dwl`) — not packaged on noble; wlroots-thin, so a source build would run with `WLR_RENDERER=pixman` (needs matching `libwlroots-dev`)
- [ ] River (`river`) — not packaged on noble; wlroots-thin, would run with pixman if built (needs the `zig` toolchain)
- [ ] Niri (`niri`) — not packaged on noble; smithay-based, would run nested via its winit backend if built (`cargo install niri` + build deps)

Installer: `install-desktops.sh`/`.cmd` now has an interactive chooser (numbers / `a` all /
`r` WSL1-recommended) and still honours `ONLY="sway labwc ..."` for automation.

### Phase 7 — WebGPU GPU acceleration path

Two meanings of "GPU support", with very different feasibility on WSL1:

**Client 3D (OpenGL/Vulkan for apps, games, wayfire/hyprland) — NOT possible on WSL1.** WSL1 has
no GPU device: no `/dev/dri` render node and no `/dev/dxg` (the paravirtual GPU that only *WSL2*
exposes via `dxgkrnl` + Mesa's `d3d12`/Dozen drivers). WebGPU cannot tunnel arbitrary GL/Vulkan
from guest to host. So GL-only compositors (wayfire, hyprland) and GPU apps can't be accelerated
here; the only route to real client GPU is running the compositor under **WSL2**, where
`/dev/dxg` provides a render node.

**Host-side compositing/presentation via WebGPU — feasible, and partly done.** `win-host` already
uploads the shared framebuffer to a texture and scales/presents it on the GPU. The next step is to
move *compositing itself* onto the host GPU:

- [*] Present path on GPU — texture upload + fullscreen-quad sampling + scaling (existing).
- [*] Protocol foundation — `SurfaceQuad` + `ServerMessage::GpuScene` (back-to-front surface list
  with geometry/opacity), clip-space mapping, encode/decode + unit tests. Additive, non-breaking.

**Remaining WebGPU work — finish this before starting Phase 8 (the `ash` backend).** Each item
lists the concrete code steps and how to verify it.

- [-] Shared-memory **surface pool** (`bridge-protocol` + `wsl-compositor`)
  - [*] `bridge-protocol`: pool layout after the framebuffer slots. `SurfacePool`
    (`region_count`, `region_stride`, `pool_size`, `pool_region`) with [`POOL_ALIGN`] (4 KiB)
    regions; per-region `RegionHeader { used, seq, width, height, stride }` (read/write +
    `region_pixels[_mut]`); and `SharedLayout { frame, pool }` giving file-absolute
    `pool_base`/`region_file_range` (what `SurfaceQuad.pool_offset` carries).
  - [*] `bridge-protocol`: `SurfacePool::pool_region(idx) -> Option<(offset, len)>` + unit tests
    for bounds, 4 KiB alignment, stride round-up, offset round-trips, region-header round-trip,
    and the composed file layout (36 tests total, green).
  - [ ] `wsl-compositor`: size the shared file via `SharedLayout`; on commit, copy each mapped
    client `wl_shm` buffer into a pool region once per frame (BGRA, keep `src_stride`, bump
    `seq`), tracking a stable surface→region mapping so an unchanged surface keeps its offset.
  - [ ] Verify: `cargo test -p bridge-protocol` (done, green); dump the pool from `fb-dump` and
    confirm a known client's pixels appear at the advertised `pool_offset` (pending the live map).
- [*] `win-host` **consume `GpuScene`** (`main.rs`) — the `ServerMessage::GpuScene` arm now maps
  the shared pool read-only and CPU-composites the back-to-front quads via
  `bridge_protocol::composite_scene(out_w, out_h, pool, quads)`, then uploads the result through
  the normal present path. Backend-agnostic (works for both `wgpu` and `vulkan`); verified by
  `composite_scene` unit tests (layering, front-overwrite, empty-clear) + `win-host` build.
- [ ] `win-host` **on-GPU compositor** (`gpu.rs`) — optimization over the CPU composite above:
  - [ ] Build a per-surface texture cache keyed by `pool_offset`, (re)uploading only when
    `src_w/src_h` or the region `seq` changed (damage-aware); evict entries absent from the scene.
  - [ ] Replace the single fullscreen blit with a loop: for each `SurfaceQuad` back-to-front, set
    a per-quad uniform (`clip_rect()` + `opacity`) and `draw(0..4)` a textured quad.
  - [ ] Enable alpha blending on the color target (`BlendState::ALPHA_BLENDING`) and multiply
    sampled alpha by `opacity` in the fragment shader (new `scene.wgsl` or a branch in
    `shader.wgsl`).
  - [ ] Verify: drive a synthetic two-surface `GpuScene` and snapshot; overlapping quads blend in
    the correct back-to-front order.
- [ ] Compositor `--gpu-composite` mode (`wsl-compositor`)
  - [ ] Behind a CLI flag, publish a `GpuScene` (pool copies + quad list) each frame instead of the
    pre-composited whole-output blit; the software path stays the default/fallback.
  - [ ] Verify: `scripts/protocol-check.sh`-style run with `--gpu-composite` + `win-host`; windows
    render through the per-surface path with identical pixels to the software path.
- [ ] GPU effects from per-surface quads — opacity, viewporter/fractional scaling, output
  transforms, done on the host GPU instead of the WSL1 CPU (fold the viewport src/dst crop and
  surface scale into the per-quad uniform).

Benefit: offloads compositing from the software-only WSL1 CPU to the Windows GPU and enables
blending/scaling/effects. It does **not** give Wayland clients their own GPU — that stays a
WSL2-only capability.

### Phase 8 — Second render backend: `gpu-allocator` + `ash` (raw Vulkan)

**Finish all remaining Phase 7 WebGPU items before starting this phase.** Phase 8 does not
replace `wgpu`; it adds a *second, selectable* host renderer so the WSL1 Wayland compositor's
host side has **full, explicit control** over the GPU.

#### 8.0 Why a second backend (rationale)

`wgpu` is a high-level, safe abstraction: it hides `VkDeviceMemory` allocation, descriptor
pools, image-layout transitions, queue submission and swapchain synchronization. That is ideal
for a portable present path, but it caps how much the compositor can control memory placement,
upload strategy, aliasing/tiling, and present timing. A raw-Vulkan backend gives that control:

- **`ash`** — thin, unsafe FFI bindings to the Vulkan API (no abstraction, 1:1 with the C API).
  We own instance/device creation, swapchain, command buffers, pipelines, synchronization.
- **`gpu-allocator`** — a sub-allocator for `VkDeviceMemory` (the piece `ash` deliberately does
  *not* provide). It manages memory types, suballocation, and host-visible vs device-local
  placement, so we can map the shared surface pool as host-visible staging and keep per-surface
  images device-local — the "full abstract control" the compositor wants over GPU memory.

Target: the `vulkan` backend reaches **feature parity with the `wgpu` present + GPU-compositor
path** (whole-framebuffer blit *and* per-surface `GpuScene` compositing), selectable at runtime.

**Status (whole-framebuffer present path implemented).** The `--backend {webgpu|vulkan}`
parameter is wired through **both** processes and defaults to `webgpu`: `wsl-compositor
--backend …` advertises the choice in the `FrameConfig` handshake (`bridge_protocol::Backend`),
and `win-host --backend …` selects the matching renderer (the compositor's value is
authoritative if they differ). The Vulkan whole-output present is implemented with
**`vkCmdBlitImage`** (hardware-scaled transfer blit) rather than a graphics pipeline + shaders:
the whole-framebuffer blit needs no pipeline/descriptors/SPIR-V, which keeps the backend small
and avoids a host shader-toolchain dependency. A graphics pipeline is only needed for the
per-surface `GpuScene` compositor (8.9), which is still pending.

**Runtime validation (2026-10-05, both backends, real hardware).** Host: NVIDIA RTX 3060 Laptop
+ Intel Iris Xe (Vulkan 1.3 ICDs, `VK_KHR_win32_surface`). Compositor ran in `WWC-WSL1` (WSL1)
serving `weston-simple-shm` frames over `127.0.0.1:7901`, shared file on `/mnt/c`.
- **webgpu**: `win-host --backend webgpu` → `renderer: webgpu (wgpu)` (wgpu Vulkan HAL),
  connected, handshake `backend=webgpu`, uploaded/presented 800×600 frames, no render errors.
- **vulkan**: `win-host --backend vulkan` (built `--features vulkan`) → `vulkan adapter: Intel(R)
  Iris(R) Xe Graphics`, `renderer: vulkan (ash + gpu-allocator)`, connected, handshake
  `backend=vulkan`; the `ash` instance/surface/device/swapchain + `gpu-allocator` memory + upload
  (`vkCmdCopyBufferToImage`) + present (`vkCmdBlitImage`) loop ran against the live frame stream
  with no crashes/errors. Confirms the single `--backend` parameter drives both sides end-to-end.

#### 8.1 Backend abstraction (prerequisite refactor in `win-host`)

- [*] Define a `Renderer` trait capturing what `main.rs`/`bridge.rs` need, matching today's
  `GpuState`: `window()`, `resize(w,h)`, `window_size()`, `tex_size()`, `upload(w,h,bgra)`,
  `render()` (returns a backend-agnostic `RenderOutcome`). `render_scene(...)` lands with 8.9.
- [*] `GpuState` (the `wgpu` renderer) now implements `Renderer`; `backend/mod.rs` adds the
    `Renderer` trait + `RenderOutcome` + a `create(Backend, window)` factory (falls back to
    `wgpu` with a warning if Vulkan is unavailable).
- [*] Added `--backend {webgpu|vulkan}` to `args.rs` (default `webgpu`), threaded into the
    factory. `cargo build -p win-host` + the existing `wgpu` path unchanged (26 tests pass).

#### 8.2 Dependencies & feature gating

- [*] Added to `win-host/Cargo.toml` behind a `vulkan` cargo feature: `ash = "0.38"`,
    `ash-window = "0.13"`, `gpu-allocator = "0.27"` (vulkan feature),
    `raw-window-handle = "0.6"`. `wgpu` stays always-on as the fallback.
- [*] Shader toolchain decision: **none needed** for the present path — `vkCmdBlitImage` does the
    scaled copy on the transfer unit, so no GLSL/SPIR-V/`shaderc`. (Revisit for 8.9: the scene
    compositor will need a textured-quad pipeline; generate SPIR-V via `naga` at runtime to avoid
    an external toolchain, since the host has no Vulkan SDK/`glslc`.)

#### 8.3 Instance, surface, physical device, queues (`backend/vulkan.rs`)

- [*] Create `VkInstance` via `ash::Entry` with the surface extensions from
    `ash_window::enumerate_required_extensions`. (Validation layer/debug messenger: deferred to
    8.10.)
- [*] Create the `VkSurfaceKHR` from the `winit` window via `ash-window` (rwh 0.6 handles).
- [*] Enumerate physical devices; pick the first with a graphics **and** present queue family,
    logging the adapter name. (Discrete-GPU preference: future refinement.)
- [*] Create the logical `VkDevice` with `VK_KHR_swapchain` + one graphics/present queue; store
    `Entry`, `Instance`, `Device`, `PhysicalDevice`, queue.
- [*] Verify: compiles with `--features vulkan`; logs the selected adapter at runtime.

#### 8.4 `gpu-allocator` integration (memory ownership)

- [*] Construct `gpu_allocator::vulkan::Allocator` from the instance/device/physical-device —
    the single owner of all non-swapchain `VkDeviceMemory`.
- [*] Helpers `alloc_image` (device-local `GpuOnly`) and `ensure_staging` (host-visible
    `CpuToGpu`, persistently mapped) allocate via the allocator and bind the handle.
- [*] Centralized teardown: `free_texture`/`free_staging` return allocations to the allocator,
    then `Drop` drops the allocator **before** destroying the device (ordered teardown).
- [ ] Verify with validation enabled — zero leak/teardown-order errors (pending 8.10, needs a
    Vulkan runtime on the host).

#### 8.5 Swapchain & per-frame sync (`backend/vulkan.rs`)

- [*] Create `VkSwapchainKHR` sized to the window, choosing `B8G8R8A8_UNORM` (non-sRGB, matches
    the `wgpu` format), `FIFO` present mode, `min_image_count+1` images, with
    `TRANSFER_DST` image usage (we clear + blit into the swapchain image). No image views /
    render pass are needed for the blit present.
- [*] Per-frame objects: `image_available` + `render_finished` semaphores, an `in_flight` fence,
    a command pool + primary command buffers (frame + upload).
- [*] `resize(w,h)`: wait idle and recreate the swapchain (`old_swapchain` reuse); acquire/
    present handle `ERROR_OUT_OF_DATE_KHR`/`SUBOPTIMAL` by reporting `RenderOutcome::Lost` so
    `main` resizes.
- [*] Verified on a real GPU (Intel Iris Xe, 2026-10-05): clears + presents live frames without
    validation/runtime errors.

#### 8.6 Textures & uploads (parity with `GpuState::upload`)

- [*] `alloc_image(w,h)`: device-local `VkImage` (`B8G8R8A8_UNORM`, `TRANSFER_SRC | TRANSFER_DST`
    so it can be uploaded to and then blitted from), (re)created when the size changes.
- [*] `upload(w,h,bgra)`: copy into the persistently-mapped host-visible staging buffer, then
    `vkCmdCopyBufferToImage` with explicit `UNDEFINED→TRANSFER_DST→TRANSFER_SRC` barriers
    (`bufferRowLength` carries the row stride). No sampler needed (the blit path samples via the
    transfer unit, not a shader).
- [*] Verified on a real GPU (2026-10-05): the whole-framebuffer upload path ran against live
    `weston-simple-shm` frames with no errors (pixel-diff vs `wgpu` snapshot still TODO in 8.10).

#### 8.7 Pipelines & descriptors (`backend/vulkan.rs`) — deferred to the scene compositor

Not required for the whole-framebuffer present path (which uses `vkCmdBlitImage`). These land
with 8.9:
- [ ] **Scene pipeline** — a textured-quad pipeline with `ALPHA_BLENDING`; per-quad data
    (`clip_rect()` corners + `opacity`) via push constants; the per-surface image bound via a
    descriptor set. Generate SPIR-V via `naga` at runtime (no host toolchain).
- [ ] Descriptor infrastructure: set layout (sampled image + sampler), descriptor pool, and
    per-texture descriptor sets.

#### 8.8 Whole-framebuffer present path (parity with `GpuState::render`)

- [*] Acquire next image, wait/reset the in-flight fence, record: barrier swapchain image to
    `TRANSFER_DST`, `vkCmdClearColorImage` to `(0.02,0.02,0.02,1)`, `vkCmdBlitImage` the
    uploaded texture into the aspect-fit rect (`LINEAR` filter), barrier to `PRESENT_SRC`,
    submit with acquire/submit semaphores, present. Aspect-fit math (`aspect_fit`) unit-tested.
- [*] Verified on a real GPU (2026-10-05): the acquire→clear→blit→present loop ran crash-free
    against live frames; side-by-side pixel-diff vs the `wgpu` blit still TODO (8.10).

#### 8.9 GPU-scene compositor (parity with Phase 7 `render_scene`)

- [ ] Map the shared **surface pool** read-only; maintain a per-`pool_offset` image cache with
    damage-aware re-upload and eviction (same policy as the `wgpu` compositor).
- [ ] `render_scene(pool, quads, out)`: for each `SurfaceQuad` back-to-front, draw a textured
    quad with `ALPHA_BLENDING` (needs the 8.7 pipeline). Add `render_scene` to the `Renderer`
    trait + both backends once the surface pool exists.
- [ ] Verify: synthetic two-surface `GpuScene` blends identically to the `wgpu` compositor;
    `--backend vulkan --gpu-composite` matches the software path pixel-for-pixel.

#### 8.10 Validation, testing & teardown

- [*] Pure helpers unit-tested without a device (`aspect_fit` letterbox/pillarbox/identity);
    Vulkan object creation/submission stays out of unit tests (needs a real device, like
    `gpu.rs`) — mirrors the §6 coverage-exclusion policy.
- [-] Run with `VK_LAYER_KHRONOS_validation` in debug and fix all errors/leaks; confirm clean
    teardown order (texture/staging via allocator → allocator → sync → pool → swapchain →
    device → surface → instance). Ran crash-free on real hardware (Intel Iris Xe, 2026-10-05)
    on the default (no-layer) path; an explicit validation-layer pass + leak check is still TODO.
- [ ] Add a `backend` comparison note: capture `wgpu` vs `vulkan` snapshots and diff.

#### 8.11 Docs & run integration

- [*] Documented `--backend vulkan` in §5 (both sides share one parameter; host-side only; needs
    a Vulkan ICD on Windows).
- [ ] Add the same note to `README.md`.
- [*] WSL1-first framing reconfirmed: this backend accelerates the **host** present/compositing
    path; it does **not** add client GPU to WSL1 and is **not** a reason to move to WSL2.

## 5. How to run (target workflow)

1. On **Windows**: `cargo run -p win-host -- --shm C:\Users\<you>\AppData\Local\Temp\wwc.fb`
   - [*] Add `--backend vulkan` to use the raw-Vulkan (`ash` + `gpu-allocator`) host renderer
     instead of the default `wgpu` one (requires building with `--features vulkan` and a Vulkan
     ICD on Windows). The same `--backend` is a single logical parameter shared by both sides.
2. In **WSL1**: `cargo run` inside `src/wsl-compositor` with
   `--shm /mnt/c/Users/<you>/AppData/Local/Temp/wwc.fb --host 127.0.0.1:8335`
   - Pass `--backend vulkan` here too; the compositor advertises it in the handshake and the
     host adopts it (the compositor's choice is authoritative). Defaults to `webgpu`.
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

