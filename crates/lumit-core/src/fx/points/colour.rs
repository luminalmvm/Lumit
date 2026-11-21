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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::cpu::{curve_identity_table, CURVE_TABLE};
    use crate::fx::points::Drive;

    fn look(ramp: Vec<[f32; CURVE_TABLE]>) -> ParticleLook {
        ParticleLook {
            life: 1.0,
            life_jitter: 0.0,
            size: 10.0,
            size_jitter: 0.0,
            size_curve: curve_identity_table(),
            opacity_curve: curve_identity_table(),
            colour: [1.0, 0.0, 0.0, 1.0],
            end_colour: [0.0, 0.0, 1.0, 1.0],
            colour_ramp: ramp,
            rotation_deg: 0.0,
            rotation_jitter_deg: 0.0,
            spin_deg: 0.0,
            align_to_motion: false,
            size_by: Drive::Age,
            opacity_by: Drive::Age,
            colour_by: Drive::Age,
            speed_range: [0.0, 1.0],
        }
    }

    /// With no ramp a particle blends from Colour to End colour, and with one
    /// it reads the tables, alpha and all. The card gets the tables after the
    /// two curves.
    #[test]
    fn a_ramp_replaces_the_blend() {
        let plain = look(Vec::new());
        assert_eq!(plain.curves().len(), 2 * CURVE_TABLE);
        let half = tint(&plain, 0.5, 0.5);
        assert_eq!(half, [0.25, 0.0, 0.25, 0.5]);
        let ramp = look(vec![
            [0.0; CURVE_TABLE],
            [1.0; CURVE_TABLE],
            [0.0; CURVE_TABLE],
            [0.5; CURVE_TABLE],
        ]);
        assert_eq!(ramp.curves().len(), 6 * CURVE_TABLE);
        let got = tint(&ramp, 0.5, 1.0);
        assert!((got[3] - 0.5).abs() < 1e-4 && (got[1] - 0.5).abs() < 1e-4);
        assert!(got[0].abs() < 1e-6 && got[2].abs() < 1e-6, "{got:?}");
    }
}
