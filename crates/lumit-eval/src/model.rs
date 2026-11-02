//! A Model layer's part of a frame's name.

use lumit_core::model_layer::ModelSettings;
use uuid::Uuid;

use crate::SourceStamper;

/// The model's file, as the stamper names it, and how it sits in the layer.
/// The scene is read from the file, so naming the file names the scene.
pub(crate) fn feed(
    h: &mut blake3::Hasher,
    item: Uuid,
    settings: &ModelSettings,
    stamper: &dyn SourceStamper,
) -> Option<()> {
    let (identity, _) = stamper.stamp(item, 0.0, false)?;
    h.update(b"model/");
    h.update(identity.as_bytes());
    if let Ok(json) = serde_json::to_vec(settings) {
        h.update(&json);
    }
    Some(())
}
