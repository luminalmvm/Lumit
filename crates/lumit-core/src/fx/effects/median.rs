//! Median (docs/08 §3.64): the middle value of a neighbourhood — AE's Median.
//!
//! **In plain terms.** Every pixel is replaced by the *middle* value of the
//! little square of pixels around it — line them all up in order and take the
//! one in the centre. That one idea does something no blur can: a stray white
//! speck has no neighbours agreeing with it, so it never wins the vote and
//! disappears completely, while a real edge has half its window on each side and
//! stays exactly where it was. Turned up it flattens the picture into paint-like
//! patches, which is the other reason people reach for it.
//!
//! **Why the radius stops at 3.** Finding a middle value costs about
//! `(2r+1)⁴ ÷ 2` comparisons a pixel, so each step of the radius is roughly four
//! times the work of the last: 45 at radius 1, 325 at 2 and 1 225 at radius 3.
//! Radius 6 would be seventeen thousand. The slider's *hard* maximum is 3 rather
//! than a soft one that could be typed past — a control that silently clamps is
//! worse than a control that stops (§3.64 decision 2).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Median's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "median",
    label = "Median",
    version = 1,
    category = Stylise,
    // The catalogue's only `heavy` single-pass kernel, and it earns it: 1 225
    // compare-exchanges a pixel at the cap (§3.64 decision 2).
    cost = Heavy,
    // Radius's own reach: 3 px@comp, which on any comp small enough to care is
    // still well under 2 % of the diagonal.
    roi = PaddedPctDiag(2.0),
    // §2.2: a median of premultiplied colour would rank a soft edge by its
    // coverage rather than by its colour.
    premultiplied = false,
)]
pub struct Median {
    /// px@comp: half the width of the window. Rounded to whole raster pixels in
    /// [`packed`](Self::packed), so both paths select over the same square.
    /// 0 is the exact identity.
    #[slider(min = 0.0, max = 3.0, default = 2.0, hard_min = 0.0, hard_max = 3.0, unit = Px)]
    pub radius: f32,

    /// AE's "Operate on Alpha Channel". Off, the coverage is left exactly as it
    /// arrived — a median of the alpha moves the shape's outline, which is a
    /// separate thing to want from despeckling its colour.
    #[toggle(label = "Operate on alpha", default = false)]
    pub alpha: bool,

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

impl Median {
    /// The hard cap on the raster radius (§3.64 decision 2). The cost is
    /// `(2r+1)⁴ ÷ 2` compare-exchanges a pixel; this is where that stops being
    /// a frame time and starts being a coffee break.
    pub const MAX_RADIUS: i32 = 3;

    /// The longest sorted run either path carries: `⌈(2R+1)² ÷ 2⌉` at the cap,
    /// which is the WGSL kernel's `array<vec4<f32>, 25>` and the CPU
    /// reference's. Both are checked against this constant by the oracle test.
    pub const KEEP: usize = 25;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// rounding happens here, once, so the CPU reference and the WGSL kernel
    /// select over identical windows — `floor(x + ½)` rather than `round`,
    /// because WGSL breaks a tie to even and Rust breaks it away from zero
    /// (§3.58 decision 2's rule, on a coordinate this time).
    #[must_use]
    pub fn packed(self) -> cpu::MedianParams {
        cpu::MedianParams {
            radius: (self.radius.max(0.0) + 0.5).floor() as i32,
            alpha: self.alpha,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
        .clamped()
    }
}

/// Median's behaviour.
pub struct MedianDef;

impl EffectDef for MedianDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Median as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::median(rgba, w, h, &Median::read(p).packed());
    }
}
