//! A particle's colour along whatever runs it: the blend from Colour to End
//! colour, or the ramp Colour over life bakes from them.

use super::ParticleLook;
use crate::fx::cpu::curve_at;

impl ParticleLook {
    /// The tables the card reads, one after another: size over life, opacity
    /// over life, then the colour ramp's red, green, blue and alpha when there
    /// is one.
    #[must_use]
    pub fn curves(&self) -> Vec<f32> {
        let mut c = Vec::with_capacity(self.size_curve.len() * (2 + self.colour_ramp.len()));
        c.extend_from_slice(&self.size_curve);
        c.extend_from_slice(&self.opacity_curve);
        for table in &self.colour_ramp {
            c.extend_from_slice(table);
        }
        c
    }
}

/// The premultiplied colour `toward` of the way along, for a particle as
/// opaque as `alpha`. Mixed in working space, which is one lerp and no encode.
pub(super) fn tint(look: &ParticleLook, toward: f32, alpha: f32) -> [f32; 4] {
    if let [r, g, b, a] = &look.colour_ramp[..] {
        let alpha = alpha * curve_at(toward, a);
        return [
            curve_at(toward, r) * alpha,
            curve_at(toward, g) * alpha,
            curve_at(toward, b) * alpha,
            alpha,
        ];
    }
    let mut colour = [alpha; 4];
    for (c, (from, to)) in colour
        .iter_mut()
        .zip(look.colour.iter().zip(look.end_colour.iter()))
        .take(3)
    {
        *c = (from + (to - from) * toward) * alpha;
    }
    colour
}
