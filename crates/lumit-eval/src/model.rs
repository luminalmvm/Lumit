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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumit_core::model_layer::UpAxis;

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
            feed(&mut h, item, settings, &Named).unwrap();
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
