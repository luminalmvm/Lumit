//! A layer's Material options' part of a frame's name.

use lumit_core::model::{Composition, Layer};

use crate::feed_f64;

/// The material shades only a 3D layer and only where there is a light,
/// and is hashed only once it has moved off what layers had before it
/// existed, so those keys stay valid.
pub(crate) fn feed(h: &mut blake3::Hasher, comp: &Composition, layer: &Layer, lt: f64) {
    let m = &layer.transform.material;
    // Its shadows, and whether a Model layer is a holdout.
    let shadows = [
        m.casts_shadows as u8,
        m.accepts_shadows as u8,
        u8::from(m.holdout),
    ];
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
        h.update(&shadows);
        // Fed only once it is asked for, so the keys before it stay.
        let reflection = m.reflection.value_at(lt);
        if reflection != 0.0 {
            h.update(b"reflection");
            feed_f64(h, reflection);
        }
    } else if m.holdout || shadows[..2].contains(&(lumit_core::model::ShadowMode::Only as u8)) {
        // Set to Only or a holdout, a layer draws differently with no light too.
        h.update(b"shadow only");
        h.update(&shadows);
    }
}
