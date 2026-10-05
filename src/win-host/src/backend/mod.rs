//! Host renderer abstraction. The `--backend` parameter selects one of these implementations
//! at runtime; both present the shared framebuffer (and, later, per-surface `GpuScene` quads)
//! into the single scalable window.
//!
//! - [`Backend::Webgpu`] -> [`crate::gpu::GpuState`] (the portable `wgpu` renderer, default).
//! - [`Backend::Vulkan`] -> [`vulkan::VulkanState`] (raw Vulkan via `ash` + `gpu-allocator`),
//!   compiled only when the `vulkan` cargo feature is enabled.

use std::sync::Arc;

use bridge_protocol::Backend;
use winit::window::Window;

#[cfg(feature = "vulkan")]
pub mod vulkan;

/// Outcome of a present, abstracted over the concrete backend's error type so `main` does not
/// depend on `wgpu` (or Vulkan) error enums directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderOutcome {
    /// Frame presented.
    Presented,
    /// Surface/swapchain was lost or outdated; the caller should resize/recreate and retry.
    Lost,
    /// Device is out of memory; the caller should exit.
    OutOfMemory,
    /// A non-fatal error occurred; logged by the backend, the caller may continue.
    Error,
}

/// What `main`'s event loop needs from a renderer, independent of the GPU API underneath.
pub trait Renderer {
    /// The window this renderer presents into.
    fn window(&self) -> &Arc<Window>;
    /// Current window (swapchain/surface) size in physical pixels.
    fn window_size(&self) -> (u32, u32);
    /// Current uploaded framebuffer texture size in pixels.
    fn tex_size(&self) -> (u32, u32);
    /// Resize the presentation surface to `(width, height)`.
    fn resize(&mut self, width: u32, height: u32);
    /// Upload BGRA framebuffer bytes (whole-output, pre-composited path).
    fn upload(&mut self, width: u32, height: u32, bgra: &[u8]);
    /// Present the current frame, scaled/letterboxed into the window.
    fn render(&mut self) -> RenderOutcome;
}

/// Build the renderer for the selected `backend`. Falls back to `wgpu` with a warning if the
/// Vulkan backend is requested but unavailable (feature disabled or initialization failed), so
/// the host always produces a window.
pub fn create(backend: Backend, window: Arc<Window>) -> Box<dyn Renderer> {
    match backend {
        Backend::Webgpu => {
            log::info!("renderer: webgpu (wgpu)");
            Box::new(crate::gpu::GpuState::new(window))
        }
        Backend::Vulkan => {
            #[cfg(feature = "vulkan")]
            {
                match vulkan::VulkanState::new(window.clone()) {
                    Ok(v) => {
                        log::info!("renderer: vulkan (ash + gpu-allocator)");
                        return Box::new(v);
                    }
                    Err(e) => log::error!("vulkan backend init failed, falling back to webgpu: {e}"),
                }
            }
            #[cfg(not(feature = "vulkan"))]
            log::warn!(
                "vulkan backend requested but win-host was built without the 'vulkan' feature; \
                 falling back to webgpu"
            );
            Box::new(crate::gpu::GpuState::new(window))
        }
    }
}
