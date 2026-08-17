//! Block glitch (docs/08 §3.12): the hashed, blocky digital tear.
//!
//! **In plain terms.** Everything this effect draws is decided by a hash, and
//! the hash needs to know *which moment* it is drawing — but not continuously,
//! or the blocks blur into noise instead of popping. So layer time is discretised
//! to a fixed tick before it reaches the hash. A tick is not a control anybody
//! sets, so it is worked out at resolve time through the one hook that sees the
//! clock ([`EffectDef::resolve_derived`], K-385) and handed to the kernel as a
//! whole number, exactly as the hand-written resolve arm handed it over before.

use crate::fx::{
    cpu, EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value, GLITCH_TICK_HZ,
};
use lumit_fx_macros::Effect;

/// Block glitch's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "block_glitch",
    label = "Block glitch",
    version = 1,
    category = Distortion,
    cost = Cheap,
    roi = FullFrame,
    seeded = true, // its pixels are a function of time under constant parameters
)]
pub struct BlockGlitch {
    /// The master dial (§1.2): scales every hashed quantity. 0 is the bit-exact
    /// passthrough (pinned by test).
    #[slider(min = 0.0, max = 1.0, default = 0.35, hard_min = 0.0, hard_max = 1.0)]
    pub intensity: f32,

    /// px@comp (§2.3): a deliberately pixel-scale look. Declared `Px`, so the
    /// resolve step scales it by the preview factor and the generic rescale
    /// moves it again — what the old arm and `rescale_px` did between them.
    #[slider(
        min = 4.0,
        max = 128.0,
        default = 24.0,
        hard_min = 2.0,
        unit = Px
    )]
    pub block_size: f32,

    /// Per cent of Block size: a hashed offset to where each nominal block's
    /// content is read from.
    #[slider(
        label = "Rows/columns jitter",
        min = 0.0,
        max = 100.0,
        default = 25.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub block_jitter: f32,

    /// % diag (§2.3), the same currency as Blur's Radius and Length: how far a
    /// torn block slides.
    #[slider(
        label = "Displacement",
        min = 0.0,
        max = 15.0,
        default = 3.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = PctDiag
    )]
    pub block_amount: f32,

    /// % diag: a per-block hashed RGB split.
    #[slider(
        min = 0.0,
        max = 10.0,
        default = 1.0,
        hard_min = 0.0,
        hard_max = 50.0,
        unit = PctDiag
    )]
    pub channel_offset: f32,

    /// Per cent odds (× Intensity) that a block folds its own content to repeat
    /// a short hashed strip instead of a plain positional read.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 20.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub slice_repeat: f32,

    /// Which roll of the dice. Sits second-last, immediately before Mix (the
    /// owner's convention for seeded effects).
    #[seed]
    pub seed: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

/// Everything [`cpu::block_glitch`] and the WGSL twin want, in their order.
type Packed = (f32, u32, i32, f32, f32, f32, f32, f32, f32);

impl BlockGlitch {
    /// Layer time discretised at the fixed [`GLITCH_TICK_HZ`] (the §3.12 status
    /// note): block hashing reads this, never raw time. Never a panel row — the
    /// rate is deliberately not exposed.
    pub const DERIVED_TICK: ParamId = ParamId::new("derived.tick");

    /// This instance's tick read back out of a resolved bag: [`BlockGlitch::
    /// packed`]'s missing argument, so no caller has to know the id.
    pub fn tick_of(p: Params<'_>) -> i32 {
        p.int(Self::DERIVED_TICK, 0)
    }

    /// The numbers the kernel wants (docs/impl/effect-registry.md §2.4), clamped
    /// exactly as the old resolve arm clamped them: the two per-cent dials become
    /// plain 0..1 fractions, the block grid never degenerates below one pixel,
    /// and the two spatial offsets — already converted from % diag by the generic
    /// resolve — floor at zero. `tick` comes from the bag rather than from a
    /// declared row, because it is a function of time. Both render paths read
    /// this one method, so the CPU reference and the WGSL kernel cannot drift
    /// apart.
    pub fn packed(self, tick: i32) -> Packed {
        (
            self.intensity.clamp(0.0, 1.0),
            self.seed,
            tick,
            self.block_size.max(1.0),
            (self.block_jitter / 100.0).clamp(0.0, 1.0),
            self.block_amount.max(0.0),
            self.channel_offset.max(0.0),
            (self.slice_repeat / 100.0).clamp(0.0, 1.0),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Block glitch's behaviour.
pub struct BlockGlitchDef;

impl EffectDef for BlockGlitchDef {
    fn schema(&self) -> &'static EffectSchema {
        &<BlockGlitch as EffectMetadata>::SCHEMA
    }

    /// The discretised tick — the whole of what the old resolve arm did beyond
    /// reading its rows, moved unchanged (K-385). `floor` in `f64` and only then
    /// narrowed, exactly as the arm ordered it, so the frame a tick changes on is
    /// the same frame it always was.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(
            BlockGlitch::DERIVED_TICK,
            Value::Int((cx.lt * GLITCH_TICK_HZ).floor() as i32),
        );
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (
            intensity,
            seed,
            tick,
            block_size_px,
            jitter_frac,
            amount_px,
            chan_px,
            slice_frac,
            mix,
        ) = BlockGlitch::read(p).packed(BlockGlitch::tick_of(p));
        cpu::block_glitch(
            rgba,
            w,
            h,
            intensity,
            seed,
            tick,
            block_size_px,
            jitter_frac,
            amount_px,
            chan_px,
            slice_frac,
            mix,
        );
    }
}
