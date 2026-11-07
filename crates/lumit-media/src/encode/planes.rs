//! Writing into an encoder's frames: a blank one for the test encode, views of
//! the audio planes, and the copies that fill the picture planes. Each raw
//! pointer touch is kept small and checked.

use crate::MediaError;
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;

/// A black frame in the encoder's own pixel format, used by the test encode.
pub(super) fn blank_frame(pix_fmt: i32, width: u32, height: u32) -> Result<AVFrame, MediaError> {
    let mut frame = AVFrame::new();
    let w = i32::try_from(width).map_err(|_| MediaError::Ffmpeg("frame width overflows".into()))?;
    let h =
        i32::try_from(height).map_err(|_| MediaError::Ffmpeg("frame height overflows".into()))?;
    frame.set_width(w);
    frame.set_height(h);
    frame.set_format(pix_fmt);
    frame
        .alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    let rows = usize::try_from(h).unwrap_or(0);
    let chroma_rows = rows.div_ceil(2);
    // The ten-bit ProRes layouts: every plane is full height, and zero is a
    // valid sample, which is all a test encode needs.
    if matches!(
        pix_fmt,
        ffi::AV_PIX_FMT_YUV422P10LE | ffi::AV_PIX_FMT_YUVA444P10LE
    ) {
        let planes = if pix_fmt == ffi::AV_PIX_FMT_YUVA444P10LE {
            4
        } else {
            3
        };
        for p in 0..planes {
            fill_plane(frame.data[p], frame.linesize[p], rows, 0)?;
        }
        return Ok(frame);
    }
    // Y = 16, chroma = 128 is video-range black for both layouts we feed.
    fill_plane(frame.data[0], frame.linesize[0], rows, 16)?;
    match pix_fmt {
        ffi::AV_PIX_FMT_NV12 => {
            fill_plane(frame.data[1], frame.linesize[1], chroma_rows, 128)?;
        }
        _ => {
            fill_plane(frame.data[1], frame.linesize[1], chroma_rows, 128)?;
            fill_plane(frame.data[2], frame.linesize[2], chroma_rows, 128)?;
        }
    }
    Ok(frame)
}

/// Fill one frame plane with a byte value — a raw-pointer touch, kept small
/// and auditable like `copy_pixels_into`.
fn fill_plane(ptr: *mut u8, linesize: i32, rows: usize, value: u8) -> Result<(), MediaError> {
    if ptr.is_null() {
        return Err(MediaError::Ffmpeg("encode frame has no data plane".into()));
    }
    let stride = usize::try_from(linesize).unwrap_or(0);
    let len = stride
        .checked_mul(rows)
        .ok_or_else(|| MediaError::Ffmpeg("frame plane size overflows".into()))?;
    // SAFETY: the frame was just filled by `alloc_buffer`, which allocates at
    // least `linesize * rows` bytes per plane; `stride` and `rows` are read
    // from that same frame, and the null check above rules out the one case
    // rsmpeg cannot statically guarantee.
    #[allow(unsafe_code)]
    let dst = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
    dst.fill(value);
    Ok(())
}

/// View an audio plane as `len` f32 samples — the audio twin of
/// `fill_plane`, with the same null discipline plus an alignment check.
pub(super) fn plane_f32_mut<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [f32], MediaError> {
    if ptr.is_null() {
        return Err(MediaError::Ffmpeg("audio frame has no data plane".into()));
    }
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<f32>()) {
        return Err(MediaError::Ffmpeg("audio plane misaligned".into()));
    }
    // SAFETY: the frame was just filled by `alloc_buffer` with
    // `nb_samples >= len` samples of AV_SAMPLE_FMT_FLTP (4 bytes each), the
    // null and alignment checks above hold, and FFmpeg's allocator aligns
    // planes far beyond 4 bytes.
    #[allow(unsafe_code)]
    unsafe {
        Ok(std::slice::from_raw_parts_mut(ptr.cast::<f32>(), len))
    }
}

/// View an audio plane as `len` i16 samples — the packed-PCM twin of
/// [`plane_f32_mut`], with the same null and alignment discipline.
pub(super) fn plane_i16_mut<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [i16], MediaError> {
    if ptr.is_null() {
        return Err(MediaError::Ffmpeg("audio frame has no data plane".into()));
    }
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<i16>()) {
        return Err(MediaError::Ffmpeg("audio plane misaligned".into()));
    }
    // SAFETY: the frame was just filled by `alloc_buffer` with `nb_samples`
    // samples of AV_SAMPLE_FMT_S16 across two channels in one packed plane
    // (`len` i16s, 2 bytes each); the null and alignment checks above hold,
    // and FFmpeg's allocator aligns planes far beyond 2 bytes.
    #[allow(unsafe_code)]
    unsafe {
        Ok(std::slice::from_raw_parts_mut(ptr.cast::<i16>(), len))
    }
}

/// View an audio plane as `len` i32 samples — the 24-bit twin of
/// [`plane_i16_mut`], with the same null and alignment discipline.
pub(super) fn plane_i32_mut<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [i32], MediaError> {
    if ptr.is_null() {
        return Err(MediaError::Ffmpeg("audio frame has no data plane".into()));
    }
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<i32>()) {
        return Err(MediaError::Ffmpeg("audio plane misaligned".into()));
    }
    // SAFETY: the frame was just filled by `alloc_buffer` with `nb_samples`
    // samples of AV_SAMPLE_FMT_S32 across the track's channels in one packed
    // plane (`len` i32s, 4 bytes each); the null and alignment checks above
    // hold, and FFmpeg's allocator aligns planes far beyond 4 bytes.
    #[allow(unsafe_code)]
    unsafe {
        Ok(std::slice::from_raw_parts_mut(ptr.cast::<i32>(), len))
    }
}

/// One float sample as a signed 32-bit PCM word, whose top three bytes are
/// what `pcm_s24le` writes. Full scale is 8388607 in the written 24 bits, so
/// the shift lands each sample exactly where the encoder reads it — the same
/// clip-not-wrap rule as [`f32_to_i16`].
pub(super) fn f32_to_i32(s: f32) -> i32 {
    ((f64::from(s.clamp(-1.0, 1.0)) * 8_388_607.0).round() as i32) << 8
}

/// One float sample as signed 16-bit PCM: full scale is 32767, and anything
/// past ±1.0 clips rather than wrapping (`as` on an out-of-range float
/// saturates in Rust, but the clamp says so where a reader can see it).
pub(super) fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Exact byte length of one packed frame at `bytes_per_px`, checked against
/// overflow so a nonsensical width/height (bad caller input, not file input,
/// but the rule is the same: no panics — docs/14-ENGINEERING-RULES.md §4)
/// errors instead of overflowing `i32` arithmetic.
pub(super) fn frame_len(width: i32, height: i32, bytes_per_px: usize) -> Result<usize, MediaError> {
    let bytes = i32::try_from(bytes_per_px)
        .map_err(|_| MediaError::Ffmpeg("frame dimensions overflow".into()))?;
    width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(bytes))
        .and_then(|len| usize::try_from(len).ok())
        .ok_or_else(|| MediaError::Ffmpeg("frame dimensions overflow".into()))
}

/// Scatter interleaved scene-linear float RGBA into a planar `gbrapf32le`
/// frame — the one shape OpenEXR's encoder takes.
///
/// The mirror of the float decode's interleave (docs/impl/media-io.md §5a):
/// G, B, R, A planes out of R, G, B, A samples in, four bytes moved per
/// sample and nothing converted.
pub(super) fn copy_planar_f32_into(
    frame: &mut AVFrame,
    rgba: &[u8],
    width: i32,
    height: i32,
) -> Result<(), MediaError> {
    let w = usize::try_from(width).unwrap_or(0);
    let h = usize::try_from(height).unwrap_or(0);
    let row = w.saturating_mul(4);
    if rgba.len() < w.saturating_mul(h).saturating_mul(16) {
        return Err(MediaError::Ffmpeg(
            "float buffer too small for frame".into(),
        ));
    }
    // Plane order is G, B, R, A; the source is R, G, B, A.
    for (plane, channel) in [(0usize, 1usize), (1, 2), (2, 0), (3, 3)] {
        let stride = usize::try_from(frame.linesize[plane]).unwrap_or(0);
        if stride < row {
            return Err(MediaError::Ffmpeg(
                "encode frame stride smaller than one row".into(),
            ));
        }
        let Some(ptr) = frame.data.get(plane).copied().filter(|p| !p.is_null()) else {
            return Err(MediaError::Ffmpeg("encode frame has no data plane".into()));
        };
        let buf_len = stride
            .checked_mul(h)
            .ok_or_else(|| MediaError::Ffmpeg("encode frame buffer size overflow".into()))?;
        // SAFETY: `frame` was just filled by `alloc_buffer` in `write_rgba`,
        // which allocates at least `linesize[i] * height` bytes for each plane
        // this format has; `stride`/`h` are read from that same frame, the null
        // check above rules out the one case rsmpeg cannot statically
        // guarantee, and the stride check keeps every write below in bounds.
        #[allow(unsafe_code)]
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr, buf_len) };
        for y in 0..h {
            for x in 0..w {
                let from = (y * w + x) * 16 + channel * 4;
                let to = y * stride + x * 4;
                let Some(sample) = rgba.get(from..from + 4) else {
                    continue;
                };
                dst[to..to + 4].copy_from_slice(sample);
            }
        }
    }
    Ok(())
}

/// Copy tight packed rows into the (possibly padded) AVFrame planes, any pixel
/// size, optionally swapping each `u16` sample's two bytes on the way (the
/// big-endian still formats). The one raw-pointer touch of the frame paths,
/// kept small and auditable.
pub(super) fn copy_pixels_into(
    frame: &mut AVFrame,
    rgba: &[u8],
    width: i32,
    height: i32,
    bytes_per_px: usize,
    swap16: bool,
) -> Result<(), MediaError> {
    let stride = usize::try_from(frame.linesize[0]).unwrap_or(0);
    let row = usize::try_from(width)
        .unwrap_or(0)
        .saturating_mul(bytes_per_px);
    let height = usize::try_from(height).unwrap_or(0);
    if stride < row {
        return Err(MediaError::Ffmpeg(
            "encode frame stride smaller than one row".into(),
        ));
    }
    if frame.data[0].is_null() {
        return Err(MediaError::Ffmpeg("encode frame has no data plane".into()));
    }
    let buf_len = stride
        .checked_mul(height)
        .ok_or_else(|| MediaError::Ffmpeg("encode frame buffer size overflow".into()))?;
    if rgba.len() < row.saturating_mul(height) {
        return Err(MediaError::Ffmpeg("rgba buffer too small for frame".into()));
    }
    // SAFETY: `frame` was just filled by `alloc_buffer` in `write_rgba`,
    // which allocates at least `linesize[0] * height` bytes for plane 0;
    // `stride`/`height` are read from that same frame, the null check above
    // rules out the one case rsmpeg cannot statically guarantee, and the
    // stride/row check makes every row write below stay in bounds.
    #[allow(unsafe_code)]
    let dst = unsafe { std::slice::from_raw_parts_mut(frame.data[0], buf_len) };
    for y in 0..height {
        let line = &mut dst[y * stride..y * stride + row];
        line.copy_from_slice(&rgba[y * row..(y + 1) * row]);
        if swap16 {
            for pair in line.chunks_exact_mut(2) {
                pair.swap(0, 1);
            }
        }
    }
    Ok(())
}
