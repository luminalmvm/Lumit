//! A layer's Material options' part of a frame's name.

use lumit_core::model::{Composition, Layer};

use crate::feed_f64;

/// The material shades only a 3D layer and only where there is a light,
/// and is hashed only once it has moved off what layers had before it
/// existed, so those keys stay valid.
pub(crate) fn feed(h: &mut blake3::Hasher, comp: &Composition, layer: &Layer, lt: f64) {
    let m = &layer.transform.material;
    if layer.switches.three_d
        && comp.layers.iter().any(|l| l.is_light())
        && **m != lumit_core::model::Material::legacy()
    {
        h.update(b"material");
        for p in [
            &m.ambient,
            &m.diffuse,
            &m.specular,
            &m.shininess,
            &m.metal,
            &m.light_transmission,
        ] {
            feed_f64(h, p.value_at(lt));
        }
        h.update(&[m.casts_shadows as u8, m.accepts_shadows as u8]);
    } else if [m.casts_shadows, m.accepts_shadows].contains(&lumit_core::model::ShadowMode::Only) {
        // Set to Only, a layer can go undrawn with no light in the comp too.
        h.update(b"shadow only");
        h.update(&[m.casts_shadows as u8, m.accepts_shadows as u8]);
    }
}
