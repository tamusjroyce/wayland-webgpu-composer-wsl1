//! Raw-Vulkan host renderer: `ash` for the Vulkan API, `gpu-allocator` for all
//! device-memory suballocation. Compiled only with `--features vulkan`.
//!
//! Presentation strategy: the compositor hands us a BGRA framebuffer; we upload it into a
//! device-local `VkImage` (staging buffer -> image copy) and present it with
//! `vkCmdBlitImage`, which does the aspect-correct scaling on the GPU's transfer/blit unit.
//! This deliberately avoids a graphics pipeline/shaders/descriptors: the whole-output blit
//! needs none of them, which keeps this backend small and robust while still giving the
//! compositor full, explicit control over GPU memory (via `gpu-allocator`) and the present
//! path. The per-surface `GpuScene` compositor (plan.md Phase 7/8) layers on top later.

use std::error::Error;
use std::sync::Arc;

use ash::vk;
use gpu_allocator::vulkan::{
    Allocation, AllocationCreateDesc, AllocationScheme, Allocator, AllocatorCreateDesc,
};
use gpu_allocator::MemoryLocation;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

use crate::backend::{RenderOutcome, Renderer};

/// A device-local image holding the latest uploaded framebuffer, kept in
/// `TRANSFER_SRC_OPTIMAL` so it is ready to blit into the swapchain.
struct Texture {
    image: vk::Image,
    allocation: Option<Allocation>,
    width: u32,
    height: u32,
}

/// Persistently-mapped host-visible staging buffer used to feed `vkCmdCopyBufferToImage`.
struct Staging {
    buffer: vk::Buffer,
    allocation: Option<Allocation>,
    size: u64,
}

pub struct VulkanState {
    window: Arc<Window>,

    // Kept alive for the lifetime of the instance/device.
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,

    swapchain_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swap_images: Vec<vk::Image>,
    surface_format: vk::SurfaceFormatKHR,
    present_mode: vk::PresentModeKHR,
    extent: vk::Extent2D,

    cmd_pool: vk::CommandPool,
    frame_cmd: vk::CommandBuffer,
    upload_cmd: vk::CommandBuffer,
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,

    allocator: Option<Allocator>,
    texture: Option<Texture>,
    staging: Option<Staging>,
}

const CLEAR: [f32; 4] = [0.02, 0.02, 0.02, 1.0];

impl VulkanState {
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn Error>> {
        let entry = unsafe { ash::Entry::load()? };

        let display_handle = window.display_handle()?.as_raw();
        let window_handle = window.window_handle()?.as_raw();

        // --- Instance ---
        let app_info =
            vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 0, 0));
        let ext_names = ash_window::enumerate_required_extensions(display_handle)?;
        let instance_ci = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(ext_names);
        let instance = unsafe { entry.create_instance(&instance_ci, None)? };

        // --- Surface ---
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)?
        };
        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);

        // --- Physical device + queue family (graphics + present) ---
        let (physical_device, queue_family) =
            pick_device(&instance, &surface_loader, surface)?;
        let props = unsafe { instance.get_physical_device_properties(physical_device) };
        let name = unsafe { std::ffi::CStr::from_ptr(props.device_name.as_ptr()) };
        log::info!("vulkan adapter: {}", name.to_string_lossy());

        // --- Logical device + queue ---
        let priorities = [1.0f32];
        let queue_ci = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities);
        let device_exts = [ash::khr::swapchain::NAME.as_ptr()];
        let device_ci = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_ci))
            .enabled_extension_names(&device_exts);
        let device = unsafe { instance.create_device(physical_device, &device_ci, None)? };
        let queue = unsafe { device.get_device_queue(queue_family, 0) };

        // --- Allocator (owns all non-swapchain device memory) ---
        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance: instance.clone(),
            device: device.clone(),
            physical_device,
            debug_settings: Default::default(),
            buffer_device_address: false,
            allocation_sizes: Default::default(),
        })?;

        // --- Command pool + buffers ---
        let cmd_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?
        };
        let bufs = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(cmd_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(2),
            )?
        };
        let (frame_cmd, upload_cmd) = (bufs[0], bufs[1]);

        // --- Per-frame sync ---
        let sem_ci = vk::SemaphoreCreateInfo::default();
        let image_available = unsafe { device.create_semaphore(&sem_ci, None)? };
        let render_finished = unsafe { device.create_semaphore(&sem_ci, None)? };
        let in_flight = unsafe {
            device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?
        };

        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);

        let size = window.inner_size();
        let mut this = VulkanState {
            window,
            _entry: entry,
            instance,
            surface_loader,
            surface,
            physical_device,
            device,
            queue,
            swapchain_loader,
            swapchain: vk::SwapchainKHR::null(),
            swap_images: Vec::new(),
            surface_format: vk::SurfaceFormatKHR::default(),
            present_mode: vk::PresentModeKHR::FIFO,
            extent: vk::Extent2D { width: size.width.max(1), height: size.height.max(1) },
            cmd_pool,
            frame_cmd,
            upload_cmd,
            image_available,
            render_finished,
            in_flight,
            allocator: Some(allocator),
            texture: None,
            staging: None,
        };
        this.create_swapchain(size.width.max(1), size.height.max(1))?;
        Ok(this)
    }

    /// (Re)create the swapchain at `(width, height)`; destroys any previous one.
    fn create_swapchain(&mut self, width: u32, height: u32) -> Result<(), Box<dyn Error>> {
        let caps = unsafe {
            self.surface_loader
                .get_physical_device_surface_capabilities(self.physical_device, self.surface)?
        };
        let formats = unsafe {
            self.surface_loader
                .get_physical_device_surface_formats(self.physical_device, self.surface)?
        };
        let surface_format = formats
            .iter()
            .copied()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .unwrap_or(formats[0]);

        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: height
                    .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };

        let mut image_count = caps.min_image_count + 1;
        if caps.max_image_count > 0 {
            image_count = image_count.min(caps.max_image_count);
        }

        let pre_transform = if caps
            .supported_transforms
            .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            caps.current_transform
        };

        let old = self.swapchain;
        let ci = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            // We clear + blit into the swapchain image, so it is a transfer destination.
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(pre_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(self.present_mode)
            .clipped(true)
            .old_swapchain(old);

        let swapchain = unsafe { self.swapchain_loader.create_swapchain(&ci, None)? };
        if old != vk::SwapchainKHR::null() {
            unsafe { self.swapchain_loader.destroy_swapchain(old, None) };
        }
        self.swapchain = swapchain;
        self.swap_images =
            unsafe { self.swapchain_loader.get_swapchain_images(swapchain)? };
        self.surface_format = surface_format;
        self.extent = extent;
        Ok(())
    }

    /// Allocate a buffer/image through `gpu-allocator` and bind it.
    fn alloc_image(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Texture, Box<dyn Error>> {
        let ci = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::B8G8R8A8_UNORM)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.device.create_image(&ci, None)? };
        let req = unsafe { self.device.get_image_memory_requirements(image) };
        let allocation = self.allocator.as_mut().unwrap().allocate(
            &AllocationCreateDesc {
                name: "framebuffer-image",
                requirements: req,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            },
        )?;
        unsafe {
            self.device
                .bind_image_memory(image, allocation.memory(), allocation.offset())?
        };
        Ok(Texture { image, allocation: Some(allocation), width, height })
    }

    fn ensure_staging(&mut self, size: u64) -> Result<(), Box<dyn Error>> {
        if self.staging.as_ref().map(|s| s.size).unwrap_or(0) >= size {
            return Ok(());
        }
        self.free_staging();
        let ci = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe { self.device.create_buffer(&ci, None)? };
        let req = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let allocation = self.allocator.as_mut().unwrap().allocate(
            &AllocationCreateDesc {
                name: "staging-buffer",
                requirements: req,
                location: MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            },
        )?;
        unsafe {
            self.device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())?
        };
        self.staging = Some(Staging { buffer, allocation: Some(allocation), size });
        Ok(())
    }

    fn free_staging(&mut self) {
        if let Some(mut s) = self.staging.take() {
            if let (Some(alloc), Some(allocator)) = (s.allocation.take(), self.allocator.as_mut())
            {
                let _ = allocator.free(alloc);
            }
            unsafe { self.device.destroy_buffer(s.buffer, None) };
        }
    }

    fn free_texture(&mut self) {
        if let Some(mut t) = self.texture.take() {
            if let (Some(alloc), Some(allocator)) = (t.allocation.take(), self.allocator.as_mut())
            {
                let _ = allocator.free(alloc);
            }
            unsafe { self.device.destroy_image(t.image, None) };
        }
    }

    fn upload_impl(&mut self, width: u32, height: u32, bgra: &[u8]) -> Result<(), Box<dyn Error>> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        let needed = (width as u64) * (height as u64) * 4;
        self.ensure_staging(needed)?;

        // Copy pixels into the persistently-mapped staging buffer.
        if let Some(s) = self.staging.as_mut() {
            if let Some(slice) = s.allocation.as_mut().and_then(|a| a.mapped_slice_mut()) {
                let n = (needed as usize).min(bgra.len()).min(slice.len());
                slice[..n].copy_from_slice(&bgra[..n]);
            }
        }

        // (Re)create the device-local image if the size changed.
        if self.texture.as_ref().map(|t| (t.width, t.height)) != Some((width, height)) {
            self.free_texture();
            let tex = self.alloc_image(width, height)?;
            self.texture = Some(tex);
        }
        let image = self.texture.as_ref().unwrap().image;
        let buffer = self.staging.as_ref().unwrap().buffer;

        unsafe {
            self.device.reset_command_buffer(
                self.upload_cmd,
                vk::CommandBufferResetFlags::empty(),
            )?;
            self.device.begin_command_buffer(
                self.upload_cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;

            image_barrier(
                &self.device,
                self.upload_cmd,
                image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
            );

            let region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(width)
                .buffer_image_height(height)
                .image_subresource(color_layers())
                .image_extent(vk::Extent3D { width, height, depth: 1 });
            self.device.cmd_copy_buffer_to_image(
                self.upload_cmd,
                buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                std::slice::from_ref(&region),
            );

            image_barrier(
                &self.device,
                self.upload_cmd,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::TRANSFER_READ,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
            );

            self.device.end_command_buffer(self.upload_cmd)?;
            let submit = vk::SubmitInfo::default()
                .command_buffers(std::slice::from_ref(&self.upload_cmd));
            self.device
                .queue_submit(self.queue, std::slice::from_ref(&submit), vk::Fence::null())?;
            self.device.queue_wait_idle(self.queue)?;
        }
        Ok(())
    }

    fn render_impl(&mut self) -> Result<RenderOutcome, vk::Result> {
        unsafe {
            self.device
                .wait_for_fences(std::slice::from_ref(&self.in_flight), true, u64::MAX)?;

            let acquire = self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available,
                vk::Fence::null(),
            );
            let (index, _suboptimal) = match acquire {
                Ok(v) => v,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(RenderOutcome::Lost),
                Err(e) => return Err(e),
            };
            self.device
                .reset_fences(std::slice::from_ref(&self.in_flight))?;

            let target = self.swap_images[index as usize];
            self.device
                .reset_command_buffer(self.frame_cmd, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                self.frame_cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;

            // Swapchain image -> TRANSFER_DST for clear + blit.
            image_barrier(
                &self.device,
                self.frame_cmd,
                target,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
            );

            let clear = vk::ClearColorValue { float32: CLEAR };
            self.device.cmd_clear_color_image(
                self.frame_cmd,
                target,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &clear,
                std::slice::from_ref(&color_range()),
            );

            if let Some(tex) = self.texture.as_ref() {
                let [x0, y0, x1, y1] =
                    aspect_fit(self.extent.width, self.extent.height, tex.width, tex.height);
                let blit = vk::ImageBlit::default()
                    .src_subresource(color_layers())
                    .src_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: tex.width as i32, y: tex.height as i32, z: 1 },
                    ])
                    .dst_subresource(color_layers())
                    .dst_offsets([
                        vk::Offset3D { x: x0, y: y0, z: 0 },
                        vk::Offset3D { x: x1, y: y1, z: 1 },
                    ]);
                self.device.cmd_blit_image(
                    self.frame_cmd,
                    tex.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    target,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    std::slice::from_ref(&blit),
                    vk::Filter::LINEAR,
                );
            }

            // Swapchain image -> PRESENT.
            image_barrier(
                &self.device,
                self.frame_cmd,
                target,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::PRESENT_SRC_KHR,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::empty(),
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            );

            self.device.end_command_buffer(self.frame_cmd)?;

            let wait_stages = [vk::PipelineStageFlags::TRANSFER];
            let submit = vk::SubmitInfo::default()
                .wait_semaphores(std::slice::from_ref(&self.image_available))
                .wait_dst_stage_mask(&wait_stages)
                .command_buffers(std::slice::from_ref(&self.frame_cmd))
                .signal_semaphores(std::slice::from_ref(&self.render_finished));
            self.device
                .queue_submit(self.queue, std::slice::from_ref(&submit), self.in_flight)?;

            let indices = [index];
            let present = vk::PresentInfoKHR::default()
                .wait_semaphores(std::slice::from_ref(&self.render_finished))
                .swapchains(std::slice::from_ref(&self.swapchain))
                .image_indices(&indices);
            match self.swapchain_loader.queue_present(self.queue, &present) {
                Ok(_) => Ok(RenderOutcome::Presented),
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                    Ok(RenderOutcome::Lost)
                }
                Err(e) => Err(e),
            }
        }
    }
}

impl Renderer for VulkanState {
    fn window(&self) -> &Arc<Window> {
        &self.window
    }
    fn window_size(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }
    fn tex_size(&self) -> (u32, u32) {
        self.texture
            .as_ref()
            .map(|t| (t.width, t.height))
            .unwrap_or((0, 0))
    }
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        unsafe { let _ = self.device.device_wait_idle(); };
        if let Err(e) = self.create_swapchain(width, height) {
            log::error!("vulkan swapchain recreation failed: {e}");
        }
    }
    fn upload(&mut self, width: u32, height: u32, bgra: &[u8]) {
        if let Err(e) = self.upload_impl(width, height, bgra) {
            log::warn!("vulkan upload failed: {e}");
        }
    }
    fn render(&mut self) -> RenderOutcome {
        match self.render_impl() {
            Ok(outcome) => outcome,
            Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY | vk::Result::ERROR_OUT_OF_HOST_MEMORY) => {
                RenderOutcome::OutOfMemory
            }
            Err(e) => {
                log::warn!("vulkan render error: {e:?}");
                RenderOutcome::Error
            }
        }
    }
}

impl Drop for VulkanState {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.free_texture();
            self.free_staging();
            // Drop the allocator before destroying the device it borrows.
            self.allocator = None;
            self.device.destroy_semaphore(self.image_available, None);
            self.device.destroy_semaphore(self.render_finished, None);
            self.device.destroy_fence(self.in_flight, None);
            self.device.destroy_command_pool(self.cmd_pool, None);
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_loader.destroy_swapchain(self.swapchain, None);
            }
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

/// Pick the first physical device exposing a queue family with both graphics and present.
fn pick_device(
    instance: &ash::Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, u32), Box<dyn Error>> {
    let devices = unsafe { instance.enumerate_physical_devices()? };
    for pd in devices {
        let families = unsafe { instance.get_physical_device_queue_family_properties(pd) };
        for (i, f) in families.iter().enumerate() {
            let graphics = f.queue_flags.contains(vk::QueueFlags::GRAPHICS);
            let present = unsafe {
                surface_loader
                    .get_physical_device_surface_support(pd, i as u32, surface)
                    .unwrap_or(false)
            };
            if graphics && present {
                return Ok((pd, i as u32));
            }
        }
    }
    Err("no Vulkan device with a graphics+present queue".into())
}

fn color_layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .mip_level(0)
        .base_array_layer(0)
        .layer_count(1)
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1)
}

#[allow(clippy::too_many_arguments)]
fn image_barrier(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    src_access: vk::AccessFlags,
    dst_access: vk::AccessFlags,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
) {
    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old)
        .new_layout(new)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(color_range())
        .src_access_mask(src_access)
        .dst_access_mask(dst_access);
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            std::slice::from_ref(&barrier),
        );
    }
}

/// Aspect-fit `tex` inside `win`, centered (letterboxed). Returns `[x0, y0, x1, y1]`.
fn aspect_fit(win_w: u32, win_h: u32, tex_w: u32, tex_h: u32) -> [i32; 4] {
    if tex_w == 0 || tex_h == 0 {
        return [0, 0, win_w as i32, win_h as i32];
    }
    let scale =
        (win_w as f32 / tex_w as f32).min(win_h as f32 / tex_h as f32);
    let draw_w = (tex_w as f32 * scale).round();
    let draw_h = (tex_h as f32 * scale).round();
    let x0 = ((win_w as f32 - draw_w) * 0.5).round() as i32;
    let y0 = ((win_h as f32 - draw_h) * 0.5).round() as i32;
    [x0, y0, x0 + draw_w as i32, y0 + draw_h as i32]
}

#[cfg(test)]
mod tests {
    use super::aspect_fit;

    #[test]
    fn aspect_fit_identity_when_equal() {
        assert_eq!(aspect_fit(100, 100, 100, 100), [0, 0, 100, 100]);
    }

    #[test]
    fn aspect_fit_letterboxes_wide_window() {
        // 200x100 window, 100x100 texture -> 100x100 centered horizontally.
        assert_eq!(aspect_fit(200, 100, 100, 100), [50, 0, 150, 100]);
    }

    #[test]
    fn aspect_fit_pillarboxes_tall_window() {
        // 100x200 window, 100x100 texture -> 100x100 centered vertically.
        assert_eq!(aspect_fit(100, 200, 100, 100), [0, 50, 100, 150]);
    }

    #[test]
    fn aspect_fit_zero_texture_is_full_window() {
        assert_eq!(aspect_fit(640, 480, 0, 0), [0, 0, 640, 480]);
    }
}
