use super::*;
use crate::fx::effects::holographic_foil::{table::band, HolographicFoil};
use crate::fx::params::Params;
use crate::fx::registry::EffectMetadata;

/// Band widths in nm, as holospot's error table has them.
const WIDTHS: [f64; 7] = [2.0, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0];

/// What the packer hands the kernel for spectrum `s` on a foil whose Tint
/// choice is `tint`: the term list, the Planck factor and the dye, read back
/// from the rows `pack.rs` lays out (the dye at 7, spectrum `s` at 27 + 4 s,
/// the terms from 65, two rows each).
fn packed_list(tint: u32, s: usize) -> (Vec<Term>, [f64; 2], Tint) {
    let f = HolographicFoil {
        tint,
        ..<HolographicFoil as EffectMetadata>::read(Params::EMPTY)
    };
    let b = f.packed(Params::EMPTY, 64, 64).blocks;
    let row = |i: usize| b[i].map(f64::from);
    let [kind, edge, width, floor] = row(7);
    let dye = Tint {
        kind: kind as u32,
        edge,
        width,
        floor,
    };
    let [first, count, temp, k] = row(27 + 4 * s);
    let terms = (first as usize..(first + count) as usize)
        .map(|i| {
            let [r, g, bl, c] = row(65 + 2 * i);
            let [s, cut, side, _] = row(66 + 2 * i);
            Term {
                amp: [r, g, bl],
                c,
                s,
                cut,
                side,
            }
        })
        .collect();
    (terms, [temp, k], dye)
}

/// The light the foil sends back per nm, the long way: spectrum times metal
/// and dye times the observer, every half nm from 300 to 900.
fn integrand(s: usize, dye: &Tint) -> Vec<(f64, [f64; 3])> {
    let t = tables();
    (0..1201)
        .map(|i| {
            let l = 300.0 + 0.5 * f64::from(i);
            let k = t.scale[s] * raw(s, l) * dye.metal(l);
            (l, to_rgb(cmf(l), t.inv_i).map(|c| c * k))
        })
        .collect()
}

/// That light seen through a unit Gaussian band at `lam`, `sig` nm wide.
fn brute(light: &[(f64, [f64; 3])], lam: f64, sig: f64) -> [f64; 3] {
    // The step, half a nm, times the band's peak.
    let norm = 0.5 / (sig * (2.0 * std::f64::consts::PI).sqrt());
    let mut out = [0.0; 3];
    for &(l, rgb) in light {
        let d = (l - lam) / sig;
        if d.abs() < 9.0 {
            let w = norm * (-0.5 * d * d).exp();
            for (o, c) in out.iter_mut().zip(rgb) {
                *o += w * c;
            }
        }
    }
    out
}

/// The kernel's closed-form band colour misses the brute-force sum by no
/// more than holospot measured (spec 1.8), a little over, for every light
/// spectrum, the Clear, Gold and Blue dyes and band widths from 2 to 160 nm.
/// It is the one place a slip in the maths would give wrong colours without
/// looking broken. `table::band` is the kernel's `holo_band` line for line.
#[test]
fn the_closed_form_band_colour_matches_a_brute_force_sum() {
    let mut misses = Vec::new();
    for tint in [0u32, 1, 2] {
        for s in 0..SPECTRA {
            let (terms, planck, dye) = packed_list(tint, s);
            let light = integrand(s, &dye);
            for sig in WIDTHS {
                let (mut peak, mut worst) = (0.0f64, 0.0f64);
                for i in 0..=175 {
                    let lam = 380.0 + 2.0 * f64::from(i);
                    let want = brute(&light, lam, sig);
                    let got = band(lam, sig, &terms, planck, &dye);
                    for (g, w) in got.into_iter().zip(want) {
                        peak = peak.max(w.abs());
                        worst = worst.max((g - w).abs());
                    }
                }
                let share = worst / peak;
                let bound = match (tint, s) {
                    (2, TUNGSTEN) if sig >= 40.0 => 0.18,
                    (2, _) => 0.06,
                    (_, FLUORESCENT) | (1, _) => 0.04,
                    _ => 0.01,
                };
                if share > bound {
                    misses.push(format!("spectrum {s} tint {tint} width {sig}: {share:.4}"));
                }
            }
        }
    }
    assert!(misses.is_empty(), "{misses:?}");
}
