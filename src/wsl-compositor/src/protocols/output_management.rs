//! `zwlr_output_management_unstable_v1` (read-only). Reference: labwc `src/output-state.c`,
//! `src/output-virtual.c`.
//!
//! The compositor drives a single fixed virtual output sized to the shared framebuffer, so the
//! configuration side is read-only: heads/modes are advertised to clients such as `wlr-randr`
//! and `kanshi`, but any `apply`/`test` of a new configuration is rejected with `failed`
//! (there is no reconfigurable hardware behind the virtual output).

use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use smithay::reexports::wayland_server::protocol::wl_output::Transform;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::state::State;

const MANAGER_VERSION: u32 = 3;

/// Global data for the `zwlr_output_manager_v1` global.
pub struct OutputManagementGlobal;
/// User data for a bound manager object.
pub struct OutputManagerData;
/// User data for an advertised head.
pub struct OutputHeadData;
/// User data for an advertised mode.
pub struct OutputModeData;
/// User data for a client-built configuration object.
pub struct OutputConfigurationData;
/// User data for a per-head configuration object.
pub struct OutputConfigurationHeadData;

/// Create the `zwlr_output_manager_v1` global.
pub fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, ZwlrOutputManagerV1, OutputManagementGlobal>(
        MANAGER_VERSION,
        OutputManagementGlobal,
    );
}

/// Advertise the single virtual output to a freshly bound manager, followed by `done`.
fn advertise(state: &State, dh: &DisplayHandle, client: &Client, manager: &ZwlrOutputManagerV1) {
    let version = manager.version();
    let Ok(head) = client.create_resource::<ZwlrOutputHeadV1, _, State>(dh, version, OutputHeadData)
    else {
        return;
    };
    manager.head(&head);
    head.name(state.output.name());
    head.description(state.output.description());

    let Ok(mode) = client.create_resource::<ZwlrOutputModeV1, _, State>(dh, version, OutputModeData)
    else {
        return;
    };
    head.mode(&mode);
    if let Some(current) = state.output.current_mode() {
        mode.size(current.size.w, current.size.h);
        mode.refresh(current.refresh);
        mode.preferred();
    }

    head.enabled(1);
    head.current_mode(&mode);
    head.position(0, 0);
    head.transform(Transform::Normal);
    head.scale(state.output.current_scale().fractional_scale());

    if version >= 2 {
        let props = state.output.physical_properties();
        head.make(props.make);
        head.model(props.model);
    }

    manager.done(next_serial());
}

fn next_serial() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SERIAL: AtomicU32 = AtomicU32::new(1);
    SERIAL.fetch_add(1, Ordering::Relaxed)
}

impl GlobalDispatch2<ZwlrOutputManagerV1, State> for OutputManagementGlobal {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, OutputManagerData);
        advertise(state, dh, client, &manager);
    }
}

impl Dispatch2<ZwlrOutputManagerV1, State> for OutputManagerData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        resource: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial: _ } => {
                data_init.init(id, OutputConfigurationData);
            }
            zwlr_output_manager_v1::Request::Stop => {
                resource.finished();
            }
            _ => {}
        }
    }
}

impl Dispatch2<ZwlrOutputHeadV1, State> for OutputHeadData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZwlrOutputHeadV1,
        _request: zwlr_output_head_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // Only `release` (destructor); nothing to clean up.
    }
}

impl Dispatch2<ZwlrOutputModeV1, State> for OutputModeData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZwlrOutputModeV1,
        _request: zwlr_output_mode_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // Only `release` (destructor); nothing to clean up.
    }
}

impl Dispatch2<ZwlrOutputConfigurationV1, State> for OutputConfigurationData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head: _ } => {
                data_init.init(id, OutputConfigurationHeadData);
            }
            zwlr_output_configuration_v1::Request::DisableHead { head: _ } => {}
            // The virtual output cannot be reconfigured: reject apply/test per spec.
            zwlr_output_configuration_v1::Request::Apply => resource.failed(),
            zwlr_output_configuration_v1::Request::Test => resource.failed(),
            zwlr_output_configuration_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch2<ZwlrOutputConfigurationHeadV1, State> for OutputConfigurationHeadData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZwlrOutputConfigurationHeadV1,
        _request: zwlr_output_configuration_head_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // All setters are ignored; the parent configuration is rejected on apply/test.
    }
}
