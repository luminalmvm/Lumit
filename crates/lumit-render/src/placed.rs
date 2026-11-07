//! A comp shown at more than one time in one frame.
//!
//! Two Precomp layers of one comp, or two clips of it on a Sequence layer, can
//! show it at two times at once. The footage inside is one layer with one id,
//! wanted at two frames. So each job says which walk of its comp asked for it,
//! and the build finds a walk's own pixels by that time.
//!
//! An ordinary frame has one job a layer, and all of this checks an empty list.

use std::collections::HashMap;

use lumit_core::comp_graph::GraphNode;
use lumit_core::model::{Composition, Document, EffectInstance};
use uuid::Uuid;

use crate::decode::{CompJob, CompLayerPixels};

/// A frame's plan with each layer's job at each time once. A layer walked twice
/// at one time is one picture, so it is decoded once and only takes on the other
/// moments the second walk wants.
pub(crate) fn merged(planned: Vec<CompJob>) -> Vec<CompJob> {
    let mut jobs: Vec<CompJob> = Vec::with_capacity(planned.len());
    for job in planned {
        let same = jobs
            .iter_mut()
            .find(|j| j.layer == job.layer && j.at.to_bits() == job.at.to_bits());
        let Some(same) = same else {
            jobs.push(job);
            continue;
        };
        for s in job.shutter {
            if same
                .shutter
                .iter()
                .all(|o| o.offset.to_bits() != s.offset.to_bits())
            {
                same.shutter.push(s);
            }
        }
        same.shutter_flow = same.shutter_flow.take().or(job.shutter_flow);
        for cut in job.cuts {
            if same.cuts.iter().all(|c| c.name() != cut.name()) {
                same.cuts.push(cut);
            }
        }
    }
    jobs
}

/// One entry a layer, with its pictures for other walks filed on it, so a map
/// by layer id can't lose them.
pub(crate) fn fold(layers: Vec<CompLayerPixels>) -> Vec<CompLayerPixels> {
    let mut out: Vec<CompLayerPixels> = Vec::with_capacity(layers.len());
    for lp in layers {
        match out.iter_mut().find(|o| o.layer == lp.layer) {
            Some(first) => first.elsewhere.push(lp),
            None => out.push(lp),
        }
    }
    out
}

impl CompLayerPixels {
    /// This layer's pixels for a walk of its comp at `t` in a frame whose own
    /// time there is `frame_t`. A walk planned at `t` draws those, and a rebuild
    /// at another moment takes its moment from the walk it belongs to. A walk
    /// nothing was planned for, as under a Posterize Time above it, draws what
    /// was planned nearest.
    pub(crate) fn placed_at(&self, t: f64, frame_t: f64) -> &Self {
        if self.elsewhere.is_empty() {
            return self;
        }
        let all = || std::iter::once(self).chain(&self.elsewhere);
        let at = |time: f64| all().find(|lp| lp.at.to_bits() == time.to_bits());
        at(t)
            .or_else(|| at(frame_t))
            .or_else(|| all().min_by(|a, b| (a.at - t).abs().total_cmp(&(b.at - t).abs())))
            .unwrap_or(self)
    }
}

/// The pixels of a walk of `comp` at `t`. `None` when they are the ones in the
/// map, so nothing is copied.
pub(crate) fn pixels_of_walk<'a>(
    doc: &Document,
    comp: &Composition,
    t: f64,
    frame_t: f64,
    pixels: &HashMap<Uuid, &'a CompLayerPixels>,
) -> Option<HashMap<Uuid, &'a CompLayerPixels>> {
    let mut out: Option<HashMap<Uuid, &'a CompLayerPixels>> = None;
    let mut place = |id: Uuid| {
        let Some(lp) = pixels.get(&id).copied() else {
            return;
        };
        let placed = lp.placed_at(t, frame_t);
        if !std::ptr::eq(placed, lp) {
            out.get_or_insert_with(|| pixels.clone()).insert(id, placed);
        }
    };
    for layer in &comp.layers {
        place(layer.id);
    }
    // A node graph files its footage under its boxes, and a graph nested in it
    // under its own.
    let mut boxes = Vec::new();
    for node in comp.graph.iter().flat_map(|g| &g.nodes) {
        place(node.id());
        if let GraphNode::Fx(inst) = node {
            crate::build::applied_boxes(
                doc,
                std::slice::from_ref(inst),
                &mut Vec::new(),
                &mut boxes,
            );
        }
    }
    for (id, _) in boxes {
        place(id);
    }
    out
}

/// The pixels of the graphs `effects` apply, on the clock of the layer they are
/// on. Two layers can apply one graph at two times, so the box's job is chosen
/// here, for this layer, along with its moment. `None` when no box has a choice
/// to make, which leaves them to [`crate::build`]'s walk.
pub(crate) fn pixels_on_clock<'a>(
    doc: &Document,
    effects: &[EffectInstance],
    lt: f64,
    frame_lt: f64,
    pixels: &HashMap<Uuid, &'a CompLayerPixels>,
) -> Option<HashMap<Uuid, &'a CompLayerPixels>> {
    let mut boxes = Vec::new();
    crate::build::applied_boxes(doc, effects, &mut Vec::new(), &mut boxes);
    let mut out: Option<HashMap<Uuid, &'a CompLayerPixels>> = None;
    for (id, graph_dt) in boxes {
        let Some(lp) = pixels
            .get(&id)
            .copied()
            .filter(|lp| !lp.elsewhere.is_empty())
        else {
            continue;
        };
        let placed = lp.placed_at(lt, frame_lt);
        let offset = crate::plan::moment_offset(lt, frame_lt, graph_dt);
        let moment = placed
            .shutter
            .iter()
            .find(|(o, _)| o.to_bits() == offset.to_bits());
        let out = out.get_or_insert_with(|| pixels.clone());
        match moment {
            Some((_, Some(moment))) => out.insert(id, &**moment),
            Some((_, None)) => out.remove(&id),
            None => out.insert(id, placed),
        };
    }
    out
}
