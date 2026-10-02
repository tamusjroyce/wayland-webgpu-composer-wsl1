//! Memory-mapping of the shared framebuffer file produced by the compositor.

use std::fs::File;

use memmap2::Mmap;

/// Open and memory-map the shared framebuffer file. The file lives on a Windows path that is
/// also visible from WSL1, so both processes map the same physical pages. Returns `None` if
/// the file cannot be opened or is not yet at least `size` bytes.
pub fn open_shared(path: &str, size: u64) -> Option<Mmap> {
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() < size {
        log::warn!("shared file smaller than expected ({size} bytes); not ready yet");
        return None;
    }
    // SAFETY: the file is backed by stable storage; concurrent writes from the compositor are
    // expected and tolerated (the published slot is re-read each frame).
    unsafe { memmap2::MmapOptions::new().len(size as usize).map(&file).ok() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::{FrameLayout, SharedFramebuffer, SharedFramebufferReader};
    use std::io::Write;

    #[test]
    fn open_and_read_published_frame() {
        let layout = FrameLayout::new(4, 2);
        let size = layout.total_size();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wwc.fb");

        // Write a full, initialized, published frame to the file.
        {
            let mut backing = vec![0u8; size as usize];
            {
                let mut fb = SharedFramebuffer::initialize(&mut backing, layout);
                let (slot, pixels) = fb.back_slot_mut();
                pixels.iter_mut().for_each(|b| *b = 0x7f);
                fb.publish(slot);
            }
            let mut f = File::create(&path).unwrap();
            f.write_all(&backing).unwrap();
            f.flush().unwrap();
        }

        let map = open_shared(path.to_str().unwrap(), size).expect("should map");
        let reader = SharedFramebufferReader::new(&map[..]).expect("valid header");
        assert_eq!(reader.layout(), layout);
        assert_eq!(reader.active_pixels()[0], 0x7f);
    }

    #[test]
    fn missing_file_returns_none() {
        assert!(open_shared("this/does/not/exist.fb", 64).is_none());
    }

    #[test]
    fn too_small_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.fb");
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&[0u8; 8]).unwrap();
        }
        assert!(open_shared(path.to_str().unwrap(), 1024).is_none());
    }
}
