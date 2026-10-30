//! Ring wave: rings that start at random places, grow outwards and give each
//! copy a value as they cross it, with a turn, a count of the rings that have
//! passed and how much they make it grow. One of the Siceliot drivers, as
//! [`super::siceliot`].

use super::siceliot::{between, VALUE_PORT};
use crate::fx::{
    params::Value, points, registry::DriverCx, registry::EffectDef, registry::EffectMetadata,
    registry::Signature, schema::EffectSchema, schema::EnabledCond, schema::EnabledWhen,
    schema::Port, schema::PortType,
};
use lumit_fx_macros::Effect;

/// Where RingWave's rings start.
pub const RING_WAVE_ORIGINS: &[&str] = &["Anywhere", "Edges", "Centre", "Away from points"];

/// The turn the rings give what they cross, degrees.
pub const ROTATION_PORT: &str = "rotation";

/// How many rings have crossed a point so far.
pub const PASSES_PORT: &str = "passes";

/// How much bigger the rings make what they cross, per cent: each ring's
/// own Grow, by how strong it is there, added up.
pub const GROW_PORT: &str = "grow";

const RING_WAVE_OUT: &[Port] = &[
    Port::new(VALUE_PORT, "Value", PortType::Number),
    Port::new(ROTATION_PORT, "Rotation", PortType::Number),
    Port::new(PASSES_PORT, "Passes", PortType::Number),
    Port::new(GROW_PORT, "Grow", PortType::Number),
];

/// Clearance is read only on Away from points.
pub const RING_WAVE_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "clearance",
    on: "origin",
    cond: EnabledCond::ChoiceIs(3),
}];

/// The dice a ring rolls for where it starts, how fast and thick it is and
/// how it turns things.
const RING_WAVE_ATTR: u32 = 120;

/// Ring wave's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "ring_wave",
    label = "Ring wave",
    version = 1,
    category = SiceliotDrivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
    enabled_when = RING_WAVE_ENABLED_WHEN,
)]
pub struct RingWave {
    /// The value where no ring is.
    #[slider(label = "Minimum", min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub minimum: f32,

    /// The value in the middle of a ring.
    #[slider(label = "Maximum", min = -100.0, max = 100.0, default = 100.0, unit = Raw)]
    pub maximum: f32,

    /// Seconds between one ring and the next.
    #[slider(
        label = "Interval",
        min = 0.05,
        max = 5.0,
        default = 0.5,
        hard_min = 0.01,
        unit = Raw
    )]
    pub interval: f32,

    /// How fast a ring grows, px@comp a second.
    #[slider(
        label = "Speed",
        min = 10.0,
        max = 2000.0,
        default = 300.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub speed: f32,

    /// How much faster or slower each ring may be.
    #[slider(
        label = "Speed variance",
        min = 0.0,
        max = 500.0,
        default = 10.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub speed_variance: f32,

    /// The thinnest a ring can be, px@comp: how far from its middle it
    /// fades to nothing.
    #[slider(
        label = "Thickness min",
        min = 1.0,
        max = 1000.0,
        default = 60.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub thickness_min: f32,

    /// The thickest.
    #[slider(
        label = "Thickness max",
        min = 1.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub thickness_max: f32,

    /// Where the rings start: anywhere on the frame, on its edges, all in its
    /// centre, or anywhere at least Clearance away from the points the driver
    /// is wired to, so a ring never starts inside the shape they fill.
    #[choice(label = "Origin", options = RING_WAVE_ORIGINS, default = 0)]
    pub origin: u32,

    /// How far from every point a ring starts, px@comp, with Origin on Away
    /// from points.
    #[slider(
        label = "Clearance",
        min = 0.0,
        max = 400.0,
        default = 20.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub clearance: f32,

    /// Let the strengths of rings that overlap add up past Maximum, as the
    /// rings of the Processing sketch do, instead of stopping there.
    #[toggle(label = "Add up", default = false)]
    pub add_up: bool,

    /// The least a ring makes what it crosses grow, per cent, read on the
    /// Grow port. Each ring has its own, between this and Grow max.
    #[slider(
        label = "Grow min",
        min = 0.0,
        max = 200.0,
        default = 20.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub grow_min: f32,

    /// The most.
    #[slider(
        label = "Grow max",
        min = 0.0,
        max = 200.0,
        default = 50.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub grow_max: f32,

    /// How much the rings turn what they cross, read on the Rotation port.
    #[slider(
        label = "Spin",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub spin: f32,

    /// Where, how fast and how thick each ring is.
    #[seed]
    pub seed: u32,
}

/// Ring wave's behaviour.
pub struct RingWaveDef;

impl EffectDef for RingWaveDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RingWave as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: RING_WAVE_OUT,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = RingWave::read(cx.params);
        let context = &cx.context;
        #[allow(clippy::cast_precision_loss)]
        let frame = context
            .comp
            .and_then(|id| context.document.comp(id))
            .map_or([1920.0, 1080.0], |c| [c.width as f32, c.height as f32]);
        let at = ring_wave(&p, cx.point.unwrap_or([0.0; 2]), frame, cx.lt, cx.cloud);
        push(VALUE_PORT, Value::Float(at.value));
        push(ROTATION_PORT, Value::Float(at.rotation));
        push(PASSES_PORT, Value::Float(at.passes));
        push(GROW_PORT, Value::Float(at.grow));
    }
}

/// What Ring wave reads at one point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RingWaveAt {
    /// Minimum where no ring is, Maximum in the middle of one.
    pub value: f32,
    /// The turn the rings give, degrees, clockwise on the frame.
    pub rotation: f32,
    /// How many rings have crossed the point: each counts once its middle
    /// is near, and for good.
    pub passes: f32,
    /// How much bigger the rings make the point, per cent.
    pub grow: f32,
}

/// One ring: where it starts, how fast it grows, how thick it is, the turn
/// it gives (a direction offset and how fast that offset spins) and how
/// much it makes things grow, 0 to 1.
struct Ring {
    origin: [f32; 2],
    speed: f32,
    thickness: f32,
    offset: f32,
    spin_rate: f32,
    grow: f32,
}

/// The most places Away from points tries before it takes the last.
const AWAY_TRIES: u32 = 64;

fn ring(
    p: &RingWave,
    k: u64,
    frame: [f32; 2],
    thin: f32,
    thick: f32,
    cloud: Option<&points::Cloud>,
) -> Ring {
    let roll = |n: u32| points::draw(p.seed, k, RING_WAVE_ATTR + n);
    let [w, h] = frame;
    let origin = match p.origin {
        // A place along the frame's edge, clockwise from the top left.
        1 => {
            let u = roll(0) * 2.0 * (w + h);
            if u < w {
                [u, 0.0]
            } else if u < w + h {
                [w, u - w]
            } else if u < 2.0 * w + h {
                [2.0 * w + h - u, h]
            } else {
                [0.0, 2.0 * (w + h) - u]
            }
        }
        2 => [w * 0.5, h * 0.5],
        // Places on the frame in turn until one is clear of the points, as
        // the sketch's do-while does.
        3 => {
            let place = |t: u32| [roll(20 + 2 * t) * w, roll(21 + 2 * t) * h];
            (0..AWAY_TRIES)
                .map(place)
                .find(|at| cloud.is_none_or(|c| !c.near(*at, p.clearance)))
                .unwrap_or_else(|| place(AWAY_TRIES - 1))
        }
        _ => [roll(0) * w, roll(1) * h],
    };
    Ring {
        origin,
        speed: (p.speed + p.speed_variance * (2.0 * roll(2) - 1.0)).max(1.0),
        thickness: thin + (thick - thin) * roll(3),
        offset: roll(4) * std::f32::consts::PI,
        spin_rate: (2.0 * roll(5) - 1.0) * std::f32::consts::FRAC_PI_4,
        grow: (p.grow_min + (p.grow_max - p.grow_min) * roll(6)) / 100.0,
    }
}

/// Ring wave at `at` (px@comp) on a frame of `frame` px, at layer time `lt`,
/// with `cloud` the points it keeps its rings away from on Away from points.
///
/// Ring `k` starts at `k` Intervals and grows at its speed; at a point its
/// strength is 1 on its middle line and fades to 0 a Thickness away. The
/// strengths of the rings on a point add up, to 1 at most unless Add up is
/// on. A ring lives until it has crossed the whole frame, so a ring gone has
/// passed every point on it, which is what keeps Passes counting without a
/// memory.
#[must_use]
pub fn ring_wave(
    p: &RingWave,
    at: [f32; 2],
    frame: [f32; 2],
    lt: f64,
    cloud: Option<&points::Cloud>,
) -> RingWaveAt {
    let interval = f64::from(p.interval.max(0.01));
    let lt = lt.max(0.0);
    let thick = p.thickness_max.max(p.thickness_min).max(1.0);
    let thin = p.thickness_min.min(p.thickness_max).max(1.0);
    let reach = frame[0].hypot(frame[1]) + thick;
    let slowest = (p.speed - p.speed_variance).max(1.0);
    let life = f64::from(reach / slowest);
    #[allow(clippy::cast_possible_truncation)]
    let newest = (lt / interval).floor() as i64;
    #[allow(clippy::cast_possible_truncation)]
    let oldest = ((lt - life) / interval).ceil().max(0.0) as i64;
    #[allow(clippy::cast_precision_loss)]
    let mut passes = oldest as f32;
    let (mut strength, mut turn, mut grow) = (0.0f32, 0.0f32, 0.0f32);
    for k in oldest..=newest {
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let age = (lt - k as f64 * interval) as f32;
        #[allow(clippy::cast_sign_loss)]
        let r = ring(p, k as u64, frame, thin, thick, cloud);
        let radius = age * r.speed;
        if radius > reach {
            passes += 1.0;
            continue;
        }
        let (dx, dy) = (at[0] - r.origin[0], at[1] - r.origin[1]);
        let d = dx.hypot(dy);
        if radius >= d - 0.2 * r.thickness {
            passes += 1.0;
        }
        let s = (1.0 - (d - radius).abs() / r.thickness).max(0.0);
        if s > 0.0 {
            strength += s;
            grow += s * r.grow;
            // The way out from the ring's start, turned by the ring's own
            // offset and spin, and given more the nearer its middle.
            let way = dy.atan2(dx) + r.offset + r.spin_rate * age;
            turn += way * 0.5 * s * s;
        }
    }
    let strength = if p.add_up {
        strength
    } else {
        strength.min(1.0)
    };
    RingWaveAt {
        value: between(p.minimum, p.maximum, strength),
        rotation: turn.to_degrees() * p.spin / 100.0,
        passes,
        grow: grow * 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RingWave's rings: the value peaks where a ring's middle is and rests at
    /// Minimum elsewhere, Passes only ever grows and counts a ring once its
    /// middle is near, and the turn is nought away from the rings. Guards
    /// the decode look's waves, and the text a wave leaves behind.
    #[test]
    fn ring_wave_rings_grow_and_count_their_passes() {
        let p = RingWave {
            minimum: 0.0,
            maximum: 100.0,
            interval: 0.5,
            speed: 300.0,
            speed_variance: 0.0,
            thickness_min: 60.0,
            thickness_max: 60.0,
            origin: 2,
            clearance: 20.0,
            add_up: false,
            grow_min: 30.0,
            grow_max: 30.0,
            spin: 100.0,
            seed: 3,
        };
        let frame = [1000.0, 1000.0];
        let point = [800.0, 500.0];
        let at = |lt: f64| ring_wave(&p, point, frame, lt, None);
        // The first ring reaches 300 px at one second.
        assert!((at(1.0).value - 100.0).abs() < 1e-3, "{:?}", at(1.0));
        assert!((at(0.9).value - 50.0).abs() < 1e-2, "half way in");
        assert_eq!(at(0.3).value, 0.0, "no ring yet");
        assert_eq!(at(0.3).rotation, 0.0, "no turn without a ring");
        assert_eq!(at(0.5).passes, 0.0);
        assert_eq!(at(1.0).passes, 1.0);
        let mut last = 0.0;
        for f in 0..600 {
            let now = at(f64::from(f) / 24.0);
            assert!((0.0..=100.0).contains(&now.value));
            assert!(now.passes >= last, "passes went back at frame {f}");
            last = now.passes;
        }
        assert!(last > 40.0, "a ring every half second: {last}");
        // Rings from anywhere fall on the frame, and the seed moves them.
        let anywhere = RingWave { origin: 0, ..p };
        let other = RingWave {
            seed: 4,
            ..anywhere
        };
        let times = (0..96).map(|f| f64::from(f) / 24.0);
        assert!(
            times
                .clone()
                .any(|lt| ring_wave(&anywhere, point, frame, lt, None)
                    != ring_wave(&other, point, frame, lt, None)),
            "the seed moves the rings"
        );
        assert!(
            times
                .clone()
                .any(|lt| ring_wave(&anywhere, point, frame, lt, None).rotation != 0.0),
            "a ring crossing turns the point"
        );
        // Grow: a ring's own share, by its strength.
        assert!((at(1.0).grow - 30.0).abs() < 1e-2, "{:?}", at(1.0));
        assert!((at(0.9).grow - 15.0).abs() < 0.1);
        assert_eq!(at(0.3).grow, 0.0);
        // Add up: two rings on one point go past Maximum only when asked.
        let busy = RingWave {
            interval: 0.05,
            thickness_min: 400.0,
            thickness_max: 400.0,
            ..p
        };
        assert!((ring_wave(&busy, point, frame, 1.0, None).value - 100.0).abs() < 1e-3);
        let added = RingWave {
            add_up: true,
            ..busy
        };
        assert!(ring_wave(&added, point, frame, 1.0, None).value > 150.0);
        // Away from points: no ring starts within Clearance of a point. A
        // block of points fills the left half of the frame.
        let block: Vec<[f32; 2]> = (0..25)
            .flat_map(|x| (0..50).map(move |y| [x as f32 * 20.0, y as f32 * 20.0]))
            .collect();
        let cloud = points::Cloud::new(block);
        let away = RingWave {
            origin: 3,
            clearance: 30.0,
            ..p
        };
        for k in 0..64 {
            let r = ring(&away, k, frame, 60.0, 60.0, Some(&cloud));
            assert!(r.origin[0] >= 500.0, "ring {k} starts at {:?}", r.origin);
        }
        assert!(cloud.near([105.0, 105.0], 10.0) && !cloud.near([600.0, 100.0], 30.0));
    }
}
