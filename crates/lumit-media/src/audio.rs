//! Audio decoding (docs/09-AUDIO.md v1; docs/impl/media-io.md §6).
//!
//! In plain terms: sound is decoded into plain floating-point samples at one
//! standard rate so the playback engine never thinks about codecs. Phase 0
//! decodes a whole track into memory (stereo f32 at 48 kHz ≈ 23 MB per
//! minute — fine for edit-length media; the streamed/cached path arrives with
//! the RAM-tier work).

use crate::MediaError;
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avutil::{AVChannelLayout, AVSamples};
use rsmpeg::ffi;
use rsmpeg::swresample::SwrContext;
use std::path::Path;

/// Interleaved stereo f32 PCM at `rate` Hz — the hand-off format for cpal.
pub struct AudioBuffer {
    pub rate: u32,
    /// Interleaved L R L R …; length = frames × 2.
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.rate)
    }
}

/// Decode the file's first audio stream entirely, resampled to stereo f32
/// at `target_rate`.
pub fn decode_all(path: &Path, target_rate: u32) -> Result<AudioBuffer, MediaError> {
    let mut input = crate::probe::open_input(path)?;
    let (stream_index, par) = input
        .streams()
        .iter()
        .find(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_AUDIO)
        .map(|s| (s.index, s.codecpar().clone()))
        .ok_or(MediaError::NoStreams)?;

    let codec = rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id)
        .ok_or_else(|| MediaError::Ffmpeg("no audio decoder".into()))?;
    let mut decoder = AVCodecContext::new(&codec);
    decoder.apply_codecpar(&par)?;
    decoder.open(None)?;

    let out_layout = AVChannelLayout::from_nb_channels(2);
    let mut swr = SwrContext::new(
        &out_layout,
        ffi::AV_SAMPLE_FMT_FLT,
        i32::try_from(target_rate).unwrap_or(48_000),
        &decoder.ch_layout,
        decoder.sample_fmt,
        decoder.sample_rate,
    )
    .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    swr.init().map_err(|e| MediaError::Ffmpeg(e.to_string()))?;

    let mut samples: Vec<f32> = Vec::new();
    let push_frame = |swr: &mut SwrContext,
                      frame: Option<&rsmpeg::avutil::AVFrame>,
                      samples: &mut Vec<f32>|
     -> Result<(), MediaError> {
        let in_count = frame.map(|f| f.nb_samples).unwrap_or(0);
        let max_out = swr.get_out_samples(in_count);
        if max_out <= 0 {
            return Ok(());
        }
        let mut out = AVSamples::new(2, max_out, ffi::AV_SAMPLE_FMT_FLT, 0)
            .ok_or_else(|| MediaError::Ffmpeg("sample alloc".into()))?;
        let converted = convert_samples(swr, &mut out, max_out, frame, in_count)?;
        if converted > 0 {
            let floats = usize::try_from(converted).unwrap_or(0) * 2;
            let bytes = plane_slice(out.audio_data[0], floats * 4)?;
            let mut chunk = vec![0f32; floats];
            byte_to_f32(bytes, &mut chunk);
            samples.extend_from_slice(&chunk);
        }
        Ok(())
    };

    loop {
        let packet = input.read_packet()?;
        let eof = packet.is_none();
        if let Some(pkt) = packet {
            if pkt.stream_index != stream_index {
                continue;
            }
            decoder.send_packet(Some(&pkt))?;
        } else {
            decoder.send_packet(None)?;
        }
        loop {
            match decoder.receive_frame() {
                Ok(frame) => push_frame(&mut swr, Some(&frame), &mut samples)?,
                Err(rsmpeg::error::RsmpegError::DecoderDrainError)
                | Err(rsmpeg::error::RsmpegError::DecoderFlushedError) => break,
                Err(e) => return Err(e.into()),
            }
        }
        if eof {
            // Flush the resampler's tail.
            push_frame(&mut swr, None, &mut samples)?;
            break;
        }
    }

    Ok(AudioBuffer {
        rate: target_rate,
        samples,
    })
}

/// The resampler call needs raw pointers on both sides; isolated here so the
/// unsafety is one auditable function (engineering rules §unsafe policy).
///
/// SAFETY: `out.audio_data[0]` was just allocated by `AVSamples::new` for at
/// least `max_out` samples, and `swr_convert`/`swr.convert` only ever writes
/// up to `max_out` samples into it. `frame`'s `extended_data`, when present,
/// is owned by the decoder-produced `AVFrame` for the lifetime of this call
/// and is read-only from swresample's side.
#[allow(unsafe_code)]
fn convert_samples(
    swr: &mut SwrContext,
    out: &mut AVSamples,
    max_out: i32,
    frame: Option<&rsmpeg::avutil::AVFrame>,
    in_count: i32,
) -> Result<i32, MediaError> {
    unsafe {
        swr.convert(
            &mut out.audio_data[0],
            max_out,
            frame
                .map(|f| f.extended_data as *const *const u8)
                .unwrap_or(std::ptr::null()),
            in_count,
        )
    }
    .map_err(|e| MediaError::Ffmpeg(e.to_string()))
}

/// The two raw-pointer touches in audio decode, kept small and auditable.
/// Returns an error rather than dereferencing a null plane pointer — a
/// defensive check against exotic sample formats/channel layouts where the
/// resampler could in principle leave a plane unset.
fn plane_slice<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], MediaError> {
    if ptr.is_null() {
        return Err(MediaError::Ffmpeg(
            "resampler returned a null output buffer".into(),
        ));
    }
    // SAFETY: caller (`push_frame`) only reaches here after `convert_samples`
    // reported `converted > 0` samples written into this same plane by
    // `AVSamples::new`'s allocation, and `len` is derived from that count,
    // so the read stays within the allocation.
    #[allow(unsafe_code)]
    unsafe {
        Ok(std::slice::from_raw_parts(ptr, len))
    }
}

fn byte_to_f32(bytes: &[u8], out: &mut [f32]) {
    for (i, chunk) in bytes.chunks_exact(4).enumerate().take(out.len()) {
        out[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::index::tests_support::ffmpeg_bin;
    use std::process::Command;

    /// 2 s of a 440 Hz sine at exactly 0.5 amplitude, stereo, AAC in mp4.
    fn audio_fixture(dir: &Path) -> Option<std::path::PathBuf> {
        let bin = ffmpeg_bin()?;
        let out = dir.join("tone.m4a");
        let status = Command::new(bin)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.5*sin(440*2*PI*t)|0.5*sin(440*2*PI*t):s=44100:d=2",
                "-c:a",
                "aac",
            ])
            .arg(&out)
            .status()
            .ok()?;
        status.success().then_some(out)
    }

    #[test]
    fn decodes_and_resamples_a_sine_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = audio_fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let buf = decode_all(&file, 48_000).unwrap();
        assert_eq!(buf.rate, 48_000);
        // ~2 s, resampled 44.1 → 48 kHz (AAC padding tolerance).
        assert!(
            (buf.duration_seconds() - 2.0).abs() < 0.15,
            "duration {}",
            buf.duration_seconds()
        );
        // RMS of a 0.5-amplitude sine is 0.5/√2 ≈ 0.354 (AAC is lossy: ±10%).
        let mid = &buf.samples[buf.samples.len() / 4..buf.samples.len() / 2];
        let rms = (mid
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / mid.len() as f64)
            .sqrt();
        assert!((rms - 0.3535).abs() < 0.035, "rms {rms}");
        // Stereo interleave: both channels carry the same mono sine.
        let l = buf.samples[1000];
        let r = buf.samples[1001];
        assert!((l - r).abs() < 1e-3, "L {l} vs R {r}");
    }

    #[test]
    fn decode_all_on_zero_byte_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::index::tests_support::zero_byte_file(dir.path());
        assert!(decode_all(&path, 48_000).is_err());
    }

    #[test]
    fn decode_all_on_garbage_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::index::tests_support::garbage_file(dir.path());
        assert!(decode_all(&path, 48_000).is_err());
    }

    #[test]
    fn decode_all_on_truncated_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = audio_fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let truncated = crate::index::tests_support::truncated_copy(&file, dir.path(), 200);
        // A cut-short m4a should fail cleanly; the important assertion is
        // that decode_all returns rather than panicking either way.
        let _ = decode_all(&truncated, 48_000);
    }

    /// Regression: a video-only file has no audio stream, so `decode_all`
    /// must return `NoStreams` rather than panicking anywhere in the
    /// packet/frame loop.
    #[test]
    fn decode_all_on_video_only_file_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = crate::index::tests_support::fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        assert!(matches!(
            decode_all(&file, 48_000),
            Err(MediaError::NoStreams)
        ));
    }
}
