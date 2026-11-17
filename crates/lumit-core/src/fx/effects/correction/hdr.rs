//! The Correction's HDR zones: six tonal zones, each with its own colour,
//! exposure and saturation, worked in scene linear before the grading space.
//!
//! **In plain terms.** The picture is split by brightness, counted in stops
//! from mid grey (0.18). Black, Dark and Shadow reach from their range down;
//! Light, Highlight and Specular from their range up. Each fades in over the
//! shared Falloff, so Shadow and Light overlap by two stops round grey at the
//! defaults, and the trims sit inside them. Where zones overlap their changes
//! add up.
//!
//! # Rows
//!
//! `hdr_balance` (colour, stops a channel), `hdr_black_offset` (scene linear),
//! `hdr_falloff` (stops), then for each zone `z` in `black`, `dark`,
//! `shadow`, `light`, `highlight`, `specular`: `hdr_z_balance` (colour, stops
//! a channel), `hdr_z_exposure` (stops), `hdr_z_saturation` (per cent) and
//! `hdr_z_range` (stops from grey).
//!
//! # The formulas
//!
//! On `u`, the pixel after Exposure and Temperature, before the encode:
//! 1. `Y = luma(u)`, `s = clamp(log2(max(Y, 0.18·2^−16)/0.18), −16, 16)`.
//! 2. A zone's weight, `F` the Falloff and `r` its range: `1 − ss(r − F, r, s)`
//!    for Black, Dark and Shadow, `ss(r, r + F, s)` for the other three. Its
//!    exposure uses the same ramp widened to `max(F, 1.5·|E|)`, so even a big
//!    pull never folds the tones over.
//! 3. `B = balance + Σ w·zone balance`, `E = Σ w_e·zone exposure`,
//!    `S = max(0, 1 + Σ w·(zone saturation − 1))`.
//! 4. `t = u·2^B`, then `t *= Y/luma(t)` when both are above nought, so the
//!    balance moves colour and not brightness; `t = Y + S·(t − Y)`;
//!    `t *= 2^E`; then each channel `t += offset·(1 − ss(0, 0.18, t))`.
//!
//! Every row at rest skips all of it, so the picture goes on untouched.

use super::Correction;

/// Mid grey, the zones' nought.
pub const GREY: f32 = 0.18;

/// The most stops either side of grey a zone reads.
pub const STOPS: f32 = 16.0;

/// Zones that reach from their range down: Black, Dark and Shadow come first.
pub const DOWN_ZONES: usize = 3;

/// How many `vec4`s [`Hdr::words`] writes.
pub const HDR_WORDS: usize = 2 + 2 * 6;

/// One zone, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Zone {
    /// Stops a channel.
    pub balance: [f32; 3],
    /// Stops.
    pub exposure: f32,
    /// Stops from grey.
    pub range: f32,
    /// A factor, 1 at rest.
    pub sat: f32,
}

/// The zones and the global rows, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Hdr {
    /// Whether any row moves the picture.
    pub on: bool,
    /// Stops.
    pub falloff: f32,
    /// Scene linear.
    pub black_offset: f32,
    /// Stops a channel.
    pub balance: [f32; 3],
    /// Black, Dark, Shadow, Light, Highlight, Specular.
    pub zones: [Zone; 6],
}

impl Hdr {
    /// The rows, read from the Correction.
    #[must_use]
    pub fn of(c: &Correction) -> Self {
        let rgb = |v: [f32; 4]| [v[0], v[1], v[2]];
        let zone = |balance: [f32; 4], exposure: f32, sat: f32, range: f32| Zone {
            balance: rgb(balance),
            exposure: exposure.clamp(-STOPS, STOPS),
            range: range.clamp(-STOPS, STOPS),
            sat: sat.max(0.0) / 100.0,
        };
        let zones = [
            zone(
                c.hdr_black_balance,
                c.hdr_black_exposure,
                c.hdr_black_saturation,
                c.hdr_black_range,
            ),
            zone(
                c.hdr_dark_balance,
                c.hdr_dark_exposure,
                c.hdr_dark_saturation,
                c.hdr_dark_range,
            ),
            zone(
                c.hdr_shadow_balance,
                c.hdr_shadow_exposure,
                c.hdr_shadow_saturation,
                c.hdr_shadow_range,
            ),
            zone(
                c.hdr_light_balance,
                c.hdr_light_exposure,
                c.hdr_light_saturation,
                c.hdr_light_range,
            ),
            zone(
                c.hdr_highlight_balance,
                c.hdr_highlight_exposure,
                c.hdr_highlight_saturation,
                c.hdr_highlight_range,
            ),
            zone(
                c.hdr_specular_balance,
                c.hdr_specular_exposure,
                c.hdr_specular_saturation,
                c.hdr_specular_range,
            ),
        ];
        let mut h = Hdr {
            on: false,
            falloff: c.hdr_falloff.clamp(0.0, STOPS),
            black_offset: c.hdr_black_offset.clamp(-0.1, 0.1),
            balance: rgb(c.hdr_balance),
            zones,
        };
        h.on = h.black_offset != 0.0
            || h.balance != [0.0; 3]
            || h.zones
                .iter()
                .any(|z| z.balance != [0.0; 3] || z.exposure != 0.0 || z.sat != 1.0);
        h
    }

    /// The `vec4`s `fx_correction.wgsl` reads after the windows.
    #[must_use]
    pub fn words(&self) -> [[f32; 4]; HDR_WORDS] {
        let mut out = [[0.0f32; 4]; HDR_WORDS];
        out[0] = [f32::from(self.on), self.falloff, self.black_offset, 0.0];
        out[1] = [self.balance[0], self.balance[1], self.balance[2], 0.0];
        for (i, z) in self.zones.iter().enumerate() {
            out[2 + 2 * i] = [z.balance[0], z.balance[1], z.balance[2], z.exposure];
            out[3 + 2 * i] = [z.range, z.sat, 0.0, 0.0];
        }
        out
    }
}

/// How much of zone `i` the stop `s` is in, with the ramp `width` wide.
#[must_use]
pub fn weight(i: usize, s: f32, range: f32, width: f32) -> f32 {
    if i < DOWN_ZONES {
        1.0 - super::maths::ss(range - width, range, s)
    } else {
        super::maths::ss(range, range + width, s)
    }
}

/// The zones over one scene-linear colour.
#[must_use]
pub fn zones(u: [f32; 3], h: &Hdr) -> [f32; 3] {
    use super::maths::{luma, ss};
    let y = luma(u);
    let s = (y.max(GREY * (-STOPS).exp2()) / GREY)
        .log2()
        .clamp(-STOPS, STOPS);
    let mut b = h.balance;
    let mut e = 0.0f32;
    let mut sat = 1.0f32;
    for (i, z) in h.zones.iter().enumerate() {
        let w = weight(i, s, z.range, h.falloff);
        let we = weight(i, s, z.range, h.falloff.max(1.5 * z.exposure.abs()));
        for (bc, zc) in b.iter_mut().zip(z.balance) {
            *bc += w * zc;
        }
        e += we * z.exposure;
        sat += w * (z.sat - 1.0);
    }
    let sat = sat.max(0.0);
    let mut t = [0, 1, 2].map(|c| u[c] * b[c].exp2());
    let lt = luma(t);
    if y > 0.0 && lt > 0.0 {
        let k = y / lt;
        t = t.map(|v| v * k);
    }
    let gain = e.exp2();
    t.map(|v| {
        let v = (y + sat * (v - y)) * gain;
        v + h.black_offset * (1.0 - ss(0.0, GREY, v))
    })
}
