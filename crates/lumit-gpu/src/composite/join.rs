//! How a model's picture joins a run of 3D layers: drawn full frame, each
//! pixel at the model's own depth there (`model_join.wgsl`).

use super::{Compositor, CUT_DEPTH, OVER};
use crate::mesh::ModelPicture;
use crate::GpuContext;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// What a model's picture joins a run with: its shader, its layout, and per
/// sample count and format the draw that lays its depth, the two that draw it
/// in front of and behind what the run laid, the two scene depth draws, and
/// the two a holdout draws in front and behind with.
/// Built the first time a model joins, so a comp with none never builds them.
#[derive(Default)]
pub(super) struct Joins {
    shader: OnceLock<(
        wgpu::ShaderModule,
        wgpu::BindGroupLayout,
        wgpu::PipelineLayout,
    )>,
    #[allow(clippy::type_complexity)]
    sets: Mutex<HashMap<(u32, wgpu::TextureFormat), Arc<[wgpu::RenderPipeline; 7]>>>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct JoinUniform {
    matrix: [[f32; 4]; 4],
    params: [f32; 4],
    flags: [f32; 4],
}

impl Compositor {
    fn join_shader(
        &self,
        ctx: &GpuContext,
    ) -> &(
        wgpu::ShaderModule,
        wgpu::BindGroupLayout,
        wgpu::PipelineLayout,
    ) {
        self.joins.shader.get_or_init(|| {
            let shader = ctx
                .device
                .create_shader_module(wgpu::include_wgsl!("../model_join.wgsl"));
            let texture = |binding, filterable| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
            let layout = ctx
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("composite-join"),
                    entries: &[
                        texture(0, true),
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        texture(2, false),
                        texture(4, false),
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });
            let pipeline_layout =
                ctx.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("composite-join"),
                        bind_group_layouts: &[&layout],
                        push_constant_ranges: &[],
                    });
            (shader, layout, pipeline_layout)
        })
    }

    /// The seven join draws at `samples`, built the first time they are asked
    /// for.
    pub(super) fn join_set(
        &self,
        ctx: &GpuContext,
        samples: u32,
    ) -> Arc<[wgpu::RenderPipeline; 7]> {
        self.join_set_as(ctx, samples, ctx.working())
    }

    /// As [`Self::join_set`], drawing into `format`.
    pub(super) fn join_set_as(
        &self,
        ctx: &GpuContext,
        samples: u32,
        format: wgpu::TextureFormat,
    ) -> Arc<[wgpu::RenderPipeline; 7]> {
        let (shader, _, layout) = self.join_shader(ctx);
        let build = || {
            let make = |entry: &str,
                        blend: Option<wgpu::BlendState>,
                        write_mask: wgpu::ColorWrites,
                        depth: Option<(bool, wgpu::CompareFunction)>| {
                ctx.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("composite-join"),
                        layout: Some(layout),
                        vertex: wgpu::VertexState {
                            module: shader,
                            entry_point: Some("vs_join"),
                            buffers: &[],
                            compilation_options: Default::default(),
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: shader,
                            entry_point: Some(entry),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend,
                                write_mask,
                            })],
                            compilation_options: Default::default(),
                        }),
                        primitive: Default::default(),
                        depth_stencil: depth.map(|(write, compare)| wgpu::DepthStencilState {
                            format: CUT_DEPTH,
                            depth_write_enabled: write,
                            depth_compare: compare,
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: wgpu::MultisampleState {
                            count: samples,
                            ..Default::default()
                        },
                        multiview: None,
                        cache: None,
                    })
            };
            use wgpu::{ColorWrites as W, CompareFunction as C};
            let over = Some(wgpu::BlendState {
                color: OVER,
                alpha: OVER,
            });
            let nearest = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Min,
            };
            Arc::new([
                make(
                    "fs_join_cut",
                    None,
                    W::empty(),
                    Some((true, C::GreaterEqual)),
                ),
                make("fs_join", over, W::ALL, Some((false, C::Equal))),
                make("fs_join", over, W::ALL, Some((false, C::Less))),
                make(
                    "fs_join_depth",
                    Some(wgpu::BlendState {
                        color: nearest,
                        alpha: nearest,
                    }),
                    W::ALL,
                    None,
                ),
                make("fs_join_depth", None, W::ALL, None),
                // A holdout puts back what was under the run, so it replaces.
                make("fs_join", None, W::ALL, Some((false, C::Equal))),
                make("fs_join", None, W::ALL, Some((false, C::Less))),
            ])
        };
        match self.joins.sets.lock() {
            Ok(mut sets) => Arc::clone(sets.entry((samples, format)).or_insert_with(build)),
            Err(_) => build(),
        }
    }

    /// A model's picture placed full frame, with `params` and `flags` as
    /// `model_join.wgsl` reads them, and `plate` what was under its run, for
    /// a holdout.
    pub(super) fn join_group(
        &self,
        ctx: &GpuContext,
        model: &ModelPicture,
        matrix: [[f32; 4]; 4],
        params: [f32; 4],
        flags: [f32; 4],
        plate: Option<&wgpu::Texture>,
    ) -> wgpu::BindGroup {
        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            &ctx.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("composite-join"),
                contents: bytemuck::bytes_of(&JoinUniform {
                    matrix,
                    params,
                    flags,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite-join"),
            layout: &self.join_shader(ctx).1,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(&model.colour)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(&model.depth)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&view(
                        plate.unwrap_or(&self.black),
                    )),
                },
            ],
        })
    }
}
