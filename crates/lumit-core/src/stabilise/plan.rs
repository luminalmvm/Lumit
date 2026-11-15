//! The plan: an analysis and the effect's rows turned into what every frame of
//! the clip is drawn with.
//!
//! In plain terms: this is the quick half. Changing Smoothness, the Method or
//! the edges never re-reads the clip; it reworks the path and the scale from
//! the analysis already in hand, and the frames are drawn again. Plans are kept
//! for the last few row settings of each effect, so scrubbing a slider back to
//! where it was costs nothing.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use uuid::Uuid;

use super::analysis::{Analysis, Step};
use super::frame::{frame, Framed, Framing};
use super::motion::{
    close_px, fit_similarity_from, four_corner, from_rows, from_similarity, within, Mat3, Pair,
    IDENTITY,
};
use super::path::{corrections, sigma_frames, Model, Smoothing};
use super::shown::Shown;
use crate::anim::Keyframe;

/// The smoothing as the Smoothness row holds it.
#[derive(Debug, Clone, PartialEq)]
pub enum Smoothness {
    /// One standard deviation, in frames, for the whole shot.
    Even(f64),
    /// The row's keys, read at the layer time each analysed frame shows at:
    /// the source frame `first + i` at `rate`, less `start`, the source time
    /// the layer starts on.
    Keyed {
        keys: Vec<Keyframe>,
        rate: f64,
        start: f64,
        first: i64,
    },
}

impl Smoothness {
    /// The standard deviation in frames on each of `frames` analysed frames,
    /// the percentage held to the row's hard range.
    #[must_use]
    pub fn sigmas(&self, frames: usize) -> Vec<f64> {
        match self {
            Smoothness::Even(s) => vec![*s; frames],
            Smoothness::Keyed {
                keys,
                rate,
                start,
                first,
            } => {
                let rate = rate.max(1.0);
                (0..frames)
                    .map(|i| {
                        let lt = (*first as f64 + i as f64) / rate - start;
                        let pct = crate::anim::evaluate(keys, lt).unwrap_or(50.0);
                        sigma_frames(pct.clamp(0.0, 300.0), rate)
                    })
                    .collect()
            }
        }
    }

    fn feed(&self, h: &mut DefaultHasher) {
        match self {
            Smoothness::Even(s) => {
                0u8.hash(h);
                s.to_bits().hash(h);
            }
            Smoothness::Keyed {
                keys,
                rate,
                start,
                first,
            } => {
                1u8.hash(h);
                serde_json::to_vec(keys).unwrap_or_default().hash(h);
                rate.to_bits().hash(h);
                start.to_bits().hash(h);
                first.hash(h);
            }
        }
    }
}

/// Everything a plan depends on beyond the analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRows {
    pub model: Model,
    /// No motion: the source frame every frame is locked to.
    pub lock: Option<i64>,
    pub smoothing: Smoothness,
    pub keep_scale: bool,
    pub framing: Framing,
    /// The analysed frames the layer shows, first and last, which are the only
    /// ones the scale is chosen for.
    pub shown: (usize, usize),
    /// The specks the user dropped, ascending.
    pub dropped: Vec<u32>,
}

impl PlanRows {
    /// A hash of every field, for the memo: the id the plan made from these
    /// rows under `generation` carries.
    #[must_use]
    pub fn key(&self, generation: u64) -> u64 {
        let mut h = DefaultHasher::new();
        generation.hash(&mut h);
        (self.model as u8).hash(&mut h);
        self.lock.hash(&mut h);
        self.smoothing.feed(&mut h);
        self.keep_scale.hash(&mut h);
        (self.framing.edges as u8).hash(&mut h);
        self.framing.max_scale.to_bits().hash(&mut h);
        self.framing.margin.to_bits().hash(&mut h);
        self.framing.smooth_more.to_bits().hash(&mut h);
        self.framing.fps.to_bits().hash(&mut h);
        self.shown.hash(&mut h);
        self.dropped.hash(&mut h);
        h.finish()
    }
}

/// What one analysed frame is drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMap {
    /// Output pixel to the input pixel it shows, scale included, in the
    /// analysis's pixels.
    pub map: Mat3,
    /// The scale the frame is drawn at.
    pub scale: f64,
    /// For Crop: the scale whose centred frame the picture is cut to. One
    /// otherwise.
    pub crop: f64,
}

/// A finished plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// A hash of the analysis generation and the rows, unique enough to name
    /// the frames drawn with it.
    pub id: u64,
    pub first: i64,
    pub frames: usize,
    pub size: [f64; 2],
    /// Output pixel to input pixel per analysed frame, before the scale.
    pub maps: Vec<Mat3>,
    pub framed: Framed,
    /// The steps the plan was made from, after any refit for dropped specks.
    pub steps: Arc<Vec<Step>>,
    /// How the picture moved while each analysed frame was read, for the
    /// rolling shutter: see [`super::shutter::motions`].
    pub motion: Vec<[f64; 4]>,
}

impl Plan {
    /// What the analysed frames `shown` are drawn with: one frame's map, or,
    /// where the layer blends two frames, the two maps mixed by the same
    /// weight.
    #[must_use]
    pub fn at(&self, shown: Shown) -> Option<FrameMap> {
        if shown.at >= self.frames || shown.blend.is_some_and(|(b, _)| b >= self.frames) {
            return None;
        }
        let one = |i: usize| {
            (
                self.maps.get(i).copied().unwrap_or(IDENTITY),
                self.framed.scale.get(i).copied().unwrap_or(1.0),
                self.framed.crop.get(i).copied().unwrap_or(1.0),
            )
        };
        let (map, scale, crop) = shown.mix(one, |a, b, w| {
            let mix = |x: f64, y: f64| x + (y - x) * w;
            let map: Mat3 =
                std::array::from_fn(|r| std::array::from_fn(|c| mix(a.0[r][c], b.0[r][c])));
            (map, mix(a.1, b.1), mix(a.2, b.2))
        });
        Some(FrameMap {
            map: super::motion::mul(&map, &zoom(self.size, scale)),
            scale,
            crop,
        })
    }
}

/// Output pixel to output pixel: the picture scaled up by `z` about the centre.
#[must_use]
pub fn zoom(size: [f64; 2], z: f64) -> Mat3 {
    let z = if z.is_finite() && z > 0.0 { z } else { 1.0 };
    let (cx, cy) = (0.5 * size[0], 0.5 * size[1]);
    [
        [1.0 / z, 0.0, cx - cx / z],
        [0.0, 1.0 / z, cy - cy / z],
        [0.0, 0.0, 1.0],
    ]
}

/// The steps with every step a dropped speck took part in fitted again
/// without it.
///
/// The refit keeps to the layer each step was measured from (the specks its
/// [`super::analysis::Run::agreed`] bits name), less the dropped ones, so a
/// drop takes specks out of the picture's layer and never sends the step
/// looking for a different one.
///
/// Linear in the clip: the specks are indexed by the frames they live on
/// once, so each step reads only those alive round it.
#[must_use]
pub fn refit(analysis: &Analysis, dropped: &[u32]) -> Vec<Step> {
    let mut steps = analysis.steps.clone();
    if dropped.is_empty() {
        return steps;
    }
    let gone = |id: u32| dropped.binary_search(&id).is_ok();
    let (agree, close) = (analysis.agree_px(), close_px(analysis.scale));
    let size = [f64::from(analysis.width), f64::from(analysis.height)];
    // The steps the dropped specks crossed, and the kept specks alive in each
    // stretch of frames, in id order.
    let mut touched = vec![false; steps.len()];
    let mut alive: Vec<Vec<usize>> = vec![Vec::new(); (analysis.frames / BUCKET + 1) as usize];
    for (k, run) in analysis.runs.iter().enumerate() {
        if gone(run.id) {
            for f in run.first..run.last() {
                if let Some(t) = touched.get_mut(f as usize) {
                    *t = true;
                }
            }
        } else {
            for b in (run.first / BUCKET)..=(run.last() / BUCKET) {
                if let Some(list) = alive.get_mut(b as usize) {
                    list.push(k);
                }
            }
        }
    }
    let point = |p: [f32; 2]| [f64::from(p[0]), f64::from(p[1])];
    for (i, step) in steps.iter_mut().enumerate() {
        if !touched[i] || step.cut {
            continue;
        }
        let (a, b) = (i as u32, i as u32 + 1);
        let mut pairs: Vec<Pair> = Vec::new();
        let mut layer: Vec<Pair> = Vec::new();
        let near = alive.get((a / BUCKET) as usize).map_or(&[][..], |v| &v[..]);
        for r in near.iter().filter_map(|&k| analysis.runs.get(k)) {
            let (Some(p), Some(q)) = (r.at(a), r.at(b)) else {
                continue;
            };
            pairs.push((point(p), point(q)));
            if r.agreed_at(a) {
                layer.push((point(p), point(q)));
            }
        }
        let seen = u32::try_from(pairs.len()).unwrap_or(u32::MAX);
        // Started from the step as analysed, so the refit stays on the
        // layer's own motion.
        let Some((sim, _)) = fit_similarity_from(&layer, step.similarity, close) else {
            *step = Step {
                seen,
                ..Step::STILL
            };
            continue;
        };
        let hom = four_corner(&layer, sim, size, close);
        let (m, ms) = (from_rows(hom), from_similarity(sim));
        let count = |m: &Mat3| {
            let n = pairs.iter().filter(|p| within(m, p, agree)).count();
            u32::try_from(n).unwrap_or(u32::MAX)
        };
        *step = Step {
            similarity: sim,
            homography: hom,
            kept: count(&m),
            kept_similarity: count(&ms),
            seen,
            cut: false,
        };
    }
    steps
}

/// Frames to a stretch of the index [`refit`] walks.
const BUCKET: u32 = 16;

/// The refits in hand, newest last: the analysis generation, the dropped
/// list, and the steps they gave.
type Refits = Vec<(u64, Vec<u32>, Arc<Vec<Step>>)>;

fn refits() -> &'static Mutex<Refits> {
    static REFITS: OnceLock<Mutex<Refits>> = OnceLock::new();
    REFITS.get_or_init(|| Mutex::new(Vec::new()))
}

/// How many refits are kept: one for each Stabilise being worked on, and a
/// few more.
const KEPT_REFITS: usize = 8;

/// [`refit`] for the analysis filed at `generation`, kept by its dropped list,
/// so a row change that makes a new plan reuses it.
#[must_use]
pub fn refit_held(analysis: &Analysis, generation: u64, dropped: &[u32]) -> Arc<Vec<Step>> {
    if dropped.is_empty() {
        return Arc::new(analysis.steps.clone());
    }
    let same = |g: u64, d: &[u32]| g == generation && d == dropped;
    if let Ok(held) = refits().lock() {
        if let Some((_, _, steps)) = held.iter().find(|(g, d, _)| same(*g, d)) {
            return Arc::clone(steps);
        }
    }
    // Made outside the lock, as a plan is.
    let steps = Arc::new(refit(analysis, dropped));
    if let Ok(mut held) = refits().lock() {
        held.retain(|(g, d, _)| !same(*g, d));
        held.push((generation, dropped.to_vec(), Arc::clone(&steps)));
        if held.len() > KEPT_REFITS {
            held.remove(0);
        }
    }
    steps
}

/// The motion a step's specks are judged against for `model`: the four-corner
/// fit for Perspective where it is trusted, and the similarity otherwise.
/// `None` where the step has no usable motion.
#[must_use]
pub fn judged_by(step: &Step, model: Model) -> Option<Mat3> {
    if model == Model::Perspective && step.trusted() {
        Some(from_rows(step.homography))
    } else if step.trusted_similarity() {
        Some(from_similarity(step.similarity))
    } else {
        None
    }
}

/// Make the plan for `analysis` under `rows`.
#[must_use]
pub fn build(analysis: &Analysis, generation: u64, rows: &PlanRows) -> Plan {
    let frames = analysis.frames as usize;
    let size = [f64::from(analysis.width), f64::from(analysis.height)];
    let steps = refit_held(analysis, generation, &rows.dropped);
    let centre = [0.5 * size[0], 0.5 * size[1]];
    let corners = [
        [0.0, 0.0],
        [size[0], 0.0],
        [0.0, size[1]],
        [size[0], size[1]],
    ];
    let last = i64::from(analysis.frames).saturating_sub(1).max(0);
    let lock = rows
        .lock
        .map(|f| u32::try_from((f - analysis.first).clamp(0, last)).unwrap_or(0));
    let sigma = rows.smoothing.sigmas(frames);
    // An analysis with no frames, which a damaged file could still name, has
    // nothing to draw: an empty plan, and no frame of the path is read.
    let maps = if frames == 0 {
        Vec::new()
    } else {
        corrections(
            &steps,
            frames,
            rows.model,
            &Smoothing {
                sigma: &sigma,
                lock,
                keep_scale: rows.keep_scale,
            },
            centre,
            corners,
        )
    };
    let framed = frame(&maps, size, &rows.framing, rows.shown);
    let motion = super::shutter::motions(&steps, frames);
    Plan {
        id: rows.key(generation),
        first: analysis.first,
        frames,
        size,
        maps,
        framed,
        steps,
        motion,
    }
}

/// The plans in hand, by instance: the last few for each, newest last.
fn plans() -> &'static Mutex<HashMap<Uuid, Vec<Arc<Plan>>>> {
    static PLANS: OnceLock<Mutex<HashMap<Uuid, Vec<Arc<Plan>>>>> = OnceLock::new();
    PLANS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How many plans an instance keeps: the settings it has now and a couple it
/// was at a moment ago, so a slider scrubbed back and forth costs nothing.
const KEPT_PLANS: usize = 3;

/// The plan for `effect`'s `analysis` (filed at `generation`) under `rows`,
/// made now if it is not already in hand.
#[must_use]
pub fn plan_for(effect: Uuid, analysis: &Analysis, generation: u64, rows: &PlanRows) -> Arc<Plan> {
    let id = rows.key(generation);
    if let Ok(held) = plans().lock() {
        if let Some(p) = held
            .get(&effect)
            .and_then(|v| v.iter().find(|p| p.id == id))
        {
            return Arc::clone(p);
        }
    }
    // Made outside the lock: two workers asking at once both make it, and the
    // second one filed simply replaces an identical first.
    let plan = Arc::new(build(analysis, generation, rows));
    if let Ok(mut held) = plans().lock() {
        let list = held.entry(effect).or_default();
        list.retain(|p| p.id != id);
        list.push(Arc::clone(&plan));
        if list.len() > KEPT_PLANS {
            list.remove(0);
        }
    }
    plan
}

/// The plan already in hand for `effect` with id `id`: what the status line
/// reads, since it never makes one.
#[must_use]
pub fn held(effect: Uuid, id: u64) -> Option<Arc<Plan>> {
    plans()
        .lock()
        .ok()?
        .get(&effect)?
        .iter()
        .find(|p| p.id == id)
        .cloned()
}

/// A plan waiting to be made: the analysis, its generation and the rows.
type Wanted = (Arc<Analysis>, u64, PlanRows);

/// The effects a worker is making a plan for, each with the plan asked for
/// since it started, latest wins. An entry is the worker's mark.
fn making() -> &'static Mutex<HashMap<Uuid, Option<Wanted>>> {
    static MAKING: OnceLock<Mutex<HashMap<Uuid, Option<Wanted>>>> = OnceLock::new();
    MAKING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Clears an effect's mark when its worker dies, so the next read starts
/// another.
struct Making(Uuid);

impl Drop for Making {
    fn drop(&mut self) {
        if std::thread::panicking() {
            if let Ok(mut m) = making().lock() {
                m.remove(&self.0);
            }
        }
    }
}

/// Make the plan for `effect`'s `analysis` (filed at `generation`) under
/// `rows` on a short-lived worker, unless it is in hand: how a read on the
/// interface thread, which never makes one, gets it made. One worker per
/// effect; rows asked for while it works replace each other, and the last is
/// made next. Nothing is made if no thread can be started, and the next frame
/// drawn makes it.
pub fn make_later(effect: Uuid, analysis: Arc<Analysis>, generation: u64, rows: PlanRows) {
    if held(effect, rows.key(generation)).is_some() {
        return;
    }
    let first = (analysis, generation, rows);
    {
        let Ok(mut m) = making().lock() else {
            return;
        };
        if let Some(next) = m.get_mut(&effect) {
            *next = Some(first);
            return;
        }
        m.insert(effect, None);
    }
    let spawned = std::thread::Builder::new()
        .name("lumit-stabilise-plan".into())
        .spawn(move || {
            let _mark = Making(effect);
            let mut job = Some(first);
            while let Some((analysis, generation, rows)) = job {
                let _ = plan_for(effect, &analysis, generation, &rows);
                // The mark goes under the same lock that finds nothing next,
                // so a request in between is never lost.
                let Ok(mut m) = making().lock() else {
                    return;
                };
                job = m.get_mut(&effect).and_then(Option::take);
                if job.is_none() {
                    m.remove(&effect);
                }
            }
        });
    if spawned.is_err() {
        if let Ok(mut m) = making().lock() {
            m.remove(&effect);
        }
    }
}

/// Drop every plan `keep` does not ask for.
pub fn retain(keep: impl Fn(&Uuid) -> bool) {
    if let Ok(mut held) = plans().lock() {
        held.retain(|id, _| keep(id));
    }
}
