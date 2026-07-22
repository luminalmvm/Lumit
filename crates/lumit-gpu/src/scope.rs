//! The GPU scope pass (docs/07-UI-SPEC.md §8, K-096 v1) — the "scopes are super
//! laggy" fix.
//!
//! # In plain terms
//!
//! A scope plots the picture's brightness and colour: a waveform, a vectorscope,
//! a histogram. Both frontends used to compute those on the CPU — walking a
//! quarter-million pixels every frame — which is what made scrubbing with a
//! scope open feel laggy. This module does that walk on the graphics card
//! instead, so the trace costs almost nothing to produce.
//!
//! It is three small compute passes (see `scope.wgsl`):
//!
//! 1. **bin** — one GPU thread per sampled pixel drops that pixel into a
//!    counting bin with an atomic add (many threads count into the same bin
//!    safely). The bins are a plain storage buffer of `u32` counters.
//! 2. **peak** — folds the bins to their single tallest value, so the trace can
//!    be normalised (a scope's brightness is relative to its busiest line).
//! 3. **colourise** — paints the 256×256 trace texture from the bins, in the
//!    fixed scope colours the caller passes in.
//!
//! The maths mirrors `crates/lumit-ui/src/shell/scopes.rs` op-for-op (the CPU is
//! the oracle; the `#[cfg(test)]` oracles at the bottom hold the two to
//! agreement on bin counts and on trace pixels). The trace colours are passed in
//! as a uniform rather than baked into the shader, so the no-hex-outside-theme
//! rule (docs/15-DESIGN.md) still holds — the caller owns the palette.
//!
//! # Lavapipe / CI
//!
//! The CI GPU oracles run on Mesa's lavapipe (a software rasteriser). This pass
//! uses only core WGSL — `atomic<u32>` storage buffers and an `rgba8unorm`
//! storage texture, both mandatory features — so it runs there unchanged, exactly
//! as the effect kernels do. No subgroup ops, no optional limits.
//!
//! # Delivery
//!
//! The trace is a tiny 256×256 RGBA image (256 KiB). It is read back to the CPU
//! (a cheap readback of the *trace*, not the full frame) and handed to the
//! frontend, which uploads it as an ordinary image. This is deliberately *not*
//! delivered over a second shared texture: the win here is moving the binning off
//! the CPU, and the trace is so small that a zero-copy hand-off would save
//! nothing while costing a second registered texture and its VRAM. See
//! `docs/flutter-port/06-REMAINING-WORK.md` for the recorded reasoning.

use crate::{GpuContext, GpuError};

/// The trace-texture resolution (columns × value levels for waveforms, the
/// square grid for the vectorscope, bins × levels for the histogram). Matches
/// the CPU path's `GRID`.
pub const GRID: u32 = 256;

/// Cap on pixels sampled per trace — scopes degrade gracefully and a 256-wide
/// plot resolves far less than a 1080p frame. Matches the CPU path's
/// `MAX_SAMPLES`.
pub const MAX_SAMPLES: usize = 240_000;

/// The counts buffer holds the largest kind's needs: the RGB waveform's three
/// `GRID × GRID` grids. Every other kind uses a prefix of it.
const COUNTS_LEN: usize = 3 * (GRID as usize) * (GRID as usize);

/// Which scope one call renders. The `u32` tags match `scope.wgsl`'s `switch`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeKind {
    /// Brightness (luma) waveform.
    WaveformLuma,
    /// Red/green/blue waveforms overlaid.
    WaveformRgb,
    /// Chroma on a circle (Rec.601 Cb/Cr).
    Vectorscope,
    /// Per-channel pixel counts by brightness.
    Histogram,
}

impl ScopeKind {
    fn tag(self) -> u32 {
        match self {
            ScopeKind::WaveformLuma => 0,
            ScopeKind::WaveformRgb => 1,
            ScopeKind::Vectorscope => 2,
            ScopeKind::Histogram => 3,
        }
    }

    /// The populated length of the counts buffer for this kind — the range the
    /// peak reduction scans and the colourise pass reads.
    fn count_len(self) -> u32 {
        match self {
            ScopeKind::WaveformLuma | ScopeKind::Vectorscope => GRID * GRID,
            ScopeKind::WaveformRgb => 3 * GRID * GRID,
            ScopeKind::Histogram => 3 * GRID,
        }
    }
}

/// The fixed trace colours, as 0..255 RGB bytes. The caller (the frontend, over
/// the bridge) owns these — they are `theme.scope`'s `ScopeColours::STANDARD`,
/// kept out of the engine so no colour literal lives here (docs/15-DESIGN.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScopeColours {
    pub bg: [u8; 3],
    pub trace: [u8; 3],
    pub red: [u8; 3],
    pub green: [u8; 3],
    pub blue: [u8; 3],
}

/// The `scope.wgsl` uniform: sampling geometry, the kind tag, and the trace
/// colours as 0..255 byte values in `f32` slots (std140: twelve `u32` then five
/// `vec4<f32>`, 128 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScopeUniform {
    kind: u32,
    grid: u32,
    n_cols: u32,
    n_rows: u32,
    sx: u32,
    sy: u32,
    width: u32,
    height: u32,
    count_len: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
    bg: [f32; 4],
    trace: [f32; 4],
    red: [f32; 4],
    green: [f32; 4],
    blue: [f32; 4],
}

fn colour_slot(rgb: [u8; 3]) -> [f32; 4] {
    [
        f32::from(rgb[0]),
        f32::from(rgb[1]),
        f32::from(rgb[2]),
        255.0,
    ]
}

/// Pixel strides (x, y) that keep the sampled pixel count near [`MAX_SAMPLES`],
/// scaling both axes by the same factor so coverage stays even — a copy of the
/// CPU path's `strides`, so the GPU walks the identical sampled grid.
pub fn strides(width: usize, height: usize) -> (usize, usize) {
    let total = width.saturating_mul(height).max(1);
    if total <= MAX_SAMPLES {
        return (1, 1);
    }
    let factor = ((total as f64 / MAX_SAMPLES as f64).sqrt()).max(1.0);
    let s = (factor.ceil() as usize).max(1);
    (s, s)
}

/// The scope-pass engine: the three compiled kernels plus the counting buffers,
/// owned alongside the `Compositor`/`FxEngine` by whoever renders. Built once
/// (shaders compile once); the buffers are re-used every trace.
pub struct ScopeEngine {
    bin: wgpu::ComputePipeline,
    peak: wgpu::ComputePipeline,
    colourise: wgpu::ComputePipeline,
    bin_layout: wgpu::BindGroupLayout,
    colourise_layout: wgpu::BindGroupLayout,
    /// The bin counters (`COUNTS_LEN` `u32`), cleared each trace.
    counts: wgpu::Buffer,
    /// The single peak value, cleared each trace.
    peak_buf: wgpu::Buffer,
}

impl ScopeEngine {
    pub fn new(ctx: &GpuContext) -> Self {
        let shader = ctx
            .device
            .create_shader_module(wgpu::include_wgsl!("scope.wgsl"));

        // Bin + peak share one layout: src texture (0), counts (1), peak (2),
        // uniform (3). The peak pass ignores `src`, but sharing the layout keeps
        // a single bind group for both passes.
        let bin_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scope-bin-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            // textureLoad only (no sampler), so unfilterable is
                            // the tightest honest declaration.
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    storage_entry(1, false),
                    storage_entry(2, false),
                    uniform_entry(3),
                ],
            });
        let bin_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scope-bin-pl"),
                bind_group_layouts: &[&bin_layout],
                push_constant_ranges: &[],
            });

        // Colourise: counts read-only (4), peak read-only (5), uniform (6), the
        // trace storage texture (7).
        let colourise_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("scope-colourise-layout"),
                    entries: &[
                        storage_entry(4, true),
                        storage_entry(5, true),
                        uniform_entry(6),
                        wgpu::BindGroupLayoutEntry {
                            binding: 7,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: TRACE_FORMAT,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            count: None,
                        },
                    ],
                });
        let colourise_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scope-colourise-pl"),
                bind_group_layouts: &[&colourise_layout],
                push_constant_ranges: &[],
            });

        let make = |layout: &wgpu::PipelineLayout, entry: &str, label: &str| {
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: Some(layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let bin = make(&bin_pl, "bin", "scope-bin");
        let peak = make(&bin_pl, "peak_reduce", "scope-peak");
        let colourise = make(&colourise_pl, "colourise", "scope-colourise");

        let counts = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope-counts"),
            size: (COUNTS_LEN * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let peak_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope-peak"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            bin,
            peak,
            colourise,
            bin_layout,
            colourise_layout,
            counts,
            peak_buf,
        }
    }

    /// Upload display-encoded RGBA8 bytes (a comp frame the Viewer shows) as a
    /// plain `Rgba8Unorm` texture the scope pass reads. Non-sRGB so `textureLoad`
    /// returns the exact display bytes the CPU scope reads back — no linearise.
    pub fn upload_frame(&self, ctx: &GpuContext, rgba: &[u8], w: u32, h: u32) -> wgpu::Texture {
        let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scope-frame"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Guard a short buffer so a malformed frame never reads out of bounds.
        let need = (w as usize) * (h as usize) * 4;
        if rgba.len() >= need && need > 0 {
            ctx.queue.write_texture(
                tex.as_image_copy(),
                &rgba[..need],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        tex
    }

    /// Compute the `GRID × GRID` RGBA8 trace for `kind` from a comp frame's
    /// display bytes, in the given `colours`. The returned buffer is the tightly
    /// packed trace (`GRID * GRID * 4` bytes), byte-for-byte the same the CPU
    /// path builds (within the ±1 rounding the fp maths allows across adapters).
    /// The heavy work — binning every sampled pixel — runs on the GPU; only the
    /// tiny trace is read back.
    pub fn trace_rgba8(
        &self,
        ctx: &GpuContext,
        kind: ScopeKind,
        colours: ScopeColours,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<Vec<u8>, GpuError> {
        let src = self.upload_frame(ctx, rgba, w, h);
        self.trace(ctx, kind, colours, &src, w, h)
    }

    /// As [`Self::trace_rgba8`], from an already-uploaded `Rgba8Unorm` frame
    /// texture — the path a fresh render can take to consume the frame it has on
    /// the GPU without a round trip through bytes.
    pub fn trace(
        &self,
        ctx: &GpuContext,
        kind: ScopeKind,
        colours: ScopeColours,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
    ) -> Result<Vec<u8>, GpuError> {
        let dst = self.run(ctx, kind, colours, src, w, h);
        readback_trace(ctx, &dst)
    }

    /// Run the three passes into a fresh trace texture, returning it (the shared
    /// core of [`Self::trace`] and the test count hook).
    fn run(
        &self,
        ctx: &GpuContext,
        kind: ScopeKind,
        colours: ScopeColours,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
    ) -> wgpu::Texture {
        let (sx, sy) = strides(w as usize, h as usize);
        let sx = sx.max(1) as u32;
        let sy = sy.max(1) as u32;
        let n_cols = w.div_ceil(sx).max(1);
        let n_rows = h.div_ceil(sy).max(1);
        let count_len = kind.count_len();

        let uniform = ScopeUniform {
            kind: kind.tag(),
            grid: GRID,
            n_cols,
            n_rows,
            sx,
            sy,
            width: w,
            height: h,
            count_len,
            _p0: 0,
            _p1: 0,
            _p2: 0,
            bg: colour_slot(colours.bg),
            trace: colour_slot(colours.trace),
            red: colour_slot(colours.red),
            green: colour_slot(colours.green),
            blue: colour_slot(colours.blue),
        };
        let ubuf = wgpu::util::DeviceExt::create_buffer_init(
            &ctx.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("scope-uniform"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );

        let dst = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scope-trace"),
            size: wgpu::Extent3d {
                width: GRID,
                height: GRID,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TRACE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let bin_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope-bin"),
            layout: &self.bin_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &src.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.peak_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let colourise_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope-colourise"),
            layout: &self.colourise_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.peak_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(
                        &dst.create_view(&Default::default()),
                    ),
                },
            ],
        });

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("scope"),
            });
        // Zero the counters and the peak before binning.
        encoder.clear_buffer(&self.counts, 0, Some((count_len as u64) * 4));
        encoder.clear_buffer(&self.peak_buf, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scope-bin"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bin);
            pass.set_bind_group(0, &bin_bind, &[]);
            pass.dispatch_workgroups(n_cols.div_ceil(8), n_rows.div_ceil(8), 1);
            pass.set_pipeline(&self.peak);
            pass.dispatch_workgroups(count_len.div_ceil(256), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scope-colourise"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.colourise);
            pass.set_bind_group(0, &colourise_bind, &[]);
            pass.dispatch_workgroups(GRID.div_ceil(8), GRID.div_ceil(8), 1);
        }
        ctx.queue.submit([encoder.finish()]);
        dst
    }
}

/// The trace texture format: plain `Rgba8Unorm` (not sRGB), so the byte values
/// written are exactly the byte values read — the trace holds display-space
/// bytes the frontend blits as-is, the same as the CPU path's raw buffer.
const TRACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Read the `GRID × GRID` trace texture back as tight RGBA8. The row is exactly
/// `GRID * 4 = 1024` bytes, already a multiple of `COPY_BYTES_PER_ROW_ALIGNMENT`
/// (256), so no per-row padding is needed.
fn readback_trace(ctx: &GpuContext, tex: &wgpu::Texture) -> Result<Vec<u8>, GpuError> {
    let row = GRID * 4;
    debug_assert_eq!(row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scope-readback"),
        size: u64::from(row) * u64::from(GRID),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(GRID),
            },
        },
        wgpu::Extent3d {
            width: GRID,
            height: GRID,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|e| GpuError::Readback(e.to_string()))?
        .map_err(|e| GpuError::Readback(e.to_string()))?;
    let data = slice.get_mapped_range();
    let out = data.to_vec();
    drop(data);
    buffer.unmap();
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const STANDARD: ScopeColours = ScopeColours {
        bg: [0x0a, 0x0b, 0x0c],
        trace: [0x86, 0xdd, 0x9a],
        red: [0xe2, 0x55, 0x5f],
        green: [0x54, 0xcf, 0x6b],
        blue: [0x53, 0x87, 0xe0],
    };

    /// A `w`×`h` frame of one solid colour, opaque.
    fn solid(w: usize, h: usize, r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            v.extend_from_slice(&[r, g, b, 0xff]);
        }
        v
    }

    // ---- CPU oracle (a verbatim port of the shell/scopes.rs counting) --------

    fn luma8(r: u8, g: u8, b: u8) -> f32 {
        (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
    }
    fn value_row(v: f32) -> usize {
        let clamped = v.clamp(0.0, 1.0);
        (((1.0 - clamped) * (GRID as f32 - 1.0)).round() as usize).min(GRID as usize - 1)
    }
    fn g() -> usize {
        GRID as usize
    }

    fn waveform_luma_counts(rgba: &[u8], w: usize, h: usize) -> Vec<u32> {
        let mut grid = vec![0u32; g() * g()];
        let (sx, sy) = strides(w, h);
        let (mut y, gg) = (0, g());
        while y < h {
            let mut x = 0;
            while x < w {
                let i = (y * w + x) * 4;
                let by = value_row(luma8(rgba[i], rgba[i + 1], rgba[i + 2]));
                let bx = (x * gg / w).min(gg - 1);
                grid[by * gg + bx] += 1;
                x += sx;
            }
            y += sy;
        }
        grid
    }

    fn histogram_counts(rgba: &[u8], w: usize, h: usize) -> [Vec<u32>; 3] {
        let mut bins = [vec![0u32; g()], vec![0u32; g()], vec![0u32; g()]];
        let (sx, sy) = strides(w, h);
        let (mut y, gg) = (0, g());
        while y < h {
            let mut x = 0;
            while x < w {
                let i = (y * w + x) * 4;
                for (c, bin) in bins.iter_mut().enumerate() {
                    bin[(rgba[i + c] as usize * (gg - 1)) / 255] += 1;
                }
                x += sx;
            }
            y += sy;
        }
        bins
    }

    /// Read the raw counts buffer back for a kind — the exact-integer oracle
    /// hook (atomics never round, so this must match the CPU bit-for-bit).
    fn gpu_counts(
        engine: &ScopeEngine,
        ctx: &GpuContext,
        kind: ScopeKind,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Vec<u32> {
        let src = engine.upload_frame(ctx, rgba, w, h);
        let _ = engine.run(ctx, kind, STANDARD, &src, w, h);
        // The counts buffer holds this kind's populated prefix after `run`.
        let len = kind.count_len() as usize;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope-counts-readback"),
            size: (len * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&engine.counts, 0, &staging, 0, (len * 4) as u64);
        ctx.queue.submit([enc.finish()]);
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let out: Vec<u32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        out
    }

    fn ctx() -> Option<GpuContext> {
        GpuContext::headless().ok()
    }

    /// A solid grey's luma waveform lands its whole sampled count on one row —
    /// the GPU bin counts equal the CPU oracle exactly (integers, no rounding).
    #[test]
    fn luma_waveform_counts_match_the_cpu_oracle() {
        let Some(ctx) = ctx() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let engine = ScopeEngine::new(&ctx);
        let frame = solid(16, 16, 128, 128, 128);
        let gpu = gpu_counts(&engine, &ctx, ScopeKind::WaveformLuma, &frame, 16, 16);
        let cpu = waveform_luma_counts(&frame, 16, 16);
        assert_eq!(gpu, cpu, "GPU luma bins must equal the CPU oracle");
        // And the CPU maths itself: all 256 samples on the grey's own row.
        let row = value_row(luma8(128, 128, 128));
        let in_row: u32 = (0..g()).map(|x| gpu[row * g() + x]).sum();
        assert_eq!(in_row, 16 * 16);
        assert_eq!(gpu.iter().sum::<u32>(), 16 * 16);
    }

    /// A solid's histogram puts every sampled pixel in one bin per channel — the
    /// GPU counts equal the CPU oracle exactly.
    #[test]
    fn histogram_counts_match_the_cpu_oracle() {
        let Some(ctx) = ctx() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let engine = ScopeEngine::new(&ctx);
        let frame = solid(10, 10, 255, 0, 64);
        let gpu = gpu_counts(&engine, &ctx, ScopeKind::Histogram, &frame, 10, 10);
        let cpu = histogram_counts(&frame, 10, 10);
        // gpu is [r bins.., g bins.., b bins..]; compare channel by channel.
        for c in 0..3 {
            let slice = &gpu[c * g()..(c + 1) * g()];
            assert_eq!(slice, &cpu[c][..], "channel {c} histogram");
            assert_eq!(slice.iter().sum::<u32>(), 100, "every pixel counted once");
        }
        assert_eq!(gpu[g() - 1], 100, "red maxed → top bin");
        assert_eq!(gpu[g()], 100, "green zero → bottom bin");
    }

    /// A neutral grey's vectorscope energy sits at the grid centre (zero chroma).
    #[test]
    fn vectorscope_centres_a_neutral_grey() {
        let Some(ctx) = ctx() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let engine = ScopeEngine::new(&ctx);
        let frame = solid(8, 8, 128, 128, 128);
        let gpu = gpu_counts(&engine, &ctx, ScopeKind::Vectorscope, &frame, 8, 8);
        let mid = (g() - 1) / 2;
        let peak_cell = (0..g() * g()).max_by_key(|&c| gpu[c]).unwrap();
        let (px, py) = (peak_cell % g(), peak_cell / g());
        assert!(px.abs_diff(mid) <= 1 && py.abs_diff(mid) <= 1);
        assert_eq!(gpu.iter().sum::<u32>(), 64, "every pixel counted once");
    }

    /// The colourised trace matches the CPU-built trace within a small rounding
    /// tolerance (fp `sqrt` differs by an ULP across adapters — the same reason
    /// the colour golden allows ±1). A solid grey's luma trace: the grey's row
    /// carries the trace colour over the backdrop; other rows stay the backdrop.
    #[test]
    fn luma_trace_pixels_match_within_tolerance() {
        let Some(ctx) = ctx() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let engine = ScopeEngine::new(&ctx);
        // 256 wide so every column bins (bx = x·256/256 = x); every column of the
        // grey's row is then fully lit (peak == the per-column count).
        let frame = solid(256, 64, 128, 128, 128);
        let trace = engine
            .trace_rgba8(&ctx, ScopeKind::WaveformLuma, STANDARD, &frame, 256, 64)
            .expect("trace");
        assert_eq!(trace.len(), g() * g() * 4);
        let row = value_row(luma8(128, 128, 128));
        // Software rasterisers land a bit off hardware on the fp maths.
        let tol: i16 = if ctx.software { 2 } else { 1 };
        // The grey's own row is fully lit (peak == count), so trace = bg + trace.
        for x in 0..g() {
            let cell = (row * g() + x) * 4;
            let got = [trace[cell], trace[cell + 1], trace[cell + 2]];
            let want = [
                (STANDARD.bg[0] as u16 + STANDARD.trace[0] as u16).min(255) as u8,
                (STANDARD.bg[1] as u16 + STANDARD.trace[1] as u16).min(255) as u8,
                (STANDARD.bg[2] as u16 + STANDARD.trace[2] as u16).min(255) as u8,
            ];
            for c in 0..3 {
                let d = (got[c] as i16 - want[c] as i16).abs();
                assert!(
                    d <= tol,
                    "row {row} col {x} chan {c}: {} vs {}",
                    got[c],
                    want[c]
                );
            }
            assert_eq!(trace[cell + 3], 0xff, "trace is opaque");
        }
        // A far-away row is untouched backdrop.
        let empty_row = if row > 10 { 0 } else { g() - 1 };
        let cell = (empty_row * g()) * 4;
        assert_eq!(&trace[cell..cell + 3], &STANDARD.bg);
    }

    /// A malformed (too-short) frame never panics or reads out of bounds: the
    /// frame texture is left zero-filled (a black frame), so the pass produces a
    /// valid, opaque `GRID × GRID` trace — the calm-degradation guarantee.
    #[test]
    fn a_short_frame_is_calm() {
        let Some(ctx) = ctx() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let engine = ScopeEngine::new(&ctx);
        let trace = engine
            .trace_rgba8(&ctx, ScopeKind::WaveformRgb, STANDARD, &[1, 2, 3], 8, 8)
            .expect("trace");
        assert_eq!(trace.len(), g() * g() * 4, "a full-size trace is produced");
        for px in trace.chunks_exact(4) {
            assert_eq!(px[3], 0xff, "the trace is opaque");
        }
    }
}
