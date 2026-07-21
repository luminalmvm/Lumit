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
