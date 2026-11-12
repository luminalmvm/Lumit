//! Tests for which matte slot each effect binds.

/// **The k-th matte-carrying op binds the k-th matte slot** (the
/// parallel-list contract with its second predicate).
///
/// The mirror of `the_kth_lut_op_binds_the_kth_slot`, and it exists for the
/// same reason: nothing but the counting joins `build.rs`'s enumeration to
/// `run_ops`'s walk, so a slot skipped or double-counted drives the wrong
/// effect — a project where reordering the stack moves which effect a matte
/// controls.
///
/// The first slot is deliberately **absent** (the unset row every list
/// allows), so an implementation that advances its counter only when a matte
/// is really there hands the second op the first op's slot and dissolves
/// nothing. The second slot is black, which switches its op off entirely —
/// so the picture must be "the first effect applied, the second not".
#[test]
fn the_kth_matte_op_binds_the_kth_slot() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (4u32, 4u32);
    // Opaque mid grey, so both an exposure up and an exposure down are
    // visible and neither clips.
    let source: Vec<f32> = (0..(w * h))
        .flat_map(|_| [0.25f32, 0.25, 0.25, 1.0])
        .collect();

    let exposure = |stops: f64| {
        let mut inst =
            lumit_core::fx::builtins::instantiate("exposure").expect("exposure is a built-in");
        for p in &mut inst.params {
            if p.id == "stops" {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(stops));
            }
        }
        inst
    };
    let ops = lumit_core::fx::resolved::resolve_stack(
        &[exposure(2.0), exposure(-2.0)],
        0.0,
        1000.0,
        1.0,
        &lumit_core::fx::markers::MarkerContext::NONE,
        std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
    );
    assert_eq!(ops.len(), 2, "two ops, two matte slots to bind");

    let black: Vec<f32> = (0..(w * h)).flat_map(|_| [0.0f32, 0.0, 0.0, 1.0]).collect();
    let off = crate::fxops::LayerInput::Texture(lumit_gpu::fx::common::upload_linear_f32(
        &ctx, &black, w, h,
    ));
    let tex = lumit_gpu::fx::common::upload_linear_f32(&ctx, &source, w, h);
    let out = crate::fxops::run_ops(
        fx,
        &ctx,
        tex,
        w,
        h,
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        // Slot 0 absent (the first exposure applies in full), slot 1 black
        // (the second is switched off).
        &[crate::fxops::LayerInput::Absent, off],
        &[],
        &[],
        &crate::fxops::Side::NONE,
        &[],
        None,
        None,
    );
    let got = lumit_gpu::fx::common::readback_linear_f32(&ctx, &out, w, h).expect("readback");

    assert!(
        (got[0] - 1.0).abs() < 1e-2,
        "red is {} — expected 0.25 lifted two stops and NOT pulled back \
             down, i.e. the second op bound the second slot",
        got[0]
    );
}
