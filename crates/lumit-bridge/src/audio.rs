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
//!   seam, so audio never queues behind a slow comp render), decode each
//!   source at the device rate (cached per item), place the clips, and hand
//!   the finished [`MixPlan`] to the audio thread. The FFI prepare call only
//!   *kicks* this worker and returns; one worker runs at a time with a
//!   one-slot latest-wins mailbox, so a burst of edits coalesces.
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
use lumit_render::export::AudioJob;
use lumit_render::headless::AudioJobsBuilder;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

/// Decoded-audio cache ceiling. When the per-item cache would exceed this,
/// items the current mix does not reference are dropped (a crude but bounded
/// budget; the egui side's byte-budgeted cache is the fuller treatment).
const DECODED_BUDGET_BYTES: usize = 512 * 1024 * 1024;

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
    Seek(f64),
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
    /// Where a fresh load should start, in seconds — set by `audio_play` when
    /// the wanted comp is not loaded yet.
    pending_start: Option<f64>,
    /// One prepare worker at a time; a request landing while it runs parks in
    /// the one-slot latest-wins mailbox.
    worker_busy: bool,
    /// The comp waiting for the worker, with the document to build it from —
    /// the snapshot is captured at request time so a later edit cannot change
    /// what a queued prepare is about to mix.
    pending_prepare: Option<(Uuid, Arc<lumit_core::Document>)>,
    /// The audio-jobs walk with its has-audio probe cache. Taken out of the
    /// state (never probed under the lock) by the worker and put back.
    jobs: AudioJobsBuilder,
    /// Decoded sources at the device rate, shared into every plan (`Arc`s, so
    /// a swap re-places without re-decoding).
    decoded: HashMap<Uuid, Arc<lumit_media::AudioBuffer>>,
    /// Which layer each meter slot of the installed plan belongs to, in the
    /// order the mixer draws its strips (K-690). Replaced with the plan, so
    /// a poll can never name a strip the sound is not on.
    meter_strips: Vec<Uuid>,
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
            pending_start: None,
            worker_busy: false,
            pending_prepare: None,
            jobs: AudioJobsBuilder::new(),
            decoded: HashMap::new(),
            meter_strips: Vec::new(),
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

/// A change-detection fingerprint of a comp's mixed audio — the bridge twin of
/// the egui side's `audio_jobs_signature`: the ordered contributing sources
/// with their placements and Volumes, plus the mix length. Any edit that
/// changes what the comp sounds like changes this; an unchanged mix is a
/// no-op. Session-only (a `DefaultHasher` is fine here — never persisted).
pub(crate) fn jobs_signature(jobs: &[AudioJob], duration_s: f64, master_db: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    jobs.len().hash(&mut h);
    duration_s.to_bits().hash(&mut h);
    master_db.to_bits().hash(&mut h);
    for j in jobs {
        j.path.hash(&mut h);
        j.in_s.to_bits().hash(&mut h);
        j.out_s.to_bits().hash(&mut h);
        j.offset_s.to_bits().hash(&mut h);
        hash_animation(&mut h, &j.volume.animation);
        hash_animation(&mut h, &j.pan.animation);
        j.carriers.len().hash(&mut h);
        for c in &j.carriers {
            c.offset_s.to_bits().hash(&mut h);
            hash_animation(&mut h, &c.volume.animation);
            hash_animation(&mut h, &c.pan.animation);
        }
        // A *Duck under* wire changes what the layer sounds like without
        // touching a keyframe, so the chain — wires and its drivers' values —
        // folds in. Debug text rather than a field-by-field walk: session-only,
        // like the rest of this hash, and a graph is a handful of nodes.
        if let Some(d) = &j.driven {
            format!("{:?}", d.graph).hash(&mut h);
        }
        // The layer's insert chain, the same way and for the same reason
        // (K-700): dropping a plugin on the row, dragging one of its knobs or
        // bypassing it all change what the comp sounds like without touching a
        // Volume keyframe, so the whole stack and its wiring fold in. Debug
        // text, like the graph above — session-only, and a stack is a handful
        // of entries.
        if let Some(c) = &j.chain {
            format!("{:?}", c.effects).hash(&mut h);
            format!("{:?}", c.graph).hash(&mut h);
            // Whether each of THIS chain's plugins is switched off folds in
            // too (AP5): flicking the switch changes what the chain opens
            // without touching the document, and the rebake this provokes is
            // what actually silences the plugin rather than only badging it.
            // This chain's plugins, not the whole list, so switching off a
            // plugin no comp uses re-plans nothing.
            if let Ok(disabled) = lumit_aplug::session_disabled().lock() {
                for effect in &c.effects {
                    let name = effect.effect.match_name.as_str();
                    if let Some(id) = name
                        .strip_prefix(lumit_core::fx::CLAP_MATCH_PREFIX)
                        .or_else(|| name.strip_prefix(lumit_core::fx::VST3_MATCH_PREFIX))
                    {
                        disabled.contains(id).hash(&mut h);
                    }
                }
            }
        }
    }
    h.finish()
}

/// Fold one Volume animation into the signature: static hashes as one f64,
/// keyframed hashes every key.
fn hash_animation(
    h: &mut std::collections::hash_map::DefaultHasher,
    a: &lumit_core::anim::Animation,
) {
    use std::hash::Hash;
    match a {
        lumit_core::anim::Animation::Static(v) => v.to_bits().hash(h),
        lumit_core::anim::Animation::Keyframed(keys) => {
            keys.len().hash(h);
            for k in keys {
                k.time.to_f64().to_bits().hash(h);
                k.value.to_bits().hash(h);
            }
        }
        lumit_core::anim::Animation::Expression(expr) => expr.hash(h),
    }
}

/// Place the decoded sources on the comp strip as a live [`MixPlan`] — pure,
/// so plan-building is a plain deterministic test. A job whose source is not
/// in `decoded` (a failed or oversized decode) contributes nothing; the same
/// placement + Volume bake the egui preview and the exporter use, so playback
/// sounds identical everywhere.
/// Which mixer strip each meter slot belongs to — [`strips`]'s answer, and
/// what a plan's `meter` indices point into.
pub(crate) fn build_plan(
    jobs: &[AudioJob],
    decoded: &HashMap<Uuid, Arc<lumit_media::AudioBuffer>>,
    rate: u32,
    duration_s: f64,
    master_db: f64,
) -> (Arc<MixPlan>, Vec<Uuid>) {
    let total_frames = (duration_s * f64::from(rate)).round().max(0.0) as usize;
    // Meter slots in first-sounding order, one per strip (K-690): several
    // jobs from one Precomp layer share its slot, and past the bank's size
    // the extras play unmetered rather than being dropped.
    let mut strips: Vec<Uuid> = Vec::new();
    let clips = jobs
        .iter()
        .filter_map(|job| {
            let buffer = decoded.get(&job.item).filter(|b| b.rate == rate)?;
            let (start_frame, src_start, len) = lumit_audio::mix::place_on_timeline(
                job.in_s,
                job.out_s,
                job.offset_s,
                buffer.samples.len() / 2,
                rate,
            )?;
            // The layer's insert chain, ahead of Volume and Pan (K-700). The
            // processed span **replaces** the decoded buffer in the plan, so
            // the realtime callback plays finished sound and never waits on a
            // plugin's process; a layer whose stack opens nothing keeps the
            // shared decoded `Arc` untouched. Realtime rather than offline
            // here, which is the one thing this path and the export's say
            // differently — the arithmetic in between is the same function.
            let wet = job.chain.as_ref().and_then(|chain| {
                lumit_render::export::chain_bake(
                    chain,
                    &buffer.samples[src_start * 2..(src_start + len) * 2],
                    start_frame,
                    rate,
                    false,
                )
            });
            let (buffer, start_frame, src_start, len) = match wet {
                // Placed the chain's summed latency earlier, so the wet lands
                // where the dry did.
                Some((samples, latency)) => {
                    let frames = samples.len() / 2;
                    (
                        Arc::new(lumit_media::AudioBuffer { rate, samples }),
                        start_frame - i64::from(latency),
                        0,
                        frames,
                    )
                }
                None => (Arc::clone(buffer), start_frame, src_start, len),
            };
            let (gain, envelope) = lumit_render::export::volume_bake(job, start_frame, len, rate);
            let slot = match strips.iter().position(|s| *s == job.layer) {
                Some(at) => at,
                None => {
                    strips.push(job.layer);
                    strips.len() - 1
                }
            };
            Some(lumit_audio::mix::PlacedClip {
                buffer,
                start_frame,
                src_start,
                len,
                gain,
                envelope: envelope.map(Arc::new),
                meter: u8::try_from(slot)
                    .ok()
                    .filter(|s| usize::from(*s) < lumit_audio::meter::MAX_STRIPS)
                    .unwrap_or(lumit_audio::mix::NO_METER),
            })
        })
        .collect();
    strips.truncate(lumit_audio::meter::MAX_STRIPS);
    (
        Arc::new(MixPlan {
            clips,
            total_frames,
            master_gain: lumit_audio::mix::db_to_gain(master_db),
        }),
        strips,
    )
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
    // Build without the lock: spawn the audio thread and wait for its verdict.
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let engine = match lumit_audio::AudioEngine::new_on(wanted.as_deref()) {
            Ok(engine) => {
                let _ = ready_tx.send(Some((
                    engine.clock(),
                    engine.meters(),
                    engine.device_rate(),
                )));
                engine
            }
            Err(_) => {
                let _ = ready_tx.send(None);
                return; // no device: the thread ends, the state stays Unavailable
            }
        };
        // The engine lives here for the session; each command is a plan swap
        // or an atomic store — nothing slow, nothing that can block the
        // realtime callback.
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Load { plan, start, play } => {
                    engine.load_plan(plan);
                    if let Some(s) = start {
                        engine.seek_seconds(s);
                    }
                    if play {
                        engine.play();
                    }
                }
                Cmd::Swap(plan) => engine.swap_plan(plan),
                Cmd::Play => engine.play(),
                Cmd::Pause => engine.pause(),
                Cmd::Seek(s) => engine.seek_seconds(s),
                Cmd::Unload => engine.unload(),
            }
        }
    });
    match ready_rx.recv() {
        Ok(Some((clock, meters, rate))) => {
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
            Some((tx, rate, generation))
        }
        _ => {
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
    }
}

/// Kick the prepare worker for `comp` (or park the request in the mailbox
/// while one runs). Callers hold the state lock.
fn kick_prepare(st: &mut AudioState, comp: Uuid, doc: Arc<lumit_core::Document>) {
    if matches!(st.device, Device::Unavailable) {
        return; // calm terminal state: no sound, no worker
    }
    if st.worker_busy {
        st.pending_prepare = Some((comp, doc));
        return;
    }
    st.worker_busy = true;
    std::thread::spawn(move || run_prepare(comp, doc));
}

/// The prepare worker: build and install one comp's mix, then drain the
/// mailbox. Never holds the audio lock across a probe, decode, or mix.
fn run_prepare(mut comp: Uuid, mut doc: Arc<lumit_core::Document>) {
    loop {
        prepare_once(comp, &doc);
        let next = {
            let mut st = lock();
            match st.pending_prepare.take() {
                Some(next) => Some(next),
                None => {
                    st.worker_busy = false;
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
fn prepare_once(comp: Uuid, doc: &Arc<lumit_core::Document>) {
    let Some(c) = doc.comp(comp) else {
        // The comp is gone (deleted, undone): silence its mix if loaded.
        let mut st = lock();
        if st.loaded_comp == Some(comp) {
            send(&st, Cmd::Unload);
            st.loaded_comp = None;
            st.loaded_sig = None;
            st.meter_strips.clear();
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
        }
        return;
    }
    let sig = jobs_signature(&jobs, duration_s, master_db);
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

    // Decode what the mix needs, without the lock; cached items are re-used
    // as shared buffers. A failed decode simply contributes nothing (calm).
    let mut decoded: HashMap<Uuid, Arc<lumit_media::AudioBuffer>> = HashMap::new();
    for job in &jobs {
        if decoded.contains_key(&job.item) {
            continue;
        }
        let hit = {
            let st = lock();
            st.decoded
                .get(&job.item)
                .filter(|b| b.rate == rate)
                .cloned()
        };
        match hit {
            Some(buffer) => {
                decoded.insert(job.item, buffer);
            }
            None => {
                if let Ok(buffer) = lumit_media::audio::decode_all(&job.path, rate) {
                    let buffer = Arc::new(buffer);
                    decoded.insert(job.item, Arc::clone(&buffer));
                    let mut st = lock();
                    st.decoded.insert(job.item, buffer);
                }
            }
        }
    }

    let (plan, strips) = build_plan(&jobs, &decoded, rate, duration_s, master_db);

    // Install: swap keeps the clock and play state (the instant-edit
    // contract); a fresh load applies the pending start and transport intent.
    let mut st = lock();
    // Unless the output device changed while this mix was being built. `tx`
    // then speaks to an engine that is being closed, and recording the comp as
    // loaded would leave the transport thinking it can hear something it
    // cannot. Drop the work; the next prepare opens the new device.
    if st.device_gen != generation {
        return;
    }
    if st.loaded_comp == Some(comp) {
        let _ = tx.send(Cmd::Swap(plan));
    } else {
        let start = st.pending_start.take();
        let _ = tx.send(Cmd::Load {
            plan,
            start,
            play: st.playing,
        });
    }
    st.loaded_comp = Some(comp);
    st.loaded_sig = Some(sig);
    st.meter_strips = strips;
    trim_decoded(&mut st, &jobs);
}

/// Hold the decoded-audio cache under its budget: when it grows past
/// [`DECODED_BUDGET_BYTES`], drop items the current mix does not reference
/// (their `Arc`s stay alive inside any installed plan until it is replaced).
fn trim_decoded(st: &mut AudioState, jobs: &[AudioJob]) {
    let bytes: usize = st
        .decoded
        .values()
        .map(|b| b.samples.len() * std::mem::size_of::<f32>())
        .sum();
    if bytes <= DECODED_BUDGET_BYTES {
        return;
    }
    let wanted: Vec<Uuid> = jobs.iter().map(|j| j.item).collect();
    st.decoded.retain(|item, _| wanted.contains(item));
}

/// Build (or refresh) `comp`'s mix in the background — called after an edit
/// while audio is loaded or playing. Returns immediately; an unchanged mix is a
/// no-op, a changed one is swapped in mid-playback.
pub(crate) fn prepare(comp: Uuid, doc: Arc<lumit_core::Document>) {
    let mut st = lock();
    kick_prepare(&mut st, comp, doc);
}

/// Start playback of `comp`'s audio from `start` seconds.
///
/// When its mix is already loaded this is an immediate seek and play; otherwise
/// the mix is prepared in the background and starts from `start` when it lands,
/// and the picture runs on the caller's own clock until then.
pub(crate) fn play(comp: Uuid, start: f64, doc: Arc<lumit_core::Document>) {
    let mut st = lock();
    st.playing = true;
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
    }
    st.pending_start = Some(start.max(0.0));
    kick_prepare(&mut st, comp, doc);
}

/// Pause playback (the transport's pause — the clock holds its position).
pub(crate) fn pause() {
    let mut st = lock();
    st.playing = false;
    send(&st, Cmd::Pause);
}

/// Start the sound again where it stopped, with no re-bake and no seek — the
/// other half of [`pause`].
///
/// Every-frame playback stops the sound when the picture falls behind (K-171:
/// a held track over a slow picture, never a track that drifts away from it).
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

/// Stop: pause and rewind to the start (the transport's stop semantics).
pub(crate) fn stop() {
    let mut st = lock();
    st.playing = false;
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
    st.pending_start = None;
    // Untried rather than Unavailable even when the last attempt found nothing:
    // choosing a device is exactly the moment to look again.
    st.device = Device::Untried;
}

/// The playback clock: `(seconds, is_playing, loaded)`.
///
/// Allocation-free — the state lock plus two atomic reads — because it is polled
/// every tick. With no device, or nothing loaded, it reads `(0.0, false, false)`
/// and the caller keeps its own clock.
pub(crate) fn clock() -> (f64, bool, bool) {
    let st = lock();
    match &st.device {
        Device::Ready { clock, .. } => {
            let loaded = st.loaded_comp.is_some();
            (clock.seconds(), clock.is_playing(), loaded)
        }
        _ => (0.0, false, false),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::Property;
    use std::path::PathBuf;

    fn job(path: &str, in_s: f64, out_s: f64, offset_s: f64) -> AudioJob {
        let item = Uuid::now_v7();
        AudioJob {
            item,
            layer: item, // one job, one strip: the test's own layer identity
            path: PathBuf::from(path),
            in_s,
            out_s,
            offset_s,
            volume: Property::zero(),
            pan: Property::zero(),
            carriers: Vec::new(),
            fade: None,
            driven: None,
            chain: None,
        }
    }

    /// The signature is the mix-change detector: identical jobs agree, and a
    /// move, a trim, a Volume nudge, a removed job, or a new duration each
    /// change it — the exact edits that must re-plan (docs/09 §6).
    #[test]
    fn the_signature_tracks_everything_that_changes_the_sound() {
        let a = vec![job("a.mp4", 0.0, 5.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        let same = vec![job("a.mp4", 0.0, 5.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        let sig = jobs_signature(&a, 10.0, 0.0);
        assert_eq!(
            sig,
            jobs_signature(&same, 10.0, 0.0),
            "identical mixes agree"
        );

        let moved = vec![job("a.mp4", 0.5, 5.5, 0.5), job("b.mp4", 1.0, 3.0, 1.0)];
        assert_ne!(sig, jobs_signature(&moved, 10.0, 0.0), "a move re-plans");

        let trimmed = vec![job("a.mp4", 0.0, 4.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        assert_ne!(sig, jobs_signature(&trimmed, 10.0, 0.0), "a trim re-plans");

        let removed = vec![job("a.mp4", 0.0, 5.0, 0.0)];
        assert_ne!(
            sig,
            jobs_signature(&removed, 10.0, 0.0),
            "a mute/delete re-plans"
        );

        let mut louder = vec![job("a.mp4", 0.0, 5.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        louder[0].volume = Property::fixed(-6.0);
        assert_ne!(
            sig,
            jobs_signature(&louder, 10.0, 0.0),
            "a Volume nudge re-plans"
        );

        assert_ne!(
            sig,
            jobs_signature(&a, 12.0, 0.0),
            "a duration change re-plans"
        );
    }

    /// Plan building is pure: placed clips land where `place_on_timeline`
    /// says, a job with no decoded source contributes nothing, and the strip
    /// length follows the comp duration.
    #[test]
    fn build_plan_places_decoded_clips_and_skips_missing_ones() {
        let rate = 48_000u32;
        let placed = job("a.mp4", 1.0, 3.0, 1.0);
        let missing = job("gone.mp4", 0.0, 2.0, 0.0);
        let mut decoded = HashMap::new();
        decoded.insert(
            placed.item,
            Arc::new(lumit_media::AudioBuffer {
                rate,
                samples: vec![0.25; 4 * rate as usize * 2], // 4 s of quiet tone
            }),
        );
        let (plan, strips) = build_plan(&[placed.clone(), missing], &decoded, rate, 5.0, 0.0);
        assert_eq!(
            strips,
            vec![placed.layer],
            "only the job that actually sounds claims a meter slot"
        );
        assert_eq!(plan.clips[0].meter, 0);
        assert_eq!(plan.total_frames, 5 * rate as usize);
        assert_eq!(plan.clips.len(), 1, "the undecoded job contributes nothing");
        // The layer starts at comp second 1 and sounds for its 2-second span.
        assert_eq!(plan.clips[0].start_frame, i64::from(rate));
        assert_eq!(plan.clips[0].src_start, 0);
        assert_eq!(plan.clips[0].len, 2 * rate as usize);
        // Frame 0 is silence; a frame inside the span carries the source.
        assert_eq!(plan.frame_at(0), (0.0, 0.0));
        let (l, _r) = plan.frame_at(rate as usize + 10);
        assert!((l - 0.25).abs() < 1e-6);
    }

    /// A decoded buffer at the wrong rate is never placed (media is decoded at
    /// the device rate; a stale entry must not sneak into the mix half-speed).
    #[test]
    fn build_plan_rejects_a_wrong_rate_buffer() {
        let rate = 48_000u32;
        let j = job("a.mp4", 0.0, 1.0, 0.0);
        let mut decoded = HashMap::new();
        decoded.insert(
            j.item,
            Arc::new(lumit_media::AudioBuffer {
                rate: 44_100,
                samples: vec![0.5; 44_100 * 2],
            }),
        );
        let (plan, strips) = build_plan(&[j], &decoded, rate, 1.0, 0.0);
        assert!(plan.clips.is_empty());
        assert!(strips.is_empty());
    }

    /// Two rows of the comp are two slots; a Precomp row's several sources
    /// share one, because that is the one strip the mixer draws (K-690).
    /// Past the bank's size the extras still sound and simply have no bar.
    #[test]
    fn meter_slots_follow_the_mixer_strips_and_the_bank_is_bounded() {
        let rate = 48_000u32;
        let tone = || {
            Arc::new(lumit_media::AudioBuffer {
                rate,
                samples: vec![0.25; rate as usize * 2],
            })
        };
        // Two jobs sharing a strip (one Precomp row), then a strip of its own.
        let a = job("a.wav", 0.0, 1.0, 0.0);
        let mut b = job("b.wav", 0.0, 1.0, 0.0);
        b.layer = a.layer;
        let c = job("c.wav", 0.0, 1.0, 0.0);
        let mut decoded = HashMap::new();
        for j in [&a, &b, &c] {
            decoded.insert(j.item, tone());
        }
        let (plan, strips) = build_plan(&[a.clone(), b, c.clone()], &decoded, rate, 1.0, 0.0);
        assert_eq!(strips, vec![a.layer, c.layer], "one slot per strip");
        assert_eq!(
            plan.clips.iter().map(|c| c.meter).collect::<Vec<_>>(),
            vec![0, 0, 1],
            "the shared row's two sources meter onto the one slot"
        );

        // More strips than the bank holds: the first MAX_STRIPS are metered,
        // the rest play with no bar rather than being dropped from the mix.
        let many: Vec<AudioJob> = (0..lumit_audio::meter::MAX_STRIPS + 3)
            .map(|_| job("x.wav", 0.0, 1.0, 0.0))
            .collect();
        let mut decoded = HashMap::new();
        for j in &many {
            decoded.insert(j.item, tone());
        }
        let (plan, strips) = build_plan(&many, &decoded, rate, 1.0, 0.0);
        assert_eq!(plan.clips.len(), many.len(), "every source still sounds");
        assert_eq!(strips.len(), lumit_audio::meter::MAX_STRIPS);
        assert_eq!(
            plan.clips
                .iter()
                .filter(|c| c.meter == lumit_audio::mix::NO_METER)
                .count(),
            3,
            "the three past the bank have no bar"
        );
    }

    /// The meter poll before any engine exists is the calm empty state — a
    /// mixer with no strips, which is what a device-less CI machine reads
    /// forever, and resetting the clip lights there is a no-op not a fault.
    #[test]
    fn meters_read_empty_before_any_engine_exists() {
        reset_clip();
        let strips = meters();
        // Another test on this shared state may have opened a device; only
        // assert what is invariant — the master is always the last row when
        // there is a device at all, and nothing panics either way.
        assert!(strips.is_empty() || strips.last().is_some_and(|s| s.layer.is_none()));
    }

    /// The clock poll before any engine exists is the calm zero state — the
    /// exact reading a device-less CI machine holds forever.
    #[test]
    fn the_clock_reads_zero_before_any_engine_exists() {
        let (secs, playing, loaded) = clock();
        // Another test on this shared state may have started an engine; only
        // assert what is invariant: no panic, and a non-playing, non-negative
        // clock while nothing has ever been loaded and played here.
        assert!(secs >= 0.0);
        assert!(
            !playing || loaded,
            "playing without a loaded mix is impossible"
        );
        let _ = loaded;
    }

    /// Choosing an output closes the open stream and forgets what was loaded,
    /// so the next prepare opens the new device — and choosing the one already
    /// in force does nothing at all, which is what lets the frontend hand the
    /// stored choice over on every boot.
    #[test]
    fn choosing_an_output_closes_the_stream_and_repeating_it_does_not() {
        let before = lock().device_gen;
        set_device(Some("Some device that is not here".to_owned()));
        {
            let st = lock();
            assert_eq!(st.device_gen, before.wrapping_add(1));
            assert!(matches!(st.device, Device::Untried));
            assert_eq!(st.loaded_comp, None);
            assert_eq!(st.loaded_sig, None);
            assert!(!st.playing);
        }

        set_device(Some("Some device that is not here".to_owned()));
        assert_eq!(
            lock().device_gen,
            before.wrapping_add(1),
            "the same choice again is a no-op"
        );

        // A chosen device that is not on this machine reads as a fallback —
        // unless the machine has no output at all, which says so by having no
        // active id rather than by claiming a substitution.
        let (_list, active, fell_back) = devices();
        assert_eq!(fell_back, active.is_some());

        // Put the shared state back for whatever else runs in this process.
        set_device(None);
        assert!(!devices().2, "the system default is never a fallback");
    }

    // ------------------------------------------- the audio insert chain --

    /// A stand-in audio plugin: it multiplies by its one row and then **hard
    /// clips**, which is what makes it useful for the order-of-stages test —
    /// a plain gain would commute with the fader and prove nothing about where
    /// the insert sits.
    struct TestInsert {
        /// The block to refuse, which is what a crashed or hung plugin costs.
        refuse: Option<usize>,
        latency: u32,
    }

    impl lumit_core::fx::AudioProcessor for TestInsert {
        fn process(
            &self,
            input: &[f32],
            output: &mut [f32],
            values: &[(lumit_core::fx::ParamId, f64)],
            steady: i64,
        ) -> bool {
            if self.refuse == Some(steady as usize / lumit_core::fx::AUDIO_BLOCK_FRAMES) {
                return false;
            }
            let g = values
                .iter()
                .find(|(id, _)| *id == lumit_core::fx::ParamId::new("p1"))
                .map_or(1.0, |(_, v)| *v) as f32;
            for (o, i) in output.iter_mut().zip(input) {
                *o = (i * g).clamp(-0.5, 0.5);
            }
            true
        }

        fn latency(&self) -> u32 {
            self.latency
        }
    }

    struct TestInsertDef {
        schema: &'static lumit_core::fx::EffectSchema,
        refuse: Option<usize>,
        latency: u32,
    }

    impl lumit_core::fx::EffectDef for TestInsertDef {
        fn schema(&self) -> &'static lumit_core::fx::EffectSchema {
            self.schema
        }

        fn is_image_op(&self) -> bool {
            false
        }

        fn open_audio(
            &self,
            _state: Option<Vec<u8>>,
            _values: &[(lumit_core::fx::ParamId, f64)],
            _offline: bool,
        ) -> Option<Arc<dyn lumit_core::fx::AudioProcessor>> {
            Some(Arc::new(TestInsert {
                refuse: self.refuse,
                latency: self.latency,
            }))
        }
    }

    /// Register one stand-in plugin under `name`. Registration is additive and
    /// by name, so each test takes a name of its own and they cannot tread on
    /// one another.
    fn register_insert(name: &'static str, refuse: Option<usize>, latency: u32) {
        let schema: &'static lumit_core::fx::EffectSchema =
            Box::leak(Box::new(lumit_core::fx::EffectSchema {
                match_name: name,
                label: "Test insert",
                version: 1,
                category: lumit_core::fx::FxCategory::Utility,
                traits: lumit_core::fx::EffectTraits {
                    cost: lumit_core::fx::CostClass::Heavy,
                    roi: lumit_core::fx::Roi::FullFrame,
                    temporal: &[0],
                    premultiplied: true,
                    seeded: false,
                    beat_input: false,
                },
                params: Box::leak(Box::new([lumit_core::fx::ParamSchema {
                    id: "p1",
                    label: "Gain",
                    kind: lumit_core::fx::ParamKind::Slider {
                        default: 1.0,
                        range: (0.0, 4.0),
                    },
                    unit: lumit_core::fx::Unit::Raw,
                }])),
                groups: &[],
                enabled_when: &[],
                matte: lumit_core::fx::MatteRole::None,
            }));
        lumit_core::fx::BUILTIN_DEFS.register(Box::leak(Box::new(TestInsertDef {
            schema,
            refuse,
            latency,
        })));
    }

    /// One instance of a stand-in plugin, its `p1` row holding `value`.
    fn insert_instance(name: &str, value: Property) -> lumit_core::model::EffectInstance {
        lumit_core::model::EffectInstance {
            id: Uuid::now_v7(),
            effect: lumit_core::model::EffectKey {
                namespace: lumit_core::model::EffectNamespace::Clap,
                match_name: name.to_owned(),
                version: 1,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: vec![lumit_core::model::EffectParam {
                id: "p1".into(),
                value: lumit_core::model::EffectValue::Float(value),
                extra: serde_json::Map::new(),
            }],
            sample_temporally: true,
            custom_name: None,
            linked_pairs: Vec::new(),
            plugin_state: None,
            extra: serde_json::Map::new(),
        }
    }

    /// The chain a job carries for `effects`.
    fn chain_of(
        effects: Vec<lumit_core::model::EffectInstance>,
    ) -> Arc<lumit_render::export::AudioChain> {
        Arc::new(lumit_render::export::AudioChain {
            doc: Arc::new(lumit_core::Document::new()),
            comp: Uuid::now_v7(),
            layer: Uuid::now_v7(),
            effects,
            graph: Default::default(),
            offset_s: 0.0,
            base_s: 0.0,
        })
    }

    fn tone(rate: u32, seconds: f64, value: f32) -> Arc<lumit_media::AudioBuffer> {
        Arc::new(lumit_media::AudioBuffer {
            rate,
            samples: vec![value; (f64::from(rate) * seconds) as usize * 2],
        })
    }

    /// **A layer with no audio effect is the mix it always was** (K-700, the
    /// note's §7 plan 3): the plan holds the *same* decoded buffer, not a copy
    /// of it, so an empty chain costs no memory, no arithmetic and no chance of
    /// drift. A stack full of picture effects is the same thing — the catalogue
    /// opens none of them as audio.
    #[test]
    fn an_empty_chain_leaves_the_decoded_buffer_untouched() {
        let rate = 48_000u32;
        let source = tone(rate, 1.0, 0.25);
        let bare = job("a.mp4", 0.0, 1.0, 0.0);
        let mut blurred = bare.clone();
        blurred.chain = Some(chain_of(vec![lumit_core::fx::instantiate("blur").unwrap()]));

        let mut decoded = HashMap::new();
        decoded.insert(bare.item, Arc::clone(&source));
        for one in [bare, blurred] {
            let (plan, _) = build_plan(&[one], &decoded, rate, 1.0, 0.0);
            assert!(
                Arc::ptr_eq(&plan.clips[0].buffer, &source),
                "the plan must hold the decoded buffer itself"
            );
        }
    }

    /// **Preview is export** through a plugin (K-031, the note's §7 plan 3).
    ///
    /// Not "they agree to within a tolerance": the live plan and the baked
    /// mixdown run the *same* chain over the *same* placement, so the two are
    /// equal sample for sample — and the sound really did go through the
    /// plugin, which the clip at ±0.5 proves.
    #[test]
    fn a_plugin_sounds_identical_through_the_live_plan_and_the_bake() {
        let rate = 48_000u32;
        register_insert("clap:lumit.test.preview", None, 0);
        let source = tone(rate, 1.0, 0.8);
        let mut one = job("a.mp4", 0.0, 1.0, 0.0);
        one.chain = Some(chain_of(vec![insert_instance(
            "clap:lumit.test.preview",
            Property::fixed(1.0),
        )]));

        let mut decoded = HashMap::new();
        decoded.insert(one.item, Arc::clone(&source));
        let (plan, _) = build_plan(&[one.clone()], &decoded, rate, 1.0, 0.0);
        let baked =
            lumit_render::export::mixdown_prepared(&[(Arc::clone(&source), one)], rate, 1.0, 1.0);

        assert!(
            (baked[0] - 0.5).abs() < 1e-6,
            "the insert really ran: 0.8 held at the clipper's ±0.5, got {}",
            baked[0]
        );
        for i in 0..rate as usize {
            let (l, r) = plan.frame_at(i);
            assert!(
                (l - baked[i * 2]).abs() < 1e-9 && (r - baked[i * 2 + 1]).abs() < 1e-9,
                "frame {i}: the live plan and the export disagree"
            );
        }
    }

    /// **The insert sits ahead of Volume** (docs/impl/audio-plugins.md §2, the
    /// decided position).
    ///
    /// The fader is put down 6 dB on a layer whose plugin clips at ±0.5.
    /// Insert first gives `fade × clip(x)`; fader first would give
    /// `clip(fade × x)`, and with a hot source those are different numbers —
    /// which is exactly why the order is decided rather than incidental.
    #[test]
    fn the_insert_runs_before_the_fader_not_after_it() {
        let rate = 48_000u32;
        register_insert("clap:lumit.test.prefader", None, 0);
        let source = tone(rate, 1.0, 0.8);
        let mut one = job("a.mp4", 0.0, 1.0, 0.0);
        one.volume = Property::fixed(-6.0);
        one.chain = Some(chain_of(vec![insert_instance(
            "clap:lumit.test.prefader",
            Property::fixed(1.0),
        )]));

        let baked = lumit_render::export::mixdown_prepared(&[(source, one)], rate, 1.0, 1.0);
        let half = lumit_audio::mix::db_to_gain(-6.0);
        assert!(
            (baked[0] - 0.5 * half).abs() < 1e-5,
            "the fade rides the processed sound: expected {}, got {}",
            0.5 * half,
            baked[0]
        );
        assert!(
            (baked[0] - 0.5).abs() > 1e-3,
            "and the fader is not being swallowed by the clipper"
        );
    }

    /// **An animated plugin parameter renders deterministically** (the note's
    /// §7 plans 4 and 7): the sweep is baked at one value per 512-frame block,
    /// those blocks are a fact about the layer rather than about the playhead,
    /// and two bakes of the same project are byte-identical.
    #[test]
    fn an_animated_plugin_parameter_bakes_per_block_and_repeats_exactly() {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        let rate = 48_000u32;
        register_insert("clap:lumit.test.sweep", None, 0);
        // 0 → 1 across the second, on a quiet source so the clipper never bites.
        let key = |t: i64, v: f64| Keyframe {
            time: lumit_core::Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let sweep = Property {
            animation: Animation::Keyframed(vec![key(0, 0.0), key(1, 1.0)]),
            extra: serde_json::Map::new(),
        };
        let source = tone(rate, 1.0, 0.2);
        let mut one = job("a.mp4", 0.0, 1.0, 0.0);
        one.chain = Some(chain_of(vec![insert_instance(
            "clap:lumit.test.sweep",
            sweep,
        )]));

        let bake = || {
            lumit_render::export::mixdown_prepared(
                &[(Arc::clone(&source), one.clone())],
                rate,
                1.0,
                1.0,
            )
        };
        let first = bake();
        assert_eq!(first, bake(), "two bakes of one project are identical");

        // Block b was handed the value at its own first frame, and held it for
        // the whole block — a step per 512 frames, not a slope per sample.
        let block = lumit_core::fx::AUDIO_BLOCK_FRAMES;
        for b in [0usize, 1, 10, 80] {
            let want = 0.2 * (b * block) as f32 / rate as f32;
            let at = b * block * 2;
            assert!(
                (first[at] - want).abs() < 1e-5,
                "block {b}: expected {want}, got {}",
                first[at]
            );
            assert!(
                (first[at] - first[at + (block - 1) * 2]).abs() < 1e-9,
                "block {b} holds one value all the way across"
            );
        }
    }

    /// **A block the plugin refuses ships dry, and the sound goes on**
    /// (docs/impl/audio-plugins.md §3): the refused block carries the chain's
    /// input unchanged, everything either side is processed, and the splice is
    /// ramped rather than cut — a hole in the music would be worse than a block
    /// of it slightly wrong, and a click worse than either.
    #[test]
    fn a_refused_block_ships_dry_and_the_rest_still_plays() {
        let rate = 48_000u32;
        register_insert("clap:lumit.test.refuse", Some(4), 0);
        let source = tone(rate, 1.0, 0.8);
        let mut one = job("a.mp4", 0.0, 1.0, 0.0);
        one.chain = Some(chain_of(vec![insert_instance(
            "clap:lumit.test.refuse",
            Property::fixed(1.0),
        )]));
        let baked = lumit_render::export::mixdown_prepared(&[(source, one)], rate, 1.0, 1.0);

        let block = lumit_core::fx::AUDIO_BLOCK_FRAMES;
        let inside = (4 * block + block / 2) * 2;
        assert!(
            (baked[inside] - 0.8).abs() < 1e-6,
            "the refused block is the input, dry: got {}",
            baked[inside]
        );
        assert!(
            (baked[0] - 0.5).abs() < 1e-6,
            "and every other block is still processed"
        );
        let worst = baked
            .chunks_exact(2)
            .zip(baked.chunks_exact(2).skip(1))
            .map(|(a, b)| (b[0] - a[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < 0.02,
            "the splice must not click: worst step {worst}"
        );
    }

    /// **Latency is compensated by placing the sound earlier** (the note's §7
    /// plan 3): a plugin that answers N frames late has its run put down N
    /// frames sooner and made N frames longer, so the wet lands where the dry
    /// did and a lookahead limiter simply works.
    #[test]
    fn a_latent_plugin_is_placed_earlier_by_exactly_its_latency() {
        let rate = 48_000u32;
        let latency = 128u32;
        register_insert("clap:lumit.test.latent", None, latency);
        let source = tone(rate, 2.0, 0.2);
        let mut one = job("a.mp4", 1.0, 2.0, 0.0);
        one.chain = Some(chain_of(vec![insert_instance(
            "clap:lumit.test.latent",
            Property::fixed(1.0),
        )]));
        let mut decoded = HashMap::new();
        decoded.insert(one.item, source);
        let (plan, _) = build_plan(&[one], &decoded, rate, 3.0, 0.0);

        let clip = &plan.clips[0];
        assert_eq!(
            clip.start_frame,
            i64::from(rate) - i64::from(latency),
            "placed earlier by the chain's own delay"
        );
        assert_eq!(
            clip.src_start, 0,
            "the processed run starts at its own zero"
        );
        assert_eq!(
            clip.len,
            rate as usize + latency as usize,
            "and is long enough for the delayed tail to come out"
        );
    }

    /// The mix signature hears the rack, not only the fader (K-700): dropping a
    /// plugin on the row, nudging one of its knobs, bypassing it or reordering
    /// the stack each change what the comp sounds like, and each must re-plan.
    #[test]
    fn the_signature_changes_when_the_insert_chain_does() {
        let plain = vec![job("a.mp4", 0.0, 5.0, 0.0)];
        let sig = jobs_signature(&plain, 10.0, 0.0);

        let eq = insert_instance("clap:lumit.test.sig", Property::fixed(1.0));
        let comp = insert_instance("clap:lumit.test.sig2", Property::fixed(1.0));
        let with = |effects: Vec<lumit_core::model::EffectInstance>| {
            let mut jobs = plain.clone();
            jobs[0].chain = Some(chain_of(effects));
            jobs_signature(&jobs, 10.0, 0.0)
        };
        let dropped = with(vec![eq.clone(), comp.clone()]);
        assert_ne!(sig, dropped, "dropping a plugin re-plans");
        assert_ne!(
            dropped,
            with(vec![comp.clone(), eq.clone()]),
            "reordering the rack re-plans"
        );

        let mut nudged = eq.clone();
        nudged.params[0].value = lumit_core::model::EffectValue::Float(Property::fixed(2.0));
        assert_ne!(
            dropped,
            with(vec![nudged, comp.clone()]),
            "a knob nudge re-plans"
        );

        let mut bypassed = eq;
        bypassed.enabled = false;
        assert_ne!(dropped, with(vec![bypassed, comp]), "a bypass re-plans");
    }

    /// **The calm badge lands on the plugin that refused** (AP5, docs/12
    /// §2.3, the OFX badge grammar): a bake that ships any of a link's blocks
    /// dry files the failure against *that instance*, the bridge's
    /// `read_instance_info` turns it into `plugin_failed`, and a later bake
    /// whose every block comes back takes the badge off again.
    #[test]
    fn a_dying_plugin_badges_its_own_instance_and_a_clean_bake_heals_it() {
        let rate = 48_000u32;
        register_insert("clap:lumit.test.badge.dying", Some(0), 0);
        register_insert("clap:lumit.test.badge.well", None, 0);
        let dying = insert_instance("clap:lumit.test.badge.dying", Property::fixed(2.0));
        let well = insert_instance("clap:lumit.test.badge.well", Property::fixed(1.0));

        // One block of sound, whose one block the dying plugin refuses; the
        // well one beside it in the same chain processes every block.
        let seconds = lumit_core::fx::AUDIO_BLOCK_FRAMES as f64 / f64::from(rate);
        let source = tone(rate, seconds, 0.25);
        let mut one = job("a.mp4", 0.0, seconds, 0.0);
        one.chain = Some(chain_of(vec![dying.clone(), well.clone()]));
        let mut decoded = HashMap::new();
        decoded.insert(one.item, source);
        build_plan(&[one.clone()], &decoded, rate, 1.0, 0.0);

        let badged =
            crate::api::effect::read_instance_info(&dying, lumit_core::time::Rational::ZERO);
        assert_eq!(badged.badge_reason.as_deref(), Some("plugin_failed"));
        assert_eq!(
            badged.badge_detail.as_deref(),
            Some("the plugin did not process this sound"),
            "a processor with no sentence of its own still explains itself"
        );
        let neighbour =
            crate::api::effect::read_instance_info(&well, lumit_core::time::Rational::ZERO);
        assert_eq!(
            neighbour.badge_reason, None,
            "the badge lands on the link that refused, not on the rack"
        );

        // Healing: a bake whose every block comes back clears the note. The
        // same instance id, moved onto the plugin that answers.
        let mut healed = well.clone();
        healed.id = dying.id;
        let mut again = one;
        again.chain = Some(chain_of(vec![healed.clone()]));
        build_plan(&[again], &decoded, rate, 1.0, 0.0);
        let cleared =
            crate::api::effect::read_instance_info(&healed, lumit_core::time::Rational::ZERO);
        assert_eq!(
            cleared.badge_reason, None,
            "a clean bake takes the badge off"
        );
    }

    /// Flicking a plugin's switch re-plans the comps whose chains hold it, and
    /// only those (AP5): the switched-off list is not in the document, so the
    /// signature is where the change is heard.
    #[test]
    fn the_signature_hears_the_switched_off_list_for_its_own_chain() {
        let mut with_chain = vec![job("a.mp4", 0.0, 5.0, 0.0)];
        with_chain[0].chain = Some(chain_of(vec![insert_instance(
            "clap:lumit.test.switched",
            Property::fixed(1.0),
        )]));
        let bystander = vec![job("b.mp4", 0.0, 5.0, 0.0)];

        let on = jobs_signature(&with_chain, 10.0, 0.0);
        let bystander_on = jobs_signature(&bystander, 10.0, 0.0);
        lumit_aplug::set_enabled("lumit.test.switched", false);
        let off = jobs_signature(&with_chain, 10.0, 0.0);
        let bystander_off = jobs_signature(&bystander, 10.0, 0.0);
        lumit_aplug::set_enabled("lumit.test.switched", true);

        assert_ne!(on, off, "the flick is a mix change for this comp");
        assert_eq!(
            bystander_on, bystander_off,
            "and a no-op for a comp that never plays the plugin"
        );
        assert_eq!(
            on,
            jobs_signature(&with_chain, 10.0, 0.0),
            "switching back on restores the signature"
        );
    }
}
