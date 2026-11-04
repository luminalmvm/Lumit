//! What a projected material adds to a frame's name: the pose of a camera that
//! need not be the comp's own, and a held picture, which isn't what the
//! material's layer row shows at this frame.

use super::{feed_depth, feed_f64, feed_layer, feed_parents, Quality, SourceStamper};
use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, EffectInstance, Layer};
use lumit_core::model_material::{camera_at, projection, stored};
use std::sync::Arc;
use uuid::Uuid;

/// Feed each of `rows` that projects. Nothing for rows on the model's uvs, so
/// every other key stays as it was.
#[allow(clippy::too_many_arguments)]
pub(super) fn feed_projections(
    h: &mut blake3::Hasher,
    doc: &Arc<Document>,
    comp: &Composition,
    rows: &[EffectInstance],
    (t, lt): (f64, f64),
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
) -> Option<()> {
    let find = |id| comp.layers.iter().find(|l| l.id == id);
    for p in rows.iter().filter_map(|r| projection(r, comp, lt)) {
        h.update(b"projection/");
        let time = p.time_at(t);
        let pose = find(p.camera).and_then(|c| camera_at(doc, comp, c, time));
        if let Some(pose) = pose {
            let (x, y, z) = pose.position;
            let (rx, ry, rz) = pose.rotation_deg;
            for v in [x, y, z, rx, ry, rz, pose.zoom] {
                feed_f64(h, v);
            }
        }
        // The models on this camera stop at the Hold frame too, and a frame's
        // own name only has where they are at the playhead.
        if let Some(held) = p.held {
            let on_camera = |l: &&Layer| {
                let lt = lumit_core::time::layer_time(t, l.start_offset.0);
                let kept = stored(l);
                l.switches.visible
                    && (kept.iter())
                        .any(|r| projection(r, comp, lt).is_some_and(|q| q.camera == p.camera))
            };
            for l in comp.layers.iter().filter(on_camera) {
                feed_placed(h, doc, comp, l, held, stamper);
            }
        }
        if let Some((held, src)) = p.held.zip(find(p.picture)) {
            // A layer holding itself, or two holding each other, would name
            // themselves for ever.
            if visited.contains(&src.id) {
                h.update(b"held-again/");
                continue;
            }
            visited.push(src.id);
            let slt = lumit_core::time::layer_time(held, src.start_offset.0);
            let fed = feed_layer(h, doc, comp, src, held, slt, quality, stamper, visited);
            visited.pop();
            fed?;
        }
    }
    Some(())
}

/// Where `layer` is at comp time `t`.
fn feed_placed(
    h: &mut blake3::Hasher,
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    stamper: &dyn SourceStamper,
) {
    let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
    let context = Arc::new(ExpressionContext {
        document: doc.clone(),
        comp: Some(comp.id),
        layer: Some(layer.id),
        comp_time: t,
        current_depth: 0,
        inputs: None,
    });
    let tr = &layer.transform;
    for p in [
        &tr.position_x,
        &tr.position_y,
        &tr.position_z,
        &tr.anchor_x,
        &tr.anchor_y,
        &tr.scale_x,
        &tr.scale_y,
        &tr.rotation,
        &tr.rotation_x,
        &tr.rotation_y,
    ] {
        feed_f64(h, p.value_at_with_context(lt, context.clone()));
    }
    feed_depth(h, tr, lt, &|p| p.value_at_with_context(lt, context.clone()));
    if layer.parent.is_some() {
        feed_parents(h, doc, comp, layer, t, stamper);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::{comp_frame_key, Quality};
    use lumit_core::anim::Property;
    use lumit_core::edits::{base_layer, new_comp_ops, CompSettings};
    use lumit_core::model::{EffectValue, LayerKind};
    use lumit_core::ops::{apply, Op};

    struct Nothing;
    impl SourceStamper for Nothing {
        fn stamp(&self, item: Uuid, _lt: f64, _native: bool) -> Option<(String, u64)> {
            Some((item.to_string(), 0))
        }
    }

    /// A Model layer holding its own picture, or two holding each other's,
    /// still has a name, rather than overflowing the stack looking for it.
    #[test]
    fn a_hold_loop_has_a_name() {
        let mut doc = Document::new();
        let (id, ops) = new_comp_ops(&doc, CompSettings::default(), None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let comp = doc.comp(id).unwrap().clone();
        let camera = lumit_core::edits::fresh_camera(&comp, "Camera".into(), None);
        let item = Uuid::now_v7();
        let model = || {
            lumit_core::edits::base_layer(
                "model".into(),
                LayerKind::Model {
                    item,
                    settings: Box::default(),
                },
                comp.duration.0,
                Default::default(),
            )
        };
        let (mut a, mut b) = (model(), model());
        let (a_id, b_id, cam) = (a.id, b.id, camera.id);
        let hold_on = |layer: &mut lumit_core::model::Layer, picture| {
            let mut rows =
                lumit_core::fx::builtins::instantiate(lumit_core::model_material::MATCH_NAME)
                    .unwrap();
            lumit_core::model_material::set_projection(&mut rows, picture, cam, Some(0));
            if let LayerKind::Model { settings, .. } = &mut layer.kind {
                settings.materials = vec![Some(rows)];
            }
        };
        let named = |a: &lumit_core::model::Layer, b: &lumit_core::model::Layer| {
            let mut doc = doc.clone();
            let c = doc.comp_mut(id).unwrap();
            c.layers = vec![camera.clone(), a.clone(), b.clone()];
            let doc = Arc::new(doc);
            let comp = doc.comp(id).unwrap().clone();
            comp_frame_key(&doc, &comp, 0.0, Quality::default(), &Nothing)
        };
        hold_on(&mut a, a_id);
        assert!(named(&a, &b).is_some(), "holding itself");
        hold_on(&mut a, b_id);
        hold_on(&mut b, a_id);
        assert!(named(&a, &b).is_some(), "holding each other");
    }

    /// Moving a camera that isn't the comp's own moves what the model shows,
    /// so a frame drawn before can't stand in for it. It isn't the active one
    /// here because a layer above it is, and only its pose says what changed.
    /// Under Hold, moving the model at the Hold frame does the same, with the
    /// model where it was at the playhead.
    #[test]
    fn a_projecting_camera_names_the_frame() {
        let mut doc = Document::new();
        let settings = CompSettings {
            width: 64,
            height: 64,
            ..CompSettings::default()
        };
        let (id, ops) = new_comp_ops(&doc, settings, None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let comp = doc.comp(id).unwrap().clone();
        let active = lumit_core::edits::fresh_camera(&comp, "Viewer".into(), None);
        let projector = lumit_core::edits::fresh_camera(&comp, "Plate".into(), None);
        let picture = base_layer(
            "picture".into(),
            LayerKind::Null,
            comp.duration.0,
            Default::default(),
        );
        let mut rows =
            lumit_core::fx::builtins::instantiate(lumit_core::model_material::MATCH_NAME).unwrap();
        for (row, value) in [
            ("base_colour_layer", EffectValue::Layer(Some(picture.id))),
            ("camera", EffectValue::Layer(Some(projector.id))),
        ] {
            rows.params.iter_mut().find(|p| p.id == row).unwrap().value = value;
        }
        let item = lumit_core::model::FootageItem {
            id: Uuid::now_v7(),
            name: "set.glb".into(),
            media: lumit_core::model::MediaRef {
                relative_path: String::new(),
                absolute_path: "set.glb".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            colour_space: None,
            sequence: None,
            extra: serde_json::Map::new(),
            source_layer: None,
            interpretation: Default::default(),
        };
        let mut model = lumit_core::model_layer::model_layer(&comp, &item);
        if let LayerKind::Model { settings, .. } = &mut model.kind {
            settings.materials = vec![Some(rows)];
        }
        let (projector_id, model_id) = (projector.id, model.id);
        for (index, layer) in [active, model, projector, picture].into_iter().enumerate() {
            let layer = Box::new(layer);
            apply(
                &mut doc,
                &Op::AddLayer {
                    comp: id,
                    index,
                    layer,
                },
            )
            .unwrap();
        }
        doc.items
            .push(lumit_core::model::ProjectItem::Footage(item));
        let name_at = |t: f64, doc: Document| {
            let doc = Arc::new(doc);
            let comp = doc.comp(id).unwrap().clone();
            comp_frame_key(&doc, &comp, t, Quality::default(), &Nothing)
        };
        let name = |doc: Document| name_at(0.0, doc);
        let held_from = |x: f64| {
            let mut doc = doc.clone();
            let c = doc.comp_mut(id).unwrap();
            let rate = c.frame_rate;
            let at = c.layers.iter_mut().find(|l| l.id == model_id).unwrap();
            let key = |frame: i64, value: f64| lumit_core::anim::Keyframe {
                time: rate.time_of_frame(frame).unwrap().0,
                value,
                interp_in: lumit_core::anim::SideInterp::Linear,
                interp_out: lumit_core::anim::SideInterp::Linear,
            };
            at.transform.position_x = Property {
                animation: lumit_core::anim::Animation::Keyframed(vec![
                    key(0, x),
                    key(rate.fps().round() as i64, 0.0),
                ]),
                ..Property::fixed(0.0)
            };
            if let LayerKind::Model { settings, .. } = &mut at.kind {
                let rows = settings.materials[0].as_mut().unwrap();
                let time = rows.params.iter_mut().find(|p| p.id == "projection_time");
                time.unwrap().value = EffectValue::Choice(1);
            }
            doc
        };
        assert_ne!(name_at(1.0, held_from(0.0)), name_at(1.0, held_from(300.0)));
        let first = name(doc.clone());
        let at = doc
            .comp_mut(id)
            .unwrap()
            .layers
            .iter_mut()
            .find(|l| l.id == projector_id)
            .unwrap();
        at.transform.position_x = Property::fixed(500.0);
        assert!(first.is_some());
        assert_ne!(first, name(doc));
    }
}
