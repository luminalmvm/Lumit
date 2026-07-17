//! The GPU effect kernels (docs/05 crate table: "WGSL effect kernels" live
//! here; docs/08-EFFECTS.md §1.1 part 2 — the production path). Each kernel
//! mirrors its CPU reference in `lumit_core::fx::cpu` op-for-op; the §1.6
//! oracle tests at the bottom hold the two to agreement.
//!
//! In plain terms: this is where effects actually run during preview and
//! export — small GPU programs working on the same linear fp16 textures the
//! compositor uses. The engine takes plain numbers (a blur radius in pixels,
//! an edge mode), so it neither knows nor cares about the project model.

use crate::{GpuContext, GpuError, WORKING_FORMAT};

/// One resolved blur, in raster pixels (the caller converts from the
/// spec's %-of-diagonal units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurOp {
    pub radius_px: f32,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// One resolved sharpen (docs/08 §3.9), amounts already fractional and the
/// gaussian radius already in raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenOp {
    /// Fraction of the detail signal added back (0..3 = 0–300%).
    pub amount: f32,
    pub radius_px: f32,
    /// Linear-light soft gate under which detail is left alone.
    pub threshold: f32,
    /// True: sharpen the Rec. 709 luma only.
    pub luma_only: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurParams {
    dir: [f32; 2],
    radius: f32,
    sigma: f32,
    edge: u32,
    mix_amt: f32,
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SharpenParams {
    amount: f32,
    threshold: f32,
    luma_only: u32,
    mix_amt: f32,
}

/// The effect-pass engine: compiled kernels plus their layouts, one per
/// device (owned alongside the Compositor by whoever renders).
pub struct FxEngine {
    blur: wgpu::ComputePipeline,
    sharpen_unpremultiply: wgpu::ComputePipeline,
    sharpen_combine: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl FxEngine {
    pub fn new(ctx: &GpuContext) -> Self {
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-pl"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let module = |wgsl: &str, name: &str| {
            ctx.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                })
        };
        let pipeline = |shader: &wgpu::ShaderModule, name: &str, entry: &str| {
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let blur_mod = module(include_str!("fx_blur.wgsl"), "fx-blur");
        let sharpen_mod = module(include_str!("fx_sharpen.wgsl"), "fx-sharpen");
        let blur = pipeline(&blur_mod, "fx-blur", "blur_pass");
        let sharpen_unpremultiply = pipeline(&sharpen_mod, "fx-sharpen-un", "unpremultiply");
        let sharpen_combine = pipeline(&sharpen_mod, "fx-sharpen", "sharpen_combine");
        Self {
            blur,
            sharpen_unpremultiply,
            sharpen_combine,
            layout,
        }
    }

    /// Apply one gaussian blur to a linear working texture, returning a new
    /// texture of the same size (two separable passes; the host Mix blends
    /// the final pass against the untouched input, docs/08 §1.5).
    pub fn blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BlurOp,
    ) -> wgpu::Texture {
        let tmp = work_texture(ctx, w, h, "fx-blur-tmp");
        let out = work_texture(ctx, w, h, "fx-blur-out");
        let sigma = (op.radius_px * 0.5).max(1e-3);
        // Horizontal into tmp (mix 1: the blend happens once, at the end).
        self.dispatch(
            ctx,
            &self.blur,
            src,
            src,
            &tmp,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [1.0, 0.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: 1.0,
                _pad: [0.0; 2],
            }),
        );
        // Vertical into out, blending against the original input.
        self.dispatch(
            ctx,
            &self.blur,
            &tmp,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [0.0, 1.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one unsharp mask (docs/08 §3.9) to a linear working texture,
    /// returning a new texture of the same size. Four passes: unpremultiply
    /// (§2.2, fused into the kernel chain), a separable gaussian on the
    /// unpremultiplied colour (reusing the blur kernel, Repeat edges — the
    /// CPU reference blurs with the same fixed policy), then the combine
    /// pass that gates, re-premultiplies and applies the host Mix.
    pub fn sharpen(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SharpenOp,
    ) -> wgpu::Texture {
        let un = work_texture(ctx, w, h, "fx-sharpen-un");
        let tmp = work_texture(ctx, w, h, "fx-sharpen-tmp");
        let blurred = work_texture(ctx, w, h, "fx-sharpen-blur");
        let out = work_texture(ctx, w, h, "fx-sharpen-out");
        let params = SharpenParams {
            amount: op.amount,
            threshold: op.threshold,
            luma_only: u32::from(op.luma_only),
            mix_amt: op.mix,
        };
        self.dispatch(
            ctx,
            &self.sharpen_unpremultiply,
            src,
            src,
            &un,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        let sigma = (op.radius_px * 0.5).max(1e-3);
        for (pass_src, pass_dst, dir) in [(&un, &tmp, [1.0, 0.0]), (&tmp, &blurred, [0.0, 1.0])] {
            self.dispatch(
                ctx,
                &self.blur,
                pass_src,
                pass_src,
                pass_dst,
                w,
                h,
                bytemuck::bytes_of(&BlurParams {
                    dir,
                    radius: op.radius_px,
                    sigma,
                    edge: 1, // Repeat, always (see the schema comment)
                    mix_amt: 1.0,
                    _pad: [0.0; 2],
                }),
            );
        }
        self.dispatch(
            ctx,
            &self.sharpen_combine,
            &blurred,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        out
    }

    /// One compute pass: `src` and `orig` sampled, `dst` written, `params`
    /// as the uniform — the shared plumbing every kernel dispatch uses.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        src: &wgpu::Texture,
        orig: &wgpu::Texture,
        dst: &wgpu::Texture,
        w: u32,
        h: u32,
        params: &[u8],
    ) {
        use wgpu::util::DeviceExt;
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-params"),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-bind"),
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
                    resource: wgpu::BindingResource::TextureView(
                        &orig.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &dst.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn work_texture(ctx: &GpuContext, w: u32, h: u32, label: &str) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: WORKING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// Upload a linear f32 RGBA image as a working (fp16) texture — test and
/// tooling support for effect kernels.
pub fn upload_linear_f32(ctx: &GpuContext, rgba: &[f32], w: u32, h: u32) -> wgpu::Texture {
    let tex = work_texture(ctx, w, h, "fx-upload");
    let halfs: Vec<u16> = rgba.iter().map(|v| f16_bits(*v)).collect();
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&halfs),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 8),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    tex
}

/// Read a working (fp16) texture back as linear f32 RGBA — the exact-linear
/// counterpart of `ColourEngine::readback8`, for oracle tests.
pub fn readback_linear_f32(
    ctx: &GpuContext,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Result<Vec<f32>, GpuError> {
    let row_bytes = w * 8;
    let padded = row_bytes.div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fx-readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fx-readback-enc"),
        });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|e| GpuError::Readback(e.to_string()))?
        .map_err(|e| GpuError::Readback(e.to_string()))?;
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let row = &data[(y * padded) as usize..(y * padded + row_bytes) as usize];
        for c in row.chunks_exact(2) {
            out.push(f16_to_f32(u16::from_le_bytes([c[0], c[1]])));
        }
    }
    Ok(out)
}

/// f32 → IEEE 754 half bits (the working format's texel channel).
pub fn f16_bits(v: f32) -> u16 {
    half::f16::from_f32(v).to_bits()
}

/// IEEE 754 half bits → f32.
pub fn f16_to_f32(bits: u16) -> f32 {
    half::f16::from_bits(bits).to_f32()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn f16_round_trips_representative_values() {
        for v in [0.0f32, 1.0, -1.0, 0.5, 4.0, 1.5e-5, 65504.0] {
            let rt = f16_to_f32(f16_bits(v));
            assert!((rt - v).abs() <= (v.abs() * 1e-3).max(1e-6), "{v} → {rt}");
        }
    }

    /// The §1.6 oracle corpus: a diagonal gradient, a hard alpha edge down
    /// the middle, and an HDR spike — already fp16-quantised, so comparisons
    /// isolate the kernel maths from upload rounding.
    fn corpus(w: u32, h: u32) -> Vec<f32> {
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let g = (x + y) as f32 / (w + h) as f32;
                let a = if x < w / 2 { 1.0 } else { 0.0 };
                img[i] = g * a;
                img[i + 1] = (1.0 - g) * a;
                img[i + 2] = 0.25 * a;
                img[i + 3] = a;
            }
        }
        let spike = ((10 * w + 20) * 4) as usize;
        img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);
        img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
    }

    /// Worst absolute difference between two images.
    fn worst_diff(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
    }

    /// The §1.6 oracle: the WGSL blur agrees with the CPU reference on a
    /// corpus of gradient + alpha edge + HDR spike, per edge policy — and is
    /// bit-stable against itself (§2.4 determinism).
    #[test]
    fn wgsl_blur_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        // Corpus (docs/08 §1.6): a diagonal gradient, a hard alpha edge down
        // the middle, and an HDR spike.
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let g = (x + y) as f32 / (w + h) as f32;
                let a = if x < w / 2 { 1.0 } else { 0.0 };
                img[i] = g * a;
                img[i + 1] = (1.0 - g) * a;
                img[i + 2] = 0.25 * a;
                img[i + 3] = a;
            }
        }
        let spike = ((10 * w + 20) * 4) as usize;
        img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);

        for edge in [0u32, 1, 2] {
            for (radius, mix) in [(3.0f32, 1.0f32), (7.5, 0.6), (0.0, 1.0)] {
                // fp16 quantise the input exactly as the GPU sees it, so the
                // comparison isolates the blur maths from upload rounding.
                let quantised: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
                let mut cpu = quantised.clone();
                lumit_core::fx::cpu::blur_gaussian(&mut cpu, w, h, radius, edge, mix);

                let tex = upload_linear_f32(&ctx, &img, w, h);
                let op = BlurOp {
                    radius_px: radius,
                    edge,
                    mix,
                };
                let out = fx.blur(&ctx, &tex, w, h, &op);
                let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

                let worst = cpu
                    .iter()
                    .zip(&gpu)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0f32, f32::max);
                // Moderate-class perceptual epsilon (§1.6), scaled for the
                // HDR corpus: fp16 has ~2^-11 relative steps, and the spike
                // sits at 6.0.
                assert!(
                    worst < 2e-2,
                    "edge {edge} radius {radius} mix {mix}: worst diff {worst}"
                );

                // Determinism: a second run is bit-identical to the first.
                let out2 = fx.blur(&ctx, &tex, w, h, &op);
                let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
                assert_eq!(gpu, gpu2, "GPU blur must be bit-stable");
            }
        }
    }

    /// The §1.6 oracle for sharpen: WGSL agrees with the CPU reference on
    /// the corpus across parameter sweeps, and is bit-stable (§2.4). The
    /// internal gaussian's intermediates round through fp16 textures on the
    /// GPU and stay f32 on the CPU, so the bound is an absolute epsilon:
    /// 5e-3 ≈ 1–2 fp16 ULP at the corpus's HDR peak of 6.0 (measured worst
    /// on NVIDIA: 2.9e-3).
    #[test]
    fn wgsl_sharpen_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (amount, radius, threshold, luma_only, mix) in [
            (0.6f32, 3.0f32, 0.05f32, true, 1.0f32),
            (1.5, 6.0, 0.0, false, 0.7),
            (3.0, 2.0, 0.2, true, 1.0),
            (0.0, 3.0, 0.0, true, 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::sharpen(&mut cpu, w, h, amount, radius, threshold, luma_only, mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = SharpenOp {
                amount,
                radius_px: radius,
                threshold,
                luma_only,
                mix,
            };
            let out = fx.sharpen(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_diff(&cpu, &gpu);
            // Logged so real cross-vendor deltas accumulate (docs/08 open
            // question 5: the class tolerances are placeholders until then).
            eprintln!("sharpen a={amount} r={radius} t={threshold}: worst {worst:.2e}");
            assert!(
                worst < 5e-3,
                "amount {amount} radius {radius} threshold {threshold} \
                 luma {luma_only} mix {mix}: worst diff {worst}"
            );

            let out2 = fx.sharpen(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU sharpen must be bit-stable");
        }
    }
}
