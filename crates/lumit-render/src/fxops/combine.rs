//! Parallel and Layer runs, and key wires, on the card: the twin of
//! `lumit_core::fx::effects::correction::combine::Run`, which says what each
//! one means.

use lumit_core::fx::effects::correction::combine::{self, Combine};
use lumit_core::fx::effects::correction::packed::Packed;
use lumit_core::fx::effects::correction::{Correction, DERIVED_PX_SCALE};
use lumit_core::fx::params::{Params, ResolvedFx, ResolvedStack};
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::correction::CorrectionOp;
use lumit_gpu::fx::{fit_centred, FxEngine};
use lumit_gpu::GpuContext;

use super::{LayerInput, Tex};

/// The op a packed Correction draws with, over its own uniform.
fn op<'a>(pk: &'a Packed, words: &'a [[f32; 4]]) -> CorrectionOp<'a> {
    CorrectionOp {
        words,
        data: &pk.data,
        neutral: pk.neutral,
        qualifier_on: pk.qualifier_on(),
        windows_on: pk.windows_on(),
        denoise_px: pk.qual.denoise_px,
        blur_px: pk.qual.blur_px,
        grow_px: pk.qual.grow_px,
        detail_px: pk.detail_px,
        detail_on: pk.detail != 0.0,
    }
}

fn packed(p: Params<'_>) -> Packed {
    Correction::read(p).packed(p.float(DERIVED_PX_SCALE, 1.0))
}

/// How far into `ops` a held picture may reach: a key is made while its node
/// draws, so the walk starts at or before the first node a wire reads a key
/// from.
pub(super) fn key_limit(ops: &ResolvedStack, mattes: &[LayerInput], limit: usize) -> usize {
    let wired = |id: uuid::Uuid| {
        mattes
            .iter()
            .any(|m| matches!(m, LayerInput::EffectKey(from) if *from == id))
    };
    ops.iter()
        .position(|op| wired(op.instance))
        .map_or(limit, |i| limit.min(i))
}

/// Where a run stands as `run_ops` walks a stack.
#[derive(Default)]
pub(super) struct Run {
    /// The picture the run's first node read, while a node below joins it.
    input: Option<Tex>,
    /// What the run had made before the joining node now drawing.
    acc: Option<Tex>,
    combine: Combine,
    /// Keys a wire further down reads, by the node that made them.
    keys: Vec<(uuid::Uuid, Tex)>,
}

impl Run {
    /// Before op `i` draws. Answers the picture it draws from: the run's
    /// input for a joining node, `tex` for any other. The run's input is taken
    /// out of `spent`, as the nodes that join still read it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin(
        &mut self,
        ctx: &GpuContext,
        ops: &ResolvedStack,
        i: usize,
        tex: Tex,
        w: u32,
        h: u32,
        spent: &mut Option<Tex>,
    ) -> Tex {
        let combine = ops
            .get(i)
            .map_or(Combine::Serial, |op| combine::combine_of(&op));
        if let (Some(input), true) = (&self.input, combine != Combine::Serial) {
            self.acc = Some(tex);
            self.combine = combine;
            return fit_centred(ctx, input.clone(), w, h);
        }
        self.combine = Combine::Serial;
        self.acc = None;
        self.input = combine::joins(ops, i + 1).then(|| tex.clone());
        if self.input.is_some() && *spent == self.input {
            *spent = None;
        }
        tex
    }

    /// Whether the node now drawing is a Layer node, whose own kernel does
    /// not run: [`Self::end`] draws it.
    pub(super) fn layers(&self) -> bool {
        self.combine == Combine::Layer
    }

    /// Whether `t` is the run's input, which nothing may hand back yet.
    pub(super) fn holds(&self, t: &Tex) -> bool {
        self.input.as_ref() == Some(t)
    }

    /// After the node drew `out` from the run's input: the run's picture.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn end(
        &mut self,
        fx: &FxEngine,
        ctx: &GpuContext,
        out: Tex,
        w: u32,
        h: u32,
        node: &ResolvedFx<'_>,
        matte: Option<&Tex>,
    ) -> Tex {
        let (Some(acc), Some(input)) = (self.acc.take(), self.input.as_ref()) else {
            return out;
        };
        let input = fit_centred(ctx, input.clone(), w, h);
        let p = node.params;
        match self.combine {
            Combine::Serial => out,
            Combine::Parallel => {
                let pk = packed(p);
                let words = pk.words(false);
                let sum = fx.correction_parallel(ctx, &acc, &out, &input, w, h, &op(&pk, &words));
                if out != input && out != acc {
                    ctx.recycle(out);
                }
                sum
            }
            Combine::Layer => {
                let pk = packed(p);
                let words = pk.words(matte.is_some());
                let key = fx.correction_key(ctx, &input, w, h, matte, &op(&pk, &words));
                let full = combine::graded_packed(p);
                let words = full.words(false);
                let graded = fx.correction(ctx, &input, w, h, None, &op(&full, &words));
                let (mode, mix) = combine::layer_mix(node.def.schema(), p);
                let over = if mode != 0 || mix < 1.0 {
                    let over = fx.blend_mix(ctx, &acc, &graded, w, h, mode, mix);
                    if graded != input {
                        ctx.recycle(graded);
                    }
                    over
                } else {
                    graded
                };
                let Some(key) = key else {
                    return over;
                };
                let laid = fx.matte_mix(ctx, &acc, &over, &key, w, h, false);
                ctx.recycle(key);
                if over != input {
                    ctx.recycle(over);
                }
                laid
            }
        }
    }

    /// Keep the key of `node`, drawn from `given` with its prepared `matte`,
    /// when a wire in `mattes` reads it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn keep_key(
        &mut self,
        fx: &FxEngine,
        ctx: &GpuContext,
        mattes: &[LayerInput],
        node: &ResolvedFx<'_>,
        given: &Tex,
        matte: Option<&Tex>,
        w: u32,
        h: u32,
    ) {
        let wired = mattes
            .iter()
            .any(|m| matches!(m, LayerInput::EffectKey(id) if *id == node.instance));
        if !wired || node.def.schema().match_name != "correction" {
            return;
        }
        let pk = packed(node.params);
        let words = pk.words(matte.is_some());
        let given = fit_centred(ctx, given.clone(), w, h);
        let key = fx.correction_key_picture(ctx, &given, w, h, matte, &op(&pk, &words));
        self.keys.push((node.instance, key));
    }

    /// The key `from` made, for the matte a wire feeds.
    pub(super) fn key_of(&self, from: &uuid::Uuid) -> Option<&Tex> {
        self.keys.iter().find(|(id, _)| id == from).map(|(_, t)| t)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use lumit_core::anim::Property;
    use lumit_core::fx::params::ResolvedStack;
    use lumit_core::model::{EffectInstance, EffectValue};

    use super::super::{run_ops, FxCache, LayerInput, Side};

    const W: u32 = 8;
    const H: u32 = 8;

    fn float(v: f64) -> EffectValue {
        EffectValue::Float(Property::fixed(v))
    }

    fn colour(v: [f64; 3]) -> EffectValue {
        let [r, g, b] = v.map(Property::fixed);
        EffectValue::Colour([r, g, b, Property::fixed(1.0)])
    }

    /// An effect at this raster with `rows` set.
    fn effect(name: &str, rows: &[(&str, EffectValue)]) -> EffectInstance {
        let mut e =
            lumit_core::fx::builtins::instantiate_for_raster(name, f64::from(W), f64::from(H))
                .expect("a built-in");
        for (id, value) in rows {
            let p = e.params.iter_mut().find(|p| p.id == *id).expect("a row");
            p.value = value.clone();
        }
        e
    }

    fn resolve(effects: &[EffectInstance]) -> ResolvedStack {
        lumit_core::fx::resolved::resolve_stack(
            effects,
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::markers::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        )
    }

    fn picture() -> Vec<f32> {
        (0..(W * H * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect()
    }

    /// A small hard ellipse in the middle of the picture.
    fn window() -> Vec<(&'static str, EffectValue)> {
        vec![
            ("win1_shape", EffectValue::Choice(1)),
            ("win1_x", float(4.0)),
            ("win1_y", float(4.0)),
            ("win1_w", float(4.0)),
            ("win1_h", float(4.0)),
            ("win1_soft", float(0.0)),
            ("win1_soft_in", float(0.0)),
        ]
    }

    fn cpu(ops: &ResolvedStack) -> Vec<f32> {
        let mut px = picture();
        lumit_core::fx::cpu::apply_stack(&mut px, W, H, ops);
        px
    }

    /// Serial, then a Parallel node, then a Layer node keyed by a window and
    /// laid on through Add at Mix 70: the card and the CPU draw the same
    /// picture, and Combine changes it.
    #[test]
    fn parallel_and_layer_runs_draw_the_same_on_both_paths() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let stack = |parallel: u32, layer: u32| {
            let mut c = window();
            c.extend([
                ("combine", EffectValue::Choice(layer)),
                ("saturation", float(30.0)),
                ("blend", EffectValue::Choice(6)),
                ("mix", float(70.0)),
            ]);
            vec![
                effect("correction", &[("gain", colour([1.4, 1.1, 0.9]))]),
                effect(
                    "correction",
                    &[
                        ("combine", EffectValue::Choice(parallel)),
                        ("lift", colour([0.1, 0.0, 0.05])),
                    ],
                ),
                effect("correction", &c),
            ]
        };
        let ops = resolve(&stack(1, 2));
        let tex = lumit_gpu::fx::common::upload_linear_f32(&ctx, &picture(), W, H);
        let out = run_ops(
            ctx.fx(),
            &ctx,
            tex,
            W,
            H,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &Side::NONE,
            &[],
            None,
            None,
        );
        let gpu = lumit_gpu::fx::common::readback_linear_f32(&ctx, &out, W, H).expect("readback");
        let cpu_px = cpu(&ops);
        for (i, (g, c)) in gpu.iter().zip(&cpu_px).enumerate() {
            assert!((g - c).abs() < 1e-2, "pixel {i}: GPU {g} vs CPU {c}");
        }
        let serial = cpu(&resolve(&stack(0, 0)));
        assert!(
            cpu_px
                .iter()
                .zip(&serial)
                .any(|(a, b)| (a - b).abs() > 1e-2),
            "Combine changed nothing"
        );
        // Parallel adds what each node changed to the same input.
        let input = picture();
        let a = cpu(&resolve(&stack(1, 2)[..1]));
        let b = cpu(&resolve(&stack(1, 2)[1..2]));
        let both = cpu(&resolve(&stack(1, 2)[..2]));
        for i in 0..both.len() {
            let want = a[i] + b[i] - input[i];
            assert!(
                (both[i] - want).abs() < 1e-5,
                "pixel {i}: {} vs {want}",
                both[i]
            );
        }
    }

    /// A Correction's key wired into Exposure's Matte: Exposure lands inside
    /// the window and nowhere else, walked fresh and from a warm cache.
    #[test]
    fn a_key_wire_gates_a_later_effect() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let keyed = effect("correction", &window());
        let id = keyed.id;
        let ops = resolve(&[keyed, effect("exposure", &[("stops", float(2.0))])]);
        let mattes = [LayerInput::Absent, LayerInput::EffectKey(id)];
        let mut cache = FxCache::default();
        cache.keep_outputs(true);
        let cache = std::cell::RefCell::new(cache);
        for _ in 0..2 {
            let tex = lumit_gpu::fx::common::upload_linear_f32(&ctx, &picture(), W, H);
            let out = run_ops(
                ctx.fx(),
                &ctx,
                tex,
                W,
                H,
                &ops,
                &[],
                &[],
                &[],
                &[],
                &[],
                &mattes,
                &[],
                &[],
                &Side::NONE,
                &[],
                None,
                Some((&cache, 7)),
            );
            let px =
                lumit_gpu::fx::common::readback_linear_f32(&ctx, &out, W, H).expect("readback");
            let input = picture();
            let at = |x: u32, y: u32| ((y * W + x) * 4) as usize;
            let (corner, middle) = (at(1, 0), at(4, 4));
            assert!((px[corner] - input[corner]).abs() < 1e-3, "outside the key");
            assert!(
                (px[middle] - input[middle] * 4.0).abs() < 1e-2,
                "inside the key: {} vs {}",
                px[middle],
                input[middle] * 4.0
            );
        }
    }
}
