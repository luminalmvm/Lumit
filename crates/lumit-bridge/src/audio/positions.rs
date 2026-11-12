//! Where each fader is now, so the mixer's faders move with the automation
//! while the comp plays.

use super::automation::{held_now, layer_in, layer_of, master_at};
use super::{heard, lock};
use crate::api::mixer::BridgeMixTarget;
use uuid::Uuid;

/// One strip's controls, or the master's, at the clock.
pub(crate) struct Position {
    /// The layer, or None for the master.
    pub layer: Option<Uuid>,
    pub volume_db: f64,
    pub pan: f64,
    /// The send levels in dB, in the layer's send order.
    pub sends: Vec<f64>,
}

/// Each node's Volume, Pan and send levels at the clock, read from the
/// document the playing plan was built from, then the master's Volume; the
/// value heard instead wherever a control is held. Empty with nothing
/// loaded. The keys are read with the lock let go.
pub(crate) fn positions() -> Vec<Position> {
    let (loaded, comp, t) = {
        let st = lock();
        let (Some(loaded), Some(comp), Some((t, _))) =
            (st.loaded_plan.clone(), st.loaded_comp, heard(&st))
        else {
            return Vec::new();
        };
        (loaded, comp, t)
    };
    let Some(c) = loaded.doc.as_deref().and_then(|d| d.comp(comp)) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(loaded.info.nodes.len() + 1);
    for node in &loaded.info.nodes {
        let Some(l) = layer_of(c, node.layer) else {
            continue;
        };
        let lt = t - l.start_offset.0.to_f64();
        out.push(Position {
            layer: Some(l.id),
            volume_db: l.volume_db.value_at(lt),
            pan: l.pan.value_at(lt),
            sends: l
                .audio_routes
                .sends
                .iter()
                .map(|s| s.level_db.value_at(lt))
                .collect(),
        });
    }
    out.push(Position {
        layer: None,
        volume_db: master_at(c, t),
        pan: 0.0,
        sends: Vec::new(),
    });

    let st = lock();
    if st.loaded_comp != Some(comp) {
        return out;
    }
    for (target, value, trim) in held_now(&st) {
        let at = match layer_in(target) {
            Some(layer) => out.iter_mut().find(|p| p.layer == Some(layer)),
            None => out.last_mut(),
        };
        let Some(at) = at else {
            continue;
        };
        let slot = match target {
            BridgeMixTarget::Volume { .. } | BridgeMixTarget::Master => Some(&mut at.volume_db),
            BridgeMixTarget::Pan { .. } => Some(&mut at.pan),
            BridgeMixTarget::Send { layer, bus } => layer_of(c, layer)
                .and_then(|l| l.audio_routes.sends.iter().position(|s| s.bus == bus))
                .and_then(|i| at.sends.get_mut(i)),
        };
        if let Some(slot) = slot {
            *slot = if trim { *slot + value } else { value };
        }
    }
    out
}
