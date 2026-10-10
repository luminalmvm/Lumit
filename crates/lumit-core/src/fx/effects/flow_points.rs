//! Flow points carries a stream's points along a field over time.
//!
//! The field says which way a point at any place is moving and how fast: a
//! swirling noise, a vortex, a pull towards a centre, a wind, or several of
//! them added together. A point's place at a time is found by walking it
//! through the field from where the wire has it, for that long, in fixed
//! steps, so a frame is the same however it is reached. The steps a walk
//! shares with an earlier frame's are kept and not taken again, which
//! changes how long a frame takes and never what it is. Each point's speed
//! becomes the way the field is carrying it. The moved stream goes out on
//! the Points socket, and is drawn as discs unless Mix is 0. Nothing wired
//! draws nothing.

use crate::fx::cpu;
use crate::fx::effects::vary_points::{
    APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN, POINTS_IN, POINTS_OUT,
};
use crate::fx::noise::perlin3;
use crate::fx::points::PointsStream;
use crate::fx::{
    CurvePoints, EffectDef, EffectMetadata, EffectSchema, EnabledWhen, ParamGroup, ParamId, Params,
    ResolveCx, ShortText, Signature, Value,
};
use lumit_fx_macros::Effect;
use std::sync::Mutex;

/// The most steps one frame may walk, over all its points.
const MAX_STEPS: usize = 1_000_000;

/// The most steps a second, whatever is typed.
const MAX_RATE: i64 = 240;

/// The most walks kept from earlier frames, and the most points over all of
/// them. A kept point is 24 bytes, so this is under 10 MB.
const KEPT_WALKS: usize = 16;
const KEPT_POINTS: usize = 400_000;

/// A walk as far as another frame can use it: where each carried point
/// stood after `steps` whole steps.
struct Walked {
    /// Every number the walk was made from: the rows it reads, then each
    /// carried point's id and where it started. Kept whole and compared
    /// exactly, so two different walks can never be taken for one another.
    key: Vec<u32>,
    steps: usize,
    at: Vec<[f32; 2]>,
}

/// Walks kept from earlier frames, the latest used first. Only ever a
/// shortcut: a frame walked from one is the frame walked from the start.
static WALKS: Mutex<Vec<Walked>> = Mutex::new(Vec::new());

/// The furthest kept walk of `key` that stops at or before `keep` steps:
/// how many steps it took, and where they left each point. It becomes the
/// latest used.
fn kept_walk(key: &[u32], keep: usize) -> Option<(usize, Vec<[f32; 2]>)> {
    let mut walks = WALKS.lock().ok()?;
    let found = walks
        .iter()
        .enumerate()
        .filter(|(_, w)| w.steps <= keep && w.key == key)
        .max_by_key(|(_, w)| w.steps)
        .map(|(n, _)| n)?;
    let walk = walks.remove(found);
    let out = (walk.steps, walk.at.clone());
    walks.insert(0, walk);
    Some(out)
}

/// Keep a walk for the frames after this one, and drop the walks used
/// longest ago once there are too many or they hold too many points.
fn keep_walk(walk: Walked) {
    let Ok(mut walks) = WALKS.lock() else {
        return;
    };
    if walks
        .iter()
        .any(|w| w.steps == walk.steps && w.key == walk.key)
    {
        return;
    }
    walks.insert(0, walk);
    walks.truncate(KEPT_WALKS);
    while walks.iter().map(|w| w.at.len()).sum::<usize>() > KEPT_POINTS {
        walks.pop();
    }
}

/// The lattice the curl noise reads, kept off the ones the patterns and the
/// producers use.
const CURL_CHANNEL: u32 = 96;

/// How far either side of a place the noise is read to find its slope, as a
/// share of Noise scale.
const CURL_REACH: f32 = 0.01;

/// The Field options, in code order. A Choice is stored as its index, so a
/// new field goes on the end.
const CURL: u32 = 0;
const VORTEX: u32 = 1;
const PULL: u32 = 2;
const WIND: u32 = 3;
const COMBINED: u32 = 4;

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    visible_when: Option<(&'static str, &'static [u32])>,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// The field, the rows only some fields read, and the walk.
pub const FLOW_GROUPS: &[ParamGroup] = &[
    group("Field", &["field", "speed"], None),
    group(
        "",
        &["curl", "vortex", "pull", "wind"],
        Some(("field", &[COMBINED])),
    ),
    group(
        "",
        &["noise_scale", "noise_speed", "seed"],
        Some(("field", &[CURL, COMBINED])),
    ),
    group(
        "",
        &["centre_x", "centre_y", "radius", "falloff"],
        Some(("field", &[VORTEX, PULL, COMBINED])),
    ),
    group("", &["direction"], Some(("field", &[WIND, COMBINED]))),
    group("Time", &["clock", "steps"], None),
    group("Point", &["feather"], None),
];

pub const FLOW_ENABLED_WHEN: &[EnabledWhen] = &[APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN];

/// Flow points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "flow_points",
    label = "Flow points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be carried anywhere.
    roi = FullFrame,
    premultiplied = true,
    // Seeded, since the points move under constant parameters.
    seeded = true,
    groups = FLOW_GROUPS,
    enabled_when = FLOW_ENABLED_WHEN,
)]
pub struct FlowPoints {
    /// Which points are carried. The rest stay where they are.
    #[choice(
        label = "Apply to",
        options = ["All points", "Picked", "Not picked"],
        default = 0
    )]
    pub apply_to: u32,

    /// The group Picked and Not picked go by: a name a Pick points or a Vary
    /// points above wrote, or one of the `@` names. Empty is the points a
    /// Pick points picked.
    #[text(label = "Group", default = "")]
    pub apply_group: ShortText,

    /// What a point's Group has to read above to be in it. Only read with a
    /// Group named.
    #[slider(label = "Threshold", min = 0.0, max = 1.0, default = 0.5, unit = Raw)]
    pub apply_threshold: f32,

    /// What carries the points. Curl noise swirls them without bunching
    /// them up, Vortex turns them round Centre, Pull draws them to it, Wind
    /// blows them one way, and Combined adds the four together, each by its
    /// own share.
    #[choice(
        label = "Field",
        options = ["Curl noise", "Vortex", "Pull", "Wind", "Combined"],
        default = 0
    )]
    pub field: u32,

    /// How fast the field carries a point where it is strongest, px@comp a
    /// second. A negative number runs it backwards: a Vortex turns
    /// anticlockwise and a Pull pushes away.
    #[slider(min = -500.0, max = 500.0, default = 100.0, unit = Px)]
    pub speed: f32,

    /// How much of Speed the Curl noise carries a point at when the Field is
    /// Combined, per cent. 0 leaves that field out, and a negative number
    /// runs it backwards.
    #[slider(
        label = "Curl noise",
        min = -200.0,
        max = 200.0,
        default = 0.0,
        unit = Percent
    )]
    pub curl: f32,

    /// Per cent, for the Vortex. See [`curl`](Self::curl).
    #[slider(
        label = "Vortex",
        min = -200.0,
        max = 200.0,
        default = 100.0,
        unit = Percent
    )]
    pub vortex: f32,

    /// Per cent, for the Pull. See [`curl`](Self::curl).
    #[slider(
        label = "Pull",
        min = -200.0,
        max = 200.0,
        default = 100.0,
        unit = Percent
    )]
    pub pull: f32,

    /// Per cent, for the Wind. See [`curl`](Self::curl).
    #[slider(
        label = "Wind",
        min = -200.0,
        max = 200.0,
        default = 0.0,
        unit = Percent
    )]
    pub wind: f32,

    /// How big the swirls are, px@comp.
    #[slider(
        label = "Noise scale",
        min = 10.0,
        max = 1000.0,
        default = 200.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub noise_scale: f32,

    /// How fast the swirls themselves change, per second. 0 holds them still.
    #[slider(
        label = "Noise speed",
        min = 0.0,
        max = 5.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub noise_speed: f32,

    /// Which noise.
    #[seed]
    pub seed: u32,

    /// The middle of a Vortex or a Pull, px@comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp. See [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// How far from Centre the field reaches, px@comp.
    #[slider(
        label = "Radius",
        min = 0.0,
        max = 2000.0,
        default = 400.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// How strong the field is from Centre, at the left, out to Radius, at
    /// the right. Past Radius it stays as strong as it is there.
    #[curve(label = "Falloff", default = [[0.0, 1.0], [1.0, 0.0]])]
    pub falloff: CurvePoints,

    /// The way the Wind blows, degrees. 0 is to the right and 90 is down.
    #[dial(label = "Direction", default = 0.0)]
    pub direction: f32,

    /// How long each point has been carried for. Layer time is the layer's
    /// own time, right for points that stand still. Point age is each
    /// point's own age, so a newborn particle starts where it was born.
    #[choice(label = "Clock", options = ["Layer time", "Point age"], default = 0)]
    pub clock: u32,

    /// How finely the walk is cut. More follows a tight field better and
    /// costs more.
    #[counter(
        label = "Steps per second",
        min = 1,
        max = 120,
        default = 30,
        hard_min = 1,
        hard_max = MAX_RATE,
        unit = Raw
    )]
    pub steps: i32,

    /// How soft the disc a point is drawn as is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

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

impl FlowPoints {
    /// The raster factor, since a stream off a wire arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// How fast and which way a point at `at` is carried at `time`, in the
    /// stream's units a second. `dt` is the step about to be taken, so a
    /// Pull can stop at Centre and not step past it.
    fn carried(
        self,
        at: [f32; 2],
        time: f32,
        dt: f32,
        falloff: &[f32; cpu::CURVE_TABLE],
    ) -> [f32; 2] {
        if self.field != COMBINED {
            return self.one_field(self.field, self.speed, at, time, dt, falloff);
        }
        // Each field's share of Speed, added up. One with no share is not
        // worked out.
        let shares = [
            (CURL, self.curl),
            (VORTEX, self.vortex),
            (PULL, self.pull),
            (WIND, self.wind),
        ];
        let mut sum = [0.0f32; 2];
        for (field, share) in shares {
            if share != 0.0 {
                let speed = self.speed * share / 100.0;
                let v = self.one_field(field, speed, at, time, dt, falloff);
                sum = [sum[0] + v[0], sum[1] + v[1]];
            }
        }
        sum
    }

    /// What one of the four fields does to a point at `at`, at its strongest
    /// carrying it at `speed`.
    fn one_field(
        self,
        field: u32,
        speed: f32,
        at: [f32; 2],
        time: f32,
        dt: f32,
        falloff: &[f32; cpu::CURVE_TABLE],
    ) -> [f32; 2] {
        // Out from Centre: how far, and which way.
        let out = [at[0] - self.centre_x, at[1] - self.centre_y];
        let far = out[0].hypot(out[1]);
        let fade = || cpu::curve_at((far / self.radius.max(1e-3)).min(1.0), falloff);
        match field {
            CURL => {
                // The noise is a height, and the points run along its
                // contours: the slope turned a quarter turn. That is what
                // keeps them from bunching up. Perlin noise, since value
                // noise is flat at every corner of its lattice and the
                // points would show the squares.
                let k = 1.0 / self.noise_scale.max(1e-3);
                let (x, y, z) = (at[0] * k, at[1] * k, time * self.noise_speed);
                let height = |x: f32, y: f32| perlin3(self.seed, CURL_CHANNEL, x, y, z, 0);
                let across = height(x + CURL_REACH, y) - height(x - CURL_REACH, y);
                let down = height(x, y + CURL_REACH) - height(x, y - CURL_REACH);
                let k = speed / (2.0 * CURL_REACH);
                [down * k, -across * k]
            }
            // Dead centre has no way round or in.
            VORTEX | PULL if far < 1e-6 => [0.0; 2],
            VORTEX => {
                let k = speed * fade() / far;
                [-out[1] * k, out[0] * k]
            }
            PULL => {
                // Never further in one step than the way to Centre.
                let k = (speed * fade()).min(far / dt.max(1e-6)) / far;
                [-out[0] * k, -out[1] * k]
            }
            _ => {
                let (sin, cos) = self.direction.to_radians().sin_cos();
                [cos * speed, sin * speed]
            }
        }
    }

    /// Everything a walk on the layer's clock is made from, as plain bits:
    /// the rows it reads, then each carried point's id and where it starts.
    fn walk_key(
        self,
        falloff: &[f32; cpu::CURVE_TABLE],
        rate: f32,
        in_stream: &PointsStream,
        moved: &[usize],
    ) -> Vec<u32> {
        let mut key = Vec::with_capacity(16 + cpu::CURVE_TABLE + moved.len() * 4);
        key.extend([self.field, self.seed]);
        let rows = [
            rate,
            self.speed,
            self.curl,
            self.vortex,
            self.pull,
            self.wind,
            self.noise_scale,
            self.noise_speed,
            self.centre_x,
            self.centre_y,
            self.radius,
            self.direction,
        ];
        key.extend(rows.map(f32::to_bits));
        key.extend(falloff.iter().map(|v| v.to_bits()));
        for &i in moved {
            let id = in_stream.id.get(i).copied().unwrap_or(0);
            let p = in_stream.position.get(i).copied().unwrap_or([0.0; 3]);
            // The id in two halves.
            key.extend([id as u32, (id >> 32) as u32, p[0].to_bits(), p[1].to_bits()]);
        }
        key
    }

    /// The wired stream with its points carried along the field, across and
    /// down the layer's own plane. `in_stream` and the bag must be in the
    /// same units, and the answer is in those units too. `t` is layer time.
    #[must_use]
    pub fn apply(self, in_stream: &PointsStream, t: f64) -> PointsStream {
        self.walked(in_stream, t, true)
    }

    /// [`apply`](Self::apply), told whether it may use the kept walks. The
    /// answer is the same to the bit either way.
    fn walked(self, in_stream: &PointsStream, t: f64, use_kept: bool) -> PointsStream {
        let mut out = in_stream.clone();
        let moved: Vec<usize> = (0..out.len())
            .filter(|i| {
                let group = self.apply_group.as_str();
                in_stream.applies(self.apply_to, group, self.apply_threshold, *i)
            })
            .collect();
        if moved.is_empty() || self.speed == 0.0 {
            return out;
        }
        let falloff = cpu::curve_table(&self.falloff);
        let rate = self.steps.clamp(1, MAX_RATE as i32) as f32;
        // MAX_STEPS holds a frame to a million steps, about a fifth of a
        // second of Curl noise: past that each point's walk is cut into
        // fewer, longer steps, which follows a tight field less well. Twenty
        // thousand points get fifty steps each.
        // Forward steps, as Caddis takes them: a tight Vortex drifts
        // outwards, and more Steps per second is the cure.
        let most = (MAX_STEPS / moved.len()).max(1);
        let now = t as f32;
        // On the layer's clock every point walks the same steps, and all but
        // the last two are whole steps that do not depend on the time asked
        // for. `keep` is how many of those there are, and where the walk
        // stood after them is what another frame can carry on from.
        // ponytail: three walks still start from the beginning every frame.
        // One past the step cap, since its steps are a length of their own.
        // One on Point age, since each point has its own length of walk and
        // its own starting time. And one whose input moves, since it starts
        // from somewhere new each frame. Keeping a walk per point id would
        // cover the last two, if they come to matter.
        let whole = (now * rate).floor();
        let shared = self.clock != 1 && now.is_finite() && now > 0.0 && whole < most as f32;
        let keep = if shared {
            (whole as usize).saturating_sub(1)
        } else {
            0
        };
        // No lock is held while walking: what is kept is copied out here and
        // the new walk is handed back at the end.
        let key = (use_kept && keep > 0 && moved.len() <= KEPT_POINTS)
            .then(|| self.walk_key(&falloff, rate, in_stream, &moved));
        let (from, kept) = key
            .as_deref()
            .and_then(|key| kept_walk(key, keep))
            .filter(|(_, at)| at.len() == moved.len())
            .unwrap_or_default();
        let mut stood = vec![[0.0f32; 2]; if key.is_some() { moved.len() } else { 0 }];
        for (j, &i) in moved.iter().enumerate() {
            let age = if self.clock == 1 {
                out.age.get(i).copied().unwrap_or(0.0)
            } else {
                now
            };
            // No time yet, or a nonsense one, is no walk.
            if !age.is_finite() || age <= 0.0 {
                continue;
            }
            let Some(p) = out.position.get_mut(i) else {
                continue;
            };
            // Whole steps, then what is left of the last one, so a point
            // moves smoothly between frames.
            let whole = (age * rate).floor();
            let (steps, step) = if whole + 1.0 > most as f32 {
                (most, age / most as f32)
            } else {
                (whole as usize + 1, 1.0 / rate)
            };
            // The field's own clock starts when the point did.
            let start = now - age;
            let mut at = kept.get(j).copied().unwrap_or([p[0], p[1]]);
            let mut going = [0.0f32; 2];
            for k in from..steps {
                if k == keep {
                    if let Some(s) = stood.get_mut(j) {
                        *s = at;
                    }
                }
                let done = k as f32 * step;
                // A step before `keep` is a whole one by its place in the
                // walk, not by a sum that could round differently from one
                // frame to the next.
                let dt = if k < keep {
                    step
                } else {
                    step.min(age - done).max(0.0)
                };
                going = self.carried(at, start + done, dt, &falloff);
                at = [at[0] + going[0] * dt, at[1] + going[1] * dt];
            }
            // A NaN never reaches a position.
            if at[0].is_finite() && at[1].is_finite() {
                (p[0], p[1]) = (at[0], at[1]);
                // The walk's last step is the way the point is going now,
                // which a Trail or a streak below reads.
                if let Some(s) = out.speed.get_mut(i) {
                    (s[0], s[1]) = (going[0], going[1]);
                }
            }
        }
        if let Some(key) = key {
            keep_walk(Walked {
                key,
                steps: keep,
                at: stood,
            });
        }
        out
    }
}

/// Flow points' behaviour, the same shape as Vary points'.
pub struct FlowPointsDef;

impl EffectDef for FlowPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<FlowPoints as EffectMetadata>::SCHEMA
    }

    /// A stream in and a stream out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(FlowPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    fn modify_points(&self, p: Params<'_>, cx: &crate::fx::ModifyCx<'_>) -> Option<PointsStream> {
        Some(FlowPoints::read(p).apply(cx.input()?, cx.t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame reached by carrying on from a kept walk is the frame walked
    /// from the start, to the bit. Without this the kept walks could change
    /// a picture with how it was scrubbed to.
    #[test]
    fn a_walk_carried_on_from_a_kept_one_is_the_whole_walk_to_the_bit() {
        let mut stream = PointsStream::default();
        for n in 0..37u64 {
            let (x, y) = ((n * 53 % 37) as f32, (n * 29 % 37) as f32);
            stream
                .position
                .push([300.0 + x * 17.3, 200.0 + y * 11.9, 0.0]);
            stream.speed.push([0.0; 3]);
            stream.age.push(0.0);
            stream.life.push(1.0);
            stream.size.push(4.0);
            stream.rotation.push(0.0);
            stream.colour.push([1.0; 4]);
            stream.id.push(n);
        }
        // Every field at once, with a noise that changes over time.
        let flow = FlowPoints {
            field: COMBINED,
            speed: 130.0,
            curl: 100.0,
            vortex: 100.0,
            pull: 40.0,
            wind: 20.0,
            noise_scale: 180.0,
            noise_speed: 0.7,
            seed: 4242,
            centre_x: 500.0,
            centre_y: 400.0,
            radius: 400.0,
            falloff: CurvePoints::sanitised(&[[0.0, 1.0], [1.0, 0.2]]),
            direction: 30.0,
            clock: 0,
            steps: 30,
            apply_to: 0,
            apply_group: ShortText::EMPTY,
            apply_threshold: 0.5,
            feather: 100.0,
            mix: 100.0,
        };
        let bits = |s: &PointsStream| -> Vec<u32> {
            let both = s.position.iter().chain(&s.speed);
            both.flat_map(|v| v.map(f32::to_bits)).collect()
        };
        // 28 whole steps early and 101 late, so 27 and 100 kept.
        let (early, late) = (0.95, 3.37);
        let whole = flow.walked(&stream, late, false);
        let moved: Vec<usize> = (0..stream.len()).collect();
        let key = flow.walk_key(&cpu::curve_table(&flow.falloff), 30.0, &stream, &moved);
        let furthest = |keep| kept_walk(&key, keep).map(|(steps, _)| steps);
        assert_eq!(furthest(usize::MAX), None, "nothing is kept yet");

        let first = flow.walked(&stream, early, true);
        assert_eq!(bits(&first), bits(&flow.walked(&stream, early, false)));
        assert_eq!(furthest(usize::MAX), Some(27));
        // Carried on from the early walk.
        let carried = flow.walked(&stream, late, true);
        assert_eq!(furthest(usize::MAX), Some(100));
        assert_eq!(bits(&carried), bits(&whole));
        // And the same frame again, from the walk it has just kept.
        assert_eq!(bits(&flow.walked(&stream, late, true)), bits(&whole));
    }
}
