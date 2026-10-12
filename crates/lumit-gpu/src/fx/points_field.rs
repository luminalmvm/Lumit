//! Points field's GPU passes: seed, jump, resolve. The kernel next door
//! (`fx_pointsfield.wgsl`) says how the search works. This is the host half,
//! which picks one seed a pixel, makes the two textures the search bounces
//! between, and runs the passes in order.

use std::collections::HashMap;

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// The seed pass's workgroup width, the same 64 the kernel declares.
const SEED_WORKGROUP: u32 = 64;

/// The most seeds one dispatch can reach: every device allows 65 535
/// workgroups along an axis.
const MAX_SEEDS: usize = 65_535 * SEED_WORKGROUP as usize;

/// One point, as the field reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldPoint {
    /// Where the point is seen on the frame, raster px.
    pub at: [f32; 2],
    /// The number the point carries.
    pub number: f32,
    /// Premultiplied scene-linear RGBA.
    pub colour: [f32; 4],
}

/// One resolved Points field. Mirrors
/// `lumit_core::fx::effects::points_field::PointsField`.
#[derive(Debug, Clone, Copy)]
pub struct PointsFieldOp<'a> {
    pub points: &'a [FieldPoint],
    /// 0 distance, 1 direction, 2 colour, 3 number.
    pub output: u32,
    /// Raster px.
    pub radius: f32,
    /// Colour and Number are transparent further than `radius` from a point.
    pub limit: bool,
    /// Distance reads 0 at a point instead of 1.
    pub invert: bool,
    /// The number that draws as white.
    pub number_range: f32,
    /// The host Mix, `0..=1`.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FieldParams {
    step: i32,
    count: u32,
    output: u32,
    flags: u32,
    radius: f32,
    range: f32,
    mix_amt: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FieldSeed {
    at: [f32; 2],
    number: f32,
    _pad: f32,
    colour: [f32; 4],
}

impl FxEngine {
    /// Draw one Points field over a working texture, returning a new texture
    /// of the same size.
    ///
    /// No points, or a Mix of nothing, hands the input back unchanged.
    pub fn points_field(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &PointsFieldOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-points-field-out");
        let seeds = one_seed_a_pixel(op.points, w, h);
        if seeds.is_empty() || op.mix <= 0.0 {
            let mut enc = ctx.encoder("fx-points-field-copy");
            enc.copy_texture_to_texture(src.as_image_copy(), out.as_image_copy(), out.size());
            return out;
        }
        let pipes = &self.points_field;
        let seed_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-points-field-seeds"),
                contents: bytemuck::cast_slice(&seeds),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // The two pictures the search bounces between: which seed each pixel
        // knows of. A new texture starts at nought, which is "none", and that
        // is why these are made here and not kept in a pool. Four bytes a
        // texel, which the ledger's table knows as R32Float.
        ctx.charge_vram(2 * crate::texture_bytes(wgpu::TextureFormat::R32Float, w, h));
        let near = |label: &str| {
            ctx.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: out.size(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R32Uint,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let mut from = near("fx-points-field-a");
        let mut to = near("fx-points-field-b");
        let src_view = src.create_view(&Default::default());
        let out_view = out.create_view(&Default::default());
        let bind = |step: u32, from: &wgpu::TextureView, to: &wgpu::TextureView| {
            let ubuf = ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("fx-points-field-params"),
                    contents: bytemuck::bytes_of(&FieldParams {
                        step: step as i32,
                        count: seeds.len() as u32,
                        output: op.output,
                        flags: u32::from(op.invert) | u32::from(op.limit) << 1,
                        radius: op.radius,
                        range: op.number_range,
                        mix_amt: op.mix.clamp(0.0, 1.0),
                        _pad: 0.0,
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let view = wgpu::BindingResource::TextureView;
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-points-field-bind"),
                layout: &pipes.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: seed_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: view(from),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: view(to),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: view(&src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: view(&out_view),
                    },
                ],
            })
        };

        let mut enc = ctx.encoder("fx-points-field");
        let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-points-field"),
            timestamp_writes: None,
        });
        // Every pass binds all six, and reads only the ones it needs.
        cp.set_pipeline(&pipes.seed);
        cp.set_bind_group(0, &bind(0, &from, &to), &[]);
        cp.dispatch_workgroups((seeds.len() as u32).div_ceil(SEED_WORKGROUP), 1, 1);
        std::mem::swap(&mut from, &mut to);
        // Half the frame's longer side first, rounded up to a power of two,
        // then half of that, down to the pixel next door. Then the pixel next
        // door once more, which mends the odd pixel the halving leaves on the
        // wrong point.
        cp.set_pipeline(&pipes.jump);
        let first = w.max(h).max(2).next_power_of_two() / 2;
        let steps = std::iter::successors(Some(first), |s| (*s > 1).then_some(s / 2)).chain([1]);
        for step in steps {
            cp.set_bind_group(0, &bind(step, &from, &to), &[]);
            cp.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            std::mem::swap(&mut from, &mut to);
        }
        cp.set_pipeline(&pipes.resolve);
        cp.set_bind_group(0, &bind(0, &from, &to), &[]);
        cp.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        drop(cp);
        drop(enc);
        out
    }
}

/// The seeds the card is sent: at most one a pixel, because the seed pass
/// writes a pixel from one thread per seed and two writers would race.
///
/// The pixel is the one the point sits on, or the nearest on the frame's edge
/// for a point off it. Where several share one, the nearest to that pixel's
/// centre stays, and the earlier of two equally near.
/// (ponytail: the others are dropped, so two points inside one pixel draw as
/// one. Keep a short list a pixel if that ever shows.)
fn one_seed_a_pixel(points: &[FieldPoint], w: u32, h: u32) -> Vec<FieldSeed> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let points = points.get(..MAX_SEEDS).unwrap_or(points);
    // Each pixel's best so far: how far from its centre, and which point.
    let mut best: HashMap<u64, (f32, usize)> = HashMap::with_capacity(points.len());
    for (i, pt) in points.iter().enumerate() {
        if !(pt.at[0].is_finite() && pt.at[1].is_finite()) {
            continue;
        }
        // The kernel's own sum: floor, then hold to the frame.
        let x = pt.at[0].floor().clamp(0.0, (w - 1) as f32);
        let y = pt.at[1].floor().clamp(0.0, (h - 1) as f32);
        let (dx, dy) = (pt.at[0] - (x + 0.5), pt.at[1] - (y + 0.5));
        let d = dx * dx + dy * dy;
        // Walked in order, so only a nearer point takes a pixel from an
        // earlier one.
        best.entry(y as u64 * u64::from(w) + x as u64)
            .and_modify(|b| {
                if d < b.0 {
                    *b = (d, i);
                }
            })
            .or_insert((d, i));
    }
    // The winners, picked back out in the stream's own order. The map is only
    // ever asked who won, so the order it happens to hold them in never shows.
    let mut kept = vec![false; points.len()];
    for (_, i) in best.into_values() {
        if let Some(keep) = kept.get_mut(i) {
            *keep = true;
        }
    }
    points
        .iter()
        .zip(kept)
        .filter(|(_, keep)| *keep)
        .map(|(pt, _)| FieldSeed {
            at: pt.at,
            number: pt.number,
            _pad: 0.0,
            colour: pt.colour,
        })
        .collect()
}

/// The three pipelines and their one layout, built once per device.
pub(super) struct PointsFieldPipelines {
    layout: wgpu::BindGroupLayout,
    seed: wgpu::ComputePipeline,
    jump: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
}

impl PointsFieldPipelines {
    pub(super) fn new(ctx: &GpuContext, module: &wgpu::ShaderModule) -> Self {
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let buffer = |ty: wgpu::BufferBindingType| wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let texture = |sample_type: wgpu::TextureSampleType| wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let storage = |format: wgpu::TextureFormat| wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-points-field-layout"),
                entries: &[
                    entry(0, buffer(wgpu::BufferBindingType::Uniform)),
                    entry(
                        1,
                        buffer(wgpu::BufferBindingType::Storage { read_only: true }),
                    ),
                    entry(2, texture(wgpu::TextureSampleType::Uint)),
                    entry(3, storage(wgpu::TextureFormat::R32Uint)),
                    entry(
                        4,
                        texture(wgpu::TextureSampleType::Float { filterable: false }),
                    ),
                    entry(5, storage(ctx.working())),
                ],
            });
        let pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-points-field-pl"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let compute = |entry: &str| {
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pl),
                    module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        Self {
            seed: compute("pf_seed"),
            jump: compute("pf_jump"),
            resolve: compute("pf_resolve"),
            layout,
        }
    }
}
