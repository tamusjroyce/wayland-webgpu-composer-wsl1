//! `ext_session_lock_v1` (staging). Reference: labwc `src/session-lock.c`.
//!
//! When a client locks the session the compositor blanks the output and renders only the
//! client's lock surface (see `render.rs`), and keyboard focus is routed to the lock surface
//! (see `input.rs`). The `locked` event is sent once the lock surface presents its first buffer,
//! as required by the spec. A single virtual output means a single lock surface.

use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::{
    ext_session_lock_manager_v1::{self, ExtSessionLockManagerV1},
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::compositor::with_states;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::state::State;

/// Active lock bookkeeping.
struct ActiveLock {
    lock: ExtSessionLockV1,
    locked: bool,
    surface: Option<WlSurface>,
    lock_surface: Option<ExtSessionLockSurfaceV1>,
}

/// Compositor-side session-lock state.
#[derive(Default)]
pub struct SessionLockState {
    inner: Option<ActiveLock>,
}

impl SessionLockState {
    /// Whether a lock is in effect (requested or fully locked). While active, normal clients
    /// must not be shown.
    pub fn is_active(&self) -> bool {
        self.inner.is_some()
    }

    /// The current lock surface, if one has been created.
    pub fn lock_surface(&self) -> Option<WlSurface> {
        self.inner.as_ref().and_then(|l| l.surface.clone())
    }
}

/// Global data for the `ext_session_lock_manager_v1` global.
pub struct SessionLockGlobal;
/// User data for a bound manager object.
pub struct LockManagerData;
/// User data for an `ext_session_lock_v1` object.
pub struct LockObjData;
/// User data for an `ext_session_lock_surface_v1` object.
pub struct LockSurfaceData {
    surface: WlSurface,
}

/// Create the `ext_session_lock_manager_v1` global.
pub fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, ExtSessionLockManagerV1, SessionLockGlobal>(1, SessionLockGlobal);
}

fn buffer_attached(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .map(|d| d.lock().unwrap().buffer().is_some())
            .unwrap_or(false)
    })
}

impl State {
    /// Called from the commit handler: when the lock surface presents its first buffer, confirm
    /// the lock by sending `locked` and routing keyboard focus to it.
    pub fn session_lock_commit(&mut self, surface: &WlSurface) {
        let should_lock = {
            let Some(active) = self.session_lock.inner.as_ref() else {
                return;
            };
            !active.locked && active.surface.as_ref() == Some(surface) && buffer_attached(surface)
        };
        if !should_lock {
            return;
        }
        if let Some(active) = self.session_lock.inner.as_mut() {
            active.locked = true;
            active.lock.locked();
        }
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(surface.clone()), serial);
        }
    }
}

impl GlobalDispatch2<ExtSessionLockManagerV1, State> for SessionLockGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ExtSessionLockManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, LockManagerData);
    }
}

impl Dispatch2<ExtSessionLockManagerV1, State> for LockManagerData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ExtSessionLockManagerV1,
        request: ext_session_lock_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_session_lock_manager_v1::Request::Lock { id } => {
                let lock = data_init.init(id, LockObjData);
                if state.session_lock.inner.is_some() {
                    // Another lock already holds the session: deny this one.
                    lock.finished();
                } else {
                    state.session_lock.inner = Some(ActiveLock {
                        lock,
                        locked: false,
                        surface: None,
                        lock_surface: None,
                    });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch2<ExtSessionLockV1, State> for LockObjData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ExtSessionLockV1,
        request: ext_session_lock_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_session_lock_v1::Request::GetLockSurface { id, surface, output: _ } => {
                let lock_surface = data_init.init(id, LockSurfaceData { surface: surface.clone() });
                let serial = u32::from(SERIAL_COUNTER.next_serial());
                lock_surface.configure(serial, state.layout.width, state.layout.height);
                if let Some(active) = state.session_lock.inner.as_mut() {
                    active.surface = Some(surface);
                    active.lock_surface = Some(lock_surface);
                }
            }
            ext_session_lock_v1::Request::UnlockAndDestroy => {
                state.session_lock.inner = None;
            }
            ext_session_lock_v1::Request::Destroy => {
                state.session_lock.inner = None;
            }
            _ => {}
        }
    }
}

impl Dispatch2<ExtSessionLockSurfaceV1, State> for LockSurfaceData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ExtSessionLockSurfaceV1,
        request: ext_session_lock_surface_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_session_lock_surface_v1::Request::Destroy => {
                if let Some(active) = state.session_lock.inner.as_mut() {
                    if active.surface.as_ref() == Some(&self.surface) {
                        active.surface = None;
                        active.lock_surface = None;
                    }
                }
            }
            // ack_configure: no dimension validation is enforced by this compositor.
            _ => {}
        }
    }
}
