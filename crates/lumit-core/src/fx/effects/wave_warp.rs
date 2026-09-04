//! Wave warp (docs/08 §3.54): a travelling wave across the frame — AE's Wave
//! Warp.
//!
//! **In plain terms.** A wave runs across the picture in the direction you set,
//! and the picture slides sideways to it — the transverse wave, which is the one
//! a flag makes. Wave width is the distance between crests, Wave height how far
//! the picture slides, Phase where along the wave the frame is caught. Pinning
//! nails chosen edges of the frame down so the wave dies away before it reaches
//! them.
//!
//! As with Ripple (§3.53), AE's Wave Speed is not here: Phase is the same motion
//! with the timeline holding the stopwatch, and nothing reads a clock (§2.4).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Wave warp's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "wave_warp",
    label = "Wave warp",
    version = 1,
    category = Distortion,
    // One wave shape and one bilinear tap a pixel.
    cost = Cheap,
    // Wave height's own reach; its hard maximum is open, so the padding is the
    // slider's 500 px@comp doubled, and the pin ramp only shortens the slide.
    roi = PaddedPx(1000.0),
    premultiplied = true,
    // The matte scales the displacement, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Wave height per pixel: white slides the full height, grey less, \
         black not at all",
    ),
)]
pub struct WaveWarp {
    /// The wave's shape. All five run between −1 and 1 over one wave; Square
    /// and Sawtooth carry a step, which the resampler renders as the hard slice
    /// they are meant to be.
    #[choice(
        label = "Wave type",
        options = ["Sine", "Square", "Triangle", "Sawtooth", "Circle"],
        default = 0
    )]
    pub wave_type: u32,

    /// px@comp: how far the picture slides at a crest. Signed — a negative
    /// height is the same wave a half-turn of Phase along.
    #[slider(
        label = "Wave height",
        min = -500.0,
        max = 500.0,
        default = 15.0,
        unit = Px
    )]
    pub wave_height: f32,

    /// px@comp: the distance between one crest and the next. Floored at a pixel
    /// so the reciprocal stays finite.
    #[slider(
        label = "Wave width",
        min = 1.0,
        max = 2000.0,
        default = 120.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub wave_width: f32,

    /// Degrees from straight up, clockwise (the catalogue's convention, §3.43,
    /// §3.46, §3.47): the direction the wave **travels**. The picture slides
    /// across it, a quarter-turn round.
    #[dial(default = 90.0, step = 15.0)]
    pub direction: f32,

    /// Degrees: where along the wave this frame is caught. AE's Wave Speed
    /// converts to a linear keyframe on this (§3.54's second note).
    #[dial(default = 0.0, step = 45.0)]
    pub phase: f32,

    /// Which of the frame's edges are held still. A pinned edge cannot move, so
    /// the slide ramps to zero across the last `|Wave height|` pixels before it.
    /// All eight of AE's combinations, because the ramp is per edge here.
    #[choice(
        label = "Pinning",
        options = [
            "None",
            "All edges",
            "Left and right",
            "Top and bottom",
            "Left edge",
            "Right edge",
            "Top edge",
            "Bottom edge"
        ],
        default = 0
    )]
    pub pinning: u32,

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

impl WaveWarp {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// direction's sine and cosine are spent here, the wave width becomes a
    /// reciprocal, Phase becomes turns and the Pinning choice becomes four
    /// per-edge flags — so the kernel runs no trigonometry and no division.
    #[must_use]
    pub fn packed(self) -> cpu::WaveWarpParams {
        let theta = self.direction.to_radians();
        let (sin, cos) = theta.sin_cos();
        // Left, right, top, bottom.
        let pin = match self.pinning {
            1 => [1.0, 1.0, 1.0, 1.0],
            2 => [1.0, 1.0, 0.0, 0.0],
            3 => [0.0, 0.0, 1.0, 1.0],
            4 => [1.0, 0.0, 0.0, 0.0],
            5 => [0.0, 1.0, 0.0, 0.0],
            6 => [0.0, 0.0, 1.0, 0.0],
            7 => [0.0, 0.0, 0.0, 1.0],
            _ => [0.0, 0.0, 0.0, 0.0],
        };
        cpu::WaveWarpParams {
            // From straight up, clockwise, on a raster whose y grows downward.
            dir: [sin, -cos],
            // That vector turned a quarter-turn clockwise on screen.
            perp: [cos, sin],
            height: self.wave_height,
            inv_width: 1.0 / self.wave_width.max(1e-3),
            turns: self.phase / 360.0,
            shape: self.wave_type.min(4),
            pin,
            // The pin ramp is |Wave height| wide, so a pinned edge cannot be
            // reached from outside the frame. Floored so a zero height does not
            // divide.
            inv_pin_band: 1.0 / self.wave_height.abs().max(1e-3),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Wave warp's behaviour.
pub struct WaveWarpDef;

impl EffectDef for WaveWarpDef {
    fn schema(&self) -> &'static EffectSchema {
        &<WaveWarp as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::wave_warp(rgba, w, h, &WaveWarp::read(p).packed());
    }
}
