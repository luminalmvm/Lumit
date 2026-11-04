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

/// Where the haze is seen from and the lights that show in it, all in comp
/// pixels: the oracle's `HazeView` plus what the raster needs.
#[derive(Debug, Clone, PartialEq)]
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

/// The two passes: the walk at half size, and laying it over the picture.
pub(super) struct HazeFx {
    march_layout: wgpu::BindGroupLayout,
    march: wgpu::ComputePipeline,
    apply_layout: wgpu::BindGroupLayout,
    apply: wgpu::ComputePipeline,
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
        let working = ctx.working();
        Self {
            compiling: std::sync::Mutex::new(Some(crate::compiling::Compiling::start(move || {
                HazeFx::new(&device, working)
            }))),
            ready: std::sync::OnceLock::new(),
        }
    }

    fn get(&self, ctx: &GpuContext) -> &HazeFx {
        self.ready.get_or_init(|| {
            let started = self.compiling.lock().ok().and_then(|mut c| c.take());
            started.map_or_else(
                || HazeFx::new(&ctx.device, ctx.working()),
                crate::compiling::Compiling::wait,
            )
        })
    }
}

impl HazeFx {
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
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[layout],
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
            march: pipeline(&march_layout, "haze_march"),
            apply: pipeline(&apply_layout, "haze_apply"),
            march_layout,
            apply_layout,
        }
    }
}

fn v4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

impl FxEngine {
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
                    view: [op.near, 0.0, STEPS as f32, 0.0],
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
        let (w, h) = (depth.width().div_ceil(2), depth.height().div_ceil(2));
        let out = work_texture(ctx, w, h, "fx-haze-scatter");
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
                binding: 2,
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
            layout: &fx.march_layout,
            entries: &entries,
        });
        let mut enc = ctx.encoder("fx-haze-march");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-haze-march"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&fx.march);
            pass.set_bind_group(0, &bind, &[]);
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::fx::common::{readback_linear_f32, upload_linear_f32};
    use crate::fx::lighting::{CasterPlane, LightingKind, LightingLight};

    /// The walk must agree with `lumit_core::haze::march`, and a spot's beam
    /// must show where its cone is and be cut below a card that casts.
    #[test]
    fn haze_matches_the_oracle_and_a_caster_cuts_the_beam() {
        use lumit_core::haze::{jitter, march, HazeLook, HazeView};
        use lumit_core::lighting::{
            caster_box, caster_square, ShadingKind, ShadingLight, ShadowCaster,
        };
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let (w, h) = (64u32, 48u32);
        // A spot overhead, aimed straight down through the middle.
        let at = [32.0, -40.0, 60.0];
        let (cone_cos, cone_inner) = (20f32.to_radians().cos(), 15f32.to_radians().cos());
        let spot = ShadingLight {
            corners: [at; 4],
            colour: [1.0, 0.9, 0.7],
            kind: ShadingKind::Point,
            falloff: lumit_core::model::LightFalloff::None,
            radius_px: 500.0,
            falloff_px: 0.0,
            legacy: false,
            cone_cos,
            cone_inner,
            axis: [0.0, 1.0, 0.0],
            shadow_darkness: 1.0,
            shadow_diffusion: 0.0,
        };
        let look = HazeLook {
            amount: 100.0,
            density: 100.0,
            colour: [1.0; 3],
            falloff: 40.0,
            scatter: 30.0,
            noise: 50.0,
            noise_size: 20.0,
            drift: 3.0,
        };
        let gpu_light = LightingLight {
            corners: [at; 4],
            colour: spot.colour,
            kind: LightingKind::Point,
            falloff: 0,
            radius_px: 500.0,
            falloff_px: 0.0,
            legacy: false,
            cone_cos,
            cone_inner,
            axis: spot.axis,
            shadow_darkness: 1.0,
            shadow_diffusion: 0.0,
        };
        let view = HazeView {
            eye: [32.0, 24.0, -100.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            zoom: 100.0,
            comp: (w as f32, h as f32),
            centre: [32.0, 24.0, 60.0],
            radius: 300.0,
        };
        let op = super::HazeOp {
            eye: view.eye,
            right: view.right,
            up: view.up,
            forward: view.forward,
            zoom: view.zoom,
            comp: view.comp,
            scale: 1.0,
            near: 0.0,
            centre: view.centre,
            radius: view.radius,
            lights: vec![(
                gpu_light,
                super::Haze {
                    amount: look.amount,
                    density: look.density,
                    falloff: look.falloff,
                    scatter: look.scatter,
                    noise: look.noise,
                    noise_size: look.noise_size,
                    drift: look.drift,
                },
            )],
        };
        // Nothing in the scene, so every ray runs out of the haze.
        let far = 60000.0;
        let depth = upload_linear_f32(&ctx, &[far, 0.0, 0.0, 1.0].repeat((w * h) as usize), w, h);
        // A flat card across the beam, below the light.
        let plane = CasterPlane {
            origin: [12.0, 10.0, 20.0],
            u: [40.0, 0.0, 0.0],
            v: [0.0, 0.0, 80.0],
            transmission: 0.0,
            opacity: 1.0,
        };
        let card = [0.5, 0.5, 0.5, 1.0].repeat(16);
        let size = 64;
        let tex = upload_linear_f32(&ctx, &card, 4, 4);
        let squares = fx.shadow_casters(&ctx, &[(&tex, plane)], size, false, 0.0);
        let place = caster_box(size, 40.0, 80.0, 0.0);
        let cpu_casters = [ShadowCaster {
            origin: plane.origin,
            u: plane.u,
            v: plane.v,
            transmission: 0.0,
            place: place.map(|t| t as f32 / size as f32),
            levels: caster_square(&card, 4, 4, 1.0, size, place, false),
            size,
        }];
        let (hw, hh) = (w / 2, h / 2);
        let run = |cut: bool| {
            let s = fx.haze_scatter(
                &ctx,
                &depth,
                &op,
                cut.then_some(squares.as_ref()).flatten(),
                None,
            );
            readback_linear_f32(&ctx, &s, hw, hh).unwrap()
        };
        let (open, shut) = (run(false), run(true));
        for (gpu, casters) in [(&open, &[][..]), (&shut, &cpu_casters[..])] {
            let mut worst = 0.0f32;
            for y in 0..hh {
                for x in 0..hw {
                    let q = [(x * 2) as f32 + 0.5, (y * 2) as f32 + 0.5];
                    let cpu = march(&view, q, far, jitter(x, y), &[(spot, look)], casters);
                    let i = ((y * hw + x) * 4) as usize;
                    for k in 0..4 {
                        worst = worst.max((cpu[k] - gpu[i + k]).abs() / cpu[k].abs().max(0.05));
                    }
                }
            }
            assert!(worst < 0.02, "worst relative difference {worst}");
        }
        let lum = |img: &[f32], x: u32, y: u32| {
            let i = ((y * hw + x) * 4) as usize;
            img[i] + img[i + 1] + img[i + 2]
        };
        // Above the card the beam shows and the card changes nothing; below
        // it the card has cut the beam away; well off to the side there is no
        // beam at all.
        let (above, below) = ((16, 2), (16, 20));
        assert!(lum(&open, above.0, above.1) > 0.05);
        assert!((lum(&shut, above.0, above.1) - lum(&open, above.0, above.1)).abs() < 1e-3);
        assert!(lum(&open, below.0, below.1) > 0.05);
        assert!(lum(&shut, below.0, below.1) < 0.2 * lum(&open, below.0, below.1));
        assert!(lum(&open, 1, 12) < 0.1 * lum(&open, above.0, above.1));
    }
}
