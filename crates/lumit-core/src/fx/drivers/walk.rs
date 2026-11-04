//! Particulate's walk as the driver graph reads it: its rows, wires and outlines
//! at any one layer time.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use uuid::Uuid;

use super::{measure, ArenaPool, Eval, EVAL_BUDGET};
use crate::expression::ExpressionContext;
use crate::fx::markers::MarkerContext;
use crate::fx::points::collider::Collider;
use crate::fx::points::stepped::{definition, stream_at, Inputs};
use crate::fx::points::{self, PointsStream};
use crate::fx::registry::{AudioTap, EffectDef, EffectMetadata};
use crate::fx::resolved::resolve_into_arena;
use crate::graph::{LayerGraph, NodeRef};
use crate::model::{EffectInstance, Layer};

impl Eval<'_> {
    /// Particulate `inst`'s rows, wires and outlines at layer time `lt` on
    /// `layer`, with `target` as what it hits. A fresh walk of the wires at
    /// that time, so nothing a walk at another time kept is read.
    #[allow(clippy::too_many_arguments)]
    fn inputs_at(
        &self,
        effect: Uuid,
        inst: &EffectInstance,
        def: &'static dyn EffectDef,
        layer: &Layer,
        target: Option<&Layer>,
        lt: f64,
        depth: u32,
    ) -> Option<Inputs> {
        let doc = &self.context.document;
        let comp = doc.comp(self.context.comp?)?;
        let tc = lt + layer.start_offset.0.to_f64();
        let context = Arc::new(ExpressionContext {
            comp_time: tc,
            ..(*self.context).clone()
        });
        let ev = Eval {
            graph: self.graph,
            context: Arc::clone(&context),
            audio: self.audio,
            projection: points::Projection::FLAT,
            budget: Cell::new(EVAL_BUDGET),
            streams: RefCell::new(Vec::new()),
            arenas: RefCell::new(ArenaPool::default()),
            cross: self.cross,
            stack: self.stack,
            clone: self.clone,
            point: self.point,
            cloud: self.cloud,
        };
        let wired = ev.wires_into(effect, lt, depth);
        let rows = ev.with_arena(|bag| {
            resolve_into_arena(
                def,
                inst,
                NodeRef::Effect(effect),
                lt,
                0.0,
                1.0,
                &MarkerContext::NONE,
                bag,
                Arc::clone(&context),
                &wired,
            );
            bag.get(0)
                .map(|b| crate::fx::effects::particulate::Particulate::read(b.params))
        })?;
        let path = match def.schema().mask_path() {
            Some((param, self_default)) => {
                crate::mask::effect_path_at(inst, param, self_default, Some(layer), lt)
            }
            None => crate::mask::MaskPolyline::default(),
        };
        let frame = measure::layer_rect(doc, comp, layer, tc, &context)
            .map_or([comp.width as f32, comp.height as f32], |r| {
                [(r[2] - r[0]) as f32, (r[3] - r[1]) as f32]
            });
        let collider = target.map(|target| Collider::of(doc, comp, layer, target, tc, &context));
        Some(Inputs::new(rows, frame, path, collider))
    }

    /// Particulate's particles at layer time `t` on `layer`, walked from the
    /// in point against `target`, if any ([`points::stepped`]). Each step is
    /// a fresh walk of the wires at that step's own time, so nothing this walk
    /// has kept for `t` is read at another.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn stepped(
        &self,
        effect: Uuid,
        inst: &EffectInstance,
        def: &'static dyn EffectDef,
        layer: &Layer,
        target: Option<&Layer>,
        t: f64,
        depth: u32,
    ) -> Option<PointsStream> {
        let doc = &self.context.document;
        let comp = doc.comp(self.context.comp?)?;
        let dt = 1.0 / comp.frame_rate.fps().max(1.0);
        let at = |lt: f64| self.inputs_at(effect, inst, def, layer, target, lt, depth);
        stream_at(definition(doc, comp, layer, inst, target), t, dt, &at)
    }
}

/// Particulate `effect`'s rows at layer time `lt`, each read at that time with
/// its wires, in px@comp, with no outline to hit. `None` when the effect is not
/// on the context's layer.
pub fn rows_at(
    graph: &LayerGraph,
    effect: Uuid,
    lt: f64,
    context: Arc<ExpressionContext>,
    audio: Option<&dyn AudioTap>,
    clone: (u32, u32),
) -> Option<Inputs> {
    let doc = Arc::clone(&context.document);
    let comp = doc.comp(context.comp?)?;
    let layer = context.layer?;
    let layer = comp.layers.iter().find(|l| l.id == layer)?;
    let inst = layer.effects.iter().find(|e| e.id == effect)?;
    let def = crate::fx::catalogue::BUILTIN_DEFS.get(&inst.effect.match_name)?;
    let ev = Eval {
        graph,
        context,
        audio,
        projection: points::Projection::FLAT,
        budget: Cell::new(EVAL_BUDGET),
        streams: RefCell::new(Vec::new()),
        arenas: RefCell::new(ArenaPool::default()),
        cross: true,
        stack: None,
        clone,
        point: None,
        cloud: None,
    };
    ev.inputs_at(effect, inst, def, layer, None, lt, 0)
}
