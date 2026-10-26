//! The lighting pass (docs/06) — shading a layer with the comp's Light
//! layers. Not an effect: it has no entry in docs/08 and no `Resolved`
//! variant, and the realiser calls it directly between a layer's effect stack
//! and its composite.
//!
//! The maths lives in `lumit_core::lighting`, which is the oracle this kernel
//! is compared against. The types are restated here because an engine GPU
//! crate does not depend on the model crate (docs/05).
//!
//! Shadows come from the frame's casting layers, each shrunk into one square
//! of a texture array with smaller levels under it. A pixel being lit looks
//! back along the way the light came and reads every caster it passes
//! through, so the same squares serve every light and every layer.

use super::{work_texture, FxEngine};
use crate::GpuContext;

/// How many lights shade one layer in a single pass — the same budget the
/// oracle keeps (`lumit_core::lighting::MAX_LIT_LIGHTS`).
pub const MAX_LIT_LIGHTS: usize = 8;

/// How many layers cast shadows in one frame, as the oracle's
/// `MAX_SHADOW_CASTERS`.
pub const MAX_SHADOW_CASTERS: usize = 16;

/// How wide a shadow's soft edge is on its caster per pixel of Shadow
/// diffusion, were the caster at the light, as the oracle's
/// `DIFFUSION_SPREAD`.
pub const DIFFUSION_SPREAD: f32 = 0.85;

/// How many times longer one way a soft edge on a caster turned from the
/// light gets, as the oracle's `MAX_STRETCH`.
pub const MAX_STRETCH: f32 = 2.0;

/// One light, flattened for the kernel: the four comp-pixel corners of its
/// emitting rectangle (all four equal for a point or a spot), its colour and
/// falloff, its spot axis and cone, and which law shades it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingLight {
    pub corners: [[f32; 3]; 4],
    pub colour: [f32; 3],
    pub kind: LightingKind,
    /// 0 none, 1 smooth, 2 inverse square clamped.
    pub falloff: u32,
    pub radius_px: f32,
    pub falloff_px: f32,
    /// A light from before it had rows, with the old smooth falloff.
    pub legacy: bool,
    /// Cosine of a spot's half-angle; below -1 means "not a spot".
    pub cone_cos: f32,
    /// Cosine of the angle the feather starts at.
    pub cone_inner: f32,
    pub axis: [f32; 3],
    /// How much light its shadows take away, 0..1. Nought casts none.
    pub shadow_darkness: f32,
    /// How soft their edges are, as the light's size in comp pixels.
    pub shadow_diffusion: f32,
}

/// The oracle's `ShadingKind`, restated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightingKind {
    Point,
    Area,
    Parallel,
    Ambient,
}

/// After Effects' Casts shadows and Accepts shadows: no, yes, or only the
/// shadow without the layer. The model's `ShadowMode`, restated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LightingShadows {
    #[default]
    Off,
    On,
    Only,
}

/// One layer's plane in comp pixels, plus the lights that reach it. An empty
/// `lights` is the no-op the realiser relies on.
#[derive(Debug, Clone, PartialEq)]
pub struct LightingOp {
    pub origin: [f32; 3],
    pub du: [f32; 3],
    pub dv: [f32; 3],
    pub normal: [f32; 3],
    pub material: LightingMaterial,
    pub lights: Vec<LightingLight>,
    /// This layer's own square among the casters, which it never shades
    /// itself with.
    pub own_caster: Option<usize>,
}

impl LightingOp {
    /// The layer's four corners in comp pixels, for a raster of `w` by `h`:
    /// its top-left, along its width, along its height, then the far one.
    #[must_use]
    pub fn corners(&self, w: u32, h: u32) -> [[f32; 3]; 4] {
        let (w, h) = (w as f32, h as f32);
        let at =
            |x: f32, y: f32| [0, 1, 2].map(|k| self.origin[k] + self.du[k] * x + self.dv[k] * y);
        [at(0.0, 0.0), at(w, 0.0), at(0.0, h), at(w, h)]
    }
}

/// The layer's Material options as fractions, and the eye: the oracle's
/// `ShadingMaterial`, restated. The default is the oracle's `PLAIN`, the
/// lights as they were before materials.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingMaterial {
    /// 1 where light adds to the picture, 0 where it lights from black.
    pub base: f32,
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub exponent: f32,
    pub metal: f32,
    pub eye: [f32; 3],
    /// Light the display-encoded picture, as an After Effects project without
    /// Linearize working space does.
    pub encoded: bool,
    pub accepts_shadows: LightingShadows,
    /// The layer's own shadows: whether it casts them, and how much of the
    /// light passing through takes its colour, 0..1. Read by the realiser,
    /// which gathers the casters.
    pub casts_shadows: LightingShadows,
    pub transmission: f32,
    /// Accepts lights off: the lights leave it as it is, but their shadows
    /// still darken it.
    pub unlit: bool,
}

impl Default for LightingMaterial {
    fn default() -> Self {
        Self {
            base: 1.0,
            ambient: 1.0,
            diffuse: 1.0,
            specular: 0.0,
            exponent: 1.0,
            metal: 1.0,
            eye: [0.0; 3],
            encoded: false,
            accepts_shadows: LightingShadows::Off,
            casts_shadows: LightingShadows::Off,
            transmission: 0.0,
            unlit: false,
        }
    }
}

/// Where one caster's picture sits: its top-left corner and its full width
/// and height as edges in comp pixels, with its Light transmission and its
/// opacity, both 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CasterPlane {
    pub origin: [f32; 3],
    pub u: [f32; 3],
    pub v: [f32; 3],
    pub transmission: f32,
    pub opacity: f32,
}

/// The frame's casters, made once and read by every layer's lighting.
pub struct ShadowCasters {
    texture: wgpu::Texture,
    planes: Vec<CasterPlane>,
    /// Where each picture sits in its square, as shares of the side.
    places: Vec<[f32; 4]>,
    size: u32,
}

/// Where a caster's picture sits in its square of `size`, in texels, as the
/// oracle's `caster_box`: the longer side fills three quarters, less where
/// `spread` needs a wider margin, and the shorter keeps the plane's shape, so
/// a texel is square in comp pixels.
fn caster_box(size: u32, u_len: f32, v_len: f32, spread: f32) -> [u32; 4] {
    let long = u_len.max(v_len).max(1e-6);
    let share = (1.0 / (1.0 + 2.5 * spread.max(0.0) / long)).min(0.75);
    let fill = size as f32 * share;
    let w = ((fill * u_len / long).round() as u32).clamp(1, size);
    let h = ((fill * v_len / long).round() as u32).clamp(1, size);
    [(size - w) / 2, (size - h) / 2, w, h]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Whether `caster` can put any of `light`'s shadow on the layer whose
/// corners are `receiver` (its top-left, then along its width, then its
/// height, then the far corner). Errs towards yes: it only says no when the
/// caster is on the light's far side of every corner, behind the layer, or
/// throws its shadow wide of it even with the diffusion added.
#[must_use]
pub fn may_shade(receiver: &[[f32; 3]; 4], light: &LightingLight, caster: &CasterPlane) -> bool {
    if light.kind == LightingKind::Ambient || light.shadow_darkness <= 0.0 {
        return false;
    }
    let parallel = light.kind == LightingKind::Parallel;
    let centre = light.corners.iter().fold([0.0; 3], |c, v| {
        [c[0] + v[0] * 0.25, c[1] + v[1] * 0.25, c[2] + v[2] * 0.25]
    });
    let n = cross3(caster.u, caster.v);
    // Which side of the caster the light is on, and whether every corner of
    // the layer is on that side too, so no ray to the light crosses it.
    let towards = if parallel {
        dot3([-light.axis[0], -light.axis[1], -light.axis[2]], n)
    } else {
        dot3(sub3(centre, caster.origin), n)
    };
    let corners = [
        caster.origin,
        [
            caster.origin[0] + caster.u[0],
            caster.origin[1] + caster.u[1],
            caster.origin[2] + caster.u[2],
        ],
        [
            caster.origin[0] + caster.v[0],
            caster.origin[1] + caster.v[1],
            caster.origin[2] + caster.v[2],
        ],
        [
            caster.origin[0] + caster.u[0] + caster.v[0],
            caster.origin[1] + caster.u[1] + caster.v[1],
            caster.origin[2] + caster.u[2] + caster.v[2],
        ],
    ];
    if receiver
        .iter()
        .all(|p| dot3(sub3(*p, caster.origin), n) * towards >= 0.0)
    {
        return false;
    }
    // The layer's own plane, and the caster's corners thrown onto it from
    // the light. A corner that can't be thrown there gives up the test.
    let (r0, a, b) = (
        receiver[0],
        sub3(receiver[1], receiver[0]),
        sub3(receiver[2], receiver[0]),
    );
    let m = cross3(a, b);
    let (cu, cv) = (cross3(b, m), cross3(m, a));
    let (du, dv) = (dot3(a, cu), dot3(b, cv));
    if du.abs() < 1e-12 || dv.abs() < 1e-12 {
        return true;
    }
    let light_side = if parallel {
        -dot3(light.axis, m)
    } else {
        dot3(sub3(centre, r0), m)
    };
    if corners
        .iter()
        .all(|c| dot3(sub3(*c, r0), m) * light_side < 0.0)
    {
        return false;
    }
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    // How far past the caster the shadow lands, in lengths from the light to
    // the caster: its soft edge there is that many times the widest it gets
    // on the caster.
    let mut past = 1.0f32;
    for c in corners {
        let ray = if parallel {
            light.axis
        } else {
            sub3(c, centre)
        };
        let den = dot3(ray, m);
        if den.abs() < 1e-12 {
            return true;
        }
        let s = dot3(sub3(r0, c), m) / den;
        if !s.is_finite() || s <= 0.0 {
            return true;
        }
        if !parallel {
            past = past.max(s);
        }
        let x = sub3(
            [c[0] + ray[0] * s, c[1] + ray[1] * s, c[2] + ray[2] * s],
            r0,
        );
        let uv = [dot3(x, cu) / du, dot3(x, cv) / dv];
        for k in 0..2 {
            lo[k] = lo[k].min(uv[k]);
            hi[k] = hi[k].max(uv[k]);
        }
    }
    // The soft edge reaches about its width past the hard one; half as much
    // again and a couple of pixels, to be safe.
    let len = |v: [f32; 3]| dot3(v, v).sqrt().max(1e-6);
    let pad = light.shadow_diffusion * DIFFUSION_SPREAD * MAX_STRETCH * past * 1.5 + 2.0;
    let pad = [pad / len(a), pad / len(b)];
    (0..2).all(|k| hi[k] >= -pad[k] && lo[k] <= 1.0 + pad[k])
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLight {
    /// w = 1 for a light from before it had rows.
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    c3: [f32; 4],
    /// rgb = colour, w = falloff px.
    colour: [f32; 4],
    /// xyz = spot axis, w = cos(half-angle).
    axis: [f32; 4],
    /// x = kind (0 point, 1 area, 2 parallel, 3 ambient), y = falloff,
    /// z = radius px, w = cos of the feather's inner angle.
    flags: [f32; 4],
    /// x = shadow darkness, y = the widest soft edge on a caster px, z =
    /// which casters can shade this layer, one bit each, w = how far it
    /// stretches on a caster turned from the light.
    shadow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuCaster {
    /// w = Light transmission.
    origin: [f32; 4],
    u: [f32; 4],
    v: [f32; 4],
    /// Where the picture sits in its square: corner, width and height.
    place: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightingParams {
    origin: [f32; 4],
    du: [f32; 4],
    dv: [f32; 4],
    /// xyz = plane normal, w = live light count.
    normal: [f32; 4],
    /// x = ambient, y = diffuse, z = specular, w = highlight exponent.
    material: [f32; 4],
    /// xyz = the eye, w = metal.
    eye: [f32; 4],
    /// x = what the light starts from, y = 1 to light the display-encoded
    /// picture, z = accepts shadows (0 off, 1 on, 2 only), w = this layer's
    /// own caster or -1.
    base: [f32; 4],
    /// x = caster count, y = the side of their squares, z = 1 for a layer
    /// that takes no light, only shadows.
    casters_head: [f32; 4],
    lights: [GpuLight; MAX_LIT_LIGHTS],
    casters: [GpuCaster; MAX_SHADOW_CASTERS],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SquareParams {
    /// The picture's place in the square, texels: corner, width and height.
    place: [u32; 4],
    opacity: f32,
    /// 1 to keep the colour display encoded.
    encoded: u32,
    _pad: [u32; 2],
}

fn v4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

/// The pass's own layouts and pipelines: the lighting kernel, which reads the
/// casters' array through a sampler, and the two that make a caster's square
/// and its smaller levels.
pub(super) struct LightingFx {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    square_layout: wgpu::BindGroupLayout,
    square: wgpu::ComputePipeline,
    halve: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    /// Bound when the frame has no casters, and never read.
    empty: wgpu::Texture,
}

/// The format a caster's square is kept in. Eight bits is plenty for how much
/// of a light a layer stops.
const SQUARE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl LightingFx {
    /// Compiled on the pool with the rest of the engine.
    pub(super) fn start(ctx: &GpuContext) -> crate::Compiling<Self> {
        let device = ctx.device.clone();
        let working = ctx.working();
        crate::Compiling::start(move || Self::new(&device, working))
    }

    fn new(device: &wgpu::Device, working: wgpu::TextureFormat) -> Self {
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
        let store = |binding: u32, format: wgpu::TextureFormat| {
            entry(
                binding,
                wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
            )
        };
        let uniform = entry(
            3,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        );
        use wgpu::TextureViewDimension as D;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-lighting-layout"),
            entries: &[
                tex(0, false, D::D2),
                store(2, working),
                uniform,
                tex(4, true, D::D2Array),
                entry(
                    5,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let square_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-shadow-square-layout"),
            entries: &[tex(0, false, D::D2), store(2, SQUARE_FORMAT), uniform],
        });
        let compute = |layout: &wgpu::BindGroupLayout, wgsl: &str, label: &str, entry: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let square_wgsl = include_str!("../fx_shadow_square.wgsl");
        Self {
            pipeline: compute(
                &layout,
                include_str!("../fx_lighting.wgsl"),
                "fx-lighting",
                "lighting",
            ),
            square: compute(&square_layout, square_wgsl, "fx-shadow-square", "square"),
            halve: compute(&square_layout, square_wgsl, "fx-shadow-halve", "halve"),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("fx-shadow-sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            empty: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fx-shadow-none"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SQUARE_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
            layout,
            square_layout,
        }
    }
}

/// How many levels a caster square of `size` has: down to a single texel, as
/// the oracle's `caster_levels`.
fn levels(size: u32) -> u32 {
    size.max(1).ilog2() + 1
}

impl FxEngine {
    /// Shrink each casting layer's finished picture into its square of one
    /// texture array, with the smaller levels a soft shadow reads, for every
    /// layer's lighting in this frame to share. `size` is the squares' side, a
    /// power of two, `encoded` keeps their colour display encoded for a comp
    /// lit that way, and `spread` is the widest soft edge any light gives
    /// them, comp pixels, which their margins are kept wide enough for.
    /// `None` when there is nothing to cast.
    pub fn shadow_casters(
        &self,
        ctx: &GpuContext,
        casters: &[(&wgpu::Texture, CasterPlane)],
        size: u32,
        encoded: bool,
        spread: f32,
    ) -> Option<ShadowCasters> {
        use wgpu::util::DeviceExt;
        let casters = casters.get(..casters.len().min(MAX_SHADOW_CASTERS))?;
        if casters.is_empty() || size < 8 {
            return None;
        }
        let fx = &self.lighting;
        let count = levels(size);
        let mut bytes = 0u64;
        for k in 0..count {
            bytes += crate::texture_bytes(SQUARE_FORMAT, size >> k, size >> k);
        }
        ctx.charge_vram(bytes * casters.len() as u64);
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx-shadow-casters"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: casters.len() as u32,
            },
            mip_level_count: count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SQUARE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let view = |layer: u32, level: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("fx-shadow-square"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_mip_level: level,
                mip_level_count: Some(1),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        let mut enc = ctx.encoder("fx-shadow-casters");
        let mut places = Vec::with_capacity(casters.len());
        for (layer, (src, plane)) in casters.iter().enumerate() {
            let layer = layer as u32;
            let len = |v: [f32; 3]| dot3(v, v).sqrt();
            let place = caster_box(size, len(plane.u), len(plane.v), spread);
            places.push(place.map(|t| t as f32 / size as f32));
            let ubuf = ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("fx-shadow-square-params"),
                    contents: bytemuck::bytes_of(&SquareParams {
                        place,
                        opacity: plane.opacity.clamp(0.0, 1.0),
                        encoded: u32::from(encoded),
                        _pad: [0; 2],
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            for level in 0..count {
                let input = if level == 0 {
                    src.create_view(&Default::default())
                } else {
                    view(layer, level - 1)
                };
                let output = view(layer, level);
                let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fx-shadow-square"),
                    layout: &fx.square_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&input),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&output),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: ubuf.as_entire_binding(),
                        },
                    ],
                });
                let n = (size >> level).max(1);
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fx-shadow-square"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(if level == 0 { &fx.square } else { &fx.halve });
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(n.div_ceil(8), n.div_ceil(8), 1);
            }
        }
        drop(enc);
        Some(ShadowCasters {
            texture,
            planes: casters.iter().map(|(_, p)| *p).collect(),
            places,
            size,
        })
    }

    /// Shade one layer's linear working texture with the comp's lights,
    /// returning a new texture of the same size. Multiplies the picture by
    /// `1 + light`, so an unlit pixel is untouched; `op.lights` empty
    /// short-circuits inside the kernel to a straight copy. `casters` are the
    /// frame's shadows, for a layer that takes them.
    pub fn lighting(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &LightingOp,
        casters: Option<&ShadowCasters>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let fx = &self.lighting;
        let out = work_texture(ctx, w, h, "fx-lighting-out");
        let mut lights = [GpuLight::default(); MAX_LIT_LIGHTS];
        let live = op.lights.len().min(MAX_LIT_LIGHTS);
        let receiver = op.corners(w, h);
        for (slot, l) in lights.iter_mut().zip(&op.lights) {
            // Only the casters that can reach this layer are read for it.
            let mask = casters.map_or(0u32, |c| {
                c.planes
                    .iter()
                    .enumerate()
                    .filter(|&(i, p)| op.own_caster != Some(i) && may_shade(&receiver, l, p))
                    .fold(0, |m, (i, _)| m | (1 << i))
            });
            *slot = GpuLight {
                c0: v4(l.corners[0], f32::from(u8::from(l.legacy))),
                c1: v4(l.corners[1], 0.0),
                c2: v4(l.corners[2], 0.0),
                c3: v4(l.corners[3], 0.0),
                colour: v4(l.colour, l.falloff_px),
                axis: v4(l.axis, l.cone_cos),
                flags: [
                    f32::from(l.kind as u8),
                    l.falloff as f32,
                    l.radius_px,
                    l.cone_inner,
                ],
                shadow: [
                    l.shadow_darkness,
                    l.shadow_diffusion * DIFFUSION_SPREAD,
                    mask as f32,
                    MAX_STRETCH,
                ],
            };
        }
        let mut planes = [GpuCaster::default(); MAX_SHADOW_CASTERS];
        let (count, size) = casters.map_or((0, 0), |c| (c.planes.len(), c.size));
        let placed = casters.iter().flat_map(|c| c.planes.iter().zip(&c.places));
        for (slot, (c, place)) in planes.iter_mut().zip(placed) {
            *slot = GpuCaster {
                origin: v4(c.origin, c.transmission),
                u: v4(c.u, 0.0),
                v: v4(c.v, 0.0),
                place: *place,
            };
        }
        let m = &op.material;
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lighting-params"),
                contents: bytemuck::bytes_of(&LightingParams {
                    origin: v4(op.origin, 0.0),
                    du: v4(op.du, 0.0),
                    dv: v4(op.dv, 0.0),
                    normal: v4(op.normal, live as f32),
                    material: [m.ambient, m.diffuse, m.specular, m.exponent],
                    eye: v4(m.eye, m.metal),
                    base: [
                        m.base,
                        f32::from(u8::from(m.encoded)),
                        f32::from(m.accepts_shadows as u8),
                        op.own_caster.map_or(-1.0, |i| i as f32),
                    ],
                    casters_head: [count as f32, size as f32, f32::from(u8::from(m.unlit)), 0.0],
                    lights,
                    casters: planes,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let array = casters.map_or(&fx.empty, |c| &c.texture);
        let (v_src, v_out) = (
            src.create_view(&Default::default()),
            out.create_view(&Default::default()),
        );
        let v_array = array.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-lighting"),
            layout: &fx.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&v_src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&v_out),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&v_array),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&fx.sampler),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-lighting");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-lighting"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&fx.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
