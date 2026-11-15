//! Move: a cut made of movement. The outgoing picture leaves and the incoming
//! one arrives, pushed, slid, zoomed or turned, with a smear along the way.
//!
//! One instance runs on each of the two pictures, and Side says which it is.
//! Completion is how far through the cut both are, so the two meet in the
//! middle: whatever the outgoing picture has done by then, the incoming one
//! is about to undo. Each picture is resampled through a few inverse affines,
//! worked out here in `f64`, and averaged, which is Shake's motion blur pass.
//! No kernel of its own.

use crate::fx::{
    builtins::EDGE_OPTIONS, cpu, effects::shake::Shaken, maths::SHAKE_MB_SAMPLES, params::Params,
    presets::EffectPreset, presets::PresetValue, registry::EffectDef, registry::EffectMetadata,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;
use PresetValue::{Bool, Choice, Float};

/// Which picture an instance is on, in schema order.
pub const SIDE_OPTIONS: &[&str] = &["Outgoing", "Incoming"];

/// Which pictures travel, in schema order. The other sits still.
pub const MOVES_OPTIONS: &[&str] = &["Both", "Outgoing", "Incoming"];

/// How the move gathers speed, in schema order.
pub const EASE_OPTIONS: &[&str] = &["Linear", "Smooth", "Sharp"];

/// Move's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "move",
    label = "Move",
    version = 1,
    category = Transition,
    // A few bilinear taps a pixel.
    cost = Cheap,
    // A picture can travel the whole frame.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct MoveTransition {
    /// How far through the cut, per cent.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub completion: f32,

    /// Which picture this is.
    #[choice(options = *SIDE_OPTIONS, default = 0)]
    pub side: u32,

    /// Which pictures travel.
    #[choice(options = *MOVES_OPTIONS, default = 0)]
    pub moves: u32,

    /// The way they travel, degrees. 0 is right and 90 is down, as Directional
    /// blur's angle.
    #[dial(label = "Direction", default = 180.0, step = 15.0)]
    pub direction: f32,

    /// How far, per cent. 100 leaves the frame exactly.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub distance: f32,

    /// The scale the outgoing picture reaches, per cent. The incoming one
    /// comes from the inverse.
    #[slider(
        min = 25.0,
        max = 800.0,
        default = 100.0,
        hard_min = 1.0,
        unit = Percent
    )]
    pub zoom: f32,

    /// The turn the outgoing picture reaches, degrees.
    #[dial(label = "Spin", default = 0.0, step = 15.0)]
    pub spin: f32,

    /// How much wider along Direction the pictures are at the middle, per
    /// cent.
    #[slider(
        min = 100.0,
        max = 800.0,
        default = 100.0,
        hard_min = 1.0,
        unit = Percent
    )]
    pub stretch: f32,

    /// How the move gathers speed.
    #[choice(options = *EASE_OPTIONS, default = 1)]
    pub ease: u32,

    /// How much of the move is smeared, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 4.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub smear: f32,

    /// Taps in the smear.
    #[counter(min = 2, max = 64, default = 16, hard_min = 2, hard_max = 64, unit = Raw)]
    pub samples: i32,

    /// The smear splits red, green and blue.
    #[toggle(label = "Split colours", default = false)]
    pub split: bool,

    /// How wide the fade about the middle is, per cent of the cut. 0 is none.
    #[slider(
        label = "Crossfade",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub cross: f32,

    /// This picture fades. On for the upper picture only, so the two always
    /// add up to a whole one.
    #[toggle(default = true)]
    pub fade: bool,

    /// What shows where a picture has moved away from.
    #[choice(label = "Edges", options = *EDGE_OPTIONS, default = 0)]
    pub edge: u32,

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

/// A tap that leaves the picture where it is.
const STILL: ([f32; 4], [f32; 2], u32) = ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0], 7);

/// What both render paths read: the taps to average, then an opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovePacked {
    /// Inverse affines `(m, off, channels)`, sampling at `q = m·p + off`.
    /// Only the first `count` are live.
    pub taps: [([f32; 4], [f32; 2], u32); SHAKE_MB_SAMPLES],
    /// `1..=SHAKE_MB_SAMPLES`.
    pub count: usize,
    /// The [`EDGE_OPTIONS`] code.
    pub edge: u32,
    /// 0..1, multiplied into all four channels after the average, Mix
    /// already in it.
    pub alpha: f32,
    /// 0..1.
    pub mix: f32,
    /// Nothing to do.
    pub identity: bool,
}

impl MoveTransition {
    /// `t` through the Ease.
    fn eased(self, t: f64) -> f64 {
        match self.ease {
            0 => t,
            2 if t < 0.5 => 16.0 * t.powi(5),
            2 => 1.0 - 16.0 * (1.0 - t).powi(5),
            _ => t * t * (3.0 - 2.0 * t),
        }
    }

    /// The inverse affine of this picture at `t` on a `w`×`h` raster, about
    /// its centre.
    fn pose(self, t: f64, w: f64, h: f64) -> ([f32; 4], [f32; 2]) {
        let incoming = self.side == 1;
        let still = match self.moves {
            1 => incoming,
            2 => !incoming,
            _ => false,
        };
        // How far along its own move: the outgoing picture from 0 to 1 as it
        // leaves, the incoming one from −1 to 0 as it arrives.
        let e = self.eased(t.clamp(0.0, 1.0));
        let k = if incoming { e - 1.0 } else { e };
        if still || k == 0.0 {
            return (STILL.0, STILL.1);
        }
        let (s, c) = f64::from(self.direction).to_radians().sin_cos();
        let reach = f64::from(self.distance) / 100.0 * (c.abs() * w + s.abs() * h);
        let offset = [c * reach * k, s * reach * k];
        let zoom = (f64::from(self.zoom) / 100.0).max(0.01).powf(k);
        let stretch = (f64::from(self.stretch) / 100.0).max(0.01).powf(k.abs());
        let (st, ct) = (f64::from(self.spin) * k).to_radians().sin_cos();
        // F = R(turn)·R(θ)·diag(zoom·stretch, zoom)·R(−θ), so its inverse is
        // R(θ)·diag(a, b)·R(−θ)·R(−turn).
        let (a, b) = (1.0 / (zoom * stretch), 1.0 / zoom);
        let along = [
            a * c * c + b * s * s,
            (a - b) * c * s,
            (a - b) * c * s,
            a * s * s + b * c * c,
        ];
        let m = [
            along[0] * ct - along[1] * st,
            along[0] * st + along[1] * ct,
            along[2] * ct - along[3] * st,
            along[2] * st + along[3] * ct,
        ];
        let (cx, cy) = (w / 2.0, h / 2.0);
        let (px, py) = (cx + offset[0], cy + offset[1]);
        let off = [cx - (m[0] * px + m[1] * py), cy - (m[2] * px + m[3] * py)];
        (m.map(|v| v as f32), off.map(|v| v as f32))
    }

    /// The bundle both render paths read, on a `w`×`h` raster.
    #[must_use]
    pub fn packed(self, w: u32, h: u32) -> MovePacked {
        let t = (f64::from(self.completion) / 100.0).clamp(0.0, 1.0);
        let (w, h) = (f64::from(w), f64::from(h));
        // The sampled shutter, narrowing to nothing at either end.
        let half = (f64::from(self.smear) / 200.0).min(t).min(1.0 - t).max(0.0);
        let count = if half > 0.0 {
            self.samples.clamp(2, SHAKE_MB_SAMPLES as i32) as usize
        } else {
            1
        };
        // A colour needs a tap of its own to split, or it would go dark.
        let split = self.split && count >= 3;
        let mut taps = [STILL; SHAKE_MB_SAMPLES];
        for (i, tap) in taps.iter_mut().take(count).enumerate() {
            let at = match count {
                1 => t,
                n => t - half + 2.0 * half * i as f64 / (n - 1) as f64,
            };
            let (m, off) = self.pose(at, w, h);
            *tap = (m, off, Shaken::tap_channels(i, count, split));
        }
        let cross = f64::from(self.cross) / 100.0;
        let shown = if self.fade && cross > 0.0 {
            let ramp = ((t - 0.5) / cross + 0.5).clamp(0.0, 1.0);
            if self.side == 1 {
                ramp
            } else {
                1.0 - ramp
            }
        } else {
            1.0
        };
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        let alpha = 1.0 - mix * (1.0 - shown as f32);
        let still = taps.iter().take(count).all(|tap| *tap == STILL);
        MovePacked {
            taps,
            count,
            edge: self.edge.min(2),
            alpha,
            mix,
            identity: mix <= 0.0 || (still && alpha >= 1.0),
        }
    }
}

/// Move's looks, which the Transition menu offers too. None keys Completion.
pub const MOVE_LOOKS: &[EffectPreset] = &[
    look("Push left", &[("direction", Float(180.0))]),
    look("Push right", &[("direction", Float(0.0))]),
    look("Push up", &[("direction", Float(270.0))]),
    look("Push down", &[("direction", Float(90.0))]),
    look(
        "Slide left",
        &[("moves", Choice(1)), ("direction", Float(180.0))],
    ),
    look(
        "Slide right",
        &[("moves", Choice(1)), ("direction", Float(0.0))],
    ),
    look(
        "Cover left",
        &[("moves", Choice(2)), ("direction", Float(180.0))],
    ),
    look(
        "Cover right",
        &[("moves", Choice(2)), ("direction", Float(0.0))],
    ),
    look(
        "Whip left",
        &[
            ("direction", Float(180.0)),
            WHIP[0],
            WHIP[1],
            WHIP[2],
            WHIP[3],
            WHIP[4],
        ],
    ),
    look(
        "Whip right",
        &[
            ("direction", Float(0.0)),
            WHIP[0],
            WHIP[1],
            WHIP[2],
            WHIP[3],
            WHIP[4],
        ],
    ),
    look(
        "Whip up",
        &[
            ("direction", Float(270.0)),
            WHIP[0],
            WHIP[1],
            WHIP[2],
            WHIP[3],
            WHIP[4],
        ],
    ),
    look(
        "Whip down",
        &[
            ("direction", Float(90.0)),
            WHIP[0],
            WHIP[1],
            WHIP[2],
            WHIP[3],
            WHIP[4],
        ],
    ),
    look(
        "RGB whip",
        &[
            ("direction", Float(180.0)),
            WHIP[0],
            WHIP[1],
            WHIP[2],
            WHIP[3],
            WHIP[4],
            ("split", Bool(true)),
        ],
    ),
    look(
        "Zoom in",
        &[
            ("zoom", Float(400.0)),
            ZOOM[0],
            ZOOM[1],
            ZOOM[2],
            ZOOM[3],
            ZOOM[4],
            ZOOM[5],
        ],
    ),
    look(
        "Zoom out",
        &[
            ("zoom", Float(25.0)),
            ZOOM[0],
            ZOOM[1],
            ZOOM[2],
            ZOOM[3],
            ZOOM[4],
            ZOOM[5],
        ],
    ),
    look(
        "Spin",
        &[
            ("distance", Float(0.0)),
            ("spin", Float(180.0)),
            ("zoom", Float(150.0)),
            ("ease", Choice(2)),
            ("smear", Float(50.0)),
            ("samples", Float(64.0)),
            ("edge", Choice(2)),
            ("cross", Float(15.0)),
        ],
    ),
    look(
        "Zoom spin",
        &[
            ("distance", Float(0.0)),
            ("spin", Float(90.0)),
            ("zoom", Float(300.0)),
            ("ease", Choice(2)),
            ("smear", Float(50.0)),
            ("samples", Float(64.0)),
            ("edge", Choice(2)),
            ("cross", Float(15.0)),
        ],
    ),
    look(
        "Stretch",
        &[
            ("distance", Float(0.0)),
            ("stretch", Float(600.0)),
            ("direction", Float(0.0)),
            ("ease", Choice(2)),
            ("smear", Float(40.0)),
            ("edge", Choice(2)),
            ("cross", Float(20.0)),
        ],
    ),
    look(
        "Drift",
        &[
            ("distance", Float(8.0)),
            ("smear", Float(0.0)),
            ("edge", Choice(2)),
            ("cross", Float(100.0)),
        ],
    ),
];

const fn look(name: &'static str, values: &'static [(&'static str, PresetValue)]) -> EffectPreset {
    EffectPreset { name, values }
}

/// What every Whip shares beside its Direction.
const WHIP: [(&str, PresetValue); 5] = [
    ("ease", Choice(2)),
    ("smear", Float(60.0)),
    ("samples", Float(64.0)),
    ("edge", Choice(2)),
    ("cross", Float(10.0)),
];

/// What both Zooms share beside their Zoom.
const ZOOM: [(&str, PresetValue); 6] = [
    ("distance", Float(0.0)),
    ("ease", Choice(2)),
    ("smear", Float(50.0)),
    ("samples", Float(64.0)),
    ("edge", Choice(2)),
    ("cross", Float(15.0)),
];

/// Move's behaviour.
pub struct MoveTransitionDef;

impl EffectDef for MoveTransitionDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MoveTransition as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let m = MoveTransition::read(p).packed(w, h);
        if m.identity {
            return;
        }
        let taps = m.taps.get(..m.count).unwrap_or(&[]);
        cpu::transform_average(rgba, w, h, taps, m.edge, [0, 0, w, h], m.mix);
        if m.alpha < 1.0 {
            rgba.iter_mut().for_each(|v| *v *= m.alpha);
        }
    }

    fn presets(&self) -> &'static [EffectPreset] {
        MOVE_LOOKS
    }
}
