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

use lumit_audio::plan::MixPlan;
use lumit_core::model::Document;
use lumit_render::headless::AudioJobsBuilder;
use lumit_render::mix::graph::{build_graph, GraphInfo, GraphPrev};
use lumit_render::mix::jobs::AudioJob;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

pub(crate) mod automation;
mod engine;
pub(crate) mod loudness;
pub(crate) mod measure;
pub(crate) mod mix_peaks;
pub(crate) mod positions;
pub(crate) mod preview;
pub(crate) mod record;
pub(crate) mod scrub;
mod signature;

use signature::{desk_signature, jobs_signature};

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
    /// Seek and play, the sound there made ready first so it is heard from
    /// its first frame ([`lumit_audio::AudioEngine::start_at`]).
    Start(f64),
    /// Sound `len` seconds from `at` once, with the transport stopped.
    Scrub {
        at: f64,
        len: f64,
    },
    /// Play toward this many seconds at the speed of the drag.
    Jog(f64),
    /// The drag is over.
    JogEnd,
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
    /// With the edit count it was asked at ([`automation::edits`]).
    pending_prepare: Option<(Uuid, Arc<Document>, u64)>,
    /// The comp and document the running build is of, and the flag that
    /// stops it once a newer mix is wanted.
    build: Option<(Uuid, std::sync::Weak<Document>, Arc<AtomicBool>)>,
    /// The audio-jobs walk with its has-audio probe cache. Taken out of the
    /// state (never probed under the lock) by the worker and put back.
    jobs: AudioJobsBuilder,
    /// What the racks made of each racked job in the plan that is loaded, so
    /// the next build runs only the racks an edit changed.
    bakes: lumit_render::mix::live::RackBakes,
    /// How much sound the engine had played as silence when it was last
    /// asked, so each shortfall is noted once ([`clock`]).
    starved: u64,
    /// How much sound has been played as silence, in milliseconds, every
    /// engine counted, and how many times the output had to be opened again
    /// because the one in use went away.
    starved_ms: u32,
    device_changes: u32,
    /// The plan the engine is playing and what it was built from. Goes with
    /// `loaded_comp`; a node's meter slot is its place in the plan.
    loaded_plan: Option<Loaded>,
    /// The faders held from the mixer and the automation pass being written.
    live: automation::Live,
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
            build: None,
            jobs: AudioJobsBuilder::new(),
            bakes: lumit_render::mix::live::RackBakes::default(),
            starved: 0,
            starved_ms: 0,
            device_changes: 0,
            loaded_plan: None,
            live: automation::Live::default(),
            scrub_doc: std::sync::Weak::new(),
            scrub_held: None,
        }
    }
}

/// The mix the engine holds, and what it was built from.
#[derive(Clone)]
pub(crate) struct Loaded {
    pub plan: Arc<MixPlan>,
    pub rate: u32,
    /// What each node of the plan is, in the plan's order.
    pub info: Arc<GraphInfo>,
    /// The document the plan was built from, where each fader is read now.
    /// None for a footage preview.
    pub doc: Option<Arc<Document>>,
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

/// Build the comp's desk over buffers already in memory, which is how the
/// tests hold the plan against the export. A job whose item is not in
/// `decoded` contributes nothing. The mix the engine plays is built by the
/// same function over the files themselves ([`prepare_once`]).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_plan(
    doc: &Arc<Document>,
    comp: &lumit_core::model::Composition,
    jobs: &[AudioJob],
    decoded: &HashMap<Uuid, Arc<lumit_media::audio::AudioBuffer>>,
    rate: u32,
    duration_s: f64,
) -> (Arc<MixPlan>, GraphInfo) {
    let built = build_graph(
        doc,
        comp,
        jobs.to_vec(),
        &|job| {
            decoded
                .get(&job.item)
                .map(|b| lumit_audio::stream::Source::whole(Arc::clone(b)))
        },
        rate,
        duration_s,
        &mut lumit_render::mix::live::RackBakes::default(),
        None,
        false,
        &AtomicBool::new(false),
        None,
    );
    let (plan, info) = built.unwrap_or_default();
    (Arc::new(plan), info)
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

/// One strip's bars, a bus's, or the master's — see [`meters`].
pub(crate) struct Strip {
    /// The layer whose row this is; `None` for the master.
    pub layer: Option<Uuid>,
    /// A Bus layer's row.
    pub bus: bool,
    pub reading: lumit_audio::meter::MeterReading,
}

/// What the mix is doing right now: one entry per node of the plan, strips
/// and buses in the plan's order, then the master.
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
    let nodes = st
        .loaded_plan
        .as_ref()
        .map_or(&[][..], |l| &l.info.nodes[..]);
    let mut out: Vec<Strip> = nodes
        .iter()
        .take(lumit_audio::plan::MAX_NODES)
        .enumerate()
        .map(|(slot, node)| Strip {
            layer: Some(node.layer),
            bus: node.bus,
            reading: meters.read(slot),
        })
        .collect();
    out.push(Strip {
        layer: None,
        bus: false,
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
fn kick_prepare(st: &mut AudioState, comp: Uuid, doc: Arc<Document>) {
    if matches!(st.device, Device::Unavailable) {
        return; // calm terminal state: no sound, no worker
    }
    let seq = automation::edits();
    if st.worker_busy {
        // A mix of something else is wanted now: a long bake of the old one
        // is abandoned rather than holding it up. Asking again for the same
        // document, as the mix row does on every poll, stops nothing.
        if let Some((building, of, cancel)) = &st.build {
            if *building != comp || of.as_ptr() != Arc::as_ptr(&doc) {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        st.pending_prepare = Some((comp, doc, seq));
        return;
    }
    st.worker_busy = true;
    engine::spawn(move || run_prepare(comp, doc, seq));
}

/// Stop the build running now, if there is one: the engine it was for is
/// closing.
fn stop_build(st: &AudioState) {
    if let Some((_, _, cancel)) = &st.build {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Forget the mix the engine holds: which comp, its signature, its plan and
/// the controls held on it. A pass being recorded is written first.
fn forget(st: &mut AudioState) {
    automation::end_pass(st);
    automation::clear(st);
    st.loaded_comp = None;
    st.loaded_sig = None;
    st.loaded_plan = None;
}

/// Where the sound is heard now, in comp seconds, and whether it is playing.
/// None with no engine.
fn heard(st: &AudioState) -> Option<(f64, bool)> {
    let Device::Ready { clock, .. } = &st.device else {
        return None;
    };
    let playing = clock.is_playing();
    // Only while playing: a stopped clock is where the playhead sits. The
    // sound heard now left the mix that long ago, which is less of the mix
    // when it is slowed.
    let late = if playing {
        f64::from(st.offset_ms) / 1000.0 / f64::from(st.slow.max(1))
    } else {
        0.0
    };
    Some((clock.seconds() - late, playing))
}

/// The prepare worker: build and install one comp's mix, then drain the
/// mailbox. Never holds the audio lock across a probe, decode, or mix.
fn run_prepare(mut comp: Uuid, mut doc: Arc<Document>, mut seq: u64) {
    loop {
        prepare_once(comp, &doc, seq);
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
            Some((next_comp, next_doc, next_seq)) => {
                comp = next_comp;
                doc = next_doc;
                seq = next_seq;
            }
            None => break,
        }
    }
}

/// One prepare pass for `comp`: jobs → signature gate → the desk → install
/// (swap when the same comp is already loaded, else load with the pending
/// start + transport intent). `seq` is the edit count the request was made
/// at: a fader held until its edit's mix lands is let go here.
fn prepare_once(comp: Uuid, doc: &Arc<Document>, seq: u64) {
    let Some(c) = doc.comp(comp) else {
        // The comp is gone (deleted, undone): silence its mix if loaded.
        let mut st = lock();
        if st.loaded_comp == Some(comp) {
            send(&st, Cmd::Unload);
            forget(&mut st);
        }
        return;
    };
    let duration_s = c.duration.0.to_f64();

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
            forget(&mut st);
        }
        return;
    }
    let sig = jobs_signature(&jobs, duration_s, c.master_volume_db)
        ^ desk_signature(c).rotate_left(1)
        ^ lumit_render::voice::stamp();
    {
        let mut st = lock();
        if st.loaded_comp == Some(comp) && st.loaded_sig == Some(sig) {
            // This exact mix is already playing — a no-op, and the plan in
            // the engine already sounds as this document does. What it does
            // not hear (an automation mode) is read from this document now.
            if let Some(loaded) = st.loaded_plan.as_mut() {
                loaded.doc = Some(Arc::clone(doc));
            }
            automation::landed(&mut st, seq);
            return;
        }
    }

    // Resolve the device (first use builds the engine; may block this worker
    // briefly — never a caller).
    let Some((tx, rate, generation)) = ensure_device() else {
        return;
    };

    // The plan names its files and decodes none of them: the engine reads
    // the blocks near the playhead as it plays. Only a rack that has to hear
    // its whole input, or a clip's rack, is run here, and only if the last
    // build did not already run it. Built without the lock; the bakes are
    // taken out of the state for it, as the jobs builder was, and the plan it
    // replaces is lent so its racks run on and its held faders stay held.
    let cancel = Arc::new(AtomicBool::new(false));
    let (mut bakes, prev) = {
        let mut st = lock();
        st.build = Some((comp, Arc::downgrade(doc), Arc::clone(&cancel)));
        let prev = st
            .loaded_plan
            .clone()
            .filter(|l| st.loaded_comp == Some(comp) && l.rate == rate);
        (std::mem::take(&mut st.bakes), prev)
    };
    bakes.stamped(rack_stamp());
    let pool = lumit_audio::stream::pool();
    let built = build_graph(
        doc,
        c,
        jobs,
        &|job| Some(pool.source(&job.sound, rate)),
        rate,
        duration_s,
        &mut bakes,
        prev.as_ref().map(|p| GraphPrev {
            plan: &p.plan,
            info: &p.info,
        }),
        false,
        &cancel,
        None,
    );
    let Ok((plan, info)) = built else {
        // A newer mix is on its way: nothing is swapped and the plan playing
        // plays on.
        let mut st = lock();
        st.bakes = bakes;
        st.build = None;
        return;
    };
    let plan = Arc::new(plan);
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
    st.build = None;
    // Unless the output device changed while this mix was being built. `tx`
    // then speaks to an engine that is being closed, and recording the comp as
    // loaded would leave the transport thinking it can hear something it
    // cannot. Drop the work; the next prepare opens the new device.
    if st.device_gen != generation {
        return;
    }
    let loaded = Loaded {
        plan: Arc::clone(&plan),
        rate,
        info: Arc::new(info),
        doc: Some(Arc::clone(doc)),
    };
    if st.loaded_comp == Some(comp) {
        // Held faders carry over here, where the plan is installed, so one
        // let go while the build ran is not left held on the new plan.
        if let Some(old) = &st.loaded_plan {
            plan.seed_live(&old.plan);
        }
        st.loaded_sig = Some(sig);
        st.loaded_plan = Some(loaded);
        automation::landed(&mut st, seq);
        let _ = tx.send(Cmd::Swap(plan));
    } else {
        if st.loaded_comp.is_some() {
            forget(&mut st);
        }
        let start = st.pending_start.take();
        let _ = tx.send(Cmd::Load {
            plan,
            start,
            play: st.playing,
        });
        st.loaded_comp = Some(comp);
        st.loaded_sig = Some(sig);
        st.loaded_plan = Some(loaded);
        if st.playing {
            automation::start_pass(&mut st, start.unwrap_or(0.0), None);
        }
    }
}

/// Build (or refresh) `comp`'s mix in the background — called after an edit
/// while audio is loaded or playing. Returns immediately; an unchanged mix is a
/// no-op, a changed one is swapped in mid-playback.
pub(crate) fn prepare(comp: Uuid, doc: Arc<Document>) {
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
pub(crate) fn play(comp: Uuid, start: f64, doc: Arc<Document>, slow: u32) {
    let mut st = lock();
    st.playing = true;
    st.slow = slow;
    send(&st, Cmd::Slow(slow));
    // The comp is what the user wants to hear now, so a preview still decoding
    // must not land on top of it.
    st.wanted_preview = None;
    if st.loaded_comp == Some(comp) {
        // A start from somewhere else, a loop going round: the pass being
        // written ends there and a new one begins.
        automation::end_pass(&mut st);
        send(&st, Cmd::Start(start.max(0.0)));
        automation::start_pass(&mut st, start.max(0.0), Some(&doc));
        return;
    }
    // A different comp's mix (or none) is in the engine: silence it now and
    // chase the wanted comp. Until the fresh load lands, `loaded` reads false
    // and the caller keeps its wall clock — never another comp's clock.
    if st.loaded_comp.is_some() {
        send(&st, Cmd::Unload);
        forget(&mut st);
    }
    st.pending_start = Some(start.max(0.0));
    kick_prepare(&mut st, comp, doc);
}

/// Pause playback (the transport's pause — the clock holds its position).
/// The automation pass being written ends here.
pub(crate) fn pause() {
    let mut st = lock();
    st.playing = false;
    automation::end_pass(&mut st);
    send(&st, Cmd::Pause);
}

/// Stop the sound while the picture catches up ([`resume`] starts it
/// again). Not a stop of the transport, so the pass being written goes on.
pub(crate) fn hold_for_picture() {
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

/// Move the audio clock to `secs` (a scrub; play state is untouched). The
/// pass being written ends there, and a new one begins if it is playing.
pub(crate) fn seek(secs: f64) {
    let mut st = lock();
    automation::end_pass(&mut st);
    send(&st, Cmd::Seek(secs.max(0.0)));
    if st.playing {
        automation::start_pass(&mut st, secs.max(0.0), None);
    }
}

/// Put the sound under the picture again at `secs`, which every-frame
/// playback does as it keeps the two level. The pass goes on.
pub(crate) fn seek_for_picture(secs: f64) {
    let st = lock();
    send(&st, Cmd::Seek(secs.max(0.0)));
}

/// Stop: pause and rewind to the start (the transport's stop semantics).
pub(crate) fn stop() {
    let mut st = lock();
    st.playing = false;
    automation::end_pass(&mut st);
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
    stop_build(&st);
    st.playing = false;
    forget(&mut st);
    st.pending_start = None;
    // Untried rather than Unavailable even when the last attempt found nothing:
    // choosing a device is exactly the moment to look again.
    st.device = Device::Untried;
}

/// The playback clock: `(seconds, is_playing, loaded)`.
///
/// Allocation-free - the state lock plus two atomic reads - because it is
/// polled every tick, save for the point a control held while recording
/// notes here now and then ([`automation::sample`]). With no device, or nothing loaded, it reads
/// `(0.0, false, false)` and the caller keeps its own clock.
///
/// It is also where a lost output is noticed: the engine is dropped, the
/// mix is prepared again on whatever output is there now, and it plays on
/// from where it was if it was playing.
pub(crate) fn clock() -> (f64, bool, bool) {
    let mut st = lock();
    let (answer, starved, lost) = match (&st.device, heard(&st)) {
        (Device::Ready { clock, .. }, Some((seconds, playing))) => (
            (seconds, playing, st.loaded_comp.is_some()),
            clock.starved_frames(),
            clock.device_lost(),
        ),
        _ => return (0.0, false, false),
    };
    if lost {
        reopen(&mut st);
        return (0.0, false, false);
    }
    if answer.1 {
        automation::sample(&mut st, answer.0);
    }
    // The engine plays silence where a block was not decoded in time and
    // counts the frames. Said here, once per shortfall, because this is the
    // one place that already looks at the engine every tick: nothing is
    // written unless the count has moved.
    if starved != st.starved {
        let rate = match &st.device {
            Device::Ready { rate, .. } => f64::from(*rate),
            _ => 48_000.0,
        };
        let ms = (starved.saturating_sub(st.starved)) as f64 * 1000.0 / rate;
        note!(
            "sound: {ms:.0} ms played as silence near {:.2} s, the decode fell behind the playhead",
            answer.0
        );
        st.starved_ms = st.starved_ms.saturating_add(ms.round() as u32);
        st.starved = starved;
    }
    answer
}

/// The output went away: drop the engine and prepare the comp again, which
/// opens the chosen output or the system default, starting where the clock
/// had got to and playing if it was.
fn reopen(st: &mut AudioState) {
    // Where the mix had got to, not where it was heard: the offset is the
    // picture's to apply.
    let at = match &st.device {
        Device::Ready { clock, .. } => clock.seconds().max(0.0),
        _ => 0.0,
    };
    // The newest document asked for: one waiting, one being built, else the
    // one the mix playing was built from.
    let again = st.loaded_comp.and_then(|comp| {
        let waiting = st
            .pending_prepare
            .as_ref()
            .filter(|(c, _, _)| *c == comp)
            .map(|(_, doc, _)| Arc::clone(doc));
        let building = || {
            st.build
                .as_ref()
                .filter(|(c, _, _)| *c == comp)
                .and_then(|(_, doc, _)| doc.upgrade())
        };
        let loaded = || st.loaded_plan.as_ref().and_then(|l| l.doc.clone());
        waiting
            .or_else(building)
            .or_else(loaded)
            .map(|doc| (comp, doc))
    });
    st.device_gen = st.device_gen.wrapping_add(1);
    st.device_changes = st.device_changes.saturating_add(1);
    stop_build(st);
    forget(st);
    st.device = Device::Untried;
    st.starved = 0;
    st.pending_start = None;
    if let Some((comp, doc)) = again {
        st.pending_start = Some(at);
        kick_prepare(st, comp, doc);
    }
}

/// How many times the output had to be opened again, and how much sound,
/// in milliseconds, was played as silence because it was not ready. Both
/// only grow.
pub(crate) fn health() -> (u32, u32) {
    let st = lock();
    (st.device_changes, st.starved_ms)
}

#[cfg(test)]
mod tests;
