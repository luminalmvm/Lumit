//! Time-based kernels (docs/08 §3.2, §3.12, §3.13): echo/trails, flow motion
//! blur and datamosh, each sampling neighbour frames or a flow field.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved echo (docs/08 §3.13; blend modes + 16-echo cap since
/// FX-17/K-149). The neighbour frames arrive as textures keyed by offset;
/// `weights[i]` is the tap intensity for the echo at offset `-(i+1)`
/// (0 = skip). `mode` is the combine blend: 0 = Add, 1 = Behind, 2 = Max,
/// 3 = Screen, 4 = Normal, 5 = Multiply, 6 = Overlay, 7 = Soft light,
/// 8 = Hard light, 9 = Darken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoOp {
    pub weights: [f32; 16],
    pub mode: u32,
    /// 0..1, blended against the leading (current) frame.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EchoParams {
    weight: f32,
    mode: u32,
    _pad: [f32; 2],
}

/// One resolved flow motion blur (docs/08 §3.2). The per-pixel motion is a
/// dense flow field passed as its own texture (see [`upload_flow_field`] and
/// [`FxEngine::motion_blur`]); this op carries only the scalars the kernel
/// turns a vector into a streak with. `samples` must equal the tap count
/// `lumit_core::fx::effects::motion_blur::MotionBlur::packed` produces, so the
/// GPU integrates the CPU oracle's exact tap count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionBlurOp {
    /// Shutter ÷ 360: streak length as a fraction of the inter-frame motion.
    pub shutter_frac: f32,
    /// Evenly spaced bilinear taps along the streak.
    pub samples: i32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// Output view (FX-19): 0 Rendered, 1 Motion vectors, 2 Confidence — the
    /// `lumit_core::fx::MbView::code()` integer, so the kernel matches the CPU
    /// oracle's `view` branch.
    pub view: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionBlurParams {
    shutter_frac: f32,
    samples: i32,
    mix_amt: f32,
    view: i32,
}

/// One resolved Datamosh pass (docs/08 §3.12, K-104; its own effect since
/// K-107; reworked to a flow-driven melt by K-164/T19). The raw -1 source
/// neighbour and the dense current→previous flow field arrive as their own
/// textures (see [`FxEngine::datamosh`]); this op carries the melt scalars.
/// Callers fold the schema's Intensity and host Mix into `intensity` before
/// calling (mixing the same two inputs twice collapses to one mix by the
/// product), so this kernel and its CPU oracle need no second blend knob.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DatamoshOp {
    /// Blended over the current frame; > 1 extrapolates (K-135/FX-14).
    pub intensity: f32,
    /// Frames of predicted motion the streamline walk reaches (K-164); the
    /// walk's `steps` taps span it.
    pub displacement: f32,
    /// 0..1, how much of the reach accumulates into the smear (K-161): 0 a
    /// short trail, 1 a long melting bloom.
    pub bloom: f32,
    /// Bilinear taps along the walk (2..64, or 1 at a sub-frame reach) — the
    /// same count the CPU oracle loops.
    pub steps: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DatamoshParams {
    intensity: f32,
    displacement: f32,
    bloom: f32,
    steps: i32,
}

impl FxEngine {
    /// Apply one echo/trails (docs/08 §3.13) to a linear working texture,
    /// returning a new texture of the same size. Starts the accumulator as
    /// the current frame (an `echo_accumulate` with weight 0 copies it), folds
    /// in each live tap's neighbour (looked up by offset `-(i+1)`), then mixes
    /// the trail back toward the current frame. A missing neighbour or a zero
    /// weight is skipped, so the pass cost tracks the live tap count.
    pub fn echo(
        &self,
        ctx: &GpuContext,
        current: &wgpu::Texture,
        neighbours: &[(i32, &wgpu::Texture)],
        w: u32,
        h: u32,
        op: &EchoOp,
    ) -> wgpu::Texture {
        let params = |weight: f32, mode: u32| EchoParams {
            weight,
            mode,
            _pad: [0.0; 2],
        };
        // acc := current (weight 0 add = a + n*0 = a).
        let mut acc = work_texture(ctx, w, h, "fx-echo-acc");
        self.dispatch(
            ctx,
            &self.echo_accumulate,
            current,
            current,
            &acc,
            w,
            h,
            bytemuck::bytes_of(&params(0.0, 0)),
        );
        for (i, &weight) in op.weights.iter().enumerate() {
            if weight <= 0.0 {
                continue;
            }
            let offset = -(i as i32 + 1);
            let Some((_, tex)) = neighbours.iter().find(|(o, _)| *o == offset) else {
                continue;
            };
            let next = work_texture(ctx, w, h, "fx-echo-acc");
            self.dispatch(
                ctx,
                &self.echo_accumulate,
                &acc,
                tex,
                &next,
                w,
                h,
                bytemuck::bytes_of(&params(weight, op.mode)),
            );
            acc = next;
        }
        let out = work_texture(ctx, w, h, "fx-echo-out");
        self.dispatch(
            ctx,
            &self.echo_mix,
            &acc,
            current,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params(op.mix, 0)),
        );
        out
    }

    /// Apply one flow motion blur (docs/08 §3.2) to a linear working texture,
    /// returning a new texture of the same size. One pass: per output pixel,
    /// read its motion vector from `flow` (a two-channel field the same size
    /// as `src`, in raster pixels) and integrate `op.samples` box-weighted
    /// bilinear taps along the centred streak `± motion × shutter_frac`, then
    /// blend against the input by the host Mix. `flow`'s vectors are consumed
    /// exactly as `lumit_core::fx::cpu::motion_blur` reads its `u`/`v` slices,
    /// so the two agree (§1.6). Its own bind group (the flow field is the one
    /// extra input over the shared two-input shape).
    pub fn motion_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        flow: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &MotionBlurOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-mb-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-mb-params"),
                contents: bytemuck::bytes_of(&MotionBlurParams {
                    shutter_frac: op.shutter_frac,
                    samples: op.samples.max(1),
                    mix_amt: op.mix,
                    view: op.view,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-mb-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                // orig-for-mix: a single pass, so the unprocessed original is
                // the source itself.
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(flow)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-mb-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-mb-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.motion_blur);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// Apply Datamosh (docs/08 §3.12, K-104; its own effect since K-107;
    /// reworked to a flow-driven melt by K-164/T19) to a linear working
    /// texture, returning a new texture of the same size. One pass: per output
    /// pixel, a streamline walk of `op.steps` taps follows the `flow` field out
    /// of `prev` (re-sampling the flow each step, advancing ~one frame of
    /// motion), accumulating the samples with a `op.bloom` geometric weight,
    /// then blends the weighted mean over `cur` by Intensity. Shares
    /// [`Self::mb_layout`]/its pipeline layout with Motion blur (same
    /// three-sampled-input shape); its own pipeline and shader.
    #[allow(clippy::too_many_arguments)]
    pub fn datamosh(
        &self,
        ctx: &GpuContext,
        cur: &wgpu::Texture,
        prev: &wgpu::Texture,
        flow: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &DatamoshOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-dm-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-dm-params"),
                contents: bytemuck::bytes_of(&DatamoshParams {
                    intensity: op.intensity,
                    displacement: op.displacement,
                    bloom: op.bloom,
                    steps: op.steps,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-dm-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(cur)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(prev)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(flow)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-dm-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-dm-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.datamosh);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
