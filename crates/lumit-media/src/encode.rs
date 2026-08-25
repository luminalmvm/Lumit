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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// What a container is stamped with so the file states its own colour
/// (docs/06 §7.4). Three numbers, all from ISO/IEC 23091-2 (the same registry
/// H.264's VUI, HEVC's VUI and the mp4 `colr`/`nclx` box all draw on): which
/// primaries the code values mix, which transfer function encodes the light,
/// and which matrix takes RGB to the YCbCr the codec actually stores.
///
/// In plain terms: a video file is just numbers, and without this the player
/// has to guess what they mean. Guessing wrong is how a delivered file comes
/// back looking washed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColourTags {
    /// sRGB: Rec.709 primaries, the IEC 61966-2-1 curve. The default, and what
    /// an untagged file is universally taken to be.
    #[default]
    Srgb,
    /// Rec.709 primaries, linear light — no transfer function at all.
    Linear,
    /// Rec.709 throughout: primaries, transfer and matrix.
    Bt709,
    /// Rec.2020 (non-constant luminance) throughout.
    Bt2020,
    /// Display P3: SMPTE ST 432-1 primaries, the sRGB curve, a Rec.709 matrix.
    DisplayP3,
    /// Say nothing. Only reached where the space has no registered code.
    Unspecified,
}

impl ColourTags {
    /// `(colour_primaries, transfer_characteristics, matrix_coefficients)` as
    /// the ISO/IEC 23091-2 code points FFmpeg's `AVCOL_*` enumerations mirror.
    fn av(self) -> (i32, i32, i32) {
        match self {
            ColourTags::Srgb => (
                ffi::AVCOL_PRI_BT709,
                ffi::AVCOL_TRC_IEC61966_2_1,
                ffi::AVCOL_SPC_BT709,
            ),
            ColourTags::Linear => (
                ffi::AVCOL_PRI_BT709,
                ffi::AVCOL_TRC_LINEAR,
                ffi::AVCOL_SPC_BT709,
            ),
            ColourTags::Bt709 => (
                ffi::AVCOL_PRI_BT709,
                ffi::AVCOL_TRC_BT709,
                ffi::AVCOL_SPC_BT709,
            ),
            ColourTags::Bt2020 => (
                ffi::AVCOL_PRI_BT2020,
                ffi::AVCOL_TRC_BT2020_10,
                ffi::AVCOL_SPC_BT2020_NCL,
            ),
            ColourTags::DisplayP3 => (
                ffi::AVCOL_PRI_SMPTE432,
                ffi::AVCOL_TRC_IEC61966_2_1,
                ffi::AVCOL_SPC_BT709,
            ),
            ColourTags::Unspecified => (
                ffi::AVCOL_PRI_UNSPECIFIED,
                ffi::AVCOL_TRC_UNSPECIFIED,
                ffi::AVCOL_SPC_UNSPECIFIED,
            ),
        }
    }
}

/// Stamp one encoder context with a colour statement. Shared by the video and
/// the still-sequence paths so a `.mp4` and a `.png` of the same export cannot
/// disagree about what they contain.
///
/// The range is always **limited** (`AVCOL_RANGE_MPEG`): that is what a
/// YCbCr delivery codec writes, and saying so beats leaving it unset.
fn set_colour_tags(ctx: &mut AVCodecContext, tags: ColourTags) {
    let (pri, trc, spc) = tags.av();
    // SAFETY: `as_mut_ptr` yields the context this wrapper exclusively owns (no
    // FFmpeg call is running concurrently), and all four are plain enum fields
    // that `avcodec_open2` reads later — the same route
    // [`set_rate_and_profile`] takes for the fields rsmpeg has no setter for.
    #[allow(unsafe_code)]
    unsafe {
        let raw = ctx.as_mut_ptr();
        (*raw).color_primaries = pri;
        (*raw).color_trc = trc;
        (*raw).colorspace = spc;
        (*raw).color_range = ffi::AVCOL_RANGE_MPEG;
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
    /// What the container states this stream's colour to be.
    pub colour: ColourTags,
}

/// How many bits each colour channel carries in the written file
/// (docs/06-RENDER-PIPELINE.md §7.4). Only the still formats can carry more
/// than eight today — see the capability table in `lumit_render::export`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, Hash,
)]
pub enum BitDepth {
    #[default]
    Eight,
    Sixteen,
}

impl BitDepth {
    /// Bytes one channel occupies in a packed frame buffer.
    pub fn bytes_per_channel(self) -> usize {
        match self {
            BitDepth::Eight => 1,
            BitDepth::Sixteen => 2,
        }
    }

    /// User-facing name (glossary voice: plain, no marketing).
    pub fn label(self) -> &'static str {
        match self {
            BitDepth::Eight => "8-bit",
            BitDepth::Sixteen => "16-bit",
        }
    }
}

/// The audio codec a container carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AudioCodec {
    /// AAC — what every delivery preset uses (docs/09 §8), and the only thing
    /// an `.mp4`/`.m4a` wants.
    #[default]
    Aac,
    /// Uncompressed signed 16-bit PCM — a `.wav` master, where "lossy" would
    /// be the wrong answer and a bitrate means nothing.
    PcmS16,
    /// Uncompressed signed 24-bit PCM — the deeper `.wav` master. Fed as
    /// 32-bit samples: `pcm_s24le` is the one FFmpeg encoder that takes
    /// `AV_SAMPLE_FMT_S32` and writes the top three bytes of each.
    PcmS24,
}

impl AudioCodec {
    fn codec_id(self) -> ffi::AVCodecID {
        match self {
            AudioCodec::Aac => ffi::AV_CODEC_ID_AAC,
            AudioCodec::PcmS16 => ffi::AV_CODEC_ID_PCM_S16LE,
            AudioCodec::PcmS24 => ffi::AV_CODEC_ID_PCM_S24LE,
        }
    }

    /// The sample format the encoder is fed: AAC takes planar float, 16-bit
    /// PCM takes packed 16-bit, and 24-bit PCM takes packed 32-bit (FFmpeg's
    /// `pcm_s24le` keeps the top three bytes of each sample).
    fn sample_fmt(self) -> i32 {
        match self {
            AudioCodec::Aac => ffi::AV_SAMPLE_FMT_FLTP,
            AudioCodec::PcmS16 => ffi::AV_SAMPLE_FMT_S16,
            AudioCodec::PcmS24 => ffi::AV_SAMPLE_FMT_S32,
        }
    }

    /// Calm user-facing name.
    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::Aac => "AAC",
            AudioCodec::PcmS16 => "PCM 16-bit",
            AudioCodec::PcmS24 => "PCM 24-bit",
        }
    }
}

/// Everything the audio stream needs to open.
#[derive(Debug, Clone)]
pub struct AudioSettings {
    /// Sample rate in Hz (delivery presets use 48 000, docs/06 §7.5).
    pub rate: u32,
    /// Bitrate in bits/second (delivery presets use 320 000). Ignored by
    /// codecs that have no such thing — PCM is what it is.
    pub bit_rate: i64,
    pub codec: AudioCodec,
    /// How many channels the stream carries: 1 (mono) or 2 (stereo). Every
    /// buffer handed to [`Encoder::write_audio`] is interleaved at this
    /// width, so a mono stream is one plain run of samples.
    pub channels: u16,
}

/// Container metadata as an **ordered** key/value set (docs/06 §7.6): title,
/// author, copyright, comment, creation time — the classic set, plus whatever
/// else a later page wants to write.
///
/// In plain terms: these are the fields a player or a file browser shows about
/// a file. Ordered rather than a hash map for two reasons — the dialog page
/// shows the rows in a fixed order, and an export must be deterministic
/// (docs/06 §7.3): a map's iteration order is not, and it would land in the
/// file's bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Metadata(Vec<(String, String)>);

impl Metadata {
    /// FFmpeg's own key for each of the classic fields, in the order the
    /// Metadata page lists them. `artist` is FFmpeg's spelling of "author" —
    /// using our own word here would write a field nothing reads.
    pub const TITLE: &'static str = "title";
    pub const AUTHOR: &'static str = "artist";
    pub const COPYRIGHT: &'static str = "copyright";
    pub const COMMENT: &'static str = "comment";
    pub const CREATION_TIME: &'static str = "creation_time";

    /// The classic set, in page order.
    pub const STANDARD_KEYS: [&'static str; 5] = [
        Self::TITLE,
        Self::AUTHOR,
        Self::COPYRIGHT,
        Self::COMMENT,
        Self::CREATION_TIME,
    ];

    pub fn new() -> Self {
        Self::default()
    }

    /// Set `key`. An existing key keeps its position (so editing a field does
    /// not reshuffle the page); a new one is appended. An empty value removes
    /// the key rather than writing a blank field — an empty title in a file is
    /// worse than no title.
    pub fn set(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            self.remove(key);
            return;
        }
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value.to_owned(),
            None => self.0.push((key.to_owned(), value.to_owned())),
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.0.retain(|(k, _)| k != key);
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
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

/// A still-image sequence format the exporter can write (K-201). PNG and TIFF
/// both carry the full RGBA frame losslessly, which is what a compositor's
/// image export is for — a lossy sequence would be an mp4 with extra steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageFormat {
    Png,
    Tiff,
}

impl ImageFormat {
    /// User-facing name (glossary voice: plain, no marketing).
    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Png => "PNG sequence",
            ImageFormat::Tiff => "TIFF sequence",
        }
    }

    /// The file extension each frame carries.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Tiff => "tiff",
        }
    }

    fn codec_id(self) -> ffi::AVCodecID {
        match self {
            ImageFormat::Png => ffi::AV_CODEC_ID_PNG,
            ImageFormat::Tiff => ffi::AV_CODEC_ID_TIFF,
        }
    }

    /// The pixel format each still codec is fed at a given depth. Both carry
    /// full RGBA either way; the sixteen-bit forms differ in byte order
    /// because the two file formats do — PNG is big-endian by specification,
    /// TIFF (as FFmpeg writes it) little. The pack stage always hands over
    /// little-endian samples and this says which of them needs swapping, so
    /// callers never have to know a format's endianness.
    pub fn pix_fmt(self, depth: BitDepth) -> i32 {
        match (self, depth) {
            (_, BitDepth::Eight) => ffi::AV_PIX_FMT_RGBA,
            (ImageFormat::Png, BitDepth::Sixteen) => ffi::AV_PIX_FMT_RGBA64BE,
            (ImageFormat::Tiff, BitDepth::Sixteen) => ffi::AV_PIX_FMT_RGBA64LE,
        }
    }

    /// Whether [`Self::pix_fmt`] wants the pack stage's little-endian samples
    /// byte-swapped on the way in.
    fn swaps_16(self, depth: BitDepth) -> bool {
        depth == BitDepth::Sixteen && self == ImageFormat::Png
    }
}

/// The numbered-frame pattern for a sequence chosen as `name.ext`:
/// `name.%05d.ext` beside it, which FFmpeg's image2 muxer expands to
/// `name.00001.ext`, `name.00002.ext`, … Five digits covers half an hour at
/// 60 fps and printf simply widens beyond it, so nothing truncates.
pub fn sequence_pattern(path: &Path) -> std::path::PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png");
    path.with_file_name(format!("{stem}.%05d.{ext}"))
}

/// The path of frame `n` (1-based) of the sequence chosen as `path` — what
/// [`sequence_pattern`] makes the muxer write, reproduced so a cancelled
/// export can remove exactly the files it made.
pub fn sequence_frame_path(path: &Path, n: usize) -> std::path::PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png");
    path.with_file_name(format!("{stem}.{n:05}.{ext}"))
}

/// Writes one image file per frame through FFmpeg's image2 muxer — the same
/// packet path the mp4 encoder uses, so there is no second way to be wrong
/// about timestamps or draining. No scaler and no audio: the frames are
/// already RGBA (both image codecs take it directly), and a folder of stills
/// has nowhere for sound to go.
pub struct ImageSequenceEncoder {
    output: AVFormatContextOutput,
    video: AVCodecContext,
    width: i32,
    height: i32,
    pix_fmt: i32,
    /// Bytes one pixel occupies in the buffer `write_rgba` is handed.
    src_bytes_per_px: usize,
    swap16: bool,
    next_pts: i64,
    finished: bool,
}

impl ImageSequenceEncoder {
    /// Open a sequence at `path` (e.g. `shot.png` — the numbered pattern is
    /// derived beside it), sized `width`×`height`, stamped at `fps_num/fps_den`,
    /// carrying `depth` bits per channel and stating `colour`.
    // Eight plain values, each of which the caller genuinely chooses; a struct
    // to carry them would be the same eight names one indirection further away.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        path: &Path,
        format: ImageFormat,
        width: u32,
        height: u32,
        fps_num: i32,
        fps_den: i32,
        depth: BitDepth,
        colour: ColourTags,
    ) -> Result<Self, MediaError> {
        let pattern = sequence_pattern(path);
        let cpath = CString::new(pattern.to_str().ok_or(MediaError::BadPath)?)
            .map_err(|_| MediaError::BadPath)?;
        let mut output = AVFormatContextOutput::create(&cpath)?;

        let codec = AVCodec::find_encoder(format.codec_id()).ok_or_else(|| {
            MediaError::Ffmpeg(format!(
                "no {} encoder in this FFmpeg build",
                format.label()
            ))
        })?;
        let mut ctx = AVCodecContext::new(&codec);
        let w =
            i32::try_from(width).map_err(|_| MediaError::Ffmpeg("frame width overflows".into()))?;
        let h = i32::try_from(height)
            .map_err(|_| MediaError::Ffmpeg("frame height overflows".into()))?;
        ctx.set_width(w);
        ctx.set_height(h);
        ctx.set_time_base(AVRational {
            num: fps_den,
            den: fps_num,
        });
        let pix_fmt = format.pix_fmt(depth);
        ctx.set_pix_fmt(pix_fmt);
        set_colour_tags(&mut ctx, colour);
        ctx.open(None)?;

        {
            let mut stream = output.new_stream();
            stream.set_codecpar(ctx.extract_codecpar());
            stream.set_time_base(AVRational {
                num: fps_den,
                den: fps_num,
            });
        }
        output.write_header(&mut None)?;

        Ok(Self {
            output,
            video: ctx,
            width: w,
            height: h,
            pix_fmt,
            src_bytes_per_px: 4 * depth.bytes_per_channel(),
            swap16: format.swaps_16(depth),
            next_pts: 0,
            finished: false,
        })
    }

    /// Encode one tightly-packed RGBA frame into the next numbered file. At
    /// sixteen bits the samples are little-endian `u16`s, four per pixel —
    /// what the pack stage produces, whatever byte order the file wants.
    pub fn write_rgba(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        let expect = frame_len(self.width, self.height, self.src_bytes_per_px)?;
        if rgba.len() != expect {
            return Err(MediaError::Ffmpeg(format!(
                "frame size {} != expected {expect}",
                rgba.len()
            )));
        }
        let mut frame = AVFrame::new();
        frame.set_width(self.width);
        frame.set_height(self.height);
        frame.set_format(self.pix_fmt);
        frame
            .alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        copy_pixels_into(
            &mut frame,
            rgba,
            self.width,
            self.height,
            self.src_bytes_per_px,
            self.swap16,
        )?;
        frame.set_pts(self.next_pts);
        self.next_pts += 1;

        self.video.send_frame(Some(&frame))?;
        drain_packets(&mut self.video, &mut self.output, 0, false)
    }

    /// Flush the encoder and close the muxer. Idempotent, like the mp4 finish.
    pub fn finish(&mut self) -> Result<(), MediaError> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.video.send_frame(None)?;
        drain_packets(&mut self.video, &mut self.output, 0, true)?;
        self.output.write_trailer()?;
        Ok(())
    }
}

/// The audio half of the muxer: AAC context plus the sample bookkeeping.
struct AudioTrack {
    ctx: AVCodecContext,
    codec: AudioCodec,
    /// Output stream index (audio is 1 behind a video stream, 0 without one).
    stream_index: usize,
    rate: u32,
    /// 1 or 2 — the interleave width of every buffer this track handles.
    channels: usize,
    /// Samples per AAC frame (1024 for the FFmpeg encoder).
    frame_size: usize,
    /// Interleaved stereo samples not yet handed to the encoder.
    pending: Vec<f32>,
    /// Next frame's pts, counted in samples.
    next_pts: i64,
}

/// The video half of the muxer: encoder context plus its frame bookkeeping.
struct VideoTrack {
    ctx: AVCodecContext,
    encoder: &'static str,
    sws: SwsContext,
    encode_pix_fmt: i32,
    width: i32,
    height: i32,
    next_pts: i64,
}

pub struct Encoder {
    output: AVFormatContextOutput,
    video: Option<VideoTrack>,
    audio: Option<AudioTrack>,
    finished: bool,
}

impl Encoder {
    /// Open a container: video per `video`, audio per `audio`, container
    /// metadata per `metadata`. The video encoder is the first ladder rung
    /// that survives a 16-frame test encode (docs/impl/media-io.md §7) —
    /// hardware that fails at runtime falls through to the next rung, never
    /// to an error, and software is always last so opening only fails when
    /// even that is missing from the FFmpeg build.
    ///
    /// **Either stream may be absent, but not both.** `video: None` is the
    /// audio-only export (an `.m4a` or a `.wav` of the comp's mix); `audio:
    /// None` is a silent film. Nothing at all would be an empty file, which
    /// is a caller bug and answers a typed error.
    pub fn open(
        path: &Path,
        video: Option<&VideoSettings>,
        audio: Option<&AudioSettings>,
        metadata: &Metadata,
    ) -> Result<Self, MediaError> {
        if video.is_none() && audio.is_none() {
            return Err(MediaError::Ffmpeg(
                "an export must carry a video or an audio stream".into(),
            ));
        }
        let cpath = CString::new(path.to_str().ok_or(MediaError::BadPath)?)
            .map_err(|_| MediaError::BadPath)?;
        let mut output = AVFormatContextOutput::create(&cpath)?;
        let global_header = (output.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32) != 0;

        let video_track = match video {
            Some(v) => Some(open_video(&mut output, v, global_header)?),
            None => None,
        };
        let audio_track = match audio {
            Some(a) => Some(open_audio(&mut output, a, global_header)?),
            None => None,
        };

        set_container_metadata(&mut output, metadata);

        // +faststart moves the index to the front of the file when the
        // trailer is written, so exports stream straight away (media-io §7).
        // Only the mp4/mov family understands it — which is exactly the family
        // that wants a global header — and a stray option would sit unread in
        // the dictionary rather than being an error, so the guard is honesty
        // rather than necessity.
        let mut header_opts = global_header
            .then(|| dict_set(None, "movflags", "+faststart"))
            .flatten();
        output.write_header(&mut header_opts)?;

        Ok(Self {
            output,
            video: video_track,
            audio: audio_track,
            finished: false,
        })
    }

    /// The FFmpeg name of the video encoder actually in use (e.g.
    /// "h264_nvenc"); the audio codec's own name on an audio-only export.
    pub fn encoder_name(&self) -> &'static str {
        match (&self.video, &self.audio) {
            (Some(v), _) => v.encoder,
            (None, Some(a)) => match a.codec {
                AudioCodec::Aac => "aac",
                AudioCodec::PcmS16 => "pcm_s16le",
                AudioCodec::PcmS24 => "pcm_s24le",
            },
            (None, None) => "",
        }
    }

    /// Calm user-facing name of the encoder in use (e.g. "NVENC").
    pub fn encoder_label(&self) -> &'static str {
        match (&self.video, &self.audio) {
            (Some(v), _) => encoder_label(v.encoder),
            (None, Some(a)) => a.codec.label(),
            (None, None) => "",
        }
    }

    /// Whether this container carries a video stream.
    pub fn has_video(&self) -> bool {
        self.video.is_some()
    }

    /// Encode one tightly-packed RGBA frame (sRGB-encoded display output).
    pub fn write_rgba(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        let Self { output, video, .. } = self;
        let Some(track) = video.as_mut() else {
            return Err(MediaError::Ffmpeg(
                "this export was opened without a video stream".into(),
            ));
        };
        let expect = rgba_frame_len(track.width, track.height)?;
        if rgba.len() != expect {
            return Err(MediaError::Ffmpeg(format!(
                "frame size {} != expected {expect}",
                rgba.len()
            )));
        }
        // RGBA source frame borrowing the caller's bytes via copy (v0).
        let mut src = AVFrame::new();
        src.set_width(track.width);
        src.set_height(track.height);
        src.set_format(ffi::AV_PIX_FMT_RGBA);
        src.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        copy_rgba_into(&mut src, rgba, track.width, track.height)?;

        let mut dst = AVFrame::new();
        dst.set_width(track.width);
        dst.set_height(track.height);
        dst.set_format(track.encode_pix_fmt);
        dst.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        track
            .sws
            .scale_frame(&src, 0, track.height, &mut dst)
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        dst.set_pts(track.next_pts);
        track.next_pts += 1;

        track.ctx.send_frame(Some(&dst))?;
        drain_packets(&mut track.ctx, output, 0, false)
    }

    /// Queue interleaved f32 samples for the audio track — L R L R … at
    /// stereo, one plain run at mono, always at the settings' channel count.
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
        let Self {
            output,
            video,
            audio,
            ..
        } = self;
        if let Some(track) = video.as_mut() {
            track.ctx.send_frame(None)?;
            drain_packets(&mut track.ctx, output, 0, true)?;
        }
        if let Some(track) = audio.as_mut() {
            pump_audio(track, output, true)?;
            track.ctx.send_frame(None)?;
            drain_packets(&mut track.ctx, output, track.stream_index, true)?;
        }
        self.output.write_trailer()?;
        Ok(())
    }
}

/// Run the encoder ladder and add the video stream to the container.
fn open_video(
    output: &mut AVFormatContextOutput,
    video: &VideoSettings,
    global_header: bool,
) -> Result<VideoTrack, MediaError> {
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
    let (ctx, encoder) = match (opened, picked) {
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
        stream.set_codecpar(ctx.extract_codecpar());
        stream.set_time_base(AVRational {
            num: video.fps_den,
            den: video.fps_num,
        });
    }

    let width = i32::try_from(video.width)
        .map_err(|_| MediaError::Ffmpeg("frame width overflows".into()))?;
    let height = i32::try_from(video.height)
        .map_err(|_| MediaError::Ffmpeg("frame height overflows".into()))?;
    let encode_pix_fmt = pix_fmt_for(encoder);
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

    Ok(VideoTrack {
        ctx,
        encoder,
        sws,
        encode_pix_fmt,
        width,
        height,
        next_pts: 0,
    })
}

/// Write the container's metadata dictionary (docs/06 §7.6), in the order the
/// [`Metadata`] set holds — an export is deterministic (docs/06 §7.3) and
/// these bytes land in the file.
fn set_container_metadata(output: &mut AVFormatContextOutput, metadata: &Metadata) {
    if metadata.is_empty() {
        return;
    }
    let mut dict: *mut ffi::AVDictionary = std::ptr::null_mut();
    for (key, value) in metadata.iter() {
        let (Ok(k), Ok(v)) = (CString::new(key), CString::new(value)) else {
            // A key or value with an interior NUL cannot be a C string. Skip
            // it rather than failing the export over one metadata field.
            continue;
        };
        // SAFETY: `dict` starts null and is only ever handed to
        // `av_dict_set`, which allocates on first use and reallocates after;
        // both pointers are valid, NUL-terminated C strings alive for the
        // call, and FFmpeg copies them (flags 0 = copy key and value).
        #[allow(unsafe_code)]
        unsafe {
            ffi::av_dict_set(&mut dict, k.as_ptr(), v.as_ptr(), 0);
        }
    }
    // Ownership passes to the format context, which frees it in
    // `avformat_free_context`.
    //
    // SAFETY: `as_mut_ptr` yields the context this wrapper exclusively owns
    // (no FFmpeg call is running concurrently), `metadata` is a plain pointer
    // field `avformat_write_header` reads later, and it was null — a freshly
    // created output context carries no metadata — so nothing is leaked by
    // overwriting it.
    #[allow(unsafe_code)]
    unsafe {
        (*output.as_mut_ptr()).metadata = dict;
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
    set_colour_tags(&mut ctx, v.colour);
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

/// Open the audio encoder and add its stream to the container.
fn open_audio(
    output: &mut AVFormatContextOutput,
    a: &AudioSettings,
    global_header: bool,
) -> Result<AudioTrack, MediaError> {
    let codec = AVCodec::find_encoder(a.codec.codec_id())
        .ok_or_else(|| MediaError::Ffmpeg(format!("no {} encoder linked", a.codec.label())))?;
    let mut ctx = AVCodecContext::new(&codec);
    let rate =
        i32::try_from(a.rate).map_err(|_| MediaError::Ffmpeg("audio rate overflows".into()))?;
    ctx.set_sample_rate(rate);
    let channels = i32::from(a.channels.clamp(1, 2));
    ctx.set_ch_layout(AVChannelLayout::from_nb_channels(channels).into_inner());
    ctx.set_sample_fmt(a.codec.sample_fmt());
    // PCM has no bitrate to choose — it is exactly rate × channels × depth —
    // and asking for one confuses the encoder rather than obeying.
    if a.codec == AudioCodec::Aac {
        ctx.set_bit_rate(a.bit_rate);
    }
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
        codec: a.codec,
        stream_index,
        rate: a.rate,
        channels: channels as usize,
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
    let chunk = track.frame_size * track.channels;
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
    let chans = track.channels;
    let mut frame = AVFrame::new();
    frame.set_format(track.codec.sample_fmt());
    frame.set_ch_layout(
        AVChannelLayout::from_nb_channels(i32::try_from(chans).unwrap_or(2)).into_inner(),
    );
    frame.set_sample_rate(
        i32::try_from(track.rate).map_err(|_| MediaError::Ffmpeg("audio rate overflows".into()))?,
    );
    frame.set_nb_samples(
        i32::try_from(n).map_err(|_| MediaError::Ffmpeg("audio frame size overflows".into()))?,
    );
    frame
        .alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    match track.codec {
        AudioCodec::Aac => {
            // Planar float: one plane per channel, de-interleaved. Stereo is
            // plane 0 = left and plane 1 = right; mono is plane 0 and nothing
            // else.
            for ch in 0..chans {
                let plane = plane_f32_mut(frame.data[ch], n)?;
                for (dst, src) in plane
                    .iter_mut()
                    .zip(interleaved.iter().skip(ch).step_by(chans))
                {
                    *dst = *src;
                }
            }
        }
        AudioCodec::PcmS16 => {
            // Packed 16-bit: one plane, L R L R …, each sample scaled and
            // clamped rather than wrapped — a signal over full scale must
            // clip, not fold over into the opposite sign.
            let out = plane_i16_mut(frame.data[0], n * chans)?;
            for (dst, src) in out.iter_mut().zip(interleaved.iter()) {
                *dst = f32_to_i16(*src);
            }
        }
        AudioCodec::PcmS24 => {
            // Packed 32-bit, of which `pcm_s24le` writes the top three bytes:
            // the same clamp, scaled to the wider full scale.
            let out = plane_i32_mut(frame.data[0], n * chans)?;
            for (dst, src) in out.iter_mut().zip(interleaved.iter()) {
                *dst = f32_to_i32(*src);
            }
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

/// View an audio plane as `len` i16 samples — the packed-PCM twin of
/// [`plane_f32_mut`], with the same null and alignment discipline.
fn plane_i16_mut<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [i16], MediaError> {
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
fn plane_i32_mut<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [i32], MediaError> {
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
fn f32_to_i32(s: f32) -> i32 {
    ((f64::from(s.clamp(-1.0, 1.0)) * 8_388_607.0).round() as i32) << 8
}

/// One float sample as signed 16-bit PCM: full scale is 32767, and anything
/// past ±1.0 clips rather than wrapping (`as` on an out-of-range float
/// saturates in Rust, but the clamp says so where a reader can see it).
fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Exact byte length of one packed RGBA frame, checked against overflow so a
/// nonsensical width/height (bad caller input, not file input, but the rule
/// is the same: no panics — docs/14-ENGINEERING-RULES.md §4) errors instead
/// of overflowing `i32` arithmetic.
fn rgba_frame_len(width: i32, height: i32) -> Result<usize, MediaError> {
    frame_len(width, height, 4)
}

/// Exact byte length of one packed frame at `bytes_per_px`, with the same
/// overflow discipline [`rgba_frame_len`] has always had.
fn frame_len(width: i32, height: i32, bytes_per_px: usize) -> Result<usize, MediaError> {
    let bytes = i32::try_from(bytes_per_px)
        .map_err(|_| MediaError::Ffmpeg("frame dimensions overflow".into()))?;
    width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(bytes))
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
    copy_pixels_into(frame, rgba, width, height, 4, false)
}

/// As [`copy_rgba_into`], for any packed pixel size, optionally swapping each
/// `u16` sample's two bytes on the way (the big-endian still formats).
fn copy_pixels_into(
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
            colour: ColourTags::Srgb,
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
        let mut enc = Encoder::open(&path, Some(&v), None, &Metadata::new()).unwrap();
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

        let mut enc = Encoder::open(
            &path,
            Some(&video_settings(VideoCodec::H264, w, h)),
            None,
            &Metadata::new(),
        )
        .unwrap();
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
        let mut enc = Encoder::open(
            &path,
            Some(&video_settings(VideoCodec::Hevc, 320, 240)),
            None,
            &Metadata::new(),
        )
        .unwrap();
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

    /// Every sample rate, sample width and channel layout the export dialog
    /// offers, opened against the real encoders and read back with our own
    /// probe. This is the capability table's evidence: the rates and depths
    /// `lumit_render::export` refuses are refused because the encoders here
    /// were asked, not because a list was guessed at.
    #[test]
    fn every_offered_rate_depth_and_layout_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        for (ext, codec, name) in [
            ("m4a", AudioCodec::Aac, "aac"),
            ("wav", AudioCodec::PcmS16, "pcm_s16le"),
            ("wav", AudioCodec::PcmS24, "pcm_s24le"),
        ] {
            for rate in [44_100u32, 48_000, 96_000] {
                for channels in [1u16, 2] {
                    let path = dir
                        .path()
                        .join(format!("sound-{name}-{rate}-{channels}.{ext}"));
                    let mut enc = Encoder::open(
                        &path,
                        None,
                        Some(&AudioSettings {
                            rate,
                            bit_rate: 320_000,
                            codec,
                            channels,
                        }),
                        &Metadata::new(),
                    )
                    .unwrap_or_else(|e| panic!("{name} at {rate} Hz, {channels} ch: {e}"));
                    // Half a second of a steady half-scale tone, interleaved
                    // at this layout's own width.
                    let frames = rate as usize / 2;
                    let samples: Vec<f32> = vec![0.5; frames * usize::from(channels)];
                    enc.write_audio(&samples).unwrap();
                    enc.finish().unwrap();

                    let probe = crate::probe::probe(&path).unwrap();
                    let audio = probe.audio.expect("it is all sound");
                    assert_eq!(
                        (audio.sample_rate as u32, audio.channels as u16),
                        (rate, channels),
                        "{name} at {rate} Hz, {channels} ch came back wrong"
                    );
                    assert_eq!(audio.codec, name);

                    // The uncompressed forms must come back at the level that
                    // went in — which is what checks the 24-bit scaling.
                    if codec != AudioCodec::Aac {
                        let back = crate::audio::decode_all(&path, rate).unwrap();
                        let mid = back.samples.len() / 2;
                        // The reader always hands back stereo, so a mono file
                        // arrives up-mixed — and swresample spreads one
                        // channel across two at -3 dB, preserving its power.
                        // That is the opposite direction from our own
                        // fold-down and is allowed to use the opposite law.
                        let expect = if channels == 1 {
                            0.5 / std::f32::consts::SQRT_2
                        } else {
                            0.5
                        };
                        assert!(
                            (back.samples[mid] - expect).abs() < 1e-3,
                            "{name} at {channels} ch came back at {}",
                            back.samples[mid]
                        );
                    }
                }
            }
        }
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
            Some(&video_settings(VideoCodec::H264, w, h)),
            Some(&AudioSettings {
                rate,
                bit_rate: 320_000,
                codec: AudioCodec::Aac,
                channels: 2,
            }),
            &Metadata::new(),
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

    /// A PNG sequence really writes one numbered file per frame, where the
    /// pattern says, and each is a readable image (the PNG magic is enough to
    /// prove the codec ran — a zero-byte or misnumbered file is the failure
    /// this guards).
    #[test]
    fn a_png_sequence_writes_one_numbered_file_per_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let mut enc = ImageSequenceEncoder::open(
            &path,
            ImageFormat::Png,
            32,
            18,
            30,
            1,
            BitDepth::Eight,
            ColourTags::Srgb,
        )
        .unwrap();
        for shade in [0u8, 128, 255] {
            enc.write_rgba(&vec![shade; 32 * 18 * 4]).unwrap();
        }
        enc.finish().unwrap();

        for n in 1..=3 {
            let frame = sequence_frame_path(&path, n);
            let bytes =
                std::fs::read(&frame).unwrap_or_else(|_| panic!("{} missing", frame.display()));
            assert_eq!(&bytes[1..4], b"PNG", "{} is not a PNG", frame.display());
        }
        assert!(
            !sequence_frame_path(&path, 4).exists(),
            "no fourth frame was asked for"
        );
    }

    /// TIFF takes the same path; one frame proves the codec is in the build.
    #[test]
    fn a_tiff_sequence_writes_readable_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plate.tiff");
        let mut enc = ImageSequenceEncoder::open(
            &path,
            ImageFormat::Tiff,
            16,
            16,
            24,
            1,
            BitDepth::Eight,
            ColourTags::Srgb,
        )
        .unwrap();
        enc.write_rgba(&vec![64u8; 16 * 16 * 4]).unwrap();
        enc.finish().unwrap();
        let bytes = std::fs::read(sequence_frame_path(&path, 1)).unwrap();
        // Little- or big-endian TIFF magic.
        assert!(bytes.starts_with(b"II") || bytes.starts_with(b"MM"));
    }

    /// The pattern and the per-frame path must agree, or a cancelled export
    /// would delete the wrong files (or none).
    #[test]
    fn the_sequence_pattern_and_frame_paths_agree() {
        let p = Path::new("C:/out/shot.png");
        assert!(sequence_pattern(p)
            .to_str()
            .unwrap()
            .ends_with("shot.%05d.png"));
        assert!(sequence_frame_path(p, 7)
            .to_str()
            .unwrap()
            .ends_with("shot.00007.png"));
    }

    /// Feeding audio to a video-only export is a caller bug and must be a
    /// typed error, never a crash or silent drop.
    #[test]
    fn write_audio_without_an_audio_stream_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out-vo.mp4");
        let mut enc = Encoder::open(
            &path,
            Some(&video_settings(VideoCodec::H264, 64, 64)),
            None,
            &Metadata::new(),
        )
        .unwrap();
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

    /// A stereo sine of `seconds`, interleaved — the audio fixture the
    /// audio-only tests write and read back.
    fn sine(rate: u32, seconds: f64) -> Vec<f32> {
        let n = (seconds * f64::from(rate)).round() as usize;
        (0..n)
            .flat_map(|i| {
                let s = (0.5
                    * (2.0 * std::f64::consts::PI * 440.0 * (i as f64) / f64::from(rate)).sin())
                    as f32;
                [s, s]
            })
            .collect()
    }

    /// Audio-only export, AAC in an `.m4a`: no video stream at all — the case
    /// `Encoder::open` could not express before — and it must probe as a
    /// sound file with no picture and decode back at the level that went in.
    #[test]
    fn an_audio_only_m4a_carries_the_mix_and_no_video() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mix.m4a");
        let rate = 48_000u32;
        let mut enc = Encoder::open(
            &path,
            None,
            Some(&AudioSettings {
                rate,
                bit_rate: 320_000,
                codec: AudioCodec::Aac,
                channels: 2,
            }),
            &Metadata::new(),
        )
        .unwrap();
        assert!(!enc.has_video());
        assert_eq!(enc.encoder_label(), "AAC");
        enc.write_audio(&sine(rate, 1.0)).unwrap();
        enc.finish().unwrap();

        let probe = crate::probe::probe(&path).unwrap();
        assert!(probe.video.is_none(), "an audio-only export has no picture");
        let audio = probe.audio.expect("it does have sound");
        assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
        assert_eq!(audio.codec, "aac");

        let buf = crate::audio::decode_all(&path, rate).unwrap();
        let mid = &buf.samples[buf.samples.len() / 4..buf.samples.len() / 2];
        let rms = (mid
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / mid.len() as f64)
            .sqrt();
        assert!((rms - 0.3535).abs() < 0.035, "rms {rms}");
    }

    /// The other audio-only face: a `.wav` of uncompressed PCM. Lossless, so
    /// the level assertion is tight where the AAC one had to be generous.
    #[test]
    fn an_audio_only_wav_is_lossless_pcm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mix.wav");
        let rate = 48_000u32;
        let mut enc = Encoder::open(
            &path,
            None,
            Some(&AudioSettings {
                rate,
                bit_rate: 0,
                codec: AudioCodec::PcmS16,
                channels: 2,
            }),
            &Metadata::new(),
        )
        .unwrap();
        enc.write_audio(&sine(rate, 1.0)).unwrap();
        enc.finish().unwrap();

        let probe = crate::probe::probe(&path).unwrap();
        assert!(probe.video.is_none());
        let audio = probe.audio.expect("a wav is all sound");
        assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
        assert_eq!(audio.codec, "pcm_s16le");

        let buf = crate::audio::decode_all(&path, rate).unwrap();
        let mid = &buf.samples[buf.samples.len() / 4..buf.samples.len() / 2];
        let rms = (mid
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / mid.len() as f64)
            .sqrt();
        // Uncompressed: within one 16-bit step of the exact 0.5/√2.
        assert!((rms - 0.3535).abs() < 1e-3, "rms {rms}");
    }

    /// Opening a container with neither stream is a caller bug and must be a
    /// typed error, not a zero-stream file the muxer chokes on later.
    #[test]
    fn a_container_with_neither_stream_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Encoder::open(
            &dir.path().join("nothing.mp4"),
            None,
            None,
            &Metadata::new()
        )
        .is_err());
    }

    /// Writing pictures to an audio-only export is the mirror of the existing
    /// "audio without an audio stream" guard.
    #[test]
    fn write_rgba_without_a_video_stream_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sound.m4a");
        let mut enc = Encoder::open(
            &path,
            None,
            Some(&AudioSettings {
                rate: 48_000,
                bit_rate: 320_000,
                codec: AudioCodec::Aac,
                channels: 2,
            }),
            &Metadata::new(),
        )
        .unwrap();
        assert!(enc.write_rgba(&[0u8; 16]).is_err());
    }

    /// The metadata set is ordered, replaces in place, and drops a key rather
    /// than writing it blank — the three rules the dialog page leans on.
    #[test]
    fn metadata_is_ordered_and_replaces_in_place() {
        let mut m = Metadata::new();
        m.set(Metadata::TITLE, "Scene 1");
        m.set(Metadata::AUTHOR, "A Person");
        m.set(Metadata::COMMENT, "first pass");
        assert_eq!(
            m.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            [Metadata::TITLE, Metadata::AUTHOR, Metadata::COMMENT]
        );
        // Editing a field keeps its row where it was.
        m.set(Metadata::AUTHOR, "Someone Else");
        assert_eq!(
            m.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            [Metadata::TITLE, Metadata::AUTHOR, Metadata::COMMENT]
        );
        assert_eq!(m.get(Metadata::AUTHOR), Some("Someone Else"));
        // Emptying one removes it.
        m.set(Metadata::AUTHOR, "");
        assert_eq!(m.get(Metadata::AUTHOR), None);
        assert_eq!(m.len(), 2);
    }

    /// Metadata really lands in the container: written on the way out, read
    /// back with FFmpeg's own reader on the way in.
    #[test]
    fn metadata_round_trips_into_the_container() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tagged.mp4");
        let mut meta = Metadata::new();
        meta.set(Metadata::TITLE, "Scene 1");
        meta.set(Metadata::AUTHOR, "A Person");
        meta.set(Metadata::COPYRIGHT, "© 2026");
        meta.set(Metadata::COMMENT, "exported by Lumit");

        let mut enc = Encoder::open(
            &path,
            Some(&video_settings(VideoCodec::H264, 64, 64)),
            None,
            &meta,
        )
        .unwrap();
        for _ in 0..10 {
            enc.write_rgba(&vec![32u8; 64 * 64 * 4]).unwrap();
        }
        enc.finish().unwrap();

        let input = crate::probe::open_input(&path).unwrap();
        let dict = input.metadata().expect("the file carries metadata");
        let got = |key: &str| {
            dict.get(&CString::new(key).unwrap(), None, 0)
                .map(|e| e.value().to_string_lossy().into_owned())
        };
        assert_eq!(got(Metadata::TITLE).as_deref(), Some("Scene 1"));
        assert_eq!(got(Metadata::AUTHOR).as_deref(), Some("A Person"));
        assert_eq!(got(Metadata::COPYRIGHT).as_deref(), Some("© 2026"));
        assert_eq!(got(Metadata::COMMENT).as_deref(), Some("exported by Lumit"));
    }

    /// The still formats' sixteen-bit pixel formats differ in byte order
    /// because the file formats do; the pack stage never has to know.
    #[test]
    fn the_still_formats_pick_their_own_sixteen_bit_order() {
        assert_eq!(
            ImageFormat::Png.pix_fmt(BitDepth::Eight),
            ffi::AV_PIX_FMT_RGBA
        );
        assert_eq!(
            ImageFormat::Tiff.pix_fmt(BitDepth::Eight),
            ffi::AV_PIX_FMT_RGBA
        );
        assert_eq!(
            ImageFormat::Png.pix_fmt(BitDepth::Sixteen),
            ffi::AV_PIX_FMT_RGBA64BE
        );
        assert_eq!(
            ImageFormat::Tiff.pix_fmt(BitDepth::Sixteen),
            ffi::AV_PIX_FMT_RGBA64LE
        );
        assert!(ImageFormat::Png.swaps_16(BitDepth::Sixteen));
        assert!(!ImageFormat::Tiff.swaps_16(BitDepth::Sixteen));
        assert!(!ImageFormat::Png.swaps_16(BitDepth::Eight));
        assert_eq!(BitDepth::Eight.bytes_per_channel(), 1);
        assert_eq!(BitDepth::Sixteen.bytes_per_channel(), 2);
    }

    /// A sixteen-bit still really writes sixteen-bit files, in both formats,
    /// from little-endian `u16` samples — and a frame sized for eight bits is
    /// refused rather than read past its end.
    #[test]
    fn sixteen_bit_stills_write_wide_files() {
        let dir = tempfile::tempdir().unwrap();
        let (w, h) = (16u32, 8u32);
        // Mid grey at full alpha, as little-endian u16 samples.
        let px: Vec<u8> = [0x8000u16, 0x4000, 0x2000, 0xffff]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let frame: Vec<u8> = px.repeat((w * h) as usize);

        for (name, format) in [
            ("wide.png", ImageFormat::Png),
            ("wide.tiff", ImageFormat::Tiff),
        ] {
            let path = dir.path().join(name);
            let mut enc = ImageSequenceEncoder::open(
                &path,
                format,
                w,
                h,
                24,
                1,
                BitDepth::Sixteen,
                ColourTags::Srgb,
            )
            .unwrap();
            enc.write_rgba(&frame).unwrap();
            // An eight-bit-sized buffer is the wrong size now, and says so.
            assert!(enc.write_rgba(&vec![0u8; (w * h * 4) as usize]).is_err());
            enc.finish().unwrap();

            let written = std::fs::read(sequence_frame_path(&path, 1)).unwrap();
            assert!(!written.is_empty(), "{name} has bytes");
            // Probe it back through our own reader: a 16-bit still decodes,
            // and its dimensions survive.
            let probe = crate::probe::probe(&sequence_frame_path(&path, 1)).unwrap();
            let video = probe.video.expect("a still is a one-frame video");
            assert_eq!((video.width, video.height), (w, h), "{name}");
        }
    }
}
