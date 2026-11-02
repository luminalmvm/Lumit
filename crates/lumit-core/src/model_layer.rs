//! A Model layer's settings: how a 3D model's metres become comp pixels.

use serde::{Deserialize, Serialize};

/// Which way is up in the model's file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpAxis {
    #[default]
    Y,
    Z,
}

/// How a 3D model sits in its layer, so the layer's own Scale stays at 100%.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSettings {
    pub up_axis: UpAxis,
    /// How many comp pixels one metre of the model is.
    pub units: f64,
    /// Size the model to fill the comp and centre it, in place of `units`.
    pub fit: bool,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            up_axis: UpAxis::Y,
            units: 100.0,
            fit: true,
        }
    }
}

/// A fresh Model layer for 3D model `item` in `comp`: always 3D, at the comp's
/// centre, as long as the comp, and sized by the default settings.
pub fn model_layer(
    comp: &crate::model::Composition,
    item: &crate::model::FootageItem,
) -> crate::model::Layer {
    let mut layer = crate::edits::base_layer(
        item.name.clone(),
        crate::model::LayerKind::Model {
            item: item.id,
            settings: Box::default(),
        },
        comp.duration.0,
        crate::edits::centred_transform(0.0, 0.0, comp.width, comp.height),
    );
    layer.switches.three_d = true;
    layer
}
