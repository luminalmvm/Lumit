//! Exact-frame video decoding (docs/impl/media-io.md §3-§4).
//!
//! In plain terms: to show frame N, we jump to the nearest keyframe at or
//! before N (from the frame index), then decode forward, discarding frames
//! until the exact timestamp matches. "Close enough" comparisons are the
//! classic off-by-one-frame scrubbing bug — we compare pts exactly against
//! the index, which came from the same container.
//!
//! Decoding itself takes the fastest path the machine offers (§4's v1
//! baseline): on Windows the bitstream is decoded by the graphics card's
//! fixed-function video unit (D3D11VA) and the finished picture transferred
//! back to ordinary memory, where the same conversion the software path uses
//! turns it into RGBA. Anything about that failing — no hardware, an
//! unsupported codec — falls back to software decoding with all cores,
//! never an error.

use crate::index::FrameIndex;
use crate::sequence::MediaSource;
use crate::MediaError;
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use rsmpeg::UnsafeDerefMut;

/// A decoded frame as straight (non-premultiplied) RGBA8, sRGB-encoded.
/// Linearisation happens on the GPU per docs/06-RENDER-PIPELINE.md; this CPU
/// struct is the hand-off format for upload.
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A decoded frame as single-channel luma, at the file's own raster size.
///
/// The tracker reads nothing but brightness, and asking for RGBA to
/// throw two thirds of it away costs a full colour conversion and four times the
/// bytes on every frame of a clip. Every pixel format ffmpeg decodes has a
/// gray8 conversion, and for the planar YUV that video actually arrives in it is
/// a plane copy.
pub struct LumaFrame {
    pub width: u32,
    pub height: u32,
    /// Row-major, `width · height` long, 0..1.
    pub luma: Vec<f32>,
}

pub struct VideoDecoder {
    input: AVFormatContextInput,
    decoder: AVCodecContext,
    stream_index: i32,
    index: FrameIndex,
    /// Frame number the decoder will produce next if we keep reading forward.
    next_sequential: Option<usize>,
    /// How many times this decoder has had to seek. Exposed by [`Self::seeks`]
    /// because seeking is the difference between realtime playback and a
    /// slideshow, and "did that need a seek?" is a far steadier thing to assert
    /// than "was that fast?".
    seeks: usize,
    /// Whether frames are decoded by the graphics card's video unit
    /// (docs/impl/media-io.md §4). Diagnostic — the pixels and the pts logic
    /// are identical either way.
    hardware: bool,
}

/// Build and open a codec context for `codec`/`par`, with the D3D11VA
/// hardware device attached when `try_hw` asks for it and the codec supports
/// it. Returns whether hardware is actually in use. A context that fails to
/// open is unusable, which is why the caller retries with a fresh one in
/// software rather than reusing this one.
fn open_codec_ctx(
    codec: &AVCodec,
    par: &rsmpeg::avcodec::AVCodecParameters,
    try_hw: bool,
) -> Result<(AVCodecContext, bool), MediaError> {
    let mut ctx = AVCodecContext::new(codec);
    ctx.apply_codecpar(par)?;
    // Library-default libav is SINGLE-threaded (unlike the ffmpeg CLI); 0 asks
    // for automatic frame/slice threading across the machine's cores, which is
    // the difference between one core grinding 4K H.264 and all of them.
    // SAFETY: plain field write on the owned, not-yet-opened context — the
    // same pattern rsmpeg's own setters use.
    #[allow(unsafe_code)]
    unsafe {
        ctx.deref_mut().thread_count = 0;
    }
    let hardware = try_hw && attach_d3d11va(codec, &mut ctx);
    ctx.open(None)?;
    Ok((ctx, hardware))
}

/// Attach a D3D11VA hardware device to `ctx` when this codec supports
/// device-context hardware decode (docs/impl/media-io.md §4, v1 baseline).
/// libav's default format negotiation then selects the hardware path on its
/// own. False — leaving the context untouched for software — when there is no
/// support or no device; never an error.
#[cfg(windows)]
fn attach_d3d11va(codec: &AVCodec, ctx: &mut AVCodecContext) -> bool {
    let supported = (0..).map_while(|i| codec.hw_config(i)).any(|c| {
        c.device_type == ffi::AV_HWDEVICE_TYPE_D3D11VA
            && (c.methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX) != 0
    });
    if !supported {
        return false;
    }
    match rsmpeg::avutil::AVHWDeviceContext::create(ffi::AV_HWDEVICE_TYPE_D3D11VA, None, None, 0) {
        Ok(device) => {
            ctx.set_hw_device_ctx(device);
            true
        }
        Err(_) => false,
    }
}

/// The macOS sibling is VideoToolbox and lands with the macOS pass;
/// until then every other platform decodes in software, on all cores.
#[cfg(not(windows))]
fn attach_d3d11va(_codec: &AVCodec, _ctx: &mut AVCodecContext) -> bool {
    false
}

impl VideoDecoder {
    pub fn open(src: impl Into<MediaSource>, index: FrameIndex) -> Result<Self, MediaError> {
        Self::open_with(src, index, true)
    }

    /// As [`Self::open`], with hardware decode refusable — the knob the
    /// decoder settings page will drive, and what the hw/sw agreement test
    /// pins its ground truth with.
    pub fn open_with(
        src: impl Into<MediaSource>,
        index: FrameIndex,
        allow_hardware: bool,
    ) -> Result<Self, MediaError> {
        let input = crate::probe::open_input(&src.into())?;
        let (stream_index, par) = input
            .streams()
            .iter()
            .find(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_VIDEO)
            .map(|s| (s.index, s.codecpar().clone()))
            .ok_or(MediaError::NoStreams)?;
        let codec = rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id)
            .ok_or_else(|| MediaError::Ffmpeg("no decoder for codec".into()))?;
        // Hardware first; a context that fails to open with the device attached
        // is rebuilt fresh in software (fallback, not error — §4).
        let (decoder, hardware) = open_codec_ctx(&codec, &par, allow_hardware)
            .or_else(|_| open_codec_ctx(&codec, &par, false))?;
        Ok(Self {
            input,
            decoder,
            stream_index,
            index,
            next_sequential: Some(0),
            seeks: 0,
            hardware,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.index.frame_count()
    }

    /// How many seeks this decoder has performed since it was opened.
    pub fn seeks(&self) -> usize {
        self.seeks
    }

    /// Whether this decoder runs on the graphics card's video unit
    /// (docs/impl/media-io.md §4). Diagnostic only.
    pub fn is_hardware(&self) -> bool {
        self.hardware
    }

    /// Decode exactly frame `n`, optionally scaled to `target_width`
    /// (aspect-preserving) — true raster downsampling for preview resolution.
    pub fn frame_rgba(
        &mut self,
        n: usize,
        target_width: Option<u32>,
    ) -> Result<DecodedFrame, MediaError> {
        let frame = self.frame_exact(n)?;
        convert_rgba(&frame, target_width)
    }

    /// Decode exactly frame `n` as luma at the file's own raster size — the
    /// tracker's tap (docs/impl/tracking.md §1: source raster pixels, no
    /// comp scaling).
    ///
    /// Deliberately unscaled: the tracker measures sub-pixel motion in *source*
    /// pixels, and a preview-tier downsample would silently change what a
    /// solved focal length means.
    pub fn frame_luma(&mut self, n: usize) -> Result<LumaFrame, MediaError> {
        let frame = self.frame_exact(n)?;
        convert_luma(&frame)
    }

    /// Decode exactly frame `n` and hand back the picture in the shared planar
    /// form both taps convert from — hardware frames transferred to system
    /// memory and repacked, exactly as they were before either tap existed.
    fn frame_exact(&mut self, n: usize) -> Result<AVFrame, MediaError> {
        let want_pts = self
            .index
            .pts_of_frame(n)
            .ok_or_else(|| MediaError::Ffmpeg(format!("frame {n} out of range")))?;

        // Where a seek would land, and therefore whether one is worth doing.
        let key = self.index.nearest_keyframe_at_or_before(n);

        // **Seek only when it actually saves work.** The decoder is already
        // positioned somewhere; a seek costs a backwards jump *and* a
        // `flush_buffers`, after which the frames between the keyframe and `n`
        // have to be decoded anyway. So it only pays when the keyframe is ahead
        // of where we already are — otherwise decoding forward from here is
        // strictly less work.
        //
        // **Why this matters far more than it looks.** Playing forward while
        // dropping frames — which is exactly what adaptive playback does the
        // moment it falls behind — asks for n, then n+2, then n+3, and the old
        // condition (`next_sequential != Some(n)`) called every one of those a
        // seek. Measured on 1080p60: 4.4 ms a frame decoding sequentially
        // (227 fps), 92 ms a frame when every request seeks (11 fps) — twenty
        // times slower. So the first dropped frame made decoding twenty times
        // more expensive, which dropped more frames, which seeked further. The
        // whole collapse followed from one frame arriving late.
        let need_seek = match self.next_sequential {
            // Already positioned to produce n next.
            Some(m) if m == n => false,
            // Ahead of us, with no keyframe in between worth jumping to: decode
            // forward through the gap, discarding what is not wanted.
            Some(m) if m <= n && key <= m => false,
            // Behind us, or a keyframe closer to n than we are: seek.
            _ => true,
        };
        if need_seek {
            let key_pts = self
                .index
                .pts_of_frame(key)
                .ok_or_else(|| MediaError::Ffmpeg("index inconsistent".into()))?;
            self.input
                .seek(self.stream_index, key_pts, ffi::AVSEEK_FLAG_BACKWARD as i32)?;
            self.decoder.flush_buffers();
            self.next_sequential = Some(key);
            self.seeks += 1;
        }

        loop {
            let frame = self.next_decoded_frame()?;
            let pts = if frame.pts != ffi::AV_NOPTS_VALUE {
                frame.pts
            } else {
                frame.best_effort_timestamp
            };
            if pts == want_pts {
                self.next_sequential = Some(n + 1);
                // A hardware frame's pixels live on the graphics card; bring
                // them to system memory (NV12) so the same swscale conversion
                // the software path uses runs on them (§4's v1 baseline —
                // the one-copy interop is the recorded follow-up).
                let frame = if frame.hw_frames_ctx.is_null() {
                    frame
                } else {
                    let mut sw = AVFrame::new();
                    sw.hwframe_transfer_data(&frame)
                        .map_err(|e| MediaError::Ffmpeg(format!("hw frame transfer: {e}")))?;
                    // Repack semi-planar NV12 as planar yuv420p — a pure
                    // layout change, no resampling — because swscale's nv12
                    // and yuv420p RGB conversions interpolate chroma
                    // DIFFERENTLY (measured: 9% of bytes off, up to 161, on
                    // a test pattern's edges). Preview == export and
                    // cross-machine determinism need one conversion, so the
                    // hardware path is made to look exactly like software
                    // before the shared RGBA step.
                    if sw.format == ffi::AV_PIX_FMT_NV12 {
                        deinterleave_to_yuv420p(&sw)?
                    } else {
                        sw
                    }
                };
                return Ok(frame);
            }
            if pts > want_pts {
                // Should not happen with an exact index; be honest about it.
                return Err(MediaError::Ffmpeg(format!(
                    "seek overshot: wanted pts {want_pts}, got {pts}"
                )));
            }
        }
    }

    fn next_decoded_frame(&mut self) -> Result<AVFrame, MediaError> {
        loop {
            match self.decoder.receive_frame() {
                Ok(frame) => return Ok(frame),
                Err(rsmpeg::error::RsmpegError::DecoderDrainError)
                | Err(rsmpeg::error::RsmpegError::DecoderFlushedError) => {}
                Err(e) => return Err(e.into()),
            }
            // Need more input.
            loop {
                match self.input.read_packet()? {
                    Some(packet) => {
                        if packet.stream_index == self.stream_index {
                            self.decoder.send_packet(Some(&packet))?;
                            break;
                        }
                    }
                    None => {
                        self.decoder.send_packet(None)?; // drain at EOF
                        break;
                    }
                }
            }
        }
    }
}

/// Repack a hardware-transferred NV12 frame as planar yuv420p. Same-size
/// point "scale" through swscale is a lossless plane copy plus chroma
/// deinterleave — no values change, only the memory layout, so the shared
/// RGBA conversion behaves identically to the software decoder's output.
fn deinterleave_to_yuv420p(src: &AVFrame) -> Result<AVFrame, MediaError> {
    let mut sws = SwsContext::get_context(
        src.width,
        src.height,
        src.format,
        src.width,
        src.height,
        ffi::AV_PIX_FMT_YUV420P,
        ffi::SWS_POINT,
        None,
        None,
        None,
    )
    .ok_or_else(|| MediaError::Ffmpeg("nv12 repack context creation failed".into()))?;
    let mut out = AVFrame::new();
    out.set_width(src.width);
    out.set_height(src.height);
    out.set_format(ffi::AV_PIX_FMT_YUV420P);
    out.alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    sws.scale_frame(src, 0, src.height, &mut out)?;
    Ok(out)
}

/// The luma tap: the same swscale call the RGBA path makes, asking for gray8 at
/// the source's own size, then a byte-to-0..1 map.
///
/// One conversion for every pixel format rather than a fast path for planar YUV
/// and a fallback for the rest: for yuv420p — which is what the software and the
/// repacked hardware path both produce — swscale's gray8 output *is* the Y
/// plane, so the branch would buy nothing but a second thing to keep correct.
fn convert_luma(frame: &AVFrame) -> Result<LumaFrame, MediaError> {
    let (w, h) = (frame.width, frame.height);
    let mut sws = SwsContext::get_context(
        w,
        h,
        frame.format,
        w,
        h,
        ffi::AV_PIX_FMT_GRAY8,
        ffi::SWS_POINT,
        None,
        None,
        None,
    )
    .ok_or_else(|| MediaError::Ffmpeg("gray8 context creation failed".into()))?;
    let mut out = AVFrame::new();
    out.set_width(w);
    out.set_height(h);
    out.set_format(ffi::AV_PIX_FMT_GRAY8);
    out.alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    sws.scale_frame(frame, 0, h, &mut out)?;

    let stride = usize::try_from(out.linesize[0]).unwrap_or(0);
    let width = u32::try_from(w).unwrap_or(0);
    let height = u32::try_from(h).unwrap_or(0);
    let row_bytes = width as usize;
    let buf_len = stride
        .checked_mul(height as usize)
        .ok_or_else(|| MediaError::Ffmpeg("luma frame buffer size overflow".into()))?;
    let data = unsafe_data_slice(&out, buf_len)?;
    let tight = copy_tight_rows(data, stride, row_bytes, height as usize)?;
    Ok(LumaFrame {
        width,
        height,
        luma: tight.iter().map(|v| f32::from(*v) / 255.0).collect(),
    })
}

fn convert_rgba(frame: &AVFrame, target_width: Option<u32>) -> Result<DecodedFrame, MediaError> {
    let src_w = frame.width;
    let src_h = frame.height;
    let dst_w = target_width
        .map(|w| i32::try_from(w).unwrap_or(src_w))
        .unwrap_or(src_w)
        .clamp(2, src_w.max(2));
    // Preserve aspect; keep even dimensions (some swscale paths prefer it).
    let dst_h =
        ((i64::from(src_h) * i64::from(dst_w) / i64::from(src_w.max(1))) as i32).max(2) & !1;

    let mut sws = SwsContext::get_context(
        src_w,
        src_h,
        frame.format,
        dst_w,
        dst_h,
        ffi::AV_PIX_FMT_RGBA,
        ffi::SWS_BILINEAR,
        None,
        None,
        None,
    )
    .ok_or_else(|| MediaError::Ffmpeg("swscale context creation failed".into()))?;

    let mut out = AVFrame::new();
    out.set_width(dst_w);
    out.set_height(dst_h);
    out.set_format(ffi::AV_PIX_FMT_RGBA);
    out.alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    sws.scale_frame(frame, 0, src_h, &mut out)?;

    // Copy out of the AVFrame's padded rows into a tight RGBA buffer.
    let stride = usize::try_from(out.linesize[0]).unwrap_or(0);
    let width = u32::try_from(dst_w).unwrap_or(0);
    let height = u32::try_from(dst_h).unwrap_or(0);
    let row_bytes = (width as usize).saturating_mul(4);
    let height_usize = height as usize;
    // Checked (not saturating): if this would overflow, the buffer cannot
    // possibly be that large, so treat it as an error rather than reading
    // out of bounds with a silently-truncated length.
    let buf_len = stride
        .checked_mul(height_usize)
        .ok_or_else(|| MediaError::Ffmpeg("scaled frame buffer size overflow".into()))?;
    let data = unsafe_data_slice(&out, buf_len)?;
    let rgba = copy_tight_rows(data, stride, row_bytes, height_usize)?;
    Ok(DecodedFrame {
        width,
        height,
        rgba,
    })
}

/// Read the frame's first data plane as a byte slice. Kept in one place so
/// the raw-pointer handling is auditable (rsmpeg exposes planes as pointers).
fn unsafe_data_slice(frame: &AVFrame, len: usize) -> Result<&[u8], MediaError> {
    if frame.data[0].is_null() {
        return Err(MediaError::Ffmpeg("decoded frame has no data plane".into()));
    }
    // SAFETY: `frame` (`out` in `convert_rgba`) was just filled by
    // `alloc_buffer` + `sws.scale_frame`, which allocate and write exactly
    // `linesize[0] * height` bytes for plane 0 of a packed RGBA frame; `len`
    // is computed by the caller from those same fields, and the null check
    // above rules out the one case rsmpeg cannot statically guarantee.
    #[allow(unsafe_code)]
    unsafe {
        Ok(std::slice::from_raw_parts(frame.data[0], len))
    }
}

/// Copy tight rows out of a stride-padded buffer into a packed `Vec`. A
/// well-behaved swscale output always satisfies `stride >= row_bytes` and
/// `data.len() >= stride * height`, but we never trust that with a bare
/// slice index (docs/14-ENGINEERING-RULES.md §4: no panics) — an
/// inconsistency here becomes a typed error instead of an out-of-bounds
/// read.
fn copy_tight_rows(
    data: &[u8],
    stride: usize,
    row_bytes: usize,
    height: usize,
) -> Result<Vec<u8>, MediaError> {
    if stride < row_bytes {
        return Err(MediaError::Ffmpeg(
            "scaled output stride smaller than one row".into(),
        ));
    }
    if data.len() < stride.saturating_mul(height) {
        return Err(MediaError::Ffmpeg("scaled output buffer too small".into()));
    }
    let mut out = Vec::with_capacity(row_bytes.saturating_mul(height));
    for row in 0..height {
        let start = row * stride;
        out.extend_from_slice(&data[start..start + row_bytes]);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::index::build_frame_index;
    use crate::index::tests_support::fixture;

    fn frame_hash(f: &DecodedFrame) -> String {
        blake3::hash(&f.rgba).to_hex().to_string()
    }

    /// The luma tap is the same picture, at the same size, seeking the same
    /// way — it just skips the colour.
    ///
    /// **Compared by correlation, not by value.** The two paths are not the
    /// same arithmetic — one is swscale's YUV→gray, the other YUV→RGB followed
    /// by a weighted sum here — and on a saturated colour bar the two
    /// conventions disagree by tens of per cent *legitimately*, because a
    /// different weighting of R, G and B is a different number. So the test
    /// asks the question the tracker asks: is this the same picture? It is
    /// gradient-based and NCC-verified, both blind to a gain and a lift, so
    /// correlation is exactly the right measure — and it is not blind to a wrong
    /// plane, a wrong frame, a wrong raster or upside-down rows, which is the
    /// whole list of ways this can break.
    #[test]
    fn the_luma_tap_is_the_same_frame_as_the_rgba_decode() {
        /// Pearson correlation; `None` where either side is flat and there is
        /// nothing to correlate.
        fn correlation(a: &[f64], b: &[f64]) -> Option<f64> {
            let n = a.len() as f64;
            let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
            let mut cov = 0.0;
            let (mut va, mut vb) = (0.0, 0.0);
            for (x, y) in a.iter().zip(b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            (va > 0.0 && vb > 0.0).then(|| cov / (va * vb).sqrt())
        }

        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let index = build_frame_index(&file).unwrap();
        let mut decoder = VideoDecoder::open(&file, index).unwrap();

        for n in [0usize, 45, 119, 30] {
            let rgba = decoder.frame_rgba(n, None).unwrap();
            let luma = decoder.frame_luma(n).unwrap();
            assert_eq!((luma.width, luma.height), (rgba.width, rgba.height));
            assert_eq!(luma.luma.len(), (rgba.width * rgba.height) as usize);

            let (w, h) = (rgba.width as usize, rgba.height as usize);
            let want: Vec<f64> = rgba
                .rgba
                .chunks_exact(4)
                .map(|px| {
                    (0.299 * f64::from(px[0]) + 0.587 * f64::from(px[1]) + 0.114 * f64::from(px[2]))
                        / 255.0
                })
                .collect();
            let got: Vec<f64> = luma.luma.iter().map(|v| f64::from(*v)).collect();
            let upright = correlation(&got, &want).expect("neither frame is flat");
            assert!(
                upright > 0.9,
                "frame {n}: the luma tap is not the frame the RGBA decode gives ({upright})"
            );

            // The same picture with its rows reversed correlates worse, which is
            // what says the stride walk is the right way up.
            let flipped: Vec<f64> = (0..h)
                .rev()
                .flat_map(|y| want[y * w..(y + 1) * w].iter().copied())
                .collect();
            let mirror = correlation(&got, &flipped).expect("neither frame is flat");
            assert!(
                mirror < upright,
                "frame {n}: upside-down rows would read the same ({mirror} vs {upright})"
            );
        }
    }

    #[test]
    fn seeked_frames_match_sequential_decode_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let index = build_frame_index(&file).unwrap();

        // Sequential ground truth.
        let mut sequential = VideoDecoder::open(&file, index.clone()).unwrap();
        let mut truth = Vec::new();
        for n in 0..sequential.frame_count() {
            truth.push(frame_hash(&sequential.frame_rgba(n, None).unwrap()));
        }

        // Random-access seeks must land on identical pixels.
        let mut seeker = VideoDecoder::open(&file, index).unwrap();
        for n in [0usize, 31, 45, 90, 119, 30, 0, 119] {
            let f = seeker.frame_rgba(n, None).unwrap();
            assert_eq!(frame_hash(&f), truth[n], "frame {n} differs after seek");
            assert_eq!((f.width, f.height), (320, 240));
        }
    }

    /// Hardware decode is an implementation detail, never a look: the frames
    /// the D3D11VA path produces must match the software decoder's. H.264
    /// decoding is bit-exact by spec, so the two only differ if the transfer
    /// or conversion path is wrong; a byte of slack per channel forgives a
    /// driver's chroma rounding without letting a real defect through. Skips
    /// where hardware decode is unavailable (non-Windows, CI, no fixture) —
    /// on those machines the fallback IS the software path.
    #[test]
    fn hardware_and_software_decode_agree_on_the_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let index = build_frame_index(&file).unwrap();
        let mut hw = VideoDecoder::open(&file, index.clone()).unwrap();
        if !hw.is_hardware() {
            eprintln!("skipping: no hardware decoder on this machine");
            return;
        }
        let mut sw = VideoDecoder::open_with(&file, index, false).unwrap();
        assert!(!sw.is_hardware());
        for n in [0usize, 7, 63, 119] {
            let a = hw.frame_rgba(n, None).unwrap();
            let b = sw.frame_rgba(n, None).unwrap();
            assert_eq!((a.width, a.height), (b.width, b.height));
            let worst = a
                .rgba
                .iter()
                .zip(&b.rgba)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            assert!(worst <= 1, "frame {n}: hw and sw differ by {worst}");
        }
    }

    /// **The playback-collapse regression.** Adaptive playback drops frames when
    /// it falls behind, so it asks for n, then n+2, then n+3 — always forward,
    /// never the same frame twice. Every one of those used to count as a seek
    /// (`next_sequential != Some(n)`), and a seek means a backwards jump plus a
    /// `flush_buffers`, throwing away the decoder state that made the *next*
    /// frame cheap.
    ///
    /// Measured on 1080p60: 4.4 ms a frame sequentially (227 fps) against 92 ms
    /// when every request seeks (11 fps). So one late frame made decoding twenty
    /// times dearer, which dropped more frames, which seeked further — playback
    /// collapsed from a single frame of jitter and never recovered.
    ///
    /// Counted rather than timed: "did that need a seek?" is the property, and
    /// it answers the same on a loaded CI box as on a quiet desk.
    #[test]
    fn playing_forward_with_dropped_frames_never_seeks() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let index = build_frame_index(&file).unwrap();
        let mut dec = VideoDecoder::open(&file, index).unwrap();

        dec.frame_rgba(0, None).unwrap();
        let after_start = dec.seeks();

        // Forward in threes, exactly as dropping two frames in three asks for
        // them. The fixture is 120 frames with a keyframe every 30.
        let wanted: Vec<usize> = (1..40).map(|k| k * 3).collect();
        for &n in &wanted {
            dec.frame_rgba(n, None).unwrap();
        }

        // Crossing into a later keyframe may still seek, and should: jumping to
        // it decodes fewer frames than walking there. What must never happen is
        // a seek *per request* — that is the collapse. So the bound is the
        // number of keyframes in the range, not the number of frames asked for.
        let seeks = dec.seeks() - after_start;
        assert!(
            seeks <= 3,
            "at most one seek per keyframe crossed (3 here), not one per frame \
             ({} requests); got {seeks}",
            wanted.len()
        );

        // Going backwards still seeks: there is no way to un-decode, so this is
        // the case the machinery exists for.
        let before_back = dec.seeks();
        dec.frame_rgba(1, None).unwrap();
        assert!(
            dec.seeks() > before_back,
            "a backwards jump has to seek — that is what seeking is for"
        );
    }

    #[test]
    fn preview_downscale_is_true_raster_downsampling() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let index = build_frame_index(&file).unwrap();
        let mut dec = VideoDecoder::open(&file, index).unwrap();
        let half = dec.frame_rgba(10, Some(160)).unwrap();
        assert_eq!((half.width, half.height), (160, 120));
        assert_eq!(half.rgba.len(), 160 * 120 * 4);
    }

    #[test]
    fn seeking_still_lands_exactly_on_a_vfr_source() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = crate::index::tests_support::vfr_fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let index = build_frame_index(&file).unwrap();
        assert!(index.vfr, "fixture should have been detected as VFR");
        let count = index.frame_count();
        assert!(count > 10, "expected several selected frames, got {count}");

        let mut sequential = VideoDecoder::open(&file, index.clone()).unwrap();
        let mut truth = Vec::new();
        for n in 0..count {
            truth.push(frame_hash(&sequential.frame_rgba(n, None).unwrap()));
        }

        // Jump around out of order; every seek must still land on the exact
        // pts the index promised, irregular spacing notwithstanding.
        let mut seeker = VideoDecoder::open(&file, index).unwrap();
        let probes = [0usize, count / 2, count - 1, count / 3, 1, count - 1, 0];
        for n in probes {
            let f = seeker.frame_rgba(n, None).unwrap();
            assert_eq!(frame_hash(&f), truth[n], "frame {n} differs after seek");
        }
    }

    #[test]
    fn video_decoder_open_on_zero_byte_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::index::tests_support::zero_byte_file(dir.path());
        let index = FrameIndex {
            timebase_num: 1,
            timebase_den: 30,
            entries: Vec::new(),
            vfr: false,
            median_delta: 0,
            fingerprint: crate::Fingerprint {
                size: 0,
                mtime_unix: 0,
                content_hash: String::new(),
            },
        };
        assert!(VideoDecoder::open(&path, index).is_err());
    }

    #[test]
    fn video_decoder_open_on_garbage_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::index::tests_support::garbage_file(dir.path());
        let index = FrameIndex {
            timebase_num: 1,
            timebase_den: 30,
            entries: Vec::new(),
            vfr: false,
            median_delta: 0,
            fingerprint: crate::Fingerprint {
                size: 0,
                mtime_unix: 0,
                content_hash: String::new(),
            },
        };
        assert!(VideoDecoder::open(&path, index).is_err());
    }

    #[test]
    fn video_decoder_open_on_truncated_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let truncated = crate::index::tests_support::truncated_copy(&file, dir.path(), 200);
        let index = FrameIndex {
            timebase_num: 1,
            timebase_den: 30,
            entries: Vec::new(),
            vfr: false,
            median_delta: 0,
            fingerprint: crate::Fingerprint {
                size: 0,
                mtime_unix: 0,
                content_hash: String::new(),
            },
        };
        assert!(VideoDecoder::open(&truncated, index).is_err());
    }

    // ---- copy_tight_rows: pure logic, no ffmpeg required ----------------

    #[test]
    fn copy_tight_rows_rejects_stride_smaller_than_row() {
        let data = vec![0u8; 10];
        assert!(copy_tight_rows(&data, 2, 4, 2).is_err());
    }

    #[test]
    fn copy_tight_rows_rejects_buffer_smaller_than_stride_times_height() {
        let data = vec![0u8; 4]; // only one row's worth, height claims two
        assert!(copy_tight_rows(&data, 4, 4, 2).is_err());
    }

    #[test]
    fn copy_tight_rows_strips_padding_correctly() {
        // stride 6, row 4: two rows of [1,2,3,4,<pad>,<pad>]
        #[rustfmt::skip]
        let data = vec![
            1, 2, 3, 4, 9, 9,
            5, 6, 7, 8, 9, 9,
        ];
        let out = copy_tight_rows(&data, 6, 4, 2).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
