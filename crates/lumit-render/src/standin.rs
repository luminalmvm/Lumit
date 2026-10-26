//! Stand-ins: a copy of a footage file made to be sent to someone in a
//! shared project who has not got the file.
//!
//! # In plain terms
//!
//! A stand-in is the whole clip at the original's own size, rate and frame
//! count, with its sound, every audio stream of it, squeezed hard. A machine that lacks the original
//! reads the stand-in as if it were the file, so every layer sits where it
//! does for the person who has the original, and only the picture is
//! softer. That is the difference from a proxy, which is smaller in pixels
//! and needs its original beside it.
//!
//! The person making one can make it smaller still ([`Smaller`]). A half or
//! quarter size one carries its original's size, which `lumit-media` reads
//! it at, and one with fewer pictures shows each for two or four frames, so
//! it still has every frame. Either way it keeps the original's shape.
//!
//! The other thing made here is the stand-in an export wants: the same
//! whole-length file, with the frames the export reads kept at a quality
//! fit to deliver from and every other frame black, which costs next to
//! nothing to send. [`ranges`] says which frames those are.
//!
//! Runs on whichever thread asks, never the UI thread: a transcode is as
//! long as the clip.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use lumit_media::audio::SoundReader;
use lumit_media::audio::SoundSource;
use lumit_media::encode::{
    AudioCodec, AudioSettings, ColourTags, Encoder, Metadata, VideoCodec, VideoSettings,
};
use lumit_media::probe::STAND_IN_TAG;

/// How many bits a second a stand-in spends on each pixel of a frame, and
/// the least and most it comes to. Enough to cut by and no more.
const STAND_IN_BITS: f64 = 0.025;
const STAND_IN_RATE: (f64, f64) = (400e3, 8e6);

/// The same for the frames an export reads: about three times what Lumit's
/// own exports spend, so a delivery made from them loses nothing to see.
const PART_BITS: f64 = 0.35;
const PART_RATE: (f64, f64) = (4e6, 200e6);

/// The least a stand-in made smaller spends, whatever it is made of.
const SMALLEST_RATE: f64 = 64e3;

/// How much smaller than its original a stand-in is made: its size divided
/// by `size` each way, and a new picture every `every` frames. Each is 1, 2
/// or 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Smaller {
    pub size: u32,
    pub every: u32,
}

impl Smaller {
    /// The original's size, a picture every frame.
    pub const NOT: Smaller = Smaller { size: 1, every: 1 };

    /// Held to 1, 2 or 4 each, as anything else would be.
    #[must_use]
    pub fn new(size: u32, every: u32) -> Smaller {
        let step = |n: u32| match n {
            0 | 1 => 1,
            2 | 3 => 2,
            _ => 4,
        };
        Smaller {
            size: step(size),
            every: step(every),
        }
    }
}

/// The sound's sample rate, and its bitrate in each kind of file.
const SOUND_RATE: u32 = 48_000;
const STAND_IN_SOUND: i64 = 128_000;
const PART_SOUND: i64 = 320_000;

/// How many frames of a stream's sound past the last picture frame are read
/// at a time.
const TAIL_FRAMES: usize = SOUND_RATE as usize;

/// How many frames either side of one an export reads are kept with it, for
/// whatever reads a neighbour the plan did not name, and how small a gap
/// between two kept runs is closed up rather than cut.
const MARGIN: usize = 8;
const CLOSE_UP: usize = 90;

/// The frames to keep for an export that reads `frames` of a clip `total`
/// long: runs of them, first frame and one past the last, in order, each
/// with [`MARGIN`] round it and none closer together than [`CLOSE_UP`].
#[must_use]
pub fn ranges(frames: &BTreeSet<usize>, total: usize) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &frame in frames {
        if frame >= total {
            continue;
        }
        let (from, to) = (
            frame.saturating_sub(MARGIN),
            (frame + MARGIN + 1).min(total),
        );
        match runs.last_mut() {
            Some(last) if from <= last.1 + CLOSE_UP => last.1 = last.1.max(to),
            _ => runs.push((from, to)),
        }
    }
    runs
}

/// Make a stand-in for `source` at `dest`. With `parts`, the frames in those
/// runs are kept at delivery quality and the rest are black, and `smaller`
/// is not looked at. Without, every frame is kept, small, and `smaller`
/// says how small.
///
/// Every frame is written, in order, so the file has exactly as many as the
/// original, at its size and its rate. `progress` is called after each, and
/// `cancel` checked before each. A cancelled run answers `Ok` with the file
/// part-written, which the caller throws away.
pub fn transcode(
    source: &Path,
    dest: &Path,
    parts: Option<&[(usize, usize)]>,
    smaller: Smaller,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), String> {
    let probe = lumit_media::probe::probe(source).map_err(|e| e.to_string())?;
    let video = probe
        .video
        .as_ref()
        .ok_or_else(|| "a stand-in needs a picture to stand in for".to_string())?;
    let fps = video.fps();
    let index = crate::media_index::load_or_build_index(source).map_err(|e| e.to_string())?;
    let total = index.frame_count();
    if total == 0 {
        return Err("the source decodes no frames".into());
    }
    let mut decoder = lumit_media::decode::VideoDecoder::open(source, index.clone())
        .map_err(|e| e.to_string())?;
    let smaller = if parts.is_some() {
        Smaller::NOT
    } else {
        smaller
    };
    // The size a smaller one is made at, even each way as H.264 wants.
    let across = (smaller.size > 1)
        .then(|| (video.width / smaller.size + 1).max(2) & !1)
        .filter(|w| *w < video.width);
    // The first frame says what size the frames arrive in, which is the size
    // the file is opened at.
    let first = decoder.frame_rgba(0, across).map_err(|e| e.to_string())?;
    let (width, height) = (first.width, first.height);
    let pictures = fps.max(1.0) / f64::from(smaller.every);
    let pixels = f64::from(width) * f64::from(height) * pictures;
    let (bits, (least, most)) = match parts {
        Some(_) => (PART_BITS, PART_RATE),
        None => (STAND_IN_BITS, STAND_IN_RATE),
    };
    // The floor comes down with the size and the pictures kept, or a small
    // one of an ordinary clip would spend what a full one does.
    let fewer = f64::from(smaller.size * smaller.size * smaller.every);
    let least = (least / fewer).max(SMALLEST_RATE.min(least));
    let rate = (pixels * bits).clamp(least, most) as i64;
    let (fps_num, fps_den) = crate::export::fps_rational(fps);
    let settings = VideoSettings {
        codec: VideoCodec::H264,
        width,
        height,
        fps_num,
        fps_den,
        bit_rate: Some(rate),
        max_rate: Some(rate.saturating_mul(2)),
        colour: ColourTags::default(),
        depth: lumit_media::encode::BitDepth::Eight,
    };
    // The sound whole, because whoever reads this file has no other.
    let sound = match &probe.audio {
        Some(_) => lumit_media::audio::decode_all(source, SOUND_RATE).ok(),
        None => None,
    };
    let sound_settings = sound.as_ref().map(|_| AudioSettings {
        rate: SOUND_RATE,
        bit_rate: if parts.is_some() {
            PART_SOUND
        } else {
            STAND_IN_SOUND
        },
        codec: AudioCodec::Aac,
        channels: 2,
    });
    // Every audio stream after the first, each read as the frames go by. A
    // layer set to the microphone of a recording has to find it in the
    // stand-in, in the place it has in the file.
    let streams = match sound {
        Some(_) => probe.audio_streams().count() as u32,
        None => 0,
    };
    let mut more_sound: Vec<(SoundReader, u64)> = (1..streams)
        .map_while(|place| {
            let stream = SoundSource::streams_of(source, &[place]);
            Some((SoundReader::open(&stream, SOUND_RATE).ok()?, 0))
        })
        .collect();
    // A smaller one says what size it stands in for, which is the size it
    // is read at.
    let mut said = Metadata::new();
    if across.is_some() {
        let original = format!("{STAND_IN_TAG} {}x{}", video.width, video.height);
        said.set(Metadata::COMMENT, &original);
    }
    let mut encoder = Encoder::open_with_sound(
        dest,
        Some(&settings),
        sound_settings.as_ref(),
        more_sound.len(),
        &said,
    )
    .map_err(|e| e.to_string())?;

    let kept = |frame: usize| match parts {
        Some(runs) => runs.iter().any(|(from, to)| (*from..*to).contains(&frame)),
        None => true,
    };
    // A frame between two pictures of one with fewer shows the one before.
    let held_over = |frame: usize| !frame.is_multiple_of(smaller.every as usize);
    let black: Vec<u8> = [0, 0, 0, 255].repeat(first.rgba.len() / 4);
    // The last frame that decoded, to write again where one will not.
    let mut held = first.rgba;
    // How much of the sound has gone in, in samples of both channels.
    let mut fed = 0usize;
    for n in 0..total {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        if !kept(n) {
            encoder.write_rgba(&black).map_err(|e| e.to_string())?;
        } else {
            if n > 0 && !held_over(n) {
                // A frame that will not decode in passing is asked for once
                // more from a decoder opened afresh. One that still will not,
                // as the tail of a recording cut short does not, is the frame
                // before it again: the file has to have every frame, and a
                // held one is the least wrong thing to put there.
                let again = |n: usize| {
                    let mut fresh =
                        lumit_media::decode::VideoDecoder::open(source, index.clone()).ok()?;
                    let frame = fresh.frame_rgba(n, across).ok()?;
                    Some((fresh, frame))
                };
                match decoder.frame_rgba(n, across) {
                    Ok(frame) => held = frame.rgba,
                    Err(_) => {
                        if let Some((fresh, frame)) = again(n) {
                            decoder = fresh;
                            held = frame.rgba;
                        }
                    }
                }
            }
            if held.len() != black.len() {
                return Err(format!("stand-in frame {n} changed size"));
            }
            encoder.write_rgba(&held).map_err(|e| e.to_string())?;
        }
        // The sound that plays under this frame goes in beside it, so the
        // two stay together in the file.
        if let Some(sound) = &sound {
            let upto = (((n + 1) as f64 / fps.max(1.0)) * f64::from(SOUND_RATE)).round() as usize;
            let upto = (upto * 2).min(sound.samples.len());
            if upto > fed {
                encoder
                    .write_audio(&sound.samples[fed..upto])
                    .map_err(|e| e.to_string())?;
                fed = upto;
            }
            let upto = (((n + 1) as f64 / fps.max(1.0)) * f64::from(SOUND_RATE)).round() as u64;
            for (place, (reader, at)) in more_sound.iter_mut().enumerate() {
                let frames = usize::try_from(upto.saturating_sub(*at)).unwrap_or(0);
                // A stream that stops decoding is silence from there on, as a
                // stream that ends early is.
                let (read, _) = reader.read(*at, frames, true).unwrap_or_default();
                *at = upto;
                if !read.is_empty() {
                    encoder
                        .write_audio_stream(place + 1, &read)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        progress(n + 1, total);
    }
    if let Some(sound) = sound.as_ref().filter(|sound| sound.samples.len() > fed) {
        encoder
            .write_audio(&sound.samples[fed..])
            .map_err(|e| e.to_string())?;
    }
    // And what each of the other streams has left past the last frame.
    for (place, (reader, at)) in more_sound.iter_mut().enumerate() {
        while let Ok((read, _)) = reader.read(*at, TAIL_FRAMES, true) {
            if read.is_empty() {
                break;
            }
            *at += (read.len() / 2) as u64;
            encoder
                .write_audio_stream(place + 1, &read)
                .map_err(|e| e.to_string())?;
        }
    }
    encoder.finish().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Frames an export reads become runs with room round them, close ones
    /// joined, and none past the end of the clip.
    #[test]
    fn frames_read_become_runs_with_room_round_them() {
        let frames: BTreeSet<usize> = [10, 11, 12, 100, 300, 900, 5000].into_iter().collect();
        assert_eq!(ranges(&frames, 1000), [(2, 109), (292, 309), (892, 909)]);
        assert!(ranges(&BTreeSet::new(), 1000).is_empty());
    }

    /// A stand-in is the original's size and has every one of its frames,
    /// which is what lets a machine read it in the original's place. The
    /// one an export wants keeps the frames asked for and blacks the rest.
    #[test]
    fn a_stand_in_matches_its_original_frame_for_frame() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shot.mp4");
        let settings = VideoSettings {
            codec: VideoCodec::H264,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            bit_rate: None,
            max_rate: None,
            colour: ColourTags::default(),
            depth: lumit_media::encode::BitDepth::Eight,
        };
        let mut enc = Encoder::open(&source, Some(&settings), None, &Metadata::new()).unwrap();
        for n in 0..40u8 {
            let mut rgba = vec![0u8; 320 * 240 * 4];
            for px in rgba.chunks_exact_mut(4) {
                px[0] = 120 + n;
                px[1] = 160;
                px[2] = 200;
                px[3] = 255;
            }
            enc.write_rgba(&rgba).unwrap();
        }
        enc.finish().unwrap();
        let frames = |path: &Path| {
            crate::media_index::load_or_build_index(path)
                .unwrap()
                .frame_count()
        };

        let whole = dir.path().join("standin.mp4");
        transcode(
            &source,
            &whole,
            None,
            Smaller::NOT,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        let video = lumit_media::probe::probe(&whole).unwrap().video.unwrap();
        assert_eq!((video.width, video.height), (320, 240));
        assert_eq!(frames(&whole), frames(&source));

        let parts = dir.path().join("parts.mp4");
        let kept = [(20usize, 30usize)];
        transcode(
            &source,
            &parts,
            Some(&kept),
            Smaller::NOT,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(frames(&parts), frames(&source));
        let index = crate::media_index::load_or_build_index(&parts).unwrap();
        let mut decoder = lumit_media::decode::VideoDecoder::open(&parts, index).unwrap();
        let red = |n: usize, decoder: &mut lumit_media::decode::VideoDecoder| {
            decoder.frame_rgba(n, None).unwrap().rgba[0]
        };
        assert!(
            red(5, &mut decoder) < 30,
            "a frame nobody asked for is black"
        );
        assert!(red(25, &mut decoder) > 100, "a frame asked for is kept");

        // A smaller one is read at the original's size and has every frame,
        // so a layer of it sits where the original's does.
        let small = dir.path().join("standin-2-2.mp4");
        let half = Smaller::new(2, 2);
        transcode(
            &source,
            &small,
            None,
            half,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        let video = lumit_media::probe::probe(&small).unwrap().video.unwrap();
        assert_eq!((video.width, video.height), (320, 240));
        assert_eq!(frames(&small), frames(&source));
        let index = crate::media_index::load_or_build_index(&small).unwrap();
        let mut decoder = lumit_media::decode::VideoDecoder::open(&small, index).unwrap();
        let frame = decoder.frame_rgba(7, None).unwrap();
        assert_eq!((frame.width, frame.height), (320, 240));

        // A size no stand-in could be made from is not believed: the file is
        // read at its own.
        let forged = dir.path().join("forged.mp4");
        let mut said = Metadata::new();
        said.set(Metadata::COMMENT, &format!("{STAND_IN_TAG} 16000x12000"));
        let mut enc = Encoder::open(&forged, Some(&settings), None, &said).unwrap();
        enc.write_rgba(&[0u8; 320 * 240 * 4]).unwrap();
        enc.finish().unwrap();
        let video = lumit_media::probe::probe(&forged).unwrap().video.unwrap();
        assert_eq!((video.width, video.height), (320, 240));
    }

    /// **A stand-in carries every audio stream of the file it stands in
    /// for**, each in its own place, so a layer set to the second stream of a
    /// recording sounds that stream for whoever has only the stand-in.
    #[test]
    fn a_stand_in_keeps_each_audio_stream_in_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let Some(source) = lumit_media::index::tests_support::two_audio_streams(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let made = dir.path().join("stand-in.mp4");
        transcode(
            &source,
            &made,
            None,
            Smaller::NOT,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        let probe = lumit_media::probe::probe(&made).unwrap();
        assert_eq!(probe.audio_streams().count(), 2);

        // The pitch of a stream: how often its wave crosses nought going up.
        let pitch = |place: u32| {
            let stream = SoundSource::streams_of(&made, &[place]);
            let sound = lumit_media::audio::decode_sound(&stream, SOUND_RATE).unwrap();
            // A second from the middle, clear of the encoder's lead-in.
            let left: Vec<f32> = sound.samples[SOUND_RATE as usize * 2..SOUND_RATE as usize * 4]
                .iter()
                .step_by(2)
                .copied()
                .collect();
            left.windows(2)
                .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
                .count() as f64
        };
        assert!((pitch(0) - 220.0).abs() < 4.0, "the first is the low tone");
        assert!(
            (pitch(1) - 1760.0).abs() < 8.0,
            "the second is the high one"
        );
    }
}
