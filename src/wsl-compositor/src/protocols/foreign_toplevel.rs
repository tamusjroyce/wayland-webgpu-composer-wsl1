//! `zwlr_foreign_toplevel_management_unstable_v1`. Reference: labwc `src/foreign-toplevel/`.
//!
//! Advertises every mapped xdg-toplevel to taskbars/docks and lets them act on windows
//! (activate, close, ...). One `zwlr_foreign_toplevel_handle_v1` is created per (manager, window)
//! pair; handles are kept in sync with title/app-id changes and emit `closed` on unmap.

use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::state::State;

const MANAGER_VERSION: u32 = 3;

/// Per-window tracking entry.
struct Toplevel {
    surface: WlSurface,
    title: String,
    app_id: String,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
}

/// Compositor-side state for the foreign-toplevel manager.
#[derive(Default)]
pub struct ForeignToplevelManagerState {
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    toplevels: Vec<Toplevel>,
}

/// Global data for the `zwlr_foreign_toplevel_manager_v1` global.
pub struct ForeignToplevelGlobal;
/// User data for a bound manager object.
pub struct ManagerData;
/// User data for a toplevel handle: identifies the window by its root surface.
pub struct HandleData {
    pub surface: WlSurface,
}

/// Create the `zwlr_foreign_toplevel_manager_v1` global.
pub fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, ZwlrForeignToplevelManagerV1, ForeignToplevelGlobal>(
        MANAGER_VERSION,
        ForeignToplevelGlobal,
    );
}

fn send_details(handle: &ZwlrForeignToplevelHandleV1, title: &str, app_id: &str) {
    handle.title(title.to_string());
    handle.app_id(app_id.to_string());
    // No per-window state is tracked yet; send an empty (valid) state array.
    handle.state(Vec::new());
    handle.done();
}

impl ForeignToplevelManagerState {
    /// A new window was mapped: tell every manager about it.
    pub fn window_created(&mut self, dh: &DisplayHandle, surface: WlSurface, title: String, app_id: String) {
        let mut handles = Vec::new();
        for manager in &self.managers {
            if let Ok(client) = dh.get_client(manager.id()) {
                if let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(
                    dh,
                    manager.version(),
                    HandleData { surface: surface.clone() },
                ) {
                    manager.toplevel(&handle);
                    send_details(&handle, &title, &app_id);
                    handles.push(handle);
                }
            }
        }
        self.toplevels.push(Toplevel { surface, title, app_id, handles });
    }

    /// A commit may have changed the window's title/app-id: resend if so.
    pub fn window_updated(&mut self, surface: &WlSurface, title: String, app_id: String) {
        if let Some(entry) = self.toplevels.iter_mut().find(|t| &t.surface == surface) {
            if entry.title == title && entry.app_id == app_id {
                return;
            }
            entry.title = title;
            entry.app_id = app_id;
            for handle in &entry.handles {
                send_details(handle, &entry.title, &entry.app_id);
            }
        }
    }

    /// The window was unmapped/destroyed: emit `closed` and drop tracking.
    pub fn window_closed(&mut self, surface: &WlSurface) {
        if let Some(pos) = self.toplevels.iter().position(|t| &t.surface == surface) {
            let entry = self.toplevels.remove(pos);
            for handle in entry.handles {
                handle.closed();
            }
        }
    }

    /// Send all current toplevels to a newly bound manager.
    fn advertise_all(&mut self, dh: &DisplayHandle, client: &Client, manager: &ZwlrForeignToplevelManagerV1) {
        for entry in &mut self.toplevels {
            if let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(
                dh,
                manager.version(),
                HandleData { surface: entry.surface.clone() },
            ) {
                manager.toplevel(&handle);
                send_details(&handle, &entry.title, &entry.app_id);
                entry.handles.push(handle);
            }
        }
    }

    fn remove_manager(&mut self, manager: &ZwlrForeignToplevelManagerV1) {
        self.managers.retain(|m| m != manager);
    }

    fn remove_handle(&mut self, handle: &ZwlrForeignToplevelHandleV1) {
        for entry in &mut self.toplevels {
            entry.handles.retain(|h| h != handle);
        }
    }
}

impl GlobalDispatch2<ZwlrForeignToplevelManagerV1, State> for ForeignToplevelGlobal {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ManagerData);
        state.foreign_toplevel.advertise_all(dh, client, &manager);
        state.foreign_toplevel.managers.push(manager);
    }
}

impl Dispatch2<ZwlrForeignToplevelManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_foreign_toplevel_manager_v1::Request::Stop => {
                resource.finished();
                state.foreign_toplevel.remove_manager(resource);
            }
            _ => {}
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ZwlrForeignToplevelManagerV1,
    ) {
        state.foreign_toplevel.remove_manager(resource);
    }
}

impl Dispatch2<ZwlrForeignToplevelHandleV1, State> for HandleData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let window = state
            .space
            .elements()
            .find(|w| w.toplevel().map(|t| t.wl_surface() == &self.surface).unwrap_or(false))
            .cloned();
        match request {
            zwlr_foreign_toplevel_handle_v1::Request::Activate { seat: _ } => {
                if let Some(window) = window {
                    state.space.raise_element(&window, true);
                    window.set_activated(true);
                    if let (Some(keyboard), Some(toplevel)) =
                        (state.seat.get_keyboard(), window.toplevel())
                    {
                        let serial = SERIAL_COUNTER.next_serial();
                        keyboard.set_focus(state, Some(toplevel.wl_surface().clone()), serial);
                        toplevel.send_pending_configure();
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => {
                if let Some(window) = window {
                    if let Some(toplevel) = window.toplevel() {
                        toplevel.send_close();
                    }
                }
            }
            // Maximize/minimize/fullscreen/rectangle: the bridge compositor uses a single
            // full-surface layout, so these are accepted but not acted upon.
            _ => {}
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ZwlrForeignToplevelHandleV1,
    ) {
        state.foreign_toplevel.remove_handle(resource);
    }
}
