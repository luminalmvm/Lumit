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
}

impl CompositeLayer<'_> {
    /// comp pixel space → NDC, with the layer transform applied.
    /// Full 4×4 (K-023). Order: quad(0..1) → layer px → −anchor → scale →
    /// rotate → +position → NDC.
    fn matrix(&self, comp_w: f32, comp_h: f32, camera: Option<&Mat4>) -> Mat4 {
        let ndc_from_comp = Mat4::from_translation(glam::vec3(-1.0, 1.0, 0.0))
            * Mat4::from_scale(glam::vec3(2.0 / comp_w, -2.0 / comp_h, 1.0));
        let place = Mat4::from_translation(glam::vec3(self.position.0, self.position.1, self.z))
            * Mat4::from_rotation_y(self.rotation_y_deg.to_radians())
            * Mat4::from_rotation_x(self.rotation_x_deg.to_radians())
            * Mat4::from_rotation_z(self.rotation_deg.to_radians())
            * Mat4::from_scale(glam::vec3(self.scale.0 / 100.0, self.scale.1 / 100.0, 1.0))
            * Mat4::from_translation(glam::vec3(-self.anchor.0, -self.anchor.1, 0.0));
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
        let pipeline = make_pipeline(blend, "composite-normal");
        let pipeline_add = make_pipeline(blend_add, "composite-add");
        let pipeline_multiply = make_pipeline(blend_multiply, "composite-multiply");
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
                | wgpu::TextureUsages::COPY_SRC,
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
        let mut first_pass = true;
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
