//! Mosaic (docs/08 §3.65): the frame in flat blocks — AE's Mosaic.
//!
//! **In plain terms.** The frame is cut into a grid of rectangles and each one
//! is painted a single colour, which is how a face gets anonymised, how a
//! transition pixelates out, and how anything gets a retro-console look.
//!
//! Two things are worth knowing. **Every block boundary is worked out in whole
//! numbers**, not by dividing a coordinate by a block width — a division that
//! comes out exact lands a pixel in different blocks on the CPU and the GPU,
//! which is K-399's rule about a threshold arriving on a *coordinate* (§3.65).
//! And **the averaged mode samples the block rather than reading all of it**: a
//! true mean of a block would be thousands of taps redone by every pixel inside
//! it, so at most an 8×8 stratified sample is taken, which for a flat block is
//! the same answer for a hundredth of the work. A block under 8 pixels across is
//! sampled completely, so a fine mosaic *is* an exact mean.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Mosaic's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "mosaic",
    label = "Mosaic",
    version = 1,
    category = Stylise,
    // At most 64 taps a pixel, and one in the sharp mode.
    cost = Cheap,
    // A block reaches across the frame at one block wide, so no padding radius
    // describes it.
    roi = FullFrame,
    // Averaging premultiplied colour is what compositing means, and the alpha is
    // blocked with it so a cut-out gets blocky edges rather than smooth ones
    // round a blocky middle.
    premultiplied = true,
)]
pub struct Mosaic {
    /// How many blocks span the frame's width. The default pair is near-square
    /// on a 16:9 frame; AE's is 10 by 10, which is not.
    #[counter(
        label = "Horizontal blocks",
        min = 1,
        max = 200,
        default = 24,
        hard_min = 1,
        hard_max = 2000,
        unit = Raw
    )]
    pub horizontal_blocks: i32,

    /// See [`horizontal_blocks`](Self::horizontal_blocks).
    #[counter(
        label = "Vertical blocks",
        min = 1,
        max = 200,
        default = 14,
        hard_min = 1,
        hard_max = 2000,
        unit = Raw
    )]
    pub vertical_blocks: i32,

    /// On, the block takes the single colour of its centre pixel — crisper on
    /// graphic material, noisier on film. **Off by default, as AE's is**,
    /// because the mean is the picture people expect from a mosaic.
    #[toggle(label = "Sharp colours", default = false)]
    pub sharp_colours: bool,

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

impl Mosaic {
    /// The most samples taken along one axis of a block (§3.65 note 2). Eight
    /// squared is 64 taps, which is a blur's worth of work and no more.
    pub const MAX_SAMPLES: i32 = 8;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    #[must_use]
    pub fn packed(self) -> cpu::MosaicParams {
        cpu::MosaicParams {
            blocks: [
                self.horizontal_blocks.clamp(1, 2000),
                self.vertical_blocks.clamp(1, 2000),
            ],
            sharp: self.sharp_colours,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Mosaic's behaviour.
pub struct MosaicDef;

impl EffectDef for MosaicDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Mosaic as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::mosaic(rgba, w, h, &Mosaic::read(p).packed());
    }
}
