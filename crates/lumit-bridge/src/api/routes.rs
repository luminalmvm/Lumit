//! Buses and sends across the bridge: adding a Bus layer, and where a
//! layer's sound goes ([`lumit_core::routes`]).
//!
//! A layer's routing rides in on [`crate::api::layer::BridgeLayerInfo`], so
//! the Audio timeline draws it without asking. Every change goes back as the
//! whole routing in one call, one undo step.

use flutter_rust_bridge::frb;
use lumit_core::routes::{AudioRoutes, AudioSend};
use uuid::Uuid;

use crate::api::effect::BridgeScalar;
use crate::api::layer::LayerReference;
use crate::api::{composition::CompositionReference, BridgeError};

/// One send: the bus, how much goes to it in dB, and whether it is taken
/// before the layer's Volume and Pan.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSend {
    pub bus: Uuid,
    pub level: BridgeScalar,
    pub pre: bool,
}

/// A layer's sends as the read model carries them.
#[frb(ignore)]
pub(crate) fn bridge_sends(layer: &lumit_core::model::Layer) -> Vec<BridgeSend> {
    layer
        .audio_routes
        .sends
        .iter()
        .map(|s| BridgeSend {
            bus: s.bus,
            level: BridgeScalar::read_at(&s.level_db, layer.start_offset.0),
            pre: s.pre,
        })
        .collect()
}

impl CompositionReference {
    /// Add a Bus layer, named after how many the comp already has. One undo
    /// step.
    #[frb(sync)]
    pub fn add_bus_layer(&self, row: Option<u32>) -> Result<LayerReference, BridgeError> {
        let comp = self.composition()?;
        let n = comp.layers.iter().filter(|l| l.is_bus()).count() + 1;
        let mut layer = crate::edits::base_layer(
            format!("Bus {n}"),
            lumit_core::model::LayerKind::Null,
            comp.duration.0,
            crate::edits::centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            ),
        );
        layer.audio_only = true;
        let id = layer.id;
        let state = self.project()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(lumit_core::ops::Op::AddLayer {
                comp: self.id,
                index: crate::api::composition::insert_row(row, comp.layers.len()),
                layer: Box::new(layer),
            })
            .map_err(BridgeError::OpError)?;
        Ok(LayerReference::new(self.project, self.id, id))
    }
}

impl LayerReference {
    /// Route this layer's sound to `output` (a Bus layer, or `None` for the
    /// master) and set its sends, as one undoable step.
    #[frb(sync)]
    pub fn set_audio_routes(
        &self,
        output: Option<Uuid>,
        sends: Vec<BridgeSend>,
    ) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let sends = sends
            .into_iter()
            .map(|s| {
                let mut level_db = lumit_core::anim::Property::zero();
                level_db.animation = s.level.animation_at(offset)?;
                Ok(AudioSend {
                    bus: s.bus,
                    level_db,
                    pre: s.pre,
                })
            })
            .collect::<Result<Vec<_>, BridgeError>>()?;
        let routes = AudioRoutes { output, sends };
        if routes == layer.audio_routes {
            return Ok(());
        }
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::ops::Op::SetLayerRoutes {
                comp: self.comp_id,
                layer: self.layer_id,
                routes,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }
}
