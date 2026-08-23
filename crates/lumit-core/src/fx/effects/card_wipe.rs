//! Card wipe (docs/08 §3.72): the frame as a grid of cards, turning away — AE's
//! Card Wipe, in its camera-less form.
//!
//! **In plain terms.** The picture is cut into a grid of rectangles and each one
//! turns edge-on and disappears, like a departures board changing. Rows and
//! Columns say how many cards there are, Flip order which corner the wave starts
//! from, Transition width how much they overlap, and Randomness how much the
//! order is shuffled.
//!
//! Two things are worth knowing.
//!
//! **The cards are geometry, not particles.** A card is a rectangle with one
//! rotation on it and a position that is a function of its place in the grid;
//! nothing is simulated. That is the standing exclusion in
//! docs/impl/ae-effect-parity.md — Lumit's particle system is a programme of its
//! own — and a grid of flipping rectangles is on the right side of it.
//!
//! **The perspective is inverted, not drawn.** Lumit's effects *gather*: a pixel
//! asks where it should read from, rather than a shape being drawn into the
//! frame. So instead of transforming the card and rasterising it, the kernel
//! solves the projection backwards — "which point of the flat card is standing
//! where I am?" — which for a one-point projection is a single division. That is
//! the whole reason this is one cheap pass and not a geometry pipeline.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Which line the card turns about, in schema order. AE's X and Y under names
/// that say what happens (docs/01 §9).
pub const FLIP_AXIS_OPTIONS: &[&str] = &["Horizontal axis", "Vertical axis", "Random"];

/// Which way it turns, in schema order — AE's Positive and Negative. With the
/// perspective in §3.72, Forwards tips the leading edge towards the viewer.
pub const FLIP_DIRECTION_OPTIONS: &[&str] = &["Forwards", "Backwards", "Random"];

/// The order the wave of flips travels in, in schema order. AE's fifth entry,
/// Gradient, is not carried — see docs/08 §3.72's fifth decision.
pub const FLIP_ORDER_OPTIONS: &[&str] = &[
    "Left to right",
    "Right to left",
    "Top to bottom",
    "Bottom to top",
];

/// Card wipe's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "card_wipe",
    label = "Card wipe",
    version = 1,
    category = Transition,
    // One hash, one divide and one bilinear tap a pixel.
    cost = Cheap,
    // One column is a card the width of the frame, so no padding radius
    // describes the reach (§3.65's reasoning).
    roi = FullFrame,
    premultiplied = true,
    // K-429: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Completion per pixel: the cards have turned further where the \
         matte is bright, and it is asked per pixel, so a card can be half \
         flipped and half standing",
    ),
    seeded = true,
)]
pub struct CardWipe {
    /// How far through the wipe we are, per cent. **50 by default, where AE's is
    /// 0**, for docs/08 §3.46's reason (§1.2: no no-op defaults).
    /// Closed 0..100 (K-414): a wipe cannot be less than begun or more than
    /// complete, so the range is the parameter, and typing past either end
    /// would offer a picture that does not exist.
    #[bounded(min = 0.0, max = 100.0, default = 50.0)]
    pub completion: f32,

    /// How much of the whole wipe one card's own flip takes, per cent. At 100
    /// every card flips together; at a few per cent they go one after another in
    /// a hard wave. Floored above zero so the ramp has a slope.
    #[slider(
        label = "Transition width",
        min = 1.0,
        max = 100.0,
        default = 50.0,
        hard_min = 1.0,
        hard_max = 100.0
    )]
    pub transition_width: f32,

    /// How many cards down the frame.
    #[counter(min = 1, max = 64, default = 6, hard_min = 1, hard_max = 256)]
    pub rows: i32,

    /// How many cards across it. The default pair is near-square on a 16:9
    /// frame, as §3.65's is.
    #[counter(min = 1, max = 64, default = 8, hard_min = 1, hard_max = 256)]
    pub columns: i32,

    /// Which line each card turns about. Random picks per card, from the Seed.
    #[choice(label = "Flip axis", options = *FLIP_AXIS_OPTIONS, default = 0)]
    pub flip_axis: u32,

    /// Which way each card turns. Random picks per card, from the Seed.
    #[choice(label = "Flip direction", options = *FLIP_DIRECTION_OPTIONS, default = 0)]
    pub flip_direction: u32,

    /// Which corner the wave of flips starts from.
    #[choice(label = "Flip order", options = *FLIP_ORDER_OPTIONS, default = 0)]
    pub flip_order: u32,

    /// How far each card's turn is shuffled away from Flip order, per cent. At
    /// 100 the order is entirely the Seed's; at 0 it is entirely Flip order's,
    /// which is AE's default.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub randomness: f32,

    /// Which shuffle this instance gets (§2.4) — and, where Flip axis or Flip
    /// direction is Random, which way each card goes.
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

impl CardWipe {
    /// How far the camera stands from the cards, in card half-widths — fixed,
    /// and deliberately so (docs/08 §3.72's fourth decision). Lives beside the
    /// kernel that uses it, [`cpu::CARD_VIEW_DISTANCE`].
    pub const VIEW_DISTANCE: f32 = cpu::CARD_VIEW_DISTANCE;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). Flip
    /// order collapses to an axis and an affine pair, so the kernel reads the
    /// ramp rather than branching four ways.
    #[must_use]
    pub fn packed(self) -> cpu::CardWipeParams {
        let (order_axis, order_bias, order_scale) = match self.flip_order {
            1 => (0u32, 1.0, -1.0),
            2 => (1u32, 0.0, 1.0),
            3 => (1u32, 1.0, -1.0),
            _ => (0u32, 0.0, 1.0),
        };
        let width = (self.transition_width / 100.0).clamp(0.01, 1.0);
        cpu::CardWipeParams {
            grid: [self.columns.clamp(1, 256), self.rows.clamp(1, 256)],
            completion: (self.completion / 100.0).clamp(0.0, 1.0),
            inv_width: 1.0 / width,
            one_minus_width: 1.0 - width,
            order_axis,
            order_bias,
            order_scale,
            axis: self.flip_axis.min(2),
            direction: self.flip_direction.min(2),
            randomness: (self.randomness / 100.0).clamp(0.0, 1.0),
            seed: self.seed,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Card wipe's behaviour.
pub struct CardWipeDef;

impl EffectDef for CardWipeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CardWipe as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::card_wipe(rgba, w, h, &CardWipe::read(p).packed());
    }
}
