//! Picks on the picture: Set, Add and Subtract on the qualifier in each of
//! its modes key what was picked, and a hue curve pick lands on the samples.

use lumit_core::fx::effects::correction::{maths, Correction};
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::grade_solve::pick::{
    circle_samples, curve_axis, curve_pick, qualifier_pick, CurveAxis, PickMode,
};
use lumit_core::grade_solve::{RowValue, Rows, SolveError};
use lumit_core::model::{EffectInstance, EffectValue};

use crate::support::{f, fx, resolve, rgb};

const W: u32 = 32;
const H: u32 = 16;

/// Orange on the left, teal on the right, both brighter down the picture.
fn picture() -> Vec<f32> {
    let mut img = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let level = 0.2 + 0.7 * (y as f32 + 0.5) / H as f32;
            let base = if x < W / 2 {
                [0.9, 0.45, 0.12]
            } else {
                [0.1, 0.6, 0.5]
            };
            img.extend_from_slice(&[base[0] * level, base[1] * level, base[2] * level, 1.0]);
        }
    }
    img
}

/// `node` with a solver's rows written over it, by id.
fn written(mut node: EffectInstance, rows: &Rows) -> EffectInstance {
    for (id, value) in rows {
        let value = match value {
            RowValue::Float(v) => f(f64::from(*v)),
            RowValue::Colour(c) => rgb(c.map(f64::from)),
            RowValue::Choice(c) => EffectValue::Choice(*c),
            RowValue::Bool(b) => EffectValue::Bool(*b),
            RowValue::Shape(s) => EffectValue::Shape(s.points().to_vec()),
        };
        let row = node.params.iter_mut().find(|p| p.id == *id).unwrap();
        row.value = value;
    }
    node
}

fn read(node: &EffectInstance) -> Correction {
    let ops = resolve(std::slice::from_ref(node));
    Correction::read(ops.get(0).unwrap().params)
}

/// A pick under a circle at `(x, y)`.
fn pick(
    img: &[f32],
    node: &EffectInstance,
    mode: PickMode,
    at: [f32; 2],
    r: f32,
) -> EffectInstance {
    let picked = circle_samples(img, W, H, at, r);
    let rows = qualifier_pick(&picked, mode, &read(node)).unwrap();
    written(node.clone(), &rows)
}

/// The node's qualifier key at pixel `(x, y)`.
fn key(img: &[f32], node: &EffectInstance, x: u32, y: u32) -> f32 {
    let q = read(node).packed(1.0).qual;
    let k = ((y * W + x) * 4) as usize;
    maths::qualify(maths::v_in([img[k], img[k + 1], img[k + 2], 1.0], 0), &q)
}

#[test]
fn picks_key_what_was_picked_in_each_mode() {
    let img = picture();

    // RGB: Set on the orange, then Add the teal.
    let rgb_node = fx("correction", &[("qual_mode", EffectValue::Choice(2))]);
    let set = pick(&img, &rgb_node, PickMode::Set, [8.0, 8.0], 4.0);
    assert!(key(&img, &set, 8, 8) > 0.99 && key(&img, &set, 24, 8) < 0.01);
    let both = pick(&img, &set, PickMode::Add, [24.0, 8.0], 4.0);
    assert!(key(&img, &both, 8, 8) > 0.99 && key(&img, &both, 24, 8) > 0.99);

    // Luma: Set on the dark rows, Add the bright ones, then Subtract them.
    let luma_node = fx("correction", &[("qual_mode", EffectValue::Choice(3))]);
    let dark = pick(&img, &luma_node, PickMode::Set, [8.0, 3.0], 2.0);
    assert!(key(&img, &dark, 8, 3) > 0.99 && key(&img, &dark, 8, 14) < 0.01);
    let wide = pick(&img, &dark, PickMode::Add, [8.0, 14.0], 1.0);
    assert!(key(&img, &wide, 8, 14) > 0.99 && key(&img, &wide, 8, 3) > 0.99);
    let cut = pick(&img, &wide, PickMode::Subtract, [8.0, 14.0], 1.0);
    assert!(key(&img, &cut, 8, 14) < 0.01 && key(&img, &cut, 8, 3) > 0.99);

    // HSL from Off: Set on the orange turns the qualifier on, Add takes in
    // the teal's hue too.
    let off = fx("correction", &[]);
    let orange = pick(&img, &off, PickMode::Set, [8.0, 8.0], 4.0);
    assert_eq!(read(&orange).qual_mode, 1);
    assert!(key(&img, &orange, 8, 8) > 0.99 && key(&img, &orange, 24, 8) < 0.01);
    let both = pick(&img, &orange, PickMode::Add, [24.0, 8.0], 4.0);
    assert!(key(&img, &both, 8, 8) > 0.99 && key(&img, &both, 24, 8) > 0.99);

    // Nothing to cut into, and nothing picked.
    let picked = circle_samples(&img, W, H, [8.0, 8.0], 2.0);
    let rows = qualifier_pick(&picked, PickMode::Subtract, &read(&off)).unwrap();
    assert!(rows.is_empty());
    assert_eq!(
        qualifier_pick(&[], PickMode::Set, &read(&off)),
        Err(SolveError::Empty)
    );
    // A circle off the picture still reads the nearest pixel; no picture
    // reads nothing.
    assert_eq!(circle_samples(&img, W, H, [-9.0, 40.0], f32::NAN).len(), 1);
    assert!(circle_samples(&[], 0, 0, [1.0, 1.0], 1.0).is_empty());
}

#[test]
fn a_curve_pick_lands_on_the_samples() {
    let img = picture();
    let node = read(&fx("correction", &[]));
    let picked = circle_samples(&img, W, H, [8.0, 8.0], 3.0);
    let encoded: Vec<[f32; 3]> = picked
        .iter()
        .map(|u| u.map(|c| c.powf(1.0 / 2.4)))
        .collect();

    let hue = curve_pick(&picked, CurveAxis::Hue, &node).unwrap();
    let c = maths::ycc(encoded[0]);
    let want = maths::hue_of(c[1], c[2]);
    assert!((hue.centre - want).abs() < 0.01, "{hue:?} {want}");

    for (axis, read) in [
        (CurveAxis::Sat, maths::chroma as fn([f32; 3]) -> f32),
        (CurveAxis::Lum, maths::luma),
    ] {
        let got = curve_pick(&picked, axis, &node).unwrap();
        let values: Vec<f32> = encoded.iter().map(|v| read(*v)).collect();
        let lo = values.iter().copied().fold(f32::MAX, f32::min);
        let hi = values.iter().copied().fold(f32::MIN, f32::max);
        assert!(got.centre >= lo && got.centre <= hi, "{axis:?}: {got:?}");
        assert!(got.spread > 0.0 && got.spread <= 0.5 * (hi - lo) + 0.01);
    }

    let greys = vec![[0.3f32; 3]; 4];
    assert_eq!(
        curve_pick(&greys, CurveAxis::Hue, &node),
        Err(SolveError::Empty)
    );
    assert_eq!(curve_axis("hue_sat"), Some(CurveAxis::Hue));
    assert_eq!(curve_axis("lum_sat"), Some(CurveAxis::Lum));
    assert_eq!(curve_axis("sat_lum"), Some(CurveAxis::Sat));
    assert_eq!(curve_axis("gain"), None);
}
