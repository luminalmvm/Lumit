//! Merge (docs/impl/node-graph-comp.md §1.3): the node that lays one picture
//! over another.
//!
//! **In plain terms.** A layer stack joins pictures by stacking layers and
//! picking a blend mode. A node graph has no stack, so it needs a box that says
//! the same thing: picture A goes over picture B, in this mode, at this
//! opacity. That box is this one, and it is what makes a fork worth having -
//! send one picture down two branches, treat each differently, and merge them
//! back.
//!
//! **A node graph's own.** Its category is Compositing, whose entries are not
//! offered on a layer stack: the stack already has blend modes, and a Merge
//! there would be a control with no second picture to reach. Like the Controls
//! family it is `is_image_op() == false`, so a resolve of a layer stack never
//! makes an op of it; the graph walk realises it with the compositor rather than
//! a kernel, which is why it has no CPU oracle and no GPU entry.
//!
//! **The mode row is spelled `mode`, never `blend`.** `EffectSchema::blend()`
//! treats a row called `blend` over the blend-mode names as the Mix seam's own
//! and would blend the result a second time against the input, which is the bug
//! schema.rs records against the Lens flare.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema, CHOICE_UNGROUPED};
use lumit_fx_macros::Effect;

/// The Merge's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "merge",
    label = "Merge",
    version = 1,
    category = Compositing,
    // Two full-frame pictures composited: the cheapest thing the walk does.
    cost = Trivial,
    // Both pictures are laid over each other whole, so the region is the frame.
    roi = FullFrame,
    // Nothing to dissolve: the box has no picture of its own that a matte could
    // say "this much of" - its two pictures arrive on wires and its Opacity is
    // the amount already.
    matte = false,
)]
pub struct Merge {
    /// How A is laid over B - the layer blend modes verbatim, so a hand that
    /// knows the Timeline's dropdown knows this one.
    #[choice(
        options = *::lumit_core::model::BlendMode::NAMES,
        default = 0,
        dividers_after = CHOICE_UNGROUPED
    )]
    pub mode: u32,

    /// How much of A is laid on, per cent. Not the host-uniform Mix: this is
    /// the box's whole arithmetic rather than a dissolve back to an input it
    /// does not have.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,
}

/// The Merge's behaviour: none of its own. The graph walk composites its two
/// wired pictures, so `apply_cpu` keeps its identity default exactly as the
/// orchestration-only effects do.
pub struct MergeDef;

impl EffectDef for MergeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Merge as EffectMetadata>::SCHEMA
    }

    /// It joins two pictures the graph walk hands it; it is not a kernel in a
    /// stack. The resolve step pushes no op for it, as it pushes none for a
    /// Slider control.
    fn is_image_op(&self) -> bool {
        false
    }
}
