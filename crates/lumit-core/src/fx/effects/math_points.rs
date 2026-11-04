//! Math points lays out a stream of points by a formula.
//!
//! Shape says which. **Waves** spreads the points along a line and moves each
//! across and down by a wave of its place along it: a sine, a cosine, a
//! triangle, a square or a straight ramp, each with its own amplitude, how
//! many times it runs along the line, and where it starts. Two waves across
//! one another draw a Lissajous figure. **Rose** lays the points round a
//! rose curve, whose petals are Petals over Denominator: a whole number of
//! them draws that many petals (twice as many for an even one), and a
//! fraction overlaps them. **Spiral** winds them out from Position.
//!
//! Point *i* is worked out from *i* alone, so the stream costs the same at
//! any frame and its `id` is *i* for ever, which a Trail below follows. The
//! stream goes out on the Points socket, and is drawn as discs unless Mix is
//! 0.

use crate::fx::effects::vary_points::POINTS_OUT;
use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, registry::RowState, registry::Signature, schema::EffectSchema,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// The shapes there are, in Choice order.
pub const SHAPE_OPTIONS: &[&str] = &["Waves", "Rose", "Spiral"];
const SHAPE_WAVES: u32 = 0;
const SHAPE_ROSE: u32 = 1;

/// The waves a Waves axis can run, in Choice order.
pub const WAVE_OPTIONS: &[&str] = &["Linear", "Sine", "Cosine", "Triangle", "Square"];

/// The rows only Waves reads, and the ones only Rose reads.
const WAVES_ROWS: &[&str] = &[
    "x_wave",
    "x_amplitude",
    "x_frequency",
    "x_phase",
    "y_wave",
    "y_amplitude",
    "y_frequency",
    "y_phase",
];
const ROSE_ROWS: &[&str] = &["petals", "denominator"];
const ROUND_ROWS: &[&str] = &["radius", "turns"];

/// Math points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "math_points",
    label = "Math points",
    version = 1,
    category = Points,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
)]
pub struct MathPoints {
    /// The formula the points are laid out by.
    #[choice(label = "Shape", options = SHAPE_OPTIONS, default = 0)]
    pub shape: u32,

    /// How many points there are.
    #[counter(
        label = "Count",
        min = 1,
        max = 500,
        default = 40,
        hard_min = 0,
        hard_max = 100_000,
        unit = Raw
    )]
    pub count: i32,

    /// The middle of the figure, px@comp.
    #[slider(label = "Position x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub position_x: f32,

    /// px@comp. See [`position_x`](Self::position_x).
    #[slider(label = "Position y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub position_y: f32,

    /// How far the whole figure is turned about Position, degrees clockwise.
    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// Waves: the wave across.
    #[choice(label = "Wave x", options = WAVE_OPTIONS, default = 0)]
    pub x_wave: u32,

    /// Waves: how far across the wave reaches either side of Position, px.
    #[slider(label = "Amplitude x", min = -2000.0, max = 2000.0, default = 400.0, unit = Px)]
    pub x_amplitude: f32,

    /// Waves: how many times the wave across runs from the first point to
    /// the last.
    #[slider(label = "Frequency x", min = 0.0, max = 20.0, default = 1.0, unit = Raw)]
    pub x_frequency: f32,

    /// Waves: where along itself the wave across starts, degrees. Keyed, it
    /// runs the wave through the points.
    #[dial(label = "Phase x", default = 0.0)]
    pub x_phase: f32,

    /// Waves: the wave down.
    #[choice(label = "Wave y", options = WAVE_OPTIONS, default = 1)]
    pub y_wave: u32,

    /// Waves: how far down the wave reaches either side of Position, px.
    #[slider(label = "Amplitude y", min = -2000.0, max = 2000.0, default = 150.0, unit = Px)]
    pub y_amplitude: f32,

    /// Waves: how many times the wave down runs from the first point to the
    /// last.
    #[slider(label = "Frequency y", min = 0.0, max = 20.0, default = 2.0, unit = Raw)]
    pub y_frequency: f32,

    /// Waves: where along itself the wave down starts, degrees.
    #[dial(label = "Phase y", default = 0.0)]
    pub y_phase: f32,

    /// Rose and Spiral: how far the figure reaches from Position, px.
    #[slider(
        label = "Radius",
        min = 0.0,
        max = 2000.0,
        default = 300.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// Rose: the petals, over Denominator.
    #[slider(label = "Petals", min = 1.0, max = 20.0, default = 5.0, hard_min = 0.0, unit = Raw)]
    pub petals: f32,

    /// Rose: what Petals is divided by. A whole number of petals over 1 is
    /// that many petals; over anything else they overlap.
    #[slider(
        label = "Denominator",
        min = 1.0,
        max = 20.0,
        default = 1.0,
        hard_min = 0.01,
        unit = Raw
    )]
    pub denominator: f32,

    /// Spiral: how many times it winds round from the first point to the last.
    #[slider(label = "Turns", min = 0.0, max = 20.0, default = 3.0, unit = Raw)]
    pub turns: f32,

    /// The diameter of the disc a point is drawn as, px@comp.
    #[slider(min = 0.0, max = 200.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    /// How soft that disc's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The colour a point is drawn in.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// handed on and nothing is drawn.
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

/// One wave at `s` turns, from -1 to 1. Linear runs from -1 at the first
/// point to 1 at the last, whatever the turns.
fn wave(kind: u32, s: f32, along: f32) -> f32 {
    use std::f32::consts::TAU;
    let f = s.rem_euclid(1.0);
    match kind {
        0 => along * 2.0 - 1.0,
        1 => (s * TAU).sin(),
        2 => (s * TAU).cos(),
        3 => 1.0 - 4.0 * (f - 0.5).abs(),
        _ => {
            if f < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

impl MathPoints {
    /// The raster factor, for the camera, as Grid's.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// Point `i` of `n`, before Rotation and Position: across and down from
    /// the middle of the figure.
    fn place(self, i: usize, n: usize) -> (f32, f32) {
        use std::f32::consts::TAU;
        // Along the figure from 0 at the first point to 1 at the last. A
        // closed figure stops a step short of where it began.
        let along = |closed: bool| {
            let steps = if closed { n } else { n.saturating_sub(1) };
            if steps == 0 {
                0.0
            } else {
                i as f32 / steps as f32
            }
        };
        match self.shape {
            SHAPE_WAVES => {
                let a = along(false);
                let at = |frequency: f32, phase: f32| a * frequency + phase / 360.0;
                (
                    self.x_amplitude * wave(self.x_wave, at(self.x_frequency, self.x_phase), a),
                    self.y_amplitude * wave(self.y_wave, at(self.y_frequency, self.y_phase), a),
                )
            }
            SHAPE_ROSE => {
                // r = cos(kθ), k = n / d, which closes after 2d half turns, or
                // after d when n·d is odd. It always runs 2d: going round once
                // only on the odd whole numbers would make the points jump when
                // a keyed Denominator lands on one, as at a key of 1.
                let d = self.denominator.max(0.01);
                let k = self.petals / d;
                let theta = along(true) * 2.0 * d * std::f32::consts::PI;
                let r = self.radius * (k * theta).cos();
                (r * theta.cos(), r * theta.sin())
            }
            _ => {
                let a = along(false);
                let theta = a * self.turns * TAU;
                let r = self.radius * a;
                (r * theta.cos(), r * theta.sin())
            }
        }
    }

    /// The figure at this frame, as a points stream, in the bag's units.
    #[must_use]
    pub fn stream(self, projection: Projection) -> PointsStream {
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        let n = self.count.clamp(0, points::CAP_HARD as i32) as usize;
        let a = self.colour[3];
        let colour = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        for i in 0..n {
            let (x, y) = self.place(i, n);
            out.position.push([
                self.position_x + x * cos - y * sin,
                self.position_y + x * sin + y * cos,
                0.0,
            ]);
            out.speed.push([0.0; 3]);
            out.age.push(0.0);
            out.life.push(1.0);
            out.size.push(self.size.max(0.0));
            out.rotation.push(0.0);
            out.colour.push(colour);
            out.id.push(i as u64);
        }
        out
    }

    /// How the stream is drawn — a feathered disc per point, and the host Mix.
    #[must_use]
    pub fn draw_style(self) -> DrawStyle {
        DrawStyle {
            mode: RenderMode::Disc,
            feather: (self.feather / 100.0).clamp(0.0, 1.0),
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Math points' behaviour, Grid's shape: worked out on the host and drawn
/// through the shared points draw.
pub struct MathPointsDef;

impl EffectDef for MathPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MathPoints as EffectMetadata>::SCHEMA
    }

    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(MathPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    /// Each shape shows its own rows and hides the others'.
    fn row_state(&self, inst: &EffectInstance) -> RowState {
        let shape = match inst.param("shape") {
            Some(EffectValue::Choice(v)) => *v,
            _ => SHAPE_WAVES,
        };
        let mut hidden = Vec::new();
        if shape != SHAPE_WAVES {
            hidden.extend_from_slice(WAVES_ROWS);
        }
        if shape != SHAPE_ROSE {
            hidden.extend_from_slice(ROSE_ROWS);
        }
        if shape == SHAPE_WAVES {
            hidden.extend_from_slice(ROUND_ROWS);
        } else if shape == SHAPE_ROSE {
            hidden.push("turns");
        }
        RowState {
            hidden,
            ..RowState::default()
        }
    }
}
