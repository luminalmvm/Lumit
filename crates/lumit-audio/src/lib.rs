//! Audio playback and the playback clock (docs/09-AUDIO.md;
//! docs/impl/playback-scheduler.md §4).
//!
//! In plain terms: sound cards ask for samples on their own strict schedule
//! through a realtime callback. That callback is sacred — it never allocates,
//! never locks, never waits; if it is ever late you *hear* it. The number of
//! samples it has consumed IS the playback clock: video asks "what time is
//! it?" and chases the answer. That is why footage and sound can never drift
//! apart — there is only one clock, and it is the audio card's.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Device;
use lumit_media::AudioBuffer;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub mod beat;
pub mod meter;
pub mod mix;
pub mod peaks;
pub mod spectra;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio output device")]
    NoDevice,
    #[error("audio device: {0}")]
    Device(String),
}

struct Shared {
    /// The live mix plan (a plain buffer loads as a one-clip plan). Swapped on
    /// load and on audio edits; the callback try-reads and plays silence on a
    /// miss. Swapping the plan — not re-baking a buffer — is what makes
    /// solo/mute/move audible on the next callback (docs/09 §6).
    plan: RwLock<Option<Arc<mix::MixPlan>>>,
    /// Frames consumed since load/seek — the clock.
    playhead: AtomicUsize,
    playing: AtomicBool,
    /// The monitor mute: the device is handed silence while the mix, the
    /// clock and the meters carry on as if it were heard. Not document data,
    /// so it never saves into a project or silences an export.
    muted: AtomicBool,
    /// A scrub burst the callback has not picked up yet ([`pack_scrub`]), or
    /// zero. Written by [`AudioEngine::scrub`] and taken by the callback.
    scrub: AtomicU64,
    /// What the mix is doing, published once per callback for the mixer's
    /// bars (docs/09 §3.1). Written only here; read by anyone holding the
    /// `Arc`, which is why it is beside the clock rather than inside the plan.
    meters: Arc<meter::Meters>,
}

/// How many frames a scrub burst takes to fade in, and again to fade out. A
/// burst starts and stops wherever the playhead happens to be, which is
/// mid-wave nearly every time, and a wave cut off there is heard as a click.
/// About 3 ms at 48 kHz.
const SCRUB_FADE_FRAMES: usize = 128;

/// The low bits of a scrub request that hold the burst's length in frames.
/// 24 of them is nearly six minutes at 48 kHz, for a burst that is a fraction
/// of a second.
const SCRUB_LEN_BITS: u32 = 24;

/// A scrub request as one number: the frame to start at above the length.
/// One atomic rather than two, so the callback can never pair one burst's
/// start with another's length. Zero is "nothing asked", which a burst of no
/// length is anyway.
fn pack_scrub(at: usize, len: usize) -> u64 {
    let at = (at as u64).min(u64::MAX >> SCRUB_LEN_BITS);
    let len = (len as u64).min((1 << SCRUB_LEN_BITS) - 1);
    (at << SCRUB_LEN_BITS) | len
}

/// One burst of scrub sound, as the callback keeps it between buffers.
#[derive(Debug, Clone, Copy, Default)]
struct Burst {
    /// The next frame of the mix to sound.
    at: usize,
    played: usize,
    left: usize,
}

impl Burst {
    /// Up from nothing over the first [`SCRUB_FADE_FRAMES`] and back down to
    /// it over the last.
    fn gain(&self) -> f32 {
        (self.played.min(self.left) as f32 / SCRUB_FADE_FRAMES as f32).min(1.0)
    }
}

/// The scrub as the callback holds it: the burst being heard, and the one it
/// replaced, which fades out underneath it. Dragging the playhead asks for a
/// burst per frame crossed, far faster than one finishes, and cutting each
/// short for the next would click as often as the playhead moves.
#[derive(Debug, Default)]
struct Scrub {
    now: Burst,
    was: Burst,
}

/// A whole buffer as a trivial plan: one clip covering the strip 1:1.
fn plan_of(buffer: Arc<AudioBuffer>) -> Arc<mix::MixPlan> {
    let frames = buffer.frames();
    Arc::new(mix::MixPlan {
        clips: vec![mix::PlacedClip {
            buffer,
            start_frame: 0,
            src_start: 0,
            len: frames,
            gain: [1.0, 1.0],
            envelope: None,
            meter: 0,
        }],
        total_frames: frames,
        master_gain: 1.0,
    })
}

/// One output the machine offers (docs/09 §3.1).
///
/// The **id is the name the sound system reports**, because cpal offers no
/// other handle that survives a restart — and a name is what the user picked
/// off the list in the first place.
//
// ponytail: two outputs with identical names are indistinguishable here and
// the first enumerated wins. A truly unique id needs a cpal that exposes the
// host's own endpoint handle; until then a duplicate name picks one of the
// two rather than failing, which is the harmless half of the problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

/// What the machine offers, and which of them it plays through by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputDevices {
    pub devices: Vec<OutputDevice>,
    /// The system default's id, when the sound system names one.
    pub default_id: Option<String>,
}

impl OutputDevices {
    /// Which output a stored choice actually resolves to: the chosen one while
    /// it is still here, else the system default, else the first output there
    /// is. `None` only when the machine has no output at all.
    ///
    /// This is the whole of the falling-back rule, and it is a plain function
    /// over a list so it can be tested without a sound card: a device that has
    /// been unplugged since the choice was made lands on the default calmly,
    /// and the caller can see it happened by comparing the answer with what it
    /// asked for.
    #[must_use]
    pub fn resolve(&self, wanted: Option<&str>) -> Option<String> {
        let known = |id: &str| self.devices.iter().any(|d| d.id == id);
        wanted
            .filter(|w| known(w))
            .map(str::to_owned)
            .or_else(|| self.default_id.clone().filter(|d| known(d)))
            .or_else(|| self.devices.first().map(|d| d.id.clone()))
    }
}

/// What the machine offers right now.
///
/// Asks the sound system, so it is not something to do in a loop — the
/// Settings page reads it when it opens, and the engine reads it once when it
/// opens a stream. Never fails: a host that will not enumerate reports
/// nothing, which is the same calm no-device state as a machine with no card.
#[must_use]
pub fn output_devices() -> OutputDevices {
    let host = cpal::default_host();
    let devices = host
        .output_devices()
        .map(|it| {
            it.filter_map(|d| d.name().ok())
                .map(|name| OutputDevice {
                    id: name.clone(),
                    name,
                })
                .collect()
        })
        .unwrap_or_default();
    // Linux/ALSA reports no usable default while still enumerating outputs, so
    // it is not asked there: `resolve` then falls back to the first enumerated
    // device, which is what this platform has always played through.
    #[cfg(target_os = "linux")]
    let default_id = None;
    #[cfg(not(target_os = "linux"))]
    let default_id = host.default_output_device().and_then(|d| d.name().ok());
    OutputDevices {
        devices,
        default_id,
    }
}

pub struct AudioEngine {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    device_rate: u32,
    channels: usize,
}

impl AudioEngine {
    /// The cpal handle for `wanted`, resolved through [`OutputDevices::resolve`]
    /// so a vanished choice quietly becomes the system default.
    pub fn get_device(wanted: Option<&str>) -> Result<Device, AudioError> {
        let id = output_devices()
            .resolve(wanted)
            .ok_or(AudioError::NoDevice)?;
        cpal::default_host()
            .output_devices()
            .map_err(|_| AudioError::NoDevice)?
            .find(|d| d.name().is_ok_and(|n| n == id))
            .ok_or(AudioError::NoDevice)
    }

    /// Open the system default output.
    pub fn new() -> Result<Self, AudioError> {
        Self::new_on(None)
    }

    /// Open `wanted`, or the system default when it is not there any more.
    pub fn new_on(wanted: Option<&str>) -> Result<Self, AudioError> {
        let device = Self::get_device(wanted)?;
        let config = device
            .default_output_config()
            .map_err(|e| AudioError::Device(e.to_string()))?;

        let device_rate = config.sample_rate().0;
        let channels = usize::from(config.channels());

        let shared = Arc::new(Shared {
            plan: RwLock::new(None),
            playhead: AtomicUsize::new(0),
            playing: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            scrub: AtomicU64::new(0),
            meters: Arc::new(meter::Meters::default()),
        });
        let cb = shared.clone();
        let mut scrub = Scrub::default();

        let stream = device
            .build_output_stream(
                &config.config(),
                move |out: &mut [f32], _| fill(&cb, &mut scrub, out, channels),
                |_err| { /* device hiccup: next callback continues; never panic */ },
                None,
            )
            .map_err(|e| AudioError::Device(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Device(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            shared,
            device_rate,
            channels,
        })
    }

    /// The rate media should be decoded at so no runtime resampling happens.
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Install a buffer (decoded at `device_rate`) and rewind.
    pub fn load(&self, buffer: Arc<AudioBuffer>) {
        self.load_plan(plan_of(buffer));
    }

    /// Install a live mix plan (clips decoded at `device_rate`) and rewind.
    pub fn load_plan(&self, plan: Arc<mix::MixPlan>) {
        self.shared.playing.store(false, Ordering::Relaxed);
        *self.shared.plan.write() = Some(plan);
        self.shared.playhead.store(0, Ordering::Relaxed);
    }

    /// Replace the plan **without touching the clock or play state** — the
    /// instant-edit path: solo, mute, move and trim swap the plan mid-playback
    /// and are heard on the next callback (~10 ms), no re-bake, no seek.
    pub fn swap_plan(&self, plan: Arc<mix::MixPlan>) {
        *self.shared.plan.write() = Some(plan);
    }

    pub fn unload(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        *self.shared.plan.write() = None;
        self.shared.playhead.store(0, Ordering::Relaxed);
    }

    pub fn play(&self) {
        self.shared.playing.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    /// Hand the device silence without stopping the mix or its clock.
    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
    }

    pub fn is_muted(&self) -> bool {
        self.shared.muted.load(Ordering::Relaxed)
    }

    pub fn seek_seconds(&self, t: f64) {
        let frame = (t.max(0.0) * f64::from(self.device_rate)) as usize;
        self.shared.playhead.store(frame, Ordering::Relaxed);
    }

    /// Sound `len` seconds of the mix from `t`, once, while the transport is
    /// stopped: what dragging the playhead with Ctrl held is heard through.
    ///
    /// The clock does not move and nothing starts playing. A burst asked for
    /// while another is sounding fades in over it, so a drag is one run of
    /// sound and not a click per frame. Asked for during playback it is
    /// dropped, since the mix is already being heard.
    pub fn scrub(&self, t: f64, len: f64) {
        let rate = f64::from(self.device_rate);
        let at = (t.max(0.0) * rate) as usize;
        let len = (len.max(0.0) * rate) as usize;
        self.shared
            .scrub
            .store(pack_scrub(at, len), Ordering::Relaxed);
    }

    /// The playback clock (docs/06-RENDER-PIPELINE.md §A/V sync: audio is
    /// master). Output latency compensation arrives with the ring buffer
    /// work; at ±half a frame tolerance it is acceptable to omit for Gate 0.
    pub fn clock_seconds(&self) -> f64 {
        self.shared.playhead.load(Ordering::Relaxed) as f64 / f64::from(self.device_rate)
    }

    /// A cheap, cloneable, thread-safe read-only view of the playback clock.
    ///
    /// The engine itself cannot leave the thread that built it (the cpal
    /// stream is not `Send`), but its clock is just a pair of atomics — this
    /// handle carries them across threads so a UI can poll "what time is it?"
    /// without owning the engine. Reads are allocation-free and lock-free.
    #[must_use]
    pub fn clock(&self) -> ClockHandle {
        ClockHandle {
            shared: Arc::clone(&self.shared),
            device_rate: self.device_rate,
        }
    }

    /// The mixer's meters, for the same reason [`Self::clock`] exists: the
    /// engine cannot leave its thread, but a panel on another one still has
    /// to read what the callback is publishing. Reads are lock-free.
    #[must_use]
    pub fn meters(&self) -> Arc<meter::Meters> {
        Arc::clone(&self.shared.meters)
    }
}

/// A read-only, `Send + Sync` view of an [`AudioEngine`]'s playback clock —
/// see [`AudioEngine::clock`]. Holding one does not keep audio playing; it
/// only observes the frames the realtime callback has consumed.
#[derive(Clone)]
pub struct ClockHandle {
    shared: Arc<Shared>,
    device_rate: u32,
}

impl ClockHandle {
    /// Seconds of audio consumed since load/seek — identical to
    /// [`AudioEngine::clock_seconds`].
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.shared.playhead.load(Ordering::Relaxed) as f64 / f64::from(self.device_rate)
    }

    /// Whether the engine is currently playing — identical to
    /// [`AudioEngine::is_playing`].
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }
}

/// The realtime callback: lock-free reads, no allocation, silence on any miss.
/// Each frame is summed live from the plan's covering clips
/// ([`mix::MixPlan::frame_at`] — a handful of multiply-adds per frame), which
/// is what lets an edit swap the plan and be heard immediately.
fn fill(shared: &Shared, scrub: &mut Scrub, out: &mut [f32], channels: usize) {
    out.fill(0.0);
    if !shared.playing.load(Ordering::Relaxed) {
        shared.meters.silence();
        fill_scrub(shared, scrub, out, channels);
        return;
    }
    // Playing: a burst cut off by the play button is not picked up again when
    // playback stops, and one asked for meanwhile is not kept for then.
    shared.scrub.store(0, Ordering::Relaxed);
    *scrub = Scrub::default();
    let Some(guard) = shared.plan.try_read() else {
        return; // plan being swapped: one quiet buffer beats a glitch
    };
    let Some(plan) = guard.as_ref() else {
        shared.meters.silence();
        return;
    };
    let total = plan.total_frames;
    let mut playhead = shared.playhead.load(Ordering::Relaxed);
    // On the stack, so a buffer's worth of metering costs no atomics and no
    // allocation; published once, below.
    let mut acc = [meter::MeterAcc::default(); meter::SLOTS];
    for frame in out.chunks_exact_mut(channels) {
        if playhead >= total {
            shared.playing.store(false, Ordering::Relaxed);
            break;
        }
        let (l, r) = plan.frame_metered(playhead, &mut acc);
        frame[0] = l;
        if channels > 1 {
            frame[1] = r;
        }
        playhead += 1;
    }
    shared.playhead.store(playhead, Ordering::Relaxed);
    shared.meters.publish(&acc);
    // After the mix and the clock, so a mute changes what is heard and
    // nothing else: playback paces on this clock, and the meters keep
    // reading the mix.
    if shared.muted.load(Ordering::Relaxed) {
        out.fill(0.0);
    }
}

/// The stopped transport's half of the callback: sound whatever scrub burst
/// is in flight, and nothing otherwise. Under the same rules as [`fill`], and
/// the clock is left where it is.
fn fill_scrub(shared: &Shared, scrub: &mut Scrub, out: &mut [f32], channels: usize) {
    let asked = shared.scrub.swap(0, Ordering::Relaxed);
    if asked != 0 {
        scrub.was = scrub.now;
        scrub.was.left = scrub.was.left.min(SCRUB_FADE_FRAMES);
        scrub.now = Burst {
            at: (asked >> SCRUB_LEN_BITS) as usize,
            played: 0,
            left: (asked & ((1 << SCRUB_LEN_BITS) - 1)) as usize,
        };
    }
    if scrub.now.left == 0 && scrub.was.left == 0 {
        return;
    }
    let Some(guard) = shared.plan.try_read() else {
        return; // plan being swapped: the burst carries on in the next buffer
    };
    let Some(plan) = guard.as_ref() else {
        *scrub = Scrub::default();
        return;
    };
    for frame in out.chunks_exact_mut(channels) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for burst in [&mut scrub.now, &mut scrub.was] {
            if burst.at >= plan.total_frames {
                burst.left = 0;
            }
            if burst.left == 0 {
                continue;
            }
            let gain = burst.gain();
            let (bl, br) = plan.frame_at(burst.at);
            l += bl * gain;
            r += br * gain;
            burst.at += 1;
            burst.played += 1;
            burst.left -= 1;
        }
        frame[0] = l;
        if channels > 1 {
            frame[1] = r;
        }
    }
    if shared.muted.load(Ordering::Relaxed) {
        out.fill(0.0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn list(names: &[&str], default: Option<&str>) -> OutputDevices {
        OutputDevices {
            devices: names
                .iter()
                .map(|n| OutputDevice {
                    id: (*n).to_owned(),
                    name: (*n).to_owned(),
                })
                .collect(),
            default_id: default.map(str::to_owned),
        }
    }

    /// The whole falling-back rule, without a sound card: a choice that is
    /// still plugged in wins, one that has vanished lands on the system
    /// default, a machine whose host names no default takes the first output,
    /// and a machine with nothing at all says so instead of pretending.
    #[test]
    fn a_chosen_device_resolves_and_a_vanished_one_falls_back() {
        let plugged_in = list(&["Speakers", "Headphones"], Some("Speakers"));
        assert_eq!(
            plugged_in.resolve(Some("Headphones")).as_deref(),
            Some("Headphones")
        );
        assert_eq!(plugged_in.resolve(None).as_deref(), Some("Speakers"));

        // The headphones were unplugged since the choice was made.
        let unplugged = list(&["Speakers"], Some("Speakers"));
        assert_eq!(
            unplugged.resolve(Some("Headphones")).as_deref(),
            Some("Speakers"),
            "a device that has gone falls back to the default, calmly"
        );

        // A host that names no default (Linux/ALSA), and one that names a
        // default which is itself no longer enumerated.
        assert_eq!(
            list(&["Speakers", "HDMI"], None)
                .resolve(Some("HDMI"))
                .as_deref(),
            Some("HDMI")
        );
        assert_eq!(
            list(&["Speakers", "HDMI"], None).resolve(None).as_deref(),
            Some("Speakers")
        );
        assert_eq!(
            list(&["HDMI"], Some("Speakers")).resolve(None).as_deref(),
            Some("HDMI")
        );

        // Nothing to play through at all.
        assert_eq!(list(&[], None).resolve(Some("Speakers")), None);
        assert_eq!(OutputDevices::default().resolve(None), None);
    }

    fn tone(frames: usize) -> Arc<AudioBuffer> {
        let mut samples = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let v = (n as f32 * 0.05).sin() * 0.25;
            samples.push(v);
            samples.push(v);
        }
        Arc::new(AudioBuffer {
            rate: 48_000,
            samples,
        })
    }

    /// The callback contract, exercised directly (no device needed in CI):
    /// silence when paused, correct samples when playing, auto-stop at end,
    /// and the clock advancing by exactly the frames consumed.
    #[test]
    fn callback_plays_advances_clock_and_stops_at_end() {
        let shared = Shared {
            plan: RwLock::new(Some(plan_of(tone(1000)))),
            playhead: AtomicUsize::new(0),
            playing: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            scrub: AtomicU64::new(0),
            meters: Arc::new(meter::Meters::default()),
        };
        let mut out = vec![1.0f32; 256 * 2];
        let mut scrub = Scrub::default();

        // Paused: silence, clock still.
        fill(&shared, &mut scrub, &mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);

        // Playing: exact samples, clock advances by frames written.
        shared.playing.store(true, Ordering::Relaxed);
        fill(&shared, &mut scrub, &mut out, 2);
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 256);
        assert!((out[0] - 0.0).abs() < 1e-6); // sin(0)·0.25
        let expected = (255.0f32 * 0.05).sin() * 0.25;
        assert!((out[510] - expected).abs() < 1e-5);

        // Run past the end: stops exactly at the last frame, playing=false.
        for _ in 0..10 {
            fill(&shared, &mut scrub, &mut out, 2);
        }
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 1000);
        assert!(!shared.playing.load(Ordering::Relaxed));
    }

    /// The scrub, through the callback: a burst sounds from where it was
    /// asked for with the transport stopped and the clock unmoved, rises
    /// from nothing so that it cannot click, and ends on its own. A second
    /// burst takes over from the first, and pressing play drops the lot.
    #[test]
    fn a_scrub_burst_sounds_while_stopped_and_leaves_the_clock_alone() {
        let steady = Arc::new(AudioBuffer {
            rate: 48_000,
            samples: vec![0.5; 4000 * 2],
        });
        let shared = Shared {
            plan: RwLock::new(Some(plan_of(steady))),
            playhead: AtomicUsize::new(0),
            playing: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            scrub: AtomicU64::new(pack_scrub(1000, 600)),
            meters: Arc::new(meter::Meters::default()),
        };
        let mut scrub = Scrub::default();
        let mut out = vec![0.0f32; 256 * 2];

        fill(&shared, &mut scrub, &mut out, 2);
        assert_eq!(out[0], 0.0, "the burst starts from nothing");
        assert!((out[255 * 2] - 0.5).abs() < 1e-6, "and reaches the mix");
        assert_eq!(scrub.now.at, 1256, "read from where it was asked for");
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);
        assert!(!shared.playing.load(Ordering::Relaxed));

        // A second burst before the first is over: the first fades out
        // under it, and the two together are never louder than the mix.
        shared.scrub.store(pack_scrub(2000, 600), Ordering::Relaxed);
        fill(&shared, &mut scrub, &mut out, 2);
        assert_eq!(scrub.now.at, 2256);
        assert_eq!(scrub.was.left, 0, "the old burst has faded out");
        assert!(out.iter().all(|s| *s <= 0.5 + 1e-6));

        // Left alone, it ends, and the buffer after that is silent.
        for _ in 0..3 {
            fill(&shared, &mut scrub, &mut out, 2);
        }
        assert!(out.iter().all(|s| *s == 0.0), "a burst is not a loop");

        // Pressing play drops a burst that was asked for and not yet heard.
        shared.scrub.store(pack_scrub(2000, 600), Ordering::Relaxed);
        shared.playing.store(true, Ordering::Relaxed);
        fill(&shared, &mut scrub, &mut out, 2);
        assert_eq!(shared.scrub.load(Ordering::Relaxed), 0);
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 256);
    }

    /// The instant-edit path: swapping the plan mid-play keeps the clock and
    /// the play state, and the very next callback reads the new plan's
    /// samples — this is what makes solo/mute/move audible immediately.
    #[test]
    fn swapping_the_plan_keeps_the_clock_and_changes_the_sound() {
        let shared = Shared {
            plan: RwLock::new(Some(plan_of(tone(1000)))),
            playhead: AtomicUsize::new(0),
            playing: AtomicBool::new(true),
            muted: AtomicBool::new(false),
            scrub: AtomicU64::new(0),
            meters: Arc::new(meter::Meters::default()),
        };
        let mut out = vec![0.0f32; 128 * 2];
        let mut scrub = Scrub::default();
        fill(&shared, &mut scrub, &mut out, 2);
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 128);

        // "Mute": swap in a silent plan (no clips) of the same length.
        *shared.plan.write() = Some(Arc::new(mix::MixPlan {
            clips: Vec::new(),
            total_frames: 1000,
            master_gain: 1.0,
        }));
        fill(&shared, &mut scrub, &mut out, 2);
        assert_eq!(
            shared.playhead.load(Ordering::Relaxed),
            256,
            "the clock kept running across the swap"
        );
        assert!(shared.playing.load(Ordering::Relaxed));
        assert!(
            out.iter().all(|s| *s == 0.0),
            "the new plan is heard immediately"
        );
    }
}
