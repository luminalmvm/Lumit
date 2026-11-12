//! What one frame of a Stabilise is drawn with, worked out from the analysis
//! and the rows, and what names that frame in the frame key.
//!
//! In plain terms: taking the shake out, the frame's map comes from the plan,
//! with the Amount and the Extra scale on top, the neighbours a Fill lays under
//! it, and the frame's motion for the rolling shutter. Putting it back
//! (Reverse), it is the stabilised layer's map turned round and carried into
//! this layer's pixels. Those values read what no row of this effect holds:
//! the analysis, the stabilised layer's own rows, and where both layers sit.
//! So the frame key is handed them too ([`stamp`]), as Depth points hands it
//! the camera it resolved.

use std::sync::Arc;

use super::fill::neighbours;
use super::motion::{invert, mul, Mat3};
use super::place::layer_to_comp;
use super::rows::{choice, Reader};
use super::shown::{nearest, Shown};
use super::shutter::{motion_at, readout};
use crate::fx::effects::stabilise::{
    blend, dropped, frame_at, source_of, Planned, EDGES_DEFAULT, EDGES_FILL, MATCH_NAME,
    MODE_REVERSE,
};
use crate::model::{Composition, Document, EffectInstance, Layer, LayerKind};

/// One frame's warp, before it is handed to the GPU op.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// Output pixel to input pixel, in the layer's own pixels.
    pub map: Mat3,
    /// Crop's scale.
    pub crop: f64,
    /// The layer's pixels, which the map is in: the analysis's, or for a
    /// Reverse its own layer's frame.
    pub size: [f64; 2],
    /// Fill's neighbours: offset, analysed frame and map, farthest first.
    pub fill: Option<Vec<(i32, usize, Mat3)>>,
    /// The rolling shutter: the frame's motion while it was read, the read
    /// time with the Amount on it, and its axis. `None` while it is off.
    pub shutter: Option<([f64; 4], f64, f64)>,
    /// Each neighbour's motion while it was read, beside `fill`, while the
    /// rolling shutter is on.
    pub fill_motion: Vec<[f64; 4]>,
}

/// What the Stabilise `r` reads draws at layer time `lt`. `None` with nothing
/// to draw with, which leaves the picture as it came.
#[must_use]
pub fn resolve(r: &Reader<'_>, lt: f64) -> Option<Resolved> {
    if choice(r.inst, "mode", 0) == MODE_REVERSE {
        return reverse(r, lt);
    }
    let (fm, shown, p) = frame_at(r, lt)?;
    let size = p.size();
    let amount = r.amount(lt);
    let crop = 1.0 + (fm.crop - 1.0) * amount;
    // Fill follows the drawn map, so the picture holds together at any
    // Amount.
    let fill =
        (choice(r.inst, "edges", EDGES_DEFAULT) == EDGES_FILL).then(|| fill_at(r, lt, &p, shown));
    // The rolling shutter is scaled by the Amount as the map is, so nought
    // draws the clip exactly as shot.
    let shutter = (choice(r.inst, "rolling_shutter", 0) == 1)
        .then(|| readout(&p.filed.analysis))
        .flatten()
        .map(|(rt, axis)| (motion_at(&p.plan.motion, shown), rt * amount, axis))
        .filter(|s| s.1 != 0.0);
    let fill_motion = match (&fill, shutter) {
        (Some(maps), Some(_)) => maps
            .iter()
            .map(|(_, j, _)| motion_at(&p.plan.motion, Shown::single(*j)))
            .collect(),
        _ => Vec::new(),
    };
    Some(Resolved {
        map: r.drawn(lt, &fm, size),
        crop,
        size,
        fill,
        shutter,
        fill_motion,
    })
}

/// The neighbour maps a Fill draws layer time `lt` with, around the frame
/// `shown`: each neighbour's analysed frame picked as the decode picks the
/// frame it lays, from the layer time it shows at.
fn fill_at(r: &Reader<'_>, lt: f64, p: &Planned, shown: Shown) -> Vec<(i32, usize, Mat3)> {
    // The neighbours' layer times, as the frame key and the decode find them.
    let comp_dt = 1.0 / r.comp_fps();
    let a = &p.filed.analysis;
    let near: Vec<(i32, f64)> = super::fill::reads(r.inst, lt / comp_dt, r.comp_fps())
        .into_iter()
        .map(|o| (o, nearest(r.layer, lt + f64::from(o) * comp_dt, p.rate, a)))
        .collect();
    neighbours(&p.plan.steps, p.plan.frames, shown.nearest() as f64, &near)
}

/// Reverse: the reversed layer's map, exactly as that layer draws it, inverted
/// and carried into this layer's pixels. The rolling shutter is left out: it
/// is a few pixels at most, and work laid over the steadied plate never had
/// it.
fn reverse(r: &Reader<'_>, lt: f64) -> Option<Resolved> {
    let (doc, comp, layer) = (r.doc, r.comp, r.layer);
    let (target, via) = reversed(doc, comp, layer, r.inst)?;
    let fx = forward(target)?;
    // The moment the reversed layer shows when this one is at `lt`: out to
    // this comp's clock, then, through a Precomp, down into the nested comp's.
    let t_comp = lt + layer.start_offset.0.to_f64();
    let (inner, t_inner) = match via {
        Some(p) => {
            let LayerKind::Precomp { comp: nested } = p.kind else {
                return None;
            };
            let lt = crate::time::layer_time(t_comp, p.start_offset.0);
            (doc.comp(nested)?, p.source_time_at(lt))
        }
        None => (comp, t_comp),
    };
    let lt_target = crate::time::layer_time(t_inner, target.start_offset.0);
    let stabilised = Reader {
        doc,
        comp: inner,
        layer: target,
        inst: fx,
    };
    let (fm, _, p) = frame_at(&stabilised, lt_target)?;
    let back = invert(&stabilised.drawn(lt_target, &fm, p.size()))?;
    // `a` takes this layer's pixels to the stabilised layer's: out to this
    // comp, into a Precomp's pixels, which are its nested comp's, and into the
    // stabilised layer's own. The shake is put back in those pixels and
    // carried home again.
    let mut a = layer_to_comp(doc, comp, layer, t_comp)?;
    if let Some(p) = via {
        a = mul(&invert(&layer_to_comp(doc, comp, p, t_comp)?)?, &a);
    }
    a = mul(&invert(&layer_to_comp(doc, inner, target, t_inner)?)?, &a);
    let m = mul(&invert(&a)?, &mul(&back, &a));
    Some(Resolved {
        map: blend(&m, r.amount(lt)),
        crop: 1.0,
        // The map is in this layer's own pixels, so its own frame is what an
        // effect above that grows the raster leaves centred.
        size: own_size(doc, comp, layer, t_comp),
        fill: None,
        shutter: None,
        fill_motion: Vec::new(),
    })
}

/// `layer`'s own frame in its pixels, where it has one the raster starts as:
/// footage, a Precomp layer, a solid, a Sequence layer or an adjustment layer.
/// Nought otherwise, which takes the map about the raster's corner.
fn own_size(doc: &Arc<Document>, comp: &Composition, layer: &Layer, t: f64) -> [f64; 2] {
    let framed = matches!(
        layer.kind,
        LayerKind::Footage { .. }
            | LayerKind::Precomp { .. }
            | LayerKind::Solid { .. }
            | LayerKind::Sequence { .. }
            | LayerKind::Adjustment
    );
    let context = crate::expression::ExpressionContext {
        document: Arc::clone(doc),
        comp: Some(comp.id),
        layer: Some(layer.id),
        comp_time: t,
        current_depth: 0,
        inputs: None,
    };
    framed
        .then(|| crate::fx::drivers::measure::layer_rect(doc, comp, layer, t, &context))
        .flatten()
        .map_or([0.0; 2], |r| [r[2] - r[0], r[3] - r[1]])
}

/// The layer a Reverse on `layer` puts back: the named layer itself, or, for
/// a Precomp layer, the first layer inside it with a Stabilise taking the shake
/// out, together with that Precomp layer, whose clock leads into the nested
/// comp's.
#[must_use]
pub fn reversed<'a>(
    doc: &'a Document,
    comp: &'a Composition,
    layer: &'a Layer,
    inst: &EffectInstance,
) -> Option<(&'a Layer, Option<&'a Layer>)> {
    let named = inst.layer_ref("stabilised").unwrap_or(layer.id);
    let target = comp.layers.iter().find(|l| l.id == named)?;
    if target.id != layer.id && forward(target).is_some() {
        return Some((target, None));
    }
    if let LayerKind::Precomp { comp: nested } = target.kind {
        let inner = doc.comp(nested)?;
        let found = inner.layers.iter().find(|l| forward(l).is_some())?;
        return Some((found, Some(target)));
    }
    None
}

/// Whether `e` is a switched-on Stabilise taking the shake out: the kind that
/// analyses, draws a plan and can be baked.
#[must_use]
pub fn takes_out(e: &EffectInstance) -> bool {
    e.enabled && e.effect.match_name == MATCH_NAME && choice(e, "mode", 0) != MODE_REVERSE
}

/// The first Stabilise taking the shake out on `layer`, which is the one
/// that analyses.
#[must_use]
pub fn forward(layer: &Layer) -> Option<&EffectInstance> {
    layer.effects.iter().find(|e| takes_out(e))
}

/// What a frame `inst` draws on `layer` of `comp` at layer time `lt` reads
/// beyond its own rows: the analysis, named by what it was asked under and how
/// far it got, the points dropped from it, and the warp resolved from them.
/// The warp covers what the plan reads off its rows at other moments, a
/// Reverse's stabilised layer and its rows, and where both layers sit. A
/// Reverse names the reversed layer's analysis. `None` with nothing in hand.
#[must_use]
pub fn stamp(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
    lt: f64,
) -> Option<[u8; 32]> {
    let (target, fx) = if choice(inst, "mode", 0) == MODE_REVERSE {
        let (target, _) = reversed(doc, comp, layer, inst)?;
        (target, forward(target)?)
    } else {
        (layer, inst)
    };
    let filed = super::analysis::filed(fx.id)?;
    if Some(filed.source) != source_of(target) {
        return None;
    }
    let mut h = blake3::Hasher::new();
    h.update(&filed.key);
    h.update(&filed.points);
    h.update(&filed.analysis.frames.to_le_bytes());
    for d in dropped(fx, &filed.points) {
        h.update(&d.to_le_bytes());
    }
    let mut feed = |v: &[f64]| {
        for x in v {
            h.update(&x.to_le_bytes());
        }
    };
    let reader = Reader {
        doc,
        comp,
        layer,
        inst,
    };
    if let Some(r) = resolve(&reader, lt) {
        for row in r.map {
            feed(&row);
        }
        feed(&[r.crop, r.size[0], r.size[1]]);
        for (o, j, m) in r.fill.iter().flatten() {
            feed(&[f64::from(*o), *j as f64]);
            for row in m {
                feed(row);
            }
        }
        if let Some((m, rt, axis)) = r.shutter {
            feed(&m);
            feed(&[rt, axis]);
        }
        for m in &r.fill_motion {
            feed(m);
        }
    }
    Some(*h.finalize().as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::anim::Property;
    use crate::model::{EffectValue, LinearColour, ProjectItem, TransformGroup};
    use crate::stabilise::analysis::{publish, Analysis, Step};
    use crate::stabilise::motion::{apply, IDENTITY};
    use crate::time::{Duration, FrameRate, Rational};
    use uuid::Uuid;

    fn set(inst: &mut EffectInstance, id: &str, value: EffectValue) {
        if let Some(p) = inst.params.iter_mut().find(|p| p.id == id) {
            p.value = value;
        }
    }

    /// Reverse puts back exactly what the stabilised layer took out, its
    /// Amount and Extra scale included: a point drawn through both comes back
    /// where it started.
    #[test]
    fn reverse_puts_back_what_a_stabilise_at_half_amount_took_out() {
        let footage = Uuid::now_v7();
        let layer = |name: &str, kind| {
            crate::edits::base_layer(
                name.into(),
                kind,
                Rational::new(10, 1).unwrap(),
                TransformGroup::default(),
            )
        };
        let mut shot = layer("shot", LayerKind::Footage { item: footage });
        let mut fx = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        set(&mut fx, "amount", EffectValue::Float(Property::fixed(50.0)));
        set(
            &mut fx,
            "extra_scale",
            EffectValue::Float(Property::fixed(110.0)),
        );
        shot.effects.push(fx.clone());
        let mut work = layer("work", LayerKind::Null);
        let mut back = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        set(&mut back, "mode", EffectValue::Choice(MODE_REVERSE));
        set(&mut back, "stabilised", EffectValue::Layer(Some(shot.id)));
        work.effects.push(back.clone());
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 64,
            height: 36,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: vec![work.clone(), shot.clone()],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp.clone()));
        let doc = Arc::new(doc);
        // A shake of five pixels either way, frame by frame.
        let steps = (0..9)
            .map(|i| Step {
                similarity: [1.0, 0.0, if i % 2 == 0 { 5.0 } else { -5.0 }, 0.0],
                homography: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                kept: 100,
                kept_similarity: 100,
                seen: 100,
                cut: false,
            })
            .collect();
        let analysis = Analysis {
            fps: 30.0,
            rate_scale: 1.0,
            width: 64,
            height: 36,
            scale: 1.0,
            first: 0,
            frames: 10,
            asked: 10,
            steps,
            runs: Vec::new(),
            readout: [0.0; 2],
        };
        publish(fx.id, [3; 32], footage, analysis);
        let reader = |layer, inst| Reader {
            doc: &doc,
            comp: &comp,
            layer,
            inst,
        };
        let lt = 3.0 / 30.0;
        let taken = resolve(&reader(&shot, &fx), lt).unwrap().map;
        let put = resolve(&reader(&work, &back), lt).unwrap().map;
        assert_ne!(taken, IDENTITY, "the shot is steadied");
        for p in [[10.0, 10.0], [50.0, 5.0], [32.0, 30.0]] {
            let q = apply(&taken, apply(&put, p).unwrap()).unwrap();
            assert!(
                (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-6,
                "{p:?} came back at {q:?}"
            );
        }
    }

    /// The status line's scale comes from a plan no frame has drawn: the read
    /// says it is being made, and it is made without one.
    #[test]
    fn the_scale_is_made_ready_without_a_frame_being_drawn() {
        use crate::fx::effects::stabilise::{needed_scale, Held};
        let footage = Uuid::now_v7();
        let mut shot = crate::edits::base_layer(
            "shot".into(),
            LayerKind::Footage { item: footage },
            Rational::new(10, 1).unwrap(),
            TransformGroup::default(),
        );
        let fx = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        shot.effects.push(fx.clone());
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 64,
            height: 36,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: vec![shot.clone()],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp.clone()));
        let doc = Arc::new(doc);
        // A shake of five pixels either way, whose edges Scale has to hide.
        let steps = (0..9)
            .map(|i| Step {
                similarity: [1.0, 0.0, if i % 2 == 0 { 5.0 } else { -5.0 }, 0.0],
                homography: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                kept: 100,
                kept_similarity: 100,
                seen: 100,
                cut: false,
            })
            .collect();
        let analysis = Analysis {
            fps: 30.0,
            rate_scale: 1.0,
            width: 64,
            height: 36,
            scale: 1.0,
            first: 0,
            frames: 10,
            asked: 10,
            steps,
            runs: Vec::new(),
            readout: [0.0; 2],
        };
        publish(fx.id, [4; 32], footage, analysis);
        let read = || needed_scale(&doc, &comp, &shot, &fx);
        assert_eq!(read(), Held::Making, "no frame has made the plan yet");
        let made = (0..500).find_map(|_| match read() {
            Held::Ready(s) => Some(s),
            _ => {
                std::thread::sleep(std::time::Duration::from_millis(10));
                None
            }
        });
        let (scale, _) = made.expect("the plan is made on a worker");
        assert!(scale > 1.0, "the shake needs scale: {scale}");
    }
}
