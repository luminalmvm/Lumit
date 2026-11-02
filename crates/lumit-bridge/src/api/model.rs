//! 3D models: telling a model item from footage, and a Model layer's settings.

use flutter_rust_bridge::frb;
use lumit_core::model::{LayerKind, ProjectItem};
use lumit_core::model_layer::{ModelSettings, UpAxis};

use crate::api::{footage::FootageReference, layer::LayerReference, BridgeError};

/// Which of a 3D model file's axes points up.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeUpAxis {
    Y,
    Z,
}

/// How a 3D model sits in its Model layer, so its Scale stays at 100%.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeModelSettings {
    pub up_axis: BridgeUpAxis,
    /// Comp pixels for each metre of the model.
    pub units: f64,
    /// Sized to fill the comp and centred, in place of `units`.
    pub fit: bool,
}

impl FootageReference {
    /// Whether this item is a 3D model rather than footage that plays.
    #[frb(sync)]
    pub fn is_model(&self) -> Result<bool, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match proj.store.snapshot().item(self.id()) {
            Some(ProjectItem::Footage(f)) => lumit_scene::is_model(f.media.on_disk()),
            _ => false,
        })
    }
}

impl LayerReference {
    /// A Model layer's settings, or `None` on any other kind of layer.
    #[frb(sync)]
    pub fn get_model_settings(&self) -> Result<Option<BridgeModelSettings>, BridgeError> {
        Ok(match self.item()?.kind {
            LayerKind::Model { settings, .. } => Some(BridgeModelSettings {
                up_axis: match settings.up_axis {
                    UpAxis::Y => BridgeUpAxis::Y,
                    UpAxis::Z => BridgeUpAxis::Z,
                },
                units: settings.units,
                fit: settings.fit,
            }),
            _ => None,
        })
    }

    /// The corners of a Model layer's box in its own pixels, three numbers
    /// each, once its file has been read. Empty before that and on any other
    /// kind of layer.
    #[frb(sync)]
    pub fn get_model_box(&self) -> Result<Vec<f64>, BridgeError> {
        let LayerKind::Model { item, settings } = self.item()?.kind else {
            return Ok(Vec::new());
        };
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let size = (comp.width as f32, comp.height as f32);
        let corners = lumit_render::model::layer_box(&doc, item, &settings, size);
        Ok(corners
            .iter()
            .flatten()
            .flatten()
            .map(|v| f64::from(*v))
            .collect())
    }

    /// Replace a Model layer's settings, as one undo step. Refused on any
    /// other kind of layer.
    #[frb(sync)]
    pub fn set_model_settings(&self, settings: BridgeModelSettings) -> Result<(), BridgeError> {
        let LayerKind::Model { item, .. } = self.item()?.kind else {
            return Err(BridgeError::InvalidLayer);
        };
        let settings = ModelSettings {
            up_axis: match settings.up_axis {
                BridgeUpAxis::Y => UpAxis::Y,
                BridgeUpAxis::Z => UpAxis::Z,
            },
            // Nought would shrink the model to nothing and no further.
            units: settings.units.max(f64::EPSILON),
            fit: settings.fit,
        };
        self.commit(lumit_core::ops::Op::SetLayerKind {
            comp: self.comp_id,
            layer: self.layer_id,
            kind: Box::new(LayerKind::Model {
                item,
                settings: Box::new(settings),
            }),
        })
    }
}
