//! A layer's Orient across the seam: the choice, and the turn it comes to at a
//! frame, so the Viewer's box sits where the picture has it.

use flutter_rust_bridge::frb;
use lumit_core::orient::{Orient, Turn};

use crate::api::{layer::LayerReference, BridgeError};

#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeOrient {
    Off,
    AlongPath,
    TowardsCamera,
}

impl BridgeOrient {
    #[frb(ignore)]
    pub(crate) fn of(orient: Orient) -> Self {
        match orient {
            Orient::Off => Self::Off,
            Orient::AlongPath => Self::AlongPath,
            Orient::TowardsCamera => Self::TowardsCamera,
        }
    }

    #[frb(ignore)]
    fn core(self) -> Orient {
        match self {
            Self::Off => Orient::Off,
            Self::AlongPath => Orient::AlongPath,
            Self::TowardsCamera => Orient::TowardsCamera,
        }
    }
}

/// What Orient adds at one frame: degrees on a flat layer, or nine numbers row
/// by row, the turn outside a 3D layer's Orientation.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeOrientTurn {
    pub degrees: f64,
    pub rows: Option<Vec<f64>>,
}

impl LayerReference {
    #[frb(sync)]
    pub fn set_orient(&self, orient: BridgeOrient) -> Result<(), BridgeError> {
        self.commit(lumit_core::ops::Op::SetLayerOrient {
            comp: self.comp_id,
            layer: self.layer_id,
            orient: orient.core(),
        })
    }

    /// The turn at comp frame `frame`, `None` with Orient off.
    #[frb(sync)]
    pub fn orient_turn_at(&self, frame: i64) -> Result<Option<BridgeOrientTurn>, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let layer = comp
            .layers
            .iter()
            .find(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;
        if layer.transform.orient.is_off() {
            return Ok(None);
        }
        let t = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?
            .0
            .to_f64();
        let context = lumit_core::model::comp_context(&doc, comp, t);
        let read = lumit_core::model::context_reader(&context, t);
        let solves = lumit_render::track::solves_for(&doc);
        let turn = lumit_core::orient::turn_at(comp, layer, t, &read, solves);
        Ok(turn.map(|turn| match turn {
            Turn::Flat(degrees) => BridgeOrientTurn {
                degrees,
                rows: None,
            },
            Turn::Rows(m) => BridgeOrientTurn {
                degrees: 0.0,
                rows: Some(m.iter().flatten().copied().collect()),
            },
        }))
    }
}
