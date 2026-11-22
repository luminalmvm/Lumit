//! Light rays' passes (`fx_light_rays.wgsl`): the light, four gathering
//! passes back towards the source point, and the rays put onto the picture.
//! Six dispatches whatever the ray length. Built the first time one is drawn.

use std::sync::OnceLock;

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// Gathering passes and taps a pass, as
/// `lumit_core::fx::effects::light_rays::{PASSES, TAPS}`.
pub const PASSES: usize = 4;
pub const TAPS: usize = 8;
/// Entries in the ray length table.
pub const TABLE: usize = 1024;

/// One frame of Light rays. Mirrors
/// `lumit_core::fx::effects::light_rays::LightRaysParams`.
#[derive(Debug, Clone)]
pub struct LightRaysOp {
    pub source: [f32; 2],
    /// [`TABLE`] ray lengths round the source point.
    pub lengths: Vec<f32>,
    pub threshold: f32,
    pub mask: [f32; 2],
    pub colourise: u32,
    pub based_on: u32,
    pub edge: f32,
    pub stops: [[f32; 3]; 7],
    pub at: [f32; 7],
    pub count: u32,
    pub gain: f32,
    pub source_opacity: f32,
    pub rays_opacity: f32,
    pub blend: u32,
    pub mix: f32,
    /// Each pass's weights, tap by tap.
    pub weights: [[f32; TAPS]; PASSES],
    /// Each pass's step per unit of ray length.
    pub scales: [f32; PASSES],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    source: [f32; 2],
    threshold: f32,
    colourise: u32,
    mask: [f32; 2],
    edge: f32,
    based_on: u32,
    gain: f32,
    source_opacity: f32,
    rays_opacity: f32,
    blend: u32,
    mix_amt: f32,
    scale: f32,
    count: u32,
    total: f32,
    weights: [[f32; 4]; 2],
    stops: [[f32; 4]; 7],
    lengths: [[f32; 4]; TABLE / 4],
}

pub(super) struct LightRaysFx {
    light: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
    combine: wgpu::ComputePipeline,
}

pub(super) type LazyLightRays = OnceLock<LightRaysFx>;

impl LightRaysFx {
    fn new(ctx: &GpuContext, layout: &wgpu::BindGroupLayout) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-light-rays"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(
                    ctx.working(),
                    concat!(
                        include_str!("../fx_blend_core.wgsl"),
                        include_str!("../fx_light_rays.wgsl")
                    ),
                )
                .into(),
            ),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-light-rays"),
            bind_group_layouts: &[layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry: &str| {
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
            light: pipeline("lr_light"),
            gather: pipeline("lr_gather"),
            combine: pipeline("lr_combine"),
        }
    }
}

impl FxEngine {
    /// Apply one frame of Light rays to a linear working texture, returning a
    /// new texture of the same size.
    pub fn light_rays(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &LightRaysOp,
    ) -> wgpu::Texture {
        let fx = self
            .light_rays
            .get_or_init(|| LightRaysFx::new(ctx, &self.layout));
        let mut stops = [[0.0f32; 4]; 7];
        for (s, (c, at)) in stops.iter_mut().zip(op.stops.iter().zip(op.at)) {
            *s = [c[0], c[1], c[2], at];
        }
        let mut lengths = [[0.0f32; 4]; TABLE / 4];
        for (i, l) in op.lengths.iter().take(TABLE).enumerate() {
            lengths[i / 4][i % 4] = *l;
        }
        let base = Params {
            source: op.source,
            threshold: op.threshold,
            colourise: op.colourise,
            mask: op.mask,
            edge: op.edge,
            based_on: op.based_on,
            gain: op.gain,
            source_opacity: op.source_opacity,
            rays_opacity: op.rays_opacity,
            blend: op.blend,
            mix_amt: op.mix,
            scale: 0.0,
            count: op.count,
            total: 1.0,
            weights: [[0.0; 4]; 2],
            stops,
            lengths,
        };
        let mut light = work_texture(ctx, w, h, "fx-light-rays-light");
        self.dispatch(
            ctx,
            &fx.light,
            src,
            src,
            &light,
            w,
            h,
            bytemuck::bytes_of(&base),
        );
        for k in 0..PASSES {
            let wt = op.weights[k];
            let pass = Params {
                scale: op.scales[k],
                total: wt.iter().sum(),
                weights: [[wt[0], wt[1], wt[2], wt[3]], [wt[4], wt[5], wt[6], wt[7]]],
                ..base
            };
            let next = work_texture(ctx, w, h, "fx-light-rays-gather");
            self.dispatch(
                ctx,
                &fx.gather,
                &light,
                &light,
                &next,
                w,
                h,
                bytemuck::bytes_of(&pass),
            );
            light = next;
        }
        let out = work_texture(ctx, w, h, "fx-light-rays-out");
        self.dispatch(
            ctx,
            &fx.combine,
            &light,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&base),
        );
        out
    }
}
