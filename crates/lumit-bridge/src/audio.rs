//! Comp audio playback for the Flutter frontend — gated, like beat detection,
//! behind the `media` + `render` features (the mix needs the decoders and the
//! audio-jobs walk `lumit-render` carries).
//!
//! # In plain terms
//!
//! The sound card asks for samples on its own strict schedule, and the number
//! it has consumed IS the playback clock — the picture asks "what time is it?"
//! and chases the answer (docs/09-AUDIO.md; docs/impl/playback-scheduler.md
//! §4). This module owns that clock for the Flutter frontend: Dart says
//! "prepare this comp's audio", "play from here", "pause", "where are we?",
//! and the answers drive the Viewer's playhead.
//!
//! Three threads cooperate, and none ever holds a lock across slow work
//! (docs/14-ENGINEERING-RULES.md):
//!
//! - **The audio thread** owns the [`lumit_audio::AudioEngine`] — the cpal
//!   stream is not `Send`, so the engine lives its whole life on the one
//!   thread that built it, taking commands (load/swap/play/pause/seek) over a
//!   channel. A machine with no output device resolves to a calm terminal
//!   "no audio" state on the first attempt: playback then simply has no
//!   sound, and nothing retries or errors per call.
//! - **The prepare worker** builds a comp's mix in the background: walk the
//!   document for audio jobs (the GPU-free [`lumit_render::headless::AudioJobsBuilder`]
//!   seam, so audio never queues behind a slow comp render), place the clips,
//!   and hand the finished [`MixPlan`] to the audio thread. It decodes
//!   nothing but the clips that carry a rack: a plan names its files, and
//!   the engine decodes the blocks near the playhead as it plays
//!   ([`lumit_audio::stream`]). The FFI prepare call only *kicks* this worker
//!   and returns; one worker runs at a time with a one-slot latest-wins
//!   mailbox, so a burst of edits coalesces.
//! - **Any caller thread** (Dart's UI isolate) takes the small state lock for
//!   microseconds — bookkeeping and channel sends only. The per-tick clock
//!   poll is allocation-free: a lock, two atomic reads, done.
//!
//! The instant-edit contract (docs/09 §6): when the same comp's audio is
//! already loaded and only the mix changed, the fresh plan is **swapped** in —
//! clock and play state untouched — so a mute/solo/move/trim is heard on the
//! next audio callback, mid-playback, with no restart. A jobs signature makes
//! an unchanged mix a no-op.

#![cfg(feature = "media")]

use lumit_audio::mix::MixPlan;
use lumit_render::headless::AudioJobsBuilder;
use lumit_render::mix::jobs::AudioJob;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

mod engine;
pub(crate) mod loudness;
pub(crate) mod mix_peaks;
mod signature;

use signature::jobs_signature;

/// A command for the audio thread — the only code that touches the engine.
enum Cmd {
    /// Install a plan and rewind, then optionally seek and play — a fresh
    /// load (a different comp, or the first mix of a session).
    Load {
        plan: Arc<MixPlan>,
        start: Option<f64>,
        play: bool,
    },
    /// Replace the plan without touching the clock or play state — the
    /// instant-edit path (docs/09 §6).
    Swap(Arc<MixPlan>),
    Play,
    Pause,
    /// The monitor mute: silence at the device, the mix and clock untouched.
    Mute(bool),
    /// How many times slower than the mix the sound plays, its pitch kept.
    Slow(u32),
    Seek(f64),
    /// Sound `len` seconds from `at` once, with the transport stopped.
    Scrub {
        at: f64,
        len: f64,
    },
    Unload,
}

/// The audio device, as this session knows it.
enum Device {
    /// Never asked for yet — the first prepare/play resolves it.
    Untried,
    /// No output device (or the stream would not open): the calm terminal
    /// state. Playback has no sound; nothing retries.
    Unavailable,
    /// A live engine on its thread: the command channel, the shared clock,
    /// the mixer's meters, and the device rate media decodes at.
    Ready {
        tx: Sender<Cmd>,
        clock: lumit_audio::ClockHandle,
        meters: Arc<lumit_audio::meter::Meters>,
        rate: u32,
    },
}

/// Everything behind the session audio lock. The lock is held only for
/// bookkeeping and channel sends — never across a probe, a decode, a mix, or
/// the FFI boundary.
struct AudioState {
    device: Device,
    /// The output the user chose, by id, or `None` for the system default.
    /// The frontend hands it over on boot and whenever it changes (it lives in
    /// the settings file, not in a project — a machine's sound card is not a
    /// property of the work). Applied when the stream is next opened, and
    /// [`set_device`] closes the open one so that is immediately.
    wanted_device: Option<String>,
    /// Bumped every time the output changes. A prepare worker carries the
    /// number its stream was opened under, so a mix that finishes building
    /// after the device moved can tell its engine was closed underneath it.
    device_gen: u64,
    /// Which comp's mix the engine holds, and the jobs signature it was built
    /// from — the swap-vs-load and no-op decisions.
    loaded_comp: Option<Uuid>,
    loaded_sig: Option<u64>,
    /// The transport intent (Dart's play/pause), applied to a fresh load when
    /// it installs.
    playing: bool,
    /// The monitor mute, kept here so it outlives the engine: a changed
    /// output opens a fresh stream, and a muted user must not get sound back.
    muted: bool,
    /// How many times slower than the mix the comp playing is heard, kept
    /// here for the same reason as `muted`.
    slow: u32,
    /// How late this machine's output is heard, in milliseconds, from the
    /// Audio settings. The picture follows the clock less this, so it lands
    /// with the sound as it reaches the ears. Below zero the picture is the
    /// late one, as on a television that holds frames back.
    offset_ms: i32,
    /// Where a fresh load should start, in seconds — set by `audio_play` when
    /// the wanted comp is not loaded yet.
    pending_start: Option<f64>,
    /// One prepare worker at a time; a request landing while it runs parks in
    /// the one-slot latest-wins mailbox.
    worker_busy: bool,
    /// Which footage item's **preview** the Project panel last asked for, if
    /// any. Cleared by anything else that takes the engine, so a preview whose
    /// file was still decoding when a comp started playing is dropped rather
    /// than stamped over the comp's mix.
    wanted_preview: Option<Uuid>,
    /// What the preview in the engine reads, where one is: the file and the
    /// streams of it. A press that asks for other streams of the same file
    /// is another preview, not a seek in this one.
    loaded_preview: Option<lumit_media::audio::SoundSource>,
    /// The comp waiting for the worker, with the document to build it from —
    /// the snapshot is captured at request time so a later edit cannot change
    /// what a queued prepare is about to mix.
    pending_prepare: Option<(Uuid, Arc<lumit_core::model::Document>)>,
    /// The audio-jobs walk with its has-audio probe cache. Taken out of the
    /// state (never probed under the lock) by the worker and put back.
    jobs: AudioJobsBuilder,
    /// What the racks made of each racked job in the plan that is loaded, so
    /// the next build runs only the racks an edit changed.
    bakes: lumit_render::mix::live::RackBakes,
    /// How much sound the engine had played as silence when it was last
    /// asked, so each shortfall is noted once ([`clock`]).
    starved: u64,
    /// Which layer each meter slot of the installed plan belongs to, in the
    /// order the mixer draws its strips. Replaced with the plan, so
    /// a poll can never name a strip the sound is not on.
    meter_strips: Vec<Uuid>,
    /// The plan the engine is playing and the rate it was built at, kept so
    /// the Timeline's Sound mix row can summarise the mix ([`mix_peaks::mix_peaks`])
    /// without a second mixer. Goes with `loaded_comp`.
    loaded_plan: Option<(Arc<MixPlan>, u32)>,
    /// The document the last [`scrub`] asked for a mix of. A scrub arrives
    /// once per frame the playhead crosses, and only the first one after an
    /// edit has anything new to prepare. Weak, so remembering it does not
    /// keep a document alive that nothing else is holding.
    scrub_doc: std::sync::Weak<lumit_core::model::Document>,
    /// The burst a [`scrub`] asked for while the mix for it was still being
    /// prepared: where, and how long. Sounded once that mix lands, so a scrub
    /// straight after an edit is never the mix from before it.
    scrub_held: Option<(f64, f64)>,
}

impl AudioState {
    fn new() -> Self {
        Self {
            device: Device::Untried,
            wanted_device: None,
            device_gen: 0,
            loaded_comp: None,
            loaded_sig: None,
            playing: false,
            muted: false,
            slow: 1,
            offset_ms: 0,
            pending_start: None,
            worker_busy: false,
            wanted_preview: None,
            loaded_preview: None,
            pending_prepare: None,
            jobs: AudioJobsBuilder::new(),
            bakes: lumit_render::mix::live::RackBakes::default(),
            starved: 0,
            meter_strips: Vec::new(),
            loaded_plan: None,
            scrub_doc: std::sync::Weak::new(),
            scrub_held: None,
        }
    }
}

/// The session audio state — its OWN lock, separate from the document lock and
/// the renderer lock, so audio bookkeeping never waits on an edit or a render.
static AUDIO: OnceLock<Mutex<AudioState>> = OnceLock::new();

fn lock() -> std::sync::MutexGuard<'static, AudioState> {
    AUDIO
        .get_or_init(|| Mutex::new(AudioState::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Place buffers already in memory on the comp strip as a live [`MixPlan`],
/// and say which mixer strip each meter slot belongs to: [`strips`]'s answer,
/// and what a plan's `meter` indices point into.
///
/// [`lumit_render::mix::live::live_plan`] over whole buffers, which is how the
/// tests hold the plan against the export. A job whose item is not in
/// `decoded` contributes nothing. The mix the engine plays is built by the
/// same function over the files themselves ([`prepare_once`]).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_plan(
    jobs: &[AudioJob],
    decoded: &HashMap<Uuid, Arc<lumit_media::audio::AudioBuffer>>,
    rate: u32,
    duration_s: f64,
    master_db: f64,
) -> (Arc<MixPlan>, Vec<Uuid>) {
    lumit_render::mix::live::live_plan(
        jobs,
        &|job| {
            decoded
                .get(&job.item)
                .map(|b| lumit_audio::stream::Source::whole(Arc::clone(b)))
        },
        rate,
        duration_s,
        master_db,
        &mut lumit_render::mix::live::RackBakes::default(),
    )
}

/// What a rack's sound depends on that its job does not carry: which plugins
/// are switched off for now (AP5), and whether the Voice addon is installed.
/// A change here empties the bakes, so flicking the switch runs the racks
/// again.
fn rack_stamp() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Ok(disabled) = lumit_aplug::ipc::broker::session_disabled().lock() {
        format!("{disabled:?}").hash(&mut h);
    }
    lumit_render::voice::installed().hash(&mut h);
    h.finish()
}

/// One strip's bars, or the master's — see [`meters`].
pub(crate) struct Strip {
    /// The layer whose row this is; `None` for the master.
    pub layer: Option<Uuid>,
    pub reading: lumit_audio::meter::MeterReading,
}

/// What the mix is doing right now: one entry per sounding strip in the order
/// the mixer draws them, then the master.
///
/// Polled at UI rate, so it takes the state lock for a moment and reads
/// lock-free atomics — no allocation beyond the answer itself, and nothing
/// the audio callback can wait on. With no device or nothing loaded the list
/// is empty, which is a mixer with no strips rather than a fault.
pub(crate) fn meters() -> Vec<Strip> {
    let st = lock();
    let Device::Ready { meters, .. } = &st.device else {
        return Vec::new();
    };
    let mut out: Vec<Strip> = st
        .meter_strips
        .iter()
        .enumerate()
        .map(|(slot, layer)| Strip {
            layer: Some(*layer),
            reading: meters.read(slot),
        })
        .collect();
    out.push(Strip {
        layer: None,
        reading: meters.read(lumit_audio::meter::MASTER),
    });
    out
}

/// Put every clip light out (docs/09 §3.1): the desk's "I have seen it".
pub(crate) fn reset_clip() {
    let st = lock();
    if let Device::Ready { meters, .. } = &st.device {
        meters.reset_clip();
    }
}

/// Resolve the audio device, building the engine on its own thread on first
/// use. Called only from the prepare worker (so at most one build races
/// nothing). `None` on the calm terminal no-device state.
/// The third answer is the generation the stream was opened under — see
/// [`AudioState::device_gen`].
fn ensure_device() -> Option<(Sender<Cmd>, u32, u64)> {
    // Fast path under the lock, which also reads the chosen output — the id is
    // taken here rather than on the audio thread so the lock is never held
    // across the device probe below.
    let (wanted, generation) = {
        let st = lock();
        match &st.device {
            Device::Unavailable => return None,
            Device::Ready { tx, rate, .. } => {
                return Some((tx.clone(), *rate, st.device_gen));
            }
            Device::Untried => (st.wanted_device.clone(), st.device_gen),
        }
    };
    // Open it without the lock.
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    match engine::open(wanted, rx) {
        Some((clock, meters, rate)) => {
            let mut st = lock();
            // The choice moved again while this stream was opening: let the
            // fresh one win rather than installing a stale engine over it.
            if st.device_gen != generation {
                return None;
            }
            st.device = Device::Ready {
                tx: tx.clone(),
                clock,
                meters,
                rate,
            };
            // A fresh stream starts unmuted and at the mix's own speed:
            // carry the mute and the speed over to it.
            if st.muted {
                let _ = tx.send(Cmd::Mute(true));
            }
            let _ = tx.send(Cmd::Slow(st.slow));
            Some((tx, rate, generation))
        }
        None => {
            let mut st = lock();
            if st.device_gen == generation {
                st.device = Device::Unavailable;
            }
            None
        }
    }
}

/// Send a command to the audio thread if it is live. Callers hold the state
/// lock; the send is non-blocking (an unbounded channel).
fn send(st: &AudioState, cmd: Cmd) {
    if let Device::Ready { tx, .. } = &st.device {
        let _ = tx.send(cmd);
        #[cfg(target_arch = "wasm32")]
        crate::turns::wake();
    }
}

/// Kick the prepare worker for `comp` (or park the request in the mailbox
/// while one runs). Callers hold the state lock.
fn kick_prepare(st: &mut AudioState, comp: Uuid, doc: Arc<lumit_core::model::Document>) {
    if matches!(st.device, Device::Unavailable) {
        return; // calm terminal state: no sound, no worker
    }
    if st.worker_busy {
        st.pending_prepare = Some((comp, doc));
        return;
    }
    st.worker_busy = true;
    engine::spawn(move || run_prepare(comp, doc));
}

/// The prepare worker: build and install one comp's mix, then drain the
/// mailbox. Never holds the audio lock across a probe, decode, or mix.
fn run_prepare(mut comp: Uuid, mut doc: Arc<lumit_core::model::Document>) {
    loop {
        prepare_once(comp, &doc);
        let next = {
            let mut st = lock();
            match st.pending_prepare.take() {
                Some(next) => Some(next),
                None => {
                    st.worker_busy = false;
                    // The mix a scrub was waiting on is in, if it is going
                    // to be: sound where the playhead got to meanwhile.
                    if let Some((at, len)) = st.scrub_held.take() {
                        if st.loaded_comp == Some(comp) {
                            send(&st, Cmd::Scrub { at, len });
                        }
                    }
                    None
                }
            }
        };
        match next {
            Some((next_comp, next_doc)) => {
                comp = next_comp;
                doc = next_doc;
            }
            None => break,
        }
    }
}

/// One prepare pass for `comp`: jobs → signature gate → decode → plan →
/// install (swap when the same comp is already loaded, else load with the
/// pending start + transport intent).
fn prepare_once(comp: Uuid, doc: &Arc<lumit_core::model::Document>) {
    let Some(c) = doc.comp(comp) else {
        // The comp is gone (deleted, undone): silence its mix if loaded.
        let mut st = lock();
        if st.loaded_comp == Some(comp) {
            send(&st, Cmd::Unload);
            st.loaded_comp = None;
            st.loaded_sig = None;
            st.meter_strips.clear();
            st.loaded_plan = None;
        }
        return;
    };
    let duration_s = c.duration.0.to_f64();
    let master_db = c.master_volume_db;

    // The jobs walk probes files, so it runs with the builder taken OUT of the
    // state — the lock is never held across disk work. One worker at a time
    // (worker_busy), so nothing else misses the builder meanwhile.
    let mut builder = {
        let mut st = lock();
        std::mem::take(&mut st.jobs)
    };
    let jobs = builder.audio_jobs(doc, c);
    {
        let mut st = lock();
        st.jobs = builder;
    }

    if jobs.is_empty() {
        // A silent comp: unload its mix if we held one (Dart falls back to
        // the wall clock, exactly as egui's AudioSync::Silence path does).
        let mut st = lock();
        if st.loaded_comp == Some(comp) {
            send(&st, Cmd::Unload);
            st.loaded_comp = None;
            st.loaded_sig = None;
            st.meter_strips.clear();
            st.loaded_plan = None;
        }
        return;
    }
    let sig = jobs_signature(&jobs, duration_s, master_db) ^ lumit_render::voice::stamp();
    {
        let st = lock();
        if st.loaded_comp == Some(comp) && st.loaded_sig == Some(sig) {
            return; // this exact mix is already playing — a no-op
        }
    }

    // Resolve the device (first use builds the engine; may block this worker
    // briefly — never a caller).
    let Some((tx, rate, generation)) = ensure_device() else {
        return;
    };

    // The plan names its files and decodes none of them: the engine reads
    // the blocks near the playhead as it plays. Only a job carrying a rack is
    // read here, and only if the last build did not already run that rack.
    // Built without the lock; the bakes are taken out of the state for it, as
    // the jobs builder was.
    let mut bakes = {
        let mut st = lock();
        std::mem::take(&mut st.bakes)
    };
    bakes.stamped(rack_stamp());
    let pool = lumit_audio::stream::pool();
    let (plan, strips) = lumit_render::mix::live::live_plan(
        &jobs,
        &|job| Some(pool.source(&job.sound, rate)),
        rate,
        duration_s,
        master_db,
        &mut bakes,
    );
    // A mix about to start somewhere has that place decoded before it is
    // handed over, so it starts with its sound. This worker may wait; the
    // audio thread it hands to may not.
    let pending = lock().pending_start;
    if let Some(start) = pending {
        let from = (start * f64::from(rate)) as usize;
        plan.warm(from, from.saturating_add(rate as usize));
    }

    // Install: swap keeps the clock and play state (the instant-edit
    // contract); a fresh load applies the pending start and transport intent.
    let mut st = lock();
    st.bakes = bakes;
    // Unless the output device changed while this mix was being built. `tx`
    // then speaks to an engine that is being closed, and recording the comp as
    // loaded would leave the transport thinking it can hear something it
    // cannot. Drop the work; the next prepare opens the new device.
    if st.device_gen != generation {
        return;
    }
    if st.loaded_comp == Some(comp) {
        let _ = tx.send(Cmd::Swap(Arc::clone(&plan)));
    } else {
        let start = st.pending_start.take();
        let _ = tx.send(Cmd::Load {
            plan: Arc::clone(&plan),
            start,
            play: st.playing,
        });
    }
    st.loaded_comp = Some(comp);
    st.loaded_sig = Some(sig);
    st.meter_strips = strips;
    st.loaded_plan = Some((plan, rate));
}

/// Build (or refresh) `comp`'s mix in the background — called after an edit
/// while audio is loaded or playing. Returns immediately; an unchanged mix is a
/// no-op, a changed one is swapped in mid-playback.
pub(crate) fn prepare(comp: Uuid, doc: Arc<lumit_core::model::Document>) {
    let mut st = lock();
    kick_prepare(&mut st, comp, doc);
}

/// Start playback of `comp`'s audio from `start` seconds.
///
/// When its mix is already loaded this is an immediate seek and play; otherwise
/// the mix is prepared in the background and starts from `start` when it lands,
/// and the picture runs on the caller's own clock until then.
///
/// `slow` is how many times slower than the comp it plays, 1 for its own
/// speed, with its pitch kept: the Viewer's playback speed.
pub(crate) fn play(comp: Uuid, start: f64, doc: Arc<lumit_core::model::Document>, slow: u32) {
    let mut st = lock();
    st.playing = true;
    st.slow = slow;
    send(&st, Cmd::Slow(slow));
    // The comp is what the user wants to hear now, so a preview still decoding
    // must not land on top of it.
    st.wanted_preview = None;
    if st.loaded_comp == Some(comp) {
        send(&st, Cmd::Seek(start.max(0.0)));
        send(&st, Cmd::Play);
        return;
    }
    // A different comp's mix (or none) is in the engine: silence it now and
    // chase the wanted comp. Until the fresh load lands, `loaded` reads false
    // and the caller keeps its wall clock — never another comp's clock.
    if st.loaded_comp.is_some() {
        send(&st, Cmd::Unload);
        st.loaded_comp = None;
        st.loaded_sig = None;
        st.meter_strips.clear();
        st.loaded_plan = None;
    }
    st.pending_start = Some(start.max(0.0));
    kick_prepare(&mut st, comp, doc);
}

/// Play one footage file's own sound from `start_s` seconds into it, with no
/// composition behind it: the Project panel's preview button (docs/07 §3.1)
/// from the top, and the Source viewer from wherever its playhead is.
///
/// `sound` is the file and which of its audio streams are heard, mixed.
///
/// [`clock`] then reads **the file's own time**, seconds from the top of the
/// file, and so starts at `start_s` and not at zero. A start past the end of
/// the file is held to the end, and one that is not a number is the top.
///
/// The mix is a single clip at unity, loaded under the **item's** id. A footage
/// id is never a comp id, so a preview and a comp's mix cannot be taken for one
/// another, and starting either silences the other — which is what one pair of
/// speakers means. Stopping is [`stop`] and where it has got to is [`clock`],
/// exactly as for a comp.
///
/// Returns at once: the file is opened on its own thread, so a slow disk does
/// not hold the press.
pub(crate) fn preview(item: Uuid, sound: lumit_media::audio::SoundSource, start_s: f64) {
    // `max` answers zero for a start that is not a number.
    let start_s = start_s.max(0.0);
    {
        let mut st = lock();
        if matches!(st.device, Device::Unavailable) {
            return;
        }
        st.wanted_preview = Some(item);
        st.slow = 1;
        send(&st, Cmd::Slow(1));
        // This file's preview is already in the engine: a second press is a
        // seek, not a second look at the file.
        if st.loaded_comp == Some(item) && st.loaded_preview.as_ref() == Some(&sound) {
            st.playing = true;
            let length = st
                .loaded_plan
                .as_ref()
                .map(|(plan, rate)| plan.total_frames as f64 / f64::from(*rate));
            send(&st, Cmd::Seek(length.map_or(start_s, |l| start_s.min(l))));
            send(&st, Cmd::Play);
            return;
        }
    }

    engine::spawn(move || {
        let Some((tx, rate, generation)) = ensure_device() else {
            return;
        };

        // The same source a comp's mix reads: previewing a clip and then
        // dropping it in a comp decodes each block of it once.
        let source = lumit_audio::stream::pool().source(&sound, rate);
        // How long the preview is, is how long the file is. Finding that
        // reads the end of the file and not the whole of it.
        let frames = source.frames_exactly();
        if frames == 0 {
            return; // a file with no sound in it, or one that will not open
        }
        let duration_s = frames as f64 / f64::from(rate);

        let jobs = vec![AudioJob {
            item,
            layer: item,
            sound: sound.clone(),
            in_s: 0.0,
            out_s: duration_s,
            offset_s: 0.0,
            volume: lumit_core::anim::Property::zero(),
            pan: lumit_core::anim::Property::zero(),
            carriers: Vec::new(),
            fade: None,
            driven: None,
            chain: None,
            clip: None,
            clip_chain: None,
            retime: None,
            scope: None,
        }];
        let (plan, strips) = lumit_render::mix::live::live_plan(
            &jobs,
            &|_| Some(Arc::clone(&source)),
            rate,
            duration_s,
            0.0,
            &mut lumit_render::mix::live::RackBakes::default(),
        );
        // Decoded where it is about to start, so the press is heard from its
        // first sample.
        let start_s = start_s.min(duration_s);
        let from = (start_s * f64::from(rate)) as usize;
        plan.warm(from, from.saturating_add(rate as usize));

        let mut st = lock();
        // The output moved, or something else took the engine while this file
        // was opened (a comp played, or another preview was pressed). Drop the
        // work rather than stamping it over what the user is listening to now.
        if st.device_gen != generation || st.wanted_preview != Some(item) {
            return;
        }
        let _ = tx.send(Cmd::Load {
            plan: Arc::clone(&plan),
            start: Some(start_s),
            play: true,
        });
        st.loaded_comp = Some(item);
        st.loaded_preview = Some(sound);
        // No signature: a preview is not a comp, so no edit can arrive that
        // would want to be recognised as the same mix.
        st.loaded_sig = None;
        st.playing = true;
        st.meter_strips = strips;
        st.loaded_plan = Some((plan, rate));
    });
}

/// Pause playback (the transport's pause — the clock holds its position).
pub(crate) fn pause() {
    let mut st = lock();
    st.playing = false;
    send(&st, Cmd::Pause);
}

/// The monitor mute. Remembered before any engine exists and re-applied to
/// every stream opened after, so it survives a change of output.
pub(crate) fn set_muted(muted: bool) {
    let mut st = lock();
    st.muted = muted;
    send(&st, Cmd::Mute(muted));
}

pub(crate) fn muted() -> bool {
    lock().muted
}

pub(crate) fn set_offset_ms(ms: i32) {
    lock().offset_ms = ms;
}

/// The same, in seconds: what playback adds when it puts the sound under a
/// picture, so the two still meet once the sound is heard.
pub(crate) fn offset_seconds() -> f64 {
    f64::from(lock().offset_ms) / 1000.0
}

/// Start the sound again where it stopped, with no re-bake and no seek — the
/// other half of [`pause`].
///
/// Every-frame playback stops the sound when the picture falls behind (a held
/// track over a slow picture, never a track that drifts away from it).
/// The picture then catches up, and the sound must start again on its own; a
/// user who must stop and start playback to get the sound back has been given a
/// fault to work around. Nothing is re-prepared here: the plan and the position
/// are as they were, thus this is one atomic and one message.
pub(crate) fn resume() {
    let mut st = lock();
    // Nothing loaded means nothing to start; `play` does the preparing.
    if st.loaded_comp.is_none() {
        return;
    }
    st.playing = true;
    send(&st, Cmd::Play);
}

/// Move the audio clock to `secs` (a scrub; play state is untouched).
pub(crate) fn seek(secs: f64) {
    let st = lock();
    send(&st, Cmd::Seek(secs.max(0.0)));
}

/// How long one scrub burst sounds, in seconds, unless the comp's frame is
/// longer. A twelfth of a second is what After Effects plays for each move of
/// its playhead: long enough to tell a kick from a snare, short enough that
/// the beat is heard on the frame it falls on and not three frames early.
const SCRUB_BURST_S: f64 = 1.0 / 12.0;

/// Sound `comp`'s mix at `frame` in one short burst, with the transport
/// stopped: the playhead dragged with Ctrl held.
///
/// When the mix is loaded this is one message to the audio thread. When it is
/// not, it is prepared in the background and the bursts start once it lands,
/// so the first drag of a session is silent for as long as the decode takes.
/// The first burst after an edit prepares again too, so scrubbing straight
/// after moving a clip is never the mix from before the move.
pub(crate) fn scrub(comp: Uuid, frame: u64, doc: Arc<lumit_core::model::Document>) {
    let Some(fps) = doc
        .comp(comp)
        .map(|c| c.frame_rate.fps())
        .filter(|fps| *fps > 0.0)
    else {
        return;
    };
    let mut st = lock();
    // The comp is what the user wants to hear now, as in `play`.
    st.wanted_preview = None;
    let loaded = st.loaded_comp == Some(comp);
    let (at, len) = (frame as f64 / fps, (1.0 / fps).max(SCRUB_BURST_S));
    let seen = Arc::downgrade(&doc);
    let mut changed = !st.scrub_doc.ptr_eq(&seen);
    if !loaded {
        // The transport stays stopped, so this comp's mix does not start
        // playing by itself when it lands.
        st.playing = false;
        if st.loaded_comp.is_some() {
            // Another comp's mix, or a footage preview, is in the engine.
            // Silence it, and prepare this one whatever was asked before.
            send(&st, Cmd::Unload);
            st.loaded_comp = None;
            st.loaded_sig = None;
            st.meter_strips.clear();
            st.loaded_plan = None;
            changed = true;
        }
    }
    // The mix for this document is in the engine: one message. Otherwise it
    // is on its way, or about to be, and the burst waits for it. Only a new
    // document has anything to prepare, so a comp with no sound in it is
    // asked about once and not on every frame the playhead crosses.
    if loaded && !changed && st.scrub_held.is_none() {
        send(&st, Cmd::Scrub { at, len });
    } else if changed || st.scrub_held.is_some() {
        st.scrub_held = Some((at, len));
    }
    if changed {
        st.scrub_doc = seen;
        kick_prepare(&mut st, comp, doc);
    }
}

/// Stop: pause and rewind to the start (the transport's stop semantics).
pub(crate) fn stop() {
    let mut st = lock();
    st.playing = false;
    // Stop means stop: a preview whose file is still decoding does not then
    // start on its own a second later.
    st.wanted_preview = None;
    send(&st, Cmd::Pause);
    send(&st, Cmd::Seek(0.0));
}

/// What the machine offers, the id actually in use, and whether that is a
/// fallback — see [`crate::api::audio::list_audio_devices`], which is the only
/// caller and the place the meaning is written down.
pub(crate) fn devices() -> (lumit_audio::OutputDevices, Option<String>, bool) {
    // The enumeration is a sound-system call, so the lock is taken only to
    // read the chosen id, and released before anything slow happens.
    let wanted = lock().wanted_device.clone();
    let list = lumit_audio::output_devices();
    let active = list.resolve(wanted.as_deref());
    // Only a fallback when something else is actually being played through: a
    // machine with no output at all has no default to fall back *to*, and says
    // so by having no active id rather than by claiming a substitution.
    let fell_back = wanted.is_some() && active.is_some() && active != wanted;
    (list, active, fell_back)
}

/// Play through `id` from now on (`None` for the system default).
///
/// The cpal stream cannot be moved between devices, so the open one is closed:
/// dropping the last sender ends the audio thread, the stream goes with it, and
/// the next prepare opens the new device. Sound stops until then, which is what
/// changing where the sound comes out means. A no-op when the choice has not
/// actually changed, so the frontend can hand this over on every boot.
pub(crate) fn set_device(id: Option<String>) {
    let mut st = lock();
    if st.wanted_device == id {
        return;
    }
    st.wanted_device = id;
    st.device_gen = st.device_gen.wrapping_add(1);
    st.playing = false;
    st.loaded_comp = None;
    st.loaded_sig = None;
    st.meter_strips.clear();
    st.loaded_plan = None;
    st.pending_start = None;
    // Untried rather than Unavailable even when the last attempt found nothing:
    // choosing a device is exactly the moment to look again.
    st.device = Device::Untried;
}

/// The playback clock: `(seconds, is_playing, loaded)`.
///
/// Allocation-free - the state lock plus two atomic reads - because it is
/// polled every tick. With no device, or nothing loaded, it reads
/// `(0.0, false, false)` and the caller keeps its own clock.
pub(crate) fn clock() -> (f64, bool, bool) {
    let mut st = lock();
    let (answer, starved) = match &st.device {
        Device::Ready { clock, .. } => {
            let loaded = st.loaded_comp.is_some();
            let playing = clock.is_playing();
            // Only while playing: a stopped clock is where the playhead sits.
            // The sound heard now left the mix that long ago, which is less
            // of the mix when it is slowed.
            let late = if playing {
                f64::from(st.offset_ms) / 1000.0 / f64::from(st.slow.max(1))
            } else {
                0.0
            };
            (
                (clock.seconds() - late, playing, loaded),
                clock.starved_frames(),
            )
        }
        _ => return (0.0, false, false),
    };
    // The engine plays silence where a block was not decoded in time and
    // counts the frames. Said here, once per shortfall, because this is the
    // one place that already looks at the engine every tick: nothing is
    // written unless the count has moved.
    // ponytail: a line on the error stream, which a release build's user
    // never sees. The status readout wants this, and reaching it takes a
    // field on the clock the frontend polls.
    if starved != st.starved {
        let rate = match &st.device {
            Device::Ready { rate, .. } => f64::from(*rate),
            _ => 48_000.0,
        };
        note!(
            "sound: {:.0} ms played as silence near {:.2} s, the decode fell behind the playhead",
            (starved.saturating_sub(st.starved)) as f64 * 1000.0 / rate,
            answer.0
        );
        st.starved = starved;
    }
    answer
}

#[cfg(test)]
mod tests;
