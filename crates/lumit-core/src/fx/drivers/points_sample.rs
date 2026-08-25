//! Points sample (K-492, K-494, points-stream.md §2.2): the first thing
//! besides Particulate's own drawing that consumes a points stream.
//!
//! **In plain terms.** Wire Particulate's teal Points socket into this driver
//! and it turns the particles into two numbers: how many are alive right now,
//! and how far the nearest one is from a place you choose. Wire those numbers
//! at a parameter — through a Remap, usually — and "the glow brightens when a
//! spark passes the lamp" is two wires and no new idea.
//!
//! **The stream it reads is the stream the picture draws.** The numbers come
//! out of the shared closed-form module ([`crate::fx::points`]) evaluated with
//! the producer's *fully driven* parameters, which is the one property this
//! driver has to keep: a count of particles the viewer cannot see would be a
//! lie the frame key could not even name.
//!
//! **Nothing wired reads as an empty stream**, which is the documented no-op:
//! Count is nought and Nearest distance is [`NOTHING_NEAR`] — "nothing is
//! anywhere near", the honest direction for the wire's usual shape, where a
//! Remap turns nearness into a value.

use crate::fx::points::PointsStream;
use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input: no stored value, nothing to keyframe, no panel
/// row (points-stream.md §4.1). A points stream has no number to fall back on,
/// which is the whole reason it is a signature port rather than a parameter.
pub const POINTS_PORT: &str = "points";

/// How many particles are live this frame.
pub const COUNT_PORT: &str = "count";

/// px@comp from Position to the nearest live particle.
pub const NEAREST_PORT: &str = "nearest_distance";

/// What Nearest distance answers over an empty stream — an unwired socket, a
/// bypassed producer, a frame before the first birth.
///
/// A large number rather than nought, and the difference matters: a Remap from
/// nearness to a value reads nought as "a particle is right here", which would
/// make an unwired driver fire everything at once. Pinned by test.
pub const NOTHING_NEAR: f32 = 1e9;

/// The stream this driver reads, and the outputs it makes of it.
const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];
const OUTPUTS: &[Port] = &[
    Port::new(COUNT_PORT, "Count", PortType::Number),
    Port::new(NEAREST_PORT, "Nearest distance", PortType::Number),
];

/// Points sample's one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "points_sample",
    label = "Points sample",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct PointsSample {
    /// The query point, px@comp (K-260 — point parameters are pixels). The
    /// schema default is the nominal 1080p centre; `instantiate_for_raster`
    /// centres a fresh node on the actual comp, because a query point in the
    /// top-left corner of a 4K frame is one somebody has to go and find.
    #[slider(label = "Position x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub position_x: f32,

    /// px@comp; see [`position_x`](Self::position_x).
    #[slider(label = "Position y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub position_y: f32,
}

/// Points sample's behaviour.
pub struct PointsSampleDef;

impl EffectDef for PointsSampleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PointsSample as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    /// The first driver to declare a **data input** (points-stream.md §4.1).
    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: POINTS_IN,
            outputs: OUTPUTS,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = PointsSample::read(cx.params);
        let stream = (cx.points_input)(POINTS_PORT);
        let (count, nearest) = sample(stream.as_deref(), [p.position_x, p.position_y]);
        push(COUNT_PORT, Value::Float(count));
        push(NEAREST_PORT, Value::Float(nearest));
    }

    // `driver_window` stays at its default nought: this driver is pointwise —
    // the stream at the frame is all it reads (points-stream.md §2.2).
}

/// The two numbers, from a stream and a query point.
///
/// The search is a linear scan over the live set, bounded by the producer's
/// own Max particles (K-475) and deterministic because the stream is ordered by
/// birth index rather than by anything a scheduler decided.
///
/// (ponytail: O(n) scan. A grid or a kd-tree only if a profile ever shows a
/// real graph spending it — at the default cap of 20 000 this is a few hundred
/// microseconds of straight-line arithmetic.)
#[must_use]
pub fn sample(stream: Option<&PointsStream>, at: [f32; 2]) -> (f32, f32) {
    let Some(s) = stream else {
        return (0.0, NOTHING_NEAR);
    };
    // Squared while scanning, rooted once: the same minimum, one square root.
    let mut nearest_sq = f32::INFINITY;
    for p in &s.position {
        let (dx, dy) = (p[0] - at[0], p[1] - at[1]);
        let d = dx.mul_add(dx, dy * dy);
        if d < nearest_sq {
            nearest_sq = d;
        }
    }
    let nearest = if nearest_sq.is_finite() {
        nearest_sq.sqrt()
    } else {
        NOTHING_NEAR
    };
    (s.len() as f32, nearest)
}
