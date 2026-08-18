//! The reference comp's media, made rather than checked in.
//!
//! # In plain terms
//!
//! docs/13 §1 describes a composition built on two 1080p60 H.264 clips, an
//! audio track and a colour grade. Committing forty megabytes of video to a
//! public repository to run a benchmark would be a poor trade, so the harness
//! *generates* the media instead: ffmpeg's own synthetic test pattern, encoded
//! exactly as a camera file would be, plus a `.cube` grade written as text.
//! Same bytes on every machine, from a command line — the same trick the media
//! tests already use for their fixtures (`lumit-media`'s `tests_support`).
//!
//! Generation is **idempotent**: a file that is already there and non-empty is
//! left alone. Point two runs at the same directory and the second pays
//! nothing, which is what makes the smoke test cheap enough for the ordinary
//! suite.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the four generated pieces landed.
#[derive(Debug, Clone)]
pub struct RefMedia {
    /// 1080p60, 20 s, H.264: the plain test pattern.
    pub clip_a: PathBuf,
    /// 1080p60, 20 s, H.264: the same pattern hue-rotated and flipped, so the
    /// two layers are told apart at a glance in a rendered frame.
    pub clip_b: PathBuf,
    /// 20 s of 48 kHz tone with a fade in and out.
    pub audio: PathBuf,
    /// A small 3D LUT: warm, lifted blacks, rolled-off blue.
    pub lut: PathBuf,
}

/// An ffmpeg CLI to generate with (any build encodes these fine), or `None`
/// when the machine has none — the caller's cue to skip politely.
///
/// The candidate list mirrors `lumit-media`'s fixture generator. It is not
/// reused from there because that module lives behind a `test-fixtures`
/// feature, and taking a *normal* dependency on it would switch the feature on
/// for every crate in a whole-workspace build, including the shipped bridge.
#[must_use]
pub fn ffmpeg_bin() -> Option<&'static str> {
    [
        "ffmpeg",
        "/opt/homebrew/opt/ffmpeg@7/bin/ffmpeg",
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
    ]
    .into_iter()
    .find(|candidate| {
        Command::new(candidate)
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

/// The grade the adjustment layer's LUT applies. `LUT_3D_SIZE 2` is the
/// smallest legal cube — eight corners, trilinearly interpolated — which is
/// enough to be a real colour transform (warm, lifted blacks, blue rolled off)
/// without a megabyte of table. Red varies fastest (Adobe order), matching
/// `lumit_core::lut`'s parser.
const REFERENCE_CUBE: &str = "\
# Lumit reference grade (docs/13 §1)
TITLE \"Lumit reference\"
LUT_3D_SIZE 2
DOMAIN_MIN 0 0 0
DOMAIN_MAX 1 1 1
0.02 0.01 0.05
1.00 0.01 0.05
0.02 0.98 0.05
1.00 0.98 0.05
0.02 0.01 0.90
1.00 0.01 0.90
0.02 0.98 0.90
1.00 0.98 0.90
";

/// Generate (or reuse) the reference media in `dir`, creating the directory if
/// it is missing. `Err` names what failed — a missing ffmpeg included, so a
/// caller can skip rather than fail.
pub fn generate(dir: &Path) -> Result<RefMedia, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("lumit-bench media dir: {e}"))?;

    let media = RefMedia {
        clip_a: dir.join("ref_a.mp4"),
        clip_b: dir.join("ref_b.mp4"),
        audio: dir.join("ref_tone.wav"),
        lut: dir.join("ref_grade.cube"),
    };

    if !present(&media.lut) {
        std::fs::write(&media.lut, REFERENCE_CUBE)
            .map_err(|e| format!("writing {}: {e}", media.lut.display()))?;
    }

    // Nothing left to encode: skip the ffmpeg probe entirely, so a warm
    // directory needs no ffmpeg on the machine at all.
    if present(&media.clip_a) && present(&media.clip_b) && present(&media.audio) {
        return Ok(media);
    }
    let bin = ffmpeg_bin().ok_or_else(|| "no ffmpeg on PATH".to_string())?;

    // 1200 frames of moving test pattern, GOP 30 — a keyframe every half
    // second, as a camera or a capture card would write it, so seeking and
    // decoding cost what they cost in real work.
    encode(
        bin,
        &media.clip_a,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=20:size=1920x1080:rate=60",
        ],
        &[],
    )?;
    encode(
        bin,
        &media.clip_b,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=20:size=1920x1080:rate=60",
        ],
        &["-vf", "hue=h=180:s=1.4,vflip"],
    )?;

    if !present(&media.audio) {
        run(
            bin,
            &[
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=20:sample_rate=48000",
                // The envelope: two-second fades either end, so the layer's own
                // volume keyframes have something to shape.
                "-af",
                "afade=t=in:st=0:d=2,afade=t=out:st=18:d=2",
                "-c:a",
                "pcm_s16le",
            ],
            &media.audio,
        )?;
    }

    Ok(media)
}

/// Whether a generated file is already there and worth keeping.
fn present(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
}

/// One H.264 encode, skipped when the file is already there.
fn encode(bin: &str, out: &Path, input: &[&str], filter: &[&str]) -> Result<(), String> {
    if present(out) {
        return Ok(());
    }
    let mut args = vec!["-v", "error", "-y"];
    args.extend_from_slice(input);
    args.extend_from_slice(filter);
    args.extend_from_slice(&[
        "-c:v", "libx264",
        // Fast enough that a cold run costs a couple of seconds per clip, at a
        // bitrate a real 1080p60 capture would carry.
        "-preset", "veryfast", "-crf", "23", "-g", "30", "-pix_fmt", "yuv420p",
    ]);
    run(bin, &args, out)
}

/// Run ffmpeg with `args` writing `out`, turning a non-zero exit into an `Err`.
fn run(bin: &str, args: &[&str], out: &Path) -> Result<(), String> {
    let output = Command::new(bin)
        .args(args)
        .arg(out)
        .output()
        .map_err(|e| format!("running {bin}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{bin} failed writing {}: {}",
            out.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}
