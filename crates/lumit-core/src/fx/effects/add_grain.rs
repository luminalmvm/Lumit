//! Add grain (docs/08 §3.77): film grain laid on by tone — AE's Add Grain.
//!
//! **In plain terms.** Real film has a texture: clumps of silver that are bigger
//! than a pixel, softer than static, and strongest in the mid tones rather than
//! in the black or the white. This effect lays that on. §3.36 Noise is the same
//! family of thing done plainly — one value per pixel — and what separates the
//! two is here: a **size**, a **softness**, and a response that follows the tone
//! range.
//!
//! Like Noise it needs one number that is not a control: which frame this is.
//! That is worked out at resolve time from the layer's own clock
//! ([`EffectDef::resolve_derived`], K-385) and handed over as a plain integer, so
//! the kernel never sees time and two exports agree bit-for-bit (§2.4).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value};
use lumit_fx_macros::Effect;

/// Add grain's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "add_grain",
    label = "Add grain",
    version = 1,
    category = Generate,
    cost = Cheap,
    roi = Exact,
    // §2.2, and §3.36's reason: grain sprinkled onto premultiplied values would
    // be scaled by coverage and fade out across a soft edge instead of lying
    // evenly over it.
    premultiplied = false,
    seeded = true,
    // K-428: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Intensity per pixel: white grains at the full Intensity, grey          more finely, black not at all",
    ),
)]
pub struct AddGrain {
    /// How strong the grain is, per cent. 0 is the bit-exact passthrough; the
    /// range runs to 200 because a deliberately coarse stock is a look.
    #[slider(min = 0.0, max = 200.0, default = 50.0, hard_min = 0.0, unit = Percent)]
    pub intensity: f32,

    /// How big one grain is, px@comp (§2.3), so a Half-resolution preview shows
    /// the grain of the export rather than twice as much of it.
    #[slider(min = 0.5, max = 20.0, default = 2.0, hard_min = 0.1, unit = Px)]
    pub size: f32,

    /// Per cent. 0 is a sharp scan-grain of flat cells, 100 a soft organic
    /// mottle — the same field read two ways and crossfaded (§3.77's second
    /// note).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub softness: f32,

    /// How much grain the red channel gets, per cent of Intensity — AE's Channel
    /// Balance.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub red: f32,

    /// The green channel's share; see [`red`](Self::red).
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub green: f32,

    /// The blue channel's share; see [`red`](Self::red).
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub blue: f32,

    /// Off (the default) draws the three channels from three independent fields,
    /// which is what colour film does. On draws one field three times, so the
    /// grain reads as luminance and cannot tint the picture.
    #[toggle(default = false)]
    pub monochrome: bool,

    /// How much grain the dark end of the range gets, per cent.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub shadows: f32,

    /// The middle of the range; see [`shadows`](Self::shadows). The three weights
    /// are hat functions summing to one, so 100/100/100 is provably neutral
    /// (§3.77's third note).
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub midtones: f32,

    /// The bright end of the range; see [`shadows`](Self::shadows).
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub highlights: f32,

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

impl AddGrain {
    /// Layer time discretised to the millisecond — §3.36's derived tick, for its
    /// reason and by its arithmetic. Never a panel row: it is what the clock
    /// produces, not what anyone typed.
    pub const DERIVED_TICK: ParamId = ParamId::new("derived.tick");

    /// This instance's tick read back out of a resolved bag.
    #[must_use]
    pub fn tick_of(p: Params<'_>) -> i32 {
        p.int(Self::DERIVED_TICK, 0)
    }

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The 0.1 that turns Intensity into an amplitude on the perceptual value
    /// lives here, once, folded into the three channel gains — a scale written
    /// down twice is a scale that will one day be two scales.
    #[must_use]
    pub fn packed(self, tick: i32) -> cpu::AddGrainParams {
        let amount = self.intensity.max(0.0) / 100.0 * 0.1;
        cpu::AddGrainParams {
            amplitude: [
                amount * self.red.max(0.0) / 100.0,
                amount * self.green.max(0.0) / 100.0,
                amount * self.blue.max(0.0) / 100.0,
            ],
            inv_size: 1.0 / self.size.max(0.1),
            softness: (self.softness / 100.0).clamp(0.0, 1.0),
            tonal: [
                self.shadows.max(0.0) / 100.0,
                self.midtones.max(0.0) / 100.0,
                self.highlights.max(0.0) / 100.0,
            ],
            monochrome: self.monochrome,
            seed: self.seed,
            tick: if self.animate { tick } else { 0 },
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Add grain's behaviour.
pub struct AddGrainDef;

impl EffectDef for AddGrainDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AddGrain as EffectMetadata>::SCHEMA
    }

    /// The frame's tick. Rounded in `f64` and only then narrowed, so the frame a
    /// draw changes on is decided once and identically on every machine (§3.36).
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(
            AddGrain::DERIVED_TICK,
            Value::Int((cx.lt * 1000.0).round() as i32),
        );
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::add_grain(rgba, w, h, &AddGrain::read(p).packed(AddGrain::tick_of(p)));
    }
}
