//! File probing: the vital statistics shown in the Project panel and used to
//! configure decoders. Read-only; never decodes a frame.

use crate::sequence::MediaSource;
use crate::MediaError;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::ffi;
use std::ffi::CString;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    /// Container-declared average rate, exact rational.
    pub fps_num: i32,
    pub fps_den: i32,
    pub codec: String,
}

impl VideoInfo {
    pub fn fps(&self) -> f64 {
        if self.fps_den == 0 {
            0.0
        } else {
            f64::from(self.fps_num) / f64::from(self.fps_den)
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioInfo {
    pub sample_rate: i32,
    pub channels: i32,
    pub codec: String,
    /// What the file calls this stream, where it calls it anything: the
    /// track name a recorder was given, such as "Microphone".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The stream's language as the file writes it (`eng`, `deu`), where it
    /// says one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MediaProbe {
    pub duration_seconds: f64,
    pub container: String,
    pub video: Option<VideoInfo>,
    /// The file's first audio stream, which is the one a layer sounds unless
    /// it is told another.
    pub audio: Option<AudioInfo>,
    /// Every audio stream after the first, in the file's own order. Empty for
    /// nearly every file. A stream's place in [`Self::audio_streams`] is what
    /// `crate::audio::SoundSource` names it by.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_audio: Vec<AudioInfo>,
    /// The file's own start timecode, as it writes it (`01:00:00:00`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecode: Option<String>,
}

impl MediaProbe {
    /// Every audio stream the file carries, the first one first.
    pub fn audio_streams(&self) -> impl Iterator<Item = &AudioInfo> {
        self.audio.iter().chain(&self.more_audio)
    }

    /// Whether this file carries a picture at all — a video stream, still image
    /// or otherwise. A music file answers false.
    #[must_use]
    pub fn has_picture(&self) -> bool {
        self.video.is_some()
    }

    /// Whether this file's picture actually **runs**: a video stream lasting
    /// more than a single frame.
    ///
    /// # In plain terms
    ///
    /// "Is this a video, or a still?" A still image probes with a video stream
    /// too — one frame of it — so the question cannot be whether the stream is
    /// there; it is whether the stream lasts. That is the distinction the
    /// Project panel needs to say *still* rather than infer sound from a
    /// picture no pixels wide, and the same one that decides whether dropping
    /// the file into a composition makes a Sequence layer to cut or a plain
    /// Footage layer (`add_footage_layer`).
    ///
    /// Both askers go through here so they cannot disagree — a panel calling a
    /// file a still while the timeline cut it as a clip is exactly the kind of
    /// split two copies of this rule would produce.
    ///
    /// Half a frame's slack, so a one-frame still cannot creep over the line on
    /// a rounded duration. Audio-only answers false. A container that declares
    /// no rate leaves no frame length to measure against, so the test falls
    /// back to "does it last at all" — which still calls a single still image
    /// (duration 0) a still, and still calls a stream that plays for a minute a
    /// video.
    #[must_use]
    pub fn runs_as_video(&self) -> bool {
        let Some(video) = self.video.as_ref() else {
            return false;
        };
        let fps = video.fps();
        let one_frame = if fps > 0.0 { 1.0 / fps } else { 0.0 };
        self.duration_seconds > one_frame * 1.5
    }
}

/// What a file says about how it should be read, for whoever is about to
/// say otherwise. Every answer is the file's own, before any interpretation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileFacts {
    /// Whether the picture carries an alpha channel at all.
    pub has_alpha: bool,
    /// Whether its samples are float, which reads as linear light.
    pub float: bool,
    /// The field order the file declares, `Off` when it says progressive or
    /// nothing.
    pub fields: crate::Fields,
    /// The pixel shape the file declares, as a fraction. 1/1 when it says
    /// square or nothing.
    pub pixel_aspect: (u32, u32),
}

/// The `timecode` tag of a container or one of its streams.
fn timecode_tag(tags: Option<rsmpeg::avutil::AVDictionaryRef>) -> Option<String> {
    let tags = tags?;
    let entry = tags.get(c"timecode", None, 0)?;
    let value = entry.value().to_string_lossy().trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// How a stand-in made smaller than its original starts its comment, which
/// goes on with the original's size: `lumit stand-in 3840x2160`.
pub const STAND_IN_TAG: &str = "lumit stand-in";

/// The size a smaller stand-in says its original is, which it is read at,
/// so a layer of it sits as the original's does.
///
/// The file came from someone else's machine, so the size is only believed
/// when it has the file's own shape and is no more than four times as big
/// each way, which is all a stand-in is ever made smaller by.
pub(crate) fn stand_in_size(
    tags: Option<rsmpeg::avutil::AVDictionaryRef>,
    coded: (u32, u32),
) -> Option<(u32, u32)> {
    let tags = tags?;
    let entry = tags.get(c"comment", None, 0)?;
    let comment = entry.value().to_string_lossy();
    let (w, h) = comment.strip_prefix(STAND_IN_TAG)?.trim().split_once('x')?;
    let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
    let (cw, ch) = coded;
    let within = |own: u32, coded: u32| own > coded && own <= coded * 4 + 8 && own <= 16384;
    let shape = |w: u32, h: u32| f64::from(w) / f64::from(h.max(1));
    let alike = (shape(w, h) / shape(cw, ch) - 1.0).abs() < 0.02;
    (cw > 0 && ch > 0 && within(w, cw) && within(h, ch) && alike).then_some((w, h))
}

/// Read [`FileFacts`] off the container. Opens the file and decodes nothing.
pub fn facts(src: &MediaSource) -> Result<FileFacts, MediaError> {
    // Lumit's own readers hand back a picture with alpha and no more to say.
    if crate::reads_own(src) {
        return Ok(FileFacts {
            has_alpha: true,
            float: crate::exr::is_exr(src.on_disk()),
            pixel_aspect: (1, 1),
            ..FileFacts::default()
        });
    }
    let input = open_input(src)?;
    let mut out = FileFacts {
        pixel_aspect: (1, 1),
        ..FileFacts::default()
    };
    let mut seen = false;
    for stream in input.streams() {
        let par = stream.codecpar();
        let picture = par.codec_type == ffi::AVMEDIA_TYPE_VIDEO
            && stream.disposition & ffi::AV_DISPOSITION_ATTACHED_PIC as i32 == 0;
        if seen || !picture {
            continue;
        }
        seen = true;
        out.has_alpha = pix_fmt_flag(par.format, ffi::AV_PIX_FMT_FLAG_ALPHA);
        out.float = pix_fmt_flag(par.format, ffi::AV_PIX_FMT_FLAG_FLOAT);
        out.fields = match par.field_order {
            ffi::AV_FIELD_TT | ffi::AV_FIELD_TB => crate::Fields::Upper,
            ffi::AV_FIELD_BB | ffi::AV_FIELD_BT => crate::Fields::Lower,
            _ => crate::Fields::Off,
        };
        let shape = [stream.sample_aspect_ratio, par.sample_aspect_ratio]
            .into_iter()
            .find(|r| r.num > 0 && r.den > 0);
        if let Some(r) = shape {
            out.pixel_aspect = (r.num.unsigned_abs(), r.den.unsigned_abs());
        }
    }
    Ok(out)
}

/// Whether libav's description of a pixel format carries `flag`.
pub(crate) fn pix_fmt_flag(format: i32, flag: u32) -> bool {
    // SAFETY: `av_pix_fmt_desc_get` takes a format value and returns either a
    // pointer to a static descriptor libav owns for the lifetime of the
    // process, or null for a value it does not recognise, which the null
    // check handles. Nothing is allocated and nothing borrowed.
    #[allow(unsafe_code)]
    unsafe {
        let desc = ffi::av_pix_fmt_desc_get(format);
        !desc.is_null() && ((*desc).flags & u64::from(flag)) != 0
    }
}

/// Point swscale at the YUV matrix a file states. Left alone it uses BT.601
/// whatever the file says, and HD video is BT.709. A file that states none is
/// read as players read it: BT.709 from 720 lines up, BT.601 below. Only the
/// matrix changes; the ranges swscale worked out stay as they were.
pub(crate) fn use_yuv_matrix(
    sws: &mut rsmpeg::swscale::SwsContext,
    colorspace: ffi::AVColorSpace,
    height: i32,
    yuv_is_source: bool,
) {
    // The enum is an `i32` where C enums are signed and a `u32` where they are
    // not, so the cast is needed on some targets and flagged on the rest.
    #[allow(clippy::unnecessary_cast)]
    let cs = match colorspace {
        ffi::AVCOL_SPC_UNSPECIFIED if height >= 720 => ffi::SWS_CS_ITU709 as i32,
        ffi::AVCOL_SPC_UNSPECIFIED => ffi::SWS_CS_ITU601 as i32,
        stated => stated as i32,
    };
    // SAFETY: `as_mut_ptr` is the context this wrapper owns, already
    // initialised, with no scale running. The get call fills the six locals
    // from it; the coefficient table is a static libswscale owns, and the set
    // call copies what it is handed.
    #[allow(unsafe_code)]
    unsafe {
        let c = sws.as_mut_ptr();
        let (mut inv, mut table) = (std::ptr::null_mut(), std::ptr::null_mut());
        let (mut src_range, mut dst_range) = (0, 0);
        let (mut brightness, mut contrast, mut saturation) = (0, 0, 0);
        if ffi::sws_getColorspaceDetails(
            c,
            &mut inv,
            &mut src_range,
            &mut table,
            &mut dst_range,
            &mut brightness,
            &mut contrast,
            &mut saturation,
        ) < 0
        {
            return;
        }
        let coefficients = ffi::sws_getCoefficients(cs);
        let (inv, table) = if yuv_is_source {
            (coefficients, table.cast_const())
        } else {
            (inv.cast_const(), coefficients)
        };
        ffi::sws_setColorspaceDetails(
            c, inv, src_range, table, dst_range, brightness, contrast, saturation,
        );
    }
}

fn codec_name(id: ffi::AVCodecID) -> String {
    rsmpeg::avcodec::AVCodec::find_decoder(id)
        .map(|c| c.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("codec#{id}"))
}

/// Open whatever `src` names: one media file, or the numbered run of stills one
/// file belongs to (`crate::sequence`). A sequence is handed to FFmpeg's
/// `image2` demuxer as a pattern, so from here down the two are the same thing
/// — a container with a video stream.
pub(crate) fn open_input(src: &MediaSource) -> Result<AVFormatContextInput, MediaError> {
    if let Some((run, fps)) = src.run() {
        return crate::sequence::open_run(&run, fps);
    }
    let cpath = CString::new(src.on_disk().to_str().ok_or(MediaError::BadPath)?)
        .map_err(|_| MediaError::BadPath)?;
    Ok(AVFormatContextInput::open(&cpath)?)
}

pub fn probe(src: impl Into<MediaSource>) -> Result<MediaProbe, MediaError> {
    let src = src.into();
    let mut probe = probe_file(&src)?;
    // The picture's numbers are the source's as it is interpreted, the same
    // frames the index counts and the decoder hands back.
    let runs = probe.runs_as_video();
    if let Some(video) = probe.video.as_mut() {
        let native = (video.fps_num, video.fps_den);
        // One picture has no rate, fields or loop to read, only a shape.
        if runs {
            probe.duration_seconds = src.interpret.duration(probe.duration_seconds, native);
            (video.fps_num, video.fps_den) = src.interpret.fps(native);
        }
        video.width = src.interpret.width(video.width);
    }
    Ok(probe)
}

/// The file's own numbers, before any interpretation.
fn probe_file(src: &MediaSource) -> Result<MediaProbe, MediaError> {
    // ffmpeg opens neither half of an Illustrator document.
    if crate::ai::is_ai(src.on_disk()) {
        return crate::ai::probe(src.on_disk());
    }
    let input = open_input(src)?;

    let duration_seconds = if input.duration > 0 {
        input.duration as f64 / f64::from(ffi::AV_TIME_BASE)
    } else {
        0.0
    };
    let container = input.iformat().name().to_string_lossy().into_owned();

    let mut video = None;
    let mut audio = None;
    let mut more_audio = Vec::new();
    let mut timecode = timecode_tag(input.metadata());
    for stream in input.streams() {
        // A timecode track says it on its own stream, a camera file often on
        // the picture's.
        if timecode.is_none() {
            timecode = timecode_tag(stream.metadata());
        }
        let par = stream.codecpar();
        match par.codec_type {
            t if t == ffi::AVMEDIA_TYPE_VIDEO && video.is_none() => {
                // Album artwork embedded in an audio file (mp3 / flac / m4a)
                // arrives as a video stream carrying the attached-picture
                // disposition — a single still, not footage. Treating it as
                // video sent the preview chasing motion frames that do not
                // exist: the failed decode job failed the whole comp frame,
                // wedging every comp holding the audio layer (tester report).
                // Skip it, and the file probes audio-only — the path that
                // needs no frame index and decodes nothing.
                if stream.disposition & ffi::AV_DISPOSITION_ATTACHED_PIC as i32 != 0 {
                    continue;
                }
                let rate = stream.avg_frame_rate;
                let coded = (
                    u32::try_from(par.width).unwrap_or(0),
                    u32::try_from(par.height).unwrap_or(0),
                );
                let (width, height) = stand_in_size(input.metadata(), coded).unwrap_or(coded);
                video = Some(VideoInfo {
                    width,
                    height,
                    fps_num: rate.num,
                    fps_den: rate.den,
                    codec: codec_name(par.codec_id),
                });
            }
            t if t == ffi::AVMEDIA_TYPE_AUDIO => {
                let tags = stream.metadata();
                let tag = |key: &std::ffi::CStr| {
                    let entry = tags.as_ref()?.get(key, None, 0)?;
                    let value = entry.value().to_string_lossy().trim().to_string();
                    (!value.is_empty()).then_some(value)
                };
                let info = AudioInfo {
                    sample_rate: par.sample_rate,
                    channels: par.ch_layout.nb_channels,
                    codec: codec_name(par.codec_id),
                    // Matroska calls a track's name its title and QuickTime
                    // and MP4 call it its name.
                    title: tag(c"title").or_else(|| tag(c"name")),
                    // `und` is the file saying it does not know.
                    language: tag(c"language").filter(|l| l != "und"),
                };
                if audio.is_none() {
                    audio = Some(info);
                } else if more_audio.len() + 1 < crate::audio::MAX_STREAMS {
                    more_audio.push(info);
                }
            }
            _ => {}
        }
    }

    if video.is_none() && audio.is_none() {
        return Err(MediaError::NoStreams);
    }
    // An EXR read a pass at a time is the size of its pass. ffmpeg says
    // nought for the ones it can't open.
    if let Some(video) = video.as_mut().filter(|_| crate::exr::reads_pass(src)) {
        if let Some((width, height)) = crate::exr::pass_size(src) {
            (video.width, video.height) = (width, height);
        }
    }
    Ok(MediaProbe {
        duration_seconds,
        container,
        video,
        audio,
        more_audio,
        timecode,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::index::tests_support::{audio_with_cover, garbage_file};

    fn probe_of(
        duration_seconds: f64,
        video: Option<VideoInfo>,
        audio: Option<AudioInfo>,
    ) -> MediaProbe {
        MediaProbe {
            duration_seconds,
            container: "test".into(),
            video,
            audio,
            more_audio: Vec::new(),
            timecode: None,
        }
    }

    fn video(fps_num: i32, fps_den: i32, codec: &str) -> VideoInfo {
        VideoInfo {
            width: 1920,
            height: 1080,
            fps_num,
            fps_den,
            codec: codec.into(),
        }
    }

    /// **A still is a still, not a sound with no width**. The Project
    /// panel's second fact line asks the probe result what the file is, and it
    /// has to answer honestly for all three shapes of media — a still image
    /// probes with a video stream of exactly one frame, so the question is
    /// whether the picture *lasts*, never whether the stream is there.
    #[test]
    fn a_still_a_video_and_a_sound_are_told_apart() {
        let still = probe_of(0.0, Some(video(25, 1, "png")), None);
        assert!(still.has_picture(), "a still image carries a picture");
        assert!(!still.runs_as_video(), "one frame is not something to cut");

        let clip = probe_of(12.0, Some(video(30000, 1001, "h264")), Some(stereo()));
        assert!(clip.has_picture());
        assert!(clip.runs_as_video());

        let music = probe_of(180.0, None, Some(stereo()));
        assert!(
            !music.has_picture(),
            "sound is not a picture no pixels wide"
        );
        assert!(!music.runs_as_video());
    }

    fn stereo() -> AudioInfo {
        AudioInfo {
            sample_rate: 48_000,
            channels: 2,
            codec: "aac".into(),
            title: None,
            language: None,
        }
    }

    /// A file's audio streams are listed in the file's own order with what it
    /// calls them, which is the order a `SoundSource` counts them in.
    #[test]
    fn every_audio_stream_is_listed_with_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = crate::index::tests_support::two_audio_streams(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };
        let p = probe(&file).unwrap();
        let named: Vec<_> = p
            .audio_streams()
            .map(|a| (a.title.as_deref(), a.language.as_deref()))
            .collect();
        assert_eq!(
            named,
            [(Some("Game"), None), (Some("Microphone"), Some("eng"))]
        );
    }

    /// Regression (tester report): an audio file with embedded cover art
    /// exposes the artwork as a video stream (attached-picture disposition).
    /// It must probe as **audio-only** — treating the still as footage made
    /// the preview chase motion frames that do not exist, and the failed
    /// decode wedged every comp holding the audio layer.
    #[test]
    fn probe_audio_with_cover_art_is_audio_only() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = audio_with_cover(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };
        let p = probe(&file).unwrap();
        assert!(p.audio.is_some(), "the audio stream must survive");
        assert!(p.video.is_none(), "cover art must not probe as video");
    }

    /// Regression: probing arbitrary non-media bytes must return a typed
    /// error and never panic.
    #[test]
    fn probe_garbage_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = garbage_file(dir.path());
        assert!(probe(&path).is_err());
    }
}
