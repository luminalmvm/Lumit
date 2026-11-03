//! A model's motion blur: each shutter sample drawn on its own picture and
//! added to the frame's at its share.

use super::{MeshFx, COLOUR};
use crate::GpuContext;

/// The pipeline that adds a sample's picture at the blend constant.
pub(super) fn add_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    let shares = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Constant,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh-add"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_full"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_add"),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOUR,
                blend: Some(wgpu::BlendState {
                    color: shares,
                    alpha: shares,
                }),
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
}

/// One sample's picture, and how much of the frame it makes.
pub(super) struct Shutter {
    pub(super) view: wgpu::TextureView,
    group: wgpu::BindGroup,
    share: f64,
}

impl Shutter {
    /// A sample picture `size` in pixels, for `samples` of them, `None` for
    /// one, which draws straight into the frame's picture.
    pub(super) fn new(
        ctx: &GpuContext,
        fx: &MeshFx,
        (width, height): (u32, u32),
        samples: usize,
    ) -> Option<Self> {
        if samples < 2 {
            return None;
        }
        ctx.charge_vram(crate::texture_bytes(COLOUR, width, height));
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh-sample"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOUR,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-add"),
            layout: &fx.down.0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&fx.down.2),
                },
            ],
        });
        Some(Self {
            view,
            group,
            share: 1.0 / samples as f64,
        })
    }

    /// Add sample `k`'s picture to `colour`, the first clearing it.
    pub(super) fn add(
        &self,
        fx: &MeshFx,
        encoder: &mut wgpu::CommandEncoder,
        colour: &wgpu::TextureView,
        k: usize,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mesh-add"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: colour,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if k == 0 {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&fx.add);
        let s = self.share;
        pass.set_blend_constant(wgpu::Color {
            r: s,
            g: s,
            b: s,
            a: s,
        });
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}
