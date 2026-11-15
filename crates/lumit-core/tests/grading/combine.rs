//! Parallel and Layer runs through the CPU stack, against the sums in
//! `correction::combine`: Parallel adds what each node changed to the run,
//! Layer lays its whole grade over the run through its key and Mix.

use lumit_core::fx::effects::correction::combine::Run;
use lumit_core::model::{EffectInstance, EffectValue};

use crate::support::{assert_near, at_xy, f, linear, resolve, rgb, run, strip};

fn gain(g: f64) -> (&'static str, EffectValue) {
    ("gain", rgb([g, g, g]))
}

fn joined(how: u32, rows: &[(&str, EffectValue)]) -> EffectInstance {
    let mut all = vec![("combine", EffectValue::Choice(how))];
    all.extend(rows.iter().cloned());
    linear(&all)
}

/// A small window round the middle of a 16 px picture.
fn window() -> Vec<(&'static str, EffectValue)> {
    vec![
        ("win1_shape", EffectValue::Choice(1)),
        ("win1_x", f(8.0)),
        ("win1_y", f(8.0)),
        ("win1_w", f(6.0)),
        ("win1_h", f(6.0)),
        ("win1_soft", f(0.0)),
    ]
}

/// The stack draws a run as its formulas say, and a node after the run
/// grades the run's picture.
#[test]
fn parallel_and_layer_runs_match_their_formulas() {
    let (w, h) = (16u32, 16u32);
    let colours: Vec<[f32; 3]> = (0..w * h)
        .map(|i| {
            let t = i as f32 / (w * h) as f32;
            [0.1 + 0.5 * t, 0.3, 0.6 - 0.4 * t]
        })
        .collect();
    let img = strip(&colours);
    let u = |x: u32, y: u32| colours[(y * w + x) as usize];

    // Two Parallel nodes each read the run's input, so their offsets add.
    let stack = [
        linear(&[gain(2.0)]),
        joined(1, &[("offset", rgb([0.1, 0.1, 0.1]))]),
        joined(1, &[("offset", rgb([0.05, 0.05, 0.05]))]),
        linear(&[gain(0.5)]),
    ];
    let out = run(&stack, &img, w, h);
    for (x, y) in [(0, 0), (8, 8), (15, 15)] {
        let want = u(x, y).map(|c| (2.0 * c + 0.15) * 0.5);
        assert_near(at_xy(&out, w, x, y), want, 1e-5, "parallel");
    }

    // Layer at Mix 50: halfway from the run to the node's whole grade,
    // inside the node's window only.
    let mut rows = vec![gain(0.5), ("mix", f(50.0))];
    rows.extend(window());
    let stack = [linear(&[gain(2.0)]), joined(2, &rows)];
    let out = run(&stack, &img, w, h);
    let inside = u(8, 8).map(|c| 0.5 * 2.0 * c + 0.5 * 0.5 * c);
    assert_near(at_xy(&out, w, 8, 8), inside, 1e-5, "layer inside");
    let outside = u(1, 1).map(|c| 2.0 * c);
    assert_near(at_xy(&out, w, 1, 1), outside, 1e-5, "layer outside");

    // A LUT walks the same run with only each node's colour part, so the
    // window keys every pixel.
    let ops = resolve(&stack);
    let mut lut = img.clone();
    let mut walk = Run::pointwise();
    for (i, op) in ops.iter().enumerate() {
        walk.begin(&ops, i, &mut lut);
        if !walk.layers() {
            op.def.apply_cpu_pointwise(&mut lut, w, h, op.params);
        }
        walk.end(&mut lut, w, h, &op);
    }
    let whole = u(1, 1).map(|c| 1.25 * c);
    assert_near(at_xy(&lut, w, 1, 1), whole, 1e-5, "layer in a LUT");

    // And a windowed node alone, drawn for a LUT, grades everywhere.
    let ops = resolve(&[linear(&[&[gain(2.0)][..], &window()].concat())]);
    let op = ops.get(0).unwrap();
    let mut alone = img.clone();
    op.def.apply_cpu_pointwise(&mut alone, w, h, op.params);
    assert_near(at_xy(&alone, w, 1, 1), outside, 1e-5, "pointwise window");
}
