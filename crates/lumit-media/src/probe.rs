//! File probing: the vital statistics shown in the Project panel and used to
//! configure decoders. Read-only; never decodes a frame.

use crate::MediaError;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::ffi;
use std::ffi::CString;
use std::path::Path;

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
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MediaProbe {
    pub duration_seconds: f64,
    pub container: String,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
}

impl MediaProbe {
    /// Whether this file carries a picture at all — a video stream, still image
    /// or otherwise (K-435). A music file answers false.
    #[must_use]
    pub fn has_picture(&self) -> bool {
        self.video.is_some()
    }

    /// Whether this file's picture actually **runs**: a video stream lasting
    /// more than a single frame (K-246).
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

fn codec_name(id: ffi::AVCodecID) -> String {
    rsmpeg::avcodec::AVCodec::find_decoder(id)
        .map(|c| c.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("codec#{id}"))
}

pub(crate) fn open_input(path: &Path) -> Result<AVFormatContextInput, MediaError> {
    let cpath =
        CString::new(path.to_str().ok_or(MediaError::BadPath)?).map_err(|_| MediaError::BadPath)?;
    Ok(AVFormatContextInput::open(&cpath)?)
}

pub fn probe(path: &Path) -> Result<MediaProbe, MediaError> {
    let input = open_input(path)?;

    let duration_seconds = if input.duration > 0 {
        input.duration as f64 / f64::from(ffi::AV_TIME_BASE)
    } else {
        0.0
    };
    let container = input.iformat().name().to_string_lossy().into_owned();

    let mut video = None;
    let mut audio = None;
    for stream in input.streams() {
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
                video = Some(VideoInfo {
                    width: u32::try_from(par.width).unwrap_or(0),
                    height: u32::try_from(par.height).unwrap_or(0),
                    fps_num: rate.num,
                    fps_den: rate.den,
                    codec: codec_name(par.codec_id),
                });
            }
            t if t == ffi::AVMEDIA_TYPE_AUDIO && audio.is_none() => {
                audio = Some(AudioInfo {
                    sample_rate: par.sample_rate,
                    channels: par.ch_layout.nb_channels,
                    codec: codec_name(par.codec_id),
                });
            }
            _ => {}
        }
    }

    if video.is_none() && audio.is_none() {
        return Err(MediaError::NoStreams);
    }
    Ok(MediaProbe {
        duration_seconds,
        container,
        video,
        audio,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::index::tests_support::{
        audio_with_cover, fixture, garbage_file, truncated_copy, zero_byte_file,
    };

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

    /// **A still is a still, not a sound with no width** (K-451). The Project
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
        }
    }

    /// Half a frame's slack, so a one-frame still cannot creep over the line on
    /// a rounded duration — and the frame after it plainly does.
    #[test]
    fn the_still_line_sits_half_a_frame_past_one_frame() {
        let one_frame = probe_of(1.0 / 25.0, Some(video(25, 1, "png")), None);
        assert!(!one_frame.runs_as_video());
        let two_frames = probe_of(2.0 / 25.0, Some(video(25, 1, "h264")), None);
        assert!(two_frames.runs_as_video());
    }

    /// A container that declares no rate leaves no frame length to measure
    /// against, so the test falls back to "does it last at all": a single
    /// undated still is still a still, and a stream that plays for a minute is
    /// still a video. This is the behaviour `add_footage_layer` shipped with
    /// (K-246) and it must not change with the rule moving here.
    #[test]
    fn a_stream_with_no_declared_rate_falls_back_to_lasting_at_all() {
        assert!(!probe_of(0.0, Some(video(0, 0, "png")), None).runs_as_video());
        assert!(probe_of(60.0, Some(video(0, 0, "vp9")), None).runs_as_video());
    }

    /// The panel's second fact line needs a codec name and the sound's shape;
    /// the probe result already carries both, per stream. This pins them so a
    /// later tidy-up cannot quietly drop what the panel reads.
    #[test]
    fn the_probe_result_names_the_codec_and_the_sounds_shape() {
        let clip = probe_of(12.0, Some(video(30000, 1001, "h264")), Some(stereo()));
        assert_eq!(clip.video.as_ref().unwrap().codec, "h264");
        let audio = clip.audio.as_ref().unwrap();
        assert_eq!(
            (audio.codec.as_str(), audio.channels, audio.sample_rate),
            ("aac", 2, 48_000)
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

    /// Regression: probing a zero-byte file must return a typed error and
    /// never panic (docs/14-ENGINEERING-RULES.md §4).
    #[test]
    fn probe_zero_byte_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = zero_byte_file(dir.path());
        assert!(probe(&path).is_err());
    }

    /// Regression: probing arbitrary non-media bytes must return a typed
    /// error and never panic.
    #[test]
    fn probe_garbage_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = garbage_file(dir.path());
        assert!(probe(&path).is_err());
    }

    /// Regression: probing a file cut off before any usable stream
    /// information (moov written at the end by this muxer) must return a
    /// typed error and never panic.
    #[test]
    fn probe_truncated_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };
        let truncated = truncated_copy(&file, dir.path(), 200);
        assert!(probe(&truncated).is_err());
    }
}
