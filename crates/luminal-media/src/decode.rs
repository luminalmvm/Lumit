//! Exact-frame video decoding (docs/impl/media-io.md §3).
//!
//! In plain terms: to show frame N, we jump to the nearest keyframe at or
//! before N (from the frame index), then decode forward, discarding frames
//! until the exact timestamp matches. "Close enough" comparisons are the
//! classic off-by-one-frame scrubbing bug — we compare pts exactly against
//! the index, which came from the same container.

use crate::index::FrameIndex;
use crate::MediaError;
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use std::path::Path;

/// A decoded frame as straight (non-premultiplied) RGBA8, sRGB-encoded.
/// Linearisation happens on the GPU per docs/06-RENDER-PIPELINE.md; this CPU
/// struct is the hand-off format for upload.
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct VideoDecoder {
    input: AVFormatContextInput,
    decoder: AVCodecContext,
    stream_index: i32,
    index: FrameIndex,
    /// Frame number the decoder will produce next if we keep reading forward.
    next_sequential: Option<usize>,
}

impl VideoDecoder {
    pub fn open(path: &Path, index: FrameIndex) -> Result<Self, MediaError> {
        let input = crate::probe::open_input(path)?;
        let (stream_index, par) = input
            .streams()
            .iter()
            .find(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_VIDEO)
            .map(|s| (s.index, s.codecpar().clone()))
            .ok_or(MediaError::NoStreams)?;
        let codec = rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id)
            .ok_or_else(|| MediaError::Ffmpeg("no decoder for codec".into()))?;
        let mut decoder = AVCodecContext::new(&codec);
        decoder.apply_codecpar(&par)?;
        decoder.open(None)?;
        Ok(Self {
            input,
            decoder,
            stream_index,
            index,
            next_sequential: Some(0),
        })
    }

    pub fn frame_count(&self) -> usize {
        self.index.frame_count()
    }

    /// Decode exactly frame `n`, optionally scaled to `target_width`
    /// (aspect-preserving) — true raster downsampling for preview resolution.
    pub fn frame_rgba(
        &mut self,
        n: usize,
        target_width: Option<u32>,
    ) -> Result<DecodedFrame, MediaError> {
        let want_pts = self
            .index
            .pts_of_frame(n)
            .ok_or_else(|| MediaError::Ffmpeg(format!("frame {n} out of range")))?;

        // Sequential fast path: already positioned to produce n next.
        let need_seek = self.next_sequential != Some(n);
        if need_seek {
            let key = self.index.nearest_keyframe_at_or_before(n);
            let key_pts = self
                .index
                .pts_of_frame(key)
                .ok_or_else(|| MediaError::Ffmpeg("index inconsistent".into()))?;
            self.input
                .seek(self.stream_index, key_pts, ffi::AVSEEK_FLAG_BACKWARD as i32)?;
            self.decoder.flush_buffers();
            self.next_sequential = Some(key);
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
                return convert_rgba(&frame, target_width);
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
