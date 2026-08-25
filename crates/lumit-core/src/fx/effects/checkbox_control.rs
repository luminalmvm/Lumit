//! Checkbox control (docs/08 §3.82): one switch, held for something else to
//! read.
//!
//! **In plain terms.** A tick box that does nothing on its own. An expression
//! reads it and decides between two behaviours — a light on or off, a rig in one
//! mode or the other — so the choice is made in one visible place rather than
//! buried in the script.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The Checkbox control's one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "checkbox_control",
    label = "Checkbox control",
    version = 1,
    category = Controls,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct CheckboxControl {
    /// Off by default, which is the reading a rig that has not been set up yet
    /// should get.
    #[toggle(default = false)]
    pub checkbox: bool,
}

/// The Checkbox control's behaviour: none, by design.
pub struct CheckboxControlDef;

impl EffectDef for CheckboxControlDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CheckboxControl as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}
