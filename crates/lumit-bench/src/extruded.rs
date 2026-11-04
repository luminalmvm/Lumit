//! **B39: one letter's mesh** (docs/13 §2).
//!
//! What a playing type-on pays for each new letter of an extruded title: a
//! letter-sized outline with two holes, cleaned, bevelled in four segments on
//! both sides and capped. The letters it has already shown come from the cache.

use crate::scenarios::{elapsed_ms, Measurement};
use lumit_core::extrude::mesh::solid;
use lumit_core::extrude::{BevelStyle, Profile};
use std::time::Instant;

const RUNS: u32 = 50;

/// A ring of `n` points about (`x`, `y`), turning as a letter's outer outline
/// does, or the other way for a hole.
fn ring(x: f64, y: f64, rx: f64, ry: f64, n: u32, hole: bool) -> Vec<[f64; 2]> {
    (0..n)
        .map(|k| {
            let a = f64::from(k) / f64::from(n) * std::f64::consts::TAU;
            let a = if hole { -a } else { a };
            [x + rx * a.cos(), y + ry * a.sin()]
        })
        .collect()
}

/// Measure B39.
pub fn budgets(progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
    // About an 8 at 200 pixels.
    let letter = vec![
        ring(60.0, 100.0, 60.0, 100.0, 160, false),
        ring(60.0, 55.0, 30.0, 32.0, 80, true),
        ring(60.0, 145.0, 34.0, 36.0, 80, true),
    ];
    let profile = Profile {
        depth: 60.0,
        bevel: 6.0,
        style: BevelStyle::Convex,
        segments: 4,
        hole: 1.0,
    };
    let mut best = f64::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        let mesh = solid(&letter, &profile);
        best = best.min(elapsed_ms(start));
        std::hint::black_box(mesh);
    }
    let m = Measurement {
        budget: "B39",
        value_ms: best,
        frames: u64::from(RUNS),
    };
    progress(m);
    Ok(vec![m])
}
