//! Audio decoding (docs/09-AUDIO.md v1; docs/impl/media-io.md §6).
//!
//! In plain terms: sound is decoded into plain floating-point samples at one
//! standard rate so the playback engine never thinks about codecs. Stereo f32
//! at 48 kHz is about 23 MB a minute, so a long cut cannot hold every file it
//! names: [`AudioReader`] decodes any stretch of a file on its own, and the
//! mixer keeps only the stretches near where it is playing
//! (`lumit_audio::stream`). [`decode_all`] is the same reader run from the top
//! to the end, for the callers that do want the whole of a short file.
//!
//! Runs on whichever thread asks; never the audio callback.

use crate::MediaError;
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::{AVChannelLayout, AVFrame, AVSamples};
use rsmpeg::ffi;
use rsmpeg::swresample::SwrContext;
use std::path::{Path, PathBuf};

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
    let mut reader = AudioReader::open(path, target_rate)?;
    while reader.step()? {}
    Ok(AudioBuffer {
        rate: target_rate,
        samples: std::mem::take(&mut reader.ready),
    })
}

/// How far ahead of where a reader stands a wanted frame may be and still be
/// reached by decoding on and throwing the sound between away, in seconds.
/// Landing costs [`LANDING_SECONDS`] of decode, so anything nearer than that
/// is cheaper walked to.
const WALK_SECONDS: u64 = 4;

/// How much is decoded ahead of a landing and thrown away, in seconds: one for
/// the codec to settle after the jump and one for the resampler's filter.
const LANDING_SECONDS: u64 = 2;

/// One file's sound, decoded a stretch at a time at one rate.
///
/// # In plain terms
///
/// Asked for frames 96,000 to 192,000 of a file, a reader hands back exactly
/// the samples [`decode_all`] would have put there. That is the whole promise,
/// and it is what lets the mixer keep two seconds of a file where it used to
/// keep all of it without the export changing by a sample.
///
/// Keeping it takes care, because a compressed file is not an array:
///
/// - A decoder that has just jumped needs a run-up before its output is right,
///   and a resampler's output depends on where it was started. So a reader
///   **lands** two whole seconds before the stretch asked for and throws the
///   run-up away. Whole seconds, because the resampler started on a whole
///   second of input is in the same phase as one that ran from the top.
/// - A jump is only as good as the file's timestamps. A reader lands only in
///   containers that say exactly where each packet's sound belongs, and it
///   checks each landing against the timestamps it meets. Anything else, and
///   any file that fails the check, is decoded **from the top** instead, the
///   sound before the wanted stretch thrown away. Slower, never wrong.
// ponytail: MP3, Matroska, Ogg and transport streams are read from the top on
// every jump, which costs the decode of everything before it (about a second
// per quarter hour of AAC). A table of packet positions written beside the
// frame index, as `index.rs` writes one for pictures, is the upgrade.
pub struct AudioReader {
    path: PathBuf,
    rate: u32,
    input: AVFormatContextInput,
    decoder: AVCodecContext,
    swr: SwrContext,
    stream_index: i32,
    /// The rate the decoder said it hands out when it was opened, which is
    /// what the resampler was told.
    in_rate: u64,
    /// The stream's time base, as a fraction of a second.
    time_base: (i64, i64),
    /// The stream's own length in output frames, where the container says.
    /// A hint: never used to place a sample, only to avoid landing past the
    /// end.
    hint: Option<u64>,
    /// Whether this file may be landed in at all.
    lands: bool,
    /// Uncompressed or lossless sound, where a landing decodes the very
    /// samples a decode from the top does.
    lossless: bool,
    /// The timestamp of the first frame a decode from the top hands out,
    /// which is sample nought.
    origin: Option<i64>,
    /// Output frames the resampler has made, counted from the top of the file.
    produced: u64,
    /// The first output frame anybody still wants.
    wanted_from: u64,
    /// Decoded and not handed out yet, and the frame its first sample is.
    ready: Vec<f32>,
    next: u64,
    ended: bool,
    /// The file's length in output frames, once a decode has reached the end.
    total: Option<u64>,
    /// Set after a landing: the bookkeeping that checks it.
    landing: Option<Landing>,
    /// A landing did not check out. The next read starts again from the top,
    /// and the file is not landed in again.
    inexact: bool,
    /// The file ended before a landing reached the point it was for, so
    /// nothing checked it. The next read starts again from the top, which
    /// finds the length, and no landing is tried past that afterwards.
    overshot: bool,
}

/// A landing being checked: every frame after a jump has to sit exactly where
/// its timestamp says, each one touching the last, or the jump is not trusted.
struct Landing {
    /// The input sample the fresh resampler is fed from.
    feed_from: u64,
    /// Where the next decoded frame has to start, once one has been seen.
    expect: Option<u64>,
}

impl AudioReader {
    /// Open `path`'s first audio stream for decoding at `target_rate`.
    pub fn open(path: &Path, target_rate: u32) -> Result<Self, MediaError> {
        let mut input = crate::probe::open_input(&crate::MediaSource::file(path))?;
        let (stream_index, par, time_base, lands, lossless, hint) = {
            let stream = input
                .streams()
                .iter()
                .find(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_AUDIO)
                .ok_or(MediaError::NoStreams)?;
            let par = stream.codecpar().clone();
            let time_base = (
                i64::from(stream.time_base.num),
                i64::from(stream.time_base.den),
            );
            let hint = (stream.duration > 0 && time_base.1 > 0).then(|| {
                (i128::from(stream.duration) * i128::from(time_base.0) * i128::from(target_rate)
                    / i128::from(time_base.1)) as u64
            });
            let lands = lands_exactly(&input, stream, &par);
            let lossless = is_lossless(&par);
            (stream.index, par, time_base, lands, lossless, hint)
        };
        // Nothing but the sound is wanted, so the demuxer is told to pass over
        // the rest: in a camera file that is nearly all of the bytes.
        for stream in input.streams_mut() {
            if stream.index != stream_index {
                // The constant is an `i32` where C enums are signed and a
                // `u32` where they are not, so the cast is needed on some
                // targets and flagged on the rest.
                #[allow(clippy::unnecessary_cast)]
                stream.set_discard(ffi::AVDISCARD_ALL as i32);
            }
        }

        let codec = rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id)
            .ok_or_else(|| MediaError::Ffmpeg("no audio decoder".into()))?;
        let mut decoder = AVCodecContext::new(&codec);
        decoder.apply_codecpar(&par)?;
        decoder.open(None)?;
        let swr = resampler(&decoder, target_rate)?;
        let in_rate = u64::try_from(decoder.sample_rate).unwrap_or(0);

        Ok(Self {
            path: path.to_path_buf(),
            rate: target_rate,
            input,
            decoder,
            swr,
            stream_index,
            in_rate,
            time_base,
            hint,
            lands: lands && in_rate > 0 && time_base.0 > 0 && time_base.1 > 0,
            lossless,
            origin: None,
            produced: 0,
            wanted_from: 0,
            ready: Vec::new(),
            next: 0,
            ended: false,
            total: None,
            landing: None,
            inexact: false,
            overshot: false,
        })
    }

    /// Whether a jump in this file lands by its timestamps. `false` is a file
    /// that is decoded from the top to reach anything.
    #[must_use]
    pub fn lands(&self) -> bool {
        self.lands
    }

    /// The frame the next [`Self::read`] continues from when it is asked for
    /// that frame: a reader already standing where it is wanted costs nothing
    /// to move.
    #[must_use]
    pub fn position(&self) -> u64 {
        self.next
    }

    /// How many frames this reader would decode and throw away to reach
    /// `start` by carrying on from where it stands: nought when it stands
    /// there. `None` when carrying on does not get there, or would not give
    /// an `exact` read (see [`Self::read`]).
    #[must_use]
    pub fn reaches(&self, start: u64, exact: bool) -> Option<u64> {
        if self.inexact || self.overshot || (exact && !self.is_exact()) {
            return None;
        }
        start.checked_sub(self.next)
    }

    /// The file's length in output frames, where a decode has reached the end
    /// and so knows.
    #[must_use]
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// Tell a fresh reader the length another reader of the same file found.
    pub fn set_total(&mut self, total: u64) {
        self.total = Some(total);
    }

    /// Up to `frames` frames from frame `start`, interleaved stereo: fewer at
    /// the end of the file and none past it, and whether they are **exactly**
    /// the samples [`decode_all`] holds at the same place.
    ///
    /// They always sit at the same place. Whether they are the same numbers
    /// is the decoder's business: a lossy decoder that fills quiet bands with
    /// noise of its own (AAC does) draws that noise from a generator it has
    /// run since the top of the file, so after a landing the noise is other
    /// noise, a few millionths of full scale apart and no different to the
    /// ear. `exact` asks for the decode from the top however far that is,
    /// which is what an export asks; playback does not, and is told which it
    /// got.
    pub fn read(
        &mut self,
        start: u64,
        frames: usize,
        exact: bool,
    ) -> Result<(Vec<f32>, bool), MediaError> {
        if self.total.is_some_and(|total| start >= total) {
            return Ok((Vec::new(), true));
        }
        // A little further on than where the reader stands, inside what it has
        // already decoded: drop the gap and carry on.
        let held = (self.ready.len() / 2) as u64;
        if start > self.next && start - self.next <= held {
            self.ready.drain(..((start - self.next) * 2) as usize);
            self.next = start;
        }
        if start != self.next || self.inexact || self.overshot || (exact && !self.is_exact()) {
            self.seek(start, exact)?;
        }
        let want = frames.saturating_mul(2);
        loop {
            if self.inexact || self.overshot {
                // The landing did not hold: the stretch is reached from the
                // top instead.
                self.restart(!self.inexact)?;
                self.wanted_from = start;
                self.next = start;
            }
            if self.ready.len() >= want || self.ended {
                break;
            }
            self.step()?;
        }
        let take = want.min(self.ready.len());
        let out: Vec<f32> = self.ready.drain(..take).collect();
        self.next += (take / 2) as u64;
        Ok((out, self.is_exact()))
    }

    /// Whether what this reader is decoding now is what a decode from the top
    /// would: it is one, or the sound is lossless.
    fn is_exact(&self) -> bool {
        self.landing.is_none() || self.lossless
    }

    /// The file's exact length in output frames, decoding to the end to find
    /// it where that has not happened yet.
    pub fn length(&mut self) -> Result<u64, MediaError> {
        if let Some(total) = self.total {
            return Ok(total);
        }
        let rate = u64::from(self.rate);
        // Near the end by the container's own reckoning, a little early so an
        // over-long claim still lands inside the file.
        let near = self.hint.map_or(0, |h| (h / rate).saturating_sub(4));
        if self.lands && near > LANDING_SECONDS {
            // A landing counts the same frames to the end whatever it decodes
            // into them, so the length may always be found from one.
            self.seek(near * rate, false)?;
        }
        loop {
            if self.inexact || self.overshot {
                self.restart(!self.inexact)?;
            }
            while !self.inexact && !self.overshot && self.step()? {
                self.ready.clear();
            }
            if !self.inexact && !self.overshot {
                break;
            }
        }
        self.ready.clear();
        self.next = self.produced;
        self.total
            .ok_or_else(|| MediaError::Ffmpeg("the sound has no end".into()))
    }

    /// Stand so that the next decode hands out frame `to` first. With `exact`
    /// a lossy file is not landed in, only walked through from the top.
    fn seek(&mut self, to: u64, exact: bool) -> Result<(), MediaError> {
        self.ready.clear();
        self.stand(to, exact)?;
        // Whatever standing there decoded on the way is not wanted.
        self.ready.clear();
        self.next = to;
        self.wanted_from = to;
        Ok(())
    }

    /// [`Self::seek`]'s choice between walking on, landing, and starting
    /// again from the top.
    fn stand(&mut self, to: u64, exact: bool) -> Result<(), MediaError> {
        let rate = u64::from(self.rate);
        if self.inexact || self.overshot {
            self.restart(!self.inexact)?;
        }
        let may_land = self.lands && (self.lossless || !exact);
        // Whether decoding on from here reaches `to`: it is ahead, and what
        // is being decoded is good enough for whoever asks.
        let ahead = |r: &Self| to >= r.produced && (r.is_exact() || !exact);
        if ahead(self) && (self.ended || to - self.produced <= WALK_SECONDS * rate || !may_land) {
            return Ok(());
        }
        if may_land && to / rate > LANDING_SECONDS {
            // A landing is measured from the first frame of the file, so a
            // reader that has not decoded one yet decodes one now.
            while self.lands && self.origin.is_none() && self.step()? {}
            // Never past the end: a jump there meets no frame to check itself
            // against, and the answer is the whole file decoded to find out.
            if self.total.is_none() && self.hint.is_none_or(|h| to + 3 * rate >= h) {
                self.length()?;
            }
            if self.total.is_some_and(|total| to >= total) {
                return Ok(());
            }
            if self.lands {
                if !self.land(to / rate) {
                    // The file would not jump. Whatever state that left the
                    // demuxer in, a fresh one from the top is not in it.
                    self.lands = false;
                    self.restart(false)?;
                }
                return Ok(());
            }
        }
        if !ahead(self) {
            self.restart(true)?;
        }
        Ok(())
    }

    /// Jump to [`LANDING_SECONDS`] before second `sec` of the output and start
    /// a fresh resampler one second before it. `false` when the file would
    /// not jump, which leaves the reader to start again from the top.
    fn land(&mut self, sec: u64) -> bool {
        let (Some(origin), (num, den)) = (self.origin, self.time_base) else {
            return false;
        };
        let feed_from = (sec - 1) * self.in_rate;
        let decode_from = (sec - LANDING_SECONDS) * self.in_rate;
        let ticks = i128::from(decode_from) * i128::from(den)
            / (i128::from(self.in_rate) * i128::from(num));
        let Ok(timestamp) = i64::try_from(i128::from(origin) + ticks) else {
            return false;
        };
        if self
            .input
            .seek(
                self.stream_index,
                timestamp,
                ffi::AVSEEK_FLAG_BACKWARD as i32,
            )
            .is_err()
        {
            return false;
        }
        self.decoder.flush_buffers();
        let Ok(swr) = resampler(&self.decoder, self.rate) else {
            return false;
        };
        self.swr = swr;
        self.produced = (sec - 1) * u64::from(self.rate);
        self.ended = false;
        self.landing = Some(Landing {
            feed_from,
            expect: None,
        });
        true
    }

    /// Open the file again and stand at its first sample. `keep` says the
    /// file may still be landed in afterwards; a landing that failed its check
    /// says it may not.
    fn restart(&mut self, keep: bool) -> Result<(), MediaError> {
        let mut fresh = Self::open(&self.path, self.rate)?;
        fresh.lands = self.lands && keep;
        fresh.total = self.total;
        *self = fresh;
        Ok(())
    }

    /// Decode one packet's worth, or drain the decoder and the resampler at
    /// the end of the file. `false` once there is nothing more to decode.
    fn step(&mut self) -> Result<bool, MediaError> {
        if self.ended {
            return Ok(false);
        }
        match self.input.read_packet()? {
            Some(packet) => {
                if packet.stream_index != self.stream_index {
                    return Ok(true);
                }
                self.decoder.send_packet(Some(&packet))?;
                self.drain()?;
            }
            None => {
                self.decoder.send_packet(None)?;
                self.drain()?;
                // The end came before the landing did, so nothing checked it.
                if self
                    .landing
                    .as_ref()
                    .is_some_and(|l| l.expect.is_none_or(|e| e < l.feed_from))
                {
                    self.overshot = true;
                }
                if !self.inexact && !self.overshot {
                    // Flush the resampler's tail.
                    self.push(None, 0)?;
                    self.total = Some(self.produced);
                }
                self.ended = true;
            }
        }
        Ok(!self.ended)
    }

    fn drain(&mut self) -> Result<(), MediaError> {
        loop {
            match self.decoder.receive_frame() {
                Ok(frame) => {
                    if self.inexact {
                        continue;
                    }
                    let Some(skip) = self.placed(&frame) else {
                        self.inexact = true;
                        continue;
                    };
                    self.push(Some(&frame), skip)?;
                }
                Err(rsmpeg::error::RsmpegError::DecoderDrainError)
                | Err(rsmpeg::error::RsmpegError::DecoderFlushedError) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// How many of `frame`'s samples fall before the point the resampler is
    /// fed from, which is none of them in a decode from the top. `None` when a
    /// landing does not check out against this frame.
    fn placed(&mut self, frame: &AVFrame) -> Option<usize> {
        if self.landing.is_none() {
            if self.origin.is_none() {
                self.origin = Some(frame.pts);
                // A file with no timestamps cannot be landed in.
                self.lands &= frame.pts != ffi::AV_NOPTS_VALUE;
            }
            return Some(0);
        }
        let origin = self.origin?;
        let (num, den) = self.time_base;
        if frame.pts == ffi::AV_NOPTS_VALUE || i64::from(frame.sample_rate) != self.in_rate as i64 {
            return None;
        }
        // Where the timestamp puts this frame, in input samples from the top.
        // It has to be a whole sample or the container is rounding.
        let ticks = i128::from(frame.pts) - i128::from(origin);
        let scaled = ticks * i128::from(num) * i128::from(self.in_rate);
        if scaled < 0 || scaled % i128::from(den) != 0 {
            return None;
        }
        let at = u64::try_from(scaled / i128::from(den)).ok()?;
        let samples = u64::try_from(frame.nb_samples).ok()?;
        let landing = self.landing.as_mut()?;
        match landing.expect {
            // The first frame after the jump has to be at or before the point
            // wanted, and every one after it has to touch the one before.
            None if at > landing.feed_from => return None,
            Some(expect) if at != expect => return None,
            _ => {}
        }
        landing.expect = Some(at + samples);
        Some(landing.feed_from.saturating_sub(at).min(samples) as usize)
    }

    /// Resample one decoded frame, less its first `skip` samples, and keep
    /// what anybody still wants of the result. `None` flushes the resampler.
    fn push(&mut self, frame: Option<&AVFrame>, skip: usize) -> Result<(), MediaError> {
        let in_count = frame
            .map(|f| f.nb_samples - i32::try_from(skip).unwrap_or(i32::MAX))
            .unwrap_or(0);
        if skip > 0 && in_count <= 0 {
            return Ok(());
        }
        let max_out = self.swr.get_out_samples(in_count);
        if max_out <= 0 {
            return Ok(());
        }
        let mut out = AVSamples::new(2, max_out, ffi::AV_SAMPLE_FMT_FLT, 0)
            .ok_or_else(|| MediaError::Ffmpeg("sample alloc".into()))?;
        let converted = convert_samples(&mut self.swr, &mut out, max_out, frame, skip, in_count)?;
        if converted <= 0 {
            return Ok(());
        }
        let frames = u64::try_from(converted).unwrap_or(0);
        let first = self.produced;
        self.produced += frames;
        // Frames before the wanted one are the run-up, or the walk to it.
        let drop = self.wanted_from.saturating_sub(first).min(frames) as usize;
        let floats = (frames as usize - drop) * 2;
        if floats == 0 {
            return Ok(());
        }
        let bytes = plane_slice(out.audio_data[0], frames as usize * 8)?;
        let at = self.ready.len();
        self.ready.resize(at + floats, 0.0);
        byte_to_f32(&bytes[drop * 8..], &mut self.ready[at..]);
        Ok(())
    }
}

/// A resampler from what `decoder` hands out to interleaved stereo f32 at
/// `target_rate`, fresh: no history, and in phase with its first sample.
fn resampler(decoder: &AVCodecContext, target_rate: u32) -> Result<SwrContext, MediaError> {
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
    cap_downmix(&mut swr)?;
    swr.init().map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    Ok(swr)
}

/// Uncompressed sound, or FLAC: codecs whose every packet decodes to the same
/// samples wherever the decoder came from.
fn is_lossless(par: &rsmpeg::avcodec::AVCodecParameters) -> bool {
    rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id).is_some_and(|c| {
        let name = c.name().to_string_lossy();
        name.starts_with("pcm_") || name == "flac"
    })
}

/// Whether a jump in this file can be trusted to land where it says.
///
/// Two things have to hold. The container has to place every packet exactly
/// (QuickTime and MP4 keep a table of them, and uncompressed sound in a WAV or
/// AIFF is arithmetic). And the sound has to be one unbroken run, because
/// [`decode_all`] counts samples and a landing reads timestamps: a file with a
/// hole in its sound has the two disagree after the hole. The table is read
/// here to rule that out, which costs no file access since the demuxer already
/// holds it.
fn lands_exactly(
    input: &AVFormatContextInput,
    stream: &rsmpeg::avformat::AVStreamRef<'_>,
    par: &rsmpeg::avcodec::AVCodecParameters,
) -> bool {
    let container = input.iformat().name().to_string_lossy().into_owned();
    let is = |name: &str| container.split(',').any(|n| n == name);
    let pcm = rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id)
        .is_some_and(|c| c.name().to_string_lossy().starts_with("pcm_"));
    if is("wav") || is("aiff") || is("w64") {
        return pcm;
    }
    if is("flac") {
        return true;
    }
    if !is("mov") {
        return false;
    }
    let (num, den) = (
        i128::from(stream.time_base.num),
        i128::from(stream.time_base.den),
    );
    let rate = i128::from(par.sample_rate);
    let entries = index_entries(stream);
    if entries.is_empty() || num <= 0 || den <= 0 || rate <= 0 {
        return false;
    }
    // Bytes to one frame of uncompressed sound. QuickTime leaves the block
    // size unset, so it is worked out from the sample's own width.
    let pcm_frame = if par.block_align > 0 {
        i128::from(par.block_align)
    } else {
        i128::from(bits_per_sample(par.codec_id) / 8) * i128::from(par.ch_layout.nb_channels)
    };
    entries.windows(2).all(|pair| {
        let (at, size) = pair[0];
        // How many samples this packet holds: a fixed frame for a codec that
        // has one, and bytes over bytes-per-frame for uncompressed sound.
        let samples = if par.frame_size > 0 {
            i128::from(par.frame_size)
        } else if pcm && pcm_frame > 0 {
            i128::from(size) / pcm_frame
        } else {
            return false;
        };
        (i128::from(pair[1].0) - i128::from(at)) * num * rate == samples * den
    })
}

/// How many bits one sample of this codec takes, or nought for a codec with
/// no fixed width.
///
/// SAFETY: a plain lookup by codec id with no pointers either way.
#[allow(unsafe_code)]
fn bits_per_sample(codec: ffi::AVCodecID) -> i32 {
    unsafe { ffi::av_get_bits_per_sample(codec) }
}

/// The demuxer's table of this stream's packets: each one's timestamp and its
/// size in bytes. Empty for a container that keeps none.
///
/// SAFETY: `stream` is a live stream of an open input for the whole call, the
/// count comes from the same stream, and each entry pointer is read at once
/// and not kept: FFmpeg only promises it until the next call that takes the
/// stream. A null entry is skipped.
#[allow(unsafe_code)]
fn index_entries(stream: &rsmpeg::avformat::AVStreamRef<'_>) -> Vec<(i64, i32)> {
    let raw = stream.as_ptr();
    let count = unsafe { ffi::avformat_index_get_entries_count(raw) };
    (0..count.max(0))
        .filter_map(|i| {
            let entry = unsafe { ffi::avformat_index_get_entry(raw.cast_mut(), i) };
            if entry.is_null() {
                return None;
            }
            Some(unsafe { ((*entry).timestamp, (*entry).size()) })
        })
        .collect()
}

/// The resampler call needs raw pointers on both sides; isolated here so the
/// unsafety is one auditable function (engineering rules §unsafe policy).
///
/// SAFETY: `out.audio_data[0]` was just allocated by `AVSamples::new` for at
/// least `max_out` samples, and `swr_convert`/`swr.convert` only ever writes
/// up to `max_out` samples into it. `frame`'s `extended_data`, when present,
/// is owned by the decoder-produced `AVFrame` for the lifetime of this call
/// and is read-only from swresample's side. With a `skip`, each plane pointer
/// is moved on by that many samples: the caller passes `skip < nb_samples`
/// and `in_count = nb_samples - skip`, so the read stays inside the plane, and
/// a frame has one plane per channel when its format is planar and one
/// otherwise, which is how many pointers are read here.
#[allow(unsafe_code)]
fn convert_samples(
    swr: &mut SwrContext,
    out: &mut AVSamples,
    max_out: i32,
    frame: Option<&AVFrame>,
    skip: usize,
    in_count: i32,
) -> Result<i32, MediaError> {
    let moved: Vec<*const u8> = match frame {
        Some(f) if skip > 0 => {
            let planar = unsafe { ffi::av_sample_fmt_is_planar(f.format) } != 0;
            let bytes =
                usize::try_from(unsafe { ffi::av_get_bytes_per_sample(f.format) }).unwrap_or(0);
            let channels = usize::try_from(f.ch_layout.nb_channels).unwrap_or(0);
            let (planes, stride) = if planar {
                (channels, bytes)
            } else {
                (1, bytes * channels)
            };
            if f.extended_data.is_null() || stride == 0 {
                return Err(MediaError::Ffmpeg("a decoded frame with no samples".into()));
            }
            (0..planes)
                .map(|p| unsafe { (*f.extended_data.add(p)).add(skip * stride) as *const u8 })
                .collect()
        }
        _ => Vec::new(),
    };
    let input = match frame {
        Some(_) if !moved.is_empty() => moved.as_ptr(),
        Some(f) => f.extended_data as *const *const u8,
        None => std::ptr::null(),
    };
    unsafe { swr.convert(&mut out.audio_data[0], max_out, input, in_count) }
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
    // SAFETY: caller (`push`) only reaches here after `convert_samples`
    // reported `converted > 0` samples written into this same plane by
    // `AVSamples::new`'s allocation, and `len` is derived from that count,
    // so the read stays within the allocation.
    #[allow(unsafe_code)]
    unsafe {
        Ok(std::slice::from_raw_parts(ptr, len))
    }
}

/// Hold a fold-down to stereo at full scale.
///
/// **In plain terms.** A 5.1 or 7.1 recording has a centre and surrounds, and
/// folding it to two speakers adds those onto the left and right. Left alone
/// the sum runs well past full scale, by 7.7 dB for 5.1 and 9.9 dB for 7.1
/// with every channel busy, and the master then clips it: the clip plays loud
/// and distorted. FFmpeg scales the fold-down back by itself for whole-number
/// samples and not for the floats Lumit asks for, so it is asked to here.
/// A stereo source has nothing folded into it and is not changed.
///
/// Wrapped so the unsafety is one auditable function, as the two beside it
/// are.
#[allow(unsafe_code)]
fn cap_downmix(swr: &mut SwrContext) -> Result<(), MediaError> {
    // SAFETY: `swr` is a live `SwrContext`, whose first field is its
    // `AVClass`, which is all `av_opt_set_double` asks of the pointer. The
    // name is a NUL-terminated literal that outlives the call.
    unsafe {
        rsmpeg::avutil::opt_set_double(swr.as_mut_ptr().cast(), c"rematrix_maxval", 1.0, 0)
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))
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

    /// A 5.1 recording with every channel at half scale folds down to stereo
    /// without passing full scale. It used to peak at 1.2, which the master
    /// clipped.
    #[test]
    fn a_surround_source_folds_down_inside_full_scale() {
        let dir = tempfile::tempdir().unwrap();
        let Some(bin) = ffmpeg_bin() else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let file = dir.path().join("surround.wav");
        let tone = "0.5*sin(440*2*PI*t)";
        let made = Command::new(bin)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!(
                "aevalsrc={}:c=5.1:s=48000:d=1",
                [tone; 6].join("|")
            ))
            .arg(&file)
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            eprintln!("skipping: this ffmpeg cannot write the fixture");
            return;
        }
        let buf = decode_all(&file, 48_000).unwrap();
        let peak = buf.samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(peak < 0.6, "peak {peak}");
        assert!(peak > 0.3, "the fold-down still carries the sound: {peak}");
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

    /// Twelve seconds of a sweep, different on each side so a slipped sample
    /// or a swapped channel cannot hide in a steady tone. `args` picks the
    /// codec and `name` the container.
    fn sweep(dir: &Path, name: &str, rate: u32, args: &[&str]) -> Option<PathBuf> {
        let bin = ffmpeg_bin()?;
        let out = dir.join(name);
        let source =
            format!("aevalsrc=0.4*sin(2*PI*(200+30*t)*t)|0.3*sin(2*PI*(900-20*t)*t):s={rate}:d=12");
        let status = Command::new(bin)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &source])
            .args(args)
            .arg(&out)
            .status()
            .ok()?;
        status.success().then_some(out)
    }

    /// **A stretch decoded on its own is the stretch the whole-file decode
    /// holds**, whatever order the stretches are asked for in. This is what
    /// lets the mixer keep blocks of a file and the export stay the export it
    /// was.
    ///
    /// Four files, for the roads through the reader. Uncompressed sound and
    /// AAC written without noise substitution land and are the same samples,
    /// through a resampler started in phase in the second case. AAC as ffmpeg
    /// writes it by default lands in the same place with the decoder's own
    /// noise differing, and is the same samples when asked for exactly. A
    /// container that is never landed in is decoded from the top.
    #[test]
    fn a_stretch_decoded_alone_matches_the_whole_file_across_a_seam() {
        let dir = tempfile::tempdir().unwrap();
        // Name, rate, encoder arguments, whether it lands, and whether a
        // landing is the same samples.
        let cases = [
            (
                "plain.m4a",
                44_100,
                &["-c:a", "aac", "-aac_pns", "0"][..],
                true,
                true,
            ),
            ("sweep.wav", 48_000, &["-c:a", "pcm_s16le"][..], true, true),
            ("noisy.m4a", 44_100, &["-c:a", "aac"][..], true, false),
            ("sweep.mka", 44_100, &["-c:a", "aac"][..], false, true),
        ];
        const RATE: u32 = 48_000;
        // Two seconds, the mixer's own block.
        const BLOCK: usize = 2 * RATE as usize;
        for (name, rate, args, lands, same) in cases {
            let Some(file) = sweep(dir.path(), name, rate, args) else {
                eprintln!("skipping: no ffmpeg CLI available");
                return;
            };
            let whole = decode_all(&file, RATE).unwrap();
            let blocks = whole.frames().div_ceil(BLOCK);
            assert!(blocks >= 6, "{name}: {} frames", whole.frames());
            let held = |block: usize| {
                let from = (block * BLOCK * 2).min(whole.samples.len());
                let to = ((block + 1) * BLOCK * 2).min(whole.samples.len());
                &whole.samples[from..to]
            };

            // Out of order on purpose: a jump forward, a jump back, two in a
            // row (the seam between them is the one a playing clip crosses),
            // the last block, and one past the end.
            let order = [4, 1, 2, 3, blocks - 1, 0, 5, blocks + 2, 2];
            let mut reader = AudioReader::open(&file, RATE).unwrap();
            assert_eq!(reader.lands(), lands, "{name}");
            for block in order {
                let (got, exact) = reader.read((block * BLOCK) as u64, BLOCK, false).unwrap();
                let want = held(block);
                assert_eq!(got.len(), want.len(), "{name}: block {block}");
                if same || exact {
                    assert!(got == want, "{name}: block {block} is not the whole file's");
                } else {
                    // The same sound in the same place, the decoder's own
                    // noise apart. A sample out of place would be out by the
                    // sweep itself, tenths of full scale.
                    let worst = got
                        .iter()
                        .zip(want)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0f32, f32::max);
                    assert!(worst < 0.02, "{name}: block {block} is out by {worst}");
                }
            }
            assert_eq!(reader.lands(), lands, "{name}: every landing checked out");

            // Asked for exactly, every file is the whole-file decode, and
            // says so: what an export reads.
            for block in order {
                let (got, exact) = reader.read((block * BLOCK) as u64, BLOCK, true).unwrap();
                assert!(
                    exact && got == held(block),
                    "{name}: block {block}, exactly"
                );
            }

            // A fresh reader finds the length without decoding the file
            // through, and reads a stretch that starts mid-block as a trimmed
            // clip does.
            let mut fresh = AudioReader::open(&file, RATE).unwrap();
            assert_eq!(fresh.length().unwrap(), whole.frames() as u64, "{name}");
            let odd = 5 * RATE as usize + 12_345;
            let (got, _) = fresh.read(odd as u64, 1000, true).unwrap();
            assert!(got == whole.samples[odd * 2..(odd + 1000) * 2], "{name}");
        }
    }
}
