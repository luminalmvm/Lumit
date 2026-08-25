//! The macOS zero-copy Viewer target: a GPU texture Flutter samples directly
//! via an IOSurface (K-195, the macOS sibling of [`crate::shared`] and
//! [`crate::shared_linux`]).
//!
//! # In plain terms
//!
//! This is the Mac twin of the Windows shared-texture and Linux DMA-BUF paths.
//! Normally a Viewer picture would make a slow round trip every frame — drawn on
//! the graphics card, copied *down* into ordinary memory, handed to Flutter, and
//! uploaded *back* onto the card. Every platform has one primitive that lets two
//! parts of a program point at the *same* piece of graphics memory instead: on
//! Windows a shared handle, on Linux a DMA-BUF file descriptor, and on macOS an
//! **IOSurface**. We draw into an IOSurface-backed texture and tell Flutter its
//! numeric id; Flutter's runner wraps that same memory in a `CVPixelBuffer` and
//! shows it. No pixel is ever copied off the card.
//!
//! # How it works, precisely
//!
//! wgpu runs over Metal on macOS. We create an `IOSurface` (`IOSurfaceCreate`
//! with the width/height/bytes-per-element/pixel-format properties), ask the
//! Metal device for a texture backed by it
//! (`newTextureWithDescriptor:iosurface:plane:`), and wrap that `MTLTexture`
//! back up as a `wgpu::Texture` (`create_texture_from_hal`) so the ordinary
//! render path can copy the finished, display-encoded frame into it. What
//! crosses the bridge is the surface's `IOSurfaceID` — a plain 32-bit number,
//! which the Swift side turns back into the surface with `IOSurfaceLookup`.
//!
//! # Why BGRA
//!
//! The surface's pixel format is `'BGRA'` (`kCVPixelFormatType_32BGRA`), the one
//! format Flutter's macOS texture path accepts, so the texture is `Bgra8Unorm`
//! holding the *already sRGB-encoded* display bytes — byte-for-byte what the
//! Windows path stores, for exactly the same reason (its consumer wants BGRA
//! too). The renderer is asked for a BGRA display texture on this platform, and
//! the copy is then a verbatim byte copy between two formats that differ only in
//! sRGB-ness, which wgpu allows.
//!
//! # Storage mode
//!
//! On a unified-memory Mac (Apple silicon) the texture is `Shared`: the CPU and
//! every GPU see one copy, so the frame Flutter reads is the frame we wrote. On
//! a Mac with a discrete GPU there is no such guarantee and the texture must be
//! `Managed`.
//!
//! # Synchronisation
//!
//! Same as the other two paths: after the copy we `poll(Wait)` so the GPU has
//! finished writing before Flutter is told the frame is ready. We render into the
//! *same* surface each frame; a fence handshake is the robust follow-up if
//! tearing ever shows (recorded with K-177).

#![allow(unsafe_code)]
// `objc`'s `sel!` expansion tests `feature = "cargo-clippy"`, a feature no crate
// here declares, so every `msg_send!` in this module would otherwise trip the
// unexpected-cfg lint — which CI runs as an error.
#![allow(unexpected_cfgs)]

use crate::GpuContext;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use metal::foreign_types::ForeignType;
use objc::{msg_send, sel, sel_impl};
use std::ffi::c_void;

/// An opaque `IOSurfaceRef`.
type IOSurfaceRef = *const c_void;

#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceCreate(properties: CFDictionaryRef) -> IOSurfaceRef;
    fn IOSurfaceGetID(buffer: IOSurfaceRef) -> u32;
}

/// `kCVPixelFormatType_32BGRA` — the four-character code `'BGRA'`, the pixel
/// format Flutter's macOS `CVPixelBuffer` texture path accepts.
const PIXEL_FORMAT_BGRA: i64 = 0x4247_5241;

/// The wgpu-side format of the shared texture. `Bgra8Unorm` (not `…Srgb`) so the
/// display-encoded bytes are stored verbatim, exactly as the Windows path does.
const SHARED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// An `IOSurface` wrapped as a `wgpu::Texture`, paired with the surface id
/// Flutter looks it up by. One is held for the whole Viewer session and
/// re-created only when the comp's dimensions change (a new id is reported
/// then). The `wgpu::Texture` owns the `MTLTexture`, which retains the surface,
/// so the id stays valid for the texture's lifetime; our own retain is released
/// on drop.
pub struct SharedIoSurface {
    /// The copy destination the render path writes the finished frame into.
    pub texture: wgpu::Texture,
    /// The surface itself, retained by us (`IOSurfaceCreate` returns +1).
    surface: IOSurfaceRef,
    /// `IOSurfaceGetID` — what crosses the bridge, and what the Swift side
    /// passes to `IOSurfaceLookup`.
    id: u32,
    pub width: u32,
    pub height: u32,
}

// The surface pointer is an immutable Core Foundation object we only ever hand
// to Metal and read an id from; keeping it beside a `Send`/`Sync` `wgpu::Texture`
// makes the whole struct shareable across the render lock, exactly as the
// Windows and Linux siblings are.
unsafe impl Send for SharedIoSurface {}
unsafe impl Sync for SharedIoSurface {}

impl Drop for SharedIoSurface {
    fn drop(&mut self) {
        if !self.surface.is_null() {
            unsafe { CFRelease(self.surface as CFTypeRef) };
        }
    }
}

impl SharedIoSurface {
    /// Create a `width`×`height` IOSurface-backed texture on `gpu`'s Metal
    /// device. `Err` when wgpu is not on the Metal backend, when the surface
    /// cannot be created, or when Metal declines to back a texture with it — the
    /// caller then reports "no shared frame" and drops the frame.
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Result<Self, String> {
        let width = width.max(1);
        let height = height.max(1);

        let surface = create_surface(width, height)?;
        // From here on the surface is ours to release on any error.
        let made = unsafe {
            gpu.device
                .as_hal::<wgpu::hal::api::Metal, _, _>(|hal_device| {
                    let hal_device = hal_device.ok_or_else(|| {
                        "iosurface texture: wgpu is not running on the Metal backend".to_string()
                    })?;
                    let raw_device = hal_device.raw_device().lock();
                    texture_from_surface(&raw_device, surface, width, height)
                })
        };
        let hal_texture = match made {
            Ok(t) => t,
            Err(e) => {
                unsafe { CFRelease(surface as CFTypeRef) };
                return Err(e);
            }
        };

        let texture = unsafe {
            gpu.device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("lumit-shared-iosurface"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SHARED_FORMAT,
                    usage: wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
            )
        };

        Ok(Self {
            texture,
            surface,
            id: unsafe { IOSurfaceGetID(surface) },
            width,
            height,
        })
    }

    /// The `IOSurfaceID` Flutter's runner looks the surface up by. Widened to
    /// `u64` so it rides the same bridge field as the Windows shared handle.
    pub fn handle(&self) -> u64 {
        u64::from(self.id)
    }

    /// Copy the finished display texture (`Bgra8UnormSrgb`) into the shared
    /// texture and block until the GPU has finished, so the frame is complete
    /// before Flutter is told it is ready. `display` must match this texture's
    /// dimensions (the caller recreates on a size change). Identical to the
    /// Windows and Linux siblings' `present`.
    pub fn present(&self, gpu: &GpuContext, display: &wgpu::Texture) {
        // The frame that produced `display` may still be sitting in the
        // batch; a copy of work that has not been submitted copies nothing.
        gpu.flush();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shared-iosurface-present"),
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
        // No fence yet: wait for the write to land so the reader never sees a
        // torn frame (see the module note). Zero *CPU* pixel work still — the
        // bytes never leave the card.
        gpu.device.poll(wgpu::Maintain::Wait);
    }
}

/// Create the IOSurface itself. The property keys are the documented literal
/// strings the framework's `kIOSurface*` constants are defined as, which spares
/// us four `extern` statics; a mistyped key would simply fail the create, which
/// this function reports.
fn create_surface(width: u32, height: u32) -> Result<IOSurfaceRef, String> {
    let props = CFDictionary::from_CFType_pairs(&[
        (
            CFString::from_static_string("IOSurfaceWidth"),
            CFNumber::from(i64::from(width)),
        ),
        (
            CFString::from_static_string("IOSurfaceHeight"),
            CFNumber::from(i64::from(height)),
        ),
        (
            CFString::from_static_string("IOSurfaceBytesPerElement"),
            CFNumber::from(4i64),
        ),
        (
            CFString::from_static_string("IOSurfacePixelFormat"),
            CFNumber::from(PIXEL_FORMAT_BGRA),
        ),
    ]);
    // The row stride is deliberately not specified: IOSurface computes an
    // aligned one, and every consumer reads it back off the surface.
    let surface = unsafe { IOSurfaceCreate(props.as_concrete_TypeRef()) };
    if surface.is_null() {
        return Err(format!(
            "iosurface texture: IOSurfaceCreate failed for {width}x{height}"
        ));
    }
    Ok(surface)
}

/// Ask Metal for a texture backed by `surface` and wrap it as a wgpu-hal Metal
/// texture. The returned hal texture owns the `MTLTexture`, which retains the
/// surface for as long as it lives.
///
/// # Safety
/// `device` must be a live `MTLDevice` and `surface` a live `IOSurfaceRef` whose
/// dimensions are `width`×`height`.
unsafe fn texture_from_surface(
    device: &metal::DeviceRef,
    surface: IOSurfaceRef,
    width: u32,
    height: u32,
) -> Result<wgpu::hal::metal::Texture, String> {
    let descriptor = metal::TextureDescriptor::new();
    descriptor.set_texture_type(metal::MTLTextureType::D2);
    descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
    descriptor.set_width(u64::from(width));
    descriptor.set_height(u64::from(height));
    descriptor.set_mipmap_level_count(1);
    descriptor.set_usage(metal::MTLTextureUsage::ShaderRead | metal::MTLTextureUsage::RenderTarget);
    // ponytail: `Managed` on a discrete-GPU Mac is written by us and read by
    // Flutter without an explicit `synchronizeResource` blit between the two —
    // correct on the unified-memory Macs this ships for, and the upgrade if a
    // dual-GPU Intel Mac ever shows a stale frame is a raw-Metal blit encoder
    // issuing that synchronise after the copy.
    descriptor.set_storage_mode(if device.has_unified_memory() {
        metal::MTLStorageMode::Shared
    } else {
        metal::MTLStorageMode::Managed
    });

    let raw: *mut metal::MTLTexture = msg_send![
        device,
        newTextureWithDescriptor: &*descriptor
        iosurface: surface
        plane: 0u64
    ];
    if raw.is_null() {
        return Err("iosurface texture: newTextureWithDescriptor:iosurface: returned nil".into());
    }
    // `new…` hands back a +1 reference, which `Texture::from_ptr` adopts.
    let texture = metal::Texture::from_ptr(raw);

    Ok(wgpu::hal::metal::Device::texture_from_raw(
        texture,
        SHARED_FORMAT,
        metal::MTLTextureType::D2,
        1,
        1,
        wgpu::hal::CopyExtent {
            width,
            height,
            depth: 1,
        },
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    extern "C" {
        fn IOSurfaceLock(buffer: IOSurfaceRef, options: u32, seed: *mut u32) -> i32;
        fn IOSurfaceUnlock(buffer: IOSurfaceRef, options: u32, seed: *mut u32) -> i32;
        fn IOSurfaceGetBaseAddress(buffer: IOSurfaceRef) -> *mut u8;
        fn IOSurfaceGetBytesPerRow(buffer: IOSurfaceRef) -> usize;
    }
    /// `kIOSurfaceLockReadOnly`.
    const LOCK_READ_ONLY: u32 = 0x0000_0001;

    /// The whole point of the hand-off, proven end to end: bytes written through
    /// the wgpu texture come back out of the **IOSurface**, read the way the
    /// runner's `CVPixelBuffer` reads it — including the channel order, which is
    /// the mistake that costs a whole session of blank Viewer if it is wrong
    /// (the Windows sibling's test exists for exactly that reason).
    #[test]
    fn the_surface_yields_the_pixels_in_bgra_order() {
        let Ok(gpu) = GpuContext::headless() else {
            eprintln!("skipping: no Metal adapter");
            return;
        };
        // A `Managed` texture on a discrete-GPU Mac needs an explicit
        // synchronise before the CPU can read it (see the descriptor's ponytail
        // note), which this path does not do — so only assert where the read is
        // meaningful.
        let unified = unsafe {
            gpu.device.as_hal::<wgpu::hal::api::Metal, _, _>(|d| {
                d.is_some_and(|d| d.raw_device().lock().has_unified_memory())
            })
        };
        if !unified {
            eprintln!("skipping: not a unified-memory Mac");
            return;
        }
        let shared = match SharedIoSurface::new(&gpu, 4, 2) {
            Ok(shared) => shared,
            Err(err) => {
                eprintln!("skipping: {err}");
                return;
            }
        };

        // Orange, asymmetric so a channel-order mistake cannot sneak past, laid
        // out the way the texture stores it: B, G, R, A.
        let pixel = [0x10u8, 0x80, 0xF0, 0xFF];
        let row: Vec<u8> = pixel.iter().copied().cycle().take(4 * 4).collect();
        let mut bytes = row.clone();
        bytes.extend_from_slice(&row);
        gpu.queue.write_texture(
            shared.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: 4,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
        gpu.submit([]);
        gpu.device.poll(wgpu::Maintain::Wait);

        // Read it back as the consumer does: straight off the surface.
        let mut seed = 0u32;
        assert_eq!(
            unsafe { IOSurfaceLock(shared.surface, LOCK_READ_ONLY, &mut seed) },
            0,
            "the surface must lock for reading"
        );
        let base = unsafe { IOSurfaceGetBaseAddress(shared.surface) };
        let stride = unsafe { IOSurfaceGetBytesPerRow(shared.surface) };
        let first = unsafe { std::slice::from_raw_parts(base, 4) };
        let second_row = unsafe { std::slice::from_raw_parts(base.add(stride), 4) };
        assert_eq!(
            first,
            &pixel[..],
            "row 0 must be the bytes we wrote, in order"
        );
        assert_eq!(
            second_row,
            &pixel[..],
            "every row must land, stride respected"
        );
        unsafe { IOSurfaceUnlock(shared.surface, LOCK_READ_ONLY, &mut seed) };

        assert_ne!(shared.handle(), 0, "the surface must have a lookup id");
    }
}
