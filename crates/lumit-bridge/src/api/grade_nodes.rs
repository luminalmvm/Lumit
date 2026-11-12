//! Adding grade nodes: a Correction right after a given node of a shot's
//! stack, meeting it Serial, Parallel or Layer, or an outside node that grades
//! what that node leaves alone. Each is one undo step and answers the new
//! node's id.
//!
//! A shot is a layer, or with `clip` one clip on a Sequence layer.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use lumit_core::grade::{self, GradeError};
use lumit_core::graph::{Edge, InputRef, OutputRef};
use lumit_core::model::{EffectInstance, EffectValue};
use lumit_core::ops::Op;

use crate::api::grade_versions::grade_edit;
use crate::api::layer::LayerReference;
use crate::api::BridgeError;

const CORRECTION: &str = "correction";

/// `inst` with the row `id` set to `value`.
fn set(inst: &mut EffectInstance, id: &str, value: EffectValue) {
    if let Some(p) = inst.params.iter_mut().find(|p| p.id == id) {
        p.value = value;
    }
}

impl LayerReference {
    /// A fresh Correction at comp size, meeting the node above as `combine`
    /// says (0 Serial, 1 Parallel, 2 Layer).
    #[frb(ignore)]
    fn correction(&self, combine: u32) -> Result<EffectInstance, BridgeError> {
        let comp = self.composition()?;
        let mut inst = lumit_core::fx::builtins::instantiate_for_raster(
            CORRECTION,
            f64::from(comp.width),
            f64::from(comp.height),
        )
        .ok_or(BridgeError::UnknownEffectName)?;
        set(&mut inst, "combine", EffectValue::Choice(combine));
        Ok(inst)
    }

    /// Put `inst` right after `after` in the shot's stack, as one undo step,
    /// with `graph_op` making whatever else the step needs.
    #[frb(ignore)]
    fn insert_node(
        &self,
        clip: Option<Uuid>,
        after: Uuid,
        inst: EffectInstance,
        graph_op: impl FnOnce(&lumit_core::model::Document) -> Option<Op>,
    ) -> Result<Uuid, BridgeError> {
        let id = inst.id;
        let shot = self.shot(clip);
        grade_edit(&*self.project()?, "Edit effects", |doc| {
            let ((), op) = grade::edit(doc, shot, |effects, _| {
                let at = effects
                    .iter()
                    .position(|e| e.id == after)
                    .ok_or(GradeError::UnknownShot)?;
                effects.insert(at + 1, inst);
                Ok(())
            })?;
            let ops: Vec<Op> = op.into_iter().chain(graph_op(doc)).collect();
            Ok((id, (!ops.is_empty()).then_some(Op::Batch { ops })))
        })
    }

    /// A new Correction right after `after`, grading what `after` made.
    /// Answers its id.
    #[frb(sync)]
    pub fn add_serial_grade_node(
        &self,
        clip: Option<Uuid>,
        after: Uuid,
    ) -> Result<Uuid, BridgeError> {
        self.insert_node(clip, after, self.correction(0)?, |_| None)
    }

    /// A new Correction right after `after`, grading the same picture
    /// `after` grades and adding its change to the run's. Answers its id.
    #[frb(sync)]
    pub fn add_parallel_grade_node(
        &self,
        clip: Option<Uuid>,
        after: Uuid,
    ) -> Result<Uuid, BridgeError> {
        self.insert_node(clip, after, self.correction(1)?, |_| None)
    }

    /// A new Correction right after `after`, grading the same picture
    /// `after` grades and laid over the run's through its key. Answers its
    /// id.
    #[frb(sync)]
    pub fn add_layer_grade_node(
        &self,
        clip: Option<Uuid>,
        after: Uuid,
    ) -> Result<Uuid, BridgeError> {
        self.insert_node(clip, after, self.correction(2)?, |_| None)
    }

    /// An outside node: a new Correction right after the Correction `after`,
    /// its Matte wired from `after`'s key and inverted, so it grades what
    /// `after` leaves alone. Answers its id. Only a layer carries wires, so a
    /// clip takes none.
    #[frb(sync)]
    pub fn add_outside_grade_node(&self, after: Uuid) -> Result<Uuid, BridgeError> {
        let layer = self.item()?;
        if !layer
            .effects
            .iter()
            .any(|e| e.id == after && e.effect.match_name == CORRECTION)
        {
            return Err(BridgeError::InvalidEffect);
        }
        let mut inst = self.correction(0)?;
        set(
            &mut inst,
            lumit_core::fx::schema::MATTE_INVERT_PARAM,
            EffectValue::Bool(true),
        );
        let to = inst.id;
        let (comp, layer_id) = (self.comp_id(), self.id());
        self.insert_node(None, after, inst, |doc| {
            let mut graph = doc
                .comp(comp)?
                .layers
                .iter()
                .find(|l| l.id == layer_id)?
                .graph
                .clone();
            graph.edges.push(Edge {
                from: OutputRef::EffectKey { effect: after },
                to: InputRef::Matte { effect: to },
            });
            Some(Op::SetLayerGraph {
                comp,
                layer: layer_id,
                graph: Box::new(graph),
            })
        })
    }
}
