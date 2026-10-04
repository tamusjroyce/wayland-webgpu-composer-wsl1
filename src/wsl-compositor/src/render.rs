//! Software compositing of client `wl_shm` surfaces into the shared framebuffer.

use std::time::Duration;

use bridge_protocol::{FrameLayout, ServerMessage, SharedFramebuffer};
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::desktop::{layer_map_for_output, Space, Window};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{with_surface_tree_downward, TraversalAction};
use smithay::wayland::shell::wlr_layer::Layer;
use smithay::wayland::shm::with_buffer_contents;
use smithay::desktop::utils::send_frames_surface_tree;

use crate::state::State;

impl State {
    /// Composite the current scene, publish it to shared memory, and notify the host.
    pub fn render(&mut self) {
        let lock_surface = self.session_lock.lock_surface();
        let session_locked = self.session_lock.is_active();
        composite(
            &self.space,
            &self.output,
            session_locked,
            lock_surface.as_ref(),
            &mut self.scratch,
            self.layout,
        );

        let (seq, slot) = {
            let mut fb = SharedFramebuffer::attach(&mut self.map[..], self.layout);
            let (slot, dst) = fb.back_slot_mut();
            dst.copy_from_slice(&self.scratch);
            (fb.publish(slot), slot)
        };

        // On WSL1 a `/mnt/c` (DrvFs) MAP_SHARED mapping is only coherent with the Windows
        // file after an msync. Use the async variant so the event loop is never blocked.
        let slot_off =
            bridge_protocol::HEADER_SIZE + self.layout.slot_bytes() as usize * slot as usize;
        let _ = self.map.flush_async_range(slot_off, self.layout.slot_bytes() as usize);
        let _ = self.map.flush_async_range(0, bridge_protocol::HEADER_SIZE);

        self.bridge.send(&ServerMessage::FrameReady {
            seq,
            width: self.layout.width,
            height: self.layout.height,
        });

        // Send frame callbacks so clients produce their next frame.
        let elapsed = self.start_time.elapsed();
        let output = self.output.clone();
        for window in self.space.elements() {
            window.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
        // Frame callbacks for wlr_layer_shell surfaces (panels/bars) too.
        {
            let map = layer_map_for_output(&output);
            for layer in map.layers() {
                layer.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            }
        }
        // Keep the session-lock surface animating while the session is locked.
        if let Some(ls) = &lock_surface {
            send_frames_surface_tree(ls, &output, elapsed, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }

        self.space.refresh();
        self.popups.cleanup();
        let _ = self.display_handle.flush_clients();
    }
}

/// Composite every mapped window's surface tree into `out` (BGRA, `layout` sized).
fn composite(
    space: &Space<Window>,
    output: &Output,
    session_locked: bool,
    lock_surface: Option<&WlSurface>,
    out: &mut [u8],
    layout: FrameLayout,
) {
    // While the session is locked, blank the output and render only the lock surface.
    if session_locked {
        for px in out.chunks_exact_mut(4) {
            px[0] = 0x00;
            px[1] = 0x00;
            px[2] = 0x00;
            px[3] = 0xff;
        }
        if let Some(ls) = lock_surface {
            blit_surface_tree(ls, (0, 0).into(), out, layout);
        }
        return;
    }

    // Clear to a dark background.
    for px in out.chunks_exact_mut(4) {
        px[0] = 0x1a;
        px[1] = 0x1a;
        px[2] = 0x1a;
        px[3] = 0xff;
    }

    let map = layer_map_for_output(output);

    // Background and bottom layer-shell surfaces render *under* the windows.
    for layer in map
        .layers()
        .filter(|l| matches!(l.layer(), Layer::Background | Layer::Bottom))
    {
        if let Some(geo) = map.layer_geometry(layer) {
            blit_surface_tree(layer.wl_surface(), geo.loc, out, layout);
        }
    }

    for window in space.elements() {
        let Some(loc) = space.element_location(window) else {
            continue;
        };
        let Some(toplevel) = window.toplevel() else {
            continue;
        };
        let root = toplevel.wl_surface().clone();
        blit_surface_tree(&root, loc, out, layout);
    }

    // Top and overlay layer-shell surfaces render *over* the windows.
    for layer in map
        .layers()
        .filter(|l| matches!(l.layer(), Layer::Top | Layer::Overlay))
    {
        if let Some(geo) = map.layer_geometry(layer) {
            blit_surface_tree(layer.wl_surface(), geo.loc, out, layout);
        }
    }
}

/// Blit a surface tree rooted at `root`, positioned at `loc`, into `out`.
fn blit_surface_tree(root: &WlSurface, loc: Point<i32, Logical>, out: &mut [u8], layout: FrameLayout) {
    // Reborrow so the closure can take ownership of a fresh `&mut` each call.
    let out_ref: &mut [u8] = out;
    with_surface_tree_downward(
        root,
        loc,
        move |_surface, states, parent_loc| {
            let mut loc = *parent_loc;
            // Use the `states` provided by the traversal; calling `with_states` again
            // (e.g. via `with_renderer_surface_state`) here would re-enter the surface
            // lock and deadlock.
            if let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() {
                let rss = data.lock().unwrap();
                if let Some(view) = rss.view() {
                    loc += view.offset;
                }
                if let Some(buffer) = rss.buffer() {
                    blit_buffer(buffer, loc, out_ref, layout);
                }
            }
            TraversalAction::DoChildren(loc)
        },
        |_, _, _| {},
        |_, _, _| true,
    );
}

/// Copy a single `wl_shm` buffer into the output framebuffer at `loc`, clipping to bounds.
fn blit_buffer(buffer: &WlBuffer, loc: Point<i32, Logical>, out: &mut [u8], layout: FrameLayout) {
    let _ = with_buffer_contents(buffer, |ptr, len, data| {
        if !matches!(
            data.format,
            wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888
        ) {
            return;
        }
        // SAFETY: `ptr`/`len` describe the SHM pool and are valid for the duration of this
        // closure (guaranteed by `with_buffer_contents`). We only read from the slice.
        let src = unsafe { std::slice::from_raw_parts(ptr, len) };
        bridge_protocol::blit_bgra(
            out,
            layout.width,
            layout.height,
            src,
            data.offset as usize,
            data.width as u32,
            data.height as u32,
            data.stride as u32,
            loc.x,
            loc.y,
        );
    });
}
