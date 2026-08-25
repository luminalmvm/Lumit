//! Particulate (K-446, K-474, K-475): a particle system that is arithmetic
//! rather than history, and a points stream beside its picture.
//!
//! **In plain terms.** Sparks, dust, snow, streaks — many small things born
//! from an emitter, carried about by gravity and wind, fading as they age. It
//! draws them over its input like any effect, and hands the same particles out
//! as **data** on a declared Points socket, so the later family (Connect
//! points, Clone to points, Trail) can build on these particles rather than
//! each inventing its own.
//!
//! **Nothing is remembered between frames.** The maths that decides where a
//! particle is lives in [`crate::fx::points`], and it answers "where is
//! particle 5 000 at frame 500?" without computing frame 499 — which is what
//! makes scrubbing instant and export equal to preview (K-474). This file is
//! the *declaration*: the controls, what they mean, and the reduction from the
//! resolved bag to the numbers those formulas read.
//!
//! **What is here and what is not.** PS1 lands the parameters, the closed
//! forms and the CPU disc reference. The GPU evaluate/compaction/draw passes,
//! the sprite and streak modes, and the schedule's carriage beside the op are
//! PS2 (points-stream.md §5); the Points socket is declared but nothing may
//! wire it until PS3 lands the edge.

use crate::fx::points::{self, Emitter, EmitterShape, Forces, ParticleLook, PointsParams};
use crate::fx::{
    cpu, CurvePoints, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen,
    ParamGroup, Port, PortType, Signature,
};
use lumit_fx_macros::Effect;

/// Particulate's declared **data** output (K-472, K-492): the same particles it
/// draws, offered as a stream. Teal, like every geometry socket, and drawn from
/// the signature rather than from anything Particulate-specific at the seam.
const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// The four kickers particulate.md §2 lists its controls under. None starts
/// closed: every one of them holds a control someone reaches for on the first
/// day.
pub const PARTICULATE_GROUPS: &[ParamGroup] = &[
    group(
        "Emitter",
        &[
            "shape",
            "position_x",
            "position_y",
            "width",
            "height",
            "emitter_angle",
            "mask_path",
            "emit_rate",
            "direction",
            "spread",
            "initial_speed",
            "speed_jitter",
        ],
    ),
    group(
        "Particle",
        &[
            "life",
            "life_jitter",
            "size",
            "size_jitter",
            "size_over_life",
            "opacity_over_life",
            "colour",
            "end_colour",
            "rotation",
            "spin",
            "align_to_motion",
        ],
    ),
    group(
        "Forces",
        &[
            "gravity",
            "wind_x",
            "wind_y",
            "drag",
            "turbulence_amount",
            "turbulence_scale",
            "turbulence_speed",
        ],
    ),
    group(
        "Render",
        &[
            "mode",
            "feather",
            "sprite_layer",
            "streak_length",
            "max_particles",
            "seed",
        ],
    ),
];

/// Each render mode's own control, live only in that mode.
///
/// Wind's dependence on Drag is deliberately **not** here: a row that means
/// nothing while a *number* beside it is zero is not one of the three shapes
/// [`EnabledCond`] carries, and inventing a fourth for one row would be
/// growing the panel's affordance set into a scripting surface. The Wind rows
/// say it in their descriptions instead, which is where particulate.md §2 puts
/// it.
pub const PARTICULATE_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "feather",
        on: "mode",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "sprite_layer",
        on: "mode",
        cond: EnabledCond::ChoiceIs(1),
    },
    EnabledWhen {
        param: "streak_length",
        on: "mode",
        cond: EnabledCond::ChoiceIs(2),
    },
];

/// Particulate's controls (particulate.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "particulate",
    label = "Particulate",
    version = 1,
    category = Generate,
    // A playback-class effect, not a simulation-class one (K-475): the budget
    // is the user's own Max particles dial.
    cost = Moderate,
    // A particle may travel anywhere, so no padding covers it.
    roi = FullFrame,
    premultiplied = true,
    // The layer's local time joins the cache key, the standard rule for a
    // seeded effect (docs/08 §1.3).
    seeded = true,
    groups = PARTICULATE_GROUPS,
    enabled_when = PARTICULATE_ENABLED_WHEN,
)]
pub struct Particulate {
    /// Where particles are born. Area shapes emit uniformly over their
    /// interior, Line along its segment, Mask path along the arc-length
    /// polyline (K-408).
    ///
    /// The option list is [`EmitterShape::OPTIONS`] rather than a second copy
    /// of the words, so the labels and `EmitterShape::from_code` cannot come to
    /// disagree about which index means what.
    #[choice(label = "Shape", options = *EmitterShape::OPTIONS, default = 0)]
    pub shape: u32,

    /// The emitter's centre, px@comp (K-260: point parameters are PIXELS).
    #[slider(label = "Position x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub position_x: f32,

    /// px@comp; see [`position_x`](Self::position_x).
    #[slider(label = "Position y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub position_y: f32,

    /// The emitter's extent across, px@comp. Line reads only this; Point and
    /// Mask path read neither it nor Height.
    #[slider(min = 0.0, max = 2000.0, default = 400.0, hard_min = 0.0, unit = Px)]
    pub width: f32,

    /// The emitter's extent down, px@comp; see [`width`](Self::width).
    #[slider(min = 0.0, max = 2000.0, default = 400.0, hard_min = 0.0, unit = Px)]
    pub height: f32,

    /// Rotates Line, Ellipse and Rectangle about Position.
    #[dial(label = "Emitter angle", default = 0.0)]
    pub emitter_angle: f32,

    /// Which of the layer's masks particles are born along, when Shape is Mask
    /// path (K-408). An empty polyline emits nothing — the documented no-op.
    #[mask_path(label = "Mask path")]
    pub mask_path: bool,

    /// Births per second of layer time. Its integral **is** the birth schedule
    /// (points.rs), which is what makes a keyframed or driven rate an ordinary
    /// control rather than a new mechanism.
    #[slider(min = 0.0, max = 1000.0, default = 150.0, hard_min = 0.0, unit = Raw)]
    pub emit_rate: f32,

    /// Which way particles leave; −90 is up.
    #[dial(label = "Direction", default = -90.0)]
    pub direction: f32,

    /// The cone about Direction. At 360 they leave every way at once.
    #[slider(
        min = 0.0,
        max = 360.0,
        default = 360.0,
        hard_min = 0.0,
        hard_max = 360.0,
        unit = Degrees
    )]
    pub spread: f32,

    /// How fast a particle leaves, px@comp per second.
    #[slider(
        label = "Initial speed",
        min = 0.0,
        max = 2000.0,
        default = 90.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub initial_speed: f32,

    /// Per-particle spread of Initial speed, per cent — from the seed, so it
    /// is the same spread on every machine and in every render.
    #[slider(
        label = "Speed jitter",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub speed_jitter: f32,

    /// How long a particle lasts, seconds.
    #[slider(min = 0.1, max = 10.0, default = 2.0, hard_min = 0.0, unit = Seconds)]
    pub life: f32,

    /// Per-particle spread of Life, per cent.
    #[slider(
        label = "Life jitter",
        min = 0.0,
        max = 100.0,
        default = 30.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub life_jitter: f32,

    /// Diameter at birth, px@comp.
    #[slider(min = 0.0, max = 200.0, default = 4.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    /// Per-particle spread of Size, per cent.
    #[slider(
        label = "Size jitter",
        min = 0.0,
        max = 100.0,
        default = 40.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub size_jitter: f32,

    /// Multiplies Size by normalised age (K-412). Flat by default: a particle
    /// is the size it was born, all its life.
    #[curve(label = "Size over life", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub size_over_life: CurvePoints,

    /// Multiplies opacity by normalised age (K-412). Born solid, dies faded —
    /// which is most of why the default look reads as motes rather than dots.
    #[curve(label = "Opacity over life", default = [[0.0, 1.0], [1.0, 0.0]])]
    pub opacity_over_life: CurvePoints,

    /// The colour at birth. Scene-linear, and values above 1 are legal and
    /// useful over a glow (§2.1).
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// The colour at death, blended to over normalised age in working space.
    #[colour(label = "End colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub end_colour: [f32; 4],

    /// How far round a particle is drawn. Invisible on a disc; it is Sprite and
    /// Streak that read it.
    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// How fast that rotation turns, degrees per second.
    #[slider(min = -720.0, max = 720.0, default = 0.0, unit = Degrees)]
    pub spin: f32,

    /// Rotation follows the direction of travel; Spin adds on top.
    #[toggle(label = "Align to motion", default = false)]
    pub align_to_motion: bool,

    /// px@comp per second², positive down.
    #[slider(min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub gravity: f32,

    /// The air's own speed across, px@comp per second. Wind acts **through**
    /// Drag: with Drag at 0 it does nothing at all.
    #[slider(label = "Wind x", min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub wind_x: f32,

    /// The air's own speed down, px@comp per second; see
    /// [`wind_x`](Self::wind_x).
    #[slider(label = "Wind y", min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub wind_y: f32,

    /// How quickly a particle's speed approaches the wind's, per second.
    #[slider(min = 0.0, max = 10.0, default = 0.5, hard_min = 0.0, unit = Raw)]
    pub drag: f32,

    /// How far the noise pushes a particle off its path, px@comp.
    #[slider(
        label = "Turbulence amount",
        min = 0.0,
        max = 500.0,
        default = 40.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub turbulence_amount: f32,

    /// The noise's spatial wavelength, px@comp: how far apart two particles
    /// have to be born before they are pushed different ways.
    #[slider(
        label = "Turbulence scale",
        min = 10.0,
        max = 1000.0,
        default = 200.0,
        hard_min = 10.0,
        unit = Px
    )]
    pub turbulence_scale: f32,

    /// How fast the noise evolves against a particle's age, Hz.
    #[slider(
        label = "Turbulence speed",
        min = 0.0,
        max = 5.0,
        default = 0.3,
        hard_min = 0.0,
        unit = Raw
    )]
    pub turbulence_speed: f32,

    /// What a particle is drawn as. **Disc** is the reference mode; Sprite and
    /// Streak land with the instanced draw (PS2).
    #[choice(label = "Mode", options = ["Disc", "Sprite", "Streak"], default = 0)]
    pub mode: u32,

    /// How soft a disc's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The layer drawn per particle in Sprite mode (K-123, K-142). **Unset
    /// draws discs** — a render mode must always draw something, which is this
    /// effect's documented deviation from the unset-is-identity convention.
    #[layer(label = "Sprite layer")]
    pub sprite_layer: bool,

    /// How long a streak's tail is, seconds: the line runs from `p(t − length)`
    /// to `p(t)`, which is the closed form again and needs no history.
    #[slider(
        label = "Streak length",
        min = 0.0,
        max = 0.1,
        default = 0.02,
        hard_min = 0.0,
        unit = Seconds
    )]
    pub streak_length: f32,

    /// **The budget dial** (K-475): the most particles that may be live at
    /// once, and the peak scratch the governor grants against. Over budget the
    /// newest survive. Deliberately **not animatable** — it is a capacity
    /// declaration, like the flare's ray budget (K-265), and animating a
    /// capacity would re-key the governor every frame.
    #[counter(
        label = "Max particles",
        min = 1,
        max = 200_000,
        default = points::CAP_DEFAULT,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_particles: i32,

    /// Which particles (§2.4). The reseed button rolls it.
    #[seed]
    pub seed: u32,

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

impl Particulate {
    /// The bundle the closed forms read (docs/impl/effect-registry.md §2.4) —
    /// this effect's `packed`.
    ///
    /// Every distance here is in whatever the bag was resolved at: px@comp for
    /// a stream read as data, raster pixels for a stream about to be drawn.
    /// The resolve step's spatial rescale does that conversion generically, so
    /// nothing in this file has to know which it is (docs/08 §2.3).
    #[must_use]
    pub fn points(self) -> PointsParams {
        PointsParams {
            emitter: Emitter {
                shape: EmitterShape::from_code(self.shape),
                position: [self.position_x, self.position_y],
                width: self.width.max(0.0),
                height: self.height.max(0.0),
                angle_deg: self.emitter_angle,
                direction_deg: self.direction,
                spread_deg: self.spread.clamp(0.0, 360.0),
                speed: self.initial_speed,
                speed_jitter: (self.speed_jitter / 100.0).clamp(0.0, 1.0),
            },
            particle: ParticleLook {
                life: self.life.max(0.0),
                life_jitter: (self.life_jitter / 100.0).clamp(0.0, 1.0),
                size: self.size.max(0.0),
                size_jitter: (self.size_jitter / 100.0).clamp(0.0, 1.0),
                size_curve: cpu::curve_table(&self.size_over_life),
                opacity_curve: cpu::curve_table(&self.opacity_over_life),
                colour: self.colour,
                end_colour: self.end_colour,
                rotation_deg: self.rotation,
                spin_deg: self.spin,
                align_to_motion: self.align_to_motion,
            },
            forces: Forces {
                gravity: self.gravity,
                wind: [self.wind_x, self.wind_y],
                drag: self.drag.max(0.0),
                turbulence: self.turbulence_amount.max(0.0),
                turbulence_scale: self.turbulence_scale.max(1.0),
                turbulence_speed: self.turbulence_speed.max(0.0),
            },
            cap: self.max_particles.clamp(1, points::CAP_HARD as i32) as u32,
            seed: self.seed,
        }
    }

    /// How many frames back a particle born then could still be alive now —
    /// the window [`points::Schedule::scan`] records births over.
    ///
    /// The longest life any particle can draw is Life plus its jitter, and the
    /// jitter is a per cent either way, so the ceiling is `life × (1 + jitter)`
    /// exactly. A frame more, because the frame the window opens on is a frame
    /// whose births are partly inside it.
    #[must_use]
    pub fn window_frames(self, dt: f64) -> i64 {
        let jitter = (self.life_jitter / 100.0).clamp(0.0, 1.0);
        let longest = f64::from(self.life.max(0.0) * (1.0 + jitter));
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 1.0 };
        ((longest / dt).ceil() as i64).saturating_add(1)
    }
}

/// Particulate's behaviour.
///
/// **No CPU reference through the trait**, the same shape Scribble and Stroke
/// have and for the same reason: what this effect draws is decided by the
/// birth schedule and the layer's own timing, neither of which is a parameter,
/// so neither is in the bag [`apply_cpu`](EffectDef::apply_cpu) is handed. The
/// schedule rides beside the op the way a mask's polyline does, and threading
/// it is PS2's work along with the GPU passes. The §1.6 oracle is
/// [`points::evaluate`] with [`points::draw_discs`], exercised directly from
/// the test suite — which is also exactly what the Points sample driver will
/// read (points-stream.md §3.1).
pub struct ParticulateDef;

impl EffectDef for ParticulateDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Particulate as EffectMetadata>::SCHEMA
    }

    /// The picture *and* the data (K-472): a stack effect that declares an
    /// output beside its image, which is the first of its kind.
    fn signature(&self) -> Signature {
        Signature::Image { extra: POINTS_OUT }
    }
}
