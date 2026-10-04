//! Hand-rolled wlroots/staging protocols not provided by the vendored smithay.
//!
//! These use smithay's [`Dispatch2`]/[`GlobalDispatch2`] trait system (the `delegate_dispatch2!`
//! macro turns them into real `wayland_server::Dispatch`/`GlobalDispatch` impls). Behaviour is
//! modelled on `labwc/` (wlroots) but written directly against the re-exported protocol bindings.
//!
//! [`Dispatch2`]: smithay::wayland::Dispatch2
//! [`GlobalDispatch2`]: smithay::wayland::GlobalDispatch2

pub mod foreign_toplevel;
pub mod output_management;
pub mod session_lock;
pub mod tearing;
