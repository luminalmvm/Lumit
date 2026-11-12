//! How a Correction meets the node above it: the stack's half of Combine.
//!
//! **In plain terms.** A Serial node grades what the node above made. A
//! Parallel or Layer node reads the same picture the node above read, the
//! run's input, and its grade is joined to what the run has made so far:
//! - Parallel adds what the node changed: `acc += node(input) - input`, the
//!   node's own key and Mix already inside `node`.
//! - Layer lays the node's whole grade over the run through its Blend, then
//!   its key times its Mix: `acc = lerp(acc, blend(acc, graded), key * mix)`.
//!
//! Only a Correction joins. Any other effect is Serial and starts a new run,
//! and so does a Correction with nothing above it.

use super::{cpu, packed::Packed, Correction, DERIVED_PX_SCALE};
use crate::fx::params::{ParamId, Params, ResolvedFx, ResolvedStack};
use crate::fx::registry::EffectMetadata;
use crate::fx::schema::{EffectSchema, MIX_ID};

/// How a node meets the run above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Combine {
    #[default]
    Serial,
    Parallel,
    Layer,
}

const COMBINE: ParamId = ParamId::new("combine");

/// How `op` meets the node above it. Serial for anything but a Correction.
#[must_use]
pub fn combine_of(op: &ResolvedFx<'_>) -> Combine {
    if op.def.schema().match_name != <Correction as EffectMetadata>::SCHEMA.match_name {
        return Combine::Serial;
    }
    match op.params.choice(COMBINE, 0) {
        1 => Combine::Parallel,
        2 => Combine::Layer,
        _ => Combine::Serial,
    }
}

/// Whether op `i` of `ops` reads the run's input rather than the picture the
/// op above made. The first op has no run to join.
#[must_use]
pub fn joins(ops: &ResolvedStack, i: usize) -> bool {
    i > 0
        && ops
            .get(i)
            .is_some_and(|op| combine_of(&op) != Combine::Serial)
}

/// A Layer node's Blend mode and Mix (0..1), read the way the Blend seam
/// reads them.
#[must_use]
pub fn layer_mix(schema: &EffectSchema, p: Params<'_>) -> (u32, f32) {
    match crate::fx::cpu::blend_seam(schema, p) {
        Some((mode, mix, _)) => (mode, mix),
        None => (0, (p.float(MIX_ID, 100.0) / 100.0).clamp(0.0, 1.0)),
    }
}

/// The node's grade at full strength over every pixel: what a Layer node
/// lays over the run before its key and Mix.
#[must_use]
pub fn graded_packed(p: Params<'_>) -> Packed {
    let mut c = Correction::read(p);
    c.mix = 100.0;
    let mut pk = c.packed(p.float(DERIVED_PX_SCALE, 1.0));
    pk.qual.mode = 0;
    for w in &mut pk.windows {
        w.shape = 0;
    }
    pk
}

/// Parallel: what the node changed (`out - input`) added to `acc`.
pub fn parallel(acc: &mut [f32], out: &[f32], input: &[f32]) {
    for ((a, o), i) in acc.iter_mut().zip(out).zip(input) {
        *a += o - i;
    }
}

/// Layer: `graded` over `acc` through the Blend `mode` and `mix`, then the
/// key (one number a pixel; `None` keys every pixel whole).
pub fn layer(acc: &mut [f32], graded: &[f32], key: Option<&[f32]>, mode: u32, mix: f32) {
    let mut over = graded.to_vec();
    if mode != 0 || mix < 1.0 {
        crate::fx::cpu::blend_mix(&mut over, acc, mode, mix);
    }
    for (i, (a, o)) in acc
        .chunks_exact_mut(4)
        .zip(over.chunks_exact(4))
        .enumerate()
    {
        let k = key.map_or(1.0, |k| k.get(i).copied().unwrap_or(1.0).clamp(0.0, 1.0));
        for c in 0..4 {
            a[c] = a[c] * (1.0 - k) + o[c] * k;
        }
    }
}

/// Where a run stands as a stack is walked on the CPU.
#[derive(Default)]
pub struct Run {
    /// The picture the run's first node read, while a node below joins it.
    input: Option<Vec<f32>>,
    /// What the run had made before the joining node now drawing.
    acc: Option<Vec<f32>>,
    combine: Combine,
}

impl Run {
    /// Before op `i` draws on `rgba`. A joining node is handed the run's
    /// input, and what the run made is kept for [`Self::end`].
    pub fn begin(&mut self, ops: &ResolvedStack, i: usize, rgba: &mut [f32]) {
        let combine = ops.get(i).map_or(Combine::Serial, |op| combine_of(&op));
        if let Some(input) = self.input.as_ref().filter(|v| v.len() == rgba.len()) {
            if combine != Combine::Serial {
                self.acc = Some(rgba.to_vec());
                rgba.copy_from_slice(input);
                self.combine = combine;
                return;
            }
        }
        self.combine = Combine::Serial;
        self.acc = None;
        self.input = joins(ops, i + 1).then(|| rgba.to_vec());
    }

    /// Whether the node now drawing is a Layer node. Its own kernel does not
    /// run: [`Self::end`] draws it.
    #[must_use]
    pub fn layers(&self) -> bool {
        self.combine == Combine::Layer
    }

    /// After the node drew `rgba`: the run's picture, back in `rgba`.
    pub fn end(&mut self, rgba: &mut [f32], w: u32, h: u32, op: &ResolvedFx<'_>) {
        let (Some(mut acc), Some(input)) = (self.acc.take(), self.input.as_ref()) else {
            return;
        };
        match self.combine {
            Combine::Serial => return,
            Combine::Parallel => parallel(&mut acc, rgba, input),
            Combine::Layer => {
                let p = op.params;
                let pk = Correction::read(p).packed(p.float(DERIVED_PX_SCALE, 1.0));
                let key = cpu::correction_key(input, w, h, &pk, &[]);
                let mut graded = input.clone();
                cpu::correction(&mut graded, w, h, &graded_packed(p), &[]);
                let (mode, mix) = layer_mix(op.def.schema(), p);
                layer(&mut acc, &graded, key.as_deref(), mode, mix);
            }
        }
        rgba.copy_from_slice(&acc);
    }
}
