//! The Linux zero-copy Viewer target: a GPU texture Flutter samples directly
//! via DMA-BUF (K-177, the Linux sibling of [`crate::shared`]).
//!
//! # In plain terms
//!
//! This is the Linux twin of the Windows shared-texture path. Normally the
//! Viewer's picture makes a slow round trip every frame — drawn on the graphics
//! card, copied *down* into ordinary memory, handed to Flutter, and uploaded
//! *back* onto the card. On Windows we skip that with a D3D12 shared handle; on
//! Linux the equivalent primitive is a **DMA-BUF**: a file descriptor that names
//! a piece of graphics memory, which another part of the program (Flutter's GTK
//! embedder) can import into an OpenGL texture and show without any copy. The
//! picture never leaves the graphics card.
//!
//! # How it works, precisely
//!
//! wgpu runs over Vulkan on Linux. We reach *through* wgpu to its Vulkan device
//! (`Device::as_hal`), create a `VkImage` whose memory is **exportable as a
//! DMA-BUF** (`VkExternalMemoryImageCreateInfo` + `VkExportMemoryAllocateInfo`
//! with `VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT`), bind dedicated memory,
//! and export a file descriptor for it (`vkGetMemoryFdKHR`, via ash's
//! `khr::external_memory_fd::Device`). We then wrap that same `VkImage` back up
//! as a `wgpu::Texture` (`create_texture_from_hal`) so the normal render path can
//! copy the finished, display-encoded frame into it. The fd, plus the buffer's
//! stride/offset and DRM format/modifier, is what Flutter's Linux runner imports
//! as an `EGLImage` (`EGL_EXT_image_dma_buf_import`) and binds to a GL texture.
//!
//! # The DRM format/modifier story we ship
//!
//! The image is `VK_FORMAT_R8G8B8A8_UNORM` holding the *already sRGB-encoded*
//! display bytes — byte-for-byte the same pixels the CPU read-back path produced
//! and the same bytes the Windows path stores (its `Rgba8Unorm`). We copy the
//! engine's `Rgba8UnormSrgb` display texture into this `Rgba8Unorm` one; wgpu
//! allows that copy because the two formats differ only in sRGB-ness (a verbatim
//! byte copy, no re-encode).
//!
//! We use **linear tiling** (`VK_IMAGE_TILING_LINEAR`) rather than the DRM
//! format-modifier route. The MIT reference (`flutter_wgpu_texture`) took the
//! full modifier route (`VK_EXT_image_drm_format_modifier`, querying and
//! selecting a modifier); linear tiling is the simpler, widely-supported fallback
//! the task sanctions, and it needs only the external-memory extensions, not the
//! modifier extension. The DRM fourcc reported is therefore `DRM_FORMAT_ABGR8888`
//! (memory order R,G,B,A — matching `R8G8B8A8_UNORM` and Flutter's RGBA8888) with
//! modifier `DRM_FORMAT_MOD_LINEAR` (0). The EGL import side states that modifier
//! explicitly whenever the driver advertises
//! `EGL_EXT_image_dma_buf_import_modifiers`, and omits the attributes only when it
//! does not (they are illegal without that extension). Stating them matters:
//! Nvidia's driver guesses without an explicit modifier and the import then yields
//! a black texture.
//!
//! # Device extensions (the runtime prerequisite)
//!
//! DMA-BUF export needs `VK_KHR_external_memory_fd` and
//! `VK_EXT_external_memory_dma_buf` enabled *at device-creation time*. wgpu 24's
//! Vulkan backend does not enable them by default (only its Windows sibling
//! `VK_KHR_external_memory_win32`), so [`crate::GpuContext::headless`] opens the
//! device itself through wgpu-hal with those extensions appended (see
//! [`open_device`]). If the adapter cannot enable them the context
//! falls back to a plain device and this path reports unavailable — the read-back
//! path still works. Verifying the export actually lands on the collaborator's
//! GPUs is their runtime gate (docs/GUIDE §9).
//!
//! # Synchronisation
//!
//! Same as the Windows path: after the copy we `poll(Wait)` so the GPU has
//! finished writing before Flutter is told the frame is ready. We render into the
//! *same* texture each frame; a fence handshake is the robust follow-up if
//! tearing ever shows (recorded with K-177).
//!
//! The reference for the plumbing (the Vulkan export chain, the fd/stride/offset
//! metadata, the EGL import attributes) is the MIT-licensed `flutter_wgpu_texture`
//! package; we borrow the *pattern*, not the code.

#![allow(unsafe_code)]

use crate::GpuContext;
use ash::vk;

/// DRM fourcc for 32-bpp RGBA in memory byte order R,G,B,A (`DRM_FORMAT_ABGR8888`,
/// `fourcc_code('A','B','2','4')`). This matches `VK_FORMAT_R8G8B8A8_UNORM` and
/// Flutter's `kFlutterDesktopPixelFormatRGBA8888` / GL `RGBA8`.
const DRM_FORMAT_ABGR8888: u32 = 0x3432_4241;

/// `DRM_FORMAT_MOD_LINEAR` — the buffer has no vendor tiling, so the EGL import
/// needs no modifier attributes.
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

/// The wgpu-side format of the shared texture. `Rgba8Unorm` (not `…Srgb`) so the
/// display-encoded bytes are stored verbatim and Flutter reads them as plain
/// RGBA8888 — the identical pixels the read-back path produced (matching the
/// Windows path exactly).
const SHARED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A `VkImage` whose memory is exported as a DMA-BUF, wrapped as a
/// `wgpu::Texture` and paired with the exported fd and its DRM metadata. One is
/// held for the whole Viewer session and re-created only when the comp's
/// dimensions change (a new fd is reported then). Its `wgpu::Texture` keeps the
/// underlying `VkImage`/`VkDeviceMemory` alive (they are freed by a drop callback
/// when the texture drops), so the exported fd stays valid for the texture's
/// lifetime.
pub struct SharedDmabuf {
    /// The copy destination the render path writes the finished frame into.
    pub texture: wgpu::Texture,
    /// The exported DMA-BUF file descriptor. **This struct owns it** and closes
    /// it in `Drop`, so an exported-but-never-registered fd is not leaked. The
    /// descriptor sent across the bridge is only a number to look at: the Flutter
    /// runner `dup()`s it on registration and closes its own copy on unregister.
    /// Exactly one owner per side, so neither can double-close the other's.
    fd: i32,
    stride: u32,
    offset: u32,
    pub width: u32,
    pub height: u32,
}

// The fd is an opaque OS descriptor, not a live pointer we dereference; keeping
// it as an `i32` next to a `Send`/`Sync` `wgpu::Texture` makes the whole struct
// safely shareable across the render lock, exactly as `SharedTexture` does on
// Windows.
unsafe impl Send for SharedDmabuf {}
unsafe impl Sync for SharedDmabuf {}

/// The DRM metadata one exported DMA-BUF frame carries — everything Flutter's
/// runner needs to import it as an `EGLImage`.
pub struct SharedDmabufInfo {
    pub fd: i32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub offset: u32,
    pub drm_fourcc: u32,
    pub modifier: u64,
}

impl SharedDmabuf {
    /// Create a `width`×`height` DMA-BUF-backed texture on `gpu`'s Vulkan device.
    /// `Err` when wgpu is not on the Vulkan backend (the DMA-BUF path needs
    /// Vulkan; the caller falls back to the read-back path), when the required
    /// external-memory device extensions were not enabled, or any Vulkan call
    /// fails.
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Result<Self, String> {
        let width = width.max(1);
        let height = height.max(1);

        // Reach through wgpu to the raw Vulkan device, create the exportable image
        // + memory there, export the fd, and build the wgpu-hal texture — all
        // while wgpu holds the device. The hal texture and metadata escape the
        // closure; wrapping into a `wgpu::Texture` happens outside it.
        let created = unsafe {
            gpu.device
                .as_hal::<wgpu::hal::api::Vulkan, _, _>(|hal_device| {
                    let hal_device = hal_device.ok_or_else(|| {
                        "dmabuf texture: wgpu is not running on the Vulkan backend".to_string()
                    })?;
                    create_exportable_image(hal_device, width, height)
                })
        };
        let Created {
            hal_texture,
            fd,
            stride,
            offset,
        } = created?;

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = unsafe {
            gpu.device
                .create_texture_from_hal::<wgpu::hal::api::Vulkan>(
                    hal_texture,
                    &wgpu::TextureDescriptor {
                        label: Some("lumit-shared-dmabuf"),
                        size: extent,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: SHARED_FORMAT,
                        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                )
        };

        Ok(Self {
            texture,
            fd,
            stride,
            offset,
            width,
            height,
        })
    }

    /// The DRM metadata Flutter's runner imports (fd, dimensions, stride, offset,
    /// fourcc, modifier).
    pub fn info(&self) -> SharedDmabufInfo {
        SharedDmabufInfo {
            fd: self.fd,
            width: self.width,
            height: self.height,
            stride: self.stride,
            offset: self.offset,
            drm_fourcc: DRM_FORMAT_ABGR8888,
            modifier: DRM_FORMAT_MOD_LINEAR,
        }
    }

    /// Copy the finished display texture (`Rgba8UnormSrgb`) into the DMA-BUF
    /// texture and block until the GPU has finished, so the frame is complete
    /// before Flutter is told it is ready. `display` must match this texture's
    /// dimensions (the caller recreates on a size change). Identical to the
    /// Windows path's `present`.
    pub fn present(&self, gpu: &GpuContext, display: &wgpu::Texture) {
        // The frame that produced `display` may still be sitting in the
        // batch; a copy of work that has not been submitted copies nothing.
        gpu.flush();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shared-dmabuf-present"),
            });
        encoder.copy_texture_to_texture(
            display.as_image_copy(),
            self.texture.as_image_copy(),
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        gpu.submit([encoder.finish()]);
        // No fence yet: wait for the write to land so the reader never sees a torn
        // frame (see the module note). Zero *CPU* pixel work still — the bytes
        // never leave the card.
        gpu.device.poll(wgpu::Maintain::Wait);
    }
}

impl Drop for SharedDmabuf {
    fn drop(&mut self) {
        if self.fd >= 0 {
            use std::os::fd::FromRawFd;
            unsafe {
                let _ = std::os::fd::OwnedFd::from_raw_fd(self.fd);
            }
            self.fd = -1;
        }
    }
}

/// What [`create_exportable_image`] hands back out of the `as_hal` closure.
struct Created {
    hal_texture: wgpu::hal::vulkan::Texture,
    fd: i32,
    stride: u32,
    offset: u32,
}

/// Create a linear-tiled, DMA-BUF-exportable `VkImage` with dedicated memory,
/// export its fd, read its stride/offset, and wrap it as a wgpu-hal Vulkan
/// texture whose drop callback frees the image and memory.
///
/// # Safety
/// `hal_device` must be a valid wgpu-hal Vulkan device.
unsafe fn create_exportable_image(
    hal_device: &wgpu::hal::vulkan::Device,
    width: u32,
    height: u32,
) -> Result<Created, String> {
    let raw_device: ash::Device = hal_device.raw_device().clone();
    let instance = hal_device.shared_instance().raw_instance();
    let physical = hal_device.raw_physical_device();

    // 1. The exportable image: linear tiling, DMA-BUF external handle type.
    let mut external_image = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::LINEAR)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external_image);
    let image = raw_device
        .create_image(&image_info, None)
        .map_err(|e| format!("dmabuf texture: vkCreateImage failed: {e}"))?;

    // From here on, free the image on any error before returning.
    let make = || -> Result<Created, String> {
        // 2. Dedicated, exportable memory sized to the image.
        let reqs = raw_device.get_image_memory_requirements(image);
        let mem_props = instance.get_physical_device_memory_properties(physical);
        let mem_type = pick_memory_type(&mem_props, reqs.memory_type_bits)
            .ok_or_else(|| "dmabuf texture: no suitable memory type".to_string())?;

        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(reqs.size)
            .memory_type_index(mem_type)
            .push_next(&mut export)
            .push_next(&mut dedicated);
        let memory = raw_device
            .allocate_memory(&alloc_info, None)
            .map_err(|e| format!("dmabuf texture: vkAllocateMemory failed: {e}"))?;

        if let Err(e) = raw_device.bind_image_memory(image, memory, 0) {
            raw_device.free_memory(memory, None);
            return Err(format!("dmabuf texture: vkBindImageMemory failed: {e}"));
        }

        // 3. Export the DMA-BUF fd.
        let fd_loader = ash::khr::external_memory_fd::Device::new(instance, &raw_device);
        let get_fd = vk::MemoryGetFdInfoKHR::default()
            .memory(memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let fd = match fd_loader.get_memory_fd(&get_fd) {
            Ok(fd) => fd,
            Err(e) => {
                raw_device.free_memory(memory, None);
                return Err(format!("dmabuf texture: vkGetMemoryFdKHR failed: {e}"));
            }
        };

        // 4. Stride/offset of the single colour plane (linear tiling).
        let subresource = vk::ImageSubresource {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            array_layer: 0,
        };
        let layout = raw_device.get_image_subresource_layout(image, subresource);
        let stride = layout.row_pitch as u32;
        let offset = layout.offset as u32;

        // 5. Wrap into a wgpu-hal texture whose drop callback frees image + memory.
        let cleanup_device = raw_device.clone();
        let drop_callback: wgpu::hal::DropCallback = Box::new(move || {
            cleanup_device.destroy_image(image, None);
            cleanup_device.free_memory(memory, None);
        });
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let hal_desc = wgpu::hal::TextureDescriptor {
            label: Some("lumit-shared-dmabuf"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHARED_FORMAT,
            usage: wgpu::hal::TextureUses::COPY_DST | wgpu::hal::TextureUses::RESOURCE,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: vec![],
        };
        let hal_texture =
            wgpu::hal::vulkan::Device::texture_from_raw(image, &hal_desc, Some(drop_callback));
        Ok(Created {
            hal_texture,
            fd,
            stride,
            offset,
        })
    };

    match make() {
        Ok(created) => Ok(created),
        Err(e) => {
            raw_device.destroy_image(image, None);
            Err(e)
        }
    }
}

/// Open a wgpu Vulkan device with the external-memory extensions DMA-BUF export
/// needs (`VK_KHR_external_memory`, `VK_KHR_external_memory_fd`,
/// `VK_EXT_external_memory_dma_buf`). wgpu 24's Vulkan backend does not enable
/// them by default, so we replicate wgpu-hal's own `open()` — the base extension
/// set and physical-device feature chain it computes — with those three appended,
/// then wrap the resulting device through `create_device_from_hal`. `Err` when
/// the adapter is not Vulkan or cannot enable the extensions; the caller then
/// opens a plain device and the DMA-BUF path stays unavailable.
///
/// # Safety
/// Uses the raw Vulkan device/instance underneath wgpu; the returned wgpu device
/// owns them exactly as a `request_device` device would.
pub(crate) fn open_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), String> {
    let features = wgpu::Features::empty();
    let memory_hints = wgpu::MemoryHints::default();
    // The extra device extensions, on top of what wgpu-hal enables for `features`.
    let extra: [&'static std::ffi::CStr; 3] = [
        vk::KHR_EXTERNAL_MEMORY_NAME,
        vk::KHR_EXTERNAL_MEMORY_FD_NAME,
        vk::EXT_EXTERNAL_MEMORY_DMA_BUF_NAME,
    ];

    let open = unsafe {
        adapter.as_hal::<wgpu::hal::api::Vulkan, _, _>(|hal_adapter| {
            let hal_adapter = hal_adapter
                .ok_or_else(|| "dmabuf device: not on the Vulkan backend".to_string())?;
            let mut extensions = hal_adapter.required_device_extensions(features);
            for name in extra {
                if !extensions.contains(&name) {
                    extensions.push(name);
                }
            }
            let mut phd_features = hal_adapter.physical_device_features(&extensions, features);

            let family_index = 0u32;
            let priorities = [1.0f32];
            let queue_info = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family_index)
                .queue_priorities(&priorities);
            let queue_infos = [queue_info];
            let ext_ptrs: Vec<*const std::os::raw::c_char> =
                extensions.iter().map(|s| s.as_ptr()).collect();
            let create_info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&queue_infos)
                .enabled_extension_names(&ext_ptrs);
            let create_info = phd_features.add_to_device_create(create_info);

            let instance = hal_adapter.shared_instance().raw_instance();
            let physical = hal_adapter.raw_physical_device();
            let raw_device = instance
                .create_device(physical, &create_info, None)
                .map_err(|e| format!("dmabuf device: vkCreateDevice failed: {e}"))?;

            hal_adapter
                .device_from_raw(
                    raw_device,
                    None,
                    &extensions,
                    features,
                    &memory_hints,
                    family_index,
                    0,
                )
                .map_err(|e| format!("dmabuf device: device_from_raw failed: {e:?}"))
        })
    }?;

    let (device, queue) = unsafe {
        adapter.create_device_from_hal::<wgpu::hal::api::Vulkan>(
            open,
            &wgpu::DeviceDescriptor::default(),
            None,
        )
    }
    .map_err(|e| format!("dmabuf device: create_device_from_hal failed: {e}"))?;
    Ok((device, queue))
}

/// Pick a memory type index satisfying `type_bits`, preferring device-local.
fn pick_memory_type(props: &vk::PhysicalDeviceMemoryProperties, type_bits: u32) -> Option<u32> {
    let types = &props.memory_types[..props.memory_type_count as usize];
    // Prefer a device-local type (the GPU's own memory), else any allowed type.
    types
        .iter()
        .enumerate()
        .find(|(i, t)| {
            type_bits & (1 << i) != 0
                && t.property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .or_else(|| {
            types
                .iter()
                .enumerate()
                .find(|(i, _)| type_bits & (1 << i) != 0)
        })
        .map(|(i, _)| i as u32)
}
