//! The GPU half of an effect whose maths is somebody else's (K-593).
//!
//! **In plain terms.** Every built-in effect in this table names a WGSL kernel.
//! A plugin has no kernel Lumit could name — its maths is compiled into a
//! library, and the only way to get a picture out of it is to hand it one. So
//! this pass is the round trip: read the picture off the card as plain floats,
//! give it to the definition, put what comes back onto the card again.
//!
//! That is a real cost — two transfers per plugin op per frame — and it is why a
//! plugin's declared cost is [`CostClass::Heavy`](lumit_core::fx::CostClass) and
//! why the degradation ladder gives one up before it gives up a built-in. OFX
//! 1.5's GPU render suites are the answer to it (docs/12 §2.4) and are a later
//! milestone; until then, the honest slow path beats a fast wrong one.
//!
//! **Nothing here knows what an OFX plugin is.** The pass talks to an
//! [`EffectDef`] and nothing else, which is what keeps `lumit-render` free of a
//! dependency on the plugin host (docs/05: engine crates do not depend on the
//! host of anything). `lumit-ofx` builds the definition; the composition root
//! registers it here and in the catalogue; this draws it.
//!
//! **It takes no side table** — `aux()` is [`AuxKind::None`] — and it consumes
//! no matte of its own, so the generic dissolve beside the dispatch spends it
//! exactly as it does for any other effect on
//! [`MatteRole::Strength`](lumit_core::fx::MatteRole) (K-395). A plugin's rows
//! are its own; a Matte the plugin never heard of would be a control nothing
//! consumes.

use std::collections::BTreeMap;
use std::sync::Mutex;

use lumit_core::fx::{EffectDef, Params};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;
use uuid::Uuid;

use super::{AuxSlot, GpuEffect};

type Tex = wgpu::Texture;

/// The ops whose plugin did not render this session, and why (docs/12 §2.3).
///
/// A plugin that crashed, missed its deadline or has been disabled renders its
/// input unchanged, and the layer wears a calm badge rather than the comp
/// stopping. This is where the badge's text comes from: the bridge reads it, the
/// Effect Controls row shows it. Keyed by the effect instance, so it is the row
/// that failed that is marked and not the effect everywhere it appears.
static ERRORED: Mutex<BTreeMap<Uuid, String>> = Mutex::new(BTreeMap::new());

/// Every op that failed, and why — newest wins for a given instance.
///
/// Empty on a session with no plugins, and on a session whose plugins all
/// worked, which is what the frontend draws nothing for.
#[must_use]
pub fn errored_ops() -> Vec<(Uuid, String)> {
    ERRORED
        .lock()
        .map(|table| {
            table
                .iter()
                .map(|(id, why)| (*id, why.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Why this one op's last frame was a placeholder, if it was.
///
/// The single-instance question, beside the whole-table one: the panel asks it
/// once per effect card it draws, and cloning the table for each of those would
/// be a copy per card per rebuild.
#[must_use]
pub fn error_of(instance: Uuid) -> Option<String> {
    ERRORED.lock().ok()?.get(&instance).cloned()
}

/// Forget an op's failure — what a re-render that worked, or a deleted effect,
/// leaves behind.
pub fn clear_errored(instance: Uuid) {
    if let Ok(mut table) = ERRORED.lock() {
        table.remove(&instance);
    }
}

/// Run one definition's own CPU render and file whatever it says about it.
///
/// This is the pass below, minus the two transfers: the picture goes to the
/// definition as plain floats and the badge comes back. It is a function rather
/// than three lines inside [`CpuPass::run`] because the failure path — a plugin
/// that died, hung or was switched off — has to be provable without a graphics
/// card, and the seam that turns "the plugin failed" into "the layer wears a
/// badge" is exactly these two calls in this order
/// (docs/impl/ofx-host.md §5 item 4).
pub fn apply_and_note(
    def: &'static dyn EffectDef,
    instance: Uuid,
    lt: f64,
    rgba: &mut [f32],
    w: u32,
    h: u32,
    p: Params<'_>,
) {
    def.apply_cpu_at(instance, lt, rgba, w, h, p);
    note(instance, def.last_error());
}

/// Record, or clear, one op's failure.
fn note(instance: Uuid, error: Option<String>) {
    let Ok(mut table) = ERRORED.lock() else {
        return;
    };
    match error {
        Some(why) => {
            table.insert(instance, why);
        }
        None => {
            table.remove(&instance);
        }
    }
}

/// A pass that renders through the definition's own CPU path.
struct CpuPass(&'static dyn EffectDef);

impl GpuEffect for CpuPass {
    fn match_name(&self) -> &'static str {
        self.0.schema().match_name
    }

    fn run(
        &self,
        _fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (instance, lt) = aux.op();
        // A read-back that fails is a passthrough, never a fault
        // (14-ENGINEERING-RULES §4): the picture the chain is carrying is
        // already the right answer for "this op did nothing".
        let Ok(mut rgba) = lumit_gpu::fx::readback_linear_f32(ctx, tex, w, h) else {
            note(
                instance,
                Some("the frame could not be read back".to_owned()),
            );
            return tex.clone();
        };
        apply_and_note(self.0, instance, lt, &mut rgba, w, h, p);
        lumit_gpu::fx::upload_linear_f32(ctx, &rgba, w, h)
    }
}

/// Register one run-time effect: its GPU pass here, and the definition itself
/// in `lumit-core`'s catalogue (K-593).
///
/// **One call, both tables, in that order.** They are joined by a `match_name`
/// string and nothing checks the join at compile time
/// (docs/impl/effect-registry.md §5, "two registries, one truth"); a caller that
/// did one and forgot the other would leave either an effect the render cannot
/// draw or a pass no effect names. The pass goes first, so that at no moment is
/// there a catalogue entry whose pass has not arrived.
///
/// `false` — and neither table touched — when the name is already known, which
/// is what makes a rescan idempotent. The pass is leaked for the same reason the
/// definition is: both are discovered while the program runs and then live as
/// long as the session.
pub fn register(def: &'static dyn EffectDef) -> bool {
    let name = def.schema().match_name;
    if super::gpu_effect(name).is_some() || lumit_core::fx::BUILTIN_DEFS.get(name).is_some() {
        return false;
    }
    super::register_gpu_effect(Box::leak(Box::new(CpuPass(def))))
        && lumit_core::fx::BUILTIN_DEFS.register(def)
}
