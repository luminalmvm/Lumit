//! Holographic foil's band colour tables, for the small elements of glitter
//! and sparkle: the kernel's closed-form band colour, summed here once a
//! frame on a grid of band centres and widths for each spectrum the lights
//! use, so a flake reads four rows instead of summing every term.
//!
//! | Row | Holds |
//! |---|---|
//! | 288, 289 | the table each spectrum reads, -1 for none |
//! | 290.. | per table, 10 widths of 96 centres: linear sRGB |

use super::spectra::{
    equal_terms, fluorescent_terms, led_terms, planck_factor, Term, Tint, FLUORESCENT,
    LED_SPECTRUM, SPECTRA,
};

const MAP: usize = 288;
const AT: usize = 290;
/// Band centres from 280 nm, every 7.
const CENTRES: usize = 96;
/// Band widths from 0.5 nm, doubling every 0.9 steps.
const WIDTHS: usize = 10;
/// At most two tables; other spectra keep the exact sums.
const TABLES: usize = 2;

/// The rows the tables take, from the first.
pub(super) const ROWS: usize = AT + TABLES * CENTRES * WIDTHS;

/// The normal CDF as the kernel has it (Abramowitz and Stegun 7.1.26).
fn cdf(u: f64) -> f64 {
    let x = u.abs() * std::f64::consts::FRAC_1_SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let tail = 0.5 * poly * (-x * x).exp();
    if u < 0.0 {
        tail
    } else {
        1.0 - tail
    }
}

fn pdf(u: f64) -> f64 {
    0.398_942_28 * (-0.5 * u * u).exp()
}

/// The slow factor at a band's mean `m` and spread `sd`, as the kernel's.
fn slow(m: f64, sd: f64, planck: [f64; 2], tint: &Tint) -> f64 {
    let mut f = 0.889 - 1.25e-4 * (m.clamp(380.0, 780.0) - 450.0);
    if planck[0] > 0.0 {
        let lm = m.max(250.0);
        let r = 560.0 / lm;
        f *= planck[1] * r.powi(5) / ((1.4388e7 / (lm * planck[0])).exp() - 1.0);
    }
    if tint.kind > 0 {
        let z = (m - tint.edge) / (0.25 * tint.width * tint.width + sd * sd).sqrt();
        let s = cdf(if tint.kind == 1 { z } else { -z });
        f *= tint.floor + (1.0 - tint.floor) * s;
    }
    f
}

/// The colour of a band at `lam` nm, `sig` nm wide, over `terms`.
pub(super) fn band(lam: f64, sig: f64, terms: &[Term], planck: [f64; 2], tint: &Tint) -> [f64; 3] {
    let mut acc = [0.0; 3];
    let s2 = sig * sig;
    for t in terms {
        let v = t.s * t.s + s2;
        let dl = lam - t.c;
        let e = dl * dl / (2.0 * v);
        if e > 14.0 {
            continue;
        }
        let g = t.s / v.sqrt() * (-e).exp();
        let k = if t.side == 0.0 {
            g
        } else {
            let mu = (t.c * s2 + lam * t.s * t.s) / v;
            let tau = t.s * sig / v.sqrt();
            let u = t.side * (mu - t.cut) / tau;
            let cp = cdf(u);
            let r = pdf(u) / cp.max(1e-6);
            let m = mu + t.side * tau * r;
            let sd = tau * (1.0 - u * r - r * r).max(0.02).sqrt();
            g * cp * slow(m, sd, planck, tint)
        };
        for (a, x) in acc.iter_mut().zip(t.amp) {
            *a += x * k;
        }
    }
    acc
}

/// Writes a table for each of the first two spectra in `used` and the map
/// that finds them.
pub(super) fn put(used: &[usize], tint: Tint, put: &mut dyn FnMut(usize, [f64; 4])) {
    let mut map = [-1.0; 8];
    let mut slot = 0;
    for &s in used {
        if s >= SPECTRA || map[s] >= 0.0 || slot >= TABLES {
            continue;
        }
        map[s] = slot as f64;
        let terms: Vec<Term> = match s {
            LED_SPECTRUM => led_terms().collect(),
            FLUORESCENT => fluorescent_terms(tint).collect(),
            _ => equal_terms().collect(),
        };
        let planck = planck_factor(s);
        for j in 0..WIDTHS {
            let sig = 0.5 * (j as f64 / 0.9).exp2();
            for i in 0..CENTRES {
                let lam = 280.0 + 7.0 * i as f64;
                let c = band(lam, sig, &terms, planck, &tint);
                put(
                    AT + (slot * WIDTHS + j) * CENTRES + i,
                    [c[0], c[1], c[2], 0.0],
                );
            }
        }
        slot += 1;
    }
    put(MAP, [map[0], map[1], map[2], map[3]]);
    put(MAP + 1, [map[4], -1.0, -1.0, -1.0]);
}
