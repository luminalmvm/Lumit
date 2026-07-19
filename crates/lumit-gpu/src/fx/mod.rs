//! The GPU effect kernels (docs/05 crate table: "WGSL effect kernels" live
//! here; docs/08-EFFECTS.md §1.1 part 2 — the production path). Each kernel
//! mirrors its CPU reference in `lumit_core::fx::cpu` op-for-op; the §1.6
//! oracle tests at the bottom hold the two to agreement.
//!
//! In plain terms: this is where effects actually run during preview and
//! export — small GPU programs working on the same linear fp16 textures the
//! compositor uses. The engine takes plain numbers (a blur radius in pixels,
//! an edge mode), so it neither knows nor cares about the project model.

use crate::{GpuContext, WORKING_FORMAT};

mod blur;
mod colour;
mod common;
mod dof;
mod engine;
mod split;
mod stylise;
mod temporal;

pub use blur::*;
pub use colour::*;
pub use common::*;
pub use split::*;
pub use stylise::*;
pub use temporal::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

/// The effect-pass engine: compiled kernels plus their layouts, one per
/// device (owned alongside the Compositor by whoever renders).
pub struct FxEngine {
    blur: wgpu::ComputePipeline,
    dir_blur: wgpu::ComputePipeline,
    radial_blur: wgpu::ComputePipeline,
    sharpen_unpremultiply: wgpu::ComputePipeline,
    sharpen_combine: wgpu::ComputePipeline,
    /// Plain 3×3 sharpen (docs/08 §3.9, K-138): a single-pass high-pass
    /// convolution, the radius-free sibling of the Unsharp mask's two-entry
    /// kernel above.
    sharpen_simple: wgpu::ComputePipeline,
    rgb_split: wgpu::ComputePipeline,
    spectral_split: wgpu::ComputePipeline,
    chromatic_aberration: wgpu::ComputePipeline,
    flash: wgpu::ComputePipeline,
    colour_balance: wgpu::ComputePipeline,
    saturation: wgpu::ComputePipeline,
    vibrancy: wgpu::ComputePipeline,
    matte_key: wgpu::ComputePipeline,
    vignette: wgpu::ComputePipeline,
    exposure: wgpu::ComputePipeline,
    temperature: wgpu::ComputePipeline,
    invert: wgpu::ComputePipeline,
    tint: wgpu::ComputePipeline,
    hue_shift: wgpu::ComputePipeline,
    contrast: wgpu::ComputePipeline,
    gamma: wgpu::ComputePipeline,
    transform: wgpu::ComputePipeline,
    glow_bright: wgpu::ComputePipeline,
    glow_combine: wgpu::ComputePipeline,
    block_glitch: wgpu::ComputePipeline,
    scanlines: wgpu::ComputePipeline,
    echo_accumulate: wgpu::ComputePipeline,
    echo_mix: wgpu::ComputePipeline,
    motion_blur: wgpu::ComputePipeline,
    /// Datamosh (docs/08 §3.12, K-104): shares [`Self::mb_layout`]/`mb_pl`
    /// with Motion blur — both need exactly three sampled inputs (the
    /// current frame, one extra neighbour-derived texture, and a flow
    /// field) plus a storage output and a uniform.
    datamosh: wgpu::ComputePipeline,
    adjust: wgpu::ComputePipeline,
    /// 3D-LUT lookup (docs/08 §3.11; docs/impl/lut.md). Its own pipeline and
    /// [`Self::lut_layout`]: the shared two sampled inputs (src, orig) plus
    /// the cube as a fifth binding — a 3D texture, the first effect to need
    /// one.
    lut: wgpu::ComputePipeline,
    /// Depth-of-field lens blur (foundation for the planned DoF effects).
    /// Shares [`Self::mb_layout`]/`mb_pl` with Motion blur and Datamosh —
    /// its three sampled inputs (source, unprocessed original, depth field)
    /// plus a storage output and a uniform fit the same shape.
    dof: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// The adjustment blend's own layout: three sampled inputs (below,
    /// processed, coverage) where every effect kernel takes two.
    adjust_layout: wgpu::BindGroupLayout,
    /// Flow motion blur's own layout: the shared two inputs (src, orig) plus
    /// the flow-field texture — the one extra sampled input this kernel
    /// needs. Also Datamosh's layout (see [`Self::datamosh`]): its three
    /// sampled inputs (current, previous, flow) fit the same shape.
    mb_layout: wgpu::BindGroupLayout,
    /// The LUT lookup's own layout (see [`Self::lut`]): src (0), orig (1),
    /// the storage output (2), the uniform (3) and the 3D cube texture (4).
    lut_layout: wgpu::BindGroupLayout,
}

impl FxEngine {
    /// One compute pass: `src` and `orig` sampled, `dst` written, `params`
    /// as the uniform — the shared plumbing every kernel dispatch uses.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        src: &wgpu::Texture,
        orig: &wgpu::Texture,
        dst: &wgpu::Texture,
        w: u32,
        h: u32,
        params: &[u8],
    ) {
        use wgpu::util::DeviceExt;
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-params"),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &src.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &orig.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &dst.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
    }
}

fn work_texture(ctx: &GpuContext, w: u32, h: u32, label: &str) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: WORKING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}
