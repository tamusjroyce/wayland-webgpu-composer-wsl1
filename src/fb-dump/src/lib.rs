//! Snapshot the shared framebuffer for headless visual testing.

use bridge_protocol::SharedFramebufferReader;

/// Convert tightly-packed BGRA bytes to RGBA.
pub fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}

/// Read the active frame from a mapped shared framebuffer. Returns `(width, height, rgba)`,
/// or `None` if the mapping does not contain a valid framebuffer header.
pub fn snapshot(map: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let reader = SharedFramebufferReader::new(map)?;
    let layout = reader.layout();
    Some((layout.width, layout.height, bgra_to_rgba(reader.active_pixels())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::{FrameLayout, SharedFramebuffer};

    #[test]
    fn bgra_to_rgba_swaps_channels() {
        // B=10 G=20 R=30 A=40  ->  R=30 G=20 B=10 A=40
        assert_eq!(bgra_to_rgba(&[10, 20, 30, 40]), vec![30, 20, 10, 40]);
    }

    #[test]
    fn bgra_to_rgba_ignores_trailing_partial_pixel() {
        // chunks_exact drops the dangling 2 bytes.
        assert_eq!(bgra_to_rgba(&[1, 2, 3, 4, 9, 9]), vec![3, 2, 1, 4]);
    }

    #[test]
    fn snapshot_reads_active_frame() {
        let layout = FrameLayout::new(2, 1);
        let mut backing = vec![0u8; layout.total_size() as usize];
        {
            let mut fb = SharedFramebuffer::initialize(&mut backing, layout);
            let (slot, px) = fb.back_slot_mut();
            px.copy_from_slice(&[1, 2, 3, 255, 4, 5, 6, 255]);
            fb.publish(slot);
        }
        let (w, h, rgba) = snapshot(&backing).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn snapshot_none_on_bad_header() {
        assert!(snapshot(&[0u8; 64]).is_none());
    }
}
