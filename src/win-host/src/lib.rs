//! Library portion of the Windows WebGPU host. The pure, testable logic (argument parsing,
//! input coordinate/keycode mapping, shared-memory access) lives here; the binary
//! ([`main`](../main.rs)) is a thin event-loop wrapper around it.

pub mod args;
pub mod bridge;
pub mod gpu;
pub mod input;
pub mod shared;

use bridge_protocol::ServerMessage;

/// Events delivered to the winit loop from the background control-channel thread.
#[derive(Debug)]
pub enum UserEvent {
    Server(ServerMessage),
}
