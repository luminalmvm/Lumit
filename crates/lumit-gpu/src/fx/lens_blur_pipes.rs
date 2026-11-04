//! Depth of field pro's passes, built once on first use.

use std::sync::Mutex;

use super::lens_blur::{bind_layout, LensBlurFx, Slot, SCENE_LAYERS_MAX};
use crate::GpuContext;

impl LensBlurFx {
    pub(super) fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-blur"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(ctx.working(), include_str!("../fx_lens_blur.wgsl")).into(),
            ),
        });
        let layout = |label: &str, slots: &[(u32, Slot)], vis: wgpu::ShaderStages| {
            bind_layout(device, label, slots, vis)
        };
        let compute = |label: &str, entry: &str, slots: &[(u32, Slot)]| {
            let bgl = layout(label, slots, wgpu::ShaderStages::COMPUTE);
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
        let prep = compute(
            "fx-lens-blur-prep",
            "lb_prep",
            &[
                (0, Slot::Tex),
                (1, Slot::Tex),
                (2, Slot::Store(F::Rgba32Float)),
                (3, Slot::Uniform),
                (11, Slot::Spans),
            ],
        );
        let tiles = compute(
            "fx-lens-blur-tiles",
            "lb_tiles",
            &[
                (4, Slot::Tex),
                (5, Slot::Store(F::R32Float)),
                (3, Slot::Uniform),
            ],
        );
        let dilate = compute(
            "fx-lens-blur-dilate",
            "lb_dilate",
            &[
                (6, Slot::Tex),
                (5, Slot::Store(F::R32Float)),
                (3, Slot::Uniform),
            ],
        );
        let gather = compute(
            "fx-lens-blur-gather",
            "lb_gather",
            &[
                (0, Slot::Tex),
                (1, Slot::Tex),
                (4, Slot::Tex),
                (6, Slot::Tex),
                (7, Slot::Tex),
                (8, Slot::Store(F::Rgba16Float)),
                (3, Slot::Uniform),
                (12, Slot::Uniform),
            ],
        );
        let smooth = compute(
            "fx-lens-blur-smooth",
            "lb_smooth",
            &[
                (4, Slot::Tex),
                (9, Slot::Tex),
                (8, Slot::Store(F::Rgba16Float)),
                (3, Slot::Uniform),
            ],
        );
        let finish = compute(
            "fx-lens-blur-finish",
            "lb_finish",
            &[
                (0, Slot::Tex),
                (4, Slot::Tex),
                (9, Slot::Tex),
                (10, Slot::Store(ctx.working())),
                (3, Slot::Uniform),
            ],
        );
        let ray_min1 = compute(
            "fx-lens-blur-ray-min1",
            "lb_ray_min1",
            &[
                (4, Slot::Tex),
                (16, Slot::Store(F::R32Float)),
                (15, Slot::StorageRw),
                (3, Slot::Uniform),
            ],
        );
        let ray_min2 = compute(
            "fx-lens-blur-ray-min2",
            "lb_ray_min2",
            &[
                (17, Slot::Tex),
                (18, Slot::Store(F::R32Float)),
                (3, Slot::Uniform),
            ],
        );
        let ray_setup = compute(
            "fx-lens-blur-ray-setup",
            "lb_ray_setup",
            &[
                (1, Slot::Tex),
                (3, Slot::Uniform),
                (13, Slot::Spans),
                (14, Slot::StorageRw),
                (20, Slot::Tex),
            ],
        );
        let ray_invert = compute(
            "fx-lens-blur-ray-invert",
            "lb_ray_invert",
            &[(13, Slot::Spans), (14, Slot::StorageRw)],
        );
        let ray_back = compute(
            "fx-lens-blur-ray-back",
            "lb_ray_back",
            &[
                (0, Slot::Tex),
                (3, Slot::Uniform),
                (4, Slot::Tex),
                (6, Slot::Tex),
                (22, Slot::Store(F::Rgba16Float)),
                (24, Slot::Store(F::R32Float)),
            ],
        );
        let ray_trace = compute(
            "fx-lens-blur-ray-trace",
            "lb_ray_trace",
            &[
                (0, Slot::Tex),
                (3, Slot::Uniform),
                (4, Slot::Tex),
                (12, Slot::Uniform),
                (13, Slot::Spans),
                (14, Slot::StorageRw),
                (15, Slot::StorageRw),
                (17, Slot::Tex),
                (19, Slot::Tex),
                (20, Slot::Tex),
                (21, Slot::StorageRw),
                (23, Slot::Tex),
                (25, Slot::Tex),
            ],
        );
        let mut scene_slots = vec![
            (3, Slot::Uniform),
            (12, Slot::Uniform),
            (13, Slot::Spans),
            (14, Slot::StorageRw),
            (20, Slot::Tex),
            (21, Slot::StorageRw),
            (26, Slot::Spans),
            (39, Slot::Tex),
        ];
        scene_slots.extend((0..SCENE_LAYERS_MAX as u32).map(|i| (27 + i, Slot::Tex)));
        let ray_scene = compute("fx-lens-blur-ray-scene", "lb_ray_scene", &scene_slots);
        let ray_resolve = compute(
            "fx-lens-blur-ray-resolve",
            "lb_ray_resolve",
            &[
                (0, Slot::Tex),
                (3, Slot::Uniform),
                (8, Slot::Store(F::Rgba16Float)),
                (12, Slot::Uniform),
                (21, Slot::StorageRw),
            ],
        );
        let sprite_bgl = layout(
            "fx-lens-blur-sprite",
            &[
                (0, Slot::Tex),
                (4, Slot::Tex),
                (7, Slot::Tex),
                (3, Slot::Uniform),
                (12, Slot::Uniform),
            ],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let sprite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-lens-blur-sprite"),
            bind_group_layouts: &[&sprite_bgl],
            push_constant_ranges: &[],
        });
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let sprite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fx-lens-blur-sprite"),
            layout: Some(&sprite_pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("lb_sprite_vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("lb_sprite_fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: F::Rgba16Float,
                    blend: Some(wgpu::BlendState {
                        color: add,
                        alpha: add,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            // Never multisampled: draw order alone fixes the sum, so a frame
            // adds up the same every time.
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self {
            prep,
            tiles,
            dilate,
            gather,
            smooth,
            sprite: (sprite_bgl, sprite),
            finish,
            ray_min1,
            ray_min2,
            ray_setup,
            ray_invert,
            ray_back,
            ray_trace,
            ray_scene,
            ray_resolve,
            cache: Mutex::new(Vec::new()),
            ray_cache: Mutex::new(Vec::new()),
        }
    }
}
