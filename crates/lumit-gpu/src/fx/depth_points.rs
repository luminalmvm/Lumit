//! Depth points: a picture and its depth drawn as points through a camera, with
//! one over clip w beside the picture for the compositor to join it with the 3D
//! layers by (`fx_depth_points.wgsl`).

use super::{work_texture, FxEngine};
use crate::GpuContext;

/// The format one over clip w is laid down in, as Particulate's in the scene.
const INV_W: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// How a picture's points are placed and seen.
#[derive(Debug, Clone, Copy)]
pub struct DepthPointsOp {
    /// The source camera's eye through the frame's camera, as (X, Y, W).
    pub origin: [f32; 3],
    /// The ray from that eye to the raster's top left corner, through the
    /// frame's camera, and how it moves a raster pixel right and down.
    pub along: [f32; 3],
    pub across: [f32; 3],
    pub down: [f32; 3],
    /// The frame drawn into, px, and its render scale.
    pub target: (u32, u32),
    pub render_scale: f32,
    /// Half a point on the layer's plane, comp pixels.
    pub half: f32,
    /// Depth as a fraction, Density as a fraction, and whether to read the depth
    /// the other way up.
    pub amount: f32,
    pub density: f32,
    pub invert: bool,
    /// The scale and offset that steady the depth.
    pub steady: [f32; 2],
    /// The camera's depth of field, as Particulate's `SceneView::lens`: the blur
    /// in frame pixels per unit of `|distance − focus| / clip w`, nought for none,
    /// the zoom, and the focus distance in comp pixels.
    pub lens: [f32; 3],
    /// One over clip w for whatever lands where no point does.
    pub fallback: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct View {
    origin: [f32; 4],
    along: [f32; 4],
    across: [f32; 4],
    down: [f32; 4],
    frame: [f32; 4],
    look: [f32; 4],
    steady: [f32; 4],
    lens: [f32; 4],
    grid: [u32; 4],
}

/// The layout and pipeline, built the first time a shot is drawn as points.
pub(super) struct Pipeline {
    layout: wgpu::BindGroupLayout,
    draw: wgpu::RenderPipeline,
}

impl Pipeline {
    fn new(ctx: &GpuContext) -> Self {
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fx-depth-points"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../fx_depth_points.wgsl").into()),
            });
        let picture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-depth-points"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    picture(1),
                    picture(2),
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-depth-points"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let target = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let draw = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("fx-depth-points"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("dp_vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("dp_fs"),
                    targets: &[target(ctx.working()), target(INV_W)],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Greater,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });
        Self { layout, draw }
    }
}

/// The step between the columns and rows a Density needs, raster pixels: the
/// widest whose grid holds every pixel `dp_joins` lets in.
fn step_of(density: f32) -> u32 {
    let level = (-0.5 * density.max(1e-9).log2()).floor();
    1 << (level.clamp(0.0, 15.0) as u32)
}

impl FxEngine {
    /// `src`'s points through the camera: the picture at `op.target`'s size,
    /// and one over clip w at each of its pixels. `depth` is at `src`'s size,
    /// and with none the points stay on the layer's plane.
    pub fn depth_points(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        depth: Option<&wgpu::Texture>,
        op: &DepthPointsOp,
    ) -> (wgpu::Texture, wgpu::Texture) {
        let (tw, th) = op.target;
        let out = work_texture(ctx, tw, th, "fx-depth-points");
        let texture = |label, format, usage| {
            ctx.charge_vram(crate::texture_bytes(format, tw, th));
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        use wgpu::TextureUsages as U;
        let inv_w = texture(
            "fx-depth-points-depth",
            INV_W,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let test = texture("fx-depth-points-test", DEPTH, U::RENDER_ATTACHMENT);
        let set = self.depth_points.get_or_init(|| Pipeline::new(ctx));
        let (w, h) = (src.width(), src.height());
        let step = step_of(op.density);
        let (columns, rows) = (w.div_ceil(step), h.div_ceil(step));
        let v3 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        let uniform = wgpu::util::DeviceExt::create_buffer_init(
            &ctx.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("fx-depth-points"),
                contents: bytemuck::bytes_of(&View {
                    origin: v3(op.origin),
                    along: v3(op.along),
                    across: v3(op.across),
                    down: v3(op.down),
                    frame: [tw as f32, th as f32, op.render_scale, op.half],
                    look: [
                        op.amount,
                        op.density,
                        f32::from(u8::from(depth.is_some())),
                        f32::from(u8::from(op.invert)),
                    ],
                    steady: [op.steady[0], op.steady[1], 0.0, 0.0],
                    lens: [op.lens[0], op.lens[1], op.lens[2], 0.0],
                    grid: [columns, step, 0, 0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-depth-points"),
            layout: &set.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view_of(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view_of(depth.unwrap_or(src))),
                },
            ],
        });
        let (out_view, inv_view, test_view) = (view_of(&out), view_of(&inv_w), view_of(&test));
        let attach = |view, clear| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut enc = ctx.encoder("fx-depth-points");
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fx-depth-points"),
            color_attachments: &[
                attach(&out_view, wgpu::Color::TRANSPARENT),
                attach(
                    &inv_view,
                    wgpu::Color {
                        r: f64::from(op.fallback),
                        ..wgpu::Color::TRANSPARENT
                    },
                ),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &test_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if op.density > 0.0 {
            rp.set_pipeline(&set.draw);
            rp.set_bind_group(0, &bind, &[]);
            rp.draw(0..4, 0..columns * rows);
        }
        drop(rp);
        drop(enc);
        (out, inv_w)
    }
}
