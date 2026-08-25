//! The zero-copy Viewer target: a GPU texture Flutter samples directly (K-177).
//!
//! # In plain terms
//!
//! Normally the Viewer's picture makes a slow round trip every frame: the engine
//! draws it on the graphics card, copies it *down* into ordinary memory, hands
//! the bytes across to Flutter, and Flutter uploads them *back* onto the card to
//! show them. This module removes that round trip on Windows. The engine draws
//! into a special texture that is *shareable*: Windows can hand the same piece of
//! graphics memory to another part of the program by name (an "NT handle").
//! Flutter's Windows layer opens that handle and shows the texture on screen
//! without any copy — the picture never leaves the graphics card.
//!
//! # How it works, precisely
//!
//! wgpu runs over Direct3D 12 on Windows. We reach *through* wgpu to its D3D12
//! device (`Device::as_hal`), create a D3D12 texture in a **shared heap**
//! (`D3D12_HEAP_FLAG_SHARED`), and export an NT handle for it
//! (`ID3D12Device::CreateSharedHandle`). We then wrap that same D3D12 resource
//! back up as a `wgpu::Texture` (`create_texture_from_hal`) so the normal render
//! path can copy the finished, display-encoded frame into it. The handle is what
//! Flutter's embedder opens as a `kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle`
//! surface (it re-opens the shared resource on its own D3D11/ANGLE device).
//!
//! The texture is `DXGI_FORMAT_B8G8R8A8_UNORM` and holds the *already sRGB-encoded*
//! display bytes — byte-for-byte the same pixels the CPU read-back path produced,
//! so Flutter shows them identically (it treats the texture as plain RGBA8888).
//! We copy the engine's `Rgba8UnormSrgb` display texture into this `Rgba8Unorm`
//! one; wgpu allows that copy because the two formats differ only in sRGB-ness
//! (a verbatim byte copy, no re-encode).
//!
//! # Synchronisation
//!
//! Two waits, one per hop, both inside [`SharedTexture::present`], because
//! *submitted* is not *finished*: both `Queue::submit` and D3D11's `Flush` hand
//! work to the driver and return immediately, while the GPU is still executing.
//!
//! 1. `poll(Wait)` after the wgpu copy, so D3D12 has finished writing the shared
//!    resource before D3D11 reads it.
//! 2. A D3D11 **event query** after the D3D11 `CopyResource`, so that copy has
//!    finished before we tell anyone the frame is ready. Without it a reader on
//!    another device — Flutter's ANGLE device, or the test's — can win the race
//!    and sample the previous (or cleared) contents.
//!
//! We render into the *same* texture each frame, so there is still a theoretical
//! race if Flutter is mid-sample when the *next* frame's copy begins. A keyed
//! mutex is the robust fix for that half and stays the follow-up (K-177): it
//! cannot land here alone, because a keyed-mutex texture must be acquired and
//! released by the *consumer* too, and ANGLE's legacy share-handle path does not
//! do that.
//!
//! The reference for the embedder-side plumbing (descriptor shape, the DXGI
//! shared-handle surface type, the texture-registrar dance) is the MIT-licensed
//! `flutter_wgpu_texture` package; we borrow the *pattern*, not the code.

#![allow(unsafe_code)]

use crate::GpuContext;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, GENERIC_ALL, HANDLE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11DeviceContext, ID3D11Query,
    ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CREATE_DEVICE_FLAG, D3D11_QUERY_DESC, D3D11_QUERY_EVENT, D3D11_RESOURCE_MISC_SHARED,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12Device, ID3D12Resource, D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_PROPERTIES,
    D3D12_HEAP_TYPE_DEFAULT, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
    D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
    D3D12_RESOURCE_STATE_COMMON, D3D12_TEXTURE_LAYOUT_UNKNOWN,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIResource,
};

/// The wgpu-side format of the shared texture. **BGRA**, because the consumer
/// dictates it: ANGLE (inside Flutter's embedder) matches share-handle surfaces
/// against its own B8G8R8A8 configs, and an RGBA texture fails that match — not
/// with an error, but with a surface that never opens and a Viewer that shows
/// its checkerboard. Non-`Srgb` so the display-encoded bytes are stored
/// verbatim; the copy in [`SharedTexture::present`] is legal because only
/// srgb-ness differs from the BGRA display texture, which wgpu treats as
/// copy-compatible.
const SHARED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// A D3D12 texture in a shared heap, wrapped as a `wgpu::Texture`, plus the
/// D3D11 hop that turns it into something Flutter can actually open.
///
/// **Why the hop exists.** Flutter's `DxgiSharedHandle` external texture goes
/// through ANGLE's `EGL_D3D_TEXTURE_2D_SHARE_HANDLE_ANGLE`, which takes a
/// *legacy* DXGI share handle — the `IDXGIResource::GetSharedHandle` kind (the
/// embedder header's own doc link says exactly this). An earlier version handed
/// it the *NT* handle from `ID3D12Device::CreateSharedHandle` instead: ANGLE
/// cannot open one of those, fails without a word, and composites a transparent
/// quad — a Viewer showing its checkerboard while every counter says frames are
/// flowing. D3D12 cannot create legacy handles at all, so the engine bridges:
/// the D3D12 texture is opened on a same-adapter D3D11 device via its NT handle,
/// and each frame is GPU-copied into a D3D11 texture created with legacy
/// `MISC_SHARED` sharing, whose legacy handle is what Flutter gets. The pixels
/// never leave the card; the price is one extra on-GPU copy.
///
/// One is held for the whole Viewer session and re-created only when the comp's
/// dimensions change (a new handle is reported then). The `wgpu::Texture` keeps
/// the D3D12 resource alive; the COM references keep the D3D11 side alive.
pub struct SharedTexture {
    /// The copy destination the render path writes the finished frame into.
    pub texture: wgpu::Texture,
    /// The NT handle for the D3D12 resource (`HANDLE.0 as isize`). Held only to
    /// be closed on drop — Flutter never sees it.
    nt_handle: isize,
    /// The *legacy* share handle of `d3d11_shared`, the one Flutter opens.
    /// Legacy handles are identifiers rather than kernel handles and must NOT be
    /// closed.
    legacy_handle: isize,
    /// The same-adapter D3D11 device and context that perform the per-frame hop.
    d3d11_context: ID3D11DeviceContext,
    /// The D3D12 resource as D3D11 sees it (opened from the NT handle).
    d3d11_view_of_d3d12: ID3D11Texture2D,
    /// The legacy-shared D3D11 texture Flutter samples.
    d3d11_shared: ID3D11Texture2D,
    /// An event query, reused every frame, that reports when the per-frame
    /// D3D11 copy has actually *finished* on the GPU rather than merely been
    /// submitted. See the module note on synchronisation.
    copy_done: ID3D11Query,
    /// Kept alive for the two textures above.
    _d3d11_device: ID3D11Device,
    pub width: u32,
    pub height: u32,
}

// The handle is an opaque OS resource identifier, not a live pointer we
// dereference; keeping it as an `isize` next to a `Send`/`Sync` `wgpu::Texture`
// makes the whole struct safely shareable across the render lock.
unsafe impl Send for SharedTexture {}
unsafe impl Sync for SharedTexture {}

impl SharedTexture {
    /// Create a `width`×`height` shared texture on `gpu`'s D3D12 device. `Err`
    /// when wgpu is not on the D3D12 backend (the shared path needs D3D12; the
    /// caller falls back to the read-back path) or any D3D12 call fails.
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Result<Self, String> {
        let width = width.max(1);
        let height = height.max(1);

        // Reach through wgpu to the raw D3D12 device, create the shared resource
        // there, and export its NT handle — all while wgpu holds the device.
        let created = unsafe {
            gpu.device
                .as_hal::<wgpu::hal::api::Dx12, _, _>(|hal_device| {
                    let hal_device = hal_device.ok_or_else(|| {
                        "shared texture: wgpu is not running on the D3D12 backend".to_string()
                    })?;
                    let device = hal_device.raw_device();
                    let luid = device.GetAdapterLuid();
                    create_shared_resource(device, width, height).map(|pair| (pair, luid))
                })
        };
        let ((resource, handle), adapter_luid) = created?;

        // The D3D11 hop — see the struct docs for why it must exist at all.
        // Same adapter by LUID, or the NT open below fails: a handle shared
        // from one GPU cannot be opened on another.
        let hop = unsafe { create_d3d11_hop(adapter_luid, handle, width, height) };
        let (
            d3d11_device,
            d3d11_context,
            d3d11_view_of_d3d12,
            d3d11_shared,
            copy_done,
            legacy_handle,
        ) = match hop {
            Ok(parts) => parts,
            Err(err) => {
                // Nothing D3D11 was kept; release the D3D12 side too.
                unsafe {
                    let _ = CloseHandle(HANDLE(handle as *mut core::ffi::c_void));
                }
                drop(resource);
                return Err(err);
            }
        };

        // Wrap the very same D3D12 resource as a wgpu texture so the render path
        // can copy into it. `texture_from_raw` takes a clone (a COM ref-count
        // bump); that clone, held by the returned `wgpu::Texture`, is what keeps
        // the resource — and therefore the exported handle — alive.
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let hal_texture = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                SHARED_FORMAT,
                wgpu::TextureDimension::D2,
                extent,
                1,
                1,
            )
        };
        let texture = unsafe {
            gpu.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("lumit-shared-target"),
                    size: extent,
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
            nt_handle: handle,
            legacy_handle,
            d3d11_context,
            d3d11_view_of_d3d12,
            d3d11_shared,
            copy_done,
            _d3d11_device: d3d11_device,
            width,
            height,
        })
    }

    /// The handle Flutter opens (`kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle`) —
    /// the *legacy* DXGI share handle of the D3D11 texture, which is the kind
    /// that surface type actually takes.
    pub fn handle(&self) -> u64 {
        self.legacy_handle as usize as u64
    }

    /// Copy the finished display texture (`Rgba8UnormSrgb`) into the shared
    /// texture and block until the GPU has finished, so the frame is complete
    /// before Flutter is told it is ready. `display` must match the shared
    /// texture's dimensions (the caller recreates on a size change).
    pub fn present(&self, gpu: &GpuContext, display: &wgpu::Texture) {
        // The frame that produced `display` may still be sitting in the
        // batch; a copy of work that has not been submitted copies nothing.
        gpu.flush();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shared-present"),
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
        // Wait for the D3D12 write to land before D3D11 reads it below: `submit`
        // only queues the copy (see the module note on synchronisation). Zero
        // *CPU* pixel work still — the bytes never leave the card.
        gpu.device.poll(wgpu::Maintain::Wait);

        // The hop: D3D12 has finished writing the simultaneous-access resource,
        // so copy it into the legacy-shared texture ANGLE samples, still on the
        // GPU. `End` marks the point the event query reports on; `Flush` submits
        // the copy and the query together; the wait turns "submitted" into
        // "finished", so a reader opening the legacy handle on its own device
        // cannot beat the copy to the pixels.
        unsafe {
            self.d3d11_context
                .CopyResource(&self.d3d11_shared, &self.d3d11_view_of_d3d12);
            self.d3d11_context.End(&self.copy_done);
            self.d3d11_context.Flush();
            wait_for_copy(&self.d3d11_context, &self.copy_done);
        }
    }
}

/// How long [`wait_for_copy`] waits before giving up. A frame copy of a few
/// megabytes finishes in microseconds; anything near this bound means the device
/// is wedged, and hanging the render thread forever would be worse than showing
/// one stale frame.
const COPY_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_millis(250);

/// Block until the GPU reports `query`'s work complete, or [`COPY_WAIT_LIMIT`]
/// passes. Never panics: a lost device or an expired bound simply returns, and
/// the caller presents whatever the texture holds.
///
/// # Safety
/// `query` must have been created on the same device as `context`, and `End`
/// must already have been called on it.
unsafe fn wait_for_copy(context: &ID3D11DeviceContext, query: &ID3D11Query) {
    let start = std::time::Instant::now();
    loop {
        // An event query's payload is a single BOOL. When the work is still in
        // flight the call returns S_FALSE and writes nothing — a *success*
        // HRESULT, so `Result` cannot tell us apart from S_OK. Starting at
        // false and reading the value back is what distinguishes them.
        let mut done = BOOL(0);
        let status = context.GetData(
            query,
            Some(std::ptr::from_mut(&mut done).cast()),
            core::mem::size_of::<BOOL>() as u32,
            0,
        );
        match status {
            Ok(()) if done.as_bool() => return,
            // Device removed or reset: no amount of waiting will finish this.
            Err(err) => {
                eprintln!("lumit-gpu: shared texture: GetData failed while awaiting the frame copy: {err}");
                return;
            }
            Ok(()) => {}
        }
        if start.elapsed() >= COPY_WAIT_LIMIT {
            eprintln!(
                "lumit-gpu: shared texture: the GPU did not report the frame copy finished within {COPY_WAIT_LIMIT:?}; showing the frame anyway"
            );
            return;
        }
        // Spin hot for the first millisecond, which is where every healthy wait
        // ends, then stop burning the core.
        if start.elapsed() < std::time::Duration::from_millis(1) {
            std::thread::yield_now();
        } else {
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
    }
}

impl Drop for SharedTexture {
    fn drop(&mut self) {
        // Release the NT handle we exported. The D3D12 resource itself is freed
        // by the `wgpu::Texture` dropping its COM reference, and the D3D11 side
        // by its COM references. The legacy handle is an identifier, not a
        // kernel handle — closing it would be an error.
        if self.nt_handle != 0 {
            let _ = unsafe { CloseHandle(HANDLE(self.nt_handle as *mut core::ffi::c_void)) };
        }
    }
}

/// Build the D3D11 side of the hop: a device on the adapter named by `luid`,
/// the D3D12 resource opened there through its NT handle, and a legacy-shared
/// texture whose share handle Flutter can open (see the struct docs).
///
/// # Safety
/// `nt_handle` must be a valid NT handle to a shareable D3D12 resource created
/// on the adapter named by `luid`.
type D3d11Hop = (
    ID3D11Device,
    ID3D11DeviceContext,
    ID3D11Texture2D,
    ID3D11Texture2D,
    ID3D11Query,
    isize,
);

unsafe fn create_d3d11_hop(
    luid: windows::Win32::Foundation::LUID,
    nt_handle: isize,
    width: u32,
    height: u32,
) -> Result<D3d11Hop, String> {
    // The adapter wgpu is on, found by LUID — sharing does not cross GPUs.
    let factory: IDXGIFactory1 = CreateDXGIFactory1()
        .map_err(|e| format!("shared texture: CreateDXGIFactory1 failed: {e}"))?;
    let mut adapter: Option<IDXGIAdapter> = None;
    for index in 0..16 {
        let Ok(candidate) = factory.EnumAdapters(index) else {
            break;
        };
        let desc = candidate
            .GetDesc()
            .map_err(|e| format!("shared texture: GetDesc failed: {e}"))?;
        if desc.AdapterLuid.LowPart == luid.LowPart && desc.AdapterLuid.HighPart == luid.HighPart {
            adapter = Some(candidate);
            break;
        }
    }
    let adapter =
        adapter.ok_or_else(|| "shared texture: no DXGI adapter matches wgpu's LUID".to_string())?;

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    D3D11CreateDevice(
        &adapter,
        // UNKNOWN is required when an explicit adapter is given.
        D3D_DRIVER_TYPE_UNKNOWN,
        None,
        D3D11_CREATE_DEVICE_FLAG(0),
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
    )
    .map_err(|e| format!("shared texture: D3D11CreateDevice failed: {e}"))?;
    let device = device.ok_or_else(|| "shared texture: D3D11 device is null".to_string())?;
    let context = context.ok_or_else(|| "shared texture: D3D11 context is null".to_string())?;

    // The D3D12 resource, as D3D11 sees it. `OpenSharedResource1` is the NT-handle
    // open; it works across the API boundary because the resource was created
    // with ALLOW_SIMULTANEOUS_ACCESS.
    let device1: ID3D11Device1 = device
        .cast()
        .map_err(|e| format!("shared texture: no ID3D11Device1: {e}"))?;
    let view: ID3D11Texture2D = device1
        .OpenSharedResource1(HANDLE(nt_handle as *mut core::ffi::c_void))
        .map_err(|e| format!("shared texture: OpenSharedResource1 failed: {e}"))?;

    // The texture Flutter samples: legacy MISC_SHARED, which is the only kind
    // ANGLE's share-handle path can open.
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
    };
    let mut shared: Option<ID3D11Texture2D> = None;
    device
        .CreateTexture2D(&desc, None, Some(&mut shared))
        .map_err(|e| format!("shared texture: CreateTexture2D failed: {e}"))?;
    let shared = shared.ok_or_else(|| "shared texture: D3D11 texture is null".to_string())?;

    let dxgi: IDXGIResource = shared
        .cast()
        .map_err(|e| format!("shared texture: no IDXGIResource: {e}"))?;
    let legacy = dxgi
        .GetSharedHandle()
        .map_err(|e| format!("shared texture: GetSharedHandle failed: {e}"))?;

    // One event query, made here and reused every frame, so `present` allocates
    // nothing per frame (docs/14 on budgeted allocations).
    let query_desc = D3D11_QUERY_DESC {
        Query: D3D11_QUERY_EVENT,
        MiscFlags: 0,
    };
    let mut query: Option<ID3D11Query> = None;
    device
        .CreateQuery(&query_desc, Some(&mut query))
        .map_err(|e| format!("shared texture: CreateQuery failed: {e}"))?;
    let query = query.ok_or_else(|| "shared texture: D3D11 query is null".to_string())?;

    Ok((device, context, view, shared, query, legacy.0 as isize))
}

/// Create a shared, simultaneous-access D3D12 texture and export its NT handle.
///
/// # Safety
/// `device` must be a valid `ID3D12Device`.
unsafe fn create_shared_resource(
    device: &ID3D12Device,
    width: u32,
    height: u32,
) -> Result<(ID3D12Resource, isize), String> {
    let heap_props = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        ..Default::default()
    };
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: u64::from(width),
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        // ALLOW_RENDER_TARGET keeps the format render-target-compatible (what a
        // display texture is); ALLOW_SIMULTANEOUS_ACCESS lets another device
        // (Flutter's) read it while it stays in the COMMON state, which is the
        // supported way to share a render target across APIs.
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
            | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
    };

    let mut resource: Option<ID3D12Resource> = None;
    device
        .CreateCommittedResource(
            &heap_props,
            D3D12_HEAP_FLAG_SHARED,
            &desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut resource,
        )
        .map_err(|e| format!("shared texture: CreateCommittedResource failed: {e}"))?;
    let resource = resource
        .ok_or_else(|| "shared texture: CreateCommittedResource returned null".to_string())?;

    let handle = device
        .CreateSharedHandle(&resource, None, GENERIC_ALL.0, PCWSTR::null())
        .map_err(|e| format!("shared texture: CreateSharedHandle failed: {e}"))?;

    Ok((resource, handle.0 as isize))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_USAGE_STAGING,
    };

    /// The whole point of the hop, proven end to end: pixels written through the
    /// wgpu texture come back out of the **legacy** share handle, opened on a
    /// separate D3D11 device exactly as ANGLE opens it inside Flutter.
    ///
    /// This is the test that was missing when the NT handle shipped: everything
    /// engine-side reported success while Flutter composited a transparent
    /// quad, because ANGLE's share-handle path cannot open an NT handle at all.
    /// A separate device making the legacy open is as close to ANGLE as a test
    /// can get without an EGL display.
    #[test]
    fn the_legacy_handle_yields_the_pixels_angle_style() {
        let Ok(gpu) = GpuContext::headless() else {
            eprintln!("skipping: no D3D12 adapter");
            return;
        };
        let shared = match SharedTexture::new(&gpu, 8, 4) {
            Ok(shared) => shared,
            Err(err) => {
                eprintln!("skipping: {err}");
                return;
            }
        };

        // Orange, written through the wgpu texture like a rendered frame — an
        // asymmetric colour, so a channel-order mistake cannot sneak past the
        // way a symmetric one (magenta: R = B) would. In BGRA bytes, RGBA
        // orange (255, 128, 0) is [0, 128, 255].
        let magenta: Vec<u8> = [0u8, 128, 255, 255].repeat(8 * 4);
        let display = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-display"),
            size: wgpu::Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHARED_FORMAT,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            display.as_image_copy(),
            &magenta,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8 * 4),
                rows_per_image: Some(4),
            },
            wgpu::Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
        shared.present(&gpu, &display);

        // ANGLE's side: a different D3D11 device, the LEGACY open, a staging
        // read-back.
        let rgba = unsafe { read_legacy_handle(shared.handle(), 8, 4) }.unwrap();
        assert_eq!(
            &rgba[0..4],
            &[0, 128, 255, 255],
            "the pixels reached the legacy-shared texture, in BGRA order"
        );
        assert!(
            rgba.chunks(4).all(|px| px == [0, 128, 255, 255]),
            "every pixel, not just the corner"
        );
    }

    /// Open `legacy` on a fresh device (as ANGLE would) and read the texture.
    unsafe fn read_legacy_handle(legacy: u64, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        D3D11CreateDevice(
            None,
            windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
        let device = device.unwrap();
        let context = context.unwrap();

        let mut opened: Option<ID3D11Texture2D> = None;
        device
            .OpenSharedResource(
                HANDLE(legacy as isize as *mut core::ffi::c_void),
                &mut opened,
            )
            .map_err(|e| format!("legacy OpenSharedResource: {e}"))?;
        let opened = opened.ok_or("legacy OpenSharedResource returned null")?;

        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging: Option<ID3D11Texture2D> = None;
        device
            .CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(|e| format!("staging CreateTexture2D: {e}"))?;
        let staging = staging.unwrap();

        context.CopyResource(&staging, &opened);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| format!("Map: {e}"))?;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let src = (mapped.pData as *const u8).add((row * mapped.RowPitch) as usize);
            out.extend_from_slice(core::slice::from_raw_parts(src, (width * 4) as usize));
        }
        context.Unmap(&staging, 0);
        Ok(out)
    }
}
