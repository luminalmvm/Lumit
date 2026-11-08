//! What Audio spectrum is handed beside its op: this frame's reading of the
//! sound, as a list of points whose first coordinate is the number, in the slot
//! its hidden row keeps. The tap is the one the drivers hear, so the bars and an
//! Audio level on the same sound can't disagree.

use std::sync::Arc;

use lumit_core::expression::ExpressionContext;
use lumit_core::fx::effects::audio_spectrum::sound::reading;
use lumit_core::mask::MaskPolyline;
use lumit_core::model::{Composition, Document, EffectInstance};

pub(super) fn levels(
    doc: &Arc<Document>,
    comp: &Composition,
    e: &EffectInstance,
    layer: Option<uuid::Uuid>,
    lt: f64,
    t_comp: f64,
) -> MaskPolyline {
    let context = Arc::new(ExpressionContext {
        document: doc.clone(),
        comp: Some(comp.id),
        layer,
        comp_time: t_comp,
        current_depth: 0,
        inputs: None,
    });
    let audio = crate::audio_tap::DocumentAudio::new(doc, comp, t_comp);
    let found = reading(e, lt, comp.frame_rate.fps(), &context, Some(&audio));
    MaskPolyline {
        arc: vec![0.0; found.len()],
        points: found.into_iter().map(|level| [level, 0.0]).collect(),
        ..MaskPolyline::default()
    }
}
