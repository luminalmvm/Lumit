//! Sprite flare's passes (`fx_sprite_flare.wgsl`): every instance on every
//! light drawn as one instanced quad, added into a half-float picture, then
//! that picture added to the layer.

use std::sync::OnceLock;

use crate::GpuContext;

use super::lens_blur::{bind_layout, scratch, Slot};
use super::{work_texture, FxEngine};

/// Floats in one instance, as `lumit_core::fx::flare::instances` packs it.
/// This crate never depends on `lumit-core`, so the shape is restated.
pub const INSTANCE_FLOATS: usize = 36;

/// One frame's flare, already expanded.
#[derive(Debug, Clone, Copy)]
pub struct SpriteFlareOp<'a> {
    pub instances: &'a [[f32; INSTANCE_FLOATS]],
    /// Each light: x, y and gain, then a 0, then its colour and a 0.
    pub lights: &'a [[f32; 8]],
    /// Raster pixels.
    pub centre: [f32; 2],
    /// Raster pixels; 0 is no fade.
    pub edge_fade: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    w: f32,
    h: f32,
    centre: [f32; 2],
    edge_fade: f32,
    lights: u32,
    _pad: [f32; 2],
}

pub(super) struct SpriteFlareFx {
    draw: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    add: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
}

/// What the quads are added into before they meet the layer: blendable on
/// every card, whatever the project's depth.
const LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl SpriteFlareFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-sprite-flare"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_sprite_flare.wgsl").into()),
        });
        let draw_bgl = bind_layout(
            device,
            "fx-sprite-flare-draw",
            &[(0, Slot::Uniform), (1, Slot::Spans), (2, Slot::Spans)],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let layout = |bgl: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-sprite-flare"),
                bind_group_layouts: &[bgl],
                push_constant_ranges: &[],
            })
        };
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fx-sprite-flare-draw"),
            layout: Some(&layout(&draw_bgl)),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("sf_vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("sf_fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: LIGHT_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: add,
                        alpha: add,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            // Never multisampled: draw order alone fixes the sum, so a frame
            // adds up the same every time.
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let add_bgl = bind_layout(
            device,
            "fx-sprite-flare-add",
            &[
                (3, Slot::Tex),
                (4, Slot::Tex),
                (5, Slot::Store(ctx.working())),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let add_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fx-sprite-flare-add"),
            layout: Some(&layout(&add_bgl)),
            module: &module,
            entry_point: Some("sf_add"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            draw: (draw_bgl, draw),
            add: (add_bgl, add_pipe),
        }
    }
}

/// Built on first use: most projects never draw a Sprite flare.
pub(super) type LazySpriteFlare = OnceLock<SpriteFlareFx>;

impl FxEngine {
    /// The flare in `op` added to `src`, alpha untouched. With nothing to
    /// draw it hands `src` back as it is.
    pub fn sprite_flare(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SpriteFlareOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        use wgpu::TextureUsages as U;
        let count = op.instances.len() * op.lights.len();
        let Ok(count) = u32::try_from(count) else {
            return src.clone();
        };
        if count == 0 || w == 0 || h == 0 {
            return src.clone();
        }
        let fx = self.sprite_flare.get_or_init(|| SpriteFlareFx::new(ctx));
        let buffer = |label, contents: &[u8], usage| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        let params = buffer(
            "fx-sprite-flare-params",
            bytemuck::bytes_of(&Params {
                w: w as f32,
                h: h as f32,
                centre: op.centre,
                edge_fade: op.edge_fade,
                lights: op.lights.len() as u32,
                _pad: [0.0; 2],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let instances = buffer(
            "fx-sprite-flare-instances",
            bytemuck::cast_slice(op.instances),
            wgpu::BufferUsages::STORAGE,
        );
        let lights = buffer(
            "fx-sprite-flare-lights",
            bytemuck::cast_slice(op.lights),
            wgpu::BufferUsages::STORAGE,
        );
        let light = scratch(
            ctx,
            w,
            h,
            LIGHT_FORMAT,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
            "fx-sprite-flare-light",
        );
        let out = work_texture(ctx, w, h, "fx-sprite-flare-out");
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (v_src, v_light, v_out) = (view(src), view(&light), view(&out));
        let draw_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-sprite-flare-draw"),
            layout: &fx.draw.0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lights.as_entire_binding(),
                },
            ],
        });
        let texture = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let add_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-sprite-flare-add"),
            layout: &fx.add.0,
            entries: &[texture(3, &v_src), texture(4, &v_light), texture(5, &v_out)],
        });
        let mut enc = ctx.encoder("fx-sprite-flare");
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fx-sprite-flare-draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &v_light,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        rp.set_pipeline(&fx.draw.1);
        rp.set_bind_group(0, &draw_group, &[]);
        rp.draw(0..6, 0..count);
        drop(rp);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-sprite-flare-add"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&fx.add.1);
        pass.set_bind_group(0, &add_group, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        drop(pass);
        drop(enc);
        out
    }
}
