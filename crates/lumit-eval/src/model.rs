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
    // A card has no file to name, and is always the same scene.
    let (identity, frame) = match item == lumit_core::model_layer::CARD {
        true => (String::new(), 0),
        false => stamper.stamp(item, at, false)?,
    };
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
#[allow(clippy::unwrap_used, clippy::panic)]
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

    /// What it reads as the stamper tells it, so a take's moment is in the name.
    struct Clock;
    impl SourceStamper for Clock {
        fn stamp(&self, item: Uuid, lt: f64, _native: bool) -> Option<(String, u64)> {
            Some((item.to_string(), lt.to_bits()))
        }
    }

    struct Unread;
    impl SourceStamper for Unread {
        fn stamp(&self, _item: Uuid, _lt: f64, _native: bool) -> Option<(String, u64)> {
            None
        }
    }

    /// A cached picture is only shown for the moment and the rows it was drawn
    /// with. A playing take and a keyed row name each frame, a model at rest with
    /// nothing keyed has one name for all of them, and a changed row or Offset
    /// is a new picture. A file nobody can name is never cached.
    #[test]
    fn the_moment_and_the_rows_rename_the_frame() {
        use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
        use lumit_core::edits::{new_comp_ops, CompSettings};
        use lumit_core::fx::builtins::instantiate;
        use lumit_core::model::{EffectValue, FootageItem, MediaRef, ProjectItem};
        use lumit_core::model_part::{part_id, playback_id, PART_NAME, PLAYBACK_NAME};
        use lumit_core::ops::{apply, Op};

        let mut doc = Document::new();
        let (comp, ops) = new_comp_ops(&doc, CompSettings::default(), None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let file = FootageItem {
            id: Uuid::now_v7(),
            name: "rig.glb".into(),
            media: MediaRef {
                relative_path: String::new(),
                absolute_path: "rig.glb".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            colour_space: None,
            sequence: None,
            extra: serde_json::Map::new(),
            source_layer: None,
            interpretation: Default::default(),
        };
        doc.items.push(ProjectItem::Footage(file.clone()));
        let layer = lumit_core::model_layer::model_layer(doc.comp(comp).unwrap(), &file);
        let id = layer.id;
        apply(
            &mut doc,
            &Op::AddLayer {
                comp,
                index: 0,
                layer: Box::new(layer),
            },
        )
        .unwrap();

        let name = |change: &dyn Fn(&mut ModelSettings), t: f64| {
            let mut doc = doc.clone();
            let c = doc.comp_mut(comp).unwrap();
            let LayerKind::Model { settings, .. } = &mut c.layers[0].kind else {
                panic!("a Model layer");
            };
            change(settings);
            let doc = Arc::new(doc);
            let c = doc.comp(comp).unwrap();
            crate::comp_frame_key(&doc, c, t, Quality::default(), &Clock).unwrap()
        };
        let key = |secs: i64, value: f64| Keyframe {
            time: lumit_core::time::Rational::new(secs, 1).unwrap(),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let row = |rows: &mut lumit_core::model::EffectInstance, id: &str, p: Property| {
            for r in &mut rows.params {
                if r.id == id {
                    r.value = EffectValue::Float(p.clone());
                }
            }
        };
        let plays = |_: &mut ModelSettings| {};
        let rest = |s: &mut ModelSettings| s.take = Take::Rest;
        assert_ne!(name(&plays, 0.5), name(&plays, 1.0), "the take plays");
        assert_eq!(name(&rest, 0.5), name(&rest, 1.0), "at rest it doesn't");

        let offset = |s: &mut ModelSettings| {
            let mut rows = instantiate(PLAYBACK_NAME).unwrap();
            rows.id = playback_id(id);
            row(&mut rows, "offset", Property::fixed(0.25));
            s.playback = Some(rows);
        };
        assert_ne!(name(&offset, 0.5), name(&plays, 0.5), "Offset");

        let still = |s: &mut ModelSettings| {
            s.take = Take::Rest;
            let mut rows = instantiate(PART_NAME).unwrap();
            rows.id = part_id(id, 1);
            row(&mut rows, "position_x", Property::fixed(5.0));
            s.parts = vec![rows];
        };
        assert_ne!(name(&still, 0.5), name(&rest, 0.5), "a changed row");
        assert_eq!(name(&still, 0.5), name(&still, 1.0), "held, it stays");

        let keyed = |s: &mut ModelSettings| {
            s.take = Take::Rest;
            let mut moved = Property::zero();
            moved.animation = Animation::Keyframed(vec![key(0, 0.0), key(2, 20.0)]);
            let mut rows = instantiate(PART_NAME).unwrap();
            rows.id = part_id(id, 1);
            row(&mut rows, "position_x", moved);
            s.parts = vec![rows];
        };
        assert_ne!(name(&keyed, 0.5), name(&keyed, 1.0), "a keyed row");

        let doc = Arc::new(doc);
        let c = doc.comp(comp).unwrap();
        assert!(crate::comp_frame_key(&doc, c, 0.5, Quality::default(), &Unread).is_none());
    }
}
