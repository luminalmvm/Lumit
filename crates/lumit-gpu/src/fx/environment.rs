//! Image lighting from an Environment light. Its picture, a panorama twice as
//! wide as it is tall, is wrapped onto a cube with smaller levels, and from
//! that two cubes are made on the card: the specular one, a roughness a
//! level, which reflections read, and the irradiance one, which a matte
//! surface reads. Each is made once for a picture and kept while that picture
//! is asked for, so playing a still HDRI makes them once. A comp with no
//! environment never builds any of this.
//!
//! The filtering follows Bevy 0.17's generated environment maps; its notice
//! is with the shader, `environment.wgsl`.

use crate::fx::FxEngine;
use crate::GpuContext;
use std::sync::{Arc, Mutex};
use wgpu::util::DeviceExt;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The irradiance cube's side: it is soft enough for this to be plenty.
const IRRADIANCE: u32 = 32;
/// The largest specular side, about a 2K panorama's own detail.
const MAX_SIDE: u32 = 512;
/// How many pictures are kept made at once.
const KEPT: usize = 4;

/// One picture's cubes on the card, and their charge on the ledger.
pub struct EnvironmentMaps {
    /// The panorama itself, which the background reads at its own detail.
    picture: wgpu::Texture,
    specular: wgpu::Texture,
    irradiance: wgpu::Texture,
    _held: Option<lumit_budget::Reservation>,
}

/// How a frame is lit by its environment.
#[derive(Clone)]
pub struct EnvironmentLight {
    pub maps: Arc<EnvironmentMaps>,
    /// Scene-linear, the intensity in it.
    pub colour: [f32; 3],
    /// Comp directions into the picture's, a column each.
    pub turn: [[f32; 3]; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuEnvironment {
    colour: [f32; 4],
    turn: [[f32; 4]; 3],
}

impl EnvironmentLight {
    fn gpu(light: Option<&Self>) -> GpuEnvironment {
        let Some(e) = light else {
            return GpuEnvironment::default();
        };
        let [c0, c1, c2] = e.turn;
        let last = e.maps.specular.mip_level_count().saturating_sub(1) as f32;
        GpuEnvironment {
            colour: [e.colour[0], e.colour[1], e.colour[2], 1.0],
            turn: [
                [c0[0], c0[1], c0[2], last],
                [c1[0], c1[1], c1[2], 0.0],
                [c2[0], c2[1], c2[2], 0.0],
            ],
        }
    }
}

/// The four entries a pass lit by an environment takes from `first`: the
/// irradiance and specular cubes, their sampler and the uniform.
pub(crate) fn layout(first: u32, stages: wgpu::ShaderStages) -> [wgpu::BindGroupLayoutEntry; 4] {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding: first + binding,
        visibility: stages,
        ty,
        count: None,
    };
    let cube = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::Cube,
        multisampled: false,
    };
    [
        entry(0, cube),
        entry(1, cube),
        entry(
            2,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        ),
        entry(
            3,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
    ]
}

fn cube_view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

fn sampler(device: &wgpu::Device, label: &str, u: wgpu::AddressMode) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: u,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

fn cube(device: &wgpu::Device, label: &str, side: u32, levels: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 6,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    })
}

/// What a pass binds: the two cubes and the uniform.
pub(crate) struct Views {
    irradiance: wgpu::TextureView,
    specular: wgpu::TextureView,
    uniform: wgpu::Buffer,
}

impl Views {
    /// `light`'s, made for one draw.
    pub(crate) fn of(ctx: &GpuContext, light: &EnvironmentLight) -> Self {
        Self {
            irradiance: cube_view(&light.maps.irradiance),
            specular: cube_view(&light.maps.specular),
            uniform: ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("environment"),
                    contents: bytemuck::bytes_of(&EnvironmentLight::gpu(Some(light))),
                    usage: wgpu::BufferUsages::UNIFORM,
                }),
        }
    }

    /// [`layout`]'s entries.
    pub(crate) fn entries<'a>(
        &'a self,
        first: u32,
        none: &'a Fallback,
    ) -> [wgpu::BindGroupEntry<'a>; 4] {
        [
            wgpu::BindGroupEntry {
                binding: first,
                resource: wgpu::BindingResource::TextureView(&self.irradiance),
            },
            wgpu::BindGroupEntry {
                binding: first + 1,
                resource: wgpu::BindingResource::TextureView(&self.specular),
            },
            wgpu::BindGroupEntry {
                binding: first + 2,
                resource: wgpu::BindingResource::Sampler(&none.sampler),
            },
            wgpu::BindGroupEntry {
                binding: first + 3,
                resource: self.uniform.as_entire_binding(),
            },
        ]
    }
}

/// What a pass binds with no environment, made once: a black cube that is
/// never read and a uniform saying there is none. And the sampler every pass
/// reads the cubes through.
pub(crate) struct Fallback {
    pub(crate) views: Views,
    sampler: wgpu::Sampler,
}

impl Fallback {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let black = cube(device, "environment-none", 1, 1);
        Self {
            views: Views {
                irradiance: cube_view(&black),
                specular: cube_view(&black),
                uniform: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("environment-none"),
                    size: std::mem::size_of::<GpuEnvironment>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                }),
            },
            sampler: sampler(device, "environment", wgpu::AddressMode::ClampToEdge),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Filter {
    roughness: f32,
    samples: u32,
    size: f32,
    pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Background {
    env: GpuEnvironment,
    ray: [[f32; 4]; 3],
    view: [f32; 4],
    raster: [f32; 4],
}

/// The passes that make the cubes and the background, and the cubes made.
pub(crate) struct EnvironmentFx {
    layout: wgpu::BindGroupLayout,
    keep: wgpu::ComputePipeline,
    to_cube: wgpu::ComputePipeline,
    down: wgpu::ComputePipeline,
    radiance: wgpu::ComputePipeline,
    irradiance: wgpu::ComputePipeline,
    background_layout: wgpu::BindGroupLayout,
    background: wgpu::ComputePipeline,
    /// Wraps across a panorama's seam.
    wrap: wgpu::Sampler,
    none: Fallback,
    kept: Mutex<Vec<(u128, Arc<EnvironmentMaps>)>>,
}

impl EnvironmentFx {
    fn new(device: &wgpu::Device) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let texture = |binding, dimension| {
            entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: dimension,
                    multisampled: false,
                },
            )
        };
        let store = |binding, dimension| {
            entry(
                binding,
                wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: FORMAT,
                    view_dimension: dimension,
                },
            )
        };
        let uniform = |binding| {
            entry(
                binding,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )
        };
        let filtering = |binding| {
            entry(
                binding,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            )
        };
        use wgpu::TextureViewDimension as D;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment-filter"),
            entries: &[
                texture(0, D::D2Array),
                filtering(1),
                store(2, D::D2Array),
                uniform(3),
            ],
        });
        let background_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment-background"),
            entries: &[
                texture(4, D::Cube),
                filtering(5),
                store(6, D::D2),
                uniform(7),
                texture(8, D::D2),
                filtering(9),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("environment"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("../environment_core.wgsl"),
                    include_str!("../environment.wgsl")
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
            keep: pipeline(&layout, "keep"),
            to_cube: pipeline(&layout, "to_cube"),
            down: pipeline(&layout, "down"),
            radiance: pipeline(&layout, "radiance"),
            irradiance: pipeline(&layout, "diffuse"),
            background: pipeline(&background_layout, "background"),
            layout,
            background_layout,
            wrap: sampler(device, "environment-wrap", wgpu::AddressMode::Repeat),
            none: Fallback::new(device),
            kept: Mutex::new(Vec::new()),
        }
    }

    /// `picture`'s cubes, made now. Every pass is recorded into the frame's
    /// one batch, so nothing waits on the card here.
    fn make(&self, ctx: &GpuContext, picture: &wgpu::Texture) -> EnvironmentMaps {
        let side = (picture.width() / 4)
            .max(16)
            .next_power_of_two()
            .min(MAX_SIDE);
        let levels = side.ilog2() + 1;
        let chain = |side: u32| -> u64 {
            (0..levels)
                .map(|k| crate::texture_bytes(FORMAT, (side >> k).max(1), (side >> k).max(1)))
                .sum::<u64>()
                * 6
        };
        let (pw, ph) = (picture.width(), picture.height());
        let bytes = 2 * chain(side)
            + crate::texture_bytes(FORMAT, IRRADIANCE, IRRADIANCE) * 6
            + crate::texture_bytes(FORMAT, pw, ph);
        let held = ctx.ledger().reserve(lumit_budget::Tier::Vram, bytes).ok();
        if held.is_none() {
            ctx.charge_vram(bytes);
        }
        let source = cube(&ctx.device, "environment-source", side, levels);
        let specular = cube(&ctx.device, "environment-specular", side, levels);
        let irradiance = cube(&ctx.device, "environment-irradiance", IRRADIANCE, 1);
        // Its own copy, as the picture it was made from goes back to the
        // frame's pool to be drawn over.
        let kept = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("environment-picture"),
            size: wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let array = |t: &wgpu::Texture, base: u32, count: Option<u32>| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: base,
                mip_level_count: count,
                ..Default::default()
            })
        };
        let mut encoder = ctx.encoder("environment");
        let mut pass = |pipeline: &wgpu::ComputePipeline,
                        input: &wgpu::TextureView,
                        sampler: &wgpu::Sampler,
                        output: &wgpu::TextureView,
                        filter: Filter,
                        out_side: (u32, u32, u32)| {
            let uniform = ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("environment-filter"),
                    contents: bytemuck::bytes_of(&filter),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("environment-filter"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(output),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("environment-filter"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            let (x, y, faces) = out_side;
            pass.dispatch_workgroups(x.div_ceil(8), y.div_ceil(8), faces);
        };
        let filter = |roughness: f32, samples: u32| Filter {
            roughness,
            samples,
            size: side as f32,
            pad: 0.0,
        };
        let clamp = &self.none.sampler;
        pass(
            &self.to_cube,
            &array(picture, 0, Some(1)),
            &self.wrap,
            &array(&source, 0, Some(1)),
            filter(0.0, 0),
            (side, side, 6),
        );
        for k in 1..levels {
            pass(
                &self.down,
                &array(&source, k - 1, Some(1)),
                clamp,
                &array(&source, k, Some(1)),
                filter(0.0, 0),
                (side >> k, side >> k, 6),
            );
        }
        let all = array(&source, 0, None);
        for k in 0..levels {
            // Bevy's: a roughness a level, and more samples the rougher.
            let roughness = k as f32 / (levels - 1).max(1) as f32;
            let samples = 32 << (roughness * 4.0) as u32;
            pass(
                &self.radiance,
                &all,
                clamp,
                &array(&specular, k, Some(1)),
                filter(roughness, samples),
                (side >> k, side >> k, 6),
            );
        }
        pass(
            &self.irradiance,
            &all,
            clamp,
            &array(&irradiance, 0, Some(1)),
            filter(1.0, 1024),
            (IRRADIANCE, IRRADIANCE, 6),
        );
        pass(
            &self.keep,
            &array(picture, 0, Some(1)),
            clamp,
            &array(&kept, 0, Some(1)),
            filter(0.0, 0),
            (pw, ph, 1),
        );
        EnvironmentMaps {
            picture: kept,
            specular,
            irradiance,
            _held: held,
        }
    }
}

impl FxEngine {
    fn environment_fx(&self, ctx: &GpuContext) -> &EnvironmentFx {
        self.environment
            .get_or_init(|| EnvironmentFx::new(&ctx.device))
    }

    /// The cubes kept for the picture named `key`, if they are.
    pub fn kept_environment(&self, key: u128) -> Option<Arc<EnvironmentMaps>> {
        let fx = self.environment.get()?;
        let mut kept = fx.kept.lock().ok()?;
        let at = kept.iter().position(|(k, _)| *k == key)?;
        let hit = kept.remove(at);
        let maps = Arc::clone(&hit.1);
        kept.push(hit);
        Some(maps)
    }

    /// `picture`'s cubes, made now and kept under `key` when it has a name,
    /// so the next frame of the same picture only reads them.
    pub fn environment(
        &self,
        ctx: &GpuContext,
        key: Option<u128>,
        picture: &wgpu::Texture,
    ) -> Arc<EnvironmentMaps> {
        let fx = self.environment_fx(ctx);
        let maps = Arc::new(fx.make(ctx, picture));
        if let (Some(key), Ok(mut kept)) = (key, fx.kept.lock()) {
            if kept.len() >= KEPT {
                kept.remove(0);
            }
            kept.push((key, Arc::clone(&maps)));
        }
        maps
    }

    /// The environment behind everything, as a camera sees it: `ray` is the
    /// camera's own directions in comp space, a column each, `zoom` its
    /// zoom, `comp` the comp's size and `blur` how soft, 0 to 1. A picture
    /// at the comp's size scaled by `scale`.
    pub fn environment_background(
        &self,
        ctx: &GpuContext,
        light: &EnvironmentLight,
        (ray, zoom): ([[f32; 3]; 3], f32),
        comp: (u32, u32),
        scale: f32,
        blur: f32,
    ) -> wgpu::Texture {
        let fx = self.environment_fx(ctx);
        let (w, h) = crate::composite::scaled_size(comp.0, comp.1, scale);
        ctx.charge_vram(crate::texture_bytes(FORMAT, w, h));
        let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("environment-background"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let column = |c: [f32; 3]| [c[0], c[1], c[2], 0.0];
        let uniform = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("environment-background"),
                contents: bytemuck::bytes_of(&Background {
                    env: EnvironmentLight::gpu(Some(light)),
                    ray: ray.map(column),
                    view: [comp.0 as f32 * 0.5, comp.1 as f32 * 0.5, zoom, blur],
                    raster: [comp.0 as f32 / w as f32, comp.1 as f32 / h as f32, 0.0, 0.0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let (cube, picture, panorama) = (
            cube_view(&light.maps.specular),
            out.create_view(&Default::default()),
            light.maps.picture.create_view(&Default::default()),
        );
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("environment-background"),
            layout: &fx.background_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&cube),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&fx.none.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&picture),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&panorama),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::Sampler(&fx.wrap),
                },
            ],
        });
        let mut encoder = ctx.encoder("environment-background");
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("environment-background"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&fx.background);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(encoder);
        out
    }
}
