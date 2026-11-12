//! Where a Correction lands: each qualifier mode and each window shape keys
//! the pixels it should and no others.
//!
//! Every node here doubles the picture in scene linear, so a pixel `u` keyed
//! `k` comes out `u·(1 + k)` and the key reads straight off it.

use lumit_core::fx::effects::correction::Correction;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::EffectValue;

use crate::support::{at, at_xy, f, flat, linear, luma, resolve, run, ss, strip};

const RED: [f32; 3] = [0.8, 0.1, 0.1];
const GREEN: [f32; 3] = [0.1, 0.7, 0.1];
const BLUE: [f32; 3] = [0.1, 0.1, 0.8];
const GREY: [f32; 3] = [0.3, 0.3, 0.3];
/// Red with little colour: `max − min` is 0.18.
const DULL: [f32; 3] = [0.28, 0.1, 0.1];
const COLOURS: [[f32; 3]; 5] = [RED, GREEN, BLUE, GREY, DULL];

/// The key a doubled pixel was given.
fn key(out: [f32; 3], u: [f32; 3]) -> f32 {
    out[1] / u[1] - 1.0
}

/// A doubling node with `rows`, over the five colours: each one's key.
fn keys(rows: &[(&str, EffectValue)]) -> [f32; 5] {
    let mut all = vec![("gain", crate::support::rgb([2.0, 2.0, 2.0]))];
    all.extend(rows.iter().cloned());
    let out = run(&[linear(&all)], &strip(&COLOURS), 5, 1);
    std::array::from_fn(|i| key(at(&out, i), COLOURS[i]))
}

fn assert_keys(got: [f32; 5], want: [f32; 5], what: &str) {
    for i in 0..5 {
        assert!(
            (got[i] - want[i]).abs() < 1e-4,
            "{what}: got {got:?}, want {want:?}"
        );
    }
}

/// The 3D key's sample at a colour: Cb and Cr doubled, and its luma.
fn sample(u: [f32; 3]) -> (EffectValue, EffectValue) {
    let y = luma(u);
    let (cb, cr) = ((u[2] - y) / 1.8556, (u[0] - y) / 1.5748);
    (
        EffectValue::Shape(vec![[2.0 * cb, 2.0 * cr]]),
        EffectValue::Shape(vec![[y, 0.0]]),
    )
}

/// HSL, RGB, Luma and 3D each key their own colours; Invert, the clips and
/// Clean black act on the key as written; and Denoise, Grow and Blur keep a
/// big patch keyed whole.
#[test]
fn each_qualifier_mode_keys_its_own_colours() {
    let hsl = [
        ("qual_mode", EffectValue::Choice(1)),
        ("qual_hue", f(0.0)),
        ("qual_hue_width", f(40.0)),
        ("qual_hue_soft", f(10.0)),
        ("qual_sat_low", f(20.0)),
        ("qual_sat_soft", f(5.0)),
    ];
    let dull = ss(0.15, 0.2, 0.18);
    assert_keys(keys(&hsl), [1.0, 0.0, 0.0, 0.0, dull], "hsl");
    let mut inverted = hsl.to_vec();
    inverted.push(("qual_invert", EffectValue::Bool(true)));
    assert_keys(
        keys(&inverted),
        [0.0, 1.0, 1.0, 1.0, 1.0 - dull],
        "hsl inverted",
    );
    let mut clipped = hsl.to_vec();
    clipped.extend([("qual_black_clip", f(30.0)), ("qual_white_clip", f(80.0))]);
    assert_keys(keys(&clipped), [0.8, 0.3, 0.3, 0.3, dull], "hsl clipped");
    let mut cleaned = hsl.to_vec();
    cleaned.push(("qual_clean_black", f(100.0)));
    let clean = ((dull - 0.49) / 0.51).clamp(0.0, 1.0);
    assert_keys(keys(&cleaned), [1.0, 0.0, 0.0, 0.0, clean], "hsl cleaned");

    let rgb_key = [
        ("qual_mode", EffectValue::Choice(2)),
        ("qual_red_low", f(50.0)),
        ("qual_green_high", f(50.0)),
        ("qual_rgb_soft", f(0.0)),
    ];
    assert_keys(keys(&rgb_key), [1.0, 0.0, 0.0, 0.0, 0.0], "rgb");

    let luma_key = [
        ("qual_mode", EffectValue::Choice(3)),
        ("qual_lum_low", f(50.0)),
        ("qual_lum_soft", f(0.0)),
    ];
    assert_keys(keys(&luma_key), [0.0, 1.0, 0.0, 0.0, 0.0], "luma");

    let (red_cbcr, red_y) = sample(RED);
    let (blue_cbcr, blue_y) = sample(BLUE);
    let near = [("qual_3d_size", f(4.0)), ("qual_3d_soft", f(4.0))];
    let mut kept = vec![
        ("qual_mode", EffectValue::Choice(4)),
        ("qual_3d_keep", red_cbcr),
        ("qual_3d_keep_luma", red_y),
    ];
    kept.extend(near.iter().cloned());
    assert_keys(keys(&kept), [1.0, 0.0, 0.0, 0.0, 0.0], "3d keep");
    let mut dropped = vec![
        ("qual_mode", EffectValue::Choice(4)),
        ("qual_3d_drop", blue_cbcr),
        ("qual_3d_drop_luma", blue_y),
    ];
    dropped.extend(near.iter().cloned());
    assert_keys(keys(&dropped), [1.0, 1.0, 0.0, 1.0, 1.0], "3d drop");

    // Red on the left, blue on the right: the finesse rows soften the edge
    // and leave the middle of each side as it was keyed.
    let (w, h) = (24u32, 8u32);
    let halves: Vec<[f32; 3]> = (0..w * h)
        .map(|i| if i % w < w / 2 { RED } else { BLUE })
        .collect();
    let img = strip(&halves);
    for grow in [1.5, -1.5] {
        let mut soft = hsl.to_vec();
        soft.extend([
            ("gain", crate::support::rgb([2.0, 2.0, 2.0])),
            ("qual_denoise", f(50.0)),
            ("qual_grow", f(grow)),
            ("qual_blur", f(1.0)),
        ]);
        let out = run(&[linear(&soft)], &img, w, h);
        assert!(key(at_xy(&out, w, 2, 4), RED) > 0.99, "grow {grow}");
        assert!(key(at_xy(&out, w, 21, 4), BLUE) < 0.01, "grow {grow}");
    }
}

/// Each window shape keys inside its outline and nothing outside, turned,
/// inverted, faded and subtracted as its rows say; and the GPU's uniform
/// carries the window the CPU drew.
#[test]
fn each_window_keys_inside_its_shape() {
    let (w, h) = (32u32, 32u32);
    let img = flat(w, h, [0.25; 3]);
    let draw = |rows: &[(&str, EffectValue)]| {
        let mut all = vec![
            ("gain", crate::support::rgb([2.0, 2.0, 2.0])),
            ("win1_x", f(16.0)),
            ("win1_y", f(16.0)),
            ("win1_soft", f(0.0)),
            ("win2_x", f(16.0)),
            ("win2_y", f(16.0)),
            ("win2_soft", f(0.0)),
        ];
        all.extend(rows.iter().cloned());
        let out = run(&[linear(&all)], &img, w, h);
        move |x: u32, y: u32| key(at_xy(&out, w, x, y), [0.25; 3])
    };
    let check = |rows: &[(&str, EffectValue)], probes: &[(u32, u32, f32)], what: &str| {
        let k = draw(rows);
        for &(x, y, want) in probes {
            let got = k(x, y);
            assert!(
                (got - want).abs() < 1e-4,
                "{what} at {x},{y}: got {got}, want {want}"
            );
        }
    };
    let ellipse = [
        ("win1_shape", EffectValue::Choice(1)),
        ("win1_w", f(16.0)),
        ("win1_h", f(8.0)),
    ];
    check(
        &ellipse,
        &[(16, 16, 1.0), (22, 16, 1.0), (16, 22, 0.0), (2, 2, 0.0)],
        "ellipse",
    );
    let mut inverted = ellipse.to_vec();
    inverted.push(("win1_invert", EffectValue::Bool(true)));
    check(&inverted, &[(16, 16, 0.0), (2, 2, 1.0)], "inverted ellipse");
    let mut faded = ellipse.to_vec();
    faded.push(("win1_opacity", f(50.0)));
    check(&faded, &[(16, 16, 0.5), (2, 2, 0.0)], "half opacity");
    let mut cut = ellipse.to_vec();
    cut.push(("win1_subtract", EffectValue::Bool(true)));
    check(&cut, &[(16, 16, 0.0), (2, 2, 1.0)], "a lone subtract");

    // A circle 8 px round with 4 px of outer softness: the edge fades as a
    // smoothstep of the distance past it.
    let soft = [
        ("win1_shape", EffectValue::Choice(1)),
        ("win1_w", f(16.0)),
        ("win1_h", f(16.0)),
        ("win1_soft", f(4.0)),
    ];
    let d = (10.5f32 * 10.5 + 0.25).sqrt() - 8.0;
    check(
        &soft,
        &[(16, 16, 1.0), (26, 16, 1.0 - ss(0.0, 4.0, d))],
        "soft",
    );

    // A square turned 45 degrees keys a diamond.
    let diamond = [
        ("win1_shape", EffectValue::Choice(2)),
        ("win1_w", f(10.0)),
        ("win1_h", f(10.0)),
        ("win1_angle", f(45.0)),
    ];
    check(
        &diamond,
        &[(16, 16, 1.0), (16, 21, 1.0), (20, 20, 0.0)],
        "turned rectangle",
    );
    let centred = [("win1_x", f(16.0)), ("win1_y", f(16.0))];
    let ops = resolve(&[linear(&[&diamond[..], &centred].concat())]);
    let pk = Correction::read(ops.get(0).unwrap().params).packed(1.0);
    let words = pk.words(false);
    assert_eq!(words[0][3], 1.0, "the key is on");
    assert_eq!(words[24], [2.0, 16.0, 16.0, 1.0]);
    assert_eq!(words[25][..2], [5.0, 5.0]);
    assert_eq!(words[44..58], pk.hdr.words());

    // A triangle, straight and smooth: the smooth outline bulges past the
    // straight one's base.
    let triangle = EffectValue::Shape(vec![[-1.0, -1.0], [1.0, -1.0], [0.0, 1.0]]);
    let polygon = [
        ("win1_shape", EffectValue::Choice(3)),
        ("win1_w", f(20.0)),
        ("win1_h", f(20.0)),
        ("win1_points", triangle.clone()),
    ];
    let mut straight = polygon.to_vec();
    straight.push(("win1_smooth", EffectValue::Bool(false)));
    check(
        &straight,
        &[(16, 16, 1.0), (16, 8, 1.0), (8, 24, 0.0), (16, 4, 0.0)],
        "straight polygon",
    );
    check(
        &polygon,
        &[(16, 16, 1.0), (16, 4, 1.0), (2, 30, 0.0)],
        "smooth polygon",
    );
    let two = [
        ("win1_shape", EffectValue::Choice(3)),
        (
            "win1_points",
            EffectValue::Shape(vec![[0.0, 0.0], [1.0, 1.0]]),
        ),
    ];
    check(&two, &[(2, 2, 1.0)], "a polygon of two points is off");

    let gradient = [("win1_shape", EffectValue::Choice(4))];
    check(&gradient, &[(16, 4, 1.0), (16, 28, 0.0)], "gradient");

    // A big circle less a small square.
    let ring = [
        ("win1_shape", EffectValue::Choice(1)),
        ("win1_w", f(30.0)),
        ("win1_h", f(30.0)),
        ("win2_shape", EffectValue::Choice(2)),
        ("win2_w", f(6.0)),
        ("win2_h", f(6.0)),
        ("win2_subtract", EffectValue::Bool(true)),
    ];
    check(
        &ring,
        &[(16, 16, 0.0), (16, 24, 1.0), (1, 1, 0.0)],
        "subtracted window",
    );
}
