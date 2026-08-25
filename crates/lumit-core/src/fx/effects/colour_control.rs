//! Colour control (docs/08 §3.83): one colour, held for something else to read.
//!
//! **In plain terms.** A swatch that tints nothing. It is where a rig keeps its
//! colour, so that six effects reading it through expressions all change
//! together when the swatch is changed once.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The Colour control's one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "colour_control",
    label = "Colour control",
    version = 1,
    category = Controls,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct ColourControl {
    /// Scene-linear RGBA, on the same 0..4 edit range every colour that carries
    /// light declares (docs/08 §2.1): whatever reads this is going to put it in
    /// a picture, and a value above 1 is a real value there.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],
}

/// The Colour control's behaviour: none, by design.
pub struct ColourControlDef;

impl EffectDef for ColourControlDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ColourControl as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}
