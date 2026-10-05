//! Shared wire protocol bridging the WSL1 Wayland compositor (Linux ELF) and the Windows
//! WebGPU host (native PE). Two transports are defined:
//!
//! 1. A **shared-memory framebuffer** laid out inside a memory-mapped file that lives on a
//!    Windows path (and is therefore visible from WSL1 under `/mnt/<drive>/...`). Both
//!    processes map the same file; on WSL1 the pages are backed by the same NT file object,
//!    giving true zero-copy shared memory.
//! 2. A **localhost TCP control channel** carrying length-prefixed binary messages for
//!    input, resize and handshake. WSL1 shares the Windows loopback interface, so this is a
//!    direct `127.0.0.1` connection with no NAT hop.
//!
//! The crate is dependency-free `std` Rust and compiles identically on Linux and Windows.

use std::io::{self, Read, Write};

/// File magic: ASCII "WWCF" (Wayland→WebGPU Composited Framebuffer), little-endian.
pub const MAGIC: u32 = 0x4643_5757; // bytes: 'W','W','C','F'
/// Shared-memory layout version. Bump on any incompatible header/slot change.
pub const VERSION: u32 = 1;
/// Number of framebuffer slots used for double buffering.
pub const SLOT_COUNT: u32 = 2;
/// Bytes reserved for the header at the start of the shared file.
pub const HEADER_SIZE: usize = 64;

/// Default base port for the localhost TCP control channel. When no explicit address is
/// given, both sides start here and scan upward: the compositor binds the first free port,
/// the host connects to the first port that accepts.
pub const DEFAULT_PORT: u16 = 8335;
/// Number of consecutive ports to try when scanning from [`DEFAULT_PORT`].
pub const PORT_SCAN_COUNT: u16 = 64;

/// Build the ordered list of `127.0.0.1:<port>` addresses to scan when no explicit address
/// is supplied: `DEFAULT_PORT, DEFAULT_PORT+1, …` for [`PORT_SCAN_COUNT`] ports.
pub fn default_scan_addrs() -> Vec<String> {
    let end = DEFAULT_PORT.saturating_add(PORT_SCAN_COUNT);
    (DEFAULT_PORT..end)
        .map(|p| format!("127.0.0.1:{p}"))
        .collect()
}

// --- Header field byte offsets (all little-endian) ---
const OFF_MAGIC: usize = 0; // u32
const OFF_VERSION: usize = 4; // u32
const OFF_WIDTH: usize = 8; // u32
const OFF_HEIGHT: usize = 12; // u32
const OFF_STRIDE: usize = 16; // u32 (bytes per row)
const OFF_FORMAT: usize = 20; // u32 (PixelFormat)
const OFF_SLOT_COUNT: usize = 24; // u32
const OFF_ACTIVE_SLOT: usize = 28; // u32 (slot holding the latest complete frame)
const OFF_FRAME_SEQ: usize = 32; // u64 (monotonic frame counter)
const OFF_SLOT_BYTES: usize = 40; // u64 (bytes per slot)

/// Pixel byte order stored in the shared framebuffer. `wl_shm` `ARGB8888` is a 32-bit
/// little-endian word `0xAARRGGBB`, i.e. the bytes `B, G, R, A` in memory, which maps
/// directly to WebGPU `Bgra8Unorm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    Bgra8Unorm = 0,
}

impl PixelFormat {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(PixelFormat::Bgra8Unorm),
            _ => None,
        }
    }
}

/// Host render backend selected on the command line. This is a single logical parameter that
/// is threaded through **both** sides: the compositor (`wsl-compositor --backend …`) advertises
/// its choice in the [`ServerMessage::FrameConfig`] handshake, and the Windows host
/// (`win-host --backend …`) picks the matching renderer. Defaults to [`Backend::Webgpu`].
///
/// - [`Backend::Webgpu`] — the portable `wgpu` renderer (default).
/// - [`Backend::Vulkan`] — the raw-Vulkan renderer (`ash` + `gpu-allocator`), which gives the
///   compositor full, explicit control over GPU memory and the present path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Backend {
    #[default]
    Webgpu = 0,
    Vulkan = 1,
}

impl Backend {
    /// Map the wire byte to a backend. Unknown values fall back to the default ([`Backend::Webgpu`]).
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Backend::Vulkan,
            _ => Backend::Webgpu,
        }
    }

    /// The wire byte for this backend.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse a CLI value (`webgpu`/`wgpu` or `vulkan`/`ash`), case-insensitively. Returns
    /// `None` for an unrecognized value so callers can show usage.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "webgpu" | "wgpu" => Some(Backend::Webgpu),
            "vulkan" | "ash" | "gpu-allocator" => Some(Backend::Vulkan),
            _ => None,
        }
    }

    /// Lowercase name for logs/usage.
    pub fn name(self) -> &'static str {
        match self {
            Backend::Webgpu => "webgpu",
            Backend::Vulkan => "vulkan",
        }
    }
}

/// Convert a `/mnt/<drive>/a/b` WSL path into a `DRIVE:\a\b` Windows path. Returns the input
/// unchanged if it does not look like a `/mnt/<drive>/` path. This lets the WSL1 compositor
/// tell the Windows host where to open the shared framebuffer file.
pub fn wsl_path_to_windows(p: &str) -> String {
    let rest = match p.strip_prefix("/mnt/") {
        Some(r) => r,
        None => return p.to_string(),
    };
    let mut chars = rest.chars();
    let drive = match chars.next() {
        Some(d) if d.is_ascii_alphabetic() => d.to_ascii_uppercase(),
        _ => return p.to_string(),
    };
    // After the drive letter there must be a path separator (or end of string).
    let after_drive = chars.as_str();
    if !after_drive.is_empty() && !after_drive.starts_with('/') {
        return p.to_string();
    }
    let tail = after_drive.trim_start_matches('/').replace('/', "\\");
    format!("{drive}:\\{tail}")
}

/// Geometry of the shared framebuffer. 4 bytes per pixel, tightly packed rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameLayout {
    pub width: u32,
    pub height: u32,
}

impl FrameLayout {
    pub const BYTES_PER_PIXEL: u32 = 4;

    pub fn new(width: u32, height: u32) -> Self {
        FrameLayout { width, height }
    }

    /// Bytes per row.
    pub fn stride(&self) -> u32 {
        self.width * Self::BYTES_PER_PIXEL
    }

    /// Bytes in a single framebuffer slot.
    pub fn slot_bytes(&self) -> u64 {
        self.stride() as u64 * self.height as u64
    }

    /// Total size of the shared file: header + all slots.
    pub fn total_size(&self) -> u64 {
        HEADER_SIZE as u64 + self.slot_bytes() * SLOT_COUNT as u64
    }

    /// Byte offset of `slot` within the shared file.
    pub fn slot_offset(&self, slot: u32) -> usize {
        HEADER_SIZE + (self.slot_bytes() * slot as u64) as usize
    }
}

/// Alignment of each surface-pool region, in bytes. Regions start on this boundary so a host
/// GPU can upload each one without sub-row copy fixups.
pub const POOL_ALIGN: u64 = 4096;

/// Geometry of the shared-memory **surface pool** used by the GPU-composite path (plan.md
/// Phase 7/8). The pool is a flat run of `region_count` equal-sized regions; the compositor
/// copies each client `wl_shm` buffer into one region per frame and references it from a
/// [`SurfaceQuad::pool_offset`]. Regions are [`POOL_ALIGN`]-aligned.
///
/// This type is pure layout math (no I/O), so it unit-tests on any platform. The live mapping
/// (appending the pool after the framebuffer slots in the shared file) is wired separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfacePool {
    region_count: u32,
    region_stride: u64,
}

impl SurfacePool {
    /// Build a pool of `region_count` regions, each large enough for `max_region_bytes`,
    /// rounded up to [`POOL_ALIGN`]. A `region_count` of 0 yields an empty pool.
    pub fn new(region_count: u32, max_region_bytes: u64) -> Self {
        SurfacePool {
            region_count,
            region_stride: align_up(max_region_bytes, POOL_ALIGN),
        }
    }

    /// Number of regions in the pool.
    pub fn region_count(&self) -> u32 {
        self.region_count
    }

    /// Bytes per region (always a multiple of [`POOL_ALIGN`]).
    pub fn region_stride(&self) -> u64 {
        self.region_stride
    }

    /// Total bytes spanned by the pool: `region_stride * region_count`.
    pub fn pool_size(&self) -> u64 {
        self.region_stride * self.region_count as u64
    }

    /// `(offset, len)` of region `idx` relative to the start of the pool, or `None` if `idx`
    /// is out of range. `offset` is [`POOL_ALIGN`]-aligned.
    pub fn pool_region(&self, idx: u32) -> Option<(u64, u64)> {
        if idx >= self.region_count {
            return None;
        }
        Some((idx as u64 * self.region_stride, self.region_stride))
    }
}

/// Bytes reserved at the start of each pool region for its [`RegionHeader`]. Pixel data begins
/// at this offset within the region. Kept a power of two so pixel rows stay naturally aligned.
pub const POOL_REGION_HEADER: usize = 32;

// --- Region header field offsets (little-endian, within a region) ---
const RH_USED: usize = 0; // u32: 0 = free, 1 = holds a live surface this frame
const RH_SEQ: usize = 4; // u32: bumped whenever the pixels change (host damage key)
const RH_WIDTH: usize = 8; // u32
const RH_HEIGHT: usize = 12; // u32
const RH_STRIDE: usize = 16; // u32: bytes per row of the pixel payload

/// Per-region bookkeeping the compositor writes and the host reads. `used`/`seq` drive the
/// host's damage-aware per-surface texture cache; `width`/`height`/`stride` describe the BGRA
/// payload that follows the header in the region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionHeader {
    pub used: bool,
    pub seq: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

impl RegionHeader {
    /// Write this header into the first [`POOL_REGION_HEADER`] bytes of `region`. No-op if the
    /// slice is too short.
    pub fn write(&self, region: &mut [u8]) {
        if region.len() < POOL_REGION_HEADER {
            return;
        }
        write_u32(region, RH_USED, self.used as u32);
        write_u32(region, RH_SEQ, self.seq);
        write_u32(region, RH_WIDTH, self.width);
        write_u32(region, RH_HEIGHT, self.height);
        write_u32(region, RH_STRIDE, self.stride);
    }

    /// Read a header from the first [`POOL_REGION_HEADER`] bytes of `region`, or `None` if the
    /// slice is too short.
    pub fn read(region: &[u8]) -> Option<Self> {
        if region.len() < POOL_REGION_HEADER {
            return None;
        }
        Some(RegionHeader {
            used: read_u32(region, RH_USED) != 0,
            seq: read_u32(region, RH_SEQ),
            width: read_u32(region, RH_WIDTH),
            height: read_u32(region, RH_HEIGHT),
            stride: read_u32(region, RH_STRIDE),
        })
    }

    /// Bytes of BGRA payload this header describes (`stride * height`).
    pub fn payload_len(&self) -> usize {
        self.stride as usize * self.height as usize
    }
}

/// Borrow the BGRA pixel payload of a region (the bytes after [`POOL_REGION_HEADER`]), or
/// `None` if the region is too short to hold the header.
pub fn region_pixels(region: &[u8]) -> Option<&[u8]> {
    region.get(POOL_REGION_HEADER..)
}

/// Mutably borrow the BGRA pixel payload of a region (for the compositor to fill).
pub fn region_pixels_mut(region: &mut [u8]) -> Option<&mut [u8]> {
    region.get_mut(POOL_REGION_HEADER..)
}

/// Full shared-file layout: the double-buffered framebuffer slots (the pre-composited path),
/// immediately followed by the optional [`SurfacePool`] (the GPU-composite path). Both sides
/// size and map the file from this so `pool_offset` references are file-absolute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedLayout {
    pub frame: FrameLayout,
    pub pool: SurfacePool,
}

impl SharedLayout {
    pub fn new(frame: FrameLayout, pool: SurfacePool) -> Self {
        SharedLayout { frame, pool }
    }

    /// File-absolute byte offset where the surface pool begins (right after the framebuffer).
    pub fn pool_base(&self) -> u64 {
        self.frame.total_size()
    }

    /// Total shared-file size: framebuffer + pool.
    pub fn total_size(&self) -> u64 {
        self.pool_base() + self.pool.pool_size()
    }

    /// File-absolute `(offset, len)` of pool region `idx`, or `None` if out of range. The
    /// `offset` is what a [`SurfaceQuad::pool_offset`] carries.
    pub fn region_file_range(&self, idx: u32) -> Option<(u64, u64)> {
        let (rel, len) = self.pool.pool_region(idx)?;
        Some((self.pool_base() + rel, len))
    }
}

/// Round `v` up to the next multiple of `align` (which must be non-zero).
#[inline]
fn align_up(v: u64, align: u64) -> u64 {
    debug_assert!(align != 0);
    v.div_ceil(align) * align
}

#[inline]
fn write_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn read_u32(buf: &[u8], off: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&buf[off..off + 4]);
    u32::from_le_bytes(b)
}

#[inline]
fn write_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn read_u64(buf: &[u8], off: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[off..off + 8]);
    u64::from_le_bytes(b)
}

/// Writer-side view over the mapped shared framebuffer (used by the compositor).
pub struct SharedFramebuffer<'a> {
    map: &'a mut [u8],
    layout: FrameLayout,
}

impl<'a> SharedFramebuffer<'a> {
    /// Initialize the header of a freshly mapped region. Writes magic/version/geometry and
    /// resets the frame sequence. The region must be at least `layout.total_size()` bytes.
    pub fn initialize(map: &'a mut [u8], layout: FrameLayout) -> Self {
        assert!(map.len() as u64 >= layout.total_size(), "mapping too small");
        write_u32(map, OFF_MAGIC, MAGIC);
        write_u32(map, OFF_VERSION, VERSION);
        write_u32(map, OFF_WIDTH, layout.width);
        write_u32(map, OFF_HEIGHT, layout.height);
        write_u32(map, OFF_STRIDE, layout.stride());
        write_u32(map, OFF_FORMAT, PixelFormat::Bgra8Unorm as u32);
        write_u32(map, OFF_SLOT_COUNT, SLOT_COUNT);
        write_u32(map, OFF_ACTIVE_SLOT, 0);
        write_u64(map, OFF_FRAME_SEQ, 0);
        write_u64(map, OFF_SLOT_BYTES, layout.slot_bytes());
        SharedFramebuffer { map, layout }
    }

    /// Wrap an already-initialized mapping without rewriting the header or resetting the
    /// frame sequence. Use this each frame after a one-time [`initialize`](Self::initialize).
    pub fn attach(map: &'a mut [u8], layout: FrameLayout) -> Self {
        SharedFramebuffer { map, layout }
    }

    pub fn layout(&self) -> FrameLayout {
        self.layout
    }

    fn active_slot(&self) -> u32 {
        read_u32(self.map, OFF_ACTIVE_SLOT)
    }

    /// Mutable pixel bytes of the slot that is currently NOT the active (published) one.
    pub fn back_slot_mut(&mut self) -> (u32, &mut [u8]) {
        let back = (self.active_slot() + 1) % SLOT_COUNT;
        let start = self.layout.slot_offset(back);
        let end = start + self.layout.slot_bytes() as usize;
        (back, &mut self.map[start..end])
    }

    /// Publish the just-written back slot: make it active and bump the frame sequence.
    /// Returns the new frame sequence number. The sequence is written last so a reader that
    /// observes the new sequence is guaranteed to see a fully written `active_slot`.
    pub fn publish(&mut self, slot: u32) -> u64 {
        let seq = read_u64(self.map, OFF_FRAME_SEQ) + 1;
        write_u32(self.map, OFF_ACTIVE_SLOT, slot);
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
        write_u64(self.map, OFF_FRAME_SEQ, seq);
        seq
    }
}

/// Reader-side view over the mapped shared framebuffer (used by the Windows host).
pub struct SharedFramebufferReader<'a> {
    map: &'a [u8],
}

impl<'a> SharedFramebufferReader<'a> {
    /// Validate the header and wrap a read-only mapping. Returns `None` if magic/version
    /// do not match (e.g. the compositor has not initialized the file yet).
    pub fn new(map: &'a [u8]) -> Option<Self> {
        if map.len() < HEADER_SIZE {
            return None;
        }
        if read_u32(map, OFF_MAGIC) != MAGIC || read_u32(map, OFF_VERSION) != VERSION {
            return None;
        }
        Some(SharedFramebufferReader { map })
    }

    pub fn layout(&self) -> FrameLayout {
        FrameLayout::new(read_u32(self.map, OFF_WIDTH), read_u32(self.map, OFF_HEIGHT))
    }

    pub fn format(&self) -> Option<PixelFormat> {
        PixelFormat::from_u32(read_u32(self.map, OFF_FORMAT))
    }

    /// Monotonic frame counter. Increment indicates a new published frame.
    pub fn frame_seq(&self) -> u64 {
        let seq = read_u64(self.map, OFF_FRAME_SEQ);
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        seq
    }

    /// Pixel bytes of the currently published slot.
    pub fn active_pixels(&self) -> &'a [u8] {
        let layout = self.layout();
        let slot = read_u32(self.map, OFF_ACTIVE_SLOT);
        let start = layout.slot_offset(slot);
        let end = start + layout.slot_bytes() as usize;
        &self.map[start..end]
    }
}

// =====================================================================================
// Software blit
// =====================================================================================

/// Copy a source BGRA image into a destination BGRA framebuffer at `(dst_x, dst_y)`,
/// clipping to the destination bounds. Pixels are copied verbatim (no blending), which is
/// correct for opaque surfaces.
///
/// - `dst` is `dst_w * dst_h * 4` bytes, tightly packed.
/// - `src` is the raw buffer; pixel data for row `r` begins at `src_offset + r*src_stride`.
/// - `src_w`/`src_h` are the source image dimensions in pixels, `src_stride` its row bytes.
///
/// Rows or columns that fall outside the destination (including from a negative origin) are
/// skipped. Reads/writes that would exceed `src`/`dst` lengths are skipped defensively.
#[allow(clippy::too_many_arguments)]
pub fn blit_bgra(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_offset: usize,
    src_w: u32,
    src_h: u32,
    src_stride: u32,
    dst_x: i32,
    dst_y: i32,
) {
    let dw = dst_w as i64;
    let dh = dst_h as i64;
    let sw = src_w as i64;
    let sh = src_h as i64;
    let stride = src_stride as i64;
    let off = src_offset as i64;

    for row in 0..sh {
        let dy = dst_y as i64 + row;
        if dy < 0 || dy >= dh {
            continue;
        }
        let src_x0 = if dst_x < 0 { (-(dst_x as i64)).min(sw) } else { 0 };
        let dst_x0 = if dst_x > 0 { (dst_x as i64).min(dw) } else { 0 };
        let copy_w = (sw - src_x0).min(dw - dst_x0);
        if copy_w <= 0 {
            continue;
        }
        let src_start = off + row * stride + src_x0 * 4;
        let src_end = src_start + copy_w * 4;
        if src_start < 0 || src_end as usize > src.len() {
            continue;
        }
        let dst_start = (dy * dw + dst_x0) * 4;
        let dst_end = dst_start + copy_w * 4;
        if dst_end as usize > dst.len() {
            continue;
        }
        dst[dst_start as usize..dst_end as usize]
            .copy_from_slice(&src[src_start as usize..src_end as usize]);
    }
}

/// CPU-composite a back-to-front list of [`SurfaceQuad`]s into a freshly allocated
/// `out_w * out_h` BGRA framebuffer. Each quad's pixels are read from `pool` starting at its
/// `pool_offset` (file-absolute when `pool` is the whole shared mapping). This is the
/// backend-agnostic consumer of a [`ServerMessage::GpuScene`]: the host composites on the CPU
/// then uploads the result through the normal present path, so it works identically for the
/// `wgpu` and `vulkan` renderers. Opaque verbatim copy (no blending); `opacity` is honored
/// only by the future on-GPU per-quad path.
pub fn composite_scene(out_w: u32, out_h: u32, pool: &[u8], quads: &[SurfaceQuad]) -> Vec<u8> {
    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    for q in quads {
        blit_bgra(
            &mut out,
            out_w,
            out_h,
            pool,
            q.pool_offset as usize,
            q.src_w,
            q.src_h,
            q.src_stride,
            q.dst_x,
            q.dst_y,
        );
    }
    out
}

// =====================================================================================
// Control channel messages
// =====================================================================================
/// Sent by the Windows host to the compositor.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    /// The host window was resized; new client-area size in physical pixels.
    Resize { width: u32, height: u32 },
    /// Absolute pointer position in framebuffer pixel coordinates.
    PointerMotion { x: f32, y: f32 },
    /// Pointer button change. `button` is a Linux input event code (e.g. `BTN_LEFT`=0x110).
    PointerButton { button: u32, pressed: bool },
    /// Scroll amounts (logical units) on each axis.
    PointerAxis { horizontal: f32, vertical: f32 },
    /// Keyboard key change. `keycode` is a raw Linux evdev keycode (XKB code = evdev + 8).
    Key { keycode: u32, pressed: bool },
    /// The host window is closing.
    Close,
}

/// One surface to be composited on the host GPU (WebGPU path). Its pixel data lives in the
/// shared-memory surface pool at `pool_offset` (BGRA, `src_stride` bytes per row, `src_h`
/// rows). Surfaces in a [`ServerMessage::GpuScene`] are listed back-to-front; `opacity` in
/// `[0, 1]` scales the surface alpha during blending.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceQuad {
    /// Byte offset of this surface's pixels within the shared-memory surface pool.
    pub pool_offset: u64,
    pub src_w: u32,
    pub src_h: u32,
    pub src_stride: u32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub dst_w: u32,
    pub dst_h: u32,
    pub opacity: f32,
}

impl SurfaceQuad {
    /// Destination rectangle in clip space (NDC) for an `out_w` x `out_h` output, returned as
    /// `[x0, y0, x1, y1]`. Framebuffer +Y (down) is flipped to clip-space +Y (up), so a quad
    /// covering the whole output maps to `[-1, 1, 1, -1]`.
    pub fn clip_rect(&self, out_w: u32, out_h: u32) -> [f32; 4] {
        let to_x = |px: f32| (px / out_w as f32) * 2.0 - 1.0;
        let to_y = |px: f32| 1.0 - (px / out_h as f32) * 2.0;
        [
            to_x(self.dst_x as f32),
            to_y(self.dst_y as f32),
            to_x((self.dst_x + self.dst_w as i32) as f32),
            to_y((self.dst_y + self.dst_h as i32) as f32),
        ]
    }
}

/// Sent by the compositor to the Windows host.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    /// Handshake: describes the shared framebuffer the host should map.
    FrameConfig {
        width: u32,
        height: u32,
        /// Total size of the shared file in bytes.
        shm_size: u64,
        /// Render backend the compositor was launched with. The host picks the matching
        /// renderer so a single `--backend` parameter drives both sides.
        backend: Backend,
        /// Path to the shared file **as the host (Windows) should open it**.
        host_path: String,
    },
    /// A new frame has been published to shared memory.
    FrameReady { seq: u64, width: u32, height: u32 },
    /// GPU-composite scene (WebGPU path): an ordered back-to-front list of surfaces for the
    /// host to composite itself. Pixels are referenced in the shared-memory surface pool.
    /// This is an additive alternative to the pre-composited [`FrameReady`](Self::FrameReady).
    GpuScene { seq: u64, surfaces: Vec<SurfaceQuad> },
}

// Message tags.
const TAG_RESIZE: u8 = 1;
const TAG_PTR_MOTION: u8 = 2;
const TAG_PTR_BUTTON: u8 = 3;
const TAG_PTR_AXIS: u8 = 4;
const TAG_KEY: u8 = 5;
const TAG_CLOSE: u8 = 6;
const TAG_FRAME_CONFIG: u8 = 7;
const TAG_FRAME_READY: u8 = 8;
const TAG_GPU_SCENE: u8 = 9;

/// Write a length-prefixed frame: `u32 payload_len` followed by `payload`.
fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// Read a length-prefixed frame payload.
fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_f32(buf: &mut Vec<u8>, v: f32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short message"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> io::Result<u64> {
        let s = self.take(8)?;
        Ok(u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
    }
    fn f32(&mut self) -> io::Result<f32> {
        let s = self.take(4)?;
        Ok(f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

impl ClientMessage {
    pub fn write<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let mut p = Vec::with_capacity(16);
        match self {
            ClientMessage::Resize { width, height } => {
                p.push(TAG_RESIZE);
                put_u32(&mut p, *width);
                put_u32(&mut p, *height);
            }
            ClientMessage::PointerMotion { x, y } => {
                p.push(TAG_PTR_MOTION);
                put_f32(&mut p, *x);
                put_f32(&mut p, *y);
            }
            ClientMessage::PointerButton { button, pressed } => {
                p.push(TAG_PTR_BUTTON);
                put_u32(&mut p, *button);
                p.push(*pressed as u8);
            }
            ClientMessage::PointerAxis { horizontal, vertical } => {
                p.push(TAG_PTR_AXIS);
                put_f32(&mut p, *horizontal);
                put_f32(&mut p, *vertical);
            }
            ClientMessage::Key { keycode, pressed } => {
                p.push(TAG_KEY);
                put_u32(&mut p, *keycode);
                p.push(*pressed as u8);
            }
            ClientMessage::Close => p.push(TAG_CLOSE),
        }
        write_frame(w, &p)
    }

    pub fn read<R: Read>(r: &mut R) -> io::Result<Self> {
        let buf = read_frame(r)?;
        let mut c = Cursor::new(&buf);
        let tag = c.u8()?;
        Ok(match tag {
            TAG_RESIZE => ClientMessage::Resize { width: c.u32()?, height: c.u32()? },
            TAG_PTR_MOTION => ClientMessage::PointerMotion { x: c.f32()?, y: c.f32()? },
            TAG_PTR_BUTTON => ClientMessage::PointerButton {
                button: c.u32()?,
                pressed: c.u8()? != 0,
            },
            TAG_PTR_AXIS => ClientMessage::PointerAxis {
                horizontal: c.f32()?,
                vertical: c.f32()?,
            },
            TAG_KEY => ClientMessage::Key { keycode: c.u32()?, pressed: c.u8()? != 0 },
            TAG_CLOSE => ClientMessage::Close,
            _ => return Err(invalid("unknown ClientMessage tag")),
        })
    }
}

impl ServerMessage {
    pub fn write<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let mut p = Vec::with_capacity(32);
        match self {
            ServerMessage::FrameConfig { width, height, shm_size, backend, host_path } => {
                p.push(TAG_FRAME_CONFIG);
                put_u32(&mut p, *width);
                put_u32(&mut p, *height);
                put_u64(&mut p, *shm_size);
                p.push(backend.as_u8());
                let bytes = host_path.as_bytes();
                put_u32(&mut p, bytes.len() as u32);
                p.extend_from_slice(bytes);
            }
            ServerMessage::FrameReady { seq, width, height } => {
                p.push(TAG_FRAME_READY);
                put_u64(&mut p, *seq);
                put_u32(&mut p, *width);
                put_u32(&mut p, *height);
            }
            ServerMessage::GpuScene { seq, surfaces } => {
                p.push(TAG_GPU_SCENE);
                put_u64(&mut p, *seq);
                put_u32(&mut p, surfaces.len() as u32);
                for s in surfaces {
                    put_u64(&mut p, s.pool_offset);
                    put_u32(&mut p, s.src_w);
                    put_u32(&mut p, s.src_h);
                    put_u32(&mut p, s.src_stride);
                    put_u32(&mut p, s.dst_x as u32);
                    put_u32(&mut p, s.dst_y as u32);
                    put_u32(&mut p, s.dst_w);
                    put_u32(&mut p, s.dst_h);
                    put_f32(&mut p, s.opacity);
                }
            }
        }
        write_frame(w, &p)
    }

    pub fn read<R: Read>(r: &mut R) -> io::Result<Self> {
        let buf = read_frame(r)?;
        let mut c = Cursor::new(&buf);
        let tag = c.u8()?;
        Ok(match tag {
            TAG_FRAME_CONFIG => {
                let width = c.u32()?;
                let height = c.u32()?;
                let shm_size = c.u64()?;
                let backend = Backend::from_u8(c.u8()?);
                let len = c.u32()? as usize;
                let path = c.take(len)?;
                let host_path = String::from_utf8(path.to_vec())
                    .map_err(|_| invalid("host_path not UTF-8"))?;
                ServerMessage::FrameConfig { width, height, shm_size, backend, host_path }
            }
            TAG_FRAME_READY => ServerMessage::FrameReady {
                seq: c.u64()?,
                width: c.u32()?,
                height: c.u32()?,
            },
            TAG_GPU_SCENE => {
                let seq = c.u64()?;
                let n = c.u32()? as usize;
                let mut surfaces = Vec::with_capacity(n);
                for _ in 0..n {
                    surfaces.push(SurfaceQuad {
                        pool_offset: c.u64()?,
                        src_w: c.u32()?,
                        src_h: c.u32()?,
                        src_stride: c.u32()?,
                        dst_x: c.u32()? as i32,
                        dst_y: c.u32()? as i32,
                        dst_w: c.u32()?,
                        dst_h: c.u32()?,
                        opacity: c.f32()?,
                    });
                }
                ServerMessage::GpuScene { seq, surfaces }
            }
            _ => return Err(invalid("unknown ServerMessage tag")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_sizes() {
        let l = FrameLayout::new(1280, 720);
        assert_eq!(l.stride(), 1280 * 4);
        assert_eq!(l.slot_bytes(), 1280 * 720 * 4);
        assert_eq!(l.total_size(), HEADER_SIZE as u64 + l.slot_bytes() * SLOT_COUNT as u64);
    }

    #[test]
    fn slot_offsets_are_distinct() {
        let l = FrameLayout::new(8, 8);
        assert_eq!(l.slot_offset(0), HEADER_SIZE);
        assert_eq!(l.slot_offset(1), HEADER_SIZE + l.slot_bytes() as usize);
    }

    #[test]
    fn surface_pool_regions_are_aligned_and_distinct() {
        // 10 regions big enough for a 300-byte surface -> each rounded up to POOL_ALIGN.
        let pool = SurfacePool::new(10, 300);
        assert_eq!(pool.region_count(), 10);
        assert_eq!(pool.region_stride(), POOL_ALIGN);
        assert_eq!(pool.pool_size(), POOL_ALIGN * 10);
        for i in 0..10 {
            let (off, len) = pool.pool_region(i).unwrap();
            assert_eq!(off, i as u64 * POOL_ALIGN, "region {i} offset");
            assert_eq!(len, POOL_ALIGN);
            assert_eq!(off % POOL_ALIGN, 0, "region {i} alignment");
        }
    }

    #[test]
    fn surface_pool_rounds_region_stride_up() {
        // A region needing 5000 bytes rounds up to 2 * POOL_ALIGN (8192).
        let pool = SurfacePool::new(3, 5000);
        assert_eq!(pool.region_stride(), 2 * POOL_ALIGN);
        assert_eq!(pool.pool_region(2), Some((2 * 2 * POOL_ALIGN, 2 * POOL_ALIGN)));
    }

    #[test]
    fn surface_pool_out_of_range_is_none() {
        let pool = SurfacePool::new(4, 100);
        assert!(pool.pool_region(3).is_some());
        assert_eq!(pool.pool_region(4), None);
        assert_eq!(pool.pool_region(99), None);
    }

    #[test]
    fn surface_pool_empty_has_zero_size() {
        let pool = SurfacePool::new(0, 4096);
        assert_eq!(pool.pool_size(), 0);
        assert_eq!(pool.pool_region(0), None);
    }

    #[test]
    fn surface_pool_exact_multiple_is_not_overpadded() {
        // Exactly POOL_ALIGN bytes stays one region wide (no extra page).
        let pool = SurfacePool::new(1, POOL_ALIGN);
        assert_eq!(pool.region_stride(), POOL_ALIGN);
    }

    #[test]
    fn region_header_roundtrips() {
        let mut region = vec![0u8; POOL_REGION_HEADER + 64];
        let h = RegionHeader { used: true, seq: 7, width: 4, height: 4, stride: 16 };
        h.write(&mut region);
        assert_eq!(RegionHeader::read(&region), Some(h));
        assert_eq!(h.payload_len(), 16 * 4);
    }

    #[test]
    fn region_header_too_short_is_none() {
        let short = vec![0u8; POOL_REGION_HEADER - 1];
        assert_eq!(RegionHeader::read(&short), None);
        // write is a no-op on a short slice (must not panic).
        let mut short = short;
        RegionHeader::default().write(&mut short);
    }

    #[test]
    fn region_pixels_follow_header() {
        let mut region = vec![0u8; POOL_REGION_HEADER + 8];
        region_pixels_mut(&mut region).unwrap()[0] = 0xAB;
        assert_eq!(region[POOL_REGION_HEADER], 0xAB);
        assert_eq!(region_pixels(&region).unwrap().len(), 8);
    }

    #[test]
    fn composite_scene_layers_back_to_front() {
        // 2x1 output. Two 1x1 surfaces at x=0 and x=1, pixels packed in a fake pool.
        let mut pool = vec![0u8; 8];
        pool[0..4].copy_from_slice(&[1, 2, 3, 4]);
        pool[4..8].copy_from_slice(&[9, 8, 7, 6]);
        let quads = vec![
            SurfaceQuad { pool_offset: 0, src_w: 1, src_h: 1, src_stride: 4, dst_x: 0, dst_y: 0, dst_w: 1, dst_h: 1, opacity: 1.0 },
            SurfaceQuad { pool_offset: 4, src_w: 1, src_h: 1, src_stride: 4, dst_x: 1, dst_y: 0, dst_w: 1, dst_h: 1, opacity: 1.0 },
        ];
        let out = composite_scene(2, 1, &pool, &quads);
        assert_eq!(&out[0..4], &[1, 2, 3, 4]);
        assert_eq!(&out[4..8], &[9, 8, 7, 6]);
    }

    #[test]
    fn composite_scene_front_quad_overwrites() {
        // Both target the same pixel; the later (front) quad wins.
        let mut pool = vec![0u8; 8];
        pool[0..4].copy_from_slice(&[1, 1, 1, 1]);
        pool[4..8].copy_from_slice(&[2, 2, 2, 2]);
        let quads = vec![
            SurfaceQuad { pool_offset: 0, src_w: 1, src_h: 1, src_stride: 4, dst_x: 0, dst_y: 0, dst_w: 1, dst_h: 1, opacity: 1.0 },
            SurfaceQuad { pool_offset: 4, src_w: 1, src_h: 1, src_stride: 4, dst_x: 0, dst_y: 0, dst_w: 1, dst_h: 1, opacity: 1.0 },
        ];
        let out = composite_scene(1, 1, &pool, &quads);
        assert_eq!(&out[0..4], &[2, 2, 2, 2]);
    }

    #[test]
    fn composite_scene_empty_is_cleared() {
        assert_eq!(composite_scene(2, 2, &[], &[]), vec![0u8; 2 * 2 * 4]);
    }

    #[test]
    fn shared_layout_places_pool_after_framebuffer() {
        let frame = FrameLayout::new(16, 16);
        let pool = SurfacePool::new(4, 1000);
        let layout = SharedLayout::new(frame, pool);
        assert_eq!(layout.pool_base(), frame.total_size());
        assert_eq!(layout.total_size(), frame.total_size() + pool.pool_size());
        // Region 0 starts exactly at the pool base; region 1 one stride later.
        assert_eq!(layout.region_file_range(0), Some((frame.total_size(), POOL_ALIGN)));
        assert_eq!(
            layout.region_file_range(1),
            Some((frame.total_size() + POOL_ALIGN, POOL_ALIGN))
        );
        assert_eq!(layout.region_file_range(4), None);
    }

    #[test]
    fn pixel_format_from_u32() {
        assert_eq!(PixelFormat::from_u32(0), Some(PixelFormat::Bgra8Unorm));
        assert_eq!(PixelFormat::from_u32(1), None);
        assert_eq!(PixelFormat::from_u32(99), None);
    }

    #[test]
    fn magic_spells_wwcf() {
        assert_eq!(&MAGIC.to_le_bytes(), b"WWCF");
    }

    #[test]
    fn gpu_scene_roundtrip() {
        let surfaces = vec![
            SurfaceQuad { pool_offset: 0, src_w: 10, src_h: 20, src_stride: 40, dst_x: -5, dst_y: 3, dst_w: 10, dst_h: 20, opacity: 1.0 },
            SurfaceQuad { pool_offset: 800, src_w: 4, src_h: 4, src_stride: 16, dst_x: 100, dst_y: 200, dst_w: 8, dst_h: 8, opacity: 0.5 },
        ];
        let msg = ServerMessage::GpuScene { seq: 42, surfaces: surfaces.clone() };
        let mut buf = Vec::new();
        msg.write(&mut buf).unwrap();
        let got = ServerMessage::read(&mut &buf[..]).unwrap();
        assert_eq!(got, ServerMessage::GpuScene { seq: 42, surfaces });
    }

    #[test]
    fn surface_quad_clip_rect_maps_full_output() {
        let q = SurfaceQuad { pool_offset: 0, src_w: 100, src_h: 100, src_stride: 400, dst_x: 0, dst_y: 0, dst_w: 100, dst_h: 100, opacity: 1.0 };
        let [x0, y0, x1, y1] = q.clip_rect(100, 100);
        assert!((x0 - -1.0).abs() < 1e-6);
        assert!((y0 - 1.0).abs() < 1e-6);
        assert!((x1 - 1.0).abs() < 1e-6);
        assert!((y1 - -1.0).abs() < 1e-6);
    }

    #[test]
    fn default_scan_addrs_starts_at_default_port() {
        let addrs = default_scan_addrs();
        assert_eq!(addrs.len(), PORT_SCAN_COUNT as usize);
        assert_eq!(addrs[0], format!("127.0.0.1:{DEFAULT_PORT}"));
        assert_eq!(addrs[1], format!("127.0.0.1:{}", DEFAULT_PORT + 1));
        assert_eq!(
            addrs[addrs.len() - 1],
            format!("127.0.0.1:{}", DEFAULT_PORT + PORT_SCAN_COUNT - 1)
        );
    }

    #[test]
    fn framebuffer_roundtrip() {
        let layout = FrameLayout::new(4, 2);

        let mut backing = vec![0u8; layout.total_size() as usize];
        let seq;
        {
            let mut fb = SharedFramebuffer::initialize(&mut backing, layout);
            assert_eq!(fb.layout(), layout);
            let (slot, pixels) = fb.back_slot_mut();
            assert_eq!(slot, 1); // active starts at 0, back is 1
            pixels.iter_mut().enumerate().for_each(|(i, b)| *b = i as u8);
            seq = fb.publish(slot);
        }
        assert_eq!(seq, 1);
        let reader = SharedFramebufferReader::new(&backing).expect("valid header");
        assert_eq!(reader.layout(), layout);
        assert_eq!(reader.format(), Some(PixelFormat::Bgra8Unorm));
        assert_eq!(reader.frame_seq(), seq);
        assert_eq!(reader.active_pixels()[1], 1);
    }

    #[test]
    fn publish_alternates_slots_and_increments_seq() {
        let layout = FrameLayout::new(2, 2);
        let mut backing = vec![0u8; layout.total_size() as usize];
        let mut fb = SharedFramebuffer::initialize(&mut backing, layout);

        let (slot_a, pix) = fb.back_slot_mut();
        pix[0] = 0xAA;
        let seq_a = fb.publish(slot_a);

        let (slot_b, pix) = fb.back_slot_mut();
        pix[0] = 0xBB;
        let seq_b = fb.publish(slot_b);

        assert_ne!(slot_a, slot_b);
        assert_eq!(seq_a, 1);
        assert_eq!(seq_b, 2);

        let reader = SharedFramebufferReader::new(&backing).unwrap();
        assert_eq!(reader.frame_seq(), 2);
        assert_eq!(reader.active_pixels()[0], 0xBB);
    }

    #[test]
    fn attach_preserves_sequence() {
        let layout = FrameLayout::new(2, 2);
        let mut backing = vec![0u8; layout.total_size() as usize];
        {
            let mut fb = SharedFramebuffer::initialize(&mut backing, layout);
            let (slot, _) = fb.back_slot_mut();
            fb.publish(slot);
        }
        {
            let mut fb = SharedFramebuffer::attach(&mut backing, layout);
            let (slot, _) = fb.back_slot_mut();
            let seq = fb.publish(slot);
            assert_eq!(seq, 2, "attach must not reset the sequence");
        }
    }

    #[test]
    fn reader_rejects_bad_header() {
        // Too short.
        assert!(SharedFramebufferReader::new(&[0u8; 4]).is_none());
        // Right size, wrong magic.
        let layout = FrameLayout::new(2, 2);
        let backing = vec![0u8; layout.total_size() as usize];
        assert!(SharedFramebufferReader::new(&backing).is_none());
    }

    #[test]
    fn client_message_roundtrip() {
        let msgs = [
            ClientMessage::Resize { width: 800, height: 600 },
            ClientMessage::PointerMotion { x: 1.5, y: 2.5 },
            ClientMessage::PointerButton { button: 0x110, pressed: true },
            ClientMessage::PointerButton { button: 0x111, pressed: false },
            ClientMessage::PointerAxis { horizontal: -1.0, vertical: 3.0 },
            ClientMessage::Key { keycode: 30, pressed: false },
            ClientMessage::Close,
        ];
        for m in msgs {
            let mut buf = Vec::new();
            m.write(&mut buf).unwrap();
            let got = ClientMessage::read(&mut &buf[..]).unwrap();
            assert_eq!(m, got);
        }
    }

    #[test]
    fn server_message_roundtrip() {
        let msgs = [
            ServerMessage::FrameConfig {
                width: 640,
                height: 480,
                shm_size: 12345,
                backend: Backend::Webgpu,
                host_path: r"C:\Temp\wwc.fb".to_string(),
            },
            ServerMessage::FrameConfig {
                width: 800,
                height: 600,
                shm_size: 999,
                backend: Backend::Vulkan,
                host_path: r"C:\Temp\v.fb".to_string(),
            },
            ServerMessage::FrameReady { seq: 42, width: 640, height: 480 },
        ];
        for m in msgs {
            let mut buf = Vec::new();
            m.write(&mut buf).unwrap();
            let got = ServerMessage::read(&mut &buf[..]).unwrap();
            assert_eq!(m, got);
        }
    }

    #[test]
    fn backend_parse_and_wire_roundtrip() {
        assert_eq!(Backend::default(), Backend::Webgpu);
        assert_eq!(Backend::parse("webgpu"), Some(Backend::Webgpu));
        assert_eq!(Backend::parse("WGPU"), Some(Backend::Webgpu));
        assert_eq!(Backend::parse("vulkan"), Some(Backend::Vulkan));
        assert_eq!(Backend::parse("ash"), Some(Backend::Vulkan));
        assert_eq!(Backend::parse("nope"), None);
        assert_eq!(Backend::from_u8(Backend::Webgpu.as_u8()), Backend::Webgpu);
        assert_eq!(Backend::from_u8(Backend::Vulkan.as_u8()), Backend::Vulkan);
        // Unknown wire bytes fall back to the default.
        assert_eq!(Backend::from_u8(200), Backend::Webgpu);
        assert_eq!(Backend::Webgpu.name(), "webgpu");
        assert_eq!(Backend::Vulkan.name(), "vulkan");
    }

    #[test]
    fn client_message_rejects_unknown_tag() {
        // payload length 1, tag 200 (unknown)
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(200);
        let err = ClientMessage::read(&mut &buf[..]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn server_message_rejects_unknown_tag() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(201);
        let err = ServerMessage::read(&mut &buf[..]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn message_read_handles_truncated_stream() {
        // Claims a 10-byte payload but provides none.
        let mut buf = Vec::new();
        buf.extend_from_slice(&10u32.to_le_bytes());
        let err = ClientMessage::read(&mut &buf[..]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn message_read_handles_short_payload() {
        // Payload length 1 but tag claims Resize which needs 8 more bytes.
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(TAG_RESIZE);
        let err = ClientMessage::read(&mut &buf[..]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn frame_config_rejects_non_utf8_path() {
        // Build a FrameConfig payload with an invalid UTF-8 path.
        let mut p = Vec::new();
        p.push(TAG_FRAME_CONFIG);
        p.extend_from_slice(&1u32.to_le_bytes()); // width
        p.extend_from_slice(&1u32.to_le_bytes()); // height
        p.extend_from_slice(&1u64.to_le_bytes()); // shm_size
        p.push(0u8); // backend = webgpu
        p.extend_from_slice(&2u32.to_le_bytes()); // path len
        p.extend_from_slice(&[0xff, 0xfe]); // invalid utf-8
        let mut buf = Vec::new();
        buf.extend_from_slice(&(p.len() as u32).to_le_bytes());
        buf.extend_from_slice(&p);
        let err = ServerMessage::read(&mut &buf[..]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn wsl_path_conversion() {
        assert_eq!(wsl_path_to_windows("/mnt/c/Temp/wwc.fb"), r"C:\Temp\wwc.fb");
        assert_eq!(wsl_path_to_windows("/mnt/d/a/b/c"), r"D:\a\b\c");
        assert_eq!(wsl_path_to_windows("/mnt/c"), r"C:\");
        assert_eq!(wsl_path_to_windows("/mnt/c/"), r"C:\");
        // Not a /mnt path: unchanged.
        assert_eq!(wsl_path_to_windows("/home/user/x"), "/home/user/x");
        assert_eq!(wsl_path_to_windows("relative/path"), "relative/path");
        // /mnt/ but not a drive letter: unchanged.
        assert_eq!(wsl_path_to_windows("/mnt/"), "/mnt/");
        assert_eq!(wsl_path_to_windows("/mnt/wsl/foo"), "/mnt/wsl/foo");
    }

    /// A BGRA image of `w*h` where each pixel's R channel encodes a value for easy assertion.
    fn solid(w: u32, h: u32, b: u8, g: u8, r: u8, a: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            v.extend_from_slice(&[b, g, r, a]);
        }
        v
    }

    #[test]
    fn blit_full_copy_at_origin() {
        let mut dst = vec![0u8; 4 * 2 * 4];
        let src = solid(4, 2, 1, 2, 3, 4);
        blit_bgra(&mut dst, 4, 2, &src, 0, 4, 2, 4 * 4, 0, 0);
        assert_eq!(dst, src);
    }

    #[test]
    fn blit_places_at_offset() {
        // 4x2 destination, 2x1 source placed at (1,1).
        let mut dst = vec![0u8; 4 * 2 * 4];
        let src = solid(2, 1, 9, 9, 9, 9);
        blit_bgra(&mut dst, 4, 2, &src, 0, 2, 1, 2 * 4, 1, 1);
        // Row 0 untouched.
        assert!(dst[0..16].iter().all(|&b| b == 0));
        // Row 1, columns 1 and 2 written.
        let row1 = &dst[16..32];
        assert!(row1[0..4].iter().all(|&b| b == 0)); // col 0
        assert_eq!(&row1[4..8], &[9, 9, 9, 9]); // col 1
        assert_eq!(&row1[8..12], &[9, 9, 9, 9]); // col 2
        assert!(row1[12..16].iter().all(|&b| b == 0)); // col 3
    }

    #[test]
    fn blit_clips_negative_x() {
        // 2-wide dst, 4-wide src at x=-2 => source cols 2,3 land in dst cols 0,1.
        let mut dst = vec![0u8; 2 * 1 * 4];
        let mut src = Vec::new();
        for col in 0..4u8 {
            src.extend_from_slice(&[col, col, col, 0xff]);
        }
        blit_bgra(&mut dst, 2, 1, &src, 0, 4, 1, 4 * 4, -2, 0);
        assert_eq!(&dst[0..4], &[2, 2, 2, 0xff]);
        assert_eq!(&dst[4..8], &[3, 3, 3, 0xff]);
    }

    #[test]
    fn blit_skips_rows_outside_vertical_bounds() {
        let mut dst = vec![0u8; 2 * 1 * 4];
        let src = solid(2, 3, 5, 5, 5, 5);
        // Place so that only the middle source row (row 1) lands on dst row 0.
        blit_bgra(&mut dst, 2, 1, &src, 0, 2, 3, 2 * 4, 0, -1);
        assert!(dst.iter().all(|&b| b == 5));
    }

    #[test]
    fn blit_fully_offscreen_is_noop() {
        let mut dst = vec![7u8; 2 * 2 * 4];
        let src = solid(2, 2, 1, 1, 1, 1);
        blit_bgra(&mut dst, 2, 2, &src, 0, 2, 2, 2 * 4, 5, 0); // x past right edge
        assert!(dst.iter().all(|&b| b == 7));
        blit_bgra(&mut dst, 2, 2, &src, 0, 2, 2, 2 * 4, 0, 5); // y past bottom edge
        assert!(dst.iter().all(|&b| b == 7));
    }

    #[test]
    fn blit_respects_src_offset() {
        // Source buffer has a 16-byte header before pixel data.
        let mut dst = vec![0u8; 1 * 1 * 4];
        let mut src = vec![0u8; 16];
        src.extend_from_slice(&[8, 8, 8, 8]);
        blit_bgra(&mut dst, 1, 1, &src, 16, 1, 1, 4, 0, 0);
        assert_eq!(dst, vec![8, 8, 8, 8]);
    }

    #[test]
    fn blit_skips_truncated_source() {
        // Claim a 4x1 source but only provide 2 pixels of data: no out-of-bounds read.
        let mut dst = vec![0u8; 4 * 1 * 4];
        let src = solid(2, 1, 1, 1, 1, 1); // only 8 bytes
        blit_bgra(&mut dst, 4, 1, &src, 0, 4, 1, 4 * 4, 0, 0);
        // Nothing copied because the single row would read past src.len().
        assert!(dst.iter().all(|&b| b == 0));
    }
}
