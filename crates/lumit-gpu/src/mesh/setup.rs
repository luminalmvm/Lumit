//! The mesh pass's shader, layouts and pipelines, made once when the first model
//! is drawn.

use super::{blur, look, samplers, shadow, MeshFx, COLOUR, INV_W};
use crate::fx::environment;
use crate::GpuContext;
use std::collections::HashMap;
use std::sync::Mutex;
use wgpu::util::DeviceExt;

pub(super) fn texture_entry(
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

pub(super) fn uniform_entry(binding: u32, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
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
    pub(super) fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("../light_core.wgsl"),
                    include_str!("../shadow_core.wgsl"),
                    include_str!("../environment_core.wgsl"),
                    include_str!("../mesh.wgsl")
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
            deform: std::sync::OnceLock::new(),
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
