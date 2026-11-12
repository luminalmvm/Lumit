//! The colour warper: a mesh at rest changes nothing, and a moved point moves
//! the colours under it and leaves the far side of the wheel alone.

use lumit_core::fx::effects::colour_warper::ColourWarper;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::EffectValue;

use crate::support::{assert_near, at, f, fx, resolve, run, strip};

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const CYAN: [f32; 3] = [0.0, 1.0, 1.0];
const GREY: [f32; 3] = [0.4, 0.4, 0.4];

/// Y, Cb and Cr of a scene-linear colour, on video gamma as the mesh reads it.
fn ycc(u: [f32; 3]) -> [f32; 3] {
    let v = u.map(|c| c.signum() * c.abs().powf(1.0 / 2.4));
    let y = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
    [y, (v[2] - y) / 1.8556, (v[0] - y) / 1.5748]
}

/// A mesh with `hues` spokes whose outer point on the red spoke moved by
/// `by`, in the mesh's own units.
fn moved(
    hues: u32,
    by: [f32; 2],
    rows: &[(&str, EffectValue)],
) -> lumit_core::model::EffectInstance {
    let n = hues as usize;
    let mut points = vec![[0.0, 0.0]; 1 + 3 * n];
    points[1 + 2 * n] = by;
    let mut all = vec![
        ("resolution", EffectValue::Choice(u32::from(hues == 8))),
        ("points", EffectValue::Shape(points)),
    ];
    all.extend(rows.iter().cloned());
    fx("colour_warper", &all)
}

#[test]
fn the_colour_warper_moves_only_the_hue_under_a_moved_point() {
    let img = strip(&[RED, CYAN, GREY]);
    assert_eq!(run(&[fx("colour_warper", &[])], &img, 3, 1), img, "at rest");

    let red = ycc(RED);
    for hues in [6, 8] {
        // Red's outer point pulled in by 0.2: Cr drops by 0.1, luma stays,
        // and cyan, opposite, and grey, at the centre, don't move.
        let out = run(&[moved(hues, [0.0, -0.2], &[])], &img, 3, 1);
        let got = ycc(at(&out, 0));
        assert_near(got, [red[0], red[1], red[2] - 0.1], 1e-4, "red moved");
        assert_near(at(&out, 1), CYAN, 1e-4, "cyan stays");
        assert_near(at(&out, 2), GREY, 1e-4, "grey stays");
    }

    // Half the strength, half the move.
    let out = run(
        &[moved(6, [0.0, -0.2], &[("strength", f(50.0))])],
        &img,
        3,
        1,
    );
    let got = ycc(at(&out, 0));
    assert_near(got, [red[0], red[1], red[2] - 0.05], 1e-4, "half strength");

    // Without Keep luma the brightest channel stays put instead.
    let out = run(
        &[moved(
            6,
            [0.0, -0.2],
            &[("keep_luma", EffectValue::Bool(false))],
        )],
        &img,
        3,
        1,
    );
    let r = at(&out, 0);
    assert!((r[0].max(r[1]).max(r[2]) - 1.0).abs() < 1e-4, "{r:?}");

    // The GPU's uniform: the header, then two moves a word.
    let ops = resolve(&[moved(6, [0.0, -0.2], &[])]);
    let pk = ColourWarper::read(ops.get(0).unwrap().params).packed();
    let words = pk.words();
    assert_eq!(words[0], [6.0, 1.0, 1.0, 0.0]);
    assert_eq!(words[1 + 13 / 2][2..], [0.0, -0.2]);
}
