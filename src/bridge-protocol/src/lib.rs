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

/// Sent by the compositor to the Windows host.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    /// Handshake: describes the shared framebuffer the host should map.
    FrameConfig {
        width: u32,
        height: u32,
        /// Total size of the shared file in bytes.
        shm_size: u64,
        /// Path to the shared file **as the host (Windows) should open it**.
        host_path: String,
    },
    /// A new frame has been published to shared memory.
    FrameReady { seq: u64, width: u32, height: u32 },
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
            ServerMessage::FrameConfig { width, height, shm_size, host_path } => {
                p.push(TAG_FRAME_CONFIG);
                put_u32(&mut p, *width);
                put_u32(&mut p, *height);
                put_u64(&mut p, *shm_size);
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
                let len = c.u32()? as usize;
                let path = c.take(len)?;
                let host_path = String::from_utf8(path.to_vec())
                    .map_err(|_| invalid("host_path not UTF-8"))?;
                ServerMessage::FrameConfig { width, height, shm_size, host_path }
            }
            TAG_FRAME_READY => ServerMessage::FrameReady {
                seq: c.u64()?,
                width: c.u32()?,
                height: c.u32()?,
            },
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
                host_path: r"C:\Temp\wwc.fb".to_string(),
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
