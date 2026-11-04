//! A Model layer's part of a frame's name.

use std::sync::Arc;

use lumit_core::model::{Composition, Document, Layer, LayerKind};
use lumit_core::model_layer::{ModelSettings, Take};
use uuid::Uuid;

use crate::{Quality, SourceStamper};

/// The model's file, as the stamper names it, how it sits in the layer, and
/// the moment of the take it shows. The scene is read from the file, so naming
/// the file names the scene.
#[allow(clippy::too_many_arguments)]
pub(crate) fn feed(
    h: &mut blake3::Hasher,
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
) -> Option<()> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return None;
    };
    let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
    named(h, *item, settings, layer.source_time_at(lt), stamper)?;
    // Its playback and part rows are picture too. Nothing at all for a layer
    // that keeps none, so its name is the one it always had.
    let rows = lumit_core::model_part::stored(layer);
    crate::feed_effect_stack(
        h,
        true,
        &rows,
        Some(layer),
        comp,
        doc,
        t,
        lt,
        quality,
        stamper,
        visited,
        true,
    )
}

/// The model's own part of the name: its file, how it sits, and when it plays a
/// take, the moment as the stamper tells it, which is nothing for a still file.
fn named(
    h: &mut blake3::Hasher,
    item: Uuid,
    settings: &ModelSettings,
    source_time: f64,
    stamper: &dyn SourceStamper,
) -> Option<()> {
    let at = if settings.take == Take::Rest {
        0.0
    } else {
        source_time
    };
    let (identity, frame) = stamper.stamp(item, at, false)?;
    h.update(b"model/");
    h.update(identity.as_bytes());
    if let Ok(json) = serde_json::to_vec(settings) {
        h.update(&json);
    }
    if settings.take != Take::Rest {
        h.update(&frame.to_le_bytes());
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
            named(&mut h, item, settings, 0.0, &Named).unwrap();
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
