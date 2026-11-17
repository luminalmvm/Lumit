//! Stabilise's rows, read the way the engine evaluates them, and the rows a
//! plan is made from.
//!
//! In plain terms: a row with keys is read at its time and a row with an
//! expression is run with the layer's own context, as every other effect's
//! rows are. The rows that shape the whole shot are read once per plan, never
//! once per analysed frame: Smoothness with keys hands its keys over, and an
//! expression on it is run once.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use uuid::Uuid;

use super::analysis::{Analysis, Filed};
use super::frame::{Edges, Framing};
use super::motion::{mul, Mat3};
use super::path::{sigma_frames, Model};
use super::plan::{zoom, FrameMap, PlanRows, Smoothness};
use super::shown::shown;
use crate::anim::Animation;
use crate::expression::ExpressionContext;
use crate::fx::effects::stabilise::{
    blend, dropped, EDGES_DEFAULT, EDGES_FILL, METHOD_DEFAULT, RESULT_NO_MOTION,
};
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer};

/// A choice row's index, or `fallback` where the row is missing.
#[must_use]
pub fn choice(inst: &EffectInstance, id: &str, fallback: u32) -> u32 {
    match inst.param(id) {
        Some(EffectValue::Choice(v)) => *v,
        _ => fallback,
    }
}

/// One Stabilise's rows on one layer of one comp.
pub struct Reader<'a> {
    pub doc: &'a Arc<Document>,
    pub comp: &'a Composition,
    pub layer: &'a Layer,
    pub inst: &'a EffectInstance,
}

impl Reader<'_> {
    /// Row `id` at layer time `lt`: its keys read there, its expression run
    /// with the layer's context at that moment. `fallback` for a missing row.
    #[must_use]
    pub fn float(&self, id: &str, lt: f64, fallback: f64) -> f64 {
        let Some(EffectValue::Float(p)) = self.inst.param(id) else {
            return fallback;
        };
        match p.animation {
            Animation::Expression(_) => {
                let context = ExpressionContext {
                    document: Arc::clone(self.doc),
                    comp: Some(self.comp.id),
                    layer: Some(self.layer.id),
                    comp_time: lt + self.layer.start_offset.0.to_f64(),
                    current_depth: 0,
                    inputs: None,
                };
                p.value_at_with_context(lt, Arc::new(context))
            }
            _ => p.value_at(lt),
        }
    }

    /// The comp's frame rate.
    #[must_use]
    pub fn comp_fps(&self) -> f64 {
        self.comp.frame_rate.fps().max(1.0)
    }

    /// The Amount at layer time `lt`, as a share.
    #[must_use]
    pub fn amount(&self, lt: f64) -> f64 {
        (self.float("amount", lt, 100.0) / 100.0).clamp(0.0, 1.0)
    }

    /// The Extra scale at layer time `lt`, as a factor, held to the row's hard
    /// range: an imported project can carry more.
    #[must_use]
    pub fn extra_scale(&self, lt: f64) -> f64 {
        (self.float("extra_scale", lt, 100.0) / 100.0).clamp(0.01, 10.0)
    }

    /// The map a frame is drawn with at layer time `lt`, from the plan's
    /// `fm` in a `size` frame: the Extra scale on it, then the Amount. The one
    /// expression taking the shake out, putting it back, placing the points
    /// and baking keys all read.
    #[must_use]
    pub fn drawn(&self, lt: f64, fm: &FrameMap, size: [f64; 2]) -> Mat3 {
        let map = mul(&fm.map, &zoom(size, self.extra_scale(lt)));
        blend(&map, self.amount(lt))
    }

    /// The rows a plan is made from for the analysis `filed`, its source
    /// playing at `rate`. Each is held to its hard range.
    #[must_use]
    pub fn plan_rows(&self, filed: &Filed, rate: f64) -> PlanRows {
        let a = &filed.analysis;
        let inst = self.inst;
        let model = match choice(inst, "method", METHOD_DEFAULT) {
            0 => Model::Position,
            1 => Model::Similarity,
            _ => Model::Perspective,
        };
        let lock = (choice(inst, "result", 0) == RESULT_NO_MOTION).then(|| {
            let src = self.layer.source_time_at(self.float("reference", 0.0, 0.0));
            (src * rate).round() as i64
        });
        let edges = match choice(inst, "edges", EDGES_DEFAULT) {
            0 => Edges::Show,
            1 => Edges::Crop,
            EDGES_FILL => Edges::Fill,
            _ => Edges::CropAndScale,
        };
        let pct = |id: &str, fallback: f64| self.float(id, 0.0, fallback) / 100.0;
        PlanRows {
            model,
            lock,
            smoothing: self.smoothing(a, rate),
            keep_scale: inst.bool_of("keep_scale").unwrap_or(false),
            framing: Framing {
                edges,
                max_scale: pct("max_scale", 150.0).clamp(1.0, 10.0),
                margin: pct("margin", 0.0).clamp(0.0, 0.45),
                smooth_more: pct("steady_crop", 100.0).clamp(0.0, 1.0),
                fps: rate,
            },
            shown: shown_span(self.layer, self.comp_fps(), rate, a, inst.id),
            dropped: dropped(inst, &filed.points),
        }
    }

    /// Smoothness: its keys handed over whole, so a plan expands them once,
    /// and a still number or an expression read once.
    fn smoothing(&self, a: &Analysis, rate: f64) -> Smoothness {
        if let Some(EffectValue::Float(p)) = self.inst.param("smoothness") {
            if let Animation::Keyframed(keys) = &p.animation {
                return Smoothness::Keyed {
                    keys: keys.clone(),
                    rate,
                    // ponytail: exact without a Retime; a retimed layer reads
                    // its keys at the source's own clock.
                    start: self.layer.source_time_at(0.0),
                    first: a.first,
                };
            }
        }
        let pct = self.float("smoothness", 0.0, 50.0).clamp(0.0, 300.0);
        Smoothness::Even(sigma_frames(pct, rate))
    }
}

/// The first and last analysed frames `layer` shows between its In and Out
/// points in a comp at `comp_fps`, its source playing at `rate`. Without a
/// Retime the source runs one way, so the two ends settle it; with one, every
/// comp frame is read, since a Retime can run out and back. That walk is kept
/// per instance until the layer's timing changes.
fn shown_span(
    layer: &Layer,
    comp_fps: f64,
    rate: f64,
    a: &Analysis,
    effect: Uuid,
) -> (usize, usize) {
    let last = (a.frames as usize).saturating_sub(1);
    let first_frame = (layer.in_point.0.to_f64() * comp_fps - 1e-9).ceil() as i64;
    let last_frame =
        ((layer.out_point.0.to_f64() * comp_fps - 1e-9).ceil() as i64 - 1).max(first_frame);
    let at = |n: i64| {
        let lt = crate::time::layer_time(n as f64 / comp_fps, layer.start_offset.0);
        shown(layer, comp_fps, lt, rate, a).map(|s| {
            let b = s.blend.map_or(s.at, |(b, _)| b);
            (s.at.min(b), s.at.max(b))
        })
    };
    let join = |acc: Option<(usize, usize)>, s: Option<(usize, usize)>| match (acc, s) {
        (Some(x), Some(y)) => Some((x.0.min(y.0), x.1.max(y.1))),
        (x, y) => x.or(y),
    };
    let span = if layer.retime.is_none() {
        join(at(first_frame), at(last_frame))
    } else {
        let mut h = DefaultHasher::new();
        (first_frame, last_frame, a.first, a.frames).hash(&mut h);
        (comp_fps.to_bits(), rate.to_bits()).hash(&mut h);
        layer.start_offset.0.to_f64().to_bits().hash(&mut h);
        serde_json::to_vec(&layer.retime)
            .unwrap_or_default()
            .hash(&mut h);
        let sig = h.finish();
        if let Some(held) = spans().lock().ok().and_then(|m| m.get(&effect).copied()) {
            if held.0 == sig {
                return held.1;
            }
        }
        let walked = (first_frame..=last_frame).fold(None, |acc, n| join(acc, at(n)));
        let found = walked.unwrap_or((0, last));
        if let Ok(mut m) = spans().lock() {
            m.insert(effect, (sig, found));
        }
        Some(found)
    };
    span.map_or((0, last), |(x, y)| (x.min(last), y.min(last)))
}

/// A walked span, first and last analysed frame, with a hash of the timing it
/// was walked under, by instance.
type Spans = HashMap<Uuid, (u64, (usize, usize))>;

/// The walked spans in hand, by instance, with what they were walked under.
/// One entry per retimed Stabilise drawn, dropped with the project.
fn spans() -> &'static Mutex<Spans> {
    static SPANS: OnceLock<Mutex<Spans>> = OnceLock::new();
    SPANS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drop every walked span `keep` does not ask for.
pub fn retain(keep: impl Fn(&Uuid) -> bool) {
    if let Ok(mut m) = spans().lock() {
        m.retain(|id, _| keep(id));
    }
}
