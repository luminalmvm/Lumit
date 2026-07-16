//! Video encoding (docs/06-RENDER-PIPELINE.md §7 export, v0).
//!
//! In plain terms: the compositor hands over finished RGBA frames; this
//! module converts and compresses them into an .mp4. v0 is H.264 via x264
//! (software — the NVENC/AMF/QSV hardware ladder joins on Windows per the
//! spec) with yuv420p output for universal playback. Audio joins with the
//! export-queue work.

use crate::MediaError;
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextOutput;
use rsmpeg::avutil::{AVFrame, AVRational};
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use std::ffi::CString;
use std::path::Path;

pub struct Encoder {
    output: AVFormatContextOutput,
    encoder: AVCodecContext,
    sws: SwsContext,
    width: i32,
    height: i32,
    next_pts: i64,
    finished: bool,
}

impl Encoder {
    /// Open an H.264/mp4 encoder at the given size and frame rate.
    pub fn open(
        path: &Path,
        width: u32,
        height: u32,
        fps_num: i32,
        fps_den: i32,
    ) -> Result<Self, MediaError> {
        Self::open_with_bitrate(path, width, height, fps_num, fps_den, None)
    }

    /// As [`Self::open`], with an explicit average bitrate in bits/second
    /// (size-targeted share exports, K-037).
    pub fn open_with_bitrate(
        path: &Path,
        width: u32,
        height: u32,
        fps_num: i32,
        fps_den: i32,
        bit_rate: Option<i64>,
    ) -> Result<Self, MediaError> {
        let cpath = CString::new(path.to_str().ok_or(MediaError::BadPath)?)
            .map_err(|_| MediaError::BadPath)?;
        let mut output = AVFormatContextOutput::create(&cpath)?;

        let codec = AVCodec::find_encoder(ffi::AV_CODEC_ID_H264)
            .ok_or_else(|| MediaError::Ffmpeg("no H.264 encoder linked".into()))?;
        let mut encoder = AVCodecContext::new(&codec);
        let width = i32::try_from(width).map_err(|_| MediaError::BadPath)?;
        let height = i32::try_from(height).map_err(|_| MediaError::BadPath)?;
        encoder.set_width(width);
        encoder.set_height(height);
        encoder.set_time_base(AVRational {
            num: fps_den,
            den: fps_num,
        });
        encoder.set_framerate(AVRational {
            num: fps_num,
            den: fps_den,
        });
        encoder.set_pix_fmt(ffi::AV_PIX_FMT_YUV420P);
        encoder.set_gop_size(30);
        if let Some(rate) = bit_rate {
            encoder.set_bit_rate(rate);
        }
        // Sensible default quality; the export dialogue's rate controls land
        // with the queue (07-UI-SPEC export settings).
        let mut opts = None;
        encoder.open(opts.take())?;

        {
            let mut stream = output.new_stream();
            stream.set_codecpar(encoder.extract_codecpar());
            stream.set_time_base(AVRational {
                num: fps_den,
                den: fps_num,
            });
        }
        output.write_header(&mut None)?;

        let sws = SwsContext::get_context(
            width,
            height,
            ffi::AV_PIX_FMT_RGBA,
            width,
            height,
            ffi::AV_PIX_FMT_YUV420P,
            ffi::SWS_BILINEAR,
            None,
            None,
            None,
        )
        .ok_or_else(|| MediaError::Ffmpeg("swscale for encode".into()))?;

        Ok(Self {
            output,
            encoder,
            sws,
            width,
            height,
            next_pts: 0,
            finished: false,
        })
    }

    /// Encode one tightly-packed RGBA frame (sRGB-encoded display output).
    pub fn write_rgba(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        let expect = rgba_frame_len(self.width, self.height)?;
        if rgba.len() != expect {
            return Err(MediaError::Ffmpeg(format!(
                "frame size {} != expected {expect}",
                rgba.len()
            )));
        }
        // RGBA source frame borrowing the caller's bytes via copy (v0).
        let mut src = AVFrame::new();
        src.set_width(self.width);
        src.set_height(self.height);
        src.set_format(ffi::AV_PIX_FMT_RGBA);
        src.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        copy_rgba_into(&mut src, rgba, self.width, self.height)?;

        let mut dst = AVFrame::new();
        dst.set_width(self.width);
        dst.set_height(self.height);
        dst.set_format(ffi::AV_PIX_FMT_YUV420P);
        dst.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        self.sws
            .scale_frame(&src, 0, self.height, &mut dst)
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        dst.set_pts(self.next_pts);
        self.next_pts += 1;

        self.encoder.send_frame(Some(&dst))?;
        self.drain(false)
    }

    fn drain(&mut self, at_eof: bool) -> Result<(), MediaError> {
        loop {
            match self.encoder.receive_packet() {
                Ok(mut packet) => {
                    packet.set_stream_index(0);
                    // INVARIANT: `open_with_bitrate` always adds exactly one
                    // output stream before an `Encoder` exists, so index 0
                    // is always present for the lifetime of `self`.
                    packet.rescale_ts(self.encoder.time_base, self.output.streams()[0].time_base);
                    self.output.write_frame(&mut packet)?;
                }
                Err(rsmpeg::error::RsmpegError::EncoderDrainError) if !at_eof => return Ok(()),
                Err(rsmpeg::error::RsmpegError::EncoderDrainError)
                | Err(rsmpeg::error::RsmpegError::EncoderFlushedError) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Flush and write the container trailer. Must be called exactly once.
    pub fn finish(&mut self) -> Result<(), MediaError> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.encoder.send_frame(None)?;
        self.drain(true)?;
        self.output.write_trailer()?;
        Ok(())
    }
}

/// Exact byte length of one packed RGBA frame, checked against overflow so a
/// nonsensical width/height (bad caller input, not file input, but the rule
/// is the same: no panics — docs/14-ENGINEERING-RULES.md §4) errors instead
/// of overflowing `i32` arithmetic.
fn rgba_frame_len(width: i32, height: i32) -> Result<usize, MediaError> {
    width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(4))
        .and_then(|len| usize::try_from(len).ok())
        .ok_or_else(|| MediaError::Ffmpeg("frame dimensions overflow".into()))
}

/// Copy tight RGBA rows into the (possibly padded) AVFrame planes — the one
/// raw-pointer touch of the encode path, kept small and auditable.
fn copy_rgba_into(
    frame: &mut AVFrame,
    rgba: &[u8],
    width: i32,
    height: i32,
) -> Result<(), MediaError> {
    let stride = usize::try_from(frame.linesize[0]).unwrap_or(0);
    let row = usize::try_from(width).unwrap_or(0).saturating_mul(4);
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
        dst[y * stride..y * stride + row].copy_from_slice(&rgba[y * row..(y + 1) * row]);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Self-verifying loop: encode a gradient sweep, then probe and index the
    /// file with our OWN readers — dimensions, rate, and frame count must
    /// round-trip exactly.
    #[test]
    fn encoded_file_round_trips_through_our_own_probe_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp4");
        let (w, h, frames) = (320u32, 240u32, 90usize);

        let mut enc = Encoder::open(&path, w, h, 60, 1).unwrap();
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for f in 0..frames {
            for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                let x = (i as u32 % w) as u8;
                px[0] = x.wrapping_add(f as u8);
                px[1] = (f * 2) as u8;
                px[2] = 128;
                px[3] = 255;
            }
            enc.write_rgba(&rgba).unwrap();
        }
        enc.finish().unwrap();

        let probe = crate::probe::probe(&path).unwrap();
        let video = probe.video.unwrap();
        assert_eq!((video.width, video.height), (w, h));
        // Container-declared average rate is advisory (rounding-prone);
        // the frame index's pts-derived estimate is what Luminal trusts.
        assert!((video.fps() - 60.0).abs() < 1.5, "fps {}", video.fps());
        assert!((probe.duration_seconds - 1.5).abs() < 0.1);

        let index = crate::index::build_frame_index(&path).unwrap();
        assert_eq!(index.frame_count(), frames);
        assert!(!index.vfr);
        assert!(
            (index.fps_estimate() - 60.0).abs() < 0.01,
            "index fps {}",
            index.fps_estimate()
        );
    }

    /// Regression: `width * height * 4` used to be raw `i32` arithmetic in
    /// `write_rgba`, which overflow-panics (debug builds) or wraps into a
    /// wrong, too-small size (release builds) for large-but-plausible
    /// dimensions. It must report a typed error instead.
    #[test]
    fn rgba_frame_len_errors_instead_of_overflowing() {
        // 50,000 x 50,000 x 4 overflows i32::MAX (2,147,483,647).
        assert!(rgba_frame_len(50_000, 50_000).is_err());
        assert_eq!(rgba_frame_len(2, 2).unwrap(), 16);
        assert_eq!(rgba_frame_len(320, 240).unwrap(), 320 * 240 * 4);
    }

    #[test]
    fn rgba_frame_len_rejects_negative_dimensions_without_panicking() {
        assert!(rgba_frame_len(-1, 100).is_err());
    }
}
