//! Shadow maps: each light that casts, seen from where it shines, with how
//! far the nearest model is at every texel. Models, 3D layers and the haze
//! read them (`mapped` in `shadow_core.wgsl`), so a model's shadow falls on
//! all three.
//!
//! Made once a frame from the models that cast, each light's views fitted
//! round them. A comp with no model that casts never makes any.

use super::setup::uniform_entry;
use super::{DrawUniform, FrameUniform, MeshFx, DEPTH};
use crate::fx::lighting::{
    caster_set, map_layout, LightingFx, LightingKind, LightingLight, ShadowCasters,
    DIFFUSION_SPREAD, MAX_LIT_LIGHTS,
};
use crate::fx::FxEngine;
use crate::GpuContext;
use glam::{Mat4, Vec3, Vec4};
use lumit_scene::ModelScene;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Six views round each of the eight lights.
const MAX_VIEWS: usize = 48;

/// Past this many views a map is drawn at half the side, so two lights all
/// round a model don't take a quarter of a gigabyte.
const FULL_SIDE_VIEWS: usize = 4;

/// The widest soft edge the shader reads, texels either side
/// (`MAP_MAX_SOFT` in `shadow_core.wgsl`).
const MAX_SOFT: f32 = 24.0;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuMap {
    /// x = its first view, y = how many, z = a texel's width per unit of w.
    view: [f32; 4],
    /// x = the nearest depth kept, y = how far past that the farthest is.
    range: [f32; 4],
}

/// The maps as `shadow_core.wgsl` reads them.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuMaps {
    /// x = how many lights have maps, y = the side in texels.
    head: [f32; 4],
    lights: [GpuMap; MAX_LIT_LIGHTS],
    views: [[[f32; 4]; 4]; MAX_VIEWS],
}

/// One frame's shadow maps, one or six views a light in one array.
pub struct ShadowMaps {
    pub(crate) texture: wgpu::Texture,
    pub(crate) uniform: wgpu::Buffer,
    lights: Vec<LightingLight>,
}

impl ShadowMaps {
    /// Which map `l` reads, counted from one, nought when it has none.
    pub(crate) fn slot(&self, l: &LightingLight) -> u32 {
        self.lights
            .iter()
            .position(|m| m == l)
            .map_or(0, |i| i as u32 + 1)
    }
}

/// One model that casts: its scene, and where it is in comp pixels.
pub struct MapCaster<'a> {
    pub scene: &'a Arc<ModelScene>,
    pub model_to_comp: Mat4,
}

/// What is bound when a frame has no maps: one empty map and a uniform
/// saying there are none.
pub(crate) fn no_maps(device: &wgpu::Device) -> (wgpu::Texture, wgpu::Buffer) {
    (
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-maps-none"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }),
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow-maps-none"),
            size: std::mem::size_of::<GpuMaps>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        }),
    )
}

/// The mesh pass's third group: the 3D layers' casters, their squares and
/// sampler, then the maps.
pub(super) fn layout() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = vec![
        uniform_entry(0, false),
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    entries.extend(map_layout(3, wgpu::ShaderStages::FRAGMENT));
    entries
}

/// A matrix from its four rows.
fn rows(r: [Vec4; 4]) -> Mat4 {
    Mat4::from_cols(r[0], r[1], r[2], r[3]).transpose()
}

/// Two axes square to `d` and to each other.
fn across(d: Vec3) -> (Vec3, Vec3) {
    let up = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::Z };
    let right = d.cross(up).normalize();
    (right, right.cross(d))
}

/// A view from `eye` along `d`, `t` the tangent of its half-angle: the map
/// at `0.5 + 0.5 x / (z t)` across and down, and depth `z` along `d`.
fn perspective(eye: Vec3, d: Vec3, t: f32) -> Mat4 {
    let (right, up) = across(d);
    let row = |a: Vec3| a.extend(-a.dot(eye));
    rows([
        row(d * 0.5 + right * (0.5 / t)),
        row(d * 0.5 - up * (0.5 / t)),
        row(d),
        row(d),
    ])
}

/// Each light's views round `points`, the casters' box corners: the views'
/// matrices, the map for the shader with its texel's width for a side of one,
/// and the most texels a side can have before the light's widest soft edge
/// is past the [`MAX_SOFT`] texels the shader reads. A parallel light looks
/// along itself over the casters. Any other looks at them from where it is,
/// all round when it is among them or they fill too wide a cone.
///
/// ponytail: one map a light round every model, so a small model beside a
/// big one gets a coarse shadow. A map a model, or cascades, if it shows.
fn fit(l: &LightingLight, points: &[Vec3], first: usize) -> Option<(Vec<Mat4>, GpuMap, f32)> {
    let centre = l.corners.iter().map(|&c| Vec3::from(c)).sum::<Vec3>() / 4.0;
    let soft = l.shadow_diffusion * DIFFUSION_SPREAD;
    let map = |count: usize, texel: f32, near: f32, far: f32| GpuMap {
        view: [first as f32, count as f32, texel, 0.0],
        range: [near, (far - near).max(1.0), 0.0, 0.0],
    };
    // The edge reaches soft * depth / away either side, as a 3D layer's
    // does, and a texel is `across` comp pixels for a side of one.
    let most =
        |across: f32, away: f32, depth: f32| MAX_SOFT * across * away / (soft * depth).max(1e-6);
    if l.kind == LightingKind::Parallel {
        let d = Vec3::from(l.axis).try_normalize()?;
        let (right, up) = across(d);
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in points {
            let q = Vec3::new(p.dot(right), p.dot(up), p.dot(d));
            (lo, hi) = (lo.min(q), hi.max(q));
        }
        // Square, with a little room round the casters for the soft edge.
        let mid = (lo + hi) * 0.5;
        let width = (hi.x - lo.x).max(hi.y - lo.y) * 1.05 + 2.0;
        let m = rows([
            (right / width).extend(0.5 - mid.x / width),
            (-up / width).extend(0.5 + mid.y / width),
            d.extend(0.0),
            Vec4::W,
        ]);
        let away = ((lo + hi) * 0.5 - Vec3::new(centre.dot(right), centre.dot(up), centre.dot(d)))
            .length()
            .max(1.0);
        let most = most(width, away, hi.z - lo.z);
        return Some((vec![m], map(1, width, lo.z - 1.0, hi.z + 1.0), most));
    }
    let (lo, hi) = points.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let middle = (lo + hi) * 0.5;
    let radius = points
        .iter()
        .map(|p| p.distance(middle))
        .fold(0.0, f32::max);
    let away = middle.distance(centre);
    let far = points
        .iter()
        .map(|p| p.distance(centre))
        .fold(0.0, f32::max)
        + 1.0;
    let nearest = points
        .iter()
        .map(|p| p.distance(centre))
        .fold(f32::MAX, f32::min)
        .max(1.0);
    // Within about 60 degrees of looking at them, one view does.
    let sin = radius / away.max(1e-3);
    if sin < 0.85 {
        let d = (middle - centre) / away;
        let t = sin / (1.0 - sin * sin).sqrt() * 1.05 + 1e-3;
        let near = points
            .iter()
            .map(|p| (*p - centre).dot(d))
            .fold(f32::MAX, f32::min);
        let near = (near - 1.0).max(0.5);
        let most = most(2.0 * t * near, near, far - near);
        return Some((
            vec![perspective(centre, d, t)],
            map(1, 2.0 * t, near, far),
            most,
        ));
    }
    // A little over 90 degrees a face, so the soft edge has room at the seams.
    let t = 1.1;
    let faces = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
    let views = faces.map(|d| perspective(centre, d, t)).to_vec();
    let most = most(2.0 * t * nearest, nearest, far - nearest);
    Some((views, map(6, 2.0 * t, 0.5, far), most))
}

impl MeshFx {
    /// The mesh pass's third group, for a model taking `casters` and `maps`.
    pub(super) fn shadows_group(
        &self,
        ctx: &GpuContext,
        lighting: &LightingFx,
        casters: Option<&ShadowCasters>,
        maps: Option<&ShadowMaps>,
    ) -> wgpu::BindGroup {
        let set = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-casters"),
                contents: &caster_set(casters),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let squares = casters.map_or(&lighting.empty, |c| &c.texture).create_view(
            &wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            },
        );
        let map_view = lighting.map_view(maps);
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: set.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&squares),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&lighting.sampler),
            },
        ];
        entries.extend(lighting.map_entries(3, &map_view, maps));
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-shadows"),
            layout: &self.shadows_layout,
            entries: &entries,
        })
    }

    /// Draws depth alone into a view of a map, made the first time.
    fn shadow_pipeline(&self, device: &wgpu::Device) -> &wgpu::RenderPipeline {
        self.shadow.get_or_init(|| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mesh-shadow"),
                bind_group_layouts: &[&self.frame_layout, &self.material_layout],
                push_constant_ranges: &[],
            });
            let attributes = wgpu::vertex_attr_array![
                0 => Float32x3, 1 => Float32x3, 2 => Float32x4,
                3 => Float32x2, 4 => Float32x2, 5 => Float32x4,
            ];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh-shadow"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_shadow"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<super::Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_shadow"),
                    targets: &[],
                    compilation_options: Default::default(),
                }),
                // Both faces, so an open model casts too.
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        })
    }
}

impl FxEngine {
    /// The shadow maps of `lights` that cast, from `casters`, `side` texels
    /// a view. `None` when no light casts or no model is there to.
    pub fn shadow_maps(
        &self,
        ctx: &GpuContext,
        casters: &[MapCaster<'_>],
        lights: &[LightingLight],
        side: u32,
    ) -> Option<ShadowMaps> {
        let fx = self.mesh.get_or_init(|| MeshFx::new(ctx));
        let models: Vec<_> = casters
            .iter()
            .filter_map(|c| Some((fx.model_for(ctx, c.scene).ok()?, c)))
            .collect();
        let points: Vec<Vec3> = models
            .iter()
            .flat_map(|(_, c)| {
                let (lo, hi) = (
                    Vec3::from(c.scene.bounds.min),
                    Vec3::from(c.scene.bounds.max),
                );
                (0..8).map(move |k| {
                    let pick = |bit: usize, a: f32, b: f32| if k & bit == 0 { a } else { b };
                    c.model_to_comp.transform_point3(Vec3::new(
                        pick(1, lo.x, hi.x),
                        pick(2, lo.y, hi.y),
                        pick(4, lo.z, hi.z),
                    ))
                })
            })
            .filter(|p| p.is_finite())
            .collect();
        if points.is_empty() {
            return None;
        }
        let mut kept: Vec<LightingLight> = Vec::new();
        let mut maps = [GpuMap::default(); MAX_LIT_LIGHTS];
        let mut views: Vec<Mat4> = Vec::new();
        let (mut sharpest, mut all_round) = (0.0f32, false);
        // ponytail: past eight lights or 48 views the rest cast no model's
        // shadow, as past eight lights a layer takes no more.
        for l in lights {
            let casts = l.kind != LightingKind::Ambient && l.shadow_darkness > 0.0;
            if !casts || kept.contains(l) || kept.len() == MAX_LIT_LIGHTS {
                continue;
            }
            let Some((these, map, most)) = fit(l, &points, views.len()) else {
                continue;
            };
            if views.len() + these.len() > MAX_VIEWS {
                continue;
            }
            sharpest = sharpest.max(most);
            all_round |= these.len() > 1;
            maps[kept.len()] = map;
            kept.push(*l);
            views.extend(these);
        }
        if kept.is_empty() {
            return None;
        }
        // Fewer texels when every light is soft, so their soft edges are as
        // wide as a 3D layer's under them. A hard light keeps them all, and a
        // soft one beside it stops at the widest edge the shader reads. Six
        // views keep enough for the seams' overlap.
        let limit = ctx.device.limits().max_texture_dimension_2d;
        let side = if views.len() > FULL_SIDE_VIEWS {
            side / 2
        } else {
            side
        }
        .min(sharpest.max(256.0) as u32)
        .max(if all_round { 512 } else { 64 })
        .min(limit);
        for m in &mut maps[..kept.len()] {
            m.view[2] /= side as f32;
        }
        let mut gpu = GpuMaps {
            head: [kept.len() as f32, side as f32, 0.0, 0.0],
            lights: maps,
            views: [[[0.0; 4]; 4]; MAX_VIEWS],
        };
        for (slot, m) in gpu.views.iter_mut().zip(&views) {
            *slot = m.to_cols_array_2d();
        }
        Some(fx.draw_maps(ctx, &models, &views, &gpu, side, kept))
    }
}

impl MeshFx {
    /// Each view's depth from every part of `models`, into one array.
    fn draw_maps(
        &self,
        ctx: &GpuContext,
        models: &[(Arc<super::GpuModel>, &MapCaster<'_>)],
        views: &[Mat4],
        gpu: &GpuMaps,
        side: u32,
        lights: Vec<LightingLight>,
    ) -> ShadowMaps {
        let layers = views.len() as u32;
        ctx.charge_vram(crate::texture_bytes(DEPTH, side, side).saturating_mul(u64::from(layers)));
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-maps"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let align = u64::from(ctx.device.limits().min_uniform_buffer_offset_alignment);
        let stride = |size: usize| wgpu::util::align_to(size as u64, align);
        // Each view's frame: the view in `clip`, its depth range in `eye`.
        let frame_size = std::mem::size_of::<FrameUniform>();
        let mut frames = vec![0u8; (stride(frame_size) * u64::from(layers)) as usize];
        for (k, m) in views.iter().enumerate() {
            let map = gpu
                .lights
                .iter()
                .find(|g| (g.view[0] as usize..(g.view[0] + g.view[1]) as usize).contains(&k));
            let range = map.map_or([0.0, 1.0], |g| [g.range[0], g.range[1]]);
            let frame = FrameUniform {
                clip: m.to_cols_array_2d(),
                eye: [range[0], range[1], 0.0, 0.0],
                ..bytemuck::Zeroable::zeroed()
            };
            let at = (stride(frame_size) * k as u64) as usize;
            frames[at..][..frame_size].copy_from_slice(bytemuck::bytes_of(&frame));
        }
        let frames = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow-frames"),
                contents: &frames,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        // Each part where its node and its model put it.
        let draw_size = std::mem::size_of::<DrawUniform>();
        let parts: Vec<_> = models
            .iter()
            .flat_map(|(model, c)| {
                model
                    .parts
                    .iter()
                    .map(move |p| (model, c.model_to_comp * p.node, p))
            })
            .collect();
        let mut draws = vec![0u8; (stride(draw_size) * parts.len().max(1) as u64) as usize];
        for (k, (_, at, _)) in parts.iter().enumerate() {
            let draw = DrawUniform {
                model: at.to_cols_array_2d(),
                normal: [[0.0; 4]; 3],
            };
            let offset = (stride(draw_size) * k as u64) as usize;
            draws[offset..][..draw_size].copy_from_slice(bytemuck::bytes_of(&draw));
        }
        let draws = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow-draws"),
                contents: &draws,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        // The frame's one copy, which the shadow's vertices don't read.
        let one = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow-copy"),
                contents: bytemuck::bytes_of(&super::ModelCopy {
                    place: Mat4::IDENTITY.to_cols_array_2d(),
                    tint: [1.0; 4],
                }),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let pipeline = self.shadow_pipeline(&ctx.device);
        let mut encoder = ctx.encoder("shadow-maps");
        for k in 0..layers {
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow-frame"),
                layout: &self.frame_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &frames,
                            offset: stride(frame_size) * u64::from(k),
                            size: wgpu::BufferSize::new(frame_size as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &draws,
                            offset: 0,
                            size: wgpu::BufferSize::new(draw_size as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: one.as_entire_binding(),
                    },
                ],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("shadow-map"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: k,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            for (i, (model, _, p)) in parts.iter().enumerate() {
                let Some((material, _, _)) = model.materials.get(p.material) else {
                    continue;
                };
                pass.set_vertex_buffer(0, model.vertices.slice(..));
                pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_bind_group(0, &group, &[(stride(draw_size) * i as u64) as u32]);
                pass.set_bind_group(1, material, &[]);
                pass.draw_indexed(p.first..p.first + p.count, p.base_vertex, 0..1);
            }
        }
        drop(encoder);
        ShadowMaps {
            texture,
            uniform: ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("shadow-maps"),
                    contents: bytemuck::bytes_of(gpu),
                    usage: wgpu::BufferUsages::UNIFORM,
                }),
            lights,
        }
    }
}
