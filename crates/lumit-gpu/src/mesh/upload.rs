//! A model put on the card: its vertices and indices in one buffer each, its
//! pictures, their smaller levels made on the card, and a bind group per
//! material.

use super::{look::Kept, GpuModel, MeshError, MeshFx, Part, Vertex};
use crate::GpuContext;
use glam::Vec3;
use lumit_scene::{AlphaMode, Image, ModelScene, TextureRef, Wrap};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// One level of a picture: its width, its height and its pixels.
type Level = (u32, u32, Vec<u8>);

/// `image`, halved until it fits in `limit` on each side, which only a picture
/// larger than the card can hold needs. A colour picture is averaged in linear
/// light. The smaller levels are made on the card ([`MeshFx::mips`]).
fn fitted(image: &Image, srgb: bool, limit: u32) -> Level {
    let table: Vec<f32> = (0..=255u8)
        .map(|v| {
            let c = f32::from(v) / 255.0;
            match srgb {
                false => c,
                true if c <= 0.04045 => c / 12.92,
                true => ((c + 0.055) / 1.055).powf(2.4),
            }
        })
        .collect();
    let encode = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        let e = match srgb {
            false => c,
            true if c <= 0.003_130_8 => c * 12.92,
            true => 1.055 * c.powf(1.0 / 2.4) - 0.055,
        };
        (e * 255.0 + 0.5) as u8
    };
    let mut out = vec![(image.width, image.height, image.rgba8.clone())];
    while let Some((w, h, px)) = out.last().filter(|(w, h, _)| *w > limit || *h > limit) {
        let (w, h) = (*w, *h);
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                        let v = px[((sy * w + sx) * 4 + c) as usize];
                        // Alpha is never a colour.
                        sum += if c == 3 {
                            f32::from(v) / 255.0
                        } else {
                            table[usize::from(v)]
                        };
                    }
                    let avg = sum * 0.25;
                    next.push(if c == 3 {
                        (avg * 255.0 + 0.5) as u8
                    } else {
                        encode(avg)
                    });
                }
            }
        }
        out.push((nw, nh, next));
    }
    out.pop().unwrap_or_default()
}

/// How many levels a picture `w` by `h` has, down to a pixel.
pub(super) fn level_count(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

/// A material's five texture slots, each with whether its picture is in the
/// sRGB encoding.
fn slots(m: &lumit_scene::Material) -> [(Option<TextureRef>, bool); 5] {
    [
        (m.base_colour_texture, true),
        (m.metallic_roughness_texture, false),
        (m.normal_texture, false),
        (m.occlusion_texture, false),
        (m.emissive_texture, true),
    ]
}

/// A primitive's corners three to a triangle, every primitive in the order
/// it is uploaded, or `None` for one pointing past its own corners, which is
/// left out.
pub(super) fn triangles(p: &lumit_scene::Primitive) -> Option<Vec<u32>> {
    let n = p.positions.len();
    let mut own: Vec<u32> = if p.indices.is_empty() {
        (0..u32::try_from(n).ok()?).collect()
    } else {
        p.indices.clone()
    };
    own.truncate(own.len() / 3 * 3);
    (!own.is_empty() && own.iter().all(|&i| (i as usize) < n)).then_some(own)
}

impl GpuModel {
    pub(super) fn upload(
        ctx: &GpuContext,
        fx: &MeshFx,
        scene: &Arc<ModelScene>,
    ) -> Result<Self, MeshError> {
        let too_large = |_| MeshError::TooLarge;
        // Every primitive of every mesh once, whichever nodes draw it.
        type Range = (u32, u32, i32, Vec3, Option<usize>);
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut ranges: Vec<Vec<Option<Range>>> = Vec::new();
        for mesh in &scene.meshes {
            let mut each = Vec::new();
            for p in &mesh.primitives {
                u32::try_from(p.positions.len()).map_err(too_large)?;
                let Some(own) = triangles(p) else {
                    each.push(None);
                    continue;
                };
                let first = u32::try_from(indices.len()).map_err(too_large)?;
                let base = i32::try_from(vertices.len()).map_err(too_large)?;
                indices.extend_from_slice(&own);
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                vertices.extend(p.positions.iter().enumerate().map(|(i, &position)| {
                    lo = lo.min(position.into());
                    hi = hi.max(position.into());
                    Vertex {
                        position,
                        normal: p.normals.get(i).copied().unwrap_or_default(),
                        tangent: p.tangents.get(i).copied().unwrap_or_default(),
                        uv0: p.uv0.get(i).copied().unwrap_or_default(),
                        uv1: p.uv1.get(i).copied().unwrap_or_default(),
                        colour: p.colours.get(i).copied().unwrap_or([1.0; 4]),
                    }
                }));
                let count = u32::try_from(own.len()).map_err(too_large)?;
                each.push(Some((first, count, base, (lo + hi) * 0.5, p.material)));
            }
            ranges.push(each);
        }
        let fallback = scene.materials.len();
        let plan = super::deform::Plan::new(scene, |m, i| {
            let range = ranges.get(m)?.get(i)?.as_ref()?;
            Some(range.2)
        });
        let mut parts = Vec::new();
        for (k, node) in scene.nodes.iter().enumerate() {
            let Some(each) = node.mesh.and_then(|m| ranges.get(m)) else {
                continue;
            };
            let at = scene.node_world(k);
            for (i, range) in each.iter().enumerate() {
                let Some((first, count, base_vertex, centre, material)) = *range else {
                    continue;
                };
                parts.push(Part {
                    first,
                    count,
                    base_vertex,
                    material: material.filter(|&m| m < fallback).unwrap_or(fallback),
                    node: at,
                    centre,
                    index: k,
                    deform: plan.slot(k, i),
                });
            }
        }

        // The pictures the materials read, a picture with no pixels left as
        // white.
        let limit = ctx.device.limits().max_texture_dimension_2d;
        let mut wanted: Vec<(usize, bool)> = Vec::new();
        for (r, srgb) in scene.materials.iter().flat_map(slots) {
            if let Some(r) = r.filter(|r| !wanted.contains(&(r.texture, srgb))) {
                wanted.push((r.texture, srgb));
            }
        }
        let pictures: Vec<((usize, bool), Level)> = wanted
            .into_iter()
            .filter_map(|(t, srgb)| {
                let image = scene.textures.get(t)?.image.as_ref()?;
                let whole = (image.width as usize)
                    .checked_mul(image.height as usize)?
                    .checked_mul(4)?;
                (whole > 0 && image.rgba8.len() == whole)
                    .then(|| ((t, srgb), fitted(image, srgb, limit)))
            })
            .filter(|(_, (w, h, _))| *w > 0 && *h > 0)
            .collect();
        let vertex_bytes = std::mem::size_of_val(vertices.as_slice()) as u64;
        let index_bytes = std::mem::size_of_val(indices.as_slice()) as u64;
        let largest = ctx.device.limits().max_buffer_size;
        if vertex_bytes > largest || index_bytes > largest {
            return Err(MeshError::TooLarge);
        }
        // The smaller levels add a third.
        let texture_bytes: u64 = pictures
            .iter()
            .map(|(_, (_, _, px))| px.len() as u64 / 3 * 4)
            .sum();
        let held = ctx.ledger().reserve(
            lumit_budget::Tier::Vram,
            vertex_bytes + index_bytes + texture_bytes + plan.bytes(),
        )?;

        // Never an empty buffer, which the card won't make.
        if vertices.is_empty() {
            vertices.push(Vertex::default());
        }
        if indices.is_empty() {
            indices.extend([0; 3]);
        }
        let buffer = |label, contents: &[u8], usage| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        // The pass that moves parts reads them too.
        let moving = plan.any().then_some(wgpu::BufferUsages::STORAGE);
        let vertices = buffer(
            "mesh-vertices",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX | moving.unwrap_or(wgpu::BufferUsages::empty()),
        );
        let indices = buffer(
            "mesh-indices",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        );
        let views: HashMap<(usize, bool), wgpu::TextureView> = pictures
            .into_iter()
            .map(|(key, top)| (key, upload_levels(ctx, fx, &top, key.1)))
            .collect();
        let white = fx.white.create_view(&Default::default());
        let reading = |t: &lumit_scene::Texture| {
            let wrap = |w: Wrap| match w {
                Wrap::Repeat => wgpu::AddressMode::Repeat,
                Wrap::Mirror => wgpu::AddressMode::MirrorRepeat,
                Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
            };
            let filter = if t.nearest {
                wgpu::FilterMode::Nearest
            } else {
                wgpu::FilterMode::Linear
            };
            (wrap(t.wrap_u), wrap(t.wrap_v), filter)
        };
        let default = lumit_scene::Material::default();
        let (materials, kept) = scene
            .materials
            .iter()
            .chain(std::iter::once(&default))
            .map(|m| {
                let mut uv = [[0.0; 4]; 10];
                let mut bound = Vec::with_capacity(5);
                let mut readings = [super::samplers::PLAIN; 5];
                for (k, (r, srgb)) in slots(m).into_iter().enumerate() {
                    let view = r.and_then(|r| views.get(&(r.texture, srgb)));
                    // KHR_texture_transform: offset, then turn, then scale.
                    let (sin, cos, offset, scale) = match r.and_then(|r| r.transform) {
                        Some(t) => {
                            let (s, c) = t.rotation.sin_cos();
                            (s, c, t.offset, t.scale)
                        }
                        None => (0.0, 1.0, [0.0; 2], [1.0; 2]),
                    };
                    let second = r.is_some_and(|r| r.uv_set == 1);
                    uv[k * 2] = [
                        cos * scale[0],
                        sin * scale[1],
                        offset[0],
                        f32::from(u8::from(second)),
                    ];
                    uv[k * 2 + 1] = [
                        -sin * scale[0],
                        cos * scale[1],
                        offset[1],
                        f32::from(u8::from(view.is_some())),
                    ];
                    if let Some(t) = r.and_then(|r| scene.textures.get(r.texture)) {
                        readings[k] = reading(t);
                    }
                    bound.push(view.unwrap_or(&white).clone());
                }
                let samplers = fx.samplers.group(&ctx.device, readings);
                let kept = Kept { uv, bound };
                let group = kept.group(ctx, fx, m, ([None; 4], true), None);
                (
                    (
                        group,
                        m.alpha_mode == AlphaMode::Blend,
                        m.double_sided,
                        samplers,
                    ),
                    kept,
                )
            })
            .unzip();
        Ok(Self {
            vertices,
            indices,
            parts,
            materials,
            edges: std::sync::OnceLock::new(),
            kept,
            deforms: plan.upload(ctx, scene, vertex_bytes),
            _held: held,
        })
    }
}

/// A picture on the card, its smaller levels made there from it.
fn upload_levels(ctx: &GpuContext, fx: &MeshFx, top: &Level, srgb: bool) -> wgpu::TextureView {
    let (w, h, px) = top;
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let size = wgpu::Extent3d {
        width: *w,
        height: *h,
        depth_or_array_layers: 1,
    };
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mesh-texture"),
        size,
        mip_level_count: level_count(*w, *h),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        px,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(*h),
        },
        size,
    );
    fx.mips(ctx, &texture);
    texture.create_view(&Default::default())
}
