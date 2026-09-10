//! Shared GPU input/output helpers for the effect kernels: uploading images,
//! LUT cubes, flow fields and depth maps into the textures the kernels sample,
//! reading a working texture back to linear f32, and the fp16 conversions the
//! oracle tests round-trip through.

use half::slice::{HalfBitsSliceExt, HalfFloatSliceExt};

use crate::{GpuContext, GpuError};

use super::new_work_texture;

/// Upload a linear f32 RGBA image as a working (fp16) texture — test and
/// tooling support for effect kernels.
pub fn upload_linear_f32(ctx: &GpuContext, rgba: &[f32], w: u32, h: u32) -> wgpu::Texture {
    // The whole plane in one call. `half` narrows eight values an instruction
    // where the card's host has F16C, and rounds to nearest even either way, so
    // the bits are the ones the per-value loop gave. At 8K the loop was 294 ms.
    let mut halfs = vec![half::f16::ZERO; rgba.len()];
    halfs.convert_from_f32_slice(rgba);
    upload_linear_f16(ctx, halfs.reinterpret_cast(), w, h)
}

/// The same upload, for a caller that already holds the fp16 bits — one that
/// can produce them without an f32 plane in between. `halfs` is RGBA, four
/// per pixel.
pub fn upload_linear_f16(ctx: &GpuContext, halfs: &[u16], w: u32, h: u32) -> wgpu::Texture {
    let tex = new_work_texture(ctx, w, h, "fx-upload");
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(halfs),
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

/// Upload a 3D colour LUT cube as an `rgba32float` 3D texture for
/// [`FxEngine::lut`]. `data` is `size³` RGB triplets, **red-fastest** (flat
/// index `r + g*size + b*size*size`, the layout `lumit_core::lut::Lut3d`
/// stores). Each triplet is padded to RGBA (alpha 1.0, unused) and written at
/// full f32 precision, so the shader's manual trilinear lookup reads the exact
/// samples the CPU oracle interpolates — the only fp16 rounding is then the
/// colour output at the working texture, matching the other tap-based kernels.
/// The `textureLoad` axis order `(x=r, y=g, z=b)` mirrors the red-fastest flat
/// index, so no transpose. `bytes_per_row = size*16` (four f32 channels),
/// `rows_per_image = size`, depth = size.
pub fn upload_lut_3d(ctx: &GpuContext, size: u32, data: &[[f32; 3]]) -> wgpu::Texture {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fx-lut-3d"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: size,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let count = (size as usize)
        .saturating_mul(size as usize)
        .saturating_mul(size as usize);
    let mut rgba = vec![0f32; count * 4];
    for (i, c) in data.iter().take(count).enumerate() {
        rgba[i * 4] = c[0];
        rgba[i * 4 + 1] = c[1];
        rgba[i * 4 + 2] = c[2];
        rgba[i * 4 + 3] = 1.0;
    }
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&rgba),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 16),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: size,
        },
    );
    tex
}

/// Upload a dense flow field (per-pixel `(u, v)` motion, raster pixels, plus a
/// per-pixel confidence `conf` in 0..1) as an `rgba32float` texture for
/// [`FxEngine::motion_blur`]. `u`, `v` and `conf` are row-major, one entry per
/// pixel (`w × h`). rgba32float, not the working fp16 format, so the kernel
/// reads the exact f32 vectors the CPU oracle integrates — the only fp16
/// rounding then is the colour taps, matching the other tap-based kernels.
/// Interleaved `[u, v, conf, 0]` per texel; `textureLoad` in the kernel reads
/// `.xy` for the motion and `.z` for the confidence (FX-19). Datamosh shares
/// this texture and reads only `.xy`, so a missing/uniform `conf` is harmless
/// to it. A short `conf` (fewer entries than pixels) reads as full confidence
/// where absent, so an older caller degrades to the plain smear.
pub fn upload_flow_field(
    ctx: &GpuContext,
    u: &[f32],
    v: &[f32],
    conf: &[f32],
    w: u32,
    h: u32,
) -> wgpu::Texture {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fx-flow-field"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let n = (w * h) as usize;
    let mut interleaved = vec![0f32; n * 4];
    for i in 0..n {
        interleaved[i * 4] = u.get(i).copied().unwrap_or(0.0);
        interleaved[i * 4 + 1] = v.get(i).copied().unwrap_or(0.0);
        // Absent confidence reads as full (no streak scaling).
        interleaved[i * 4 + 2] = conf.get(i).copied().unwrap_or(1.0);
    }
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&interleaved),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 16),
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

/// Upload a per-pixel depth map (one value per pixel, row-major, `w × h`) as a
/// single-channel `r32float` texture for [`FxEngine::dof`]. r32float, not the
/// working fp16 format, so the kernel reads the exact f32 depths the CPU oracle
/// turns into circle-of-confusion radii — the only fp16 rounding is then the
/// colour taps, matching the flow-field and other tap-based kernels. Values are
/// depth in `[0, 1]` by convention (near..far), but the kernel clamps its ramp
/// so any finite input is defined. `textureLoad` in the kernel reads `.x`;
/// `bytes_per_row = w*4`.
pub fn upload_depth_map(ctx: &GpuContext, depth: &[f32], w: u32, h: u32) -> wgpu::Texture {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fx-depth-map"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let n = (w * h) as usize;
    let mut data = vec![0f32; n];
    data[..n.min(depth.len())].copy_from_slice(&depth[..n.min(depth.len())]);
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
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
    // Inside a frame batch every pass records into one shared encoder that is
    // submitted when the frame ends, and the copy below submits at once. Without
    // this it would run ahead of the drawing that filled the texture and read
    // back zeroes, which is the empty frame every OFX plugin was being handed.
    ctx.flush();

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
    ctx.submit([enc.finish()]);
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
    // A row at a time, straight out of the mapped bytes: same widening, eight
    // values an instruction where the host has F16C. The chunk is the padded
    // row and the slice is the real one, so the padding is skipped rather than
    // converted. At 8K the per-value loop was 237 ms.
    let bits: &[u16] =
        bytemuck::try_cast_slice(&data).map_err(|e| GpuError::Readback(e.to_string()))?;
    let halfs: &[half::f16] = bits.reinterpret_cast();
    let mut out = vec![0f32; (w * h * 4) as usize];
    for (row, dst) in halfs
        .chunks_exact((padded / 2) as usize)
        .zip(out.chunks_mut(row_bytes as usize / 2))
    {
        row.get(..dst.len())
            .ok_or_else(|| GpuError::Readback("short readback row".into()))?
            .convert_to_f32_slice(dst);
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

    /// Upload a plane of the awkward values and read it straight back: the
    /// largest half, both zeroes, a float too small to be a half at all, and
    /// negatives of each. Every channel returns the bits the per-value
    /// narrowing gives, and a width whose row is 264 bytes, not a multiple of
    /// 256, says the padding at the end of a row is skipped and not read as
    /// picture.
    #[test]
    fn a_padded_row_round_trips_to_the_same_bits() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let (w, h) = (33u32, 5u32);
        let awkward = [
            65504.0f32, -65504.0, 0.0, -0.0, 1e-8, -1e-8, 1.0, -1.0, 6.1e-5, 0.333, -0.333, 1024.5,
        ];
        let src: Vec<f32> = (0..(w * h * 4) as usize)
            .map(|i| awkward[i % awkward.len()])
            .collect();
        let tex = upload_linear_f32(&ctx, &src, w, h);
        let out = readback_linear_f32(&ctx, &tex, w, h).unwrap();
        assert_eq!(out.len(), src.len());
        for (i, (got, want)) in out.iter().zip(&src).enumerate() {
            assert_eq!(
                got.to_bits(),
                f16_to_f32(f16_bits(*want)).to_bits(),
                "channel {i}: {want}"
            );
        }
    }
}
