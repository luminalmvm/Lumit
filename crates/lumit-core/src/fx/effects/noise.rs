//! Noise (docs/08 §3.36): per-pixel uniform or gaussian grain.
//!
//! **In plain terms.** One of the two things in this batch that does not replace
//! the picture — it sprinkles a little light and dark onto the picture that
//! arrived. Turned up it is video noise; turned down it is the film grain that
//! stops a flat gradient looking like a computer drew it.
//!
//! The only number the kernel wants that is not a control is *which frame this
//! is*, because grain that does not move is not grain. That is worked out at
//! resolve time from the layer's own clock ([`EffectDef::resolve_derived`],
//! K-385) and handed over as a plain integer, so the kernel never sees time and
//! two exports agree bit-for-bit (§2.4).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value};
use lumit_fx_macros::Effect;

/// Noise's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "noise",
    label = "Noise",
    version = 1,
    category = Generate,
    cost = Cheap,
    roi = Exact,
    // §2.2: adding a signed amount to a channel is affine, exactly as
    // Brightness is — grain sprinkled onto premultiplied values would be scaled
    // by coverage and fade out across a soft edge instead of lying evenly over
    // it.
    premultiplied = false,
    seeded = true,
)]
pub struct Noise {
    /// Per cent of full scene-linear scale: the grain's amplitude. 0 is the
    /// bit-exact passthrough (pinned by test); unbounded above, because a
    /// deliberate wall of static is a legitimate look (§1.2's one-sided range).
    #[slider(min = 0.0, max = 100.0, default = 25.0, hard_min = 0.0, unit = Percent)]
    pub amount: f32,

    /// Uniform draws flat across the range; Gaussian clusters near zero, which
    /// reads as film grain rather than video noise.
    #[choice(options = ["Uniform", "Gaussian"], default = 0)]
    pub distribution: u32,

    /// Off (the default, matching AE) draws one value for all three channels, so
    /// the grain reads as luminance and does not tint the picture. On draws the
    /// three independently.
    #[toggle(label = "Colour noise", default = false)]
    pub colour_noise: bool,

    /// On, the grain is redrawn every frame — what grain does. Off freezes one
    /// draw — what a texture does.
    #[toggle(default = true)]
    pub animate: bool,

    /// Which draw the grain follows (§2.4).
    #[seed]
    pub seed: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// Everything [`cpu::noise`] and the WGSL twin want, in their order.
type Packed = (f32, bool, bool, u32, i32, f32);

impl Noise {
    /// Layer time discretised to the millisecond — a distinct draw per frame at
    /// any frame rate up to 1000 fps, which is the documented ceiling of "a
    /// fresh draw every frame" (docs/08 §3.36). Never a panel row: it is what
    /// the clock produces, not what anyone typed. The Block glitch tick (K-385)
    /// is the same shape; this one is finer because grain must not repeat
    /// between frames where a block pop may.
    pub const DERIVED_TICK: ParamId = ParamId::new("derived.tick");

    /// This instance's tick read back out of a resolved bag: [`Noise::packed`]'s
    /// missing argument, so no caller has to know the id.
    #[must_use]
    pub fn tick_of(p: Params<'_>) -> i32 {
        p.int(Self::DERIVED_TICK, 0)
    }

    /// The numbers the kernel wants (docs/impl/effect-registry.md §2.4). Amount
    /// becomes a plain fraction of scene-linear scale, and the tick is pinned to
    /// zero when Animate is off — which is the whole of what that switch does,
    /// decided here rather than in two kernels. Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    #[must_use]
    pub fn packed(self, tick: i32) -> Packed {
        (
            (self.amount / 100.0).max(0.0),
            self.distribution == 1,
            self.colour_noise,
            self.seed,
            if self.animate { tick } else { 0 },
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Noise's behaviour.
pub struct NoiseDef;

impl EffectDef for NoiseDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Noise as EffectMetadata>::SCHEMA
    }

    /// The frame's tick. Rounded in `f64` and only then narrowed, so the frame a
    /// draw changes on is decided once and identically on every machine.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(
            Noise::DERIVED_TICK,
            Value::Int((cx.lt * 1000.0).round() as i32),
        );
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (amount, gaussian, colour, seed, tick, mix) = Noise::read(p).packed(Noise::tick_of(p));
        cpu::noise(rgba, w, h, amount, gaussian, colour, seed, tick, mix);
    }
}
