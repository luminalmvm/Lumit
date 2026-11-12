//! The mixer's ops: a layer's gain, automation mode, solo safe and lower
//! behind, and a comp's master keys, master automation mode and loudness
//! target. Each writes one value and answers the one it replaced.

use super::{Op, OpError};
use crate::model::{Document, Layer};
use uuid::Uuid;

fn layer_mut(doc: &mut Document, comp: Uuid, layer: Uuid) -> Result<&mut Layer, OpError> {
    doc.comp_mut(comp)
        .ok_or(OpError::UnknownComp)?
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .ok_or(OpError::UnknownLayer)
}

/// Apply one of the mixer's ops. `apply` sends only those here.
pub(super) fn apply(doc: &mut Document, op: &Op) -> Result<Op, OpError> {
    match op {
        Op::SetLayerAudioGain { comp, layer, db } => {
            let l = layer_mut(doc, *comp, *layer)?;
            Ok(Op::SetLayerAudioGain {
                comp: *comp,
                layer: *layer,
                db: std::mem::replace(&mut l.audio_gain_db, *db),
            })
        }
        Op::SetLayerAutomation { comp, layer, mode } => {
            let l = layer_mut(doc, *comp, *layer)?;
            Ok(Op::SetLayerAutomation {
                comp: *comp,
                layer: *layer,
                mode: std::mem::replace(&mut l.automation, *mode),
            })
        }
        Op::SetLayerSoloSafe {
            comp,
            layer,
            solo_safe,
        } => {
            let l = layer_mut(doc, *comp, *layer)?;
            Ok(Op::SetLayerSoloSafe {
                comp: *comp,
                layer: *layer,
                solo_safe: std::mem::replace(&mut l.switches.solo_safe, *solo_safe),
            })
        }
        Op::SetLayerDuck { comp, layer, duck } => {
            let c = doc.comp(*comp).ok_or(OpError::UnknownComp)?;
            let routes = c
                .layers
                .iter()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?
                .audio_routes
                .clone();
            crate::routes::check_routes(c, *layer, &routes, duck.as_ref())?;
            let l = layer_mut(doc, *comp, *layer)?;
            Ok(Op::SetLayerDuck {
                comp: *comp,
                layer: *layer,
                duck: std::mem::replace(&mut l.duck, *duck),
            })
        }
        Op::SetMasterAutomation { comp, keys } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            Ok(Op::SetMasterAutomation {
                comp: *comp,
                keys: std::mem::replace(&mut c.master_keys, keys.clone()),
            })
        }
        Op::SetMasterAutomationMode { comp, mode } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            Ok(Op::SetMasterAutomationMode {
                comp: *comp,
                mode: std::mem::replace(&mut c.master_automation, *mode),
            })
        }
        Op::SetLoudnessTarget { comp, target } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            Ok(Op::SetLoudnessTarget {
                comp: *comp,
                target: std::mem::replace(&mut c.loudness_target, *target),
            })
        }
        // Never reached: `apply` hands this function the ops above and no
        // others.
        _ => Err(OpError::UnknownItem),
    }
}
