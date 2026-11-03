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
    /// The rows of each material the user has changed, by its place in the
    /// file ([`crate::model_material`]). Nothing for one left as it is.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Option<crate::model::EffectInstance>>,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            up_axis: UpAxis::Y,
            units: 100.0,
            fit: true,
            materials: Vec::new(),
        }
    }
}

/// A fresh Model layer for 3D model `item` in `comp`: always 3D, at the comp's
/// centre, as long as the comp, sized by the default settings, and casting
/// shadows, so it stands on whatever is under it.
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
    layer.transform.material.casts_shadows = crate::model::ShadowMode::On;
    layer
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::edits::{new_comp_ops, CompSettings};
    use crate::model::{Document, FootageItem, Layer, LayerKind, MediaRef, ProjectItem};
    use crate::ops::{apply, Op, OpError};
    use uuid::Uuid;

    fn robot() -> FootageItem {
        FootageItem {
            id: Uuid::now_v7(),
            name: "robot.glb".into(),
            media: MediaRef {
                relative_path: "robot.glb".into(),
                absolute_path: String::new(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            colour_space: None,
            sequence: None,
            extra: serde_json::Map::new(),
            source_layer: None,
            interpretation: Default::default(),
        }
    }

    /// A Model layer and its settings save and open as they were, an older
    /// file with no settings opens with the defaults, and the settings change
    /// and undo as one op that never swaps the model for another.
    #[test]
    fn a_model_layer_saves_and_opens() {
        let mut doc = Document::new();
        let (comp, ops) = new_comp_ops(&doc, CompSettings::default(), None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let item = robot();
        let layer = model_layer(doc.comp(comp).unwrap(), &item);
        let id = layer.id;
        assert!(layer.is_3d());
        apply(
            &mut doc,
            &Op::AddLayer {
                comp,
                index: 0,
                layer: Box::new(layer),
            },
        )
        .unwrap();
        doc.items.push(ProjectItem::Footage(item.clone()));
        assert!(doc.item_is_used(item.id));

        let settings = ModelSettings {
            up_axis: UpAxis::Z,
            units: 250.0,
            fit: false,
            // A changed material is kept, an unchanged one is nothing.
            materials: vec![None, crate::fx::builtins::instantiate("model_material")],
        };
        let set = Op::SetLayerKind {
            comp,
            layer: id,
            kind: Box::new(LayerKind::Model {
                item: item.id,
                settings: Box::new(settings.clone()),
            }),
        };
        let undo = apply(&mut doc, &set).unwrap();

        let saved = doc.comp(comp).unwrap().layers[0].clone();
        let json = serde_json::to_string(&saved).unwrap();
        let opened: Layer = serde_json::from_str(&json).unwrap();
        assert_eq!(opened, saved);
        assert!(matches!(&opened.kind, LayerKind::Model { settings: s, .. } if **s == settings));

        let old = format!(r#"{{"Model":{{"item":"{}"}}}}"#, item.id);
        let LayerKind::Model { settings: s, .. } = serde_json::from_str(&old).unwrap() else {
            panic!("not a Model layer");
        };
        assert_eq!(*s, ModelSettings::default());

        apply(&mut doc, &undo).unwrap();
        assert!(matches!(
            &doc.comp(comp).unwrap().layers[0].kind,
            LayerKind::Model { settings: s, .. } if **s == ModelSettings::default()
        ));

        let other = Op::SetLayerKind {
            comp,
            layer: id,
            kind: Box::new(LayerKind::Model {
                item: Uuid::now_v7(),
                settings: Box::default(),
            }),
        };
        assert_eq!(apply(&mut doc, &other), Err(OpError::KindNotConvertible));
    }
}
