//! Scene passes: which way each pixel of a 3D scene faces, where it is and
//! which part it belongs to, or a matte of the parts asked for. Models and 3D
//! layers are drawn once more on their own road, so a normal model draw is
//! untouched and only a frame that reads a pass pays for one.

use super::look::Look;
use super::setup::{texture_entry, uniform_entry};
use super::{DrawUniform, GpuModel, Vertex, DEPTH};
use crate::composite::scaled_size;
use crate::fx::FxEngine;
use crate::GpuContext;
use glam::{Mat3, Mat4, Vec3};
use lumit_scene::pose::Pose;
use lumit_scene::ModelScene;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wgpu::util::DeviceExt;

/// What a pass writes at each pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassLook {
    /// Which way it faces, each axis from nought to one with a half for none.
    Normal,
    /// Where it is, metres.
    Position,
    /// Its part's number as a colour, a byte to a channel as it shows.
    PartId,
    /// White over the parts asked for. Their edges are soft, as it is drawn
    /// with as many samples as the card has.
    Matte,
}

/// A 3D layer in a pass: its picture, whose alpha cuts it, and where it is.
pub struct PassPlane<'a> {
    pub texture: &'a wgpu::Texture,
    /// Its unit square to comp pixels.
    pub world: Mat4,
    pub opacity: f32,
    pub id: u32,
    /// Which group of 3D layers it is in, counted up from the bottom.
    pub group: u32,
}

/// A Model layer in a pass.
pub struct PassModel<'a> {
    pub scene: &'a Arc<ModelScene>,
    /// Model space to comp pixels.
    pub model_to_comp: Mat4,
    /// The layer's own id, which each part's number is made from.
    pub layer: [u8; 16],
    /// Its materials as its layer's rows have them this frame.
    pub looks: &'a [Look],
    /// Its pose this frame, which is empty when it draws as its file has it.
    pub poses: &'a [Pose],
    /// Which group of 3D layers it is in, counted up from the bottom.
    pub group: u32,
}

/// One pass of one scene.
pub struct PassView<'a> {
    pub comp: (u32, u32),
    pub render_scale: f32,
    /// The comp's camera, as [`crate::composite::camera_matrix`] makes it.
    pub camera: Mat4,
    pub eye: Vec3,
    /// Comp pixels to the metres Position and Normal are read in.
    pub to_space: Mat4,
    pub look: PassLook,
    /// The parts a matte shows.
    pub parts: &'a [u32],
    pub planes: &'a [PassPlane<'a>],
    pub models: &'a [PassModel<'a>],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PassFrame {
    clip: [[f32; 4]; 4],
    to_space: [[f32; 4]; 4],
    eye: [f32; 4],
}

/// A matte's multisample colour and every pass's depth, kept from one pass to
/// the next of the same shape, as they are large.
struct PassScratch {
    shape: (u32, u32, u32, wgpu::TextureFormat),
    msaa: Option<wgpu::Texture>,
    z: wgpu::Texture,
    held: Option<lumit_budget::Reservation>,
}

/// The passes' shader and layouts, made the first time a pass is asked for.
pub(super) struct PassFx {
    shader: wgpu::ShaderModule,
    frame_layout: wgpu::BindGroupLayout,
    plane_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// By sample count, format and kind ([`PLANE`] or whether a model's part
    /// is double-sided).
    sets: Mutex<HashMap<(u32, wgpu::TextureFormat, usize), Arc<wgpu::RenderPipeline>>>,
    scratch: Mutex<Option<PassScratch>>,
}

/// The pipeline kind that draws a 3D layer.
const PLANE: usize = 2;

impl PassFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh-pass"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../mesh_pass.wgsl").into()),
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh-pass-frame"),
            entries: &[uniform_entry(0, false), uniform_entry(1, true)],
        });
        let plane_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh-pass-plane"),
            entries: &[
                texture_entry(
                    20,
                    wgpu::TextureSampleType::Float { filterable: true },
                    false,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 21,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mesh-pass-plane"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            shader,
            frame_layout,
            plane_layout,
            sampler,
            sets: Mutex::new(HashMap::new()),
            scratch: Mutex::new(None),
        }
    }

    fn pipeline(
        &self,
        device: &wgpu::Device,
        model_layouts: [&wgpu::BindGroupLayout; 2],
        (samples, format, kind): (u32, wgpu::TextureFormat, usize),
    ) -> Arc<wgpu::RenderPipeline> {
        let key = (samples, format, kind);
        if let Some(kept) = self.sets.lock().ok().and_then(|s| s.get(&key).cloned()) {
            return kept;
        }
        // Made with the lock let go, as making one takes a while.
        let made = {
            let plane = kind == PLANE;
            let groups = [&self.frame_layout, model_layouts[0], model_layouts[1]];
            let plane_groups = [&self.frame_layout, &self.plane_layout];
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mesh-pass"),
                bind_group_layouts: if plane { &plane_groups } else { &groups },
                push_constant_ranges: &[],
            });
            let attributes = wgpu::vertex_attr_array![
                0 => Float32x3, 1 => Float32x3, 2 => Float32x4,
                3 => Float32x2, 4 => Float32x2, 5 => Float32x4,
            ];
            let vertex_buffers = [wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            }];
            Arc::new(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("mesh-pass"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &self.shader,
                        entry_point: Some(if plane { "vs_plane" } else { "vs_model" }),
                        buffers: if plane { &[] } else { &vertex_buffers },
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some(if plane { "fs_plane" } else { "fs_model" }),
                        targets: &[Some(format.into())],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: (kind == 0).then_some(wgpu::Face::Back),
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH,
                        depth_write_enabled: true,
                        depth_compare: wgpu::CompareFunction::GreaterEqual,
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: samples,
                        // A soft picture's alpha is how many samples it takes.
                        alpha_to_coverage_enabled: samples > 1,
                        ..Default::default()
                    },
                    multiview: None,
                    cache: None,
                }),
            )
        };
        match self.sets.lock() {
            Ok(mut sets) => Arc::clone(sets.entry(key).or_insert(made)),
            Err(_) => made,
        }
    }
}

/// One draw: its uniform's place, then a 3D layer's picture, or which model
/// and which of its parts and whether that part shows both sides.
enum PassDraw<'a> {
    Plane(u32, &'a wgpu::Texture),
    Part(u32, usize, usize, bool),
}

/// A model on the card and its materials as its layer has them this frame, each
/// with what it blends, whether it shows both sides and its samplers.
type Drawn<'a> = (
    &'a PassModel<'a>,
    Arc<GpuModel>,
    Vec<(wgpu::BindGroup, bool, bool, wgpu::BindGroup)>,
);

impl FxEngine {
    /// One pass of a scene at render size, in the working format, or sixteen
    /// bits for a matte, whose samples a wider format may not have. A model
    /// the card can't hold is left out of it. `None`, with nothing made, for
    /// a matte of parts none of which is there.
    pub fn scene_pass(&self, ctx: &GpuContext, view: &PassView<'_>) -> Option<wgpu::Texture> {
        let mesh = self.mesh.get_or_init(|| super::MeshFx::new(ctx));
        let fx = mesh.passes.get_or_init(|| PassFx::new(ctx));

        // Every draw's uniform in one buffer, and what each one draws.
        let size = std::mem::size_of::<DrawUniform>();
        let stride = wgpu::util::align_to(
            size as u64,
            u64::from(ctx.device.limits().min_uniform_buffer_offset_alignment),
        ) as usize;
        let mut uniforms: Vec<u8> = Vec::new();
        let mut draws: Vec<(u32, PassDraw<'_>)> = Vec::new();
        let mut found = false;
        let mut push = |at: Mat4, n: Mat3, first_w: f32, id: u32| {
            let offset = uniforms.len();
            uniforms.resize(offset + stride, 0);
            let shown = view.parts.contains(&id);
            let uniform = DrawUniform {
                model: at.to_cols_array_2d(),
                normal: [
                    n.x_axis.extend(first_w).to_array(),
                    n.y_axis.extend(id as f32).to_array(),
                    n.z_axis.extend(f32::from(u8::from(shown))).to_array(),
                ],
                rest: [[0.0; 4]; 4],
            };
            uniforms[offset..][..size].copy_from_slice(bytemuck::bytes_of(&uniform));
            offset as u32
        };
        for plane in view.planes {
            found |= view.parts.contains(&plane.id);
            let offset = push(plane.world, Mat3::IDENTITY, plane.opacity, plane.id);
            draws.push((plane.group, PassDraw::Plane(offset, plane.texture)));
        }
        // All the models first, as a pose borrows its model.
        let models: Vec<Drawn<'_>> = view
            .models
            .iter()
            .filter_map(|m| {
                let model = mesh.model_for(ctx, m.scene).ok()?;
                let materials = mesh.looked(ctx, &model, m.looks, None);
                Some((m, model, materials))
            })
            .collect();
        let posed: Vec<_> = models
            .iter()
            .map(|(m, model, _)| mesh.posed(ctx, model, m.poses))
            .collect();
        for (which, ((m, model, materials), posed)) in models.iter().zip(&posed).enumerate() {
            for (k, p) in model.parts.iter().enumerate() {
                // A part hidden this frame is still there to be kept.
                let id = m.scene.part_number(&m.layer, p.index, p.material);
                found |= view.parts.contains(&id);
                if posed.hidden(p, 0) {
                    continue;
                }
                let at = m.model_to_comp * posed.node(p, 0);
                let m3 = Mat3::from_mat4(at);
                let det = m3.determinant();
                if !det.is_normal() {
                    continue;
                }
                let mirrored = det < 0.0;
                let offset = push(
                    at,
                    m3.inverse().transpose(),
                    f32::from(u8::from(mirrored)),
                    id,
                );
                // ponytail: a mirrored part is drawn from both sides, as the
                // mesh pass does.
                let both = mirrored || materials.get(p.material).is_some_and(|m| m.2);
                draws.push((m.group, PassDraw::Part(offset, which, k, both)));
            }
        }
        let matte = view.look == PassLook::Matte;
        if matte && !found {
            return None;
        }
        if uniforms.is_empty() {
            uniforms.resize(stride, 0);
        }
        // A group is drawn over the groups below it, as it is in the picture.
        draws.sort_by_key(|(group, _)| *group);

        let (w, h) = view.comp;
        let (tw, th) = scaled_size(w, h, view.render_scale);
        let samples = if matte {
            ctx.cut_sample_count(8).max(1)
        } else {
            1
        };
        let format = if matte { super::COLOUR } else { ctx.working() };
        let texture = |label, format, samples: u32, usage| {
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        use wgpu::TextureUsages as U;
        ctx.charge_vram(crate::texture_bytes(format, tw, th));
        let target = texture(
            "mesh-pass",
            format,
            1,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC | U::COPY_DST,
        );
        let shape = (tw, th, samples, format);
        let kept = fx.scratch.lock().ok().and_then(|mut k| k.take());
        let scratch = match kept.filter(|k| k.shape == shape) {
            Some(kept) => kept,
            None => {
                let colour = if samples > 1 {
                    crate::texture_bytes(format, tw, th)
                } else {
                    0
                };
                let wanted = (colour + crate::texture_bytes(DEPTH, tw, th)) * u64::from(samples);
                let held = ctx.ledger().reserve(lumit_budget::Tier::Vram, wanted).ok();
                if held.is_none() {
                    ctx.charge_vram(wanted);
                }
                PassScratch {
                    shape,
                    msaa: (samples > 1)
                        .then(|| texture("mesh-pass-msaa", format, samples, U::RENDER_ATTACHMENT)),
                    z: texture("mesh-pass-depth", DEPTH, samples, U::RENDER_ATTACHMENT),
                    held,
                }
            }
        };

        // Comp pixels to clip space, as the mesh pass maps them.
        let clip = Mat4::from_translation(Vec3::new(-1.0, 1.0, 0.0))
            * Mat4::from_scale(Vec3::new(
                2.0 / w.max(1) as f32,
                -2.0 / h.max(1) as f32,
                0.0,
            ))
            * view.camera;
        let what = match view.look {
            PassLook::Normal => 0.0,
            PassLook::Position => 1.0,
            PassLook::PartId => 2.0,
            PassLook::Matte if samples > 1 => 3.0,
            PassLook::Matte => 4.0,
        };
        let frame = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-pass-frame"),
                contents: bytemuck::bytes_of(&PassFrame {
                    clip: clip.to_cols_array_2d(),
                    to_space: view.to_space.to_cols_array_2d(),
                    eye: view.eye.extend(what).to_array(),
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let uniforms = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-pass-draws"),
                contents: &uniforms,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let frame_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-pass-frame"),
            layout: &fx.frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(size as u64),
                    }),
                },
            ],
        });
        // Bind groups first, as a pass mustn't outlive what it borrows.
        let plane_groups: Vec<Option<wgpu::BindGroup>> = draws
            .iter()
            .map(|(_, d)| match d {
                PassDraw::Plane(_, picture) => {
                    Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mesh-pass-plane"),
                        layout: &fx.plane_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 20,
                                resource: wgpu::BindingResource::TextureView(
                                    &picture.create_view(&Default::default()),
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 21,
                                resource: wgpu::BindingResource::Sampler(&fx.sampler),
                            },
                        ],
                    }))
                }
                PassDraw::Part(..) => None,
            })
            .collect();
        let layouts = [&mesh.material_layout, &mesh.samplers.layout];
        let pipeline = |kind| fx.pipeline(&ctx.device, layouts, (samples, format, kind));
        let pipelines = [pipeline(0), pipeline(1), pipeline(PLANE)];

        let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (target_view, z_view) = (view_of(&target), view_of(&scratch.z));
        let msaa_view = scratch.msaa.as_ref().map(view_of);
        let mut encoder = ctx.encoder("mesh-pass");
        // A render pass a group, its depth cleared and what's below kept. With
        // no draws one still runs, for the clear.
        let mut at = 0;
        loop {
            let group = draws.get(at).map(|(group, _)| *group);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view.as_ref().unwrap_or(&target_view),
                    resolve_target: msaa_view.as_ref().map(|_| &target_view),
                    ops: wgpu::Operations {
                        load: if at == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &z_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            // What's bound starts again with each pass, and with each model.
            let (mut rest, mut bound) = (false, None);
            while let Some((_, d)) = draws.get(at).filter(|(g, _)| Some(*g) == group) {
                match (d, &plane_groups[at]) {
                    (PassDraw::Plane(offset, _), Some(picture)) => {
                        pass.set_pipeline(&pipelines[PLANE]);
                        pass.set_bind_group(0, &frame_group, &[*offset]);
                        pass.set_bind_group(1, picture, &[]);
                        pass.draw(0..6, 0..1);
                    }
                    (PassDraw::Part(offset, m, k, both), _) => {
                        let drawn = models.get(*m);
                        let p = drawn.and_then(|(_, model, _)| model.parts.get(*k));
                        let material = drawn
                            .zip(p)
                            .and_then(|((_, _, all), p)| all.get(p.material));
                        if let (
                            Some((_, model, _)),
                            Some(p),
                            Some((look, _, _, samplers)),
                            Some(pose),
                        ) = (drawn, p, material, posed.get(*m))
                        {
                            if bound != Some(*m) {
                                bound = Some(*m);
                                rest = false;
                                pass.set_index_buffer(
                                    model.indices.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                            }
                            pass.set_pipeline(&pipelines[usize::from(*both)]);
                            pass.set_bind_group(0, &frame_group, &[*offset]);
                            pass.set_bind_group(1, look, &[]);
                            pass.set_bind_group(2, samplers, &[]);
                            let base = pose.bind(&mut pass, p, 0, &mut rest);
                            pass.draw_indexed(p.first..p.first + p.count, base, 0..1);
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            drop(pass);
            if at >= draws.len() {
                break;
            }
        }
        drop(encoder);
        // Kept for the next pass only when the ledger had room for it.
        if scratch.held.is_some() {
            if let Ok(mut kept) = fx.scratch.lock() {
                *kept = Some(scratch);
            }
        }
        Some(target)
    }
}
