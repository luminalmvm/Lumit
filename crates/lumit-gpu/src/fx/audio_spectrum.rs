//! Audio spectrum's pass (`fx_audio_spectrum.wgsl`): the pieces the host laid out,
//! drawn over the picture.

use std::sync::OnceLock;

use crate::GpuContext;

use super::lens_blur::{bind_layout, Slot};
use super::{work_texture, FxEngine};

/// One frame's drawing, already worked out by `lumit_core`, which this crate
/// doesn't see.
#[derive(Debug, Clone, Copy)]
pub struct AudioSpectrumOp<'a> {
    /// Three per piece: `(ax, ay, bx, by)`, then the colour at each end.
    pub pieces: &'a [[f32; 4]],
    /// The tile columns and rows.
    pub tiles: [u32; 2],
    /// `cols * rows + 1` list starts.
    pub starts: &'a [u32],
    pub items: &'a [u32],
    pub half: f32,
    pub edge: f32,
    pub composite: bool,
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    w: u32,
    h: u32,
    cols: u32,
    rows: u32,
    half: f32,
    edge: f32,
    composite: u32,
    mix_amt: f32,
    items_at: u32,
    _pad: [u32; 3],
}

/// The pipeline, built the first time one is drawn.
#[derive(Default)]
pub(super) struct LazyAudioSpectrum(OnceLock<(wgpu::BindGroupLayout, wgpu::ComputePipeline)>);

impl LazyAudioSpectrum {
    fn get(&self, ctx: &GpuContext) -> &(wgpu::BindGroupLayout, wgpu::ComputePipeline) {
        self.0.get_or_init(|| {
            let device = &ctx.device;
            let working = ctx.working();
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fx-audio-spectrum"),
                source: wgpu::ShaderSource::Wgsl(
                    super::at_depth(working, include_str!("../fx_audio_spectrum.wgsl")).into(),
                ),
            });
            let bgl = bind_layout(
                device,
                "fx-audio-spectrum",
                &[
                    (0, Slot::Uniform),
                    (1, Slot::Spans),
                    (2, Slot::Spans),
                    (3, Slot::Tex),
                    (4, Slot::Store(working)),
                ],
                wgpu::ShaderStages::COMPUTE,
            );
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-audio-spectrum"),
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });
            let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-audio-spectrum"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("audio_spectrum"),
                compilation_options: Default::default(),
                cache: None,
            });
            (bgl, pipe)
        })
    }
}

impl FxEngine {
    /// Draw one frame's spectrum over a linear working texture, returning a new
    /// one of the same size. Mix 0 hands the picture back.
    pub fn audio_spectrum(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &AudioSpectrumOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        use wgpu::BufferUsages as B;
        if op.mix <= 0.0 || w == 0 || h == 0 {
            return src.clone();
        }
        let (Ok(items_at), [cols, rows]) = (u32::try_from(op.starts.len()), op.tiles) else {
            return src.clone();
        };
        let (bgl, pipe) = self.audio_spectrum.get(ctx);
        let buffer = |label, contents: &[u8], usage| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        let params = buffer(
            "fx-audio-spectrum-params",
            bytemuck::bytes_of(&Params {
                w,
                h,
                cols,
                rows,
                half: op.half,
                edge: op.edge,
                composite: u32::from(op.composite),
                mix_amt: op.mix,
                items_at,
                _pad: [0; 3],
            }),
            B::UNIFORM,
        );
        // A binding can't be empty, and no tile lists this one.
        let none = [[0.0f32; 4]; 3];
        let pieces = buffer(
            "fx-audio-spectrum-pieces",
            bytemuck::cast_slice(if op.pieces.is_empty() {
                &none
            } else {
                op.pieces
            }),
            B::STORAGE,
        );
        // One buffer: the starts, then the lists. Never empty, since a binding
        // can't be.
        let mut words = Vec::with_capacity(op.starts.len() + op.items.len());
        words.extend_from_slice(op.starts);
        words.extend_from_slice(op.items);
        let bins = buffer(
            "fx-audio-spectrum-bins",
            bytemuck::cast_slice(&words),
            B::STORAGE,
        );
        let out = work_texture(ctx, w, h, "fx-audio-spectrum-out");
        let (v_src, v_out) = (
            src.create_view(&Default::default()),
            out.create_view(&Default::default()),
        );
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-audio-spectrum"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pieces.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: bins.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&v_src),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&v_out),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-audio-spectrum");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-audio-spectrum"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
