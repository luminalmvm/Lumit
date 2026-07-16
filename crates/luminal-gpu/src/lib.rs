//! The GPU colour foundation (docs/impl/gpu-foundation.md §1–2, slice 5).
//!
//! In plain terms: the engine does all its maths on light-linear values (where
//! "add two lights" behaves like real light), but files and screens use sRGB
//! encoding. This crate owns the ONLY two crossings: decode-side linearise
//! (sRGB bytes → linear fp16 working texture) and display-side encode
//! (linear → sRGB for the screen). Keeping both crossings in one module with
//! a round-trip test is what prevents the classic "double gamma" washed-out /
//! too-dark bugs — and it is why preview can be bit-identical to export
//! (decision K-031).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("no suitable GPU adapter")]
    NoAdapter,
    #[error("device request failed: {0}")]
    Device(String),
    #[error("readback failed: {0}")]
    Readback(String),
}

/// Device + queue. In the app these come from eframe's render state; tests
/// and future headless export create their own.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Wrap an existing device/queue (eframe's render state — wgpu handles
    /// are internally reference-counted, so cloning shares the one device).
    pub fn from_parts(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self { device, queue }
    }

    /// Headless context (tests, future CLI export).
    pub fn headless() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok_or(GpuError::NoAdapter)?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .map_err(|e| GpuError::Device(e.to_string()))?;
        Ok(Self { device, queue })
    }
}

/// The two colour crossings (linearise, display) as render pipelines.
pub struct ColourEngine {
    linearise: wgpu::RenderPipeline,
    display: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// The engine's working format (docs/06-RENDER-PIPELINE.md §3).
pub const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Source/display byte format: sRGB-encoded, hardware-converted at the edges.
pub const SRGB_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

impl ColourEngine {
    pub fn new(ctx: &GpuContext) -> Self {
        let shader = ctx
            .device
            .create_shader_module(wgpu::include_wgsl!("colour.wgsl"));
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("colour-src"),
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
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("colour"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let make = |target: wgpu::TextureFormat, label: &str| {
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_fullscreen"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_copy"),
                        targets: &[Some(target.into())],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview: None,
                    cache: None,
                })
        };
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("colour-nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self {
            linearise: make(WORKING_FORMAT, "linearise"),
            display: make(SRGB_FORMAT, "display"),
            layout,
            sampler,
        }
    }

    /// Upload sRGB-encoded RGBA8 bytes (a decoded frame) ready for linearising.
    pub fn upload_srgb8(
        &self,
        ctx: &GpuContext,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame-srgb8"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SRGB_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    fn pass(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::RenderPipeline,
        src: &wgpu::Texture,
        format: wgpu::TextureFormat,
        extra_usage: wgpu::TextureUsages,
        label: &str,
    ) -> wgpu::Texture {
        let size = src.size();
        let dst = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | extra_usage,
            view_formats: &[],
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &src.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        {
            let view = dst.create_view(&Default::default());
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, &bind, &[]);
            rpass.draw(0..3, 0..1);
        }
        ctx.queue.submit([encoder.finish()]);
        dst
    }

    /// sRGB source texture → linear fp16 working texture.
    pub fn linearise(&self, ctx: &GpuContext, src: &wgpu::Texture) -> wgpu::Texture {
        self.pass(
            ctx,
            &self.linearise,
            src,
            WORKING_FORMAT,
            wgpu::TextureUsages::empty(),
            "linearise",
        )
    }

    /// Linear working texture → sRGB display texture (register this with the
    /// UI, or read it back for export/tests).
    pub fn display(&self, ctx: &GpuContext, src: &wgpu::Texture) -> wgpu::Texture {
        self.pass(
            ctx,
            &self.display,
            src,
            SRGB_FORMAT,
            wgpu::TextureUsages::COPY_SRC,
            "display",
        )
    }

    /// Read a display texture back as tight RGBA8 bytes (tests, export).
    pub fn readback8(&self, ctx: &GpuContext, tex: &wgpu::Texture) -> Result<Vec<u8>, GpuError> {
        let size = tex.size();
        let row = size.width * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        ctx.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;

        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((row * size.height) as usize);
        for r in 0..size.height {
            let start = (r * padded) as usize;
            out.extend_from_slice(&data[start..start + row as usize]);
        }
        drop(data);
        buffer.unmap();
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The gpu-foundation §7 golden: every 8-bit value survives
    /// sRGB → linear fp16 → sRGB within 1 LSB. This is the test that makes
    /// double-gamma bugs impossible to reintroduce silently (K-031).
    #[test]
    fn colour_round_trip_is_within_one_lsb() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let engine = ColourEngine::new(&ctx);

        // 16×16: every possible byte value in R, G and B (offset per channel).
        let (w, h) = (16u32, 16u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for i in 0..256u32 {
            rgba.push(i as u8); // R = 0..255
            rgba.push((255 - i) as u8); // G reversed
            rgba.push(((i * 7) % 256) as u8); // B strided
            rgba.push(255);
        }

        let src = engine.upload_srgb8(&ctx, &rgba, w, h);
        let linear = engine.linearise(&ctx, &src);
        let shown = engine.display(&ctx, &linear);
        let back = engine.readback8(&ctx, &shown).unwrap();

        assert_eq!(back.len(), rgba.len());
        let mut worst = 0i16;
        for (i, (a, b)) in rgba.iter().zip(back.iter()).enumerate() {
            let d = (i16::from(*a) - i16::from(*b)).abs();
            worst = worst.max(d);
            assert!(d <= 1, "byte {i}: {a} → {b} (Δ{d})");
        }
        eprintln!("worst Δ = {worst}");
    }

    /// The working texture really is fp16 linear: mid-grey sRGB 128 must
    /// round-trip through a value near 0.216 linear, not 0.5 — proven by the
    /// round trip staying exact where a linear-as-srgb confusion would clamp
    /// or shift the dark end.
    #[test]
    fn dark_end_precision_survives_fp16() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let engine = ColourEngine::new(&ctx);
        // The 64 darkest values — where fp16-in-linear-light is tightest.
        let (w, h) = (8u32, 8u32);
        let mut rgba = Vec::new();
        for i in 0..64u8 {
            rgba.extend_from_slice(&[i, i, i, 255]);
        }
        let src = engine.upload_srgb8(&ctx, &rgba, w, h);
        let back = engine
            .readback8(&ctx, &engine.display(&ctx, &engine.linearise(&ctx, &src)))
            .unwrap();
        for (i, (a, b)) in rgba.iter().zip(back.iter()).enumerate() {
            let d = (i16::from(*a) - i16::from(*b)).abs();
            assert!(d <= 1, "dark byte {i}: {a} → {b}");
        }
    }
}

pub mod composite;
pub mod oklab;
pub use composite::{camera_matrix, Blend, CompositeLayer, Compositor, MatteInput};
pub use glam::Mat4;
