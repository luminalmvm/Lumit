//! The Correction's passes (`fx_correction.wgsl`), in the order
//! `lumit_core::fx::effects::correction::cpu` runs them.
//!
//! **In plain terms.** The key and the grade are separate steps.
//! [`FxEngine::correction_key`] makes the node's key as a picture of its own,
//! the same number in every channel, and [`FxEngine::correction`] grades with
//! it. Only the passes a node needs run: no qualifier, no window and no matte
//! means no key at all, and a blur at nought is never drawn.

use std::sync::OnceLock;

use crate::GpuContext;

use super::blur::BlurOp;
use super::lens_blur::{bind_layout, Slot};
use super::stylise::{MatteTidyParams, MATTE_MORPH_MAX_PX};
use super::{work_texture, FxEngine};

/// One Correction, already packed. Mirrors
/// `lumit_core::fx::effects::correction::packed::Packed`, whose `words` and
/// `data` these are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorrectionOp<'a> {
    /// The uniform, laid out as `Params` in `fx_correction.wgsl`.
    pub words: &'a [[f32; 4]],
    /// The curve tables, then the window outlines.
    pub data: &'a [f32],
    /// The grade is at rest: the picture goes through untouched.
    pub neutral: bool,
    pub qualifier_on: bool,
    pub windows_on: bool,
    /// Raster pixels. Nought runs no pass.
    pub denoise_px: f32,
    pub blur_px: f32,
    /// Positive grows, negative shrinks.
    pub grow_px: f32,
    /// Midtone detail's own blur, read only when `detail_on`.
    pub detail_px: f32,
    pub detail_on: bool,
}

pub(super) struct CorrectionFx {
    layout: wgpu::BindGroupLayout,
    vin: wgpu::ComputePipeline,
    qualify: wgpu::ComputePipeline,
    key: wgpu::ComputePipeline,
    grade: wgpu::ComputePipeline,
    parallel: wgpu::ComputePipeline,
}

impl CorrectionFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-correction"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(ctx.working(), include_str!("../fx_correction.wgsl")).into(),
            ),
        });
        let layout = bind_layout(
            device,
            "fx-correction",
            &[
                (0, Slot::Tex),
                (1, Slot::Tex),
                (2, Slot::Store(ctx.working())),
                (3, Slot::Uniform),
                (4, Slot::Tex),
                (6, Slot::Spans),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-correction"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipe = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            vin: pipe("correction_vin"),
            qualify: pipe("correction_qualify"),
            key: pipe("correction_key"),
            grade: pipe("correction_grade"),
            parallel: pipe("correction_parallel"),
            layout,
        }
    }
}

/// Built on first use.
pub(super) type LazyCorrection = OnceLock<CorrectionFx>;

/// The two buffers every pass of one Correction reads.
struct Buffers {
    params: wgpu::Buffer,
    data: wgpu::Buffer,
}

impl Buffers {
    fn new(ctx: &GpuContext, op: &CorrectionOp<'_>) -> Self {
        use wgpu::util::DeviceExt;
        // A storage buffer may not be empty.
        let data: &[f32] = if op.data.is_empty() { &[0.0] } else { op.data };
        Self {
            params: ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("fx-correction-params"),
                    contents: bytemuck::cast_slice(op.words),
                    usage: wgpu::BufferUsages::UNIFORM,
                }),
            data: ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("fx-correction-data"),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
        }
    }
}

impl FxEngine {
    /// One pass of the Correction into a new texture: `src` at binding 0,
    /// `a` and `b` at 1 and 4, which a pass that reads neither is handed
    /// `src` for.
    #[allow(clippy::too_many_arguments)]
    fn correction_pass(
        &self,
        ctx: &GpuContext,
        pipe: &wgpu::ComputePipeline,
        bufs: &Buffers,
        src: &wgpu::Texture,
        a: Option<&wgpu::Texture>,
        b: Option<&wgpu::Texture>,
        w: u32,
        h: u32,
    ) -> wgpu::Texture {
        let fx = self.correction.get_or_init(|| CorrectionFx::new(ctx));
        let out = work_texture(ctx, w, h, "fx-correction");
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (v0, v1, v2, v4) = (
            view(src),
            view(a.unwrap_or(src)),
            view(&out),
            view(b.unwrap_or(src)),
        );
        fn tex(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(v),
            }
        }
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-correction"),
            layout: &fx.layout,
            entries: &[
                tex(0, &v0),
                tex(1, &v1),
                tex(2, &v2),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: bufs.params.as_entire_binding(),
                },
                tex(4, &v4),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: bufs.data.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-correction");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-correction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// The Gaussian blur Denoise, Blur and Midtone detail use, with the
    /// edge held.
    fn correction_blur(
        &self,
        ctx: &GpuContext,
        src: wgpu::Texture,
        w: u32,
        h: u32,
        radius_px: f32,
    ) -> wgpu::Texture {
        let op = BlurOp {
            radius_px,
            edge: 1,
            mix: 1.0,
        };
        let out = self.blur(ctx, &src, w, h, None, &op);
        ctx.recycle(src);
        out
    }

    /// The key step on buffers already made.
    #[allow(clippy::too_many_arguments)]
    fn correction_key_with(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &CorrectionOp<'_>,
        bufs: &Buffers,
    ) -> Option<wgpu::Texture> {
        if !op.qualifier_on && !op.windows_on && matte.is_none() {
            return None;
        }
        let fx = self.correction.get_or_init(|| CorrectionFx::new(ctx));
        let qual = op.qualifier_on.then(|| {
            let judged = (op.denoise_px > 0.0).then(|| {
                let vin = self.correction_pass(ctx, &fx.vin, bufs, src, None, None, w, h);
                self.correction_blur(ctx, vin, w, h, op.denoise_px)
            });
            let mut k =
                self.correction_pass(ctx, &fx.qualify, bufs, src, judged.as_ref(), None, w, h);
            if let Some(spent) = judged {
                ctx.recycle(spent);
            }
            if op.grow_px != 0.0 {
                k = self.correction_grow(ctx, k, w, h, op.grow_px);
            }
            if op.blur_px > 0.0 {
                k = self.correction_blur(ctx, k, w, h, op.blur_px);
            }
            k
        });
        let key = self.correction_pass(ctx, &fx.key, bufs, src, qual.as_ref(), matte, w, h);
        if let Some(spent) = qual {
            ctx.recycle(spent);
        }
        Some(key)
    }

    /// Grow or shrink the qualifier's key: the Matte key's two separable
    /// passes.
    fn correction_grow(
        &self,
        ctx: &GpuContext,
        k: wgpu::Texture,
        w: u32,
        h: u32,
        amount: f32,
    ) -> wgpu::Texture {
        let r = amount.abs().min(MATTE_MORPH_MAX_PX);
        let ri = r.floor() as i32;
        let tidy = |dx: i32, dy: i32| MatteTidyParams {
            dx,
            dy,
            ri,
            frac: r - ri as f32,
            grow: u32::from(amount > 0.0),
            black: 0.0,
            white: 0.0,
            _pad0: 0.0,
        };
        let across = work_texture(ctx, w, h, "fx-correction-grow");
        self.dispatch(
            ctx,
            &self.matte_morph,
            &k,
            &k,
            &across,
            w,
            h,
            bytemuck::bytes_of(&tidy(1, 0)),
        );
        ctx.recycle(k);
        let out = work_texture(ctx, w, h, "fx-correction-grow");
        self.dispatch(
            ctx,
            &self.matte_morph,
            &across,
            &across,
            &out,
            w,
            h,
            bytemuck::bytes_of(&tidy(0, 1)),
        );
        ctx.recycle(across);
        out
    }

    /// The node's key: the windows, times the qualifier's key, times the
    /// matte, the same number in every channel. `None` when nothing keys, so
    /// every pixel takes the whole grade. The picture the key is judged on is
    /// `src`; `matte` is the prepared matte.
    pub fn correction_key(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &CorrectionOp<'_>,
    ) -> Option<wgpu::Texture> {
        let bufs = Buffers::new(ctx, op);
        self.correction_key_with(ctx, src, w, h, matte, op, &bufs)
    }

    /// The node's key as a picture even when nothing keys, white then: what
    /// a key wire hands on.
    pub fn correction_key_picture(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &CorrectionOp<'_>,
    ) -> wgpu::Texture {
        let bufs = Buffers::new(ctx, op);
        self.correction_key_with(ctx, src, w, h, matte, op, &bufs)
            .unwrap_or_else(|| {
                // With nothing keying, the key pass draws the windows' 1.
                let fx = self.correction.get_or_init(|| CorrectionFx::new(ctx));
                self.correction_pass(ctx, &fx.key, &bufs, src, None, None, w, h)
            })
    }

    /// A Parallel node joined to its run: `acc + (out - input)`, where `out`
    /// is the node drawn from the run's `input`. `op` only fills the buffers.
    #[allow(clippy::too_many_arguments)]
    pub fn correction_parallel(
        &self,
        ctx: &GpuContext,
        acc: &wgpu::Texture,
        out: &wgpu::Texture,
        input: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &CorrectionOp<'_>,
    ) -> wgpu::Texture {
        let fx = self.correction.get_or_init(|| CorrectionFx::new(ctx));
        let bufs = Buffers::new(ctx, op);
        self.correction_pass(ctx, &fx.parallel, &bufs, acc, Some(out), Some(input), w, h)
    }

    /// The Correction over a linear working texture (premultiplied). A node at
    /// rest hands `src` back without drawing.
    pub fn correction(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &CorrectionOp<'_>,
    ) -> wgpu::Texture {
        if op.neutral {
            return src.clone();
        }
        let fx = self.correction.get_or_init(|| CorrectionFx::new(ctx));
        let bufs = Buffers::new(ctx, op);
        let key = self.correction_key_with(ctx, src, w, h, matte, op, &bufs);
        let detail = op.detail_on.then(|| {
            let vin = self.correction_pass(ctx, &fx.vin, &bufs, src, None, None, w, h);
            self.correction_blur(ctx, vin, w, h, op.detail_px)
        });
        let out = self.correction_pass(
            ctx,
            &fx.grade,
            &bufs,
            src,
            key.as_ref(),
            detail.as_ref(),
            w,
            h,
        );
        for spent in [key, detail].into_iter().flatten() {
            ctx.recycle(spent);
        }
        out
    }
}
