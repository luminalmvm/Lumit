//! The compositor seed (evaluator v0): transformed, opacity-blended layer
//! quads rendered bottom-up into the linear fp16 working format.
//!
//! In plain terms: each layer is a picture on a piece of glass; the
//! compositor stacks the glass. Position/scale/rotation move the glass (as a
//! full 4×4 matrix so 3D later needs no rewrite), opacity fades it, and the
//! stacking maths happens in linear light where "add two lights" is physically
//! correct — the same working format the colour golden test locks.

use crate::{ColourEngine, GpuContext, WORKING_FORMAT};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerUniform {
    matrix: [[f32; 4]; 4],
    /// opacity, use_matte, matte_luma, matte_inverted
    params: [f32; 4],
    /// comp target size (xy) + padding
    target: [f32; 4],
}

/// Composite operator (linear subset — docs/06-RENDER-PIPELINE.md §blend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Normal,
    Add,
    Multiply,
    /// Shader-computed via the dst snapshot (perceptual — 06 §blend domains).
    Screen,
    Overlay,
    SoftLight,
    HardLight,
    /// Per-channel max/min: domain-invariant, computed in linear (06 §blend
    /// domains) — snapshot path so opacity and mattes mix correctly.
    Lighten,
    Darken,
    /// dst − src per channel, clamped at black — the photographic subtract
    /// (GEN-1, K-151), computed in linear (Add's darkening twin). Snapshot
    /// path so opacity and mattes mix correctly.
    Subtract,
}

impl Blend {
    /// True for blends the fragment computes itself from a dst snapshot.
    fn uses_snapshot(self) -> bool {
        !matches!(self, Blend::Normal | Blend::Add | Blend::Multiply)
    }

    /// Shader selector (composite.wgsl blend_encoded / fs_layer_snapshot).
    fn snapshot_mode(self) -> f32 {
        match self {
            Blend::Screen => 0.0,
            Blend::Overlay => 1.0,
            Blend::SoftLight => 2.0,
            Blend::HardLight => 3.0,
            Blend::Lighten => 4.0,
            Blend::Darken => 5.0,
            Blend::Subtract => 6.0,
            Blend::Normal | Blend::Add | Blend::Multiply => -1.0,
        }
    }
}

/// A comp-space matte gating a layer (docs/06-RENDER-PIPELINE.md mattes).
pub struct MatteInput<'a> {
    /// The matte layer rendered alone at comp size (linear fp16).
    pub texture: &'a wgpu::Texture,
    /// Luma matte (else alpha).
    pub luma: bool,
    pub inverted: bool,
}

/// One layer to draw: a linear texture plus its placement in comp space.
pub struct CompositeLayer<'a> {
    /// Linear-light texture (run sources through ColourEngine::linearise).
    pub texture: &'a wgpu::Texture,
    /// Layer-pixel size the transform applies to (usually the texture size).
    pub size: (f32, f32),
    /// Comp-space placement: position of the layer's anchor in comp pixels,
    /// anchor point in layer pixels, scale in percent, rotation in degrees.
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    /// 0..100 (UI percent; folded to 0..1 in the uniform).
    pub opacity: f32,
    pub matte: Option<MatteInput<'a>>,
    pub blend: Blend,
    /// 2.5D placement (K-023): z position and x/y rotations, honoured when
    /// the comp provides a camera.
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
    pub three_d: bool,
    /// Layer-space mask coverage (alpha channel), for GPU-sourced layers
    /// whose masks cannot be applied CPU-side. None = fully visible.
    pub layer_mask: Option<&'a wgpu::Texture>,
    /// A parent placement multiplied in front of this layer's own (collapsed
    /// Precomp layers, docs/06 §1.4): the inner layer places in its nested
    /// comp's pixels, then `pre` carries it into the parent comp's pixels.
    /// Column-major 4×4 from [`place_matrix`]. None = placed directly.
    pub pre: Option<[[f32; 4]; 4]>,
}

/// One sub-frame placement for per-layer motion blur (docs/06 §4, K-120): the
/// layer's own transform re-evaluated at one shutter sample time. The layer's
/// SAME texture is drawn at each of these placements and the draws averaged
/// ([`Compositor::motion_blur_average`]), so the layer smears along its own
/// motion. Carries only the per-sample transform; the layer's `three_d`,
/// parent placement (`pre`) and camera are the same for every sample and are
/// passed to the averaging helper separately.
#[derive(Debug, Clone, Copy)]
pub struct MbSample {
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
}

/// A layer transform as a comp-pixel placement matrix — the single source of
/// truth for how (position, anchor, scale %, rotations, z) become a 4×4:
/// `T(pos, z) · Ry · Rx · Rz · S(scale/100) · T(−anchor)`. Public so the
/// draw-list builder can concatenate parent placements for collapsed Precomp
/// layers with exactly the compositor's maths.
#[allow(clippy::too_many_arguments)]
pub fn place_matrix(
    position: (f32, f32),
    anchor: (f32, f32),
    scale: (f32, f32),
    rotation_deg: f32,
    z: f32,
    rotation_x_deg: f32,
    rotation_y_deg: f32,
) -> [[f32; 4]; 4] {
    (Mat4::from_translation(glam::vec3(position.0, position.1, z))
        * Mat4::from_rotation_y(rotation_y_deg.to_radians())
        * Mat4::from_rotation_x(rotation_x_deg.to_radians())
        * Mat4::from_rotation_z(rotation_deg.to_radians())
        * Mat4::from_scale(glam::vec3(scale.0 / 100.0, scale.1 / 100.0, 1.0))
        * Mat4::from_translation(glam::vec3(-anchor.0, -anchor.1, 0.0)))
    .to_cols_array_2d()
}

/// Concatenate two placements: `outer` applied after `inner` (matrix product
/// `outer · inner`), for chains of collapsed Precomp layers.
pub fn concat_place(outer: [[f32; 4]; 4], inner: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    (Mat4::from_cols_array_2d(&outer) * Mat4::from_cols_array_2d(&inner)).to_cols_array_2d()
}

impl CompositeLayer<'_> {
    /// comp pixel space → NDC, with the layer transform applied.
    /// Full 4×4 (K-023). Order: quad(0..1) → layer px → −anchor → scale →
    /// rotate → +position → (parent `pre`, when collapsed) → NDC.
    fn matrix(&self, comp_w: f32, comp_h: f32, camera: Option<&Mat4>) -> Mat4 {
        let ndc_from_comp = Mat4::from_translation(glam::vec3(-1.0, 1.0, 0.0))
            * Mat4::from_scale(glam::vec3(2.0 / comp_w, -2.0 / comp_h, 1.0));
        let mut place = Mat4::from_cols_array_2d(&place_matrix(
            self.position,
            self.anchor,
            self.scale,
            self.rotation_deg,
            self.z,
            self.rotation_x_deg,
            self.rotation_y_deg,
        ));
        if let Some(pre) = &self.pre {
            place = Mat4::from_cols_array_2d(pre) * place;
        }
        let quad_to_px = Mat4::from_scale(glam::vec3(self.size.0, self.size.1, 1.0));
        match camera {
            Some(view_proj) if self.three_d => ndc_from_comp * *view_proj * place * quad_to_px,
            _ => ndc_from_comp * place * quad_to_px,
        }
    }
}

/// f32 → IEEE half bits (enough for writing the constant white texel).
fn half_bits(v: f32) -> u16 {
    // 1.0 and 0.0 are the only values we write; exact per IEEE 754.
    if v >= 1.0 {
        0x3C00
    } else {
        0
    }
}

/// Build the comp-space camera matrix (view * perspective) from the AE
/// model: the camera sits `zoom` px back from its position, and the z=0
/// plane maps 1:1 when the camera is at the comp centre with no rotation.
pub fn camera_matrix(
    comp_w: f32,
    comp_h: f32,
    zoom: f32,
    position: (f32, f32, f32),
    rotation_deg: (f32, f32, f32),
) -> Mat4 {
    let zoom = zoom.max(1.0);
    // Perspective in comp space: x' = cx + (x-cx)·zoom/(z+zoom), with the
    // homogeneous divide doing the work (w = (z+zoom)/zoom).
    let (cx, cy) = (comp_w * 0.5, comp_h * 0.5);
    let persp = Mat4::from_cols_array_2d(&[
        // column-major: each inner array is one column
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        // z output is flattened to 0: layer order is painter's (timeline
        // stacking), so depth only feeds the divide, never the depth test.
        [cx / zoom, cy / zoom, 0.0, 1.0 / zoom],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    // View: undo the camera's own placement (rotate about its position).
    // cam_place maps "default camera at the comp centre" to the actual pose,
    // so its inverse is the identity when the camera hasn't moved.
    let cam_place = Mat4::from_translation(glam::vec3(position.0, position.1, position.2))
        * Mat4::from_rotation_y(rotation_deg.1.to_radians())
        * Mat4::from_rotation_x(rotation_deg.0.to_radians())
        * Mat4::from_rotation_z(rotation_deg.2.to_radians())
        * Mat4::from_translation(glam::vec3(-cx, -cy, 0.0));
    persp * cam_place.inverse()
}

pub struct Compositor {
    pipeline: wgpu::RenderPipeline,
    pipeline_add: wgpu::RenderPipeline,
    pipeline_multiply: wgpu::RenderPipeline,
    pipeline_snapshot: wgpu::RenderPipeline,
    /// Pure-additive blend (BOTH colour and alpha add), used only to
    /// accumulate the per-layer motion-blur sub-frame copies into a true
    /// premultiplied average — distinct from `pipeline_add` (the Add blend
    /// mode), which over-composites alpha (docs/06 §4, K-120).
    pipeline_accumulate: wgpu::RenderPipeline,
    /// Pure-additive blend over a PREMULTIPLIED-passthrough fragment
    /// (`fs_accumulate`), for accumulation motion blur (docs/08 §3.26): the
    /// inputs are already-premultiplied comp composites, so it scales each by its
    /// weight without re-premultiplying by alpha (as `pipeline_accumulate`'s
    /// `fs_layer` would). See [`Self::accumulate`].
    pipeline_premul_accumulate: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bound at binding 3 when a layer has no matte.
    white: wgpu::Texture,
    /// Bound at binding 4 when a layer needs no dst snapshot.
    black: wgpu::Texture,
}

impl Compositor {
    pub fn new(ctx: &GpuContext) -> Self {
        let shader = ctx
            .device
            .create_shader_module(wgpu::include_wgsl!("composite.wgsl"));
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("composite-layer"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("composite"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        // Linear-light blend states (docs/06-RENDER-PIPELINE.md §blend):
        // Normal = premultiplied over; Add = pure light addition; Multiply
        // via DstColor (with over-style alpha accumulation).
        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let blend = wgpu::BlendState {
            color: over,
            alpha: over,
        };
        let blend_add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: over,
        };
        let blend_multiply = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: over,
        };
        let make_pipeline = |state: wgpu::BlendState, label: &str| {
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_layer"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_layer"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: WORKING_FORMAT,
                            blend: Some(state),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview: None,
                    cache: None,
                })
        };
        // Pure add on both channels: the motion-blur accumulator wants the
        // arithmetic mean of the premultiplied sub-frames, so alpha must add
        // (weight 1/N each), not over-composite the way Add does.
        let blend_accumulate = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = make_pipeline(blend, "composite-normal");
        let pipeline_add = make_pipeline(blend_add, "composite-add");
        let pipeline_multiply = make_pipeline(blend_multiply, "composite-multiply");
        let pipeline_accumulate = make_pipeline(blend_accumulate, "composite-mb-accumulate");
        // Additive blend over the premultiplied-passthrough fragment, for
        // accumulation motion blur (docs/08 §3.26): the sub-frame comp
        // composites are already premultiplied, so fs_accumulate scales each by
        // its weight without re-premultiplying (which fs_layer would).
        let pipeline_premul_accumulate =
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("composite-accumulate-premul"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_layer"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_accumulate"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: WORKING_FORMAT,
                            blend: Some(blend_accumulate),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview: None,
                    cache: None,
                });
        // Snapshot blends: no fixed-function blending — the fragment
        // composites itself from the dst snapshot and writes the final value.
        let pipeline_snapshot =
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("composite-snapshot"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_layer"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_layer_snapshot"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: WORKING_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview: None,
                    cache: None,
                });
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("composite-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("matte-none"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::WORKING_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ones = [1.0f32; 4].map(half_bits);
        ctx.queue.write_texture(
            white.as_image_copy(),
            bytemuck::cast_slice(&ones),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let black = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dst-none"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::WORKING_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            pipeline,
            pipeline_add,
            pipeline_multiply,
            pipeline_snapshot,
            pipeline_accumulate,
            pipeline_premul_accumulate,
            layout,
            sampler,
            white,
            black,
        }
    }

    /// Render layers bottom-up over a linear background colour; returns the
    /// linear fp16 comp frame (feed to ColourEngine::display for the screen).
    pub fn composite(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompositeLayer<'_>],
    ) -> wgpu::Texture {
        self.composite_with_camera(ctx, width, height, background, layers, None)
    }

    /// As [`Self::composite`], with a comp-space camera matrix applied to
    /// 3D-switched layers (the AE 2.5D model — docs/03-DATA-MODEL.md §9.3).
    pub fn composite_with_camera(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompositeLayer<'_>],
        camera: Option<Mat4>,
    ) -> wgpu::Texture {
        self.composite_seeded(ctx, width, height, background, layers, camera, None)
    }

    /// As [`Self::composite_with_camera`], but when `seed` is given the
    /// target starts as a copy of it instead of the cleared background —
    /// the continuation half of adjustment-layer staging (docs/06 §1.5):
    /// the layers above an adjustment composite onto the blended
    /// intermediate exactly as they would have onto the live accumulation,
    /// with no intervening resample. `seed` must be a comp-sized working
    /// texture (the previous stage's output).
    #[allow(clippy::too_many_arguments)]
    pub fn composite_seeded(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompositeLayer<'_>],
        camera: Option<Mat4>,
        seed: Option<&wgpu::Texture>,
    ) -> wgpu::Texture {
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("comp-frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORKING_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        // Snapshot of the accumulation for shader-computed blends: one
        // per-frame scratch, copied into just before each such layer draws.
        let needs_snapshot = layers.iter().any(|l| l.blend.uses_snapshot());
        let snapshot = needs_snapshot.then(|| {
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("comp-dst-snapshot"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WORKING_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });

        // Per-layer bind groups first (uniforms are tiny; pooling later).
        let binds: Vec<wgpu::BindGroup> = layers
            .iter()
            .map(|layer| {
                let uniform = LayerUniform {
                    matrix: layer
                        .matrix(width as f32, height as f32, camera.as_ref())
                        .to_cols_array_2d(),
                    params: [
                        (layer.opacity / 100.0).clamp(0.0, 1.0),
                        f32::from(layer.matte.is_some()),
                        f32::from(layer.matte.as_ref().is_some_and(|m| m.luma)),
                        f32::from(layer.matte.as_ref().is_some_and(|m| m.inverted)),
                    ],
                    target: [
                        width as f32,
                        height as f32,
                        layer.blend.snapshot_mode(),
                        0.0,
                    ],
                };
                let buffer = wgpu::util::DeviceExt::create_buffer_init(
                    &ctx.device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("layer-uniform"),
                        contents: bytemuck::bytes_of(&uniform),
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("composite-layer"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &layer.texture.create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(
                                &layer
                                    .matte
                                    .as_ref()
                                    .map(|m| m.texture)
                                    .unwrap_or(&self.white)
                                    .create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(
                                &layer
                                    .layer_mask
                                    .unwrap_or(&self.white)
                                    .create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(
                                &if layer.blend.uses_snapshot() {
                                    snapshot.as_ref().unwrap_or(&self.black)
                                } else {
                                    &self.black
                                }
                                .create_view(&Default::default()),
                            ),
                        },
                    ],
                })
            })
            .collect();

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("composite"),
            });

        // Draw in pass segments: shader-computed blends need the accumulated
        // target copied out first (a copy cannot happen inside a pass).
        let clear = wgpu::LoadOp::Clear(wgpu::Color {
            r: background[0],
            g: background[1],
            b: background[2],
            a: background[3],
        });
        // A seed replaces the clear: the target starts as the previous
        // stage's pixels, and every pass loads instead of clearing.
        if let Some(s) = seed {
            encoder.copy_texture_to_texture(
                s.as_image_copy(),
                target.as_image_copy(),
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let mut first_pass = seed.is_none();
        let mut i = 0usize;
        while i < layers.len() {
            if layers[i].blend.uses_snapshot() {
                if let Some(snap) = &snapshot {
                    if first_pass {
                        // Materialise the background before snapshotting.
                        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("composite-clear"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: clear,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            ..Default::default()
                        });
                        first_pass = false;
                    }
                    encoder.copy_texture_to_texture(
                        target.as_image_copy(),
                        snap.as_image_copy(),
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
            // One pass: this layer plus any following fixed-function layers.
            let mut end = i + 1;
            while end < layers.len() && !layers[end].blend.uses_snapshot() {
                end += 1;
            }
            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("composite"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if first_pass {
                                clear
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                first_pass = false;
                for idx in i..end {
                    rpass.set_pipeline(match layers[idx].blend {
                        Blend::Normal => &self.pipeline,
                        Blend::Add => &self.pipeline_add,
                        Blend::Multiply => &self.pipeline_multiply,
                        _ => &self.pipeline_snapshot,
                    });
                    rpass.set_bind_group(0, &binds[idx], &[]);
                    rpass.draw(0..6, 0..1);
                }
            }
            i = end;
        }
        if first_pass {
            // No layers at all: still clear to the background.
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: clear,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        ctx.queue.submit([encoder.finish()]);
        target
    }

    /// The premultiplied average of one layer drawn at N sub-frame placements —
    /// the per-layer motion-blur smear (docs/06 §4, K-120).
    ///
    /// The SAME `texture` is composited at each [`MbSample`] placement into a
    /// fresh transparent comp-sized target with a pure-additive blend (BOTH
    /// colour and alpha add) at weight `1/N`, so the target holds
    /// `(1/N)·Σ premul(sample_k)` — the arithmetic mean of the premultiplied
    /// sub-frame images. A static layer (every placement equal) averages back
    /// to itself, alpha and all; a moving one smears, its coverage translucent
    /// in proportion to how much of the shutter each pixel was covered.
    ///
    /// `three_d`, `pre` and `camera` place every sub-copy exactly as the
    /// layer's own draw would be placed. Parent motion within the shutter is a
    /// follow-up: `pre` is the frame-time parent placement, applied to every
    /// sample. The caller composites the returned comp-sized texture 1:1
    /// (identity placement, `size = (width, height)`) carrying the layer's real
    /// blend, opacity, matte and mask, so those apply once to the averaged
    /// image, never per sub-copy.
    ///
    /// This is the single helper both the preview and the export path call, so
    /// per-layer motion blur is identical between them (K-031). An empty
    /// `samples` returns a transparent frame (the caller only invokes this with
    /// a non-empty set, so that is a defensive no-op, never a panic).
    #[allow(clippy::too_many_arguments)]
    pub fn motion_blur_average(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        texture: &wgpu::Texture,
        size: (f32, f32),
        samples: &[MbSample],
        three_d: bool,
        pre: Option<[[f32; 4]; 4]>,
        camera: Option<Mat4>,
    ) -> wgpu::Texture {
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mb-average"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORKING_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        // Equal weights sum to 1: N copies at opacity 1/N (UI percent 100/N).
        let weight = if samples.is_empty() {
            0.0
        } else {
            100.0 / samples.len() as f32
        };
        let src_view = texture.create_view(&Default::default());
        let white_view = self.white.create_view(&Default::default());
        let black_view = self.black.create_view(&Default::default());
        let binds: Vec<wgpu::BindGroup> = samples
            .iter()
            .map(|s| {
                let layer = CompositeLayer {
                    texture,
                    size,
                    position: s.position,
                    anchor: s.anchor,
                    scale: s.scale,
                    rotation_deg: s.rotation_deg,
                    opacity: weight,
                    matte: None,
                    blend: Blend::Add,
                    z: s.z,
                    rotation_x_deg: s.rotation_x_deg,
                    rotation_y_deg: s.rotation_y_deg,
                    three_d,
                    layer_mask: None,
                    pre,
                };
                let uniform = LayerUniform {
                    matrix: layer
                        .matrix(width as f32, height as f32, camera.as_ref())
                        .to_cols_array_2d(),
                    // No matte on a sub-copy: the layer's matte applies to the
                    // averaged result at the caller's 1:1 composite.
                    params: [(weight / 100.0).clamp(0.0, 1.0), 0.0, 0.0, 0.0],
                    target: [width as f32, height as f32, -1.0, 0.0],
                };
                let buffer = wgpu::util::DeviceExt::create_buffer_init(
                    &ctx.device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("mb-sample-uniform"),
                        contents: bytemuck::bytes_of(&uniform),
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mb-sample"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&src_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&white_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(&white_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&black_view),
                        },
                    ],
                })
            })
            .collect();

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mb-average"),
            });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mb-average"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Transparent black: the additive accumulation builds
                        // the average up from nothing.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rpass.set_pipeline(&self.pipeline_accumulate);
            for bind in &binds {
                rpass.set_bind_group(0, bind, &[]);
                rpass.draw(0..6, 0..1);
            }
        }
        ctx.queue.submit([encoder.finish()]);
        target
    }

    /// The premultiplied weighted sum of N comp-sized textures — the
    /// accumulation motion-blur combine (docs/08 §3.26, docs/impl/
    /// temporal-rerender.md §3).
    ///
    /// Each `(texture, weight)` is drawn 1:1 (identity placement, full comp
    /// size) into a fresh transparent comp-sized target with the pure-additive
    /// blend (BOTH colour and alpha add) over the premultiplied-passthrough
    /// fragment, so the target holds `Σ weight_k · premul(texture_k)`. The inputs
    /// are already-premultiplied comp composites, so — unlike
    /// [`Self::motion_blur_average`], which premultiplies a straight-alpha source
    /// and re-draws ONE texture at N placements — this scales each premultiplied
    /// texel by its weight and never re-premultiplies. With equal weights `1/N`
    /// it is the arithmetic mean of the N DIFFERENT below-composites (a still
    /// scene, every texture equal, averages back to itself bit-for-bit when `N`
    /// is a power of two, since `1/N` is exact in fp16; a moving one smears). The
    /// caller also uses it to blend the averaged result against the frame-time
    /// composite by the effect's Mix — two weighted layers `1 − mix` and `mix`,
    /// the pure linear interpolation the additive blend gives exactly.
    ///
    /// An empty `layers` returns a transparent frame (a defensive no-op, never a
    /// panic — the caller only invokes this with a non-empty set).
    pub fn accumulate(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        layers: &[(&wgpu::Texture, f32)],
    ) -> wgpu::Texture {
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("accumulate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORKING_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let white_view = self.white.create_view(&Default::default());
        let black_view = self.black.create_view(&Default::default());
        let binds: Vec<wgpu::BindGroup> = layers
            .iter()
            .map(|(texture, weight)| {
                // Identity placement at comp size: a full-frame 1:1 quad. The
                // weight rides in params.x (fs_accumulate scales the premultiplied
                // texel by it); the matrix carries no opacity, so it stays plain.
                let layer = CompositeLayer {
                    texture,
                    size: (width as f32, height as f32),
                    position: (0.0, 0.0),
                    anchor: (0.0, 0.0),
                    scale: (100.0, 100.0),
                    rotation_deg: 0.0,
                    opacity: 100.0,
                    matte: None,
                    blend: Blend::Add,
                    z: 0.0,
                    rotation_x_deg: 0.0,
                    rotation_y_deg: 0.0,
                    three_d: false,
                    layer_mask: None,
                    pre: None,
                };
                let uniform = LayerUniform {
                    matrix: layer
                        .matrix(width as f32, height as f32, None)
                        .to_cols_array_2d(),
                    params: [weight.clamp(0.0, 1.0), 0.0, 0.0, 0.0],
                    target: [width as f32, height as f32, -1.0, 0.0],
                };
                let buffer = wgpu::util::DeviceExt::create_buffer_init(
                    &ctx.device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("accumulate-uniform"),
                        contents: bytemuck::bytes_of(&uniform),
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("accumulate"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &texture.create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&white_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(&white_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&black_view),
                        },
                    ],
                })
            })
            .collect();

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("accumulate"),
            });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("accumulate"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Transparent black: the additive sum builds up from nothing.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rpass.set_pipeline(&self.pipeline_premul_accumulate);
            for bind in &binds {
                rpass.set_bind_group(0, bind, &[]);
                rpass.draw(0..6, 0..1);
            }
        }
        ctx.queue.submit([encoder.finish()]);
        target
    }
}

/// Convenience: full comp render → display-encoded sRGB texture.
pub fn render_for_display(
    ctx: &GpuContext,
    colour: &ColourEngine,
    compositor: &Compositor,
    width: u32,
    height: u32,
    background: [f64; 4],
    layers: &[CompositeLayer<'_>],
) -> wgpu::Texture {
    let linear = compositor.composite(ctx, width, height, background, layers);
    colour.display(ctx, &linear)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Collapse maths (docs/06 §1.4): concatenating placements composes the
    /// transforms — a point placed by the inner layer then carried by the
    /// parent lands where applying both transforms in sequence puts it, and
    /// a parent scale multiplies the inner offset.
    #[test]
    fn place_matrix_concatenation_matches_composed_transforms() {
        let ident_scale = (100.0, 100.0);
        let parent = place_matrix((100.0, 50.0), (0.0, 0.0), ident_scale, 0.0, 0.0, 0.0, 0.0);
        let inner = place_matrix((10.0, 20.0), (0.0, 0.0), ident_scale, 0.0, 0.0, 0.0, 0.0);
        let p =
            Mat4::from_cols_array_2d(&concat_place(parent, inner)) * glam::vec4(0.0, 0.0, 0.0, 1.0);
        assert!((p.x - 110.0).abs() < 1e-4 && (p.y - 70.0).abs() < 1e-4);
        // A 200% parent doubles the inner offset: (100,50) + 2·(10,20).
        let parent2 = place_matrix(
            (100.0, 50.0),
            (0.0, 0.0),
            (200.0, 200.0),
            0.0,
            0.0,
            0.0,
            0.0,
        );
        let q = Mat4::from_cols_array_2d(&concat_place(parent2, inner))
            * glam::vec4(0.0, 0.0, 0.0, 1.0);
        assert!((q.x - 120.0).abs() < 1e-4 && (q.y - 90.0).abs() < 1e-4);
    }

    fn solid_linear(
        ctx: &GpuContext,
        colour: &ColourEngine,
        rgba8: [u8; 4],
        w: u32,
        h: u32,
    ) -> wgpu::Texture {
        let px: Vec<u8> = std::iter::repeat_n(rgba8, (w * h) as usize)
            .flatten()
            .collect();
        let src = colour.upload_srgb8(ctx, &px, w, h);
        colour.linearise(ctx, &src)
    }

    fn srgb_encode(linear: f64) -> f64 {
        if linear <= 0.003_130_8 {
            12.92 * linear
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        }
    }
    fn srgb_decode(encoded: f64) -> f64 {
        if encoded <= 0.040_45 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Half-opacity sRGB-red over sRGB-green background must blend in LINEAR
    /// light — the physically-correct result, distinct from naive byte
    /// averaging by ~19 code values on the red channel.
    #[test]
    fn blending_happens_in_linear_light() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);

        let red = solid_linear(&ctx, &colour, [255, 0, 0, 255], 4, 4);
        let layer = CompositeLayer {
            texture: &red,
            size: (4.0, 4.0),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            opacity: 50.0,
            matte: None,
            blend: Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
        };
        // Background: linear green = sRGB 0,255,0 decoded.
        let g_lin = srgb_decode(1.0);
        let shown = render_for_display(
            &ctx,
            &colour,
            &compositor,
            4,
            4,
            [0.0, g_lin, 0.0, 1.0],
            &[layer],
        );
        let back = colour.readback8(&ctx, &shown).unwrap();

        // Expected: 0.5·linear(red) over linear(green), then sRGB-encoded.
        let expect_r = (srgb_encode(0.5 * srgb_decode(1.0)) * 255.0).round() as i16;
        let expect_g = (srgb_encode(0.5 * srgb_decode(1.0)) * 255.0).round() as i16;
        let (r, g, b) = (i16::from(back[0]), i16::from(back[1]), i16::from(back[2]));
        assert!((r - expect_r).abs() <= 2, "r {r} vs {expect_r}");
        assert!((g - expect_g).abs() <= 2, "g {g} vs {expect_g}");
        assert!(b <= 2, "b {b}");
        // And the linear result is NOT the gamma-naive 128:
        assert!((r - 128).abs() > 10, "blend looks gamma-naive: r {r}");
    }

    /// One matte layer gates a consumer without duplication or precomping
    /// (the K-020-era matte model): alpha matte passes the covered half,
    /// inverted flips it — verified per pixel.
    #[test]
    fn matte_gates_a_layer_per_pixel() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);

        // The matte: a quad covering the LEFT half of the 8×8 comp,
        // rendered alone into comp space (transparent background).
        let white = solid_linear(&ctx, &colour, [255, 255, 255, 255], 4, 8);
        let matte_tex = compositor.composite(
            &ctx,
            8,
            8,
            [0.0, 0.0, 0.0, 0.0],
            &[CompositeLayer {
                texture: &white,
                size: (4.0, 8.0),
                position: (0.0, 0.0),
                anchor: (0.0, 0.0),
                scale: (100.0, 100.0),
                rotation_deg: 0.0,
                opacity: 100.0,
                matte: None,
                blend: Blend::Normal,
                z: 0.0,
                rotation_x_deg: 0.0,
                rotation_y_deg: 0.0,
                three_d: false,
                layer_mask: None,
                pre: None,
            }],
        );

        // The consumer: full-comp red, gated by the matte's alpha.
        let red = solid_linear(&ctx, &colour, [255, 0, 0, 255], 8, 8);
        let consumer = |inverted: bool| CompositeLayer {
            texture: &red,
            size: (8.0, 8.0),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            opacity: 100.0,
            matte: Some(MatteInput {
                texture: &matte_tex,
                luma: false,
                inverted,
            }),
            blend: Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
        };

        let shown = render_for_display(
            &ctx,
            &colour,
            &compositor,
            8,
            8,
            [0.0, 0.0, 0.0, 1.0],
            &[consumer(false)],
        );
        let back = colour.readback8(&ctx, &shown).unwrap();
        let red_at = |x: usize, y: usize| back[(y * 8 + x) * 4];
        assert!(red_at(1, 4) > 250, "left (matted-in) {}", red_at(1, 4));
        assert!(red_at(6, 4) < 5, "right (matted-out) {}", red_at(6, 4));

        let shown_inv = render_for_display(
            &ctx,
            &colour,
            &compositor,
            8,
            8,
            [0.0, 0.0, 0.0, 1.0],
            &[consumer(true)],
        );
        let back = colour.readback8(&ctx, &shown_inv).unwrap();
        let red_at = |x: usize, y: usize| back[(y * 8 + x) * 4];
        assert!(red_at(1, 4) < 5, "inverted: left now out {}", red_at(1, 4));
        assert!(
            red_at(6, 4) > 250,
            "inverted: right now in {}",
            red_at(6, 4)
        );
    }

    /// The AE camera model: at default placement the z=0 plane maps 1:1;
    /// pushing a 3D layer back in z shrinks it by zoom/(z+zoom).
    #[test]
    fn camera_perspective_scales_by_depth() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        let white = solid_linear(&ctx, &colour, [255, 255, 255, 255], 8, 8);
        let cam = camera_matrix(32.0, 32.0, 100.0, (16.0, 16.0, 0.0), (0.0, 0.0, 0.0));
        let layer = |z: f32| CompositeLayer {
            texture: &white,
            size: (8.0, 8.0),
            position: (16.0, 16.0),
            anchor: (4.0, 4.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            opacity: 100.0,
            matte: None,
            blend: Blend::Normal,
            z,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: true,
            layer_mask: None,
            pre: None,
        };
        let count_white = |z: f32| {
            let linear = compositor.composite_with_camera(
                &ctx,
                32,
                32,
                [0.0, 0.0, 0.0, 1.0],
                &[layer(z)],
                Some(cam),
            );
            let shown = colour.display(&ctx, &linear);
            let back = colour.readback8(&ctx, &shown).unwrap();
            back.chunks_exact(4).filter(|p| p[0] > 200).count() as f64
        };
        let at_zero = count_white(0.0);
        let at_back = count_white(100.0); // zoom/(z+zoom) = 0.5 → area ×0.25
        assert!((at_zero - 64.0).abs() <= 8.0, "z=0 area {at_zero} (≈64)");
        let ratio = at_back / at_zero;
        assert!(
            (ratio - 0.25).abs() < 0.08,
            "depth scaling ratio {ratio} (≈0.25)"
        );
    }

    /// Screen is computed perceptually: grey over grey must land at the
    /// encoded-space screen result (~192), not the linear one.
    #[test]
    fn screen_blend_matches_the_perceptual_formula() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        let grey = solid_linear(&ctx, &colour, [128, 128, 128, 255], 4, 4);
        let g_lin = srgb_decode(128.0 / 255.0);
        let shown = render_for_display(
            &ctx,
            &colour,
            &compositor,
            4,
            4,
            [g_lin, g_lin, g_lin, 1.0],
            &[CompositeLayer {
                texture: &grey,
                size: (4.0, 4.0),
                position: (0.0, 0.0),
                anchor: (0.0, 0.0),
                scale: (100.0, 100.0),
                rotation_deg: 0.0,
                opacity: 100.0,
                matte: None,
                blend: Blend::Screen,
                z: 0.0,
                rotation_x_deg: 0.0,
                rotation_y_deg: 0.0,
                three_d: false,
                layer_mask: None,
                pre: None,
            }],
        );
        let out = colour.readback8(&ctx, &shown).unwrap()[0];
        // encoded screen: 1-(1-0.502)^2 = 0.752 → byte ≈ 192
        let s = 128.0 / 255.0;
        let expect = ((1.0 - (1.0 - s) * (1.0 - s)) * 255.0_f64).round() as i16;
        assert!(
            (i16::from(out) - expect).abs() <= 2,
            "screen {out} vs {expect}"
        );
    }

    /// The layer-space mask binding gates alpha: a white layer with a
    /// left-half mask texture shows exactly its left half (GPU mask pass for
    /// Precomp layers, whose pixels never exist CPU-side).
    #[test]
    fn layer_mask_texture_gates_alpha() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        let white = solid_linear(&ctx, &colour, [255, 255, 255, 255], 8, 8);
        // White RGBA whose alpha is the coverage: left half on, right off.
        let mask_rgba: Vec<u8> = (0..8u32 * 8)
            .flat_map(|i| [255, 255, 255, if i % 8 < 4 { 255 } else { 0 }])
            .collect();
        let mask = colour.upload_srgb8(&ctx, &mask_rgba, 8, 8);
        let linear = compositor.composite(
            &ctx,
            8,
            8,
            [0.0, 0.0, 0.0, 1.0],
            &[CompositeLayer {
                texture: &white,
                size: (8.0, 8.0),
                position: (0.0, 0.0),
                anchor: (0.0, 0.0),
                scale: (100.0, 100.0),
                rotation_deg: 0.0,
                opacity: 100.0,
                matte: None,
                blend: Blend::Normal,
                z: 0.0,
                rotation_x_deg: 0.0,
                rotation_y_deg: 0.0,
                three_d: false,
                layer_mask: Some(&mask),
                pre: None,
            }],
        );
        let shown = colour.display(&ctx, &linear);
        let back = colour.readback8(&ctx, &shown).unwrap();
        let px = |x: usize, y: usize| back[(y * 8 + x) * 4];
        assert!(px(1, 4) > 240, "inside mask stays white: {}", px(1, 4));
        assert!(px(6, 4) < 15, "outside mask goes black: {}", px(6, 4));
    }

    /// Every snapshot blend matches its CPU oracle: Overlay/Soft/Hard light
    /// perceptually (encoded W3C formulas), Lighten/Darken per-channel in
    /// linear — the domain table of docs/06 §blend, pinned per mode.
    #[test]
    fn snapshot_blends_match_their_formulas() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        // src byte 200 over dst byte 64: exercises both formula branches.
        let (s8, d8) = (200u8, 64u8);
        let src_tex = solid_linear(&ctx, &colour, [s8, s8, s8, 255], 4, 4);
        let d_lin = srgb_decode(f64::from(d8) / 255.0);
        let read = |blend: Blend| {
            let shown = render_for_display(
                &ctx,
                &colour,
                &compositor,
                4,
                4,
                [d_lin, d_lin, d_lin, 1.0],
                &[CompositeLayer {
                    texture: &src_tex,
                    size: (4.0, 4.0),
                    position: (0.0, 0.0),
                    anchor: (0.0, 0.0),
                    scale: (100.0, 100.0),
                    rotation_deg: 0.0,
                    opacity: 100.0,
                    matte: None,
                    blend,
                    z: 0.0,
                    rotation_x_deg: 0.0,
                    rotation_y_deg: 0.0,
                    three_d: false,
                    layer_mask: None,
                    pre: None,
                }],
            );
            colour.readback8(&ctx, &shown).unwrap()[0]
        };
        // CPU oracles in encoded space (display bytes ARE encoded space, so
        // perceptual results compare directly).
        let s = f64::from(s8) / 255.0;
        let d = f64::from(d8) / 255.0;
        let overlay = if d <= 0.5 {
            2.0 * s * d
        } else {
            1.0 - 2.0 * (1.0 - s) * (1.0 - d)
        };
        let soft_d = if d <= 0.25 {
            ((16.0 * d - 12.0) * d + 4.0) * d
        } else {
            d.sqrt()
        };
        let soft = if s <= 0.5 {
            d - (1.0 - 2.0 * s) * d * (1.0 - d)
        } else {
            d + (2.0 * s - 1.0) * (soft_d - d)
        };
        let hard = if s <= 0.5 {
            2.0 * s * d
        } else {
            1.0 - 2.0 * (1.0 - s) * (1.0 - d)
        };
        // Lighten/Darken run in linear; on solid colours per-channel max/min
        // commutes with the transfer function, so the byte answer is plain.
        let cases: [(Blend, f64, &str); 5] = [
            (Blend::Overlay, overlay, "overlay"),
            (Blend::SoftLight, soft, "soft light"),
            (Blend::HardLight, hard, "hard light"),
            (Blend::Lighten, s.max(d), "lighten"),
            (Blend::Darken, s.min(d), "darken"),
        ];
        for (blend, expect, name) in cases {
            let out = read(blend);
            let expect = (expect * 255.0).round() as i16;
            assert!(
                (i16::from(out) - expect).abs() <= 3,
                "{name}: {out} vs {expect}"
            );
        }
    }

    /// Add blend genuinely adds light: half-grey over half-grey doubles the
    /// linear value where Normal-over would darken toward the top layer.
    #[test]
    fn add_blend_adds_light_linearly() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        let grey = solid_linear(&ctx, &colour, [128, 128, 128, 255], 4, 4);
        let layer = |blend: Blend| CompositeLayer {
            texture: &grey,
            size: (4.0, 4.0),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            opacity: 100.0,
            matte: None,
            blend,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
        };
        let g_lin = srgb_decode(128.0 / 255.0);
        let bg = [g_lin, g_lin, g_lin, 1.0];
        let read = |blend: Blend| {
            let shown = render_for_display(&ctx, &colour, &compositor, 4, 4, bg, &[layer(blend)]);
            colour.readback8(&ctx, &shown).unwrap()[0]
        };
        let normal = read(Blend::Normal);
        let added = read(Blend::Add);
        // Normal over: result == the top layer (opaque) == 128.
        assert!((i16::from(normal) - 128).abs() <= 1, "normal {normal}");
        // Add: linear doubles → sRGB-encode(2·linear(0.5)) ≈ 188.
        let expect = (srgb_encode(2.0 * g_lin).min(1.0) * 255.0).round() as i16;
        assert!(
            (i16::from(added) - expect).abs() <= 2,
            "add {added} vs {expect}"
        );
    }

    /// Subtract removes light in LINEAR (GEN-1, K-151): a darker grey layer
    /// over a lighter grey background lands at sRGB-encode(max(dst − src, 0)),
    /// the photographic subtract, and never goes negative. Its snapshot path
    /// mixes by coverage, so full-opacity opaque solids read the raw formula.
    #[test]
    fn subtract_blend_removes_light_linearly() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        // Layer sRGB 64 over background sRGB 200: dst − src is a real, positive
        // remainder in linear light.
        let (s8, d8) = (64u8, 200u8);
        let src = solid_linear(&ctx, &colour, [s8, s8, s8, 255], 4, 4);
        let d_lin = srgb_decode(f64::from(d8) / 255.0);
        let s_lin = srgb_decode(f64::from(s8) / 255.0);
        let shown = render_for_display(
            &ctx,
            &colour,
            &compositor,
            4,
            4,
            [d_lin, d_lin, d_lin, 1.0],
            &[CompositeLayer {
                texture: &src,
                size: (4.0, 4.0),
                position: (0.0, 0.0),
                anchor: (0.0, 0.0),
                scale: (100.0, 100.0),
                rotation_deg: 0.0,
                opacity: 100.0,
                matte: None,
                blend: Blend::Subtract,
                z: 0.0,
                rotation_x_deg: 0.0,
                rotation_y_deg: 0.0,
                three_d: false,
                layer_mask: None,
                pre: None,
            }],
        );
        let out = colour.readback8(&ctx, &shown).unwrap()[0];
        let expect = (srgb_encode((d_lin - s_lin).max(0.0)) * 255.0).round() as i16;
        assert!(
            (i16::from(out) - expect).abs() <= 2,
            "subtract {out} vs {expect}"
        );
        // And it is genuinely a LINEAR subtract, not a byte one: naive
        // 200 − 64 = 136 in sRGB space would read very differently.
        assert!(
            (i16::from(out) - 136).abs() > 10,
            "subtract looks byte-naive: {out}"
        );
    }

    /// Seeding continues the accumulation exactly: compositing A then B in
    /// one call equals compositing A alone, then B seeded on the result —
    /// the invariant adjustment-layer staging rests on (docs/06 §1.5). B
    /// uses a snapshot blend so the seeded path's snapshot branch is
    /// exercised too.
    #[test]
    fn a_seeded_composite_continues_the_accumulation() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        let red = solid_linear(&ctx, &colour, [255, 0, 0, 255], 8, 8);
        let grey = solid_linear(&ctx, &colour, [128, 128, 128, 255], 8, 8);
        fn layer(tex: &wgpu::Texture, x: f32, blend: Blend) -> CompositeLayer<'_> {
            CompositeLayer {
                texture: tex,
                size: (8.0, 8.0),
                position: (x, 4.0),
                anchor: (0.0, 0.0),
                scale: (100.0, 100.0),
                rotation_deg: 0.0,
                opacity: 60.0,
                matte: None,
                blend,
                z: 0.0,
                rotation_x_deg: 0.0,
                rotation_y_deg: 0.0,
                three_d: false,
                layer_mask: None,
                pre: None,
            }
        }
        let bg = [0.1, 0.2, 0.3, 1.0];
        let both = compositor.composite(
            &ctx,
            16,
            16,
            bg,
            &[
                layer(&red, 2.0, Blend::Normal),
                layer(&grey, 6.0, Blend::Screen),
            ],
        );
        let first = compositor.composite(&ctx, 16, 16, bg, &[layer(&red, 2.0, Blend::Normal)]);
        let seeded = compositor.composite_seeded(
            &ctx,
            16,
            16,
            [0.0; 4], // ignored: the seed replaces the clear
            &[layer(&grey, 6.0, Blend::Screen)],
            None,
            Some(&first),
        );
        let a = crate::fx::readback_linear_f32(&ctx, &both, 16, 16).unwrap();
        let b = crate::fx::readback_linear_f32(&ctx, &seeded, 16, 16).unwrap();
        assert_eq!(a, b, "seeded continuation must be bit-identical");
    }

    /// Per-layer motion blur (docs/06 §4, K-120): averaging a moving layer's
    /// sub-frame placements widens its coverage, while a static layer (every
    /// placement equal) averages back to a full-alpha copy of itself — the
    /// premultiplied-average property the pure-additive accumulator gives that
    /// the Add blend mode (over-alpha) would not.
    #[test]
    fn motion_blur_average_widens_coverage_and_preserves_static_alpha() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        // A small opaque white quad, its anchor at the top-left so `position`
        // is the quad's left edge in comp pixels.
        let white = solid_linear(&ctx, &colour, [255, 255, 255, 255], 4, 4);
        let (w, h) = (40u32, 16u32);
        let sample_at = |x: f32| MbSample {
            position: (x, 6.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
        };
        let readback = |samples: &[MbSample]| {
            let tex = compositor.motion_blur_average(
                &ctx,
                w,
                h,
                &white,
                (4.0, 4.0),
                samples,
                false,
                None,
                None,
            );
            crate::fx::readback_linear_f32(&ctx, &tex, w, h).unwrap()
        };
        let alpha = |px: &[f32], x: usize, y: usize| px[(y * w as usize + x) * 4 + 3];
        let covered_cols = |px: &[f32]| {
            (0..w as usize)
                .filter(|&x| (0..h as usize).any(|y| alpha(px, x, y) > 0.01))
                .count()
        };

        // Static: four identical placements — coverage is just the 4px quad,
        // and its interior alpha averages back to 1.0 (4 copies × 1/4), NOT
        // the ~0.68 an over-composited alpha would give.
        let still = [sample_at(18.0); 4];
        let still_px = readback(&still);
        let still_cols = covered_cols(&still_px);
        assert!(
            (3..=5).contains(&still_cols),
            "static coverage {still_cols} ≈ quad width"
        );
        assert!(
            alpha(&still_px, 20, 8) > 0.9,
            "static interior alpha {} must stay opaque (premultiplied average)",
            alpha(&still_px, 20, 8)
        );

        // Moving: the same quad slid rightward across the shutter — coverage
        // spreads well past the static 4px width.
        let moving = [
            sample_at(6.0),
            sample_at(12.0),
            sample_at(18.0),
            sample_at(24.0),
        ];
        let moving_px = readback(&moving);
        let moving_cols = covered_cols(&moving_px);
        assert!(
            moving_cols > still_cols + 5,
            "moving coverage {moving_cols} must widen past static {still_cols}"
        );
    }

    /// Accumulation motion blur (docs/08 §3.26): the additive average of N
    /// DIFFERENT premultiplied below-composites. A still scene (N identical
    /// frames) averages back to itself bit-for-bit — the identity the whole
    /// preview==export promise rests on — while a moving scene (the same quad at
    /// two positions) spreads coverage across both.
    #[test]
    fn accumulate_averages_premultiplied_frames() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        let red = solid_linear(&ctx, &colour, [255, 0, 0, 255], 4, 8);
        // A genuinely premultiplied comp: a red quad over the LEFT half of an
        // 8×8 frame on a transparent background (the right half is alpha 0).
        let frame = |x: f32| {
            compositor.composite(
                &ctx,
                8,
                8,
                [0.0, 0.0, 0.0, 0.0],
                &[CompositeLayer {
                    texture: &red,
                    size: (4.0, 8.0),
                    position: (x, 0.0),
                    anchor: (0.0, 0.0),
                    scale: (100.0, 100.0),
                    rotation_deg: 0.0,
                    opacity: 100.0,
                    matte: None,
                    blend: Blend::Normal,
                    z: 0.0,
                    rotation_x_deg: 0.0,
                    rotation_y_deg: 0.0,
                    three_d: false,
                    layer_mask: None,
                    pre: None,
                }],
            )
        };
        let left = frame(0.0);
        // Four identical copies at 1/4 must return the frame bit-for-bit (1/4 is
        // exact in fp16, four copies sum back exactly) — the still-scene identity.
        let avg = compositor.accumulate(
            &ctx,
            8,
            8,
            &[(&left, 0.25), (&left, 0.25), (&left, 0.25), (&left, 0.25)],
        );
        let a = crate::fx::readback_linear_f32(&ctx, &left, 8, 8).unwrap();
        let b = crate::fx::readback_linear_f32(&ctx, &avg, 8, 8).unwrap();
        assert_eq!(
            a, b,
            "averaging identical premultiplied frames is the identity"
        );

        // Moving: the same quad on the RIGHT half, averaged 50/50 with the left —
        // both halves are now half-covered (the smear), where neither single
        // frame covers both.
        let right = frame(4.0);
        let mixed = compositor.accumulate(&ctx, 8, 8, &[(&left, 0.5), (&right, 0.5)]);
        let m = crate::fx::readback_linear_f32(&ctx, &mixed, 8, 8).unwrap();
        let alpha = |px: &[f32], x: usize, y: usize| px[(y * 8 + x) * 4 + 3];
        assert!(
            (alpha(&m, 1, 4) - 0.5).abs() < 0.05,
            "left half-covered ~0.5; got {}",
            alpha(&m, 1, 4)
        );
        assert!(
            (alpha(&m, 6, 4) - 0.5).abs() < 0.05,
            "right half-covered ~0.5; got {}",
            alpha(&m, 6, 4)
        );
        assert!(
            alpha(&a, 6, 4) < 0.05,
            "the left frame alone leaves the right transparent; got {}",
            alpha(&a, 6, 4)
        );
    }

    /// A quarter-size quad placed at the centre covers exactly the centre
    /// quarter: transforms map comp pixels correctly (and the rest of the
    /// frame keeps the background).
    #[test]
    fn transforms_place_layers_in_comp_pixels() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let colour = ColourEngine::new(&ctx);
        let compositor = Compositor::new(&ctx);
        let white = solid_linear(&ctx, &colour, [255, 255, 255, 255], 8, 8);
        let layer = CompositeLayer {
            texture: &white,
            size: (8.0, 8.0),
            position: (8.0, 8.0), // centre of a 16×16 comp
            anchor: (4.0, 4.0),   // layer centre
            scale: (50.0, 50.0),  // 8px quad → 4px
            rotation_deg: 0.0,
            opacity: 100.0,
            matte: None,
            blend: Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
        };
        let shown = render_for_display(
            &ctx,
            &colour,
            &compositor,
            16,
            16,
            [0.0, 0.0, 0.0, 1.0],
            &[layer],
        );
        let back = colour.readback8(&ctx, &shown).unwrap();
        let px = |x: usize, y: usize| back[(y * 16 + x) * 4];
        // Centre 4×4 block is white; corners stay background.
        assert!(px(8, 8) > 250, "centre {}", px(8, 8));
        assert!(px(6, 6) > 250 && px(9, 9) > 250);
        assert!(px(0, 0) < 5 && px(15, 15) < 5);
        assert!(px(4, 8) < 5 && px(11, 8) < 5, "outside the scaled quad");
    }
}
