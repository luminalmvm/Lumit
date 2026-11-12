//! CDL in and out: the CDL effect against the ASC formula worked by hand, and
//! a Correction no CDL can say exactly, fitted closely.

use lumit_core::fx::effects::cdl::Cdl;
use lumit_core::fx::effects::correction::Correction;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::grade_solve::cdl_fit::cdl_of;

use crate::support::{assert_near, at, f, fx, resolve, rgb, run, strip};

const PROBES: [[f32; 3]; 4] = [
    [0.2, 0.4, 0.6],
    [0.05, 0.05, 0.05],
    [0.8, 0.3, 0.1],
    [0.9, 0.9, 0.9],
];

/// The ASC list on one scene-linear colour, worked on video gamma.
fn asc(u: [f32; 3], slope: [f32; 3], offset: [f32; 3], power: [f32; 3], sat: f32) -> [f32; 3] {
    let v = u.map(|c| c.powf(1.0 / 2.4));
    let x: [f32; 3] =
        std::array::from_fn(|c| (v[c] * slope[c] + offset[c]).clamp(0.0, 1.0).powf(power[c]));
    let y = 0.2126 * x[0] + 0.7152 * x[1] + 0.0722 * x[2];
    x.map(|c| (y + sat * (c - y)).clamp(0.0, 1.0).powf(2.4))
}

#[test]
fn cdl_follows_the_asc_formula_and_a_correction_fits_close() {
    let mut img = strip(&PROBES);
    let half = PROBES[0];
    img.extend_from_slice(&[half[0] * 0.5, half[1] * 0.5, half[2] * 0.5, 0.5]);
    let n = PROBES.len() as u32 + 1;
    assert_eq!(run(&[fx("cdl", &[])], &img, n, 1), img, "at rest");

    let (slope, offset, power, sat) = ([1.2, 1.0, 0.8], [0.02, 0.0, -0.01], [1.1, 1.0, 0.9], 0.8);
    let rows = |mix: f64| {
        let wide = |c: [f32; 3]| c.map(f64::from);
        fx(
            "cdl",
            &[
                ("slope", rgb(wide(slope))),
                ("offset", rgb(wide(offset))),
                ("power", rgb(wide(power))),
                ("saturation", f(f64::from(sat))),
                ("mix", f(mix)),
            ],
        )
    };
    let out = run(&[rows(100.0)], &img, n, 1);
    for (i, u) in PROBES.iter().enumerate() {
        let want = asc(*u, slope, offset, power, sat);
        assert_near(at(&out, i), want, 1e-4, &format!("probe {i}"));
    }
    let want = asc(half, slope, offset, power, sat).map(|c| c * 0.5);
    assert_near(at(&out, PROBES.len()), want, 1e-4, "half clear");
    assert_eq!(out[PROBES.len() * 4 + 3], 0.5);

    // At Mix 50 the halfway point is on the video signal.
    let out = run(&[rows(50.0)], &img, n, 1);
    let u = PROBES[2];
    let full = asc(u, slope, offset, power, sat);
    let want: [f32; 3] = std::array::from_fn(|c| {
        (0.5 * u[c].powf(1.0 / 2.4) + 0.5 * full[c].powf(1.0 / 2.4)).powf(2.4)
    });
    assert_near(at(&out, 2), want, 1e-4, "mix");

    // The GPU's uniform holds the same numbers.
    let ops = resolve(&[rows(100.0)]);
    let pk = Cdl::read(ops.get(0).unwrap().params).packed();
    let words = pk.words();
    assert_eq!(words[0], [0.0, sat, 1.0, 0.0]);
    assert_eq!(words[1][..3], slope);
    assert_eq!(words[3][..3], power);

    // A CDL has no Contrast row, so these numbers are a fit. On video gamma
    // contrast is a straight line, so the fit lands on it.
    let node = fx(
        "correction",
        &[
            ("contrast", f(80.0)),
            ("pivot", f(0.5)),
            ("saturation", f(80.0)),
        ],
    );
    let ops = resolve(&[node]);
    let node = Correction::read(ops.get(0).unwrap().params);
    let fit = cdl_of(&node);
    assert!(!fit.exact);
    for c in 0..3 {
        assert!((fit.slope[c] - 0.8).abs() < 0.01, "{fit:?}");
        assert!((fit.offset[c] - 0.1).abs() < 0.01, "{fit:?}");
        assert!((fit.power[c] - 1.0).abs() < 0.02, "{fit:?}");
    }
    assert!((fit.saturation - 0.8).abs() < 0.05, "{fit:?}");
    let mut cdl = Cdl::read(Params::EMPTY);
    cdl.slope = [fit.slope[0], fit.slope[1], fit.slope[2], 1.0];
    cdl.offset = [fit.offset[0], fit.offset[1], fit.offset[2], 1.0];
    cdl.power = [fit.power[0], fit.power[1], fit.power[2], 1.0];
    cdl.saturation = fit.saturation;
    let cdl = cdl.packed();
    let pk = node.packed(1.0);
    for i in 0..16 {
        let g = 0.05 + 0.04 * i as f32;
        let u = [g, g * 0.9, g * 1.1].map(|c| c.powf(2.4));
        let want = lumit_core::fx::effects::correction::maths::grade(
            [u[0], u[1], u[2], 1.0],
            1.0,
            0.0,
            &pk,
        );
        let got = cdl.pixel([u[0], u[1], u[2], 1.0]);
        assert_near(
            [got[0], got[1], got[2]],
            [want[0], want[1], want[2]],
            2e-3,
            &format!("fit at {g}"),
        );
    }
}
