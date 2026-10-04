//! `wp_tearing_control_v1` (staging). Reference: labwc `src/tearing.c`.
//!
//! The shared-memory present path always publishes whole frames, so there is no tearing to
//! enable. The protocol is still advertised and fully honoured at the wire level: clients can
//! create a per-surface control object and set the presentation hint; the hint is accepted and
//! ignored, which the spec explicitly permits ("The compositor is free to dynamically respect
//! or ignore this hint").

use smithay::reexports::wayland_protocols::wp::tearing_control::v1::server::{
    wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
    wp_tearing_control_v1::{self, WpTearingControlV1},
};
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::state::State;

/// Global data for the `wp_tearing_control_manager_v1` global.
pub struct TearingControlGlobal;
/// User data for a bound manager object.
pub struct TearingControlManager;
/// User data for a per-surface `wp_tearing_control_v1` object.
pub struct TearingControlData;

/// Create the `wp_tearing_control_manager_v1` global.
pub fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, WpTearingControlManagerV1, TearingControlGlobal>(
        1,
        TearingControlGlobal,
    );
}

impl GlobalDispatch2<WpTearingControlManagerV1, State> for TearingControlGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WpTearingControlManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, TearingControlManager);
    }
}

impl Dispatch2<WpTearingControlManagerV1, State> for TearingControlManager {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &WpTearingControlManagerV1,
        request: wp_tearing_control_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            wp_tearing_control_manager_v1::Request::GetTearingControl { id, surface: _ } => {
                data_init.init(id, TearingControlData);
            }
            wp_tearing_control_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch2<WpTearingControlV1, State> for TearingControlData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &WpTearingControlV1,
        _request: wp_tearing_control_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // set_presentation_hint / destroy: nothing to do on the software present path.
    }
}
