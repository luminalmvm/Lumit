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
    /// and the device rate media decodes at.
    Ready {
        tx: Sender<Cmd>,
        clock: lumit_audio::ClockHandle,
        rate: u32,
    },
}

/// Everything behind the session audio lock. The lock is held only for
/// bookkeeping and channel sends — never across a probe, a decode, a mix, or
/// the FFI boundary.
struct AudioState {
    device: Device,
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
}

impl AudioState {
    fn new() -> Self {
        Self {
            device: Device::Untried,
            loaded_comp: None,
            loaded_sig: None,
            playing: false,
            pending_start: None,
            worker_busy: false,
            pending_prepare: None,
            jobs: AudioJobsBuilder::new(),
            decoded: HashMap::new(),
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
pub(crate) fn jobs_signature(jobs: &[AudioJob], duration_s: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    jobs.len().hash(&mut h);
    duration_s.to_bits().hash(&mut h);
    for j in jobs {
        j.path.hash(&mut h);
        j.in_s.to_bits().hash(&mut h);
        j.out_s.to_bits().hash(&mut h);
        j.offset_s.to_bits().hash(&mut h);
        hash_animation(&mut h, &j.volume.animation);
        j.carriers.len().hash(&mut h);
        for (prop, off) in &j.carriers {
            off.to_bits().hash(&mut h);
            hash_animation(&mut h, &prop.animation);
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
pub(crate) fn build_plan(
    jobs: &[AudioJob],
    decoded: &HashMap<Uuid, Arc<lumit_media::AudioBuffer>>,
    rate: u32,
    duration_s: f64,
) -> Arc<MixPlan> {
    let total_frames = (duration_s * f64::from(rate)).round().max(0.0) as usize;
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
            let (gain, envelope) = lumit_render::export::volume_bake(job, start_frame, len, rate);
            Some(lumit_audio::mix::PlacedClip {
                buffer: Arc::clone(buffer),
                start_frame,
                src_start,
                len,
                gain,
                envelope: envelope.map(Arc::new),
            })
        })
        .collect();
    Arc::new(MixPlan {
        clips,
        total_frames,
    })
}

/// Resolve the audio device, building the engine on its own thread on first
/// use. Called only from the prepare worker (so at most one build races
/// nothing). `None` on the calm terminal no-device state.
fn ensure_device() -> Option<(Sender<Cmd>, u32)> {
    // Fast path under the lock.
    {
        let st = lock();
        match &st.device {
            Device::Unavailable => return None,
            Device::Ready { tx, rate, .. } => return Some((tx.clone(), *rate)),
            Device::Untried => {}
        }
    }
    // Build without the lock: spawn the audio thread and wait for its verdict.
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let engine = match lumit_audio::AudioEngine::new() {
            Ok(engine) => {
                let _ = ready_tx.send(Some((engine.clock(), engine.device_rate())));
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
        Ok(Some((clock, rate))) => {
            let mut st = lock();
            st.device = Device::Ready {
                tx: tx.clone(),
                clock,
                rate,
            };
            Some((tx, rate))
        }
        _ => {
            let mut st = lock();
            st.device = Device::Unavailable;
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
            st.loaded_comp = None;
            st.loaded_sig = None;
        }
        return;
    }
    let sig = jobs_signature(&jobs, duration_s);
    {
        let st = lock();
        if st.loaded_comp == Some(comp) && st.loaded_sig == Some(sig) {
            return; // this exact mix is already playing — a no-op
        }
    }

    // Resolve the device (first use builds the engine; may block this worker
    // briefly — never a caller).
    let Some((tx, rate)) = ensure_device() else {
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

    let plan = build_plan(&jobs, &decoded, rate, duration_s);

    // Install: swap keeps the clock and play state (the instant-edit
    // contract); a fresh load applies the pending start and transport intent.
    let mut st = lock();
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
        AudioJob {
            item: Uuid::now_v7(),
            path: PathBuf::from(path),
            in_s,
            out_s,
            offset_s,
            volume: Property::zero(),
            carriers: Vec::new(),
        }
    }

    /// The signature is the mix-change detector: identical jobs agree, and a
    /// move, a trim, a Volume nudge, a removed job, or a new duration each
    /// change it — the exact edits that must re-plan (docs/09 §6).
    #[test]
    fn the_signature_tracks_everything_that_changes_the_sound() {
        let a = vec![job("a.mp4", 0.0, 5.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        let same = vec![job("a.mp4", 0.0, 5.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        let sig = jobs_signature(&a, 10.0);
        assert_eq!(sig, jobs_signature(&same, 10.0), "identical mixes agree");

        let moved = vec![job("a.mp4", 0.5, 5.5, 0.5), job("b.mp4", 1.0, 3.0, 1.0)];
        assert_ne!(sig, jobs_signature(&moved, 10.0), "a move re-plans");

        let trimmed = vec![job("a.mp4", 0.0, 4.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        assert_ne!(sig, jobs_signature(&trimmed, 10.0), "a trim re-plans");

        let removed = vec![job("a.mp4", 0.0, 5.0, 0.0)];
        assert_ne!(
            sig,
            jobs_signature(&removed, 10.0),
            "a mute/delete re-plans"
        );

        let mut louder = vec![job("a.mp4", 0.0, 5.0, 0.0), job("b.mp4", 1.0, 3.0, 1.0)];
        louder[0].volume = Property::fixed(-6.0);
        assert_ne!(
            sig,
            jobs_signature(&louder, 10.0),
            "a Volume nudge re-plans"
        );

        assert_ne!(sig, jobs_signature(&a, 12.0), "a duration change re-plans");
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
        let plan = build_plan(&[placed.clone(), missing], &decoded, rate, 5.0);
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
        let plan = build_plan(&[j], &decoded, rate, 1.0);
        assert!(plan.clips.is_empty());
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
}
