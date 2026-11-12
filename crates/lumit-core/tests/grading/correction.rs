//! The Correction's grade rows on the CPU twin, each against the sum its
//! module writes down, and the HDR zones against theirs.

use lumit_core::model::{EffectInstance, EffectValue};

use crate::support::{assert_near, at, at_xy, f, flat, fx, linear, luma, rgb, run, ss, strip};

/// Opaque colours across the range, light and dark.
const PROBES: [[f32; 3]; 5] = [
    [0.2, 0.4, 0.6],
    [0.05, 0.05, 0.05],
    [0.8, 0.3, 0.1],
    [0.44, 0.44, 0.44],
    [0.9, 0.9, 0.9],
];

/// The probes, then the first one again half clear and premultiplied.
fn probes() -> Vec<f32> {
    let mut img = strip(&PROBES);
    let u = PROBES[0];
    img.extend_from_slice(&[u[0] * 0.5, u[1] * 0.5, u[2] * 0.5, 0.5]);
    img
}

/// `node` over the probes gives `want` of each, and the half-clear pixel is
/// graded unpremultiplied and keeps its alpha.
fn check(node: EffectInstance, want: impl Fn([f32; 3]) -> [f32; 3], tol: f32, what: &str) {
    let out = run(&[node], &probes(), PROBES.len() as u32 + 1, 1);
    for (i, u) in PROBES.iter().enumerate() {
        assert_near(at(&out, i), want(*u), tol, &format!("{what}, probe {i}"));
    }
    let half = want(PROBES[0]).map(|c| c * 0.5);
    assert_near(
        at(&out, PROBES.len()),
        half,
        tol,
        &format!("{what}, half clear"),
    );
    assert_eq!(out[PROBES.len() * 4 + 3], 0.5, "{what}: alpha is kept");
}

/// `u` with its Cb and Cr turned by `deg`, Y kept, worked out by hand from
/// Rec. 709's YCbCr.
fn turned(u: [f32; 3], deg: f32) -> [f32; 3] {
    let y = luma(u);
    let (cb, cr) = ((u[2] - y) / 1.8556, (u[0] - y) / 1.5748);
    let (s, c) = deg.to_radians().sin_cos();
    let (cb, cr) = (cb * c - cr * s, cb * s + cr * c);
    let r = y + 1.5748 * cr;
    let b = y + 1.8556 * cb;
    [r, (y - 0.2126 * r - 0.0722 * b) / 0.7152, b]
}

/// A fresh Correction hands the picture on bit for bit, and so does a grade
/// at Mix 0. Then each group of rows, alone, grades as its formula says, in
/// each of the three spaces.
#[test]
fn each_grade_row_of_a_correction_follows_its_formula() {
    let img = probes();
    let n = PROBES.len() as u32 + 1;
    assert_eq!(run(&[fx("correction", &[])], &img, n, 1), img);
    let none = linear(&[("gain", rgb([2.0, 2.0, 2.0])), ("mix", f(0.0))]);
    assert_eq!(run(&[none], &img, n, 1), img);

    // Wheels.
    check(
        linear(&[("gain", rgb([2.0, 1.0, 0.5]))]),
        |u| [2.0 * u[0], u[1], 0.5 * u[2]],
        1e-5,
        "gain",
    );
    check(
        linear(&[
            ("offset", rgb([0.1, 0.0, -0.02])),
            ("lift", rgb([0.2, 0.0, 0.1])),
        ]),
        |u| {
            let o = [0.1, 0.0, -0.02];
            let l = [0.2, 0.0, 0.1];
            [0, 1, 2].map(|c| {
                let x = u[c] + o[c];
                x + l[c] * (1.0 - x)
            })
        },
        1e-5,
        "offset and lift",
    );
    check(
        linear(&[("gamma", rgb([0.5, 0.0, -0.5]))]),
        |u| [u[0].sqrt(), u[1], u[2] * u[2]],
        1e-5,
        "gamma",
    );
    // In video gamma a gain of 1.5 is 1.5^2.4 in scene linear; in ACEScct an
    // offset of 0.1 is 17.52 tenths of a stop.
    check(
        fx("correction", &[("gain", rgb([1.5, 1.5, 1.5]))]),
        |u| u.map(|c| c * 1.5f32.powf(2.4)),
        1e-4,
        "gain in video",
    );
    check(
        fx(
            "correction",
            &[
                ("space", EffectValue::Choice(1)),
                ("offset", rgb([0.1, 0.1, 0.1])),
            ],
        ),
        |u| u.map(|c| c * 1.752f32.exp2()),
        1e-4,
        "offset in log",
    );
    check(
        linear(&[
            ("shadow", rgb([0.1, 0.0, 0.0])),
            ("midtone", rgb([0.0, 0.1, 0.0])),
            ("highlight", rgb([0.0, 0.0, 0.1])),
        ]),
        |u| {
            let y = luma(u);
            let s = ss(0.233, 0.433, y);
            let h = ss(0.45, 0.65, y);
            [u[0] + 0.1 * (1.0 - s), u[1] + 0.1 * (s - h), u[2] + 0.1 * h]
        },
        1e-5,
        "log wheels",
    );

    // Adjust.
    check(
        linear(&[("exposure", f(1.0))]),
        |u| u.map(|c| 2.0 * c),
        1e-5,
        "exposure",
    );
    check(
        linear(&[("contrast", f(200.0)), ("pivot", f(0.5))]),
        |u| u.map(|c| (c - 0.5) * 2.0 + 0.5),
        1e-5,
        "contrast",
    );
    check(
        linear(&[("saturation", f(0.0))]),
        |u| [luma(u); 3],
        1e-5,
        "saturation",
    );
    check(
        linear(&[("hue", f(90.0))]),
        |u| turned(u, 90.0),
        1e-5,
        "hue",
    );
    check(
        linear(&[("shadows", f(100.0))]),
        |u| {
            let lift = 0.25 * (1.0 - ss(0.0, 0.5, luma(u)));
            u.map(|c| c + lift)
        },
        1e-5,
        "shadows",
    );
    check(
        linear(&[
            ("mixer_red", rgb([0.0, 0.0, 1.0])),
            ("mixer_blue", rgb([1.0, 0.0, 0.0])),
        ]),
        |u| [u[2], u[1], u[0]],
        1e-5,
        "mixer",
    );
    check(
        linear(&[
            ("mixer_red", rgb([0.0, 0.0, 1.0])),
            ("mixer_blue", rgb([1.0, 0.0, 0.0])),
            ("mixer_keep_luma", EffectValue::Bool(true)),
        ]),
        |u| {
            let back = luma(u) - luma([u[2], u[1], u[0]]);
            [u[2] + back, u[1] + back, u[0] + back]
        },
        1e-5,
        "mixer keeping luma",
    );

    // Curves.
    check(
        linear(&[
            ("curve_y", EffectValue::Curve(vec![[0.0, 0.0], [1.0, 0.5]])),
            ("curve_r", EffectValue::Curve(vec![[0.0, 0.0], [1.0, 0.8]])),
        ]),
        |u| [0.4 * u[0], 0.5 * u[1], 0.5 * u[2]],
        1e-5,
        "curves",
    );
    check(
        linear(&[("soft_high", f(60.0)), ("soft_low", f(40.0))]),
        |u| {
            u.map(|c| {
                let (kl, kh) = (0.2, 0.7);
                if c > kh {
                    let t = (c - kh) / (1.0 - kh);
                    kh + (1.0 - kh) * t / (1.0 + t)
                } else if c < kl {
                    let t = (kl - c) / kl;
                    kl - kl * t / (1.0 + t)
                } else {
                    c
                }
            })
        },
        1e-5,
        "soft clip",
    );
    // Hue v hue at three quarters everywhere turns every hue a quarter; Hue v
    // sat at nought takes every colour out.
    check(
        linear(&[(
            "hue_hue",
            EffectValue::Curve(vec![[0.0, 0.75], [0.5, 0.75]]),
        )]),
        |u| turned(u, 90.0),
        1e-4,
        "hue v hue",
    );
    check(
        linear(&[("hue_sat", EffectValue::Curve(vec![[0.0, 0.0], [1.0, 0.0]]))]),
        |u| [luma(u); 3],
        1e-5,
        "hue v sat",
    );
    check(
        linear(&[("gain", rgb([2.0, 2.0, 2.0])), ("mix", f(50.0))]),
        |u| u.map(|c| 1.5 * c),
        1e-5,
        "mix",
    );

    // Midtone detail leaves a flat picture alone and sharpens an edge.
    let detail = || linear(&[("midtone_detail", f(100.0))]);
    let grey = flat(8, 8, [0.3, 0.3, 0.3]);
    let out = run(&[detail()], &grey, 8, 8);
    assert_near(at(&out, 27), [0.3; 3], 1e-4, "detail on flat grey");
    let (w, h) = (64u32, 4u32);
    let edge: Vec<f32> = (0..w * h)
        .flat_map(|i| {
            let v = if i % w < w / 2 { 0.2 } else { 0.6 };
            [v, v, v, 1.0]
        })
        .collect();
    let out = run(&[detail()], &edge, w, h);
    assert!(at_xy(&out, w, 31, 2)[1] < 0.2, "the dark side darkens");
    assert!(at_xy(&out, w, 32, 2)[1] > 0.6, "the light side lightens");
}

/// Each HDR zone moves the stops it covers and leaves the rest, and the
/// global balance moves colour without moving luma.
#[test]
fn hdr_zones_move_only_their_own_stops() {
    let stops = [-4.0f32, 0.0, 7.0];
    let greys: Vec<[f32; 3]> = stops.iter().map(|s| [0.18 * s.exp2(); 3]).collect();
    let img = strip(&greys);
    let n = greys.len() as u32;
    let zone = |rows: &[(&str, EffectValue)], want: [f32; 3], what: &str| {
        let out = run(&[linear(rows)], &img, n, 1);
        for (i, g) in greys.iter().enumerate() {
            let tol = 1e-4 * g[0].max(1.0);
            assert_near(at(&out, i), g.map(|c| c * want[i]), tol, what);
        }
    };
    // Specular from 4 stops up: the spike halves, the rest stays.
    zone(
        &[("hdr_specular_exposure", f(-1.0))],
        [1.0, 1.0, 0.5],
        "specular",
    );
    // Shadow from 1 stop down over a falloff of 2: whole at -4, half at grey.
    zone(
        &[("hdr_shadow_exposure", f(1.0))],
        [2.0, 0.5f32.exp2(), 1.0],
        "shadow",
    );

    // Light saturation at nought greys the bright colour only.
    let bright = [2.0f32, 1.3, 1.0];
    let dark = [0.02f32, 0.01, 0.005];
    let out = run(
        &[linear(&[("hdr_light_saturation", f(0.0))])],
        &strip(&[bright, dark]),
        2,
        1,
    );
    assert_near(at(&out, 0), [luma(bright); 3], 1e-4, "light saturation");
    assert_near(at(&out, 1), dark, 1e-6, "below the light zone");

    // Black offset lifts black and fades out by mid grey.
    let out = run(
        &[linear(&[("hdr_black_offset", f(0.01))])],
        &strip(&[[0.0; 3], [0.5; 3]]),
        2,
        1,
    );
    assert_near(at(&out, 0), [0.01; 3], 1e-6, "black offset at black");
    assert_near(at(&out, 1), [0.5; 3], 1e-6, "black offset past grey");

    // A stop more red, luma put back.
    let u = PROBES[0];
    let out = run(
        &[linear(&[("hdr_balance", rgb([1.0, 0.0, 0.0]))])],
        &strip(&[u]),
        1,
        1,
    );
    let t = [2.0 * u[0], u[1], u[2]];
    let k = luma(u) / luma(t);
    assert_near(at(&out, 0), t.map(|c| c * k), 1e-5, "global balance");
}
