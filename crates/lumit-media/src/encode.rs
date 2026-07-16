//! Video and audio encoding (docs/06-RENDER-PIPELINE.md §7.4; docs/impl/media-io.md §7).
//!
//! In plain terms: the compositor hands over finished RGBA frames (and the
//! mixer hands over finished stereo samples); this module compresses them
//! into an .mp4. The video encoder is picked from a ladder — NVENC (NVIDIA),
//! then AMF (AMD), then Quick Sync (Intel), then software x264/x265 — and
//! each rung is *proven* with a short test encode before it is trusted,
//! because hardware encoders can exist in the FFmpeg build yet fail at
//! runtime (wrong vendor's GPU, driver sessions exhausted). Whatever rung
//! works first wins; software always works, so export never fails just
//! because a GPU said no. Audio joins as AAC in the same file, interleaved
//! with the video so players can stream it.

use crate::MediaError;
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextOutput;
use rsmpeg::avutil::{AVChannelLayout, AVDictionary, AVFrame, AVRational};
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use std::ffi::CString;
use std::path::Path;

/// Delivery codec choice (docs/06-RENDER-PIPELINE.md §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
}

impl VideoCodec {
    /// User-facing name (glossary voice: plain, no marketing).
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC",
        }
    }
}

/// Everything the video stream needs to open.
#[derive(Debug, Clone)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub fps_num: i32,
    pub fps_den: i32,
    /// Average target bitrate in bits/second; None = encoder default quality.
    pub bit_rate: Option<i64>,
    /// VBR peak in bits/second (docs/06 §7.5 preset table's "peak").
    pub max_rate: Option<i64>,
}

/// Everything the AAC audio stream needs to open.
#[derive(Debug, Clone)]
pub struct AudioSettings {
    /// Sample rate in Hz (delivery presets use 48 000, docs/06 §7.5).
    pub rate: u32,
    /// Bitrate in bits/second (delivery presets use 320 000).
    pub bit_rate: i64,
}

/// The encoder ladder for a codec, best first (docs/impl/media-io.md §7):
/// NVIDIA NVENC → AMD AMF → Intel Quick Sync → software. Pure data so the
/// priority is unit-tested, not folklore.
pub fn encoder_candidates(codec: VideoCodec) -> [&'static str; 4] {
    match codec {
        VideoCodec::H264 => ["h264_nvenc", "h264_amf", "h264_qsv", "libx264"],
        VideoCodec::Hevc => ["hevc_nvenc", "hevc_amf", "hevc_qsv", "libx265"],
    }
}

/// First candidate for which `works` returns true — the fallback rule
/// separated from FFmpeg so the "hardware exists but fails to open" cases
/// are plain unit tests.
pub fn pick_first_working<'a>(
    candidates: &[&'a str],
    mut works: impl FnMut(&'a str) -> bool,
) -> Option<&'a str> {
    candidates.iter().copied().find(|name| works(name))
}

/// The pixel format each encoder is fed. Quick Sync encoders take NV12 only;
/// everything else on the ladder accepts planar 4:2:0.
pub fn pix_fmt_for(encoder: &str) -> i32 {
    if encoder.ends_with("_qsv") {
        ffi::AV_PIX_FMT_NV12
    } else {
        ffi::AV_PIX_FMT_YUV420P
    }
}

/// Calm user-facing name for an encoder ("Encoded with NVENC" style).
pub fn encoder_label(encoder: &str) -> &'static str {
    match encoder {
        "h264_nvenc" | "hevc_nvenc" => "NVENC",
        "h264_amf" | "hevc_amf" => "AMD AMF",
        "h264_qsv" | "hevc_qsv" => "Intel Quick Sync",
        "libx264" => "software x264",
        "libx265" => "software x265",
        _ => "software",
    }
}

/// The audio half of the muxer: AAC context plus the sample bookkeeping.
struct AudioTrack {
    ctx: AVCodecContext,
    /// Output stream index (video is 0, audio is 1).
    stream_index: usize,
    rate: u32,
    /// Samples per AAC frame (1024 for the FFmpeg encoder).
    frame_size: usize,
    /// Interleaved stereo samples not yet handed to the encoder.
    pending: Vec<f32>,
    /// Next frame's pts, counted in samples.
    next_pts: i64,
}

pub struct Encoder {
    output: AVFormatContextOutput,
    video: AVCodecContext,
    video_encoder: &'static str,
    sws: SwsContext,
    encode_pix_fmt: i32,
    width: i32,
    height: i32,
    next_pts: i64,
    audio: Option<AudioTrack>,
    finished: bool,
}

impl Encoder {
    /// Open an mp4 encoder: video per `video`, optional AAC audio per
    /// `audio`. The video encoder is the first ladder rung that survives a
    /// 16-frame test encode (docs/impl/media-io.md §7) — hardware that fails
    /// at runtime falls through to the next rung, never to an error, and
    /// software is always last so opening only fails when even that is
    /// missing from the FFmpeg build.
    pub fn open(
        path: &Path,
        video: &VideoSettings,
        audio: Option<&AudioSettings>,
    ) -> Result<Self, MediaError> {
        let cpath = CString::new(path.to_str().ok_or(MediaError::BadPath)?)
            .map_err(|_| MediaError::BadPath)?;
        let mut output = AVFormatContextOutput::create(&cpath)?;
        let global_header = (output.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32) != 0;

        // The ladder: prove each rung with a test encode, then open the real
        // context; a rung that proves but won't re-open also falls through.
        let mut opened: Option<AVCodecContext> = None;
        let picked = pick_first_working(&encoder_candidates(video.codec), |name| {
            test_encode(name, video).is_ok()
                && match build_video_ctx(name, video, global_header) {
                    Ok(ctx) => {
                        opened = Some(ctx);
                        true
                    }
                    Err(_) => false,
                }
        });
        let (encoder, video_encoder) = match (opened, picked) {
            (Some(ctx), Some(name)) => (ctx, name),
            _ => {
                return Err(MediaError::Ffmpeg(format!(
                    "no working {} encoder in this FFmpeg build",
                    video.codec.label()
                )))
            }
        };

        {
            let mut stream = output.new_stream();
            stream.set_codecpar(encoder.extract_codecpar());
            stream.set_time_base(AVRational {
                num: video.fps_den,
                den: video.fps_num,
            });
        }

        let audio_track = match audio {
            Some(a) => Some(open_audio(&mut output, a, global_header)?),
            None => None,
        };

        // +faststart moves the index to the front of the file when the
        // trailer is written, so exports stream straight away (media-io §7).
        let mut header_opts = dict_set(None, "movflags", "+faststart");
        output.write_header(&mut header_opts)?;

        let width = i32::try_from(video.width)
            .map_err(|_| MediaError::Ffmpeg("frame width overflows".into()))?;
        let height = i32::try_from(video.height)
            .map_err(|_| MediaError::Ffmpeg("frame height overflows".into()))?;
        let encode_pix_fmt = pix_fmt_for(video_encoder);
        let sws = SwsContext::get_context(
            width,
            height,
            ffi::AV_PIX_FMT_RGBA,
            width,
            height,
            encode_pix_fmt,
            ffi::SWS_BILINEAR,
            None,
            None,
            None,
        )
        .ok_or_else(|| MediaError::Ffmpeg("swscale for encode".into()))?;

        Ok(Self {
            output,
            video: encoder,
            video_encoder,
            sws,
            encode_pix_fmt,
            width,
            height,
            next_pts: 0,
            audio: audio_track,
            finished: false,
        })
    }

    /// The FFmpeg name of the encoder actually in use (e.g. "h264_nvenc").
    pub fn encoder_name(&self) -> &'static str {
        self.video_encoder
    }

    /// Calm user-facing name of the encoder in use (e.g. "NVENC").
    pub fn encoder_label(&self) -> &'static str {
        encoder_label(self.video_encoder)
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
        dst.set_format(self.encode_pix_fmt);
        dst.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        self.sws
            .scale_frame(&src, 0, self.height, &mut dst)
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        dst.set_pts(self.next_pts);
        self.next_pts += 1;

        self.video.send_frame(Some(&dst))?;
        drain_packets(&mut self.video, &mut self.output, 0, false)
    }

    /// Queue interleaved stereo f32 samples (L R L R …) for the AAC track.
    /// Whole AAC frames are encoded immediately; a trailing partial frame
    /// waits for more samples (or for [`Self::finish`], which pads it with
    /// silence — at most one AAC frame, ~21 ms, of quiet tail).
    pub fn write_audio(&mut self, interleaved: &[f32]) -> Result<(), MediaError> {
        let Self { output, audio, .. } = self;
        let Some(track) = audio.as_mut() else {
            return Err(MediaError::Ffmpeg(
                "this export was opened without an audio stream".into(),
            ));
        };
        track.pending.extend_from_slice(interleaved);
        pump_audio(track, output, false)
    }

    /// Flush both encoders and write the container trailer. Must be called
    /// exactly once; calling again is a no-op.
    pub fn finish(&mut self) -> Result<(), MediaError> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.video.send_frame(None)?;
        drain_packets(&mut self.video, &mut self.output, 0, true)?;
        let Self { output, audio, .. } = self;
        if let Some(track) = audio.as_mut() {
            pump_audio(track, output, true)?;
            track.ctx.send_frame(None)?;
            drain_packets(&mut track.ctx, output, track.stream_index, true)?;
        }
        self.output.write_trailer()?;
        Ok(())
    }
}

/// Configure and open one video encoder context. Shared by the test encode
/// and the real open so a rung is only picked with the exact options it will
/// run with.
fn build_video_ctx(
    name: &str,
    v: &VideoSettings,
    global_header: bool,
) -> Result<AVCodecContext, MediaError> {
    let cname = CString::new(name).map_err(|_| MediaError::BadPath)?;
    let codec = AVCodec::find_encoder_by_name(&cname)
        .ok_or_else(|| MediaError::Ffmpeg(format!("encoder {name} not in this FFmpeg build")))?;
    let mut ctx = AVCodecContext::new(&codec);
    let width =
        i32::try_from(v.width).map_err(|_| MediaError::Ffmpeg("frame width overflows".into()))?;
    let height =
        i32::try_from(v.height).map_err(|_| MediaError::Ffmpeg("frame height overflows".into()))?;
    ctx.set_width(width);
    ctx.set_height(height);
    ctx.set_time_base(AVRational {
        num: v.fps_den,
        den: v.fps_num,
    });
    ctx.set_framerate(AVRational {
        num: v.fps_num,
        den: v.fps_den,
    });
    ctx.set_pix_fmt(pix_fmt_for(name));
    ctx.set_gop_size(30);
    if let Some(rate) = v.bit_rate {
        ctx.set_bit_rate(rate);
    }
    if global_header {
        // mp4 wants codec parameters up front, not repeated in-band.
        ctx.set_flags(ctx.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    // H.264 high profile (docs/06 §7.5) and the VBR peak. These are plain
    // context fields rsmpeg has no safe setter for yet; written directly
    // rather than through an options dictionary because rsmpeg's
    // `open(Some(dict))` error path double-frees the dictionary when a
    // hardware encoder refuses to open — exactly the case the ladder must
    // survive.
    let profile = (v.codec == VideoCodec::H264).then_some(ffi::AV_PROFILE_H264_HIGH as i32);
    set_rate_and_profile(&mut ctx, profile, v.max_rate);
    ctx.open(None)?;
    Ok(ctx)
}

/// Prove an encoder actually works at the export's exact size and rate by
/// encoding 16 blank frames (docs/impl/media-io.md §7: hardware encoders
/// fail late and weirdly — a build can carry NVENC on a machine with no
/// NVIDIA driver, or the driver can be out of sessions). Errors mean "next
/// rung, please", never a failed export.
fn test_encode(name: &str, v: &VideoSettings) -> Result<(), MediaError> {
    let mut ctx = build_video_ctx(name, v, false)?;
    let mut frame = blank_frame(pix_fmt_for(name), v.width, v.height)?;
    for n in 0..16 {
        frame.set_pts(n);
        ctx.send_frame(Some(&frame))?;
        discard_packets(&mut ctx, false)?;
    }
    ctx.send_frame(None)?;
    discard_packets(&mut ctx, true)
}

/// Pull and drop every ready packet from a test-encode context.
fn discard_packets(ctx: &mut AVCodecContext, at_eof: bool) -> Result<(), MediaError> {
    loop {
        match ctx.receive_packet() {
            Ok(_) => {}
            Err(rsmpeg::error::RsmpegError::EncoderDrainError) if !at_eof => return Ok(()),
            Err(rsmpeg::error::RsmpegError::EncoderDrainError)
            | Err(rsmpeg::error::RsmpegError::EncoderFlushedError) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}

/// Open the AAC encoder and add its stream to the container.
fn open_audio(
    output: &mut AVFormatContextOutput,
    a: &AudioSettings,
    global_header: bool,
) -> Result<AudioTrack, MediaError> {
    let codec = AVCodec::find_encoder(ffi::AV_CODEC_ID_AAC)
        .ok_or_else(|| MediaError::Ffmpeg("no AAC encoder linked".into()))?;
    let mut ctx = AVCodecContext::new(&codec);
    let rate =
        i32::try_from(a.rate).map_err(|_| MediaError::Ffmpeg("audio rate overflows".into()))?;
    ctx.set_sample_rate(rate);
    ctx.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
    ctx.set_sample_fmt(ffi::AV_SAMPLE_FMT_FLTP);
    ctx.set_bit_rate(a.bit_rate);
    ctx.set_time_base(AVRational { num: 1, den: rate });
    if global_header {
        ctx.set_flags(ctx.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    ctx.open(None)?;
    let frame_size = usize::try_from(ctx.frame_size).unwrap_or(0);
    let frame_size = if frame_size == 0 { 1024 } else { frame_size };

    let stream_index = {
        let mut stream = output.new_stream();
        stream.set_codecpar(ctx.extract_codecpar());
        stream.set_time_base(AVRational { num: 1, den: rate });
        usize::try_from(stream.index).unwrap_or(1)
    };

    Ok(AudioTrack {
        ctx,
        stream_index,
        rate: a.rate,
        frame_size,
        pending: Vec::new(),
        next_pts: 0,
    })
}

/// Encode every whole AAC frame waiting in the track's pending buffer; at
/// EOF the final partial frame is padded with silence to a whole frame
/// (safer across encoders than a short last frame).
fn pump_audio(
    track: &mut AudioTrack,
    output: &mut AVFormatContextOutput,
    at_eof: bool,
) -> Result<(), MediaError> {
    let chunk = track.frame_size * 2;
    if at_eof {
        let partial = track.pending.len() % chunk;
        if partial != 0 {
            track
                .pending
                .resize(track.pending.len() + (chunk - partial), 0.0);
        }
    }
    // Take the buffer out for the loop (encode_audio_frame needs the whole
    // track mutably), then hand back whatever a partial tail leaves over.
    let pending = std::mem::take(&mut track.pending);
    let mut consumed = 0;
    while pending.len() - consumed >= chunk {
        encode_audio_frame(track, output, &pending[consumed..consumed + chunk])?;
        consumed += chunk;
    }
    track.pending = pending;
    track.pending.drain(..consumed);
    Ok(())
}

/// Encode exactly one AAC frame's worth of interleaved samples.
fn encode_audio_frame(
    track: &mut AudioTrack,
    output: &mut AVFormatContextOutput,
    interleaved: &[f32],
) -> Result<(), MediaError> {
    let n = track.frame_size;
    let mut frame = AVFrame::new();
    frame.set_format(ffi::AV_SAMPLE_FMT_FLTP);
    frame.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
    frame.set_sample_rate(
        i32::try_from(track.rate).map_err(|_| MediaError::Ffmpeg("audio rate overflows".into()))?,
    );
    frame.set_nb_samples(
        i32::try_from(n).map_err(|_| MediaError::Ffmpeg("audio frame size overflows".into()))?,
    );
    frame
        .alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    {
        // Planar float: plane 0 is all left samples, plane 1 all right.
        let left = plane_f32_mut(frame.data[0], n)?;
        for (dst, src) in left.iter_mut().zip(interleaved.iter().step_by(2)) {
            *dst = *src;
        }
        let right = plane_f32_mut(frame.data[1], n)?;
        for (dst, src) in right.iter_mut().zip(interleaved.iter().skip(1).step_by(2)) {
            *dst = *src;
        }
    }
    frame.set_pts(track.next_pts);
    track.next_pts += i64::try_from(n).unwrap_or(0);
    track.ctx.send_frame(Some(&frame))?;
    drain_packets(&mut track.ctx, output, track.stream_index, false)
}

/// Move every ready packet from `ctx` into the container, interleaved with
/// the other stream and rescaled from the encoder's timebase to the
/// stream's (docs/impl/media-io.md pins the timebase discipline: rescale at
/// the mux boundary, never guess).
fn drain_packets(
    ctx: &mut AVCodecContext,
    output: &mut AVFormatContextOutput,
    stream_index: usize,
    at_eof: bool,
) -> Result<(), MediaError> {
    loop {
        match ctx.receive_packet() {
            Ok(mut packet) => {
                packet.set_stream_index(
                    i32::try_from(stream_index)
                        .map_err(|_| MediaError::Ffmpeg("stream index overflows".into()))?,
                );
                let stream_tb = output
                    .streams()
                    .get(stream_index)
                    .map(|s| s.time_base)
                    .ok_or_else(|| MediaError::Ffmpeg("output stream missing".into()))?;
                packet.rescale_ts(ctx.time_base, stream_tb);
                output.interleaved_write_frame(&mut packet)?;
            }
            Err(rsmpeg::error::RsmpegError::EncoderDrainError) if !at_eof => return Ok(()),
            Err(rsmpeg::error::RsmpegError::EncoderDrainError)
            | Err(rsmpeg::error::RsmpegError::EncoderFlushedError) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}

/// Append `key = value` to an FFmpeg options dictionary (creating it on
/// first use). Keys/values with interior NULs are silently skipped — they
/// cannot come from our static tables.
fn dict_set(dict: Option<AVDictionary>, key: &str, value: &str) -> Option<AVDictionary> {
    let (Ok(k), Ok(v)) = (CString::new(key), CString::new(value)) else {
        return dict;
    };
    Some(match dict {
        Some(d) => d.set(&k, &v, 0),
        None => AVDictionary::new(&k, &v, 0),
    })
}

/// Write the profile and VBR-peak fields rsmpeg's safe setters do not cover:
/// one audited raw-struct touch, before `open`, on a context we exclusively
/// own — the same discipline as the plane helpers below.
fn set_rate_and_profile(ctx: &mut AVCodecContext, profile: Option<i32>, max_rate: Option<i64>) {
    // SAFETY: `as_mut_ptr` yields the context this wrapper exclusively owns
    // (no FFmpeg call is running concurrently), and `profile`,
    // `rc_max_rate`, `rc_buffer_size` are plain integer fields that
    // `avcodec_open2` reads later.
    #[allow(unsafe_code)]
    unsafe {
        let raw = ctx.as_mut_ptr();
        if let Some(p) = profile {
            (*raw).profile = p;
        }
        if let Some(peak) = max_rate {
            (*raw).rc_max_rate = peak;
            // Decoder buffer: two seconds' worth of peak is the customary
            // VBV window; clamped because the field is 32-bit.
            (*raw).rc_buffer_size =
                i32::try_from(peak.saturating_mul(2).min(i64::from(i32::MAX))).unwrap_or(i32::MAX);
        }
    }
}

/// A black frame in the encoder's own pixel format, used by the test encode.
fn blank_frame(pix_fmt: i32, width: u32, height: u32) -> Result<AVFrame, MediaError> {
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
/// and auditable like `copy_rgba_into`.
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
fn plane_f32_mut<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [f32], MediaError> {
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
/// raw-pointer touch of the video path, kept small and auditable.
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

    fn video_settings(codec: VideoCodec, w: u32, h: u32) -> VideoSettings {
        VideoSettings {
            codec,
            width: w,
            height: h,
            fps_num: 60,
            fps_den: 1,
            bit_rate: None,
            max_rate: None,
        }
    }

    /// The ladder order is a spec decision (docs/impl/media-io.md §7), not
    /// an accident of iteration order.
    #[test]
    fn encoder_ladder_order_is_nvenc_amf_qsv_software() {
        assert_eq!(
            encoder_candidates(VideoCodec::H264),
            ["h264_nvenc", "h264_amf", "h264_qsv", "libx264"]
        );
        assert_eq!(
            encoder_candidates(VideoCodec::Hevc),
            ["hevc_nvenc", "hevc_amf", "hevc_qsv", "libx265"]
        );
    }

    /// The core fallback rule: hardware that exists in the build but fails
    /// its test open is skipped, not fatal.
    #[test]
    fn pick_falls_through_hardware_that_fails_to_open() {
        let candidates = encoder_candidates(VideoCodec::H264);
        // Simulate a machine where NVENC and AMF exist but cannot open
        // (wrong GPU vendor) and QSV is absent: software must win.
        let picked = pick_first_working(&candidates, |name| name == "libx264");
        assert_eq!(picked, Some("libx264"));
        // Simulate a working NVIDIA machine: the first rung wins.
        let picked = pick_first_working(&candidates, |name| {
            name.ends_with("_nvenc") || name.starts_with("libx")
        });
        assert_eq!(picked, Some("h264_nvenc"));
    }

    #[test]
    fn pick_is_none_when_every_rung_fails() {
        let candidates = encoder_candidates(VideoCodec::Hevc);
        assert_eq!(pick_first_working(&candidates, |_| false), None);
    }

    /// Each candidate is probed at most once and in ladder order — the probe
    /// is a real (if short) encode, so re-probing would be wasteful.
    #[test]
    fn pick_probes_each_candidate_once_in_order() {
        let candidates = encoder_candidates(VideoCodec::H264);
        let mut probed = Vec::new();
        let picked = pick_first_working(&candidates, |name| {
            probed.push(name);
            name == "h264_qsv"
        });
        assert_eq!(picked, Some("h264_qsv"));
        assert_eq!(probed, vec!["h264_nvenc", "h264_amf", "h264_qsv"]);
    }

    #[test]
    fn qsv_is_fed_nv12_and_everything_else_planar() {
        assert_eq!(pix_fmt_for("h264_qsv"), ffi::AV_PIX_FMT_NV12);
        assert_eq!(pix_fmt_for("hevc_qsv"), ffi::AV_PIX_FMT_NV12);
        assert_eq!(pix_fmt_for("h264_nvenc"), ffi::AV_PIX_FMT_YUV420P);
        assert_eq!(pix_fmt_for("h264_amf"), ffi::AV_PIX_FMT_YUV420P);
        assert_eq!(pix_fmt_for("libx264"), ffi::AV_PIX_FMT_YUV420P);
        assert_eq!(pix_fmt_for("libx265"), ffi::AV_PIX_FMT_YUV420P);
    }

    #[test]
    fn encoder_labels_are_calm_and_vendor_true() {
        assert_eq!(encoder_label("h264_nvenc"), "NVENC");
        assert_eq!(encoder_label("hevc_nvenc"), "NVENC");
        assert_eq!(encoder_label("h264_amf"), "AMD AMF");
        assert_eq!(encoder_label("h264_qsv"), "Intel Quick Sync");
        assert_eq!(encoder_label("libx264"), "software x264");
        assert_eq!(encoder_label("libx265"), "software x265");
        assert_eq!(encoder_label("mystery"), "software");
    }

    /// The real ladder on the machine the tests run on: whatever rung wins
    /// must be a known candidate, and the file it produces must round-trip
    /// through our own probe. On an NVIDIA box this genuinely exercises
    /// NVENC; on CI without a GPU it proves the graceful fall to software.
    #[test]
    fn real_ladder_picks_a_working_encoder_and_its_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ladder.mp4");
        let v = video_settings(VideoCodec::H264, 320, 240);
        let mut enc = Encoder::open(&path, &v, None).unwrap();
        let candidates = encoder_candidates(VideoCodec::H264);
        assert!(
            candidates.contains(&enc.encoder_name()),
            "picked {}",
            enc.encoder_name()
        );
        eprintln!(
            "ladder picked: {} ({})",
            enc.encoder_name(),
            enc.encoder_label()
        );
        let rgba = vec![128u8; 320 * 240 * 4];
        for _ in 0..30 {
            enc.write_rgba(&rgba).unwrap();
        }
        enc.finish().unwrap();
        let probe = crate::probe::probe(&path).unwrap();
        let video = probe.video.unwrap();
        assert_eq!((video.width, video.height), (320, 240));
        assert_eq!(video.codec, "h264");
    }

    /// Self-verifying loop: encode a gradient sweep, then probe and index the
    /// file with our OWN readers — dimensions, rate, and frame count must
    /// round-trip exactly.
    #[test]
    fn encoded_file_round_trips_through_our_own_probe_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp4");
        let (w, h, frames) = (320u32, 240u32, 90usize);

        let mut enc = Encoder::open(&path, &video_settings(VideoCodec::H264, w, h), None).unwrap();
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
        // the frame index's pts-derived estimate is what Lumit trusts.
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

    /// HEVC is a first-class delivery codec now: the ladder must open one
    /// (hardware or x265) and the result must probe as HEVC.
    #[test]
    fn hevc_round_trips_through_our_own_probe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out-hevc.mp4");
        let mut enc =
            Encoder::open(&path, &video_settings(VideoCodec::Hevc, 320, 240), None).unwrap();
        eprintln!("hevc ladder picked: {}", enc.encoder_name());
        let rgba = vec![90u8; 320 * 240 * 4];
        for _ in 0..30 {
            enc.write_rgba(&rgba).unwrap();
        }
        enc.finish().unwrap();
        let probe = crate::probe::probe(&path).unwrap();
        let video = probe.video.unwrap();
        assert_eq!(video.codec, "hevc");
        assert_eq!((video.width, video.height), (320, 240));
    }

    /// Audio joins the container: a 440 Hz sine goes in as f32 samples and
    /// must come back out — probed as 48 kHz stereo AAC, decodable by our
    /// own audio reader, at the amplitude that went in (AAC is lossy, so a
    /// generous tolerance).
    #[test]
    fn audio_round_trips_interleaved_with_video() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out-av.mp4");
        let (w, h, fps, frames) = (320u32, 240u32, 60.0f64, 90usize);
        let rate = 48_000u32;
        let mut enc = Encoder::open(
            &path,
            &video_settings(VideoCodec::H264, w, h),
            Some(&AudioSettings {
                rate,
                bit_rate: 320_000,
            }),
        )
        .unwrap();

        let rgba = vec![64u8; (w * h * 4) as usize];
        let total_samples = ((frames as f64 / fps) * f64::from(rate)).round() as usize;
        let sine: Vec<f32> = (0..total_samples)
            .flat_map(|i| {
                let s = 0.5
                    * (2.0 * std::f64::consts::PI * 440.0 * (i as f64) / f64::from(rate)).sin()
                        as f32;
                [s, s]
            })
            .collect();
        // Interleave like the export loop does: one video frame, then the
        // samples that cover it.
        let mut fed = 0usize;
        for n in 0..frames {
            enc.write_rgba(&rgba).unwrap();
            let upto = (((n + 1) as f64 / fps) * f64::from(rate)).round() as usize;
            let upto = upto.min(total_samples);
            enc.write_audio(&sine[fed * 2..upto * 2]).unwrap();
            fed = upto;
        }
        enc.finish().unwrap();

        let probe = crate::probe::probe(&path).unwrap();
        assert!(probe.video.is_some());
        let audio = probe.audio.expect("exported file must carry audio");
        assert_eq!(audio.sample_rate, 48_000);
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.codec, "aac");
        // ~1.5 s of both streams.
        assert!(
            (probe.duration_seconds - 1.5).abs() < 0.15,
            "duration {}",
            probe.duration_seconds
        );

        // Decode our own file back: the sine must survive with its level.
        let buf = crate::audio::decode_all(&path, rate).unwrap();
        assert!(
            (buf.duration_seconds() - 1.5).abs() < 0.15,
            "audio duration {}",
            buf.duration_seconds()
        );
        let mid = &buf.samples[buf.samples.len() / 4..buf.samples.len() / 2];
        let rms = (mid
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / mid.len() as f64)
            .sqrt();
        // RMS of a 0.5-amplitude sine is 0.5/√2 ≈ 0.354 (AAC lossy: ±10%).
        assert!((rms - 0.3535).abs() < 0.035, "rms {rms}");
    }

    /// Feeding audio to a video-only export is a caller bug and must be a
    /// typed error, never a crash or silent drop.
    #[test]
    fn write_audio_without_an_audio_stream_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out-vo.mp4");
        let mut enc =
            Encoder::open(&path, &video_settings(VideoCodec::H264, 64, 64), None).unwrap();
        assert!(enc.write_audio(&[0.0, 0.0]).is_err());
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
