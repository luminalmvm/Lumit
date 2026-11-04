//! A Model layer's part of a frame's name, and a 3D shape's.

use lumit_core::model::LayerKind;

use crate::SourceStamper;

/// The model's file, as the stamper names it, and how it sits in the layer.
/// The scene is read from the file, so naming the file names the scene. A 3D
/// shape is named by its shape, as its rows go in with the materials'.
pub(crate) fn feed(
    h: &mut blake3::Hasher,
    kind: &LayerKind,
    stamper: &dyn SourceStamper,
) -> Option<()> {
    let (item, settings) = match kind {
        LayerKind::Model { item, settings } => (*item, settings),
        LayerKind::Mesh { shape } => {
            h.update(b"mesh/");
            h.update(shape.options().as_bytes());
            return Some(());
        }
        _ => return Some(()),
    };
    let (identity, _) = stamper.stamp(item, 0.0, false)?;
    h.update(b"model/");
    h.update(identity.as_bytes());
    if let Ok(json) = serde_json::to_vec(&**settings) {
        h.update(&json);
    }
    Some(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumit_core::model_layer::{ModelSettings, UpAxis};
    use uuid::Uuid;

    struct Named;
    impl SourceStamper for Named {
        fn stamp(&self, item: Uuid, _lt: f64, _native: bool) -> Option<(String, u64)> {
            Some((item.to_string(), 0))
        }
    }

    /// Changing how the model sits in its layer renames the frame, so a cached
    /// picture with the old Up axis is never shown.
    #[test]
    fn settings_rename_the_frame() {
        let item = Uuid::now_v7();
        let name = |settings: &ModelSettings| {
            let mut h = blake3::Hasher::new();
            let kind = LayerKind::Model {
                item,
                settings: Box::new(settings.clone()),
            };
            feed(&mut h, &kind, &Named).unwrap();
            h.finalize()
        };
        let z = ModelSettings {
            up_axis: UpAxis::Z,
            ..ModelSettings::default()
        };
        assert_eq!(name(&z), name(&z.clone()));
        assert_ne!(name(&z), name(&ModelSettings::default()));
    }
}
