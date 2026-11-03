//! The comp's lights in its frame key.

use std::sync::Arc;

use lumit_core::model::{Composition, Document};

use crate::{feed_f64, SourceStamper};

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
    stamper: &dyn SourceStamper,
) {
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
}
