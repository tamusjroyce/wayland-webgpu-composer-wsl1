//! Compositor state and Wayland socket setup.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Instant;

use bridge_protocol::FrameLayout;
use memmap2::MmapMut;
use smithay::desktop::{PopupManager, Space, Window, WindowSurfaceType};
use smithay::input::{Seat, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode as CalloopMode, PostAction};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{Logical, Point, Transform};
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelListState;
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::idle_notify::IdleNotifierState;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::pointer_gestures::PointerGesturesState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::ext_data_control::DataControlState as ExtDataControlState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xdg_activation::XdgActivationState;

use crate::bridge::Bridge;

pub struct State {
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub running: bool,
    pub output: Output,

    // Shared framebuffer target.
    pub map: MmapMut,
    pub layout: FrameLayout,
    pub scratch: Vec<u8>,
    pub bridge: Bridge,

    // Smithay protocol state.
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    // wlroots-compatibility protocol globals.
    pub xdg_decoration_state: XdgDecorationState,
    pub xdg_activation_state: XdgActivationState,
    pub primary_selection_state: PrimarySelectionState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub fractional_scale_state: FractionalScaleManagerState,
    pub viewporter_state: ViewporterState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub relative_pointer_state: RelativePointerManagerState,
    pub pointer_gestures_state: PointerGesturesState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub idle_notifier: IdleNotifierState<State>,
    pub foreign_toplevel_list: ForeignToplevelListState,
    pub data_control_state: DataControlState,
    pub ext_data_control_state: ExtDataControlState,
    pub popups: PopupManager,
    pub seat: Seat<Self>,
}

impl State {
    pub fn new(
        event_loop: &mut EventLoop<'static, Self>,
        display: Display<Self>,
        layout: FrameLayout,
        map: MmapMut,
        bridge: Bridge,
    ) -> Self {
        let start_time = Instant::now();
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        // wlroots-compatibility protocol globals (advertised via delegate_dispatch2!).
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let keyboard_shortcuts_inhibit_state = KeyboardShortcutsInhibitState::new::<Self>(&dh);
        let fractional_scale_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let pointer_gestures_state = PointerGesturesState::new::<Self>(&dh);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        // Global-only protocols (no state to keep; the global persists in the display).
        smithay::wayland::content_type::ContentTypeState::new::<Self>(&dh);
        smithay::wayland::alpha_modifier::AlphaModifierState::new::<Self>(&dh);
        smithay::wayland::text_input::TextInputManagerState::new::<Self>(&dh);
        smithay::wayland::tablet_manager::TabletManagerState::new::<Self>(&dh);
        smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        smithay::wayland::presentation::PresentationState::new::<Self>(&dh, 1); // CLOCK_MONOTONIC
        smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(&dh);
        smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<Self, _>(&dh, |_client| true);
        let idle_notifier = IdleNotifierState::<State>::new(&dh, event_loop.handle());
        let foreign_toplevel_list = ForeignToplevelListState::new::<Self>(&dh);
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_client| true);
        let ext_data_control_state =
            ExtDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_client| true);
        smithay::wayland::input_method::InputMethodManagerState::new::<Self, _>(&dh, |_client| true);
        let popups = PopupManager::default();

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "wwc");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let mut space = Space::default();

        // A single output matching the shared framebuffer size.
        let output = Output::new(
            "wwc-webgpu".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "WaylandWebGPUComposer".into(),
                model: "WebGPU".into(),
                serial_number: "1".into(),
            },
        );
        let _global = output.create_global::<Self>(&dh);
        let mode = Mode {
            size: (layout.width as i32, layout.height as i32).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), Some(Transform::Normal), None, Some((0, 0).into()));
        output.set_preferred(mode);
        space.map_output(&output, (0, 0));

        let socket_name = Self::init_wayland_listener(display, event_loop);

        let scratch = vec![0u8; layout.slot_bytes() as usize];

        Self {
            start_time,
            socket_name,
            display_handle: dh,
            space,
            running: true,
            output,
            map,
            layout,
            scratch,
            bridge,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            xdg_decoration_state,
            xdg_activation_state,
            primary_selection_state,
            keyboard_shortcuts_inhibit_state,
            fractional_scale_state,
            viewporter_state,
            single_pixel_buffer_state,
            relative_pointer_state,
            pointer_gestures_state,
            pointer_constraints_state,
            idle_notifier,
            foreign_toplevel_list,
            data_control_state,
            ext_data_control_state,
            popups,
            seat,
        }
    }

    fn init_wayland_listener(display: Display<State>, event_loop: &mut EventLoop<Self>) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let handle = event_loop.handle();

        handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("failed to init the wayland event source");

        handle
            .insert_source(
                Generic::new(display, Interest::READ, CalloopMode::Level),
                |_, display, state| {
                    // SAFETY: we do not drop the display.
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space.element_under(pos).and_then(|(window, location)| {
            window
                .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + location).to_f64()))
        })
    }
}

/// Per-client state.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
