//! Lens dirt's passes (`fx_lens_dirt.wgsl`): the highlights at a quarter of the
//! size, spread wide, then the dirt drawn and lit at full size.

use std::sync::OnceLock;

use crate::GpuContext;

use super::lens_blur::{bind_layout, scratch, Slot};
use super::{work_texture, FxEngine};

/// One frame's settings, already resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensDirtOp {
    pub seed: u32,
    pub intensity: f32,
    /// How far out of focus the glass is, thousandths of the frame diagonal.
    pub defocus: f32,
    /// The pattern's own scale, 1 as designed.
    pub scale: f32,
    pub dust: f32,
    pub smudges: f32,
    pub haze: f32,
    pub scratches: f32,
    pub droplets: f32,
    pub refraction: f32,
    /// Fine sharp specks.
    pub grit: f32,
    /// Curling fibres.
    pub hairs: f32,
    /// Light on all of the glass, whatever the picture.
    pub ambient: f32,
    /// Draw the lit dirt alone, over black.
    pub alone: bool,
    pub blades: u32,
    pub roundness: f32,
    pub rotation_deg: f32,
    pub threshold: f32,
    /// How far a light reaches, as a share of the frame's diagonal.
    pub spread: f32,
    pub tint: [f32; 3],
    /// The far end of the colours, when `gradient`.
    pub tint_2: [f32; 3],
    /// Each speck picks its colour between `tint` and `tint_2`.
    pub gradient: bool,
    /// How far each speck's hue and brightness stray, 0..1.
    pub variation: f32,
    /// How far the glass bulges, -2..2: 0 flat.
    pub curvature: f32,
    /// 0..1.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    w: u32,
    h: u32,
    qw: u32,
    qh: u32,
    seed: u32,
    blades: u32,
    no_cycle: i32,
    alone: u32,
    k: f32,
    scale: f32,
    rd: f32,
    rotation: f32,
    roundness: f32,
    dust: f32,
    smudges: f32,
    haze: f32,
    scratches: f32,
    droplets: f32,
    refraction: f32,
    intensity: f32,
    threshold: f32,
    sigma_q: f32,
    mix_amt: f32,
    ambient: f32,
    grit: f32,
    hairs: f32,
    curvature: f32,
    variation: f32,
    tint: [f32; 4],
    tint_2: [f32; 4],
    gradient: u32,
    _g1: u32,
    _g2: u32,
    _g3: u32,
}

pub(super) struct LensDirtFx {
    bright: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    blur_h: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    blur_v: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    dirt: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
}

impl LensDirtFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-dirt"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}{}",
                    include_str!("../fx_noise_core.wgsl"),
                    include_str!("../fx_lens_dirt.wgsl")
                )
                .into(),
            ),
        });
        let compute = |label: &str, entry: &str, slots: &[(u32, Slot)]| {
            let bgl = bind_layout(device, label, slots, wgpu::ShaderStages::COMPUTE);
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });
            let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
            (bgl, pipe)
        };
        use wgpu::TextureFormat as F;
        let blur = [
            (3, Slot::Tex),
            (4, Slot::Store(F::Rgba16Float)),
            (1, Slot::Uniform),
        ];
        Self {
            bright: compute(
                "fx-lens-dirt-bright",
                "ld_bright",
                &[
                    (0, Slot::Tex),
                    (2, Slot::Store(F::Rgba16Float)),
                    (1, Slot::Uniform),
                ],
            ),
            blur_h: compute("fx-lens-dirt-blur-h", "ld_blur_h", &blur),
            blur_v: compute("fx-lens-dirt-blur-v", "ld_blur_v", &blur),
            dirt: compute(
                "fx-lens-dirt",
                "ld_dirt",
                &[
                    (0, Slot::Tex),
                    (3, Slot::Tex),
                    (5, Slot::Store(ctx.working())),
                    (1, Slot::Uniform),
                ],
            ),
        }
    }
}

/// Built on first use: most projects never draw any lens dirt.
pub(super) type LazyLensDirt = OnceLock<LensDirtFx>;

impl FxEngine {
    /// Lens dirt over a linear working texture (premultiplied), returning a new
    /// one of the same size.
    pub fn lens_dirt(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &LensDirtOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        use wgpu::TextureUsages as U;
        let fx = self.lens_dirt.get_or_init(|| LensDirtFx::new(ctx));
        let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
        let diag = (w as f32).hypot(h as f32);
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-dirt-params"),
                contents: bytemuck::bytes_of(&Params {
                    w,
                    h,
                    qw,
                    qh,
                    seed: op.seed,
                    blades: op.blades.max(3),
                    no_cycle: 0,
                    alone: u32::from(op.alone),
                    k: diag / 1000.0,
                    scale: op.scale.max(0.01),
                    rd: op.defocus / op.scale.max(0.01),
                    rotation: op.rotation_deg.to_radians(),
                    roundness: op.roundness,
                    dust: op.dust,
                    smudges: op.smudges,
                    haze: op.haze,
                    scratches: op.scratches,
                    droplets: op.droplets,
                    refraction: op.refraction,
                    intensity: op.intensity,
                    threshold: op.threshold,
                    sigma_q: op.spread * diag / 4.0 * 0.25,
                    mix_amt: op.mix,
                    ambient: op.ambient,
                    grit: op.grit,
                    hairs: op.hairs,
                    curvature: op.curvature,
                    variation: op.variation,
                    tint: [op.tint[0], op.tint[1], op.tint[2], 1.0],
                    tint_2: [op.tint_2[0], op.tint_2[1], op.tint_2[2], 1.0],
                    gradient: u32::from(op.gradient),
                    _g1: 0,
                    _g2: 0,
                    _g3: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let quarter = |label| {
            scratch(
                ctx,
                qw,
                qh,
                wgpu::TextureFormat::Rgba16Float,
                U::STORAGE_BINDING | U::TEXTURE_BINDING,
                label,
            )
        };
        let (a, b) = (quarter("fx-lens-dirt-a"), quarter("fx-lens-dirt-b"));
        let out = work_texture(ctx, w, h, "fx-lens-dirt-out");
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (v_src, v_a, v_b, v_out) = (view(src), view(&a), view(&b), view(&out));
        let bind = |bgl: &wgpu::BindGroupLayout, entries: &[(u32, &wgpu::TextureView)]| {
            let mut list: Vec<_> = entries
                .iter()
                .map(|(binding, v)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: wgpu::BindingResource::TextureView(v),
                })
                .collect();
            list.push(wgpu::BindGroupEntry {
                binding: 1,
                resource: ubuf.as_entire_binding(),
            });
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-lens-dirt"),
                layout: bgl,
                entries: &list,
            })
        };
        let bright = bind(&fx.bright.0, &[(0, &v_src), (2, &v_a)]);
        let blur_h = bind(&fx.blur_h.0, &[(3, &v_a), (4, &v_b)]);
        let blur_v = bind(&fx.blur_v.0, &[(3, &v_b), (4, &v_a)]);
        let dirt = bind(&fx.dirt.0, &[(0, &v_src), (3, &v_a), (5, &v_out)]);
        let mut enc = ctx.encoder("fx-lens-dirt");
        for (pipe, group, gw, gh) in [
            (&fx.bright.1, &bright, qw, qh),
            (&fx.blur_h.1, &blur_h, qw, qh),
            (&fx.blur_v.1, &blur_v, qw, qh),
            (&fx.dirt.1, &dirt, w, h),
        ] {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-lens-dirt"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(gw.div_ceil(8), gh.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
