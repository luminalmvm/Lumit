//! Rasterise Turbiniu's coloured triangles, then resolve their coverage.

use super::lens_blur::{bind_layout, scratch, Slot};
use super::{at_depth, work_texture, FxEngine};
use crate::GpuContext;
use wgpu::util::DeviceExt;

const INK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

pub(super) struct TurbiniuPipelines {
    draw_layout: wgpu::BindGroupLayout,
    normal: wgpu::RenderPipeline,
    add: wgpu::RenderPipeline,
    resolve_layout: wgpu::BindGroupLayout,
    resolve: wgpu::ComputePipeline,
}

impl TurbiniuPipelines {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("turbiniu"),
            source: wgpu::ShaderSource::Wgsl(
                at_depth(ctx.working(), include_str!("../fx_turbiniu.wgsl")).into(),
            ),
        });
        let layout = |bind| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("turbiniu"),
                bind_group_layouts: &[bind],
                push_constant_ranges: &[],
            })
        };
        let draw_layout = bind_layout(
            device,
            "turbiniu-draw",
            &[(0, Slot::Uniform)],
            wgpu::ShaderStages::VERTEX,
        );
        let draw = |blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("turbiniu-draw"),
                layout: Some(&layout(&draw_layout)),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("draw_vertex"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 24,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRIBUTES,
                    }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("draw_fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: INK_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let normal = draw(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let add = draw(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        });
        let resolve_layout = bind_layout(
            device,
            "turbiniu-resolve",
            &[
                (0, Slot::Uniform),
                (1, Slot::Tex),
                (2, Slot::Tex),
                (3, Slot::Store(ctx.working())),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let resolve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("turbiniu-resolve"),
            layout: Some(&layout(&resolve_layout)),
            module: &module,
            entry_point: Some("resolve_ink"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            draw_layout,
            normal,
            add,
            resolve_layout,
            resolve,
        }
    }
}

impl FxEngine {
    pub fn turbiniu(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        vertices: &[[f32; 6]],
        additive: bool,
        five_samples: bool,
        mix: f32,
    ) -> wgpu::Texture {
        if mix <= 0.0 {
            return src.clone();
        }
        let (w, h) = (src.width(), src.height());
        let max_dimension = ctx.device.limits().max_texture_dimension_2d;
        let five_samples = five_samples && w.max(h) <= max_dimension / 2;
        let oversample = if five_samples { 2 } else { 1 };
        let fx = self.turbiniu.get_or_init(|| TurbiniuPipelines::new(ctx));
        let buffer = |label, contents: &[u8], usage| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        let params = [
            w as f32,
            h as f32,
            f32::from(five_samples),
            mix,
            f32::from(additive),
            0.0,
            0.0,
            0.0,
        ];
        let params = buffer(
            "turbiniu-params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let geometry = (!vertices.is_empty()).then(|| {
            buffer(
                "turbiniu-vertices",
                bytemuck::cast_slice(vertices),
                wgpu::BufferUsages::VERTEX,
            )
        });
        let ink = scratch(
            ctx,
            w * oversample,
            h * oversample,
            INK_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "turbiniu-ink",
        );
        let out = work_texture(ctx, w, h, "turbiniu-out");
        let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
        let (ink_view, src_view, out_view) = (view(&ink), view(src), view(&out));
        let group = |layout, entries: &[wgpu::BindGroupEntry<'_>]| {
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("turbiniu"),
                layout,
                entries,
            })
        };
        let bind = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let draw_group = group(&fx.draw_layout, &[bind(0, params.as_entire_binding())]);
        let resolve_group = group(
            &fx.resolve_layout,
            &[
                bind(0, params.as_entire_binding()),
                bind(1, wgpu::BindingResource::TextureView(&src_view)),
                bind(2, wgpu::BindingResource::TextureView(&ink_view)),
                bind(3, wgpu::BindingResource::TextureView(&out_view)),
            ],
        );
        let mut enc = ctx.encoder("turbiniu");
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("turbiniu-draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ink_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(if additive { &fx.add } else { &fx.normal });
            pass.set_bind_group(0, &draw_group, &[]);
            if let Some(geometry) = geometry {
                pass.set_vertex_buffer(0, geometry.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("turbiniu-resolve"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&fx.resolve);
            pass.set_bind_group(0, &resolve_group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        out
    }
}
