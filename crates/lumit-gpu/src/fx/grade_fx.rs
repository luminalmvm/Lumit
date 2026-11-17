//! CDL and the colour warper (`fx_grade.wgsl`): one pass each, built the
//! first time either is drawn.
//!
//! **In plain terms.** Both read sixteen `vec4`s their packed forms in
//! `lumit_core::fx::effects` wrote, so the CPU twin and the GPU cannot read
//! the rows differently.

use std::sync::OnceLock;

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// How many `vec4`s the uniform holds.
pub const GRADE_WORDS: usize = 16;

pub(super) struct GradeFx {
    cdl: wgpu::ComputePipeline,
    warper: wgpu::ComputePipeline,
}

impl GradeFx {
    fn new(ctx: &GpuContext, layout: &wgpu::BindGroupLayout) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-grade"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(ctx.working(), include_str!("../fx_grade.wgsl")).into(),
            ),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-grade"),
            bind_group_layouts: &[layout],
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
            cdl: pipe("cdl"),
            warper: pipe("colour_warper"),
        }
    }
}

/// Built on first use.
pub(super) type LazyGrade = OnceLock<GradeFx>;

impl FxEngine {
    fn grade_pass(
        &self,
        ctx: &GpuContext,
        pick: impl Fn(&GradeFx) -> &wgpu::ComputePipeline,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        words: &[[f32; 4]],
    ) -> wgpu::Texture {
        let fx = self.grade.get_or_init(|| GradeFx::new(ctx, &self.layout));
        let mut uniform = [[0.0f32; 4]; GRADE_WORDS];
        for (slot, v) in uniform.iter_mut().zip(words) {
            *slot = *v;
        }
        let out = work_texture(ctx, w, h, "fx-grade");
        self.dispatch(
            ctx,
            pick(fx),
            src,
            src,
            &out,
            w,
            h,
            bytemuck::cast_slice(&uniform),
        );
        out
    }

    /// One CDL over a linear working texture, `words` from
    /// `CdlPacked::words`.
    pub fn cdl(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        words: &[[f32; 4]],
    ) -> wgpu::Texture {
        self.grade_pass(ctx, |fx| &fx.cdl, src, w, h, words)
    }

    /// One colour warper over a linear working texture, `words` from
    /// `WarpPacked::words`.
    pub fn colour_warper(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        words: &[[f32; 4]],
    ) -> wgpu::Texture {
        self.grade_pass(ctx, |fx| &fx.warper, src, w, h, words)
    }
}
