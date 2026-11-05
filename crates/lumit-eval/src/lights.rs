//! The comp's lights in its frame key.

use std::sync::Arc;

use lumit_core::model::{Composition, Document};
use uuid::Uuid;

use crate::{feed_f64, feed_layer, Quality, SourceStamper};

/// The comp's lights (docs/06). Hashed once here rather than per
/// layer, because a light shades every layer that accepts it — and hashed
/// only when the comp actually has one, so every key made before lighting
/// existed stays valid. A Light layer draws no pixels of its own, so
/// nothing else in this walk would notice it moving.
pub(crate) fn feed_lights(
    h: &mut blake3::Hasher,
    doc: &Arc<Document>,
    comp: &Composition,
    t: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
) -> Option<()> {
    // A box of smoke is read from its whole walk, whose rows and sources all feed
    // this name. Its layer may have no picture for the rest of this walk to notice.
    for layer in &comp.layers {
        if let Some(e) = lumit_core::fx::effects::smoke::on_layer(layer) {
            let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
            h.update(b"smoke/");
            h.update(&lumit_core::fx::effects::smoke::definition(
                doc, comp, layer, e,
            ));
            let _ = serde_json::to_writer(&mut *h, e);
            let _ = serde_json::to_writer(&mut *h, &layer.transform);
            feed_f64(h, layer.source_time_at(lt));
        }
    }
    let context = lumit_core::model::comp_context(doc, comp, t);
    let read = lumit_core::model::context_reader(&context, t);
    let lights = comp.lights_at(t, &read, stamper.solves().map(|store| (doc, store)));
    if !lights.is_empty() {
        h.update(b"lights/");
        if let lumit_core::model::CompLighting::AfterEffects { linear } = comp.lighting {
            h.update(b"after-effects/");
            h.update(&[u8::from(linear)]);
        }
        for l in &lights {
            h.update(&[match l.kind {
                lumit_core::model::LightKind::Point => 0u8,
                lumit_core::model::LightKind::Spot => 1,
                lumit_core::model::LightKind::Area => 2,
                lumit_core::model::LightKind::Parallel => 3,
                lumit_core::model::LightKind::Ambient => 4,
                lumit_core::model::LightKind::Environment => 5,
            }]);
            h.update(&[l.falloff as u8, u8::from(l.legacy_shape)]);
            for v in [
                l.position.0,
                l.position.1,
                l.z,
                l.half_size.0,
                l.half_size.1,
                l.cone_deg,
                l.rotation_deg,
                l.rotation_x_deg,
                l.rotation_y_deg,
                l.falloff_px,
                l.cone_feather,
                l.radius_px,
                f64::from(l.gain),
            ] {
                feed_f64(h, v);
            }
            l.colour.iter().for_each(|c| feed_f64(h, f64::from(*c)));
            if l.shadow_darkness > 0.0 {
                h.update(b"shadows/");
                feed_f64(h, f64::from(l.shadow_darkness));
                feed_f64(h, f64::from(l.shadow_diffusion));
            }
            l.haze.cache_key(|v| feed_f64(h, f64::from(v)));
        }
    }
    // The environment, fed only when there is one, its picture with it, as
    // the walk leaves out a layer that isn't drawn.
    if let Some(e) = comp.environment_at(t, &read, stamper.solves().map(|store| (doc, store))) {
        h.update(b"environment/");
        let (x, y, z) = e.rotation_deg;
        for v in [x, y, z, f64::from(e.blur)] {
            feed_f64(h, v);
        }
        e.colour.iter().for_each(|c| feed_f64(h, f64::from(*c)));
        h.update(&[u8::from(e.background)]);
        if let lumit_core::model::CompLighting::AfterEffects { linear } = comp.lighting {
            h.update(&[u8::from(linear)]);
        }
        if let Some(source) = comp.layers.iter().find(|l| l.id == e.source) {
            let lt = lumit_core::time::layer_time(t, source.start_offset.0);
            feed_layer(h, doc, comp, source, t, lt, quality, stamper, visited)?;
        }
    }
    Some(())
}
