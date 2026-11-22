//! Holographic foil's renormalisation of the grating's orders.
//!
//! The thin-phase model hands each order J_m squared of the light, which adds
//! up to one only when every order can leave. Orders past the lacquer's reach
//! never form, and orders that reach it but not the air bounce back off the
//! lacquer's top to the metal, which sends part of them round again. The
//! factor here puts that light back on the orders that do leave. It is worked
//! out at the sheet's centre for each light, at four wavelengths the kernel
//! reads between, and for up to three pitches.

use super::spectra::{Tint, NODES};

/// Orders tried on each side of the mirror: enough for the finest pitch the
/// rows allow at the bluest node.
const ORDERS: i32 = 16;

/// The thin-phase limit the kernel holds the phase to.
const PHASE_MAX: f64 = 5.5;

/// Bessel function J_m by its series, plenty of terms for the phases here.
fn bessel(m: u32, x: f64) -> f64 {
    let h = 0.5 * x;
    let mut term = 1.0;
    for k in 1..=m {
        term *= h / f64::from(k);
    }
    let mut sum = term;
    for k in 1..40 {
        let kf = f64::from(k);
        term *= -h * h / (kf * (kf + f64::from(m)));
        sum += term;
        if term.abs() < 1e-14 {
            break;
        }
    }
    sum
}

/// A dielectric's unpolarised reflectance at cosine `c`, index `n`.
fn fresnel(c: f64, n: f64) -> f64 {
    let c = c.clamp(1e-4, 1.0);
    let g = (n * n - 1.0 + c * c).max(0.0).sqrt();
    let a = (g - c) / (g + c);
    let b = (c * (g + c) - 1.0) / (c * (g - c) + 1.0);
    0.5 * a * a * (1.0 + b * b)
}

/// How much of the light inside the lacquer with tangential part `u` the
/// lacquer's top sends back: Fresnel below the critical angle, all of it past
/// it, averaged over a small window so the turn is not a step.
fn back(u: f64, nc: f64) -> f64 {
    let one = |u: f64| {
        if u < 1.0 {
            fresnel((1.0 - u * u).max(0.0).sqrt(), nc)
        } else {
            1.0
        }
    };
    (-3..=3).map(|k| one(u + 0.01 * f64::from(k))).sum::<f64>() / 7.0
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the grating needs: 1 / pitch in 1/nm, groove depth in nm and the
/// lacquer's index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Grating {
    pub inv_d: f64,
    pub depth: f64,
    pub nc: f64,
}

/// The factor at `lam` nm for light arriving with tangential part `l` (in air,
/// on the sheet) on grooves whose normal is `g`.
fn factor(gr: Grating, l: [f64; 2], g: [f64; 2], lam: f64, metal: f64) -> f64 {
    let nc = gr.nc;
    let lg = l[0] * g[0] + l[1] * g[1];
    let lt = -l[0] * g[1] + l[1] * g[0];
    let ci = (1.0 - (l[0] * l[0] + l[1] * l[1]) / (nc * nc))
        .max(0.0)
        .sqrt();
    let mut sum = 0.0;
    let mut trapped = 0.0;
    for m in -ORDERS..=ORDERS {
        let vx = f64::from(m) * lam * gr.inv_d - lg;
        let u = vx.hypot(lt);
        if u >= nc {
            continue;
        }
        let keep = smoothstep(nc, nc - 0.1, u);
        let co = (1.0 - u * u / (nc * nc)).max(0.0).sqrt();
        let ph = (std::f64::consts::PI * nc * gr.depth * (ci + co) / lam).min(PHASE_MAX);
        let j = bessel(m.unsigned_abs(), ph);
        let eta = keep * j * j;
        sum += eta;
        trapped += eta * back(u, nc);
    }
    1.0 / (sum - metal * trapped).max(0.2)
}

/// The factor at the four nodes, for grooves whose normal is `g`.
pub(super) fn nodes(gr: Grating, l: [f64; 2], g: [f64; 2], tint: &Tint) -> [f64; 4] {
    NODES.map(|lam| factor(gr, l, g, lam, tint.metal(lam)))
}

/// The same, averaged over eight groove directions: for patterns whose
/// grooves turn across the sheet.
pub(super) fn nodes_mean(gr: Grating, l: [f64; 2], tint: &Tint) -> [f64; 4] {
    let mut out = [0.0; 4];
    for k in 0..8 {
        let (s, c) = (std::f64::consts::PI * f64::from(k) / 8.0).sin_cos();
        for (o, v) in out.iter_mut().zip(nodes(gr, l, [c, s], tint)) {
            *o += v / 8.0;
        }
    }
    out
}
