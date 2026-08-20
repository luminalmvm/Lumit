//! Radio waves (docs/08 §3.75): shapes emitted from a point and expanding —
//! AE's Radio Waves.
//!
//! **In plain terms.** A point on the frame throws out a shape — a ring, a
//! polygon, a star — over and over at a steady rate. Each one grows, turns and
//! fades as it ages, so what you see is a set of expanding outlines at different
//! sizes: a sonar ping, a shock wave, a drop in a pond.
//!
//! **Time is a control, not the clock** (§2.4, and §3.53's ruling). AE's version
//! reads what second it is; Lumit's takes the second as a parameter the timeline
//! animates, so a preview and an export cannot disagree and scrubbing back puts
//! every wave exactly where it was. Frequency, Expansion, Lifespan and Spin keep
//! their per-second units and mean what they say against *that* Time.
//!
//! Every wave is the same shape at a different size, so §3.71's sector solve is
//! done here once for a **unit** shape and the kernel multiplies it.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, Params};
use lumit_fx_macros::Effect;

/// A star's depth means nothing until the star is switched on — §3.71's row.
pub const RADIO_WAVES_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "star_depth",
    on: "star",
    cond: EnabledCond::BoolIs(true),
}];

/// Radio waves' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "radio_waves",
    label = "Radio waves",
    version = 1,
    category = Generate,
    // One `atan2` and up to 32 cheap rings a pixel — §3.71's admission again
    // (K-399).
    cost = Cheap,
    roi = Exact,
    premultiplied = true,
    enabled_when = RADIO_WAVES_ENABLED_WHEN,
)]
pub struct RadioWaves {
    /// Where the waves are emitted, px@comp (K-260: point parameters are
    /// PIXELS). The schema default is nominal 1080p centre;
    /// [`instantiate_for_raster`](crate::fx::instantiate_for_raster) centres a
    /// fresh instance on the actual comp.
    #[slider(label = "Producer x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Producer y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// The second the emitter has reached. Keyframe it linearly and the effect
    /// is AE's exactly; hold it and the picture holds still.
    #[slider(min = 0.0, max = 10.0, default = 3.0, hard_min = 0.0, unit = Seconds)]
    pub time: f32,

    /// Waves a second.
    #[slider(min = 0.1, max = 20.0, default = 2.0, hard_min = 0.01)]
    pub frequency: f32,

    /// How fast a wave grows, px@comp a second (§2.3).
    #[slider(min = 0.0, max = 1000.0, default = 260.0, hard_min = 0.0, unit = Px)]
    pub expansion: f32,

    /// How long a wave lives, seconds.
    #[slider(min = 0.1, max = 10.0, default = 2.0, hard_min = 0.02, unit = Seconds)]
    pub lifespan: f32,

    /// How many corners the shape has. 32 reads as a circle, which is what a
    /// radio wave usually wants; drop it to six for a polygon.
    #[counter(min = 3, max = 64, default = 32, hard_min = 3, hard_max = 64)]
    pub sides: i32,

    /// Put every second vertex on an inner radius, making a star of the polygon.
    #[toggle(default = false)]
    pub star: bool,

    /// How deep the star's notches go, per cent of the radius.
    #[slider(
        label = "Star depth",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub star_depth: f32,

    /// Which way the shape faces, degrees from straight up, clockwise.
    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    /// How fast a wave turns as it ages, degrees a second. It is a *per-wave*
    /// spin, so older waves have turned further than younger ones — which is
    /// what makes the set read as a fan rather than as one shape.
    #[dial(default = 0.0, step = 15.0)]
    pub spin: f32,

    /// How thick the outline is, px@comp.
    #[slider(
        label = "Stroke width",
        min = 0.0,
        max = 100.0,
        default = 4.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub stroke_width: f32,

    /// The outline's colour. Scene-linear and open above 1 (§2.1).
    #[colour(default = [0.25, 0.70, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// How strong the outline is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub opacity: f32,

    /// How much of a wave's life it spends fading up, per cent.
    #[slider(
        label = "Fade in",
        min = 0.0,
        max = 100.0,
        default = 15.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub fade_in: f32,

    /// How much of a wave's life it spends fading out, per cent.
    #[slider(
        label = "Fade out",
        min = 0.0,
        max = 100.0,
        default = 45.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub fade_out: f32,

    /// On, the layer that arrived stays under the waves; off, the waves are all
    /// there is.
    #[toggle(label = "Composite on original", default = true)]
    pub composite_on_original: bool,

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

impl RadioWaves {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The unit shape's sector is solved here, once, exactly as §3.71 solves its
    /// polygon — and the newest wave's index is taken here too, because
    /// `floor(Time × Frequency)` decides *which* rings exist and one bit of
    /// disagreement about it is a whole ring (K-399).
    #[must_use]
    pub fn packed(self) -> cpu::RadioWavesParams {
        use std::f32::consts::TAU;
        let n = self.sides.clamp(3, 64) as f32;
        let period = TAU / n;
        let (angle_b, radius_b) = if self.star {
            (
                period * 0.5,
                1.0 - (self.star_depth / 100.0).clamp(0.0, 1.0),
            )
        } else {
            (period, 1.0)
        };
        let a = [1.0f32, 0.0];
        let b = [radius_b * angle_b.cos(), radius_b * angle_b.sin()];
        // §3.71's outward normal, on the unit shape.
        let normal = [b[1] - a[1], a[0] - b[0]];
        let inv_len = 1.0
            / (normal[0] * normal[0] + normal[1] * normal[1])
                .sqrt()
                .max(1e-6);
        let frequency = self.frequency.max(0.01);
        let time = self.time.max(0.0);
        let lifespan = self.lifespan.max(0.02);
        let alive = (lifespan * frequency).ceil() as i32 + 1;
        cpu::RadioWavesParams {
            centre: [self.centre_x, self.centre_y],
            vertex: a,
            normal: [normal[0] * inv_len, normal[1] * inv_len],
            period,
            rotation: self.rotation.to_radians(),
            spin: self.spin.to_radians(),
            newest: (time * frequency).floor() as i32,
            count: alive.clamp(1, cpu::RADIO_WAVES_MAX),
            time,
            period_s: 1.0 / frequency,
            expansion: self.expansion.max(0.0),
            lifespan,
            half_width: self.stroke_width.max(0.0) * 0.5,
            // Floored so a hard fade is a step rather than a divide by zero
            // (docs/14 §4); neither path divides by nothing.
            fade_in: (self.fade_in / 100.0).clamp(0.0, 1.0).max(1e-3),
            fade_out: (self.fade_out / 100.0).clamp(0.0, 1.0).max(1e-3),
            colour: [self.colour[0], self.colour[1], self.colour[2]],
            opacity: (self.opacity / 100.0).clamp(0.0, 1.0),
            composite: self.composite_on_original,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Radio waves' behaviour.
pub struct RadioWavesDef;

impl EffectDef for RadioWavesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RadioWaves as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::radio_waves(rgba, w, h, &RadioWaves::read(p).packed());
    }
}
