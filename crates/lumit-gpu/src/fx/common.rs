//! Shared GPU input/output helpers for the effect kernels: uploading images,
//! LUT cubes, flow fields and depth maps into the textures the kernels sample,
//! reading a working texture back - to linear f32 for the oracle tests, or to
//! the halves themselves for an effect that works in fp16 - and the fp16
//! conversions the oracle tests round-trip through.

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
///
/// The texture must be [`crate::WORKING_FORMAT`] itself, not merely whatever
/// [`GpuContext::working`] happens to be: `read_back_rows` refuses the rest,
/// for the reason stated there.
///
/// # Errors
///
/// [`GpuError::Readback`] if the texture is not the fp16 working format, if the
/// buffer never maps, or if the mapped bytes are not the shape the copy asked
/// for. A read-back of no area at all is an empty `Vec`, not an error.
pub fn readback_linear_f32(
    ctx: &GpuContext,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Result<Vec<f32>, GpuError> {
    let mut out = vec![0f32; (w as usize) * (h as usize) * 4];
    read_back_rows(ctx, tex, w, h, &mut |y, row| {
        // The destination row is the source row's own length, so the two cannot
        // come to disagree about how wide a row is - `convert_to_f32_slice`
        // would be a panic rather than a `GpuError` if they ever did.
        let dst = out
            .get_mut(y * row.len()..(y + 1) * row.len())
            .ok_or_else(|| GpuError::Readback("readback output shorter than the picture".into()))?;
        // Eight values an instruction where the host has F16C, a whole row at a
        // time. At 8K the per-value loop was 237 ms.
        row.convert_to_f32_slice(dst);
        Ok(())
    })?;
    Ok(out)
}

/// Read a working (fp16) texture back as the **halves themselves** - the twin
/// of [`upload_linear_f16`], and the host side of docs/impl/lfx.md §4.5.
///
/// # In plain terms
///
/// [`readback_linear_f32`] widens every channel on the way out, and whoever
/// uploads the answer narrows it again. For an fp32 project that is the right
/// shape and costs nothing that was not already lost. For an **fp16 project**
/// it is a conversion either side of an effect that works in fp16 and was told
/// there would be none. This reads the same texture without touching a value:
/// `w * h * 4` IEEE 754 half bits, RGBA, row-major, tightly packed, in exactly
/// the layout [`upload_linear_f16`] takes back.
///
/// It shares [`readback_linear_f32`]'s body rather than copying it, which is
/// deliberate: the `ctx.flush()` that stops a read-back running ahead of the
/// drawing that filled the texture is the one line a second copy would be
/// likely to leave out, and a shared body cannot leave it out at all.
///
/// What comes back is the **bits**, as `u16`, which is what
/// [`upload_linear_f16`] takes and not what the hook the halves are for takes:
/// `EffectDef::apply_f16_temporal` is handed a `&mut [half::f16]`, so the pass
/// that joins the two casts the slice with `half`'s
/// `HalfBitsSliceExt::reinterpret_cast_mut` - a second view of the same bytes,
/// never a conversion. The pair either side of the card is therefore in one
/// type and the effect hook in the other, on purpose: a read-back hands back a
/// picture's bits, and the cast is where they become numbers.
///
/// # Errors
///
/// [`GpuError::Readback`] if the texture is not the fp16 working format, if the
/// buffer never maps, or if the mapped bytes are not the shape the copy asked
/// for. A read-back of no area at all is an empty `Vec`, not an error.
pub fn readback_linear_f16(
    ctx: &GpuContext,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Result<Vec<u16>, GpuError> {
    let mut out = vec![0u16; (w as usize) * (h as usize) * 4];
    read_back_rows(ctx, tex, w, h, &mut |y, row| {
        // As in the f32 twin: the destination row is the source row's own
        // length, so `copy_from_slice` cannot be handed two different widths.
        let dst = out
            .get_mut(y * row.len()..(y + 1) * row.len())
            .ok_or_else(|| GpuError::Readback("readback output shorter than the picture".into()))?;
        dst.copy_from_slice(row.reinterpret_cast());
        Ok(())
    })?;
    Ok(out)
}

/// What [`read_back_rows`] hands each row to: the row's index from the top, and
/// its `w * 4` halves with the padding already dropped.
type RowSink<'a> = dyn FnMut(usize, &[half::f16]) -> Result<(), GpuError> + 'a;

/// The copy both read-backs are: flush, copy the texture into a staging buffer
/// whose rows are padded to the 256 bytes wgpu demands, wait for it, and hand
/// each row to `row`.
///
/// The rows are read as fp16 texels and nothing further down asks the texture
/// what it is, so the format is checked here rather than assumed.
/// [`GpuContext::working`] follows the project's colour depth - `Rgba8Unorm` at
/// eight bits, `Rgba32Float` at thirty-two - and every working texture is made
/// in it, so "the working texture" is only sometimes an fp16 one. Read as fp16
/// the eight-bit one gives a copy that is *valid and wrong*: its row is half
/// the length, so half of what comes back is the staging buffer's own padding
/// read as picture. The thirty-two-bit one gives a copy wgpu refuses, and an
/// uncaptured device error ends the process - a panic reached from library
/// code. A typed refusal instead: both callers propagate it, and the render
/// path turns it into a passthrough with a note (docs/14 §4).
fn read_back_rows(
    ctx: &GpuContext,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
    row: &mut RowSink<'_>,
) -> Result<(), GpuError> {
    if tex.format() != crate::WORKING_FORMAT {
        return Err(GpuError::Readback(format!(
            "read-back wants {:?}, texture is {:?}",
            crate::WORKING_FORMAT,
            tex.format()
        )));
    }
    // No area is no rows. A width of nought would ask for a zero-sized staging
    // buffer, which wgpu refuses, and then chunk the mapped bytes into rows of
    // nought, which is a panic; both callers already size the empty answer.
    if w == 0 || h == 0 {
        return Ok(());
    }

    // Inside a frame batch every pass records into one shared encoder that is
    // submitted when the frame ends, and the copy below submits at once. Without
    // this it would run ahead of the drawing that filled the texture and read
    // back zeroes, which is the empty frame every OFX plugin was being handed.
    ctx.flush();

    // Eight bytes a texel, which is what the format check above makes true. A
    // width whose row does not fit a `u32` is not a texture any card will make,
    // but the arithmetic says so rather than wrapping round to one that does.
    let row_bytes = w
        .checked_mul(8)
        .ok_or_else(|| GpuError::Readback(format!("a width of {w} has no fp16 row")))?;
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
    // A row at a time, straight out of the mapped bytes. The chunk is the padded
    // row and the slice handed on is the real one, so the padding at the end of
    // a row is skipped rather than read as picture.
    let bits: &[u16] =
        bytemuck::try_cast_slice(&data).map_err(|e| GpuError::Readback(e.to_string()))?;
    let halfs: &[half::f16] = bits.reinterpret_cast();
    let tight = (w as usize) * 4;
    for (y, padded_row) in halfs.chunks_exact((padded / 2) as usize).enumerate() {
        row(
            y,
            padded_row
                .get(..tight)
                .ok_or_else(|| GpuError::Readback("short readback row".into()))?,
        )?;
    }
    Ok(())
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
                "pixel {}, channel {}: {want}",
                i / 4,
                i % 4
            );
        }
    }

    /// The fp16 read-back is a copy, not a conversion (docs/impl/lfx.md §4.5).
    ///
    /// Most awkward halves prove nothing about which path was taken: f32 holds
    /// every finite half exactly, and an fp16 subnormal widens into an ordinary
    /// f32 normal, so both zeroes, both infinities, the largest half and the
    /// smallest subnormal all come back from a trip through f32 unchanged. The
    /// one value that does not is a **signalling NaN**, which any widening
    /// quiets: `0x7C01` goes out as `0x7FC02000` and comes back `0x7E01`. So a
    /// read-back written as "widen, then narrow" - the conversion this seam
    /// exists to remove, and what a later refactor reusing the f32 body might
    /// write - fails here, and a bit copy passes.
    ///
    /// A width of 33 makes the row 264 bytes, which is not a multiple of 256,
    /// so the staging buffer pads it and the padding must be dropped rather
    /// than read as picture; fourteen values over a row of 132 channels put a
    /// different one of them under that padding on every row. And the f32 twin
    /// reading the same texture must be exactly the widening of these bits,
    /// because the two share a body and would otherwise be reading different
    /// pictures out of one texture.
    #[test]
    fn halves_read_back_are_the_bits_that_were_uploaded() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let (w, h) = (33u32, 5u32);
        let awkward: [u16; 14] = [
            0x0000, // +0
            0x8000, // -0
            0x0001, // the smallest subnormal
            0x8001, // and its negative
            0x7BFF, // 65504, the largest half
            0xFBFF, // -65504
            0x7C00, // +inf
            0xFC00, // -inf
            0x7C01, // a signalling NaN: 0x7E01 after any trip through f32
            0xFC01, // and its negative
            0x3C00, // 1.0
            0xBC00, // -1.0
            0x0400, // the smallest normal
            0x3555, // a third, rounded
        ];
        let src: Vec<u16> = (0..(w as usize) * (h as usize) * 4)
            .map(|i| awkward[i % awkward.len()])
            .collect();
        let tex = upload_linear_f16(&ctx, &src, w, h);

        let halves = readback_linear_f16(&ctx, &tex, w, h).unwrap();
        assert_eq!(halves.len(), src.len());
        for (i, (got, want)) in halves.iter().zip(&src).enumerate() {
            assert_eq!(
                got,
                want,
                "pixel {}, channel {}: read back {got:#06x}, uploaded {want:#06x} - the fp16 path converted something",
                i / 4,
                i % 4
            );
        }

        let floats = readback_linear_f32(&ctx, &tex, w, h).unwrap();
        assert_eq!(floats.len(), src.len());
        for (i, (got, want)) in floats.iter().zip(&src).enumerate() {
            assert_eq!(
                got.to_bits(),
                f16_to_f32(*want).to_bits(),
                "pixel {}, channel {}: the two read-backs disagree about one texture",
                i / 4,
                i % 4
            );
        }
    }

    /// A read-back is fp16 or it is a refusal (docs/impl/lfx.md §4.5, D7).
    ///
    /// [`GpuContext::working`] follows the project's colour depth and every
    /// working texture is made in it, so an eight-bit project's is `Rgba8Unorm`
    /// and a thirty-two-bit one's is `Rgba32Float`. Read as fp16 the first
    /// hands back half of the staging buffer's padding as picture and says
    /// nothing; the second is a copy wgpu refuses, and an uncaptured device
    /// error ends the process. Both read-backs refuse first instead, with a
    /// `GpuError` the render path already turns into a passthrough with a note.
    #[test]
    fn a_read_back_refuses_a_texture_that_is_not_the_working_depth() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let (w, h) = (4u32, 4u32);
        for bits in [8u32, 32] {
            ctx.set_working_bits(bits);
            // A card without `FLOAT32_FILTERABLE` is given sixteen bits instead
            // of thirty-two, and then there is nothing here to refuse.
            if ctx.working() == crate::WORKING_FORMAT {
                continue;
            }
            let tex = new_work_texture(&ctx, w, h, "fx-wrong-depth");
            // The sentence names what it was handed as well as what it wanted,
            // because "read-back wants Rgba16Float" on its own leaves whoever
            // reads the note guessing which depth the project was in.
            let handed = format!("{:?}", tex.format());
            match readback_linear_f16(&ctx, &tex, w, h) {
                Err(GpuError::Readback(why)) => assert!(
                    why.contains(&handed),
                    "the refusal does not say it was handed {handed}: {why}"
                ),
                other => panic!("a {bits}-bit working texture was read back as halves: {other:?}"),
            }
            match readback_linear_f32(&ctx, &tex, w, h) {
                Err(GpuError::Readback(why)) => assert!(
                    why.contains(&handed),
                    "the refusal does not say it was handed {handed}: {why}"
                ),
                other => panic!("a {bits}-bit working texture was read back as floats: {other:?}"),
            }
        }
        // Back to sixteen for whatever runs next. `SharedGpu::reset` does this
        // too, for the test that leaves here by failing rather than by falling
        // off the end.
        ctx.set_working_bits(16);
    }

    /// A read-back of no area at all is an empty picture, not a fault.
    ///
    /// A width of nought makes the staging row nought bytes: wgpu refuses a
    /// zero-sized buffer, and `chunks_exact` panics on a chunk of nought - in a
    /// crate whose lints deny panics outside tests. Both wrappers answer with
    /// the empty `Vec` they had already sized.
    #[test]
    fn a_read_back_of_no_area_is_empty_rather_than_a_fault() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let tex = new_work_texture(&ctx, 4, 4, "fx-no-area");
        for (w, h) in [(0u32, 4u32), (4, 0), (0, 0)] {
            assert!(
                readback_linear_f16(&ctx, &tex, w, h).unwrap().is_empty(),
                "{w}x{h} read halves back"
            );
            assert!(
                readback_linear_f32(&ctx, &tex, w, h).unwrap().is_empty(),
                "{w}x{h} read floats back"
            );
        }
    }

    /// A fp16 read-back inside a frame batch sees the frame's own drawing.
    ///
    /// The twin of `a_readback_inside_a_batch_waits_for_what_is_still_batched`,
    /// and here because trap 3 of docs/impl/lfx.md is precisely that a second
    /// read-back written beside the first is where the `ctx.flush()` gets left
    /// out. Inside a batch every pass records into one shared encoder that is
    /// submitted when the frame ends; a read-back that submits its own encoder
    /// at once would reach the queue ahead of the drawing that filled the
    /// texture and come back all zeroes. Take the flush out of `read_back_rows`
    /// and this fails with every half nought.
    #[test]
    fn halves_read_back_inside_a_batch_wait_for_what_is_still_batched() {
        let Ok(ctx) = GpuContext::headless() else {
            crate::no_adapter();
            return;
        };
        let (w, h) = (4u32, 4u32);
        // Half grey, opaque, in every texel. A queue write is ordered against
        // submissions, so this lands before anything below.
        let texel = [f16_bits(0.5), f16_bits(0.5), f16_bits(0.5), f16_bits(1.0)];
        let src: Vec<u16> = (0..(w * h) as usize).flat_map(|_| texel).collect();
        let src = upload_linear_f16(&ctx, &src, w, h);
        let dst = new_work_texture(&ctx, w, h, "batch-f16-dst");

        ctx.begin_frame();
        {
            // Recorded into the frame's shared encoder, so not submitted when
            // the guard drops, exactly as every pass in a real frame is.
            let mut encoder = ctx.encoder("batch-f16-copy");
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &src,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &dst,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let read = readback_linear_f16(&ctx, &dst, w, h);
        ctx.end_frame();

        let read = read.expect("the read-back itself failed");
        let nonzero = read.iter().filter(|v| **v != 0).count();
        assert_eq!(
            nonzero,
            read.len(),
            "an fp16 read-back inside a batch returned {} zeroes of {}: the copy was submitted ahead of the batch it should have queued behind",
            read.len() - nonzero,
            read.len()
        );
    }
}
