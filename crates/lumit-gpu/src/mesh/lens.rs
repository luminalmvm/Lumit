//! The camera's depth of field on a model's picture: each pixel blurred by how
//! far its own depth is off the focus, with the blur a 3D layer takes, and the
//! depth spread to the fringe so it still meets the scene.

use super::{ModelPicture, COLOUR, INV_W};
use crate::fx::blur::BlurOp;
use crate::fx::FxEngine;
use crate::GpuContext;
use wgpu::util::DeviceExt;

/// How one picture is blurred.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelLens {
    /// A depth's blur, in texels: this times its distance off `focus` over
    /// its distance from the eye.
    pub reach: f32,
    pub focus: f32,
    /// The widest blur in the picture, in texels.
    pub cap: f32,
    /// Distance from the eye for each unit of clip w.
    pub z_per_w: f32,
}

/// The passes, made the first time a model is blurred.
pub(super) struct LensPasses {
    layout: wgpu::BindGroupLayout,
    spread: wgpu::RenderPipeline,
    wider: wgpu::RenderPipeline,
    depth_picture: wgpu::RenderPipeline,
    depth: wgpu::RenderPipeline,
}

impl LensPasses {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh-lens"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../mesh_lens.wgsl").into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh-lens"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh-lens"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let make = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh-lens"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_lens"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(format.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        Self {
            spread: make("fs_spread", COLOUR),
            wider: make("fs_wider", COLOUR),
            depth_picture: make("fs_depth_picture", COLOUR),
            depth: make("fs_depth", INV_W),
            layout,
        }
    }
}

/// One full-picture pass of `pipeline` over `a` and `b`, into a new picture.
fn pass(
    ctx: &GpuContext,
    fx: &LensPasses,
    pipeline: &wgpu::RenderPipeline,
    (a, b): (&wgpu::Texture, &wgpu::Texture),
    uniform: &wgpu::Buffer,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    use wgpu::TextureUsages as U;
    let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mesh-lens"),
        size: a.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC,
        view_formats: &[],
    });
    ctx.charge_vram(crate::texture_bytes(format, a.width(), a.height()));
    let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
    let (va, vb) = (view_of(a), view_of(b));
    let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mesh-lens"),
        layout: &fx.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&va),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&vb),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    });
    let mut encoder = ctx.encoder("mesh-lens");
    {
        let target = view_of(&out);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mesh-lens"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    drop(encoder);
    out
}

impl FxEngine {
    /// `picture` blurred by `lens`, its depth grown to cover the blur.
    #[must_use]
    pub fn model_lens(
        &self,
        ctx: &GpuContext,
        picture: &ModelPicture,
        lens: &ModelLens,
    ) -> ModelPicture {
        let mesh = self.mesh.get_or_init(|| super::MeshFx::new(ctx));
        let fx = mesh.lens.get_or_init(|| LensPasses::new(&ctx.device));
        let uniform = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-lens"),
                contents: bytemuck::bytes_of(lens),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let (w, h) = (picture.colour.width(), picture.colour.height());
        let blur = |src: &wgpu::Texture, matte: Option<&wgpu::Texture>| {
            let op = BlurOp {
                radius_px: lens.cap,
                // A model wider than the frame carries on past its edge.
                edge: 1,
                mix: 1.0,
            };
            self.blur(ctx, src, w, h, matte, &op)
        };
        let run = |pipeline, a, b, format| pass(ctx, fx, pipeline, (a, b), &uniform, format);
        let own = &picture.depth;
        let spread = run(&fx.spread, own, own, COLOUR);
        let near = blur(&spread, None);
        let matte = run(&fx.wider, &spread, &near, COLOUR);
        let colour = blur(&picture.colour, Some(&matte));
        let depth_picture = run(&fx.depth_picture, own, own, COLOUR);
        let depth_near = blur(&depth_picture, Some(&matte));
        let depth = run(&fx.depth, own, &depth_near, INV_W);
        ModelPicture {
            colour,
            depth,
            holdout: picture.holdout,
        }
    }
}
