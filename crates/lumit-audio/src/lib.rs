//! Audio playback and the playback clock (docs/09-AUDIO.md;
//! docs/impl/playback-scheduler.md §4).
//!
//! In plain terms: sound cards ask for samples on their own strict schedule
//! through a realtime callback. That callback is sacred — it never allocates,
//! never locks, never waits; if it is ever late you *hear* it. The number of
//! samples it has consumed IS the playback clock: video asks "what time is
//! it?" and chases the answer. That is why footage and sound can never drift
//! apart — there is only one clock, and it is the audio card's.
//!
//! The callback has a partner: a thread that **fills ahead** of it. A long
//! cut's sound is not all in memory ([`stream`]), so that thread keeps the
//! blocks around the playhead decoded while the callback plays whatever is
//! there. Where the callback gets to a block first it plays silence, counts
//! the frames ([`ClockHandle::starved_frames`]) and carries on: the clock
//! never waits for a decode.
//!
//! The mix itself is a graph of nodes ([`plan`], [`block`]): the callback
//! walks it once a buffer with its own [`block::MixWork`], made when the
//! stream opens.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::Device;
use lumit_media::audio::AudioBuffer;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub mod beat;
pub mod block;
pub mod duck;
mod jog;
pub mod limiter;
pub mod live;
pub mod loudness;
pub mod meter;
pub mod mix;
pub mod peaks;
pub mod plan;
pub mod rack;
mod rack_thread;
mod slow;
pub mod spectra;
pub mod speed;
pub mod stereo_peaks;
pub mod stream;
pub mod sync;
#[cfg(target_arch = "wasm32")]
mod web;

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
    plan: RwLock<Option<Arc<plan::MixPlan>>>,
    /// Frames consumed since load/seek — the clock.
    playhead: AtomicUsize,
    /// How many frames the last callback moved the playhead by, and when it
    /// did ([`nanos`]). A sound card takes its samples a buffer at a time, so
    /// the playhead alone moves in steps of ten milliseconds or more, and a
    /// picture timed off those steps arrives unevenly. With these two a
    /// reader can say where the clock is between one callback and the next
    /// ([`Shared::clock_frames`]).
    step: AtomicUsize,
    stepped_at: AtomicU64,
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
    /// A scrub burst somebody has asked for and the filling thread has not
    /// made ready yet ([`pack_scrub`]), or zero. It decodes what the burst
    /// will sound and then hands it to `scrub`, so a burst is heard whole
    /// and a moment late, and not at once with its first blocks missing.
    scrub_wanted: AtomicU64,
    /// Where the jog is asked to go, as the bits of a frame count (f64), or
    /// [`jog::NO_JOG`]. Written by [`AudioEngine::jog`] and read by the
    /// callback and the filling thread.
    jog: AtomicU64,
    /// How many frames of some clip were due and not in memory, and were
    /// played as silence. Only ever grows.
    starved: AtomicU64,
    /// Set when the engine goes, which is what ends the filling thread.
    closed: AtomicBool,
    /// How many times slower than the mix the sound plays ([`slow`]).
    slow: AtomicU32,
    /// How late what the callback mixes is heard, in frames: the device's
    /// own delay as it reports it, smoothed, and the limiter's look ahead.
    device_latency: AtomicU32,
    lookahead: AtomicU32,
    /// The stream has stopped for good: the device went away.
    device_lost: AtomicBool,
}

impl Shared {
    fn new(plan: Option<Arc<plan::MixPlan>>) -> Self {
        Self {
            plan: RwLock::new(plan),
            playhead: AtomicUsize::new(0),
            step: AtomicUsize::new(0),
            stepped_at: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            scrub: AtomicU64::new(0),
            meters: Arc::new(meter::Meters::default()),
            scrub_wanted: AtomicU64::new(0),
            jog: AtomicU64::new(jog::NO_JOG),
            starved: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            slow: AtomicU32::new(1),
            device_latency: AtomicU32::new(0),
            lookahead: AtomicU32::new(0),
            device_lost: AtomicBool::new(false),
        }
    }

    /// How late the sound is heard after it is mixed, in frames.
    fn latency_frames(&self) -> u32 {
        self.device_latency
            .load(Ordering::Relaxed)
            .saturating_add(self.lookahead.load(Ordering::Relaxed))
    }

    /// Where the clock stands, in frames.
    ///
    /// The callback hands the card a buffer and counts all of it at once.
    /// While playing, that buffer is counted in here as time passes, so the
    /// clock runs smoothly from the start of the buffer to its end, and
    /// waits there if the next callback is late. Playing, it is also held
    /// back by how late the sound is heard, so it reads what is heard now.
    /// Stopped, it is the playhead.
    // The reads are not one read. One that lands across a callback is out by
    // a buffer at most, for that one reading.
    fn clock_frames(&self, rate: u32) -> f64 {
        let playhead = self.playhead.load(Ordering::Relaxed) as f64;
        if !self.playing.load(Ordering::Relaxed) {
            return playhead;
        }
        let step = self.step.load(Ordering::Relaxed) as f64;
        let since = nanos().saturating_sub(self.stepped_at.load(Ordering::Relaxed));
        let slow = f64::from(self.slow.load(Ordering::Relaxed).max(1));
        let passed = since as f64 * f64::from(rate) / 1e9 / slow;
        let late = f64::from(self.latency_frames()) / slow;
        (playhead - step + passed.min(step) - late).max(0.0)
    }
}

/// How many frames the callback mixes at a time: more than a sound card asks
/// for in one go, so a callback is one walk of the plan.
const SCRATCH_FRAMES: usize = block::SCRATCH_FRAMES;

/// How much of each new reading of the device's delay the smoothed figure
/// takes. The reading wobbles from buffer to buffer; the clock must not.
#[cfg(not(target_arch = "wasm32"))]
const LATENCY_SMOOTHING: f64 = 0.05;

/// How far ahead of the playhead the filling thread keeps the sound decoded,
/// in seconds: while playing, and while stopped (when what matters is that
/// the next press of play starts with sound).
pub const PLAY_AHEAD_SECONDS: usize = 8;
pub const STOPPED_AHEAD_SECONDS: usize = 4;

/// The longest a start waits for its sound to be ready
/// ([`AudioEngine::start_at`]).
pub const START_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

/// How long the filling thread rests when there is nothing to decode. A seek,
/// a new plan or a scrub wakes it at once.
#[cfg(not(target_arch = "wasm32"))]
const FILL_REST: std::time::Duration = std::time::Duration::from_millis(40);

/// How long past now the filling thread wants its blocks for, in
/// milliseconds: longer than its rest, so they are still wanted when it next
/// looks, and long enough that an export reading through the same pool does
/// not push the playhead's own blocks out.
const FILL_HOLD_MS: u64 = 2_000;

/// Nanoseconds since the first call, for placing the clock between callbacks.
fn nanos() -> u64 {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(web_time::Instant::now);
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
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
    /// The varispeed scrub ([`jog`]).
    jog: jog::Jog,
}

/// A whole buffer as a trivial plan: one clip covering the strip 1:1, into
/// one node.
fn plan_of(buffer: Arc<AudioBuffer>) -> Arc<plan::MixPlan> {
    let frames = buffer.frames();
    Arc::new(plan::MixPlan::single(
        vec![plan::PlacedClip {
            source: stream::Source::whole(buffer),
            start_frame: 0,
            src_start: 0,
            len: frames,
            gain: [1.0, 1.0],
            envelope: None,
            node: 0,
            silent: false,
        }],
        frames,
    ))
}

/// Hand one mixed stereo frame to a device frame: both sides to a stereo
/// device, `(L + R) / 2` to a mono one, silence on any channel past two.
/// With `gain`, added rather than written.
#[inline]
fn put(frame: &mut [f32], l: f32, r: f32, gain: Option<f32>) {
    let (l, r) = match frame.len() {
        1 => ((l + r) * 0.5, 0.0),
        _ => (l, r),
    };
    for (d, s) in frame.iter_mut().zip([l, r]) {
        match gain {
            Some(g) => *d += s * g,
            None => *d = s,
        }
    }
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
    #[cfg(not(target_arch = "wasm32"))]
    _stream: cpal::Stream,
    /// What the callback keeps between buffers, on the page ([`web`]).
    #[cfg(target_arch = "wasm32")]
    output: std::cell::RefCell<web::Output>,
    shared: Arc<Shared>,
    device_rate: u32,
    channels: usize,
    /// The thread that fills ahead and the rack thread, to wake them when
    /// where they should be filling has changed.
    filler: std::thread::Thread,
    racks: std::thread::Thread,
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
        self.wake();
    }
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
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_on(wanted: Option<&str>) -> Result<Self, AudioError> {
        use cpal::traits::StreamTrait;
        let device = Self::get_device(wanted)?;
        let config = device
            .default_output_config()
            .map_err(|e| AudioError::Device(e.to_string()))?;

        let device_rate = config.sample_rate().0;
        let channels = usize::from(config.channels());

        let shared = Arc::new(Shared::new(None));
        let cb = shared.clone();
        let mut scrub = Scrub::default();
        let mut edge = Edge::default();
        let mut stretch = slow::Stretch::new(device_rate);
        let mut loudness = loudness::Loudness::new(device_rate);
        let mut work = block::MixWork::new(device_rate).owning_live();
        let mut late = 0.0f64;
        let lost = shared.clone();

        let stream = device
            .build_output_stream(
                &config.config(),
                move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    let stamp = info.timestamp();
                    if let Some(gap) = stamp.playback.duration_since(&stamp.callback) {
                        let frames = gap.as_secs_f64() * f64::from(device_rate);
                        late += (frames - late) * LATENCY_SMOOTHING;
                        cb.device_latency
                            .store(late.round() as u32, Ordering::Relaxed);
                    }
                    let meter = &mut loudness;
                    fill_smooth(
                        &cb,
                        &mut scrub,
                        &mut edge,
                        &mut stretch,
                        &mut work,
                        meter,
                        out,
                        channels,
                    )
                },
                move |err| {
                    // A hiccup carries on with the next callback. A device
                    // that has gone ends the stream, and whoever owns the
                    // engine opens another.
                    if matches!(err, cpal::StreamError::DeviceNotAvailable) {
                        lost.device_lost.store(true, Ordering::Relaxed);
                    }
                },
                None,
            )
            .map_err(|e| AudioError::Device(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Device(e.to_string()))?;

        let ahead = shared.clone();
        let filler = std::thread::Builder::new()
            .name("lumit-audio-fill".into())
            .spawn(move || {
                while !ahead.closed.load(Ordering::Relaxed) {
                    if !fill_ahead(&ahead, device_rate) {
                        std::thread::park_timeout(FILL_REST);
                    }
                }
            })
            .map_err(|e| AudioError::Device(e.to_string()))?
            .thread()
            .clone();
        let racks = rack_thread::spawn(Arc::clone(&shared), device_rate)
            .map_err(|e| AudioError::Device(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            shared,
            device_rate,
            channels,
            filler,
            racks,
        })
    }

    /// Wake the filling thread and the rack thread: what they should be
    /// filling has changed.
    fn wake(&self) {
        self.filler.unpark();
        self.racks.unpark();
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
    pub fn load_plan(&self, plan: Arc<plan::MixPlan>) {
        self.shared.playing.store(false, Ordering::Relaxed);
        *self.shared.plan.write() = Some(plan);
        self.shared.playhead.store(0, Ordering::Relaxed);
        // Another composition: its loudness is measured from the start.
        self.shared.meters.reset_loudness();
        self.wake();
    }

    /// Replace the plan **without touching the clock or play state** — the
    /// instant-edit path: solo, mute, move and trim swap the plan mid-playback
    /// and are heard on the next callback (~10 ms), no re-bake, no seek.
    pub fn swap_plan(&self, plan: Arc<plan::MixPlan>) {
        *self.shared.plan.write() = Some(plan);
        self.wake();
    }

    pub fn unload(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        *self.shared.plan.write() = None;
        self.shared.playhead.store(0, Ordering::Relaxed);
    }

    pub fn play(&self) {
        self.shared.playing.store(true, Ordering::Relaxed);
        self.wake();
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
        // No buffer has been counted from here yet.
        self.shared.step.store(0, Ordering::Relaxed);
        self.shared.playhead.store(frame, Ordering::Relaxed);
        self.wake();
    }

    /// Play from `t` seconds, heard from its first frame: where the sound
    /// there is not decoded yet, or a rack has not run in, the transport
    /// stands stopped at `t` until it is, for up to [`START_WAIT`]. Waits on
    /// the caller's thread, never the callback's; the page cannot wait, so
    /// there it starts at once.
    pub fn start_at(&self, t: f64) {
        let frame = (t.max(0.0) * f64::from(self.device_rate)) as usize;
        let want = self.device_rate as usize / 20;
        let ready = || {
            let plan = self.shared.plan.read();
            plan.as_ref()
                .is_none_or(|p| p.ready(frame, want.min(p.total_frames.saturating_sub(frame))))
        };
        if cfg!(not(target_arch = "wasm32")) && !ready() {
            self.pause();
            self.seek_seconds(t);
            let began = web_time::Instant::now();
            while !ready() && began.elapsed() < START_WAIT {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        } else {
            self.seek_seconds(t);
        }
        self.play();
    }

    /// Sound `len` seconds of the mix from `t`, once, while the transport is
    /// stopped: what dragging the playhead with Ctrl held is heard through.
    ///
    /// The clock does not move and nothing starts playing. A burst asked for
    /// while another is sounding fades in over it, so a drag is one run of
    /// sound and not a click per frame. Asked for during playback it is
    /// dropped, since the mix is already being heard.
    ///
    /// The burst goes by way of the filling thread, which decodes what it
    /// will sound before the callback is handed it.
    pub fn scrub(&self, t: f64, len: f64) {
        let rate = f64::from(self.device_rate);
        let at = (t.max(0.0) * rate) as usize;
        let len = (len.max(0.0) * rate) as usize;
        self.shared
            .scrub_wanted
            .store(pack_scrub(at, len), Ordering::Relaxed);
        self.wake();
    }

    /// The playback clock (docs/06-RENDER-PIPELINE.md §A/V sync: audio is
    /// master), less how late the sound is heard while playing.
    pub fn clock_seconds(&self) -> f64 {
        self.shared.clock_frames(self.device_rate) / f64::from(self.device_rate)
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
        self.shared.clock_frames(self.device_rate) / f64::from(self.device_rate)
    }

    /// Whether the engine is currently playing — identical to
    /// [`AudioEngine::is_playing`].
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    /// How many frames of sound have been played as silence because their
    /// block was not decoded in time, since the engine opened. It only grows,
    /// so a reader that remembers the last figure sees each shortfall once.
    #[must_use]
    pub fn starved_frames(&self) -> u64 {
        self.shared.starved.load(Ordering::Relaxed)
    }

    /// The frame the mix has got to, with no allowance for how late it is
    /// heard: where a rack thread fills its rings from.
    #[must_use]
    pub fn playhead_frames(&self) -> usize {
        self.shared.playhead.load(Ordering::Relaxed)
    }

    /// The rate the engine mixes at.
    #[must_use]
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    /// How late what is mixed is heard, in seconds: the device's own delay
    /// and the master limiter's look ahead. The clock already takes it off.
    #[must_use]
    pub fn output_latency_seconds(&self) -> f64 {
        f64::from(self.shared.latency_frames()) / f64::from(self.device_rate.max(1))
    }

    /// The device has gone and the stream with it. Nothing more will be heard
    /// until an engine is opened again.
    #[must_use]
    pub fn device_lost(&self) -> bool {
        self.shared.device_lost.load(Ordering::Relaxed)
    }
}

/// **One step of the filling thread**: make a waiting scrub ready, then decode
/// the block the playhead will need soonest. `true` when it decoded one, which
/// means there may be another and it should be called again at once.
///
/// Reads the plan with a read lock, which the callback's own `try_read` never
/// loses to. Everything that waits happens here: file access, decoding, the
/// pool's lock.
fn fill_ahead(shared: &Shared, rate: u32) -> bool {
    let plan = shared.plan.read().clone();
    let Some(plan) = plan else {
        // Nothing loaded, so a scrub asked for has nothing to sound.
        shared.scrub_wanted.store(0, Ordering::Relaxed);
        return false;
    };
    // The scrub first: it is what somebody is listening for right now.
    let asked = shared.scrub_wanted.swap(0, Ordering::Relaxed);
    if asked != 0 {
        let at = (asked >> SCRUB_LEN_BITS) as usize;
        let len = (asked & ((1 << SCRUB_LEN_BITS) - 1)) as usize;
        plan.warm(at, at.saturating_add(len));
        shared.scrub.store(asked, Ordering::Relaxed);
    }
    // And the stretch around where the jog is heading.
    if let Some(target) = shared.jog_target() {
        let (at, half) = (target as usize, rate as usize / 2);
        plan.warm(at.saturating_sub(half), at.saturating_add(half));
    }
    let playhead = shared.playhead.load(Ordering::Relaxed);
    let ahead = if shared.playing.load(Ordering::Relaxed) {
        PLAY_AHEAD_SECONDS
    } else {
        STOPPED_AHEAD_SECONDS
    } * rate as usize;
    let until = stream::now().saturating_add(FILL_HOLD_MS);
    if plan.fill_step(playhead, playhead.saturating_add(ahead), until) {
        return true;
    }
    // A plan whose every file fits in half the budget is simply decoded, as
    // it always used to be: a song and a few clips never wait for a block
    // wherever the playhead is put.
    if plan
        .file_frames()
        .saturating_mul(2 * std::mem::size_of::<f32>())
        <= stream::DEFAULT_BUDGET_BYTES / 2
    {
        return plan.fill_step(0, plan.total_frames, until);
    }
    false
}

/// What the callback remembers of the buffer before, to join this one to it.
#[derive(Debug, Default)]
struct Edge {
    /// Whether the last buffer was playing, and where the next was to begin.
    playing: bool,
    next: usize,
    /// The last frame handed to the card.
    last: [f32; 2],
}

/// [`fill`], with the join made good where the sound stops, starts or moves.
///
/// Each of those cuts the wave off wherever it happened to be, which is heard
/// as a click: at every stop and start, and at every turn of a loop. The
/// first [`SCRUB_FADE_FRAMES`] of such a buffer are blended up from the last
/// frame of the one before, so the wave arrives where it is going and does
/// not jump there.
#[allow(clippy::too_many_arguments)]
fn fill_smooth(
    shared: &Shared,
    scrub: &mut Scrub,
    edge: &mut Edge,
    stretch: &mut slow::Stretch,
    work: &mut block::MixWork,
    loudness: &mut loudness::Loudness,
    out: &mut [f32],
    channels: usize,
) {
    if shared.meters.take_loudness_reset() {
        loudness.reset();
    }
    let playing = shared.playing.load(Ordering::Relaxed);
    let began = shared.playhead.load(Ordering::Relaxed);
    let lookahead = if stretch.fill(shared, scrub, work, out, channels) {
        // Slowed down is not the mix as it will be heard, so it is not measured.
        loudness.pause();
        0
    } else {
        fill(shared, scrub, work, Some(&mut *loudness), out, channels);
        work.latency()
    };
    shared.lookahead.store(lookahead, Ordering::Relaxed);
    shared.meters.publish_loudness(loudness.reading());
    if channels == 0 {
        return;
    }
    if playing != edge.playing || (playing && began != edge.next) {
        let frames = out.chunks_exact_mut(channels).take(SCRUB_FADE_FRAMES);
        for (n, frame) in frames.enumerate() {
            let up = n as f32 / SCRUB_FADE_FRAMES as f32;
            for (sample, last) in frame.iter_mut().zip(edge.last) {
                *sample = *sample * up + last * (1.0 - up);
            }
        }
    }
    edge.playing = playing;
    edge.next = shared.playhead.load(Ordering::Relaxed);
    if let Some(frame) = out.chunks_exact(channels).last() {
        edge.last = [frame[0], frame.get(1).copied().unwrap_or(frame[0])];
    }
}

/// The realtime callback: lock-free reads, no allocation, silence on any miss.
/// Each buffer is mixed live from the plan ([`plan::MixPlan::mix_block`]),
/// which is what lets an edit swap the plan and be heard immediately. A clip
/// whose block is not in memory yet is silence for those frames, and so is a
/// rack's output not yet in its ring; both are counted. The master's loudness
/// is measured into `loudness` when one is given.
fn fill(
    shared: &Shared,
    scrub: &mut Scrub,
    work: &mut block::MixWork,
    mut loudness: Option<&mut loudness::Loudness>,
    out: &mut [f32],
    channels: usize,
) {
    out.fill(0.0);
    if channels == 0 {
        return;
    }
    if !shared.playing.load(Ordering::Relaxed) {
        shared.meters.silence();
        if let Some(loudness) = loudness {
            loudness.pause();
        }
        work.set_limiter(false);
        fill_scrub(shared, scrub, work, out, channels);
        return;
    }
    work.set_limiter(true);
    // Playing: a burst cut off by the play button is not picked up again when
    // playback stops, and one asked for meanwhile is not kept for then.
    shared.scrub.store(0, Ordering::Relaxed);
    shared.jog.store(jog::NO_JOG, Ordering::Relaxed);
    *scrub = Scrub::default();
    let Some(guard) = shared.plan.try_read() else {
        return; // plan being swapped: one quiet buffer beats a glitch
    };
    let Some(plan) = guard.as_ref() else {
        shared.meters.silence();
        if let Some(loudness) = loudness {
            loudness.pause();
        }
        return;
    };
    let total = plan.total_frames;
    let began = shared.playhead.load(Ordering::Relaxed);
    let mut playhead = began;
    // On the stack, so a buffer's worth of mixing and metering costs no
    // atomics and no allocation; published once, below.
    let mut acc = [meter::MeterAcc::default(); meter::SLOTS];
    let mut scratch = [0.0f32; SCRATCH_FRAMES * 2];
    let mut rings = block::RingSource::default();
    let mut missed = 0usize;
    for chunk in out.chunks_mut(SCRATCH_FRAMES * channels) {
        let wanted = chunk.len() / channels;
        let frames = wanted.min(total.saturating_sub(playhead));
        let mixed = &mut scratch[..frames * 2];
        missed += plan.mix_block(playhead, mixed, work, &mut rings, Some(&mut acc), None);
        if let Some(loudness) = loudness.as_deref_mut() {
            loudness.add(mixed);
        }
        for (frame, s) in chunk.chunks_exact_mut(channels).zip(mixed.chunks_exact(2)) {
            put(frame, s[0], s[1], None);
        }
        playhead += frames;
        if frames < wanted {
            shared.playing.store(false, Ordering::Relaxed);
            break;
        }
    }
    // Only if nothing moved the playhead meanwhile. A seek that landed while
    // this buffer was being mixed used to be written over here, and the
    // sound then carried on from where it had been.
    if shared
        .playhead
        .compare_exchange(began, playhead, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        shared.step.store(playhead - began, Ordering::Relaxed);
        shared.stepped_at.store(nanos(), Ordering::Relaxed);
    }
    shared.meters.publish(&acc, plan.nodes.len());
    missed += rings.missed;
    if missed > 0 {
        shared.starved.fetch_add(missed as u64, Ordering::Relaxed);
    }
    // After the mix and the clock, so a mute changes what is heard and
    // nothing else: playback paces on this clock, and the meters keep
    // reading the mix.
    if shared.muted.load(Ordering::Relaxed) {
        out.fill(0.0);
    }
}

/// The stopped transport's half of the callback: sound whatever scrub burst
/// is in flight, and nothing otherwise. Under the same rules as [`fill`], and
/// the clock is left where it is. Racks are heard dry: their streams run
/// ahead of the playhead, not of a scrub.
fn fill_scrub(
    shared: &Shared,
    scrub: &mut Scrub,
    work: &mut block::MixWork,
    out: &mut [f32],
    channels: usize,
) {
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
    let target = shared.jog_target();
    if scrub.now.left == 0 && scrub.was.left == 0 && !scrub.jog.wanted(target) {
        return;
    }
    let Some(guard) = shared.plan.try_read() else {
        return; // plan being swapped: the burst carries on in the next buffer
    };
    let Some(plan) = guard.as_ref() else {
        *scrub = Scrub::default();
        return;
    };
    let mut scratch = [0.0f32; SCRATCH_FRAMES * 2];
    let mut missed = 0usize;
    for chunk in out.chunks_mut(SCRATCH_FRAMES * channels) {
        // The burst being heard, then the one fading out under it, each mixed
        // for as much of this stretch as it has left and added in.
        for burst in [&mut scrub.now, &mut scrub.was] {
            // No further than the mix runs. Shortened and not stopped, so a
            // burst that reaches the end fades into it and does not click.
            burst.left = burst.left.min(plan.total_frames.saturating_sub(burst.at));
            let frames = (chunk.len() / channels)
                .min(burst.left)
                .min(plan.total_frames.saturating_sub(burst.at));
            let mixed = &mut scratch[..frames * 2];
            missed += plan.mix_block(burst.at, mixed, work, &mut block::DrySource, None, None);
            for (frame, s) in chunk.chunks_exact_mut(channels).zip(mixed.chunks_exact(2)) {
                put(frame, s[0], s[1], Some(burst.gain()));
                burst.at += 1;
                burst.played += 1;
                burst.left -= 1;
            }
            if burst.at >= plan.total_frames {
                burst.left = 0;
            }
        }
    }
    missed += scrub.jog.fill(plan, target, work, out, channels);
    if missed > 0 {
        shared.starved.fetch_add(missed as u64, Ordering::Relaxed);
    }
    if shared.muted.load(Ordering::Relaxed) {
        out.fill(0.0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
