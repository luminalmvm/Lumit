//! The mesh pass: a 3D model drawn through the comp's camera and lit by the
//! comp's lights, into a picture and a depth the compositor joins with the 3D
//! layers ([`crate::composite::Compositor::composite_joined`]).
//!
//! A model is uploaded once per scene and kept while the scene lives, or
//! until video memory runs short ([`FxEngine::model`]). Nothing here is
//! built until the first model is drawn, so a comp with none never pays for
//! it.

use crate::composite::scaled_size;
use crate::fx::environment::{self, EnvironmentLight};
use crate::fx::lighting::{
    gpu_light, GpuLight, LightingFx, LightingLight, LightingMaterial, LightingShadows,
    ShadowCasters, MAX_LIT_LIGHTS,
};
use crate::fx::FxEngine;
use crate::GpuContext;
use glam::{Mat3, Mat4, Vec3};
use lumit_scene::ModelScene;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use wgpu::util::DeviceExt;

mod blur;
pub mod display;
pub mod lens;
pub mod look;
mod samplers;
pub mod shadow;
mod upload;

/// What the picture is made in: linear, premultiplied, the compositor's own
/// sixteen bits.
const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The depth the compositor reads: one over clip w, nought where empty.
const INV_W: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// The pipeline set that draws edges as lines.
const WIRE: usize = 6;
/// The card's slots each kept material holds, for its numbers and pictures.
const MATERIAL_SLOTS: usize = 8;
/// The most slots kept models hold between them: half of the million DirectX 12
/// has, with the rest for what a frame binds.
const KEPT_SLOTS: usize = 500_000;

/// Why a model can't be drawn.
#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    #[error("there isn't the video memory for this model: {0}")]
    Budget(#[from] lumit_budget::BudgetError),
    #[error("this model is larger than the graphics card can hold in one piece")]
    TooLarge,
}

/// How one model is seen: through which camera, from where, with which
/// lights.
pub struct ModelView<'a> {
    /// The comp's size in pixels, and the share of that the picture is made
    /// at.
    pub comp: (u32, u32),
    pub render_scale: f32,
    /// The comp's anti-aliasing, as the compositor takes it.
    pub samples: u32,
    /// The comp's camera, as [`crate::composite::camera_matrix`] makes it.
    pub camera: Mat4,
    /// Model space (metres, Y up) to comp pixels, the half turn about X
    /// already in it.
    pub model_to_comp: Mat4,
    /// The same at each shutter sample when the model blurs, empty when not.
    pub shutter: &'a [Mat4],
    /// The scene at each of those samples where its shape moves, empty when
    /// it holds still. Each has the materials of the one drawn.
    pub shutter_scenes: &'a [Arc<ModelScene>],
    /// The comp's lights as a 3D layer takes them, at most eight, and the
    /// eye and Accepts lights from the layer's Material options. Its numbers
    /// are left out, as a model's materials come from its file. No lights
    /// draws the model unlit.
    pub lights: &'a [LightingLight],
    pub material: LightingMaterial,
    /// Each copy of the model. Empty draws it once where it is.
    pub instances: &'a [ModelCopy],
    /// The shadows it takes, as `material.accepts_shadows` says: the 3D
    /// layers' casters and the models' maps. Only the look it exports with
    /// reads them.
    pub casters: Option<&'a ShadowCasters>,
    pub maps: Option<&'a shadow::ShadowMaps>,
    /// How the view shows it, which is the look it exports with everywhere
    /// but a Viewer that asks otherwise.
    pub display: display::ModelDisplay,
    /// The comp's environment, which lights it unless Accepts lights is off.
    pub environment: Option<&'a EnvironmentLight>,
    /// The materials its layer's rows change, and the comp's picture under
    /// its run of 3D layers for glass to bend.
    pub looks: &'a [look::Look],
    pub behind: Option<&'a wgpu::Texture>,
}

/// One copy of a model: where it goes after `model_to_comp`, in comp pixels,
/// and the colour its own is multiplied by.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelCopy {
    pub place: [[f32; 4]; 4],
    pub tint: [f32; 4],
}

/// A model's picture at render size, and its depth as one over clip w at
/// each pixel, nought where it shows nothing. A holdout shows what is under
/// its run where it is, with its picture over that.
#[derive(Clone)]
pub struct ModelPicture {
    pub colour: wgpu::Texture,
    pub depth: wgpu::Texture,
    pub holdout: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 4],
    uv0: [f32; 2],
    uv1: [f32; 2],
    colour: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    clip: [[f32; 4]; 4],
    eye: [f32; 4],
    flags: [f32; 4],
    wire: [f32; 4],
    lights: [GpuLight; MAX_LIT_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawUniform {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 3],
}

/// One primitive where one node puts it.
struct Part {
    first: u32,
    count: u32,
    base_vertex: i32,
    material: usize,
    node: Mat4,
    centre: Vec3,
}

/// A model on the graphics card: every primitive in one vertex and one index
/// buffer, a bind group per material, and its charge on the ledger.
pub(crate) struct GpuModel {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    parts: Vec<Part>,
    /// Each material's bind group, whether it blends, whether it is
    /// double-sided and the samplers it reads with. The last is glTF's
    /// default, for parts with none.
    materials: Vec<(wgpu::BindGroup, bool, bool, wgpu::BindGroup)>,
    /// Its edges as lines, made the first time a view draws them.
    edges: OnceLock<(wgpu::Buffer, lumit_budget::Reservation)>,
    kept: Vec<look::Kept>,
    _held: lumit_budget::Reservation,
}

/// The pass's shader, layouts and pipelines, and the models kept for the
/// scenes still alive.
pub(crate) struct MeshFx {
    shader: wgpu::ShaderModule,
    frame_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    shadows_layout: wgpu::BindGroupLayout,
    samplers: samplers::Samplers,
    environment: environment::Fallback,
    layout: wgpu::PipelineLayout,
    /// Draws a shadow map's view ([`shadow`]), made when first asked for.
    shadow: std::sync::OnceLock<wgpu::RenderPipeline>,
    /// The camera's blur ([`lens`]), made when first asked for.
    lens: std::sync::OnceLock<lens::LensPasses>,
    depth_one: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    depth_many: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    /// A picture's next level down from the one above, plain, sRGB and in
    /// the pass's own format.
    down: (
        wgpu::BindGroupLayout,
        [wgpu::RenderPipeline; 3],
        wgpu::Sampler,
    ),
    /// By sample count and kind: opaque and blended, each culled and
    /// double-sided, then depth alone, culled and double-sided, then lines
    /// ([`WIRE`]). Each is made the first time a draw needs it, as making one
    /// takes a while.
    sets: Mutex<HashMap<(u32, usize), Arc<wgpu::RenderPipeline>>>,
    /// Adds a shutter sample's picture to the rest ([`blur`]).
    add: wgpu::RenderPipeline,
    white: wgpu::Texture,
    shared: look::Shared,
    models: Mutex<Vec<(Weak<ModelScene>, Arc<GpuModel>)>>,
    scratch: Mutex<Option<Scratch>>,
}

/// The pass's multisample colour and depth, kept from one draw to the next
/// of the same size, as making them took longer than drawing into them.
/// ponytail: held while the engine lives once a model has been drawn. Let it
/// go when the comp has no model left if that memory is ever missed.
struct Scratch {
    shape: (u32, u32, u32),
    msaa: Option<wgpu::Texture>,
    z: wgpu::Texture,
    held: Option<lumit_budget::Reservation>,
}

fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    multisampled: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

impl MeshFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("light_core.wgsl"),
                    include_str!("shadow_core.wgsl"),
                    include_str!("environment_core.wgsl"),
                    include_str!("mesh.wgsl")
                )
                .into(),
            ),
        });
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let frame_layout = layout(
            "mesh-frame",
            &[
                uniform_entry(0, false),
                uniform_entry(1, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        );
        let mut entries = vec![uniform_entry(0, false)];
        entries.extend((1..=5).map(|b| texture_entry(b, float, false)));
        // The opacity picture, and the picture behind for glass.
        entries.extend([11, 12].map(|b| texture_entry(b, float, false)));
        let material_layout = layout("mesh-material", &entries);
        let shadows_layout = layout("mesh-shadows", &shadow::layout());
        let samplers = samplers::Samplers::new(device);
        let pipeline_layout = |label, layouts: &[&wgpu::BindGroupLayout]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            })
        };
        let resolve = |binding, multisampled, entry| {
            let bgl = layout(
                "mesh-depth",
                &[texture_entry(
                    binding,
                    wgpu::TextureSampleType::Depth,
                    multisampled,
                )],
            );
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh-depth"),
                layout: Some(&pipeline_layout("mesh-depth", &[&bgl])),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(INV_W.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });
            (bgl, pipeline)
        };
        let depth_one = resolve(2, false, "fs_depth_one");
        let depth_many = resolve(3, true, "fs_depth_many");
        let down_layout = layout("mesh-down", &[texture_entry(4, float, false), sampler(5)]);
        let down_pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh-down"),
                layout: Some(&pipeline_layout("mesh-down", &[&down_layout])),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_down"),
                    targets: &[Some(format.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let down = (
            [
                down_pipeline(wgpu::TextureFormat::Rgba8Unorm),
                down_pipeline(wgpu::TextureFormat::Rgba8UnormSrgb),
                down_pipeline(COLOUR),
            ],
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("mesh-down"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        );
        let down = (down_layout, down.0, down.1);
        let add = blur::add_pipeline(device, &shader, &pipeline_layout("mesh-add", &[&down.0]));
        let white = device.create_texture_with_data(
            &ctx.queue,
            &wgpu::TextureDescriptor {
                label: Some("mesh-white"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255; 4],
        );
        Self {
            layout: pipeline_layout(
                "mesh",
                &[
                    &frame_layout,
                    &material_layout,
                    &samplers.layout,
                    &shadows_layout,
                ],
            ),
            shader,
            frame_layout,
            material_layout,
            shadows_layout,
            samplers,
            environment: environment::Fallback::new(device),
            shadow: std::sync::OnceLock::new(),
            lens: std::sync::OnceLock::new(),
            depth_one,
            depth_many,
            down,
            add,
            sets: Mutex::new(HashMap::new()),
            shared: look::Shared::new(&white),
            white,
            models: Mutex::new(Vec::new()),
            scratch: Mutex::new(None),
        }
    }
}

impl MeshFx {
    /// The mesh pipeline of kind `set` at `samples`, made the first time it
    /// is asked for.
    fn pipeline(
        &self,
        device: &wgpu::Device,
        samples: u32,
        set: usize,
    ) -> Arc<wgpu::RenderPipeline> {
        let build = || {
            if set == WIRE {
                return Arc::new(display::wire_pipeline(
                    device,
                    &self.shader,
                    &self.layout,
                    samples,
                ));
            }
            let attributes = wgpu::vertex_attr_array![
                0 => Float32x3, 1 => Float32x3, 2 => Float32x4,
                3 => Float32x2, 4 => Float32x2, 5 => Float32x4,
            ];
            let make = |blend: bool, cull: Option<wgpu::Face>, writes: wgpu::ColorWrites| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("mesh"),
                    layout: Some(&self.layout),
                    vertex: wgpu::VertexState {
                        module: &self.shader,
                        entry_point: Some("vs_mesh"),
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &attributes,
                        }],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some("fs_mesh"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: COLOUR,
                            blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                            write_mask: writes,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: cull,
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
                        ..Default::default()
                    },
                    multiview: None,
                    cache: None,
                })
            };
            let cull = set.is_multiple_of(2).then_some(wgpu::Face::Back);
            let writes = match set {
                4.. => wgpu::ColorWrites::empty(),
                _ => wgpu::ColorWrites::ALL,
            };
            Arc::new(make(set == 2 || set == 3, cull, writes))
        };
        match self.sets.lock() {
            Ok(mut sets) => Arc::clone(sets.entry((samples, set)).or_insert_with(build)),
            Err(_) => build(),
        }
    }

    /// The model kept for `scene`, uploaded now if it has none. Models whose
    /// scenes have gone are let go here too. The lock is never held while
    /// the card is busy.
    fn model_for(
        &self,
        ctx: &GpuContext,
        scene: &Arc<ModelScene>,
    ) -> Result<Arc<GpuModel>, MeshError> {
        let kept = self.models.lock().ok().and_then(|mut models| {
            models.retain(|(s, _)| s.strong_count() > 0);
            // The one drawn last goes to the end, so the front was drawn
            // longest ago.
            let at = models
                .iter()
                .position(|(s, _)| std::ptr::eq(s.as_ptr(), Arc::as_ptr(scene)));
            match at {
                Some(at) => {
                    let kept = models.remove(at);
                    let model = Arc::clone(&kept.1);
                    models.push(kept);
                    Some(model)
                }
                // Short of video memory or binding slots, the older half goes
                // before another comes.
                None if ctx
                    .ledger()
                    .pressure(lumit_budget::Tier::Vram)
                    .should_trim()
                    || models
                        .iter()
                        .map(|(_, m)| m.materials.len() * MATERIAL_SLOTS)
                        .sum::<usize>()
                        >= KEPT_SLOTS =>
                {
                    let half = models.len().div_ceil(2);
                    models.drain(..half);
                    None
                }
                None => None,
            }
        });
        if let Some(model) = kept {
            return Ok(model);
        }
        let model = Arc::new(GpuModel::upload(ctx, self, scene)?);
        if let Ok(mut models) = self.models.lock() {
            models.push((Arc::downgrade(scene), Arc::clone(&model)));
        }
        Ok(model)
    }

    fn draw(
        &self,
        ctx: &GpuContext,
        lighting: &LightingFx,
        models: &[Arc<GpuModel>],
        view: &ModelView<'_>,
    ) -> ModelPicture {
        // Each shutter sample's model, the first for any it has none of.
        let model = &*models[0];
        let at_sample = |k: usize| models.get(k).unwrap_or(&models[0]);
        let (w, h) = view.comp;
        let (tw, th) = scaled_size(w, h, view.render_scale);
        let samples = ctx.cut_sample_count(view.samples).max(1);
        let bytes = |format, samples: u32| {
            crate::texture_bytes(format, tw, th).saturating_mul(u64::from(samples))
        };
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
        let read = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC;
        ctx.charge_vram(bytes(COLOUR, 1) + bytes(INV_W, 1));
        let colour = texture("mesh-colour", COLOUR, 1, read);
        let depth = texture("mesh-inv-w", INV_W, 1, read);
        let shape = (tw, th, samples);
        let kept = self.scratch.lock().ok().and_then(|mut k| k.take());
        let scratch = match kept.filter(|k| k.shape == shape) {
            Some(kept) => kept,
            None => {
                let wanted = bytes(DEPTH, samples)
                    + if samples > 1 {
                        bytes(COLOUR, samples)
                    } else {
                        0
                    };
                let held = ctx.ledger().reserve(lumit_budget::Tier::Vram, wanted).ok();
                if held.is_none() {
                    ctx.charge_vram(wanted);
                }
                Scratch {
                    shape,
                    msaa: (samples > 1)
                        .then(|| texture("mesh-msaa", COLOUR, samples, U::RENDER_ATTACHMENT)),
                    z: texture(
                        "mesh-depth",
                        DEPTH,
                        samples,
                        U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                    ),
                    held,
                }
            }
        };
        let (msaa, z) = (&scratch.msaa, &scratch.z);

        // Comp pixels to clip space, as the compositor maps them.
        let clip = Mat4::from_translation(Vec3::new(-1.0, 1.0, 0.0))
            * Mat4::from_scale(Vec3::new(
                2.0 / w.max(1) as f32,
                -2.0 / h.max(1) as f32,
                0.0,
            ))
            * view.camera;
        let m = &view.material;
        let takes = m.accepts_shadows != LightingShadows::Off && view.display.takes_shadows();
        let (casters, maps) = (view.casters.filter(|_| takes), view.maps.filter(|_| takes));
        let mut lights = [GpuLight::default(); MAX_LIT_LIGHTS];
        for (slot, l) in lights.iter_mut().zip(view.lights) {
            *slot = gpu_light(l, 0xffff, maps);
        }
        let live = view.lights.len().min(MAX_LIT_LIGHTS);
        let (shade, faces) = view.display.flags();
        let frame = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-frame"),
                contents: bytemuck::bytes_of(&FrameUniform {
                    clip: clip.to_cols_array_2d(),
                    eye: [m.eye[0], m.eye[1], m.eye[2], live as f32],
                    flags: [
                        f32::from(u8::from(m.unlit)),
                        shade,
                        faces,
                        f32::from(if takes { m.accepts_shadows as u8 } else { 0 }),
                    ],
                    wire: view.display.wire(),
                    lights,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Opaque first, by material, then what blends, farthest first.
        // ponytail: blended parts sort by their middles, not per triangle.
        let world = |p: &Part| view.model_to_comp * p.node;
        let far = |p: &Part| (view.camera * world(p).transform_point3(p.centre).extend(1.0)).w;
        let materials = self.looked(ctx, model, view.looks, view.behind);
        let look = |p: &Part| {
            materials
                .get(p.material)
                .map_or((false, false), |m| (m.1, m.2))
        };
        let order_of = |model: &'_ GpuModel| {
            let mut order: Vec<usize> = (0..model.parts.len()).collect();
            order.sort_by(|&a, &b| {
                let (a, b) = (&model.parts[a], &model.parts[b]);
                let blends = look(a).0;
                blends.cmp(&look(b).0).then_with(|| {
                    if blends {
                        far(b).total_cmp(&far(a))
                    } else {
                        a.material.cmp(&b.material)
                    }
                })
            });
            order
        };
        let size = std::mem::size_of::<DrawUniform>();
        let stride = wgpu::util::align_to(
            size as u64,
            u64::from(ctx.device.limits().min_uniform_buffer_offset_alignment),
        );
        // The frame's placement, or each shutter sample's.
        let placements = if view.shutter.is_empty() {
            std::slice::from_ref(&view.model_to_comp)
        } else {
            view.shutter
        };
        let parts: usize = (0..placements.len())
            .map(|k| at_sample(k).parts.len())
            .sum();
        let mut bytes = vec![0u8; (stride * parts.max(1) as u64) as usize];
        let mut next = 0;
        let mut draws = Vec::with_capacity(placements.len());
        for (k, placed) in placements.iter().enumerate() {
            let model = at_sample(k);
            let order = order_of(model);
            let mut these = Vec::with_capacity(order.len());
            for p in order.iter().filter_map(|&i| model.parts.get(i)) {
                let at = *placed * p.node;
                let m3 = Mat3::from_mat4(at);
                let det = m3.determinant();
                if !det.is_normal() {
                    continue;
                }
                let n = m3.inverse().transpose();
                let mirrored = det < 0.0;
                let uniform = DrawUniform {
                    model: at.to_cols_array_2d(),
                    normal: [
                        n.x_axis.extend(f32::from(u8::from(mirrored))).to_array(),
                        n.y_axis.extend(0.0).to_array(),
                        n.z_axis.extend(0.0).to_array(),
                    ],
                };
                let offset = next * stride;
                next += 1;
                bytes[offset as usize..][..size].copy_from_slice(bytemuck::bytes_of(&uniform));
                let (blend, double) = look(p);
                // ponytail: a mirrored part is drawn from both sides rather than
                // with its winding turned round.
                // Tinted faces show their backs too.
                let both = double || mirrored || view.display.faces;
                let set = usize::from(blend) * 2 + usize::from(both);
                these.push((p, offset as u32, set));
            }
            draws.push(these);
        }
        let uniforms = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-draws"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let one = ModelCopy {
            place: Mat4::IDENTITY.to_cols_array_2d(),
            tint: [1.0; 4],
        };
        let places = match view.instances {
            [] => std::slice::from_ref(&one),
            many => many,
        };
        let instances = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-instances"),
                contents: bytemuck::cast_slice(places),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let frame_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-frame"),
            layout: &self.frame_layout,
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: instances.as_entire_binding(),
                },
            ],
        });
        let surround = view.environment.filter(|_| !m.unlit);
        let shadows_group = self.shadows_group(ctx, lighting, casters, maps, surround);
        let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (colour_view, z_view, depth_view) = (view_of(&colour), view_of(z), view_of(&depth));
        let msaa_view = msaa.as_ref().map(view_of);
        // With motion blur each sample is drawn on its own and added to the
        // picture at its share.
        let blurred = draws.len() > 1;
        // Depth alone never blends, and is culled as the part is.
        let depth_set = |set: usize| 4 + set % 2;
        let mut pipelines: [Option<Arc<wgpu::RenderPipeline>>; 7] = Default::default();
        let edges = model.edges.get();
        let edges = edges.filter(|_| view.display.draws_wires());
        if edges.is_some() {
            pipelines[WIRE] = Some(self.pipeline(&ctx.device, samples, WIRE));
        }
        for &(_, _, set) in draws.iter().flatten() {
            let wanted = std::iter::once(set).chain(blurred.then(|| depth_set(set)));
            for set in wanted {
                pipelines[set].get_or_insert_with(|| self.pipeline(&ctx.device, samples, set));
            }
        }
        let shutter = blur::Shutter::new(ctx, self, (tw, th), draws.len());
        let target = shutter.as_ref().map_or(&colour_view, |s| &s.view);
        let mesh_pass = |encoder: &mut wgpu::CommandEncoder, label| {
            encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: msaa_view.as_ref().unwrap_or(target),
                        resolve_target: msaa_view.as_ref().map(|_| target),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &z_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                })
                .forget_lifetime()
        };
        let draw_parts = |pass: &mut wgpu::RenderPass<'_>,
                          k: usize,
                          these: &[(&Part, u32, usize)],
                          depth: bool| {
            let at = at_sample(k);
            pass.set_vertex_buffer(0, at.vertices.slice(..));
            pass.set_index_buffer(at.indices.slice(..), wgpu::IndexFormat::Uint32);
            for &(p, offset, set) in these.iter().filter(|_| view.display.draws_faces()) {
                let Some((group, _, _, samplers)) = materials.get(p.material) else {
                    continue;
                };
                let Some(pipeline) = &pipelines[if depth { depth_set(set) } else { set }] else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &frame_group, &[offset]);
                pass.set_bind_group(1, group, &[]);
                pass.set_bind_group(2, samplers, &[]);
                pass.set_bind_group(3, &shadows_group, &[]);
                pass.draw_indexed(
                    p.first..p.first + p.count,
                    p.base_vertex,
                    0..places.len() as u32,
                );
            }
            // Each triangle's edges, two indices for each of its corners.
            let (Some((lines, _)), Some(wire), false) = (edges, &pipelines[WIRE], depth) else {
                return;
            };
            pass.set_index_buffer(lines.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_pipeline(wire);
            for &(p, offset, _) in these {
                let Some((group, _, _, samplers)) = model.materials.get(p.material) else {
                    continue;
                };
                pass.set_bind_group(0, &frame_group, &[offset]);
                pass.set_bind_group(1, group, &[]);
                pass.set_bind_group(2, samplers, &[]);
                pass.set_bind_group(3, &shadows_group, &[]);
                pass.draw_indexed(
                    p.first * 2..(p.first + p.count) * 2,
                    p.base_vertex,
                    0..places.len() as u32,
                );
            }
        };
        let mut encoder = ctx.encoder("mesh");
        for (k, these) in draws.iter().enumerate() {
            draw_parts(&mut mesh_pass(&mut encoder, "mesh"), k, these, false);
            if let Some(shutter) = &shutter {
                shutter.add(self, &mut encoder, &colour_view, k);
            }
        }
        if blurred {
            // The nearest depth of every sample, so where the model meets the
            // scene its smear isn't cut to the outline it has at rest.
            let mut pass = mesh_pass(&mut encoder, "mesh-shutter-depth");
            for (k, these) in draws.iter().enumerate() {
                draw_parts(&mut pass, k, these, true);
            }
        }
        {
            let ((bgl, pipeline), binding) = if samples > 1 {
                (&self.depth_many, 3)
            } else {
                (&self.depth_one, 2)
            };
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mesh-depth"),
                layout: bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(&z_view),
                }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh-depth"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &depth_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        drop(encoder);
        // Kept for the next draw only when the ledger had room for it.
        if scratch.held.is_some() {
            if let Ok(mut k) = self.scratch.lock() {
                *k = Some(scratch);
            }
        }
        ModelPicture {
            colour,
            depth,
            holdout: false,
        }
    }
}

impl MeshFx {
    /// Let the older half of the kept models go.
    fn trim(&self) {
        if let Ok(mut models) = self.models.lock() {
            let half = models.len().div_ceil(2);
            models.drain(..half);
        }
    }
}

impl FxEngine {
    /// Let the older half of the kept models go, for a frame the card is
    /// short of room for.
    pub fn trim_models(&self) {
        if let Some(fx) = self.mesh.get() {
            fx.trim();
        }
    }

    /// One model drawn as `view` sees it, for the compositor to join with the
    /// comp's 3D layers. The scene is uploaded the first time it is drawn and
    /// kept while it is alive, so a later frame only draws.
    pub fn model(
        &self,
        ctx: &GpuContext,
        scene: &Arc<ModelScene>,
        view: &ModelView<'_>,
    ) -> Result<ModelPicture, MeshError> {
        let fx = self.mesh.get_or_init(|| MeshFx::new(ctx));
        let model = fx.model_for(ctx, scene)?;
        if view.display.draws_wires() && model.edges.get().is_none() {
            if let Some(edges) = display::edges(ctx, scene) {
                let _ = model.edges.set(edges);
            }
        }
        // Lines stay on the frame's shape, as they belong to its model.
        let mut models = vec![model];
        if !view.display.draws_wires() {
            for scene in view.shutter_scenes {
                models.push(fx.model_for(ctx, scene)?);
            }
            if models.len() > 1 {
                models.remove(0);
            }
        }
        Ok(fx.draw(ctx, &self.lighting, &models, view))
    }
}
