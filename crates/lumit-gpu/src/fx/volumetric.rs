//! Volumetric light: the comp's lights seen in the air, laid over its 3D
//! layers. Not an effect: the realiser runs it once a comp's 3D layers are
//! composited, and only when a light shows in the air.
//!
//! The maths lives in `lumit_core::haze`, the oracle this kernel is compared
//! against.

use super::lighting::{air_params, map_layout, LightingLight, ShadowCasters};
use super::{work_texture, FxEngine};
use crate::mesh::shadow::ShadowMaps;
use crate::GpuContext;
use bytemuck::Zeroable;

/// How many steps a ray takes, as the oracle's `STEPS`.
pub const STEPS: u32 = 48;

/// One light's haze, the oracle's `HazeLook` restated: Volumetric, Haze
/// density, Height falloff, Forward scatter and Noise in percent, Noise size
/// and how far the noise has drifted in comp pixels. Its colour is already in
/// the light's.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Haze {
    pub amount: f32,
    pub density: f32,
    pub falloff: f32,
    pub scatter: f32,
    pub noise: f32,
    pub noise_size: f32,
    pub drift: f32,
}

/// The most boxes of smoke one walk takes. A comp with more walks again.
pub const MAX_VOLUMES: usize = 4;

/// One box of smoke as the walk reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeLook {
    /// The box's nought to one to comp pixels: three axes, then its corner.
    pub from_box: [[f32; 3]; 4],
    /// How much a density of one dims per comp pixel.
    pub thickness: f32,
    pub smoke: [f32; 3],
    /// How much its shadows count, nought to one.
    pub shadow: f32,
    /// The fire at its coolest and hottest, the light a flame gives per comp
    /// pixel, and the heat a flame starts at.
    pub fire_low: [f32; 3],
    pub fire_high: [f32; 3],
    pub fire_gain: f32,
    pub fire_start: f32,
    /// How strongly its flames light the gas round them.
    pub fire_glow: f32,
    /// How many cells fine detail pushes gas about by, and the walk's time,
    /// which its patches rise with.
    pub detail: f32,
    pub clock: f32,
    /// How far in from its sides gas thins out, a share of the box. Nought for
    /// none, and two thirds more at the top.
    pub fade: f32,
}

/// One box of smoke: its look, and a 3D field of density in red and heat in
/// green, on the card.
#[derive(Clone)]
pub struct VolumeOp {
    pub fields: wgpu::Texture,
    pub look: VolumeLook,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVolume {
    to_box: [[f32; 4]; 3],
    cells: [f32; 4],
    smoke: [f32; 4],
    fire_low: [f32; 4],
    fire_high: [f32; 4],
    detail: [f32; 4],
    detail_scale: [f32; 4],
}

/// Cells across one of the fine detail's patches.
const DETAIL_CELLS: f32 = 10.0;

/// A box's height in patches its detail rises each second, about as gas does.
const DETAIL_RISE: f32 = 0.45;

/// How many cells each way the detail's noise holds.
const NOISE_CELLS: usize = 32;

/// A tiling 3D noise, three channels, the same every time: a few octaves of smooth
/// value noise, so it holds patches a good deal finer than a cell of the grid.
fn noise_bytes() -> Vec<u8> {
    let n = NOISE_CELLS;
    // One lattice of numbers per octave, from a plain whole-number hash.
    let hash = |x: usize, y: usize, z: usize, c: usize, o: usize| {
        let v = (x as u32).wrapping_mul(73_856_093)
            ^ (y as u32).wrapping_mul(19_349_663)
            ^ (z as u32).wrapping_mul(83_492_791)
            ^ (c as u32).wrapping_mul(2_654_435_761)
            ^ (o as u32).wrapping_mul(40_503);
        let v = (v ^ (v >> 13)).wrapping_mul(1_274_126_177);
        ((v ^ (v >> 16)) & 0xffff) as f32 / 65535.0
    };
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let mut out = Vec::with_capacity(n * n * n * 4);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                for c in 0..3 {
                    let mut sum = 0.0;
                    let mut weight = 0.0;
                    for (o, (period, w)) in
                        [(8usize, 1.0f32), (4, 0.6), (2, 0.35)].iter().enumerate()
                    {
                        let cell = |v: usize| (v / period, (v % period) as f32 / *period as f32);
                        let (ax, fx) = cell(x);
                        let (ay, fy) = cell(y);
                        let (az, fz) = cell(z);
                        let wrap = n / period;
                        let at = |i: usize, j: usize, k: usize| {
                            hash((ax + i) % wrap, (ay + j) % wrap, (az + k) % wrap, c, o)
                        };
                        let (sx, sy, sz) = (smooth(fx), smooth(fy), smooth(fz));
                        let along =
                            |j: usize, k: usize| at(0, j, k) * (1.0 - sx) + at(1, j, k) * sx;
                        let across = |k: usize| along(0, k) * (1.0 - sy) + along(1, k) * sy;
                        sum += (across(0) * (1.0 - sz) + across(1) * sz) * w;
                        weight += w;
                    }
                    out.push(((sum / weight).clamp(0.0, 1.0) * 255.0) as u8);
                }
                out.push(255);
            }
        }
    }
    out
}

fn gpu_volumes(volumes: &[VolumeOp]) -> [GpuVolume; MAX_VOLUMES] {
    let mut out = [GpuVolume::zeroed(); MAX_VOLUMES];
    for (slot, v) in out.iter_mut().zip(volumes) {
        let l = &v.look;
        let c = l.from_box.map(glam::Vec3::from);
        let to_box = glam::Mat4::from_cols(
            c[0].extend(0.0),
            c[1].extend(0.0),
            c[2].extend(0.0),
            c[3].extend(1.0),
        )
        .inverse()
        .transpose()
        .to_cols_array_2d();
        let f = v.fields.size();
        *slot = GpuVolume {
            to_box: [to_box[0], to_box[1], to_box[2]],
            cells: [
                f.width as f32,
                f.height as f32,
                f.depth_or_array_layers as f32,
                l.thickness,
            ],
            smoke: v4(l.smoke, l.shadow),
            fire_low: v4(l.fire_low, l.fire_gain),
            fire_high: v4(l.fire_high, l.fire_start),
            detail: [
                l.detail,
                l.clock * DETAIL_RISE * f.height as f32 / DETAIL_CELLS,
                l.fade,
                0.0,
            ],
            detail_scale: [
                f.width as f32 / DETAIL_CELLS,
                f.height as f32 / DETAIL_CELLS,
                f.depth_or_array_layers as f32 / DETAIL_CELLS,
                0.0,
            ],
        };
    }
    out
}

/// Where the haze is seen from and the lights that show in it, all in comp
/// pixels: the oracle's `HazeView` plus what the raster needs.
#[derive(Clone)]
pub struct HazeOp {
    pub eye: [f32; 3],
    /// The camera's right, down-the-picture and forward axes.
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub zoom: f32,
    pub comp: (f32, f32),
    /// Raster pixels per comp pixel.
    pub scale: f32,
    /// The clip w the depth texture is counted from.
    pub near: f32,
    /// The sphere the lit haze is kept inside.
    pub centre: [f32; 3],
    pub radius: f32,
    pub lights: Vec<(LightingLight, Haze)>,
    /// The boxes of smoke on the way.
    pub volumes: Vec<VolumeOp>,
    /// The depth of the layers that take no shadow, if any don't.
    pub no_shadow: Option<wgpu::Texture>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HazeParams {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    sphere: [f32; 4],
    view: [f32; 4],
    look: [[f32; 4]; super::lighting::MAX_LIT_LIGHTS],
    grain: [[f32; 4]; super::lighting::MAX_LIT_LIGHTS],
}

/// The passes: the light reaching a box, the walk at half size, and laying it
/// over the picture.
pub(super) struct HazeFx {
    march_layout: wgpu::BindGroupLayout,
    march: wgpu::ComputePipeline,
    /// The same walk into half floats whatever the project's depth, for boxes.
    wide_layout: wgpu::BindGroupLayout,
    wide: wgpu::ComputePipeline,
    apply_layout: wgpu::BindGroupLayout,
    apply: wgpu::ComputePipeline,
    boxes_layout: wgpu::BindGroupLayout,
    lit_layout: wgpu::BindGroupLayout,
    lit: wgpu::ComputePipeline,
    /// What an unused box binds.
    no_box: wgpu::TextureView,
    /// What no shadow binds when every layer takes one.
    takes_all: wgpu::TextureView,
    /// The fine detail's noise, and how it is read.
    noise: wgpu::TextureView,
    noise_wrap: wgpu::Sampler,
}

/// The passes, compiled on the pool with the rest of the engine but never
/// waited for until a comp first shows a light in the air.
pub(super) struct HazeSlot {
    compiling: std::sync::Mutex<Option<crate::compiling::Compiling<HazeFx>>>,
    ready: std::sync::OnceLock<HazeFx>,
}

impl HazeSlot {
    pub(super) fn start(ctx: &GpuContext) -> Self {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        let working = ctx.working();
        Self {
            compiling: std::sync::Mutex::new(Some(crate::compiling::Compiling::start(move || {
                HazeFx::new(&device, &queue, working)
            }))),
            ready: std::sync::OnceLock::new(),
        }
    }

    fn get(&self, ctx: &GpuContext) -> &HazeFx {
        self.ready.get_or_init(|| {
            let started = self.compiling.lock().ok().and_then(|mut c| c.take());
            started.map_or_else(
                || HazeFx::new(&ctx.device, &ctx.queue, ctx.working()),
                crate::compiling::Compiling::wait,
            )
        })
    }
}

impl HazeFx {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, working: wgpu::TextureFormat) -> Self {
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let tex = |binding: u32, filterable: bool, dim: wgpu::TextureViewDimension| {
            entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable },
                    view_dimension: dim,
                    multisampled: false,
                },
            )
        };
        let uniform = |binding: u32| {
            entry(
                binding,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )
        };
        let store = entry(
            2,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: working,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        );
        use wgpu::TextureViewDimension as D;
        let mut entries = vec![
            tex(1, false, D::D2),
            store,
            uniform(3),
            tex(4, true, D::D2Array),
            entry(
                5,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
            uniform(6),
        ];
        entries.extend(map_layout(8, wgpu::ShaderStages::COMPUTE));
        let march_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-haze-march-layout"),
            entries: &entries,
        });
        entries[1] = entry(
            20,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        );
        let wide_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-haze-wide-layout"),
            entries: &entries,
        });
        let apply_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-haze-apply-layout"),
            entries: &[
                tex(0, false, D::D2),
                tex(1, false, D::D2),
                store,
                uniform(6),
                tex(7, false, D::D2),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-haze"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(
                    working,
                    concat!(
                        include_str!("../fx_noise_core.wgsl"),
                        include_str!("../light_core.wgsl"),
                        include_str!("../shadow_core.wgsl"),
                        include_str!("../environment_core.wgsl"),
                        include_str!("../fx_lighting.wgsl"),
                        include_str!("../fx_volumetric.wgsl")
                    ),
                )
                .into(),
            ),
        });
        let box_entries: Vec<_> = std::iter::once(uniform(0))
            .chain((1..=8).map(|b| tex(b, true, D::D3)))
            .chain(std::iter::once(tex(9, false, D::D2)))
            .chain(std::iter::once(tex(13, true, D::D3)))
            .chain(std::iter::once(entry(
                14,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            )))
            .collect();
        let boxes_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-haze-boxes-layout"),
            entries: &box_entries,
        });
        let mut lit_entries: Vec<_> = std::iter::once(uniform(0))
            .chain((1..=4).map(|b| tex(b, true, D::D3)))
            .collect();
        lit_entries.extend([
            entry(
                10,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            ),
            uniform(11),
            uniform(12),
        ]);
        let lit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-haze-lit-layout"),
            entries: &lit_entries,
        });
        let pipeline = |layouts: &[&wgpu::BindGroupLayout], entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            march: pipeline(&[&march_layout, &boxes_layout], "haze_march"),
            wide: pipeline(&[&wide_layout, &boxes_layout], "boxes_march"),
            apply: pipeline(&[&apply_layout], "haze_apply"),
            lit: pipeline(&[&wide_layout, &lit_layout], "smoke_lit"),
            march_layout,
            wide_layout,
            apply_layout,
            boxes_layout,
            lit_layout,
            no_box: device
                .create_texture(&field_descriptor("fx-haze-no-box", [1; 3]))
                .create_view(&Default::default()),
            noise: {
                use wgpu::util::DeviceExt;
                let n = NOISE_CELLS as u32;
                device
                    .create_texture_with_data(
                        queue,
                        &wgpu::TextureDescriptor {
                            label: Some("fx-haze-noise"),
                            size: wgpu::Extent3d {
                                width: n,
                                height: n,
                                depth_or_array_layers: n,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D3,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        },
                        wgpu::util::TextureDataOrder::LayerMajor,
                        &noise_bytes(),
                    )
                    .create_view(&Default::default())
            },
            noise_wrap: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("fx-haze-noise-wrap"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            takes_all: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("fx-haze-takes-all"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
        }
    }
}

fn v4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

/// How many pixels of the picture a texel of the walk covers. Haze is smooth, so
/// two. Flames want every pixel, and on the largest pictures they get two.
fn march_step(op: &HazeOp) -> u32 {
    let longest = op.comp.0.max(op.comp.1) * op.scale;
    if op.volumes.is_empty() || longest > 2048.0 {
        2
    } else {
        1
    }
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

/// A 3D field of half floats, sized `cells`, read filtered and copied into.
fn field_descriptor(label: &'static str, cells: [u32; 3]) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: cells[0],
            height: cells[1],
            depth_or_array_layers: cells[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    }
}

/// The cells of a row of the light's rows, which a copy wants a round number of
/// bytes.
fn lit_pitch(width: u32) -> u32 {
    (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) / 8
}

/// The cells of the light worked out for a field of `cells`: half as many each
/// way, as light changes slowly, and never too few to hold a shape.
fn lit_cells(cells: wgpu::Extent3d) -> [u32; 3] {
    [cells.width, cells.height, cells.depth_or_array_layers].map(|c| c.div_ceil(2).clamp(8, 128))
}

impl FxEngine {
    /// A depth picture with nothing in it, as the compositor leaves one with no
    /// 3D layers.
    #[must_use]
    pub fn far_depth(&self, ctx: &GpuContext, w: u32, h: u32) -> wgpu::Texture {
        let far = half::f16::MAX.to_f32();
        super::common::upload_linear_f32(ctx, &[far, 0.0, 0.0, 1.0].repeat((w * h) as usize), w, h)
    }

    fn haze_uniform(ctx: &GpuContext, op: &HazeOp) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let mut look = [[0.0; 4]; super::lighting::MAX_LIT_LIGHTS];
        let mut grain = look;
        for ((l, p), (_, h)) in look.iter_mut().zip(grain.iter_mut()).zip(&op.lights) {
            *l = [h.amount, h.density, h.scatter, h.noise];
            *p = [h.noise_size, h.drift, h.falloff, 0.0];
        }
        ctx.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-haze-params"),
                contents: bytemuck::bytes_of(&HazeParams {
                    eye: v4(op.eye, op.zoom),
                    right: v4(op.right, op.comp.0),
                    up: v4(op.up, op.comp.1),
                    forward: v4(op.forward, op.scale),
                    sphere: v4(op.centre, op.radius),
                    view: [
                        op.near,
                        march_step(op) as f32,
                        STEPS as f32,
                        op.volumes.len().min(MAX_VOLUMES) as f32,
                    ],
                    look,
                    grain,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            })
    }

    /// The light the haze scatters towards the eye and how much of what is
    /// behind is left, at half the size of `depth` (the scene's clip w less
    /// [`HazeOp::near`], as [`crate::Compositor::composite_depth`] makes it).
    /// `casters` and the models' `maps` cut the lights that cast shadows.
    pub fn haze_scatter(
        &self,
        ctx: &GpuContext,
        depth: &wgpu::Texture,
        op: &HazeOp,
        casters: Option<&ShadowCasters>,
        maps: Option<&ShadowMaps>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let fx = self.haze.get(ctx);
        let step = march_step(op);
        let (w, h) = (depth.width().div_ceil(step), depth.height().div_ceil(step));
        let volumes = &op.volumes[..op.volumes.len().min(MAX_VOLUMES)];
        let out = if volumes.is_empty() {
            work_texture(ctx, w, h, "fx-haze-scatter")
        } else {
            use wgpu::TextureUsages as U;
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fx-haze-scatter-wide"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: U::STORAGE_BINDING | U::TEXTURE_BINDING | U::COPY_SRC,
                view_formats: &[],
            })
        };
        let lights: Vec<LightingLight> = op.lights.iter().map(|(l, _)| *l).collect();
        let pbuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-haze-lights"),
                contents: &air_params(&lights, casters, maps),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let hbuf = Self::haze_uniform(ctx, op);
        let array = casters.map_or(&self.lighting.empty, |c| &c.texture);
        let v_array = array.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let (v_depth, v_out) = (
            depth.create_view(&Default::default()),
            out.create_view(&Default::default()),
        );
        let v_maps = self.lighting.map_view(maps);
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&v_depth),
            },
            wgpu::BindGroupEntry {
                binding: if volumes.is_empty() { 2 } else { 20 },
                resource: wgpu::BindingResource::TextureView(&v_out),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: pbuf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&v_array),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&self.lighting.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: hbuf.as_entire_binding(),
            },
        ];
        entries.extend(self.lighting.map_entries(8, &v_maps, maps));
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-haze-march"),
            layout: if volumes.is_empty() {
                &fx.march_layout
            } else {
                &fx.wide_layout
            },
            entries: &entries,
        });
        // The boxes, and the light reaching each.
        let vbuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-haze-volumes"),
                contents: bytemuck::bytes_of(&gpu_volumes(volumes)),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let fields: Vec<wgpu::TextureView> = volumes
            .iter()
            .map(|v| v.fields.create_view(&Default::default()))
            .collect();
        let field_view = |k: usize| fields.get(k).unwrap_or(&fx.no_box);
        // Each box's light is worked out into rows, then copied into a 3D texture.
        let lit: Vec<wgpu::Texture> = volumes
            .iter()
            .map(|v| {
                let size = lit_cells(v.fields.size());
                ctx.device
                    .create_texture(&field_descriptor("fx-haze-lit", size))
            })
            .collect();
        let lit_rows: Vec<(wgpu::Buffer, u32)> = lit
            .iter()
            .map(|t| {
                let size = t.size();
                let pitch = lit_pitch(size.width);
                let bytes = u64::from(pitch)
                    * 8
                    * u64::from(size.height)
                    * u64::from(size.depth_or_array_layers);
                let rows = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("fx-haze-lit-rows"),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                (rows, pitch)
            })
            .collect();
        let lit_views: Vec<wgpu::TextureView> = lit
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let lit_binds: Vec<wgpu::BindGroup> = volumes
            .iter()
            .zip(&lit)
            .zip(&lit_rows)
            .enumerate()
            .map(|(k, ((v, t), (rows, pitch)))| {
                let small = |label, contents: &[u8]| {
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(label),
                            contents,
                            usage: wgpu::BufferUsages::UNIFORM,
                        })
                };
                let size = t.size();
                let which = small(
                    "fx-haze-lit-which",
                    bytemuck::cast_slice(&[
                        k as u32,
                        *pitch,
                        v.look.fire_glow.to_bits(),
                        0,
                        size.width,
                        size.height,
                        size.depth_or_array_layers,
                        0,
                    ]),
                );
                let from = small(
                    "fx-haze-lit-from",
                    bytemuck::cast_slice(&v.look.from_box.map(|c| v4(c, 0.0))),
                );
                ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fx-haze-lit"),
                    layout: &fx.lit_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: vbuf.as_entire_binding(),
                        },
                        view_entry(1, field_view(0)),
                        view_entry(2, field_view(1)),
                        view_entry(3, field_view(2)),
                        view_entry(4, field_view(3)),
                        wgpu::BindGroupEntry {
                            binding: 10,
                            resource: rows.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 11,
                            resource: which.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 12,
                            resource: from.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        let lit_view = |k: usize| lit_views.get(k).unwrap_or(&fx.no_box);
        let no_shadow = op
            .no_shadow
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let boxes = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-haze-boxes"),
            layout: &fx.boxes_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vbuf.as_entire_binding(),
                },
                view_entry(1, field_view(0)),
                view_entry(2, field_view(1)),
                view_entry(3, field_view(2)),
                view_entry(4, field_view(3)),
                view_entry(5, lit_view(0)),
                view_entry(6, lit_view(1)),
                view_entry(7, lit_view(2)),
                view_entry(8, lit_view(3)),
                view_entry(9, no_shadow.as_ref().unwrap_or(&fx.takes_all)),
                view_entry(13, &fx.noise),
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: wgpu::BindingResource::Sampler(&fx.noise_wrap),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-haze-march");
        if !volumes.is_empty() {
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fx-haze-lit"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&fx.lit);
                pass.set_bind_group(0, &bind, &[]);
                for (group, t) in lit_binds.iter().zip(&lit) {
                    let size = t.size();
                    pass.set_bind_group(1, group, &[]);
                    pass.dispatch_workgroups(
                        size.width.div_ceil(4),
                        size.height.div_ceil(4),
                        size.depth_or_array_layers.div_ceil(4),
                    );
                }
            }
            for (t, (rows, pitch)) in lit.iter().zip(&lit_rows) {
                enc.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: rows,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(pitch * 8),
                            rows_per_image: Some(t.height()),
                        },
                    },
                    t.as_image_copy(),
                    t.size(),
                );
            }
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-haze-march"),
                timestamp_writes: None,
            });
            pass.set_pipeline(if volumes.is_empty() {
                &fx.march
            } else {
                &fx.wide
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.set_bind_group(1, &boxes, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// `frame` with the haze laid over it: dimmed by what the haze takes
    /// away, and the light it scatters added. `frame` and `depth` are the
    /// same size.
    pub fn haze(
        &self,
        ctx: &GpuContext,
        frame: &wgpu::Texture,
        depth: &wgpu::Texture,
        op: &HazeOp,
        casters: Option<&ShadowCasters>,
        maps: Option<&ShadowMaps>,
    ) -> wgpu::Texture {
        let scatter = self.haze_scatter(ctx, depth, op, casters, maps);
        let fx = self.haze.get(ctx);
        let (w, h) = (frame.width(), frame.height());
        let out = work_texture(ctx, w, h, "fx-haze-out");
        let hbuf = Self::haze_uniform(ctx, op);
        let views = [frame, depth, &out, &scatter].map(|t| t.create_view(&Default::default()));
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-haze-apply"),
            layout: &fx.apply_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: hbuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&views[3]),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-haze-apply");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-haze-apply"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&fx.apply);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}

#[cfg(test)]
mod tests;
