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
mod lens_flare;
mod lighting;
mod split;
mod stylise;
mod temporal;

pub use blur::*;
pub use colour::*;
pub use common::*;
// `dof` exposes its `impl FxEngine` methods, which are reachable without a
// re-export — but it also houses the `DofOp` parameter struct that carries the
// effect's two dozen scalars, and a public type does need naming.
pub use dof::*;
pub use lens_flare::*;
pub use lighting::*;
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
    /// Sprite flare (docs/08 §3.29, K-359): one procedural pass, placed from
    /// a light position rather than from the picture's bright pixels.
    sprite_flare: wgpu::ComputePipeline,
    /// Light wrap (docs/08 §3.28, K-358): the two passes that fold the
    /// background's blur and the foreground's softened matte into the edge.
    light_wrap_pack: wgpu::ComputePipeline,
    light_wrap_combine: wgpu::ComputePipeline,
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
    /// The lighting pass (docs/06, K-361). Not an effect — the realiser calls
    /// it directly, between a layer's effect stack and its composite.
    lighting: wgpu::ComputePipeline,
    temperature: wgpu::ComputePipeline,
    invert: wgpu::ComputePipeline,
    tint: wgpu::ComputePipeline,
    hue_shift: wgpu::ComputePipeline,
    contrast: wgpu::ComputePipeline,
    gamma: wgpu::ComputePipeline,
    transform: wgpu::ComputePipeline,
    /// The shake's own motion blur (docs/08 §3.4, T18/K-165): averages the
    /// shake resampled at its motion-blur sub-frames. Its own kernel rather
    /// than the Transform kernel because it reads the input at several affines
    /// in one pass; it uses the shared two-input layout all the same.
    shake_mb: wgpu::ComputePipeline,
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
    /// Lens flare (docs/08 §3.27, K-256): the one effect that owns a render
    /// pass — its pipelines, layouts and bake cache live in their own
    /// sub-struct rather than six more fields here.
    lens_flare: lens_flare::LazyFlare,
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
    /// Let this engine make a Lens flare's bake **beside** the frame rather
    /// than inside it (K-350).
    ///
    /// Off by default, and that default is the safe one: an engine nobody has
    /// told otherwise bakes inside the frame exactly as it always did, so a
    /// path that has not opted in — the exporter's, which builds its own
    /// engine on its own device — cannot draw a provisional picture by
    /// omission. The Viewer's engine turns it on, and a frame whose lens is
    /// still being baked draws the lens the last frame drew (or, with none
    /// yet, no flare) instead of stopping for half a second of optics.
    pub fn set_deferred_flare_bakes(&self, deferred: bool) {
        self.lens_flare.set_deferred(deferred);
    }

    /// Whether a flare bake is being made right now.
    ///
    /// Answered without waiting for the flare's own pipelines to finish
    /// compiling (see [`lens_flare::LazyFlare`]): nothing can be baking before
    /// there is anything to bake with.
    #[must_use]
    pub fn flare_bake_pending(&self) -> bool {
        self.lens_flare
            .ready()
            .is_some_and(lens_flare::LensFlareFx::bake_pending)
    }

    /// A number that moves whenever a flare bake is queued or lands.
    ///
    /// What a caller compares either side of a render to answer the only
    /// question deferring the bake raises: *did this frame draw the lens its
    /// parameters name?* If the number moved, it may not have, and the frame
    /// must not be filed under a name that says it did — the frame caches are
    /// keyed by what is *in* a frame (K-178), and an entry that lies about
    /// that outlives every edit and undo that might have fixed it.
    #[must_use]
    pub fn flare_bake_generation(&self) -> u64 {
        // Zero until the flare engine exists, and honestly so: a bake cannot
        // have been queued or landed before there is one.
        self.lens_flare.ready().map_or(0, |lf| {
            lf.generation.load(std::sync::atomic::Ordering::Relaxed)
        })
    }

    /// Start making a lens's bake now, before any frame asks to draw it.
    ///
    /// The same queue a deferred miss uses, offered by name so a caller that
    /// knows a lens is *about* to be wanted can have the optics started
    /// early — and so the rule that a frame made while a bake is in flight is
    /// unnameable can be checked without waiting on a real half-second of it.
    ///
    /// Answers whether it was queued: `false` when the key is already held or
    /// already baking, or when this machine gave us no bake thread. Queueing
    /// is never required — a miss makes the bake either way.
    pub fn warm_flare_bake(&self, key: u64, bake: &lens_flare::FlareBake) -> bool {
        // The one flare call that *does* wait for the pipelines: a caller
        // asking for a bake by name has decided a flare is wanted.
        self.lens_flare.get().warm(key, bake)
    }

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
        let mut enc = ctx.encoder("fx-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
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
