//! The zero-copy Viewer target: a GPU texture Flutter samples directly.
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
//! mutex is the robust fix for that half and stays the follow-up: it
//! cannot land here alone, because a keyed-mutex texture must be acquired and
//! released by the *consumer* too, and ANGLE's legacy share-handle path does not
//! do that.
//!
//! The reference for the embedder-side plumbing (descriptor shape, the DXGI
//! shared-handle surface type, the texture-registrar dance) is the MIT-licensed
//! `flutter_wgpu_texture` package; we borrow the *pattern*, not the code.
//!
//! # Through memory
//!
//! Two things can break the hop on the card, and both may scramble the picture
//! rather than fail: Flutter on one card and wgpu on another (a two-card
//! laptop), and a driver that reads the D3D12 texture wrongly through D3D11
//! (suspected on Polaris under Windows 10). So each new texture first sends a test picture
//! through and reads it back the way ANGLE opens it. If the cards differ or the
//! picture comes back wrong, the Viewer goes through memory for the rest of the
//! run: wgpu reads each frame back and D3D11 uploads it on Flutter's card.
//! Slower, but right, and the reason is written to the diagnostics file.

#![allow(unsafe_code)]

use crate::GpuContext;
use std::sync::OnceLock;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, GENERIC_ALL, HANDLE, LUID};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11DeviceContext, ID3D11Query,
    ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_QUERY_DESC, D3D11_QUERY_EVENT, D3D11_RESOURCE_MISC_SHARED, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
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
    /// How a finished frame gets from wgpu into `d3d11_shared`.
    hop: Hop,
    /// The *legacy* share handle of `d3d11_shared`, the one Flutter opens.
    /// Legacy handles are identifiers rather than kernel handles and must NOT be
    /// closed.
    legacy_handle: isize,
    /// The D3D11 device and context on Flutter's card that do the per-frame hop.
    d3d11_context: ID3D11DeviceContext,
    /// The legacy-shared D3D11 texture Flutter samples.
    d3d11_shared: ID3D11Texture2D,
    /// An event query, reused every frame, that reports when the per-frame
    /// D3D11 copy has actually *finished* on the GPU rather than merely been
    /// submitted. See the module note on synchronisation.
    copy_done: ID3D11Query,
    /// The device the two textures above are on.
    d3d11_device: ID3D11Device,
    pub width: u32,
    pub height: u32,
}

// The handle is an opaque OS resource identifier, not a live pointer we
// dereference; keeping it as an `isize` next to a `Send`/`Sync` `wgpu::Texture`
// makes the whole struct safely shareable across the render lock.
unsafe impl Send for SharedTexture {}
unsafe impl Sync for SharedTexture {}

/// The two ways a finished frame reaches the texture Flutter samples.
enum Hop {
    /// On the card: wgpu copies into a shared D3D12 texture, and D3D11 copies
    /// that into the legacy one. The pixels never leave the card.
    Card {
        /// The copy destination the render path writes the finished frame into.
        texture: wgpu::Texture,
        /// The NT handle for the D3D12 resource (`HANDLE.0 as isize`). Held
        /// only to be closed on drop — Flutter never sees it.
        nt_handle: isize,
        /// The D3D12 resource as D3D11 sees it (opened from the NT handle).
        view: ID3D11Texture2D,
    },
    /// Through memory: wgpu reads the frame back into `buffer`, rows
    /// `row_bytes` apart, and D3D11 uploads it. See the module note.
    Memory {
        buffer: wgpu::Buffer,
        row_bytes: u32,
    },
}

/// Why the Viewer goes through memory, once it does. For the rest of the run:
/// a card or driver that got one texture wrong is not trusted with the next.
static THROUGH_MEMORY: OnceLock<String> = OnceLock::new();

/// Where that reason is written. The bridge sets it, so the line reaches the
/// diagnostics file a bug report is asked for.
static REPORT: OnceLock<fn(&str)> = OnceLock::new();

/// Send the reason the Viewer goes through memory to `report`, when it does.
pub fn report_to(report: fn(&str)) {
    let _ = REPORT.set(report);
}

/// The card Flutter draws with, by name, when the runner said which.
pub fn flutter_adapter() -> Option<String> {
    flutter_luid().map(|luid| unsafe { adapter_name(luid) })
}

/// Rows of the test picture: every tile across the full width, which is where
/// a layout or a compression the reader gets wrong shows, at almost no cost.
const PROBE_ROWS: u32 = 64;

impl SharedTexture {
    /// Create a `width`×`height` shared texture for the Viewer, on the card
    /// when that works and through memory when it does not (see the module
    /// note). `Err` only when neither can be made.
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Result<Self, String> {
        let width = width.max(1);
        let height = height.max(1);
        let drawn_on = wgpu_luid(gpu);
        let shown_on = flutter_luid();
        let target = shown_on.or(drawn_on).ok_or_else(|| {
            "shared texture: wgpu is not running on the D3D12 backend".to_string()
        })?;

        if THROUGH_MEMORY.get().is_none() {
            let why = match (shown_on, drawn_on) {
                (Some(shown), Some(drawn)) if !same_luid(shown, drawn) => unsafe {
                    format!(
                        "Flutter shows it on {} and the engine draws it on {}",
                        adapter_name(shown),
                        adapter_name(drawn),
                    )
                },
                _ => match Self::on_card(gpu, width, height) {
                    Ok(card) => match unsafe { card.probe(gpu) } {
                        Ok(()) => return Ok(card),
                        Err(why) => why,
                    },
                    Err(why) => why,
                },
            };
            go_through_memory(why);
        }
        Self::through_memory(gpu, target, width, height)
    }

    /// The hop on the card, described on [`SharedTexture`].
    fn on_card(gpu: &GpuContext, width: u32, height: u32) -> Result<Self, String> {
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
        let (d3d11_device, d3d11_context, view, d3d11_shared, copy_done, legacy_handle) = match hop
        {
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
            hop: Hop::Card {
                texture,
                nt_handle: handle,
                view,
            },
            legacy_handle,
            d3d11_context,
            d3d11_shared,
            copy_done,
            d3d11_device,
            width,
            height,
        })
    }

    /// The hop through memory, onto the card named by `luid` (Flutter's).
    fn through_memory(
        gpu: &GpuContext,
        luid: LUID,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let (device, context) = unsafe { d3d11_device(luid) }?;
        let (d3d11_shared, legacy_handle, copy_done) =
            unsafe { legacy_target(&device, width, height) }?;
        let row_bytes = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lumit-shared-readback"),
            size: u64::from(row_bytes) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            hop: Hop::Memory { buffer, row_bytes },
            legacy_handle,
            d3d11_context: context,
            d3d11_shared,
            copy_done,
            d3d11_device: device,
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
        let extent = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shared-present"),
            });
        match &self.hop {
            Hop::Card { texture, .. } => encoder.copy_texture_to_texture(
                display.as_image_copy(),
                texture.as_image_copy(),
                extent,
            ),
            Hop::Memory { buffer, row_bytes } => encoder.copy_texture_to_buffer(
                display.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(*row_bytes),
                        rows_per_image: Some(self.height),
                    },
                },
                extent,
            ),
        }
        gpu.submit([encoder.finish()]);

        match &self.hop {
            Hop::Card { view, .. } => {
                // Wait for the D3D12 write to land before D3D11 reads it below:
                // `submit` only queues the copy (see the module note on
                // synchronisation). Zero *CPU* pixel work still — the bytes
                // never leave the card.
                gpu.device.poll(wgpu::Maintain::Wait);
                // D3D12 has finished writing the simultaneous-access resource,
                // so copy it into the legacy-shared texture ANGLE samples,
                // still on the GPU.
                unsafe { self.d3d11_context.CopyResource(&self.d3d11_shared, view) };
            }
            Hop::Memory { buffer, row_bytes } => {
                let slice = buffer.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |r| {
                    let _ = tx.send(r);
                });
                gpu.device.poll(wgpu::Maintain::Wait);
                if !matches!(rx.recv(), Ok(Ok(()))) {
                    note!("lumit-gpu: shared texture: the frame could not be read back; showing the last one");
                    return;
                }
                let data = slice.get_mapped_range();
                unsafe {
                    self.d3d11_context.UpdateSubresource(
                        &self.d3d11_shared,
                        0,
                        None,
                        data.as_ptr().cast(),
                        *row_bytes,
                        0,
                    );
                }
                drop(data);
                buffer.unmap();
            }
        }
        unsafe { self.finish_hop() };
    }

    /// Wait for the D3D11 write into `d3d11_shared` to finish. `End` marks the
    /// point the event query reports on; `Flush` submits the write and the
    /// query together; the wait turns "submitted" into "finished", so a reader
    /// opening the legacy handle on its own device cannot beat it to the pixels.
    unsafe fn finish_hop(&self) {
        self.d3d11_context.End(&self.copy_done);
        self.d3d11_context.Flush();
        wait_for_copy(&self.d3d11_context, &self.copy_done);
    }

    /// Send a test picture through the hop on the card and read back what
    /// landed in the texture Flutter samples. `Err` says why the picture can't
    /// be trusted. Read on the hop's own device, since making another costs a
    /// tenth of a second or more and this runs on every new Viewer size.
    ///
    /// # Safety
    /// Calls D3D11 on the textures this owns.
    unsafe fn probe(&self, gpu: &GpuContext) -> Result<(), String> {
        let Hop::Card { texture, view, .. } = &self.hop else {
            return Ok(());
        };
        let rows = self.height.min(PROBE_ROWS);
        let picture: Vec<u8> = (0..rows)
            .flat_map(|y| (0..self.width).flat_map(move |x| probe_pixel(x, y)))
            .collect();
        gpu.queue.write_texture(
            texture.as_image_copy(),
            &picture,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: self.width,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([]);
        gpu.device.poll(wgpu::Maintain::Wait);
        self.d3d11_context.CopyResource(&self.d3d11_shared, view);
        self.finish_hop();

        let read = read_rows(
            &self.d3d11_device,
            &self.d3d11_context,
            &self.d3d11_shared,
            self.width,
            rows,
        )?;
        let row = self.width as usize * 4;
        let wrong = read
            .chunks(row)
            .zip(picture.chunks(row))
            .filter(|(got, want)| got != want)
            .count();
        if wrong == 0 {
            Ok(())
        } else {
            Err(format!(
                "a test picture came back wrong through the card on {wrong} of {rows} rows ({}x{})",
                self.width, self.height
            ))
        }
    }
}

/// The first `rows` rows of `texture` as tight BGRA bytes.
///
/// # Safety
/// `texture` must be a `width`-wide BGRA texture on `device`.
unsafe fn read_rows(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    width: u32,
    rows: u32,
) -> Result<Vec<u8>, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: rows,
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
        .map_err(|e| format!("shared texture: staging CreateTexture2D failed: {e}"))?;
    let staging = staging.ok_or("shared texture: staging texture is null")?;
    let area = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: width,
        bottom: rows,
        back: 1,
    };
    context.CopySubresourceRegion(&staging, 0, 0, 0, 0, texture, 0, Some(&area));
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context
        .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        .map_err(|e| format!("shared texture: staging Map failed: {e}"))?;
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * rows as usize);
    for y in 0..rows as usize {
        let src = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
        out.extend_from_slice(core::slice::from_raw_parts(src, row));
    }
    context.Unmap(&staging, 0);
    Ok(out)
}

/// One pixel of the test picture, in BGRA bytes. Flat, graded and noisy 8×8
/// cells side by side, because a compression the reader gets wrong spoils the
/// cells that compress and a tiling it gets wrong spoils the detailed ones.
fn probe_pixel(x: u32, y: u32) -> [u8; 4] {
    let cell = (x / 8) ^ (y / 8);
    match cell % 3 {
        0 => [cell as u8, 0x80, 0xff, 0xff],
        1 => [(x * 32) as u8, (y * 32) as u8, 0x40, 0xff],
        _ => (x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B))
            .rotate_left(13)
            .wrapping_mul(0xC2B2_AE35)
            .to_le_bytes(),
    }
}

/// From now on the Viewer goes through memory, and the reason is said once.
fn go_through_memory(why: String) {
    let line = format!("the Viewer goes through memory: {why}");
    note!("lumit-gpu: {line}");
    if let Some(report) = REPORT.get() {
        report(&line);
    }
    let _ = THROUGH_MEMORY.set(line);
}

/// The card wgpu draws on, or `None` off the D3D12 backend.
fn wgpu_luid(gpu: &GpuContext) -> Option<LUID> {
    unsafe {
        gpu.device
            .as_hal::<wgpu::hal::api::Dx12, _, _>(|d| d.map(|d| d.raw_device().GetAdapterLuid()))
    }
}

/// The card Flutter draws with, as the runner put it in the environment once
/// its engine was up: the LUID as 16 hex digits, high part first.
fn flutter_luid() -> Option<LUID> {
    let raw = std::env::var("LUMIT_FLUTTER_ADAPTER").ok()?;
    let value = u64::from_str_radix(raw.trim(), 16).ok()?;
    Some(LUID {
        LowPart: value as u32,
        HighPart: (value >> 32) as i32,
    })
}

fn same_luid(a: LUID, b: LUID) -> bool {
    a.LowPart == b.LowPart && a.HighPart == b.HighPart
}

/// A card's name for the diagnostics file, or why it has none.
///
/// # Safety
/// Calls DXGI.
unsafe fn adapter_name(luid: LUID) -> String {
    find_adapter(luid).map_or_else(|err| err, |(_, name)| name)
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
                note!("lumit-gpu: shared texture: GetData failed while awaiting the frame copy: {err}");
                return;
            }
            Ok(()) => {}
        }
        if start.elapsed() >= COPY_WAIT_LIMIT {
            note!(
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
        if let Hop::Card { nt_handle, .. } = self.hop {
            if nt_handle != 0 {
                let _ = unsafe { CloseHandle(HANDLE(nt_handle as *mut core::ffi::c_void)) };
            }
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
    luid: LUID,
    nt_handle: isize,
    width: u32,
    height: u32,
) -> Result<D3d11Hop, String> {
    let (device, context) = d3d11_device(luid)?;

    // The D3D12 resource, as D3D11 sees it. `OpenSharedResource1` is the NT-handle
    // open; it works across the API boundary because the resource was created
    // with ALLOW_SIMULTANEOUS_ACCESS.
    let device1: ID3D11Device1 = device
        .cast()
        .map_err(|e| format!("shared texture: no ID3D11Device1: {e}"))?;
    let view: ID3D11Texture2D = device1
        .OpenSharedResource1(HANDLE(nt_handle as *mut core::ffi::c_void))
        .map_err(|e| format!("shared texture: OpenSharedResource1 failed: {e}"))?;

    let (shared, legacy, query) = legacy_target(&device, width, height)?;
    Ok((device, context, view, shared, query, legacy))
}

/// The DXGI adapter with this LUID, and its name.
///
/// # Safety
/// Calls DXGI.
unsafe fn find_adapter(luid: LUID) -> Result<(IDXGIAdapter, String), String> {
    let factory: IDXGIFactory1 = CreateDXGIFactory1()
        .map_err(|e| format!("shared texture: CreateDXGIFactory1 failed: {e}"))?;
    for index in 0..16 {
        let Ok(candidate) = factory.EnumAdapters(index) else {
            break;
        };
        let desc = candidate
            .GetDesc()
            .map_err(|e| format!("shared texture: GetDesc failed: {e}"))?;
        if same_luid(desc.AdapterLuid, luid) {
            let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
            let name = String::from_utf16_lossy(&desc.Description[..len]);
            return Ok((candidate, name));
        }
    }
    Err("shared texture: no DXGI adapter matches the LUID".to_string())
}

/// A D3D11 device and context on the adapter named by `luid`. Sharing does not
/// cross cards, so this is always the card the texture is shared on.
///
/// # Safety
/// Calls DXGI and D3D11.
unsafe fn d3d11_device(luid: LUID) -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    let (adapter, _) = find_adapter(luid)?;
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
    Ok((device, context))
}

/// The texture Flutter samples, its legacy share handle, and the event query
/// the per-frame wait reuses.
///
/// # Safety
/// Calls D3D11 on `device`.
unsafe fn legacy_target(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(ID3D11Texture2D, isize, ID3D11Query), String> {
    // Legacy MISC_SHARED, which is the only kind ANGLE's share-handle path can
    // open.
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

    Ok((shared, legacy.0 as isize, query))
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

    /// The whole point of the hop, proven end to end for both ways a frame can
    /// take: pixels written through wgpu come back out of the **legacy** share
    /// handle, opened on a separate D3D11 device exactly as ANGLE opens it
    /// inside Flutter. And the test picture passes on a card that is fine, or
    /// every Viewer would go through memory.
    ///
    /// This is the test that was missing when the NT handle shipped: everything
    /// engine-side reported success while Flutter composited a transparent
    /// quad, because ANGLE's share-handle path cannot open an NT handle at all.
    /// The width is not a multiple of 64 pixels, so the read-back rows are
    /// padded and the memory hop has to step over the padding.
    #[test]
    fn the_legacy_handle_yields_the_pixels_angle_style() {
        let (w, h) = (300, 70);
        let Ok(gpu) = GpuContext::headless() else {
            eprintln!("skipping: no D3D12 adapter");
            return;
        };
        let Some(luid) = wgpu_luid(&gpu) else {
            eprintln!("skipping: not on D3D12");
            return;
        };

        // Orange, written like a rendered frame — an asymmetric colour, so a
        // channel-order mistake cannot sneak past the way a symmetric one
        // (magenta: R = B) would. In BGRA bytes, RGBA orange (255, 128, 0) is
        // [0, 128, 255].
        let orange: Vec<u8> = [0u8, 128, 255, 255].repeat((w * h) as usize);
        let extent = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let display = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-display"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHARED_FORMAT,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            display.as_image_copy(),
            &orange,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            extent,
        );

        for made in [
            SharedTexture::on_card(&gpu, w, h),
            SharedTexture::through_memory(&gpu, luid, w, h),
        ] {
            let shared = match made {
                Ok(shared) => shared,
                Err(err) => {
                    eprintln!("skipping: {err}");
                    return;
                }
            };
            unsafe { shared.probe(&gpu) }.unwrap();
            shared.present(&gpu, &display);
            // ANGLE's side: a device of its own and the legacy open.
            let bgra = unsafe {
                let (device, context) = d3d11_device(luid).unwrap();
                let mut opened: Option<ID3D11Texture2D> = None;
                device
                    .OpenSharedResource(
                        HANDLE(shared.legacy_handle as *mut core::ffi::c_void),
                        &mut opened,
                    )
                    .unwrap();
                read_rows(&device, &context, &opened.unwrap(), w, h).unwrap()
            };
            assert_eq!(
                &bgra[0..4],
                &[0, 128, 255, 255],
                "the pixels reached the legacy-shared texture, in BGRA order"
            );
            assert!(
                bgra.chunks(4).all(|px| px == [0, 128, 255, 255]),
                "every pixel, not just the corner"
            );
        }
    }
}
