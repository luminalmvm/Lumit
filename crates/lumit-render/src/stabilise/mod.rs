//! The Stabilise analysis: its job, its queue, its sidecar, and what opening a
//! project and exporting ask of it.
//!
//! # In plain terms
//!
//! Adding a Stabilise starts an analysis. It reads every frame of the layer's
//! clip (or of the comp a Precomp layer shows), follows specks through them,
//! and files what it found where the effect's frames read it
//! ([`lumit_core::stabilise::analysis`]). It runs on its own thread, one at a
//! time; a second one asked for while it runs waits its turn rather than being
//! refused, so stabilising a dozen clips in a row just works.
//!
//! What it found depends only on the clip's content and the rows that change
//! what is followed, so it is written to the `track/` cache folder under a name
//! made of exactly those. The next session reads it back instead of analysing
//! again. Deleting the folder costs a re-analysis and nothing else.
//!
//! Threads: the job runs on `lumit-stabilise`, the frames are decoded on
//! `lumit-stabilise-decode` ([`frames`]), and the queue below is the only state
//! they share, locked for one push, pop or read at a time.

pub mod frames;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use lumit_core::fx::effects::stabilise::{MATCH_NAME, MODE_REVERSE};
use lumit_core::model::{
    Document, EffectInstance, EffectValue, Fingerprint, Layer, LayerKind, ProjectItem,
};
use lumit_core::stabilise::analysis::{self, Analysis};
use lumit_core::stabilise::resolve::{forward, reversed};
use lumit_core::stabilise::rows::choice;
use lumit_track::exclude::ExclusionMask;
use lumit_track::solve::SolveError;
use lumit_track::stabilise::{analyse as fit_analysis, to_tracked, Shape, StabiliseError};
use lumit_track::{FramePlane, TrackSettings, Tracker};
use uuid::Uuid;

use crate::track::{
    AnalysisError, AnalysisKey, AnalysisSettings, CompLuma, LumaFrames, MaskTrack, MediaLuma,
    Progress,
};

/// The sidecar's layout. Bump it when [`Analysis`] changes shape: an older file
/// then has another name and is never asked for.
const FORMAT_VERSION: u16 = 3;

/// `LUMSTB\0`, read before anything is deserialised.
const MAGIC: &[u8; 7] = b"LUMSTB\0";

/// What one analysis is filed under, and what the effect's freshness reading
/// compares with.
pub type Key = [u8; 32];

/// One analysis, as handed to the job thread.
pub struct Job {
    /// The Stabilise instance it is for: what progress is read by and the
    /// answer filed under.
    pub effect: Uuid,
    /// The footage item or nested comp it reads.
    pub source: Uuid,
    /// What it is called: the content and the rows that change what is found.
    pub key: Key,
    /// Whether the key names a file's content, so the answer can go in the
    /// sidecar. A nested comp's picture is the whole document beneath it, so
    /// its analysis lives for the session only.
    pub on_disk: bool,
    pub settings: AnalysisSettings,
    /// Regions no speck may be born in or wander into, in layer pixels.
    pub masks: MaskTrack,
    /// Opens the frames, on the decode thread.
    pub open: Box<dyn FnOnce() -> Option<Box<dyn LumaFrames>> + Send>,
    /// A press of Analyse: the clip is analysed again even when the sidecar
    /// has an answer. Every automatic start reads the sidecar first.
    pub fresh: bool,
    /// The source seconds to read, or the whole clip.
    pub range: Option<(f64, f64)>,
    /// The rate the source plays at now where the item sets one over the
    /// file's own, which the range's seconds are counted in.
    pub rate: Option<f64>,
    /// Analysed frames per frame of the item's rate: see
    /// [`Analysis::rate_scale`].
    pub rate_scale: f64,
}

/// The key for a Stabilise on a file with `fingerprint`: the trackers' own key
/// for that content, settings and masks, salted so the two can never share a
/// sidecar file, and limited to `range` when the Frames row limits it, the
/// range read at `rate`. The whole clip is the same frames at any rate, so
/// the rate is only part of the key when a range is.
#[must_use]
pub fn key_for(
    fingerprint: &Fingerprint,
    settings: AnalysisSettings,
    masks: &MaskTrack,
    range: Option<(f64, f64)>,
    rate: Option<f64>,
) -> Key {
    let mut h = blake3::Hasher::new();
    h.update(b"lumit-stabilise/");
    h.update(&FORMAT_VERSION.to_le_bytes());
    h.update(
        AnalysisKey::new(fingerprint, settings, masks)
            .file_name()
            .as_bytes(),
    );
    if let Some((a, b)) = range {
        h.update(b"frames/");
        h.update(&a.to_le_bytes());
        h.update(&b.to_le_bytes());
        h.update(&rate.unwrap_or(0.0).to_le_bytes());
    }
    *h.finalize().as_bytes()
}

/// The first enabled Stabilise on `layer` taking the shake out, which is the
/// one that analyses.
#[must_use]
pub fn stabilise_effect(layer: &Layer) -> Option<&EffectInstance> {
    forward(layer)
}

/// The job for `fx` on footage layer `layer`, reading the file at `path`.
/// The file is read at its own rate: an Interpret footage rate re-times the
/// same frames, so the analysis stands whatever rate is set later.
#[must_use]
pub fn job_for(
    layer: &Layer,
    fx: &EffectInstance,
    path: impl Into<lumit_media::sequence::MediaSource>,
    fingerprint: &Fingerprint,
    fresh: bool,
    range: Option<(f64, f64)>,
) -> Option<Job> {
    let LayerKind::Footage { item } = layer.kind else {
        return None;
    };
    let mut path: lumit_media::sequence::MediaSource = path.into();
    let set = path.interpret.conform_fps.or(path.sequence_fps);
    path.interpret.conform_fps = None;
    // Frames per frame of the rate: what fields or a pulldown make of it.
    const BASE: i32 = 1_000_000;
    let (n, d) = path.interpret.fps((BASE, 1));
    let rate_scale = if n > 0 && d > 0 {
        f64::from(n) / f64::from(d) / f64::from(BASE)
    } else {
        1.0
    };
    let rate = set
        .filter(|&(n, d)| n > 0 && d > 0)
        .map(|(n, d)| f64::from(n) / f64::from(d) * rate_scale);
    let settings = AnalysisSettings::of(fx);
    let masks = MaskTrack::of(layer, settings, 1.0);
    Some(Job {
        effect: fx.id,
        source: item,
        key: key_for(fingerprint, settings, &masks, range, rate),
        on_disk: true,
        settings,
        masks,
        open: Box::new(move || MediaLuma::open(&path).map(|s| Box::new(s) as Box<dyn LumaFrames>)),
        fresh,
        range,
        rate,
        rate_scale,
    })
}

/// The job for `fx` on Precomp layer `layer`: the nested comp's frames are
/// rendered rather than decoded, at the trackers' analysis size.
#[must_use]
pub fn job_for_precomp(
    doc: &Arc<Document>,
    layer: &Layer,
    fx: &EffectInstance,
    fresh: bool,
    range: Option<(f64, f64)>,
) -> Option<Job> {
    let LayerKind::Precomp { comp: nested } = layer.kind else {
        return None;
    };
    let fps = doc.comp(nested)?.frame_rate.fps();
    let settings = AnalysisSettings::of(fx);
    // The masks are in the Precomp layer's own pixels, the nested comp's
    // raster, which is what the analysis is measured in too.
    let masks = MaskTrack::of(layer, settings, 1.0);
    // Named by which comp it is and its rate, which decides which pictures
    // its frames are, with the rows and the masks: all its freshness can be
    // judged by.
    let named = Fingerprint {
        size: 0,
        mtime_secs: 0,
        head_tail_hash: format!("comp/{nested}/{}", fps.to_bits()),
    };
    let doc = Arc::clone(doc);
    Some(Job {
        effect: fx.id,
        source: nested,
        key: key_for(&named, settings, &masks, range, None),
        on_disk: false,
        settings,
        masks,
        open: Box::new(move || {
            CompLuma::open(doc, nested).map(|s| Box::new(s) as Box<dyn LumaFrames>)
        }),
        fresh,
        range,
        rate: None,
        rate_scale: 1.0,
    })
}

/// The job for `fx` on `layer` of `doc`, read from the footage item's own
/// file or the nested comp: what every automatic start asks for. `None` for
/// footage that is offline, or a layer showing neither.
fn job_in(
    doc: &Arc<Document>,
    comp: &lumit_core::model::Composition,
    layer: &Layer,
    fx: &EffectInstance,
) -> Option<Job> {
    let range = lumit_core::planes::source_span(fx, Some(layer), comp);
    match layer.kind {
        LayerKind::Footage { item: media } => {
            let footage = doc.items.iter().find_map(|i| match i {
                ProjectItem::Footage(f) if f.id == media => Some(f),
                _ => None,
            })?;
            let fingerprint = footage.media.fingerprint.as_ref()?;
            if footage.media.absolute_path.is_empty() {
                return None;
            }
            let (path, fingerprint) = crate::source::analysed(
                footage,
                PathBuf::from(&footage.media.absolute_path),
                fingerprint,
            );
            job_for(layer, fx, path, &fingerprint, false, range)
        }
        LayerKind::Precomp { .. } => job_for_precomp(doc, layer, fx, false, range),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The queue
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Queue {
    /// Whether the runner thread is alive. Set under the lock by the request
    /// that starts it, and cleared under the same lock by the runner when it
    /// finds nothing waiting, or by its guard when it dies: so two requests
    /// never start two runners, and a dead one never stops the next.
    runner: bool,
    /// The job running, by effect, and the flag Cancel raises.
    running: Option<(Uuid, Arc<AtomicBool>)>,
    /// Jobs waiting their turn, oldest first. One per effect at most.
    waiting: VecDeque<Job>,
    /// The last reading of every effect asked about this session.
    progress: HashMap<Uuid, Progress>,
    /// The key each effect was last asked for under, so asking again for the
    /// same thing does nothing.
    asked: HashMap<Uuid, Key>,
}

/// Eviction: one reading per Stabilise instance asked about this session,
/// dropped with the project by [`forget`].
fn queue() -> MutexGuard<'static, Queue> {
    static QUEUE: OnceLock<Mutex<Queue>> = OnceLock::new();
    // A thread that died holding the lock left the queue as it was between
    // two whole steps, so it is taken on rather than every call failing.
    QUEUE
        .get_or_init(|| Mutex::new(Queue::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How far the analysis for `effect` has got, or `None` when nothing has been
/// asked for it this session.
#[must_use]
pub fn progress(effect: Uuid) -> Option<Progress> {
    queue().progress.get(&effect).cloned()
}

fn report(effect: Uuid, step: Progress) {
    queue().progress.insert(effect, step);
}

/// Ask for `job`. It starts now if nothing else is running, and otherwise
/// waits its turn; asking again for an effect already waiting replaces the job
/// it was waiting with.
pub fn request(job: Job) {
    let start = {
        let mut q = queue();
        q.progress.insert(job.effect, Progress::Queued);
        q.asked.insert(job.effect, job.key);
        if let Some((running, flag)) = &q.running {
            if *running == job.effect {
                // Running already: the new job replaces it once it stops.
                flag.store(true, Ordering::Relaxed);
            }
        }
        q.waiting.retain(|w| w.effect != job.effect);
        q.waiting.push_back(job);
        let start = !q.runner;
        q.runner = true;
        start
    };
    if start && !spawn_runner() {
        // No thread could be started: nothing will ever run what was queued,
        // so it is said rather than left waiting.
        let mut q = queue();
        q.runner = false;
        for job in q.waiting.drain(..).collect::<Vec<_>>() {
            q.progress
                .insert(job.effect, Progress::Failed(AnalysisError::Unreadable));
        }
    }
}

/// Ask for `job` unless the same analysis has already been asked for: what
/// every automatic start goes through, so a frame drawn while an analysis
/// runs, or one that failed or was cancelled, never starts it again by itself.
/// Analyse is the one way to ask twice.
pub fn ask(job: Job) {
    let again = queue().asked.get(&job.effect) == Some(&job.key);
    if !again {
        request(job);
    }
}

/// Stop `effect`'s analysis, running or waiting. Nothing it found is filed.
pub fn cancel(effect: Uuid) {
    let mut q = queue();
    let running = q.running.as_ref().is_some_and(|(e, _)| *e == effect);
    if let Some((_, flag)) = q.running.as_ref().filter(|_| running) {
        flag.store(true, Ordering::Relaxed);
    }
    let before = q.waiting.len();
    q.waiting.retain(|w| w.effect != effect);
    if q.waiting.len() != before || running {
        q.progress.insert(effect, Progress::Cancelled);
    }
}

/// Forget every reading and every filed analysis but `keep`'s: what closing a
/// project does. Running and waiting jobs for anything else are stopped.
pub fn forget(keep: impl Fn(&Uuid) -> bool) {
    {
        let mut q = queue();
        q.waiting.retain(|w| keep(&w.effect));
        if let Some((running, flag)) = &q.running {
            if !keep(running) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        q.progress.retain(|id, _| keep(id));
        q.asked.retain(|id, _| keep(id));
    }
    analysis::retain(&keep);
    lumit_core::stabilise::plan::retain(&keep);
    lumit_core::stabilise::rows::retain(&keep);
}

/// Every Stabilise instance in `doc`, either way round.
fn instances(doc: &Document) -> Vec<Uuid> {
    let mut out = Vec::new();
    for item in &doc.items {
        if let ProjectItem::Composition(comp) = item {
            for layer in &comp.layers {
                out.extend(
                    layer
                        .effects
                        .iter()
                        .filter(|e| e.effect.match_name == MATCH_NAME)
                        .map(|e| e.id),
                );
            }
        }
    }
    out
}

/// Drop everything `doc` holds: what closing its project does. Its running
/// and waiting analyses are stopped; the sidecar files stay.
pub fn forget_document(doc: &Document) {
    let mine = instances(doc);
    forget(|id| !mine.contains(id));
}

/// A project was opened: from now on a frame that finds a Stabilise with no
/// analysis asks for one, and every Stabilise in it is asked for now. Another
/// open project's analyses are its own, and only closing it lets them go
/// ([`forget_document`]), as the trackers' are.
pub fn opened(doc: &Arc<Document>) {
    analysis::on_missing(Box::new(wanted));
    for job in open_jobs(doc, None) {
        ask(job);
    }
}

/// What a frame calls when it draws a Stabilise with nothing filed: the job
/// for it, asked for once.
fn wanted(effect: Uuid, doc: &Arc<Document>, comp: Uuid, layer: Uuid) {
    // A run in hand, or one that ended without filing anything, is not asked
    // for again by a frame. Cheap, because every frame of a running analysis
    // comes through here.
    if progress(effect).is_some_and(|p| !matches!(p, Progress::Done)) {
        return;
    }
    let Some(comp) = doc.comp(comp) else {
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer) else {
        return;
    };
    let Some(fx) = stabilise_effect(layer).filter(|fx| fx.id == effect) else {
        return;
    };
    if let Some(job) = job_in(doc, comp, layer, fx) {
        ask(job);
    }
}

/// The Stabilise layers an export of `comp` draws, as `(comp, layer)` ids:
/// those on the layers it shows, through every Precomp it shows, on every
/// layer one of those reads as a track matte or through an effect's layer
/// row, hidden or not, and the stabilised layer each Reverse among them
/// reads, which may be a guide or hidden. `None` is every Stabilise in `doc`.
fn drawn(doc: &Document, comp: Option<Uuid>) -> Vec<(Uuid, Uuid)> {
    let Some(top) = comp else {
        return doc
            .items
            .iter()
            .filter_map(|i| match i {
                ProjectItem::Composition(c) => Some(c),
                _ => None,
            })
            .flat_map(|c| {
                c.layers
                    .iter()
                    .filter(|l| forward(l).is_some())
                    .map(move |l| (c.id, l.id))
            })
            .collect();
    };
    let mut out = Vec::new();
    let mut comps = vec![top];
    let mut seen_comps = Vec::new();
    let mut layers: Vec<(Uuid, Uuid)> = Vec::new();
    let mut seen = Vec::new();
    loop {
        // Every comp entered shows its visible layers that are not guides.
        while let Some(id) = comps.pop() {
            if seen_comps.contains(&id) {
                continue;
            }
            seen_comps.push(id);
            if let Some(c) = doc.comp(id) {
                layers.extend(
                    (c.layers.iter())
                        .filter(|l| l.switches.visible && !l.switches.guide)
                        .map(|l| (id, l.id)),
                );
            }
        }
        let Some((cid, lid)) = layers.pop() else {
            break;
        };
        if seen.contains(&(cid, lid)) {
            continue;
        }
        seen.push((cid, lid));
        let Some(c) = doc.comp(cid) else {
            continue;
        };
        let Some(l) = c.layers.iter().find(|l| l.id == lid) else {
            continue;
        };
        if let LayerKind::Precomp { comp: nested } = l.kind {
            comps.push(nested);
        }
        if forward(l).is_some() {
            out.push((cid, lid));
        }
        // A track matte is drawn, whether or not it is shown itself.
        if let Some(m) = &l.matte {
            layers.push((cid, m.layer));
        }
        let live = l.effects.iter().filter(|e| e.enabled && l.switches.fx);
        for e in live {
            if e.effect.match_name == MATCH_NAME && choice(e, "mode", 0) == MODE_REVERSE {
                if let Some((target, via)) = reversed(doc, c, l, e) {
                    let inner = match via.map(|p| &p.kind) {
                        Some(LayerKind::Precomp { comp: nested }) => *nested,
                        _ => cid,
                    };
                    if forward(target).is_some() {
                        out.push((inner, target.id));
                    }
                }
            }
            // A layer an effect's row reads for its picture is drawn too. A
            // row read for its layer's shapes, as Reverse's is, draws nothing.
            let def = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&e.effect.match_name);
            for p in &e.params {
                let EffectValue::Layer(Some(read)) = p.value else {
                    continue;
                };
                if def.is_none_or(|d| !d.shape_inputs().contains(&p.id.as_str())) {
                    layers.push((cid, read));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// How many of the Stabilise layers `of` are still waiting for their
/// analysis: running, and with nothing filed for what the layer shows.
fn pending(doc: &Document, of: &[(Uuid, Uuid)]) -> usize {
    of.iter()
        .filter(|(comp, layer)| {
            let Some(l) = doc
                .comp(*comp)
                .and_then(|c| c.layers.iter().find(|l| l.id == *layer))
            else {
                return false;
            };
            let Some(fx) = forward(l) else {
                return false;
            };
            let running = matches!(
                progress(fx.id),
                Some(Progress::Queued | Progress::Tracking { .. } | Progress::Solving)
            );
            let source = lumit_core::fx::effects::stabilise::source_of(l);
            running && analysis::filed(fx.id).is_none_or(|f| Some(f.source) != source)
        })
        .count()
}

/// What an export of `comp` does before its first frame: wait for every
/// Stabilise it draws to have its analysis, giving up when `cancel` is
/// raised.
pub fn hold_export(doc: &Arc<Document>, comp: Uuid, cancel: &AtomicBool) {
    let _ = wait_for(doc, Some(comp), &|| cancel.load(Ordering::Relaxed));
}

/// Hold an export of `comp` (every comp, for `None`) until every Stabilise it
/// draws has its analysis, so no frame of it goes out unsteadied. Asks for
/// any that are missing, then waits. `false` when `stop` said to give up.
pub fn wait_for(doc: &Arc<Document>, comp: Option<Uuid>, stop: &dyn Fn() -> bool) -> bool {
    let of = drawn(doc, comp);
    for job in open_jobs(doc, Some(&of)) {
        let source = job.source;
        if analysis::filed(job.effect).is_none_or(|f| f.source != source) {
            ask(job);
        }
    }
    loop {
        if pending(doc, &of) == 0 {
            return true;
        }
        if stop() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Clears the runner's mark however its thread ends. A runner that dies
/// inside a job marks that job failed, and starts another for whatever still
/// waits.
struct Runner;

impl Drop for Runner {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let again = {
            let mut q = queue();
            if let Some((effect, _)) = q.running.take() {
                q.progress
                    .insert(effect, Progress::Failed(AnalysisError::Unreadable));
            }
            q.runner = !q.waiting.is_empty();
            q.runner
        };
        if again && !spawn_runner() {
            queue().runner = false;
        }
    }
}

/// The one runner thread: takes the oldest waiting job, runs it, and carries
/// on until nothing waits. `false` when the thread could not be started.
fn spawn_runner() -> bool {
    std::thread::Builder::new()
        .name("lumit-stabilise".into())
        .spawn(|| {
            let _guard = Runner;
            loop {
                let (job, cancel) = {
                    let mut q = queue();
                    let Some(job) = q.waiting.pop_front() else {
                        q.running = None;
                        q.runner = false;
                        return;
                    };
                    let cancel = Arc::new(AtomicBool::new(false));
                    q.running = Some((job.effect, Arc::clone(&cancel)));
                    (job, cancel)
                };
                let effect = job.effect;
                let outcome = run(job, &cancel);
                let mut q = queue();
                // A job replaced while it ran reads as the replacement's
                // Queued, not as its own Cancelled.
                if !q.waiting.iter().any(|w| w.effect == effect) {
                    q.progress.insert(effect, outcome);
                }
                q.running = None;
            }
        })
        .is_ok()
}

/// One job: the sidecar if it has the answer and the job is not a press of
/// Analyse, and otherwise the analysis. An analysis that stopped short of
/// what was asked, a frame that would not decode, is filed for the session
/// but never written as the whole.
fn run(job: Job, cancel: &Arc<AtomicBool>) -> Progress {
    let dir = crate::track::cache_dir();
    if job.on_disk && !job.fresh {
        if let Some(found) = dir.as_deref().and_then(|d| read_sidecar(d, &job.key)) {
            analysis::publish(job.effect, job.key, job.source, found);
            return Progress::Done;
        }
    }
    let (effect, key, source, on_disk) = (job.effect, job.key, job.source, job.on_disk);
    match analyse(job, cancel, &|step| report(effect, step)) {
        Ok((found, whole)) => {
            if let (true, true, Some(d)) = (on_disk, whole, dir.as_deref()) {
                write_sidecar(d, &key, &found);
            }
            analysis::publish(effect, key, source, found);
            Progress::Done
        }
        Err(AnalysisError::Cancelled) => Progress::Cancelled,
        Err(e) => Progress::Failed(e),
    }
}

/// The tracker as a stabiliser drives it: patches that only slide, seeded from
/// the frame's own motion and refined on the two finest levels, in an
/// eleven-pixel window. About a tenth of the camera solve's cost per frame for
/// the same tracks.
#[must_use]
pub fn tracker_settings(density: u32) -> TrackSettings {
    let (across, down, per_bucket) = lumit_core::fx::effects::camera_track::density(density);
    TrackSettings {
        grid: (across, down),
        per_bucket,
        half_window: 5,
        seeded_levels: 2,
        affine: false,
        ..TrackSettings::default()
    }
}

/// The source frames, first and last, a `range` of source seconds covers in a
/// clip of `clip` frames at `fps`, picked as a layer picks the frame it shows.
fn frames_in(range: Option<(f64, f64)>, fps: f64, clip: usize) -> (usize, usize) {
    let pick = |t| lumit_core::pixels::frame_pick(t, fps, clip, false, None).0;
    range.map_or((0, clip.saturating_sub(1)), |(a, b)| (pick(a), pick(b)))
}

/// Read, follow and fit. A pure function of the frames and the settings.
/// Answers the analysis and whether every frame asked for was read.
fn analyse(
    job: Job,
    cancel: &Arc<AtomicBool>,
    report: &dyn Fn(Progress),
) -> Result<(Analysis, bool), AnalysisError> {
    let (range, rate) = (job.range, job.rate);
    let feed = frames::start(
        job.open,
        move |clip, fps| frames_in(range, rate.unwrap_or(fps), clip),
        Arc::clone(cancel),
    )
    .ok_or(AnalysisError::Unreadable)?;
    let o = feed.opened;
    let (first, last) = frames_in(job.range, job.rate.unwrap_or(o.fps), o.clip);
    let total = last.saturating_sub(first) + 1;
    let (tw, th) = o.tracked();
    if tw < 32 || th < 32 {
        return Err(AnalysisError::NoFrames);
    }
    // Masks arrive in layer pixels; the tracker works `div` decoded pixels to
    // one, and a nested comp is decoded smaller than its layer.
    let scale = o.to_layer * o.div as f64;
    let shrink = |masks: Vec<ExclusionMask>| -> Vec<ExclusionMask> {
        masks
            .into_iter()
            .map(|m| {
                let (contours, inverted) = m.outlines();
                ExclusionMask::from_contours(
                    contours
                        .iter()
                        .map(|c| c.iter().map(|p| to_tracked(*p, scale)).collect())
                        .collect(),
                    inverted,
                )
            })
            .collect()
    };
    let mut tracker = Tracker::new(tracker_settings(job.settings.density));
    let animated = job.masks.animated();
    if !animated {
        tracker.set_masks(shrink(job.masks.at(0.0)));
    }
    let rate = job.rate.unwrap_or(o.fps).max(1e-6);
    let mut changes: Vec<f32> = Vec::with_capacity(total);
    let mut previous: Option<[f32; 64]> = None;
    let mut done = 0usize;
    report(Progress::Tracking { done, total });
    for plane in feed.frames.iter() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        if animated {
            tracker.set_masks(shrink(job.masks.at(plane.n as f64 / rate)));
        }
        let fp = FramePlane::new(&plane.luma, tw, th).map_err(AnalysisError::Tracking)?;
        tracker
            .push(plane.n as i64, fp, None)
            .map_err(AnalysisError::Tracking)?;
        let sig = frames::signature(&plane.luma, tw, th);
        if let Some(prev) = previous {
            changes.push(frames::change(&prev, &sig));
        }
        previous = Some(sig);
        done += 1;
        if done.is_multiple_of(4) || done == total {
            report(Progress::Tracking { done, total });
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(AnalysisError::Cancelled);
    }
    report(Progress::Solving);
    let set = tracker.finish();
    let shape = Shape {
        fps: o.fps,
        rate_scale: job.rate_scale,
        decoded_from: first as i64,
        first: 0,
        asked: u32::try_from(total).unwrap_or(u32::MAX),
        width: (o.width as f64 * o.to_layer).round() as u32,
        height: (o.height as f64 * o.to_layer).round() as u32,
        scale,
    };
    let stop = || cancel.load(Ordering::Relaxed);
    let found =
        fit_analysis(&set, &shape, &frames::jumps(&changes), &stop).map_err(|e| match e {
            StabiliseError::Cancelled => AnalysisError::Cancelled,
            // Frames were read but nothing in them could be followed: an empty
            // comp, or a flat picture.
            StabiliseError::TooShort if done >= 2 => AnalysisError::Solve(SolveError::NoTracks),
            StabiliseError::TooShort => AnalysisError::NoFrames,
        })?;
    Ok((found, done == total))
}

// ---------------------------------------------------------------------------
// The sidecar
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Record {
    /// Repeated inside the file as well as in its name, so a renamed file is
    /// caught rather than believed.
    key: Key,
    analysis: Analysis,
}

/// [`Record`] as it is written, borrowing the analysis rather than copying
/// it: on a long clip that copy is hundreds of megabytes.
#[derive(serde::Serialize)]
struct Written<'a> {
    key: &'a Key,
    analysis: &'a Analysis,
}

fn file_name(key: &Key) -> String {
    let mut name = String::with_capacity(70);
    for byte in key {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push_str(".lstb");
    name
}

/// The analysis filed under `key`, if its file is there, is the one named and
/// holds together ([`Analysis::is_sound`]). A damaged file is read as none.
fn read_sidecar(dir: &std::path::Path, key: &Key) -> Option<Analysis> {
    let bytes = std::fs::read(dir.join(file_name(key))).ok()?;
    let body = crate::sidecar::unframe(&bytes, MAGIC, FORMAT_VERSION)?;
    let record: Record = bincode::deserialize(body).ok()?;
    (record.key == *key && record.analysis.is_sound()).then_some(record.analysis)
}

fn write_sidecar(dir: &std::path::Path, key: &Key, found: &Analysis) {
    let Ok(body) = bincode::serialize(&Written {
        key,
        analysis: found,
    }) else {
        return;
    };
    crate::sidecar::write(
        dir,
        &file_name(key),
        &crate::sidecar::frame(MAGIC, FORMAT_VERSION, &body),
    );
}

// ---------------------------------------------------------------------------
// Opening a project
// ---------------------------------------------------------------------------

/// Every Stabilise in `doc` as a job, or only those on the layers `of`:
/// footage ones read from the sidecar, and analysed if the sidecar has
/// nothing, since a Stabilise with no analysis draws its clip unsteadied.
/// Precomp ones are analysed afresh, their answer living only for a session.
/// Footage that is offline is skipped.
#[must_use]
pub fn open_jobs(doc: &Arc<Document>, of: Option<&[(Uuid, Uuid)]>) -> Vec<Job> {
    let mut out = Vec::new();
    for item in &doc.items {
        let ProjectItem::Composition(comp) = item else {
            continue;
        };
        for layer in &comp.layers {
            if of.is_some_and(|of| !of.contains(&(comp.id, layer.id))) {
                continue;
            }
            if let Some(fx) = stabilise_effect(layer) {
                out.extend(job_in(doc, comp, layer, fx));
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
