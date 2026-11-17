//! The Viewer's matte view: the patched document draws the shot alone, its
//! stack cut after the chosen node, and that node shows its key.

use lumit_core::grade::matte_view::{apply, MatteMode, MatteView, MATTE_VIEW_ROW};
use lumit_core::grade::Shot;
use lumit_core::model::{Document, EffectInstance, EffectValue};
use uuid::Uuid;

use crate::support::{assert_near, at_xy, comp_of, f, flat, fx, linear, rgb, run, stack_of};

/// A window round the middle of a 16 px picture.
fn windowed(mut rows: Vec<(&'static str, EffectValue)>) -> EffectInstance {
    rows.extend([
        ("win1_shape", EffectValue::Choice(1)),
        ("win1_x", f(8.0)),
        ("win1_y", f(8.0)),
        ("win1_w", f(6.0)),
        ("win1_h", f(6.0)),
        ("win1_soft", f(0.0)),
    ]);
    linear(&rows)
}

/// The patched stack of `view`, drawn over flat grey: the pixel in the
/// window and one outside it.
fn drawn(doc: &Document, view: MatteView) -> ([f32; 3], [f32; 3]) {
    let patched = apply(doc, view).unwrap();
    let out = run(
        &stack_of(&patched, view.shot),
        &flat(16, 16, [0.25; 3]),
        16,
        16,
    );
    (at_xy(&out, 16, 8, 8), at_xy(&out, 16, 1, 1))
}

#[test]
fn the_matte_view_draws_the_shot_alone_and_cuts_after_the_node() {
    let a = windowed(vec![("gain", rgb([2.0, 2.0, 2.0]))]);
    let b = windowed(vec![
        ("combine", EffectValue::Choice(1)),
        ("offset", rgb([0.1, 0.1, 0.1])),
    ]);
    let after = linear(&[("gain", rgb([3.0, 3.0, 3.0]))]);
    let exposure = fx("exposure", &[]);
    let grade = vec![a.clone(), b.clone(), after, exposure.clone()];
    let (doc, shots) = comp_of(2, || grade.clone());
    let shot = shots[0];
    let view = |effect: Uuid, mode: MatteMode| MatteView { shot, effect, mode };

    // Key on the first node: only the shot shows, the stack stops at the
    // node, and the node draws its window in black and white.
    let patched = apply(&doc, view(a.id, MatteMode::Key)).unwrap();
    let layers = &patched.comp(shot.comp).unwrap().layers;
    assert!(layers[0].switches.visible && !layers[1].switches.visible);
    let stack = stack_of(&patched, shot);
    assert_eq!(stack.len(), 1);
    assert_eq!(
        stack[0].param(MATTE_VIEW_ROW),
        Some(&EffectValue::Choice(MatteMode::Key as u32))
    );
    assert_eq!(
        drawn(&doc, view(a.id, MatteMode::Key)),
        ([1.0; 3], [0.0; 3])
    );

    // Highlight: the graded picture in the window, grey elsewhere.
    let (inside, outside) = drawn(&doc, view(a.id, MatteMode::Highlight));
    assert_near(inside, [0.5; 3], 1e-5, "highlight inside");
    assert_near(outside, [0.18; 3], 1e-6, "highlight outside");
    // Over black, white, or the node's picture in black and white.
    for (mode, back) in [
        (MatteMode::HighlightBlack, 0.0),
        (MatteMode::HighlightWhite, 1.0),
        (MatteMode::HighlightMono, 0.25),
    ] {
        let (inside, outside) = drawn(&doc, view(a.id, mode));
        assert_near(inside, [0.5; 3], 1e-5, "backdrop inside");
        assert_near(outside, [back; 3], 1e-5, "backdrop outside");
    }

    // A Parallel node keeps its run: the view goes over the run's picture,
    // keyed by the node's own window.
    let (inside, outside) = drawn(&doc, view(b.id, MatteMode::Highlight));
    assert_near(inside, [0.6; 3], 1e-5, "run inside");
    assert_near(outside, [0.18; 3], 1e-6, "run outside");

    // With nothing on above it the node is Serial, and says so.
    let mut lone = doc.clone();
    let layer = &mut lone.comp_mut(shot.comp).unwrap().layers[0];
    layer.effects[0].enabled = false;
    let patched = apply(&lone, view(b.id, MatteMode::Key)).unwrap();
    let node = stack_of(&patched, shot)[1].clone();
    assert_eq!(node.param("combine"), Some(&EffectValue::Choice(0)));
    assert_eq!(
        drawn(&lone, view(b.id, MatteMode::Key)),
        ([1.0; 3], [0.0; 3])
    );

    // Nothing to show: a node that isn't a Correction, or a shot that's gone.
    assert!(apply(&doc, view(exposure.id, MatteMode::Key)).is_none());
    let gone = MatteView {
        shot: Shot {
            layer: Uuid::now_v7(),
            ..shot
        },
        effect: a.id,
        mode: MatteMode::Key,
    };
    assert!(apply(&doc, gone).is_none());
}
