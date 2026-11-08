//! Particulate's particles in the comp's 3D scene: worked out by the same
//! passes, then drawn through the comp's camera onto the whole frame, with one
//! over clip w beside the picture for the compositor to join it with the 3D
//! layers by (`fx_particulate_scene.wgsl`).

use super::particulate::{scan_len, ParticulateOp, EVAL_WORKGROUP, SCAN_BLOCK, SCAN_HEAD};
use super::{work_texture, FxEngine};
use crate::GpuContext;

/// [`ParticulateOp::blend`]'s Add and Screen, which the card's own blending
/// draws in any order.
const BLEND_ADD: u32 = 6;
const BLEND_SCREEN: u32 = 8;

/// How the particles are seen: the frame drawn into, and the numbers the
/// kernel turns its own foreshortening into distances with.
#[derive(Debug, Clone, Copy)]
pub struct SceneView<'a> {
    /// The frame drawn into, px: the comp at the render scale.
    pub target: (u32, u32),
    /// One over clip w is this times the projection's own foreshortening.
    pub k: f32,
    /// One over the camera's zoom times the render scale.
    pub soft: f32,
    /// Clip w of the nearest 3D layer at each pixel, at `target`'s size, less
    /// `near`.
    pub occluder: Option<&'a wgpu::Texture>,
    pub near: f32,
    /// One over clip w for whatever lands where no particle does, such as a
    /// glow added after this: the middle of the cloud.
    pub fallback: f32,
    /// The camera's depth of field: the blur in raster pixels per unit of
    /// `|distance − focus| / clip w`, nought for none, the zoom, and the focus
    /// distance in comp pixels.
    pub lens: [f32; 3],
    /// How much the layer's scale sizes each particle.
    pub scale: f32,
    /// Whether each particle is drawn across the shutter, its tail where it
    /// was when the shutter opened.
    pub blur: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniform {
    target: [f32; 2],
    k: f32,
    soft: f32,
    flags: [f32; 4],
    lens: [f32; 4],
    look: [f32; 4],
}

/// The scene draw's layout and pipelines, built the first time particles are
/// put in a scene.
pub(super) struct ScenePipelines {
    layout: wgpu::BindGroupLayout,
    keys: wgpu::ComputePipeline,
    normal: wgpu::RenderPipeline,
    add: wgpu::RenderPipeline,
    screen: wgpu::RenderPipeline,
    depth: wgpu::RenderPipeline,
}

/// The format one over clip w is laid down in.
const INV_W: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl ScenePipelines {
    fn new(fx: &FxEngine, ctx: &GpuContext) -> Self {
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fx-particulate-scene"),
                source: wgpu::ShaderSource::Wgsl(
                    [
                        include_str!("../fx_noise_core.wgsl"),
                        include_str!("../fx_blend_core.wgsl"),
                        include_str!("../fx_particulate.wgsl"),
                        include_str!("../fx_particulate_scene.wgsl"),
                    ]
                    .concat()
                    .into(),
                ),
            });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        use wgpu::ShaderStages as S;
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-scene"),
                entries: &[
                    entry(
                        0,
                        S::VERTEX_FRAGMENT,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                    ),
                    entry(
                        1,
                        S::VERTEX,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                    ),
                    entry(
                        2,
                        S::FRAGMENT,
                        wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                    ),
                ],
            });
        let (empty, draw) = (&fx.particulate_empty_layout, &fx.particulate_draw_layout);
        let keys_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-scene-keys"),
                bind_group_layouts: &[empty, draw, &fx.particulate_blend.tiles_layout],
                push_constant_ranges: &[],
            });
        let draw_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-scene-draw"),
                bind_group_layouts: &[empty, draw, empty, &layout],
                push_constant_ranges: &[],
            });
        let render = |fs: &str, format, blend, depth: bool| {
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("fx-particulate-scene"),
                    layout: Some(&draw_pl),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("pt_vs_scene"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(fs),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: depth.then_some(wgpu::DepthStencilState {
                        format: DEPTH,
                        depth_write_enabled: true,
                        depth_compare: wgpu::CompareFunction::Greater,
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                })
        };
        use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
        let both = |src, dst| C {
            src_factor: src,
            dst_factor: dst,
            operation: O::Add,
        };
        let working = ctx.working();
        Self {
            keys: ctx
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("fx-particulate-scene-keys"),
                    layout: Some(&keys_pl),
                    module: &module,
                    entry_point: Some("pt_scene_keys"),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            normal: render(
                "pt_fs_scene",
                working,
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                false,
            ),
            add: render(
                "pt_fs_scene",
                working,
                Some(wgpu::BlendState {
                    color: both(F::One, F::One),
                    alpha: both(F::One, F::One),
                }),
                false,
            ),
            // Premultiplied screen is s + d − s·d.
            screen: render(
                "pt_fs_scene",
                working,
                Some(wgpu::BlendState {
                    color: both(F::One, F::OneMinusSrc),
                    alpha: both(F::One, F::OneMinusSrcAlpha),
                }),
                false,
            ),
            depth: render("pt_fs_scene_depth", INV_W, None, true),
            layout,
        }
    }
}

impl FxEngine {
    /// One Particulate's particles through the comp's camera: the picture at
    /// `view.target`'s size, and one over clip w at each of its pixels. `src`,
    /// `w` and `h` are the layer's own picture and raster, which the particles
    /// are worked out in, and `op.projection` takes them to the frame.
    ///
    /// Normal particles are sorted far to near on the card, Add and Screen
    /// ones are drawn as they come, and every other blend draws as Normal.
    pub fn particulate_scene(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        (w, h): (u32, u32),
        sprite: Option<&wgpu::Texture>,
        op: &ParticulateOp<'_>,
        view: &SceneView<'_>,
    ) -> (wgpu::Texture, wgpu::Texture) {
        let (tw, th) = view.target;
        let out = work_texture(ctx, tw, th, "fx-particulate-scene");
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
            "fx-particulate-scene-depth",
            INV_W,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let depth = texture("fx-particulate-scene-test", DEPTH, U::RENDER_ATTACHMENT);
        let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (out_view, inv_view) = (view_of(&out), view_of(&inv_w));
        let mut enc = ctx.encoder("fx-particulate-scene");
        let clear = |enc: &mut wgpu::CommandEncoder, target, colour| {
            drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx-particulate-scene-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(colour),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            }));
        };
        let fallback = wgpu::Color {
            r: f64::from(view.fallback),
            ..wgpu::Color::TRANSPARENT
        };
        let Some(pass) = self.particulate_prepare(ctx, src, w, h, sprite, op) else {
            clear(&mut enc, &out_view, wgpu::Color::TRANSPARENT);
            clear(&mut enc, &inv_view, fallback);
            return (out, inv_w);
        };
        let set = self
            .particulate_scene
            .get_or_init(|| ScenePipelines::new(self, ctx));
        pass.evaluate(self, &mut enc);
        let storage = |label, bytes: u64| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.max(16),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let sorted = op.blend != BLEND_ADD && op.blend != BLEND_SCREEN;
        let pairs = u64::from(op.cap.next_multiple_of(SCAN_BLOCK)) * 8;
        let (a, b) = if sorted {
            (
                storage("fx-particulate-scene-a", pairs),
                storage("fx-particulate-scene-b", pairs),
            )
        } else {
            (storage("fx-particulate-scene-a", 0), storage("", 0))
        };
        if sorted {
            // Far to near by the tiles' own stable sort, eight bits a pass.
            let len = scan_len(op.cap, w, h);
            let scan = storage(
                "fx-particulate-scene-scan",
                (u64::from(SCAN_HEAD) + u64::from(len) + u64::from(len.div_ceil(SCAN_BLOCK))) * 4,
            );
            // The tiles' own picture, particle rows and base, which the sort
            // never reads.
            let dummy = storage("", 0);
            let picture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d::default(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: U::STORAGE_BINDING,
                view_formats: &[],
            });
            let base = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d::default(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: U::TEXTURE_BINDING,
                view_formats: &[],
            });
            let blend = &self.particulate_blend;
            let bind = |from: &wgpu::Buffer, to: &wgpu::Buffer| {
                ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fx-particulate-scene-sort"),
                    layout: &blend.tiles_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: pass.args.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: dummy.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&view_of(&picture)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: from.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: to.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: scan.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(&view_of(&base)),
                        },
                    ],
                })
            };
            let (f, r) = (&bind(&a, &b), &bind(&b, &a));
            let numbers = (op.cap.next_multiple_of(SCAN_BLOCK)).div_ceil(SCAN_BLOCK);
            let t = &blend.tiles;
            let steps = [
                (f, &set.keys, op.cap.div_ceil(EVAL_WORKGROUP)),
                (f, &t.hist_lo, numbers),
                (f, &t.scan_block, numbers),
                (f, &t.scan_sums, 1),
                (f, &t.scan_add, numbers),
                (f, &t.place_lo, numbers),
                (r, &t.hist_hi, numbers),
                (r, &t.scan_block, numbers),
                (r, &t.scan_sums, 1),
                (r, &t.scan_add, numbers),
                (r, &t.place_hi, numbers),
            ];
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-particulate-scene-sort"),
                timestamp_writes: None,
            });
            cp.set_bind_group(0, &pass.empty, &[]);
            cp.set_bind_group(1, &pass.draw_bind, &[]);
            for (group, pipeline, x) in steps {
                cp.set_bind_group(2, group, &[]);
                cp.set_pipeline(pipeline);
                cp.dispatch_workgroups(x, 1, 1);
            }
        }
        let uniform = wgpu::util::DeviceExt::create_buffer_init(
            &ctx.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-scene"),
                contents: bytemuck::bytes_of(&SceneUniform {
                    target: [tw as f32, th as f32],
                    k: view.k,
                    soft: view.soft,
                    flags: [
                        f32::from(u8::from(sorted)),
                        f32::from(u8::from(view.occluder.is_some())),
                        view.near,
                        0.0,
                    ],
                    lens: [view.lens[0], view.lens[1], view.lens[2], 0.0],
                    look: [view.scale, f32::from(u8::from(view.blur)), 0.0, 0.0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let occluder = view_of(view.occluder.unwrap_or(src));
        let scene = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-scene"),
            layout: &set.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: a.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&occluder),
                },
            ],
        });
        let colour = match op.blend {
            BLEND_ADD => &set.add,
            BLEND_SCREEN => &set.screen,
            _ => &set.normal,
        };
        let depth_view = view_of(&depth);
        for (pipeline, target, load, test) in [
            (colour, &out_view, wgpu::Color::TRANSPARENT, None),
            (&set.depth, &inv_view, fallback, Some(&depth_view)),
        ] {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx-particulate-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(load),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: test.map(|view| wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, &pass.empty, &[]);
            rp.set_bind_group(1, &pass.draw_bind, &[]);
            rp.set_bind_group(2, &pass.empty, &[]);
            rp.set_bind_group(3, &scene, &[]);
            rp.draw_indirect(&pass.args, 0);
        }
        drop(enc);
        (out, inv_w)
    }

    /// The first `n` particles' place, size, turn, colour and id, read back for
    /// drawing a model at each, with the live count. Laid out as the stream
    /// is, each range cut to `n`, so the cost follows `n` and not the cap.
    pub fn particulate_copies(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        (w, h): (u32, u32),
        op: &ParticulateOp<'_>,
        n: u32,
    ) -> Option<(u32, Vec<u32>)> {
        let pass = self.particulate_prepare(ctx, src, w, h, None, op)?;
        let (n, cap) = (u64::from(n.min(op.cap)), u64::from(op.cap));
        // (first word in the stream, words) for position, size, turn, colour
        // and id.
        let ranges = [
            (0, 3 * n),
            (8 * cap, n),
            (9 * cap, n),
            (10 * cap, 2 * n),
            (12 * cap, 2 * n),
        ];
        let words: u64 = ranges.iter().map(|r| r.1).sum();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-copies"),
            size: 16 + words * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        {
            let mut enc = ctx.encoder("fx-particulate-copies");
            pass.evaluate(self, &mut enc);
            enc.copy_buffer_to_buffer(&pass.args, 0, &staging, 0, 16);
            let mut at = 16;
            for (from, len) in ranges {
                enc.copy_buffer_to_buffer(&pass.stream, from * 4, &staging, at, len * 4);
                at += len * 4;
            }
        }
        ctx.flush();
        // The browser build answers with what an earlier frame left.
        let read: Vec<u32> =
            crate::device::read_buffer(ctx, ("particulate-copies", n, w, h), &staging)?;
        Some((*read.get(1)?, read.get(4..)?.to_vec()))
    }
}
