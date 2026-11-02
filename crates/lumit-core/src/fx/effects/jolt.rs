//! Jolt: sudden hits that kick the picture and let it settle. Six kinds of
//! hit, each with its own switch and amount: a blur, a colour, a flash of
//! light, a scale, a slide and a jump in time.
//!
//! When the hits come is worked out at resolve time, from the seed, the clock
//! and the markers, and handed over as plain numbers with no unit in them, as
//! Shake's wobble is. The px@comp rows stay rows so they follow the raster, and
//! [`Jolt::packed`] puts the two together for both render paths. A jump in
//! time is the layer built again at another moment ([`crate::fx::moments`]),
//! which the GPU pass reads in place of its input.

use crate::fx::{
    builtins::EDGE_OPTIONS, cpu, markers::MarkerContext, maths::hue_matrix_rgb,
    maths::rgb_split_offset, moments::OwnMoments, noise::hash01, params::ParamId, params::Params,
    params::Value, presets::EffectPreset, presets::PresetValue, registry::EffectDef,
    registry::EffectMetadata, registry::ResolveCx, schema::EffectSchema, schema::ParamGroup,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// One twirl per kind of hit, its switch first.
const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// Rate only means something on the Random trigger, and each kind of hit
/// folds away under its own heading.
pub const JOLT_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "",
        params: &["rate", "separate"],
        collapsed: false,
        visible_when: Some(("trigger", &[0])),
        visible_when_lens_elements: None,
    },
    group("Blur", &["blur_on", "blur_amount", "blur_aspect"]),
    group(
        "Colour",
        &["colour_on", "colour_amount", "colour", "random_hue"],
    ),
    group("Light", &["light_on", "light_amount", "light_direction"]),
    group(
        "Scale",
        &[
            "scale_on",
            "scale_amount",
            "origin_x",
            "origin_y",
            "random_origin",
        ],
    ),
    group(
        "Slide",
        &[
            "slide_on",
            "slide_amount",
            "slide_direction",
            "spread",
            "rgb_split",
        ],
    ),
    group("Time", &["time_on", "time_amount", "time_direction"]),
];

/// Jolt's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "jolt",
    label = "Jolt",
    version = 1,
    category = Distortion,
    cost = Cheap,
    // A scale or a slide reads from anywhere in the frame, as Shake does.
    roi = FullFrame,
    // Its pixels change with time under fixed controls, and on the marker
    // triggers with the markers too, which the frame key reads.
    seeded = true,
    beat_input = true,
    groups = JOLT_GROUPS,
)]
pub struct Jolt {
    /// How hard every kind of hit lands, per cent.
    #[bounded(min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub amount: f32,

    /// What fires a jolt. Markers are the layer's own and the comp's that are
    /// not beats. On either marker trigger every kind of hit fires at once.
    #[choice(options = ["Random", "Markers", "Beats"], default = 0)]
    pub trigger: u32,

    /// Jolts a second on the Random trigger.
    #[slider(min = 0.0, max = 20.0, default = 2.0, hard_min = 0.0, unit = Raw)]
    pub rate: f32,

    /// On the Random trigger, each kind of hit keeps its own time rather than
    /// all landing together as one jolt.
    #[toggle(label = "Separate timing", default = false)]
    pub separate: bool,

    /// Frames a jolt takes to reach its peak. 0 is an instant hit.
    #[slider(
        label = "Ease in",
        min = 0.0,
        max = 12.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Frames
    )]
    pub ease_in: f32,

    /// Frames a jolt takes to settle after its peak.
    #[slider(
        label = "Ease out",
        min = 0.0,
        max = 24.0,
        default = 4.0,
        hard_min = 0.0,
        unit = Frames
    )]
    pub ease_out: f32,

    /// Per instance, so two layers do not jolt together.
    #[seed]
    pub seed: u32,

    /// What shows where a scale or a slide pulls the picture off its edge.
    /// Mirror reads as more picture rather than as a border.
    #[choice(label = "Edges", options = *EDGE_OPTIONS, default = 2)]
    pub edge: u32,

    /// The layer shows only while a jolt is on it.
    #[toggle(label = "Jolts only", default = false)]
    pub jolts_only: bool,

    /// Each hit blurs the picture.
    #[toggle(label = "Blur", default = true)]
    pub blur_on: bool,

    /// The blur's radius at a full hit, px@comp.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 100.0,
        default = 20.0,
        hard_min = 0.0,
        hard_max = 500.0,
        unit = Px
    )]
    pub blur_amount: f32,

    /// 0 blurs evenly. Towards 100 the blur is only sideways, towards −100
    /// only up and down.
    #[bounded(
        label = "Aspect",
        min = -100.0,
        max = 100.0,
        default = 0.0,
        unit = Percent
    )]
    pub blur_aspect: f32,

    /// Each hit mixes in a colour.
    #[toggle(label = "Colour", default = false)]
    pub colour_on: bool,

    /// How far a full hit mixes the picture towards the colour, per cent.
    #[bounded(
        label = "Amount",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub colour_amount: f32,

    /// The colour a hit mixes towards, scene-linear.
    #[colour(label = "Tint", default = [0.9, 0.05, 0.35, 1.0])]
    pub colour: [f32; 4],

    /// How far each hit's hue wanders from the colour's, per cent of half
    /// the wheel.
    #[bounded(
        label = "Random hue",
        min = 0.0,
        max = 100.0,
        default = 25.0,
        unit = Percent
    )]
    pub random_hue: f32,

    /// Each hit turns the light up or down.
    #[toggle(label = "Light", default = true)]
    pub light_on: bool,

    /// Stops of light at a full hit.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        hard_max = 10.0,
        unit = Raw
    )]
    pub light_amount: f32,

    /// Which way a hit turns the light. Both picks one at random each time.
    #[choice(
        label = "Direction",
        options = ["Brighter", "Darker", "Both"],
        default = 0
    )]
    pub light_direction: u32,

    /// Each hit scales the picture.
    #[toggle(label = "Scale", default = true)]
    pub scale_on: bool,

    /// How much a full hit scales the picture, up or down at random, per cent.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 50.0,
        default = 10.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub scale_amount: f32,

    /// What the scale grows from, px@comp. A fresh instance sits on the
    /// middle of the frame it lands on.
    #[slider(label = "Origin x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub origin_x: f32,

    /// See [`origin_x`](Self::origin_x).
    #[slider(label = "Origin y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub origin_y: f32,

    /// How far each hit's origin wanders, per cent of half the frame.
    #[bounded(
        label = "Random origin",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        unit = Percent
    )]
    pub random_origin: f32,

    /// Each hit slides the picture.
    #[toggle(label = "Slide", default = true)]
    pub slide_on: bool,

    /// How far a full hit slides the picture, px@comp.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 200.0,
        default = 40.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
    pub slide_amount: f32,

    /// The way a hit slides, degrees clockwise from the right.
    #[dial(label = "Direction", default = 0.0)]
    pub slide_direction: f32,

    /// How far either side of Direction a hit may go. 180 is any way at all.
    #[bounded(min = 0.0, max = 180.0, default = 180.0, unit = Degrees)]
    pub spread: f32,

    /// How far red and blue part along the slide, per cent of it.
    #[bounded(
        label = "RGB split",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        unit = Percent
    )]
    pub rgb_split: f32,

    /// Each hit jumps the layer to another moment and back.
    #[toggle(label = "Time", default = false)]
    pub time_on: bool,

    /// How far a full hit jumps, frames.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 30.0,
        default = 4.0,
        hard_min = 0.0,
        hard_max = 240.0,
        unit = Frames
    )]
    pub time_amount: f32,

    /// Which way a hit jumps: either, only ahead, or only back.
    #[choice(label = "Direction", options = ["Both", "Forward", "Backward"], default = 0)]
    pub time_direction: u32,

    /// The host-uniform Mix every effect ends with, per cent.
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

/// One frame of Jolt, as both render paths draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JoltParams {
    /// The blur's radius across and down, raster pixels.
    pub blur: [f32; 2],
    /// Each pixel reads the picture at `pixel × zoom + offset`.
    pub zoom: f32,
    /// See [`zoom`](Self::zoom), raster pixels.
    pub offset: [f32; 2],
    /// Red is read this far back and blue this far on, raster pixels.
    pub split: [f32; 2],
    /// The colour mixed in, scene-linear, and how far.
    pub tint: [f32; 4],
    /// The light, multiplied in.
    pub gain: f32,
    /// The whole pixel, multiplied in. Below 1 only with Jolts only on.
    pub opacity: f32,
    /// [`EdgesMode`](crate::fx::schema::EdgesMode) code.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl JoltParams {
    /// Nothing is hitting: the picture goes through untouched, bit for bit.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.mix == 0.0
            || (self.blur == [0.0; 2]
                && self.zoom == 1.0
                && self.offset == [0.0; 2]
                && self.split == [0.0; 2]
                && self.tint[3] == 0.0
                && self.gain == 1.0
                && self.opacity == 1.0)
    }
}

impl Jolt {
    /// Each kind of hit's strength this frame, 0..1: blur, colour, light and
    /// scale. Light and scale carry their direction in the sign.
    pub const DERIVED_HIT: ParamId = ParamId::new("derived.hit");
    /// Where each hit points, each −1..1: the colour's hue, the scale's
    /// origin across and down, and the slide's way within Spread.
    pub const DERIVED_AIM: ParamId = ParamId::new("derived.aim");
    /// The slide's strength, and how much of a jolt is on at all for Jolts
    /// only.
    pub const DERIVED_SLIDE: ParamId = ParamId::new("derived.slide");

    /// The numbers both kernels read, from the rows and the derived hits in
    /// `p`, on a `w` by `h` raster.
    #[must_use]
    pub fn packed(self, p: Params<'_>, w: u32, h: u32) -> JoltParams {
        let [blur, colour, light, scale] = p.vec4(Self::DERIVED_HIT, [0.0; 4]);
        let [hue, wander_x, wander_y, way] = p.vec4(Self::DERIVED_AIM, [0.0; 4]);
        let [slide, shown, _, _] = p.vec4(Self::DERIVED_SLIDE, [0.0; 4]);
        let master = (self.amount / 100.0).clamp(0.0, 1.0);
        let share = |v: f32| (v / 100.0).clamp(0.0, 1.0);

        let r = self.blur_amount.max(0.0) * blur * master;
        let aspect = (self.blur_aspect / 100.0).clamp(-1.0, 1.0);

        // Up by the amount, or down by the same factor, about the origin.
        let grow = 1.0 + self.scale_amount.max(0.0) / 100.0 * scale.abs() * master;
        let zoom = if scale < 0.0 { grow } else { 1.0 / grow };
        let wander = share(self.random_origin) * 0.5;
        let origin = [
            self.origin_x + wander_x * wander * w as f32,
            self.origin_y + wander_y * wander * h as f32,
        ];
        let (sx, sy) = rgb_split_offset(
            self.slide_amount.max(0.0) * slide * master,
            self.slide_direction + way * self.spread.clamp(0.0, 180.0),
        );
        let split = share(self.rgb_split);

        let m = hue_matrix_rgb(f64::from(hue * share(self.random_hue) * 180.0));
        let c = self.colour;
        let tint = [
            m[0] * c[0] + m[1] * c[1] + m[2] * c[2],
            m[3] * c[0] + m[4] * c[1] + m[5] * c[2],
            m[6] * c[0] + m[7] * c[1] + m[8] * c[2],
            share(self.colour_amount) * colour * master,
        ];

        JoltParams {
            blur: [r * (1.0 + aspect).min(1.0), r * (1.0 - aspect).min(1.0)],
            zoom,
            offset: [
                origin[0] - origin[0] * zoom - sx,
                origin[1] - origin[1] * zoom - sy,
            ],
            split: [sx * split, sy * split],
            tint,
            gain: (self.light_amount.max(0.0) * light * master).exp2(),
            opacity: if self.jolts_only { shown } else { 1.0 },
            edge: self.edge.min(2),
            mix: share(self.mix),
        }
    }
}

/// How far through its hit a jolt is, `dt` frames after it fired: 0 before,
/// up to 1 over `ease_in` frames along a smoothstep, then back to 0 over
/// `ease_out` frames, quickly at first, so the hit is sharp and the settle
/// soft. With no ease out the peak lasts one frame.
#[must_use]
pub fn jolt_envelope(dt: f64, ease_in: f64, ease_out: f64) -> f64 {
    if dt < 0.0 {
        return 0.0;
    }
    if dt < ease_in {
        let u = dt / ease_in;
        return u * u * (3.0 - 2.0 * u);
    }
    let u = (dt - ease_in) / ease_out.max(1.0);
    if u >= 1.0 {
        0.0
    } else {
        (1.0 - u) * (1.0 - u)
    }
}

/// The marker times `e` fires on, layer-local seconds, or None on the Random
/// trigger.
#[must_use]
pub fn trigger_times<'a>(e: &EffectInstance, markers: &'a MarkerContext) -> Option<&'a [f64]> {
    match e.param("trigger") {
        Some(EffectValue::Choice(1)) => Some(&markers.cues),
        Some(EffectValue::Choice(2)) => Some(&markers.beats),
        _ => None,
    }
}

/// The frame the latest Random jolt at or before `frame` fired on, and which
/// one it was. Each slot `slot` frames long holds one jolt, at a seeded place
/// in it, on a whole frame.
fn random_start(seed: u32, op: u32, frame: f64, slot: f64) -> Option<(f64, i32)> {
    let k = (frame / slot).floor();
    (0..3).map(|back| k - f64::from(back)).find_map(|k| {
        let start = ((k + f64::from(hash01(seed, op, k as i32, 0, 0))) * slot).ceil();
        (start <= frame).then_some((start, k as i32))
    })
}

/// The latest jolt of kind `op` at layer time `lt`: its strength, how far
/// through it is, and three seeded draws for where it points. `fl` reads a
/// row at `lt`, with a default.
fn latest_hit(
    e: &EffectInstance,
    lt: f64,
    markers: &MarkerContext,
    op: u32,
    fl: &dyn Fn(&str, f64) -> f64,
) -> Option<(f32, f32, [f32; 3])> {
    let seed = match e.param("seed") {
        Some(EffectValue::Seed(s)) => *s,
        _ => 0,
    };
    // Frames are comp frames. With no comp in play a jolt still lands.
    let fps = if markers.fps > 0.0 { markers.fps } else { 30.0 };
    // Rounded, so a frame's time does not land a hair either side of it.
    let snap = |f: f64| (f * 1000.0).round() / 1000.0;
    let frame = snap(lt * fps);
    let (ease_in, ease_out) = (fl("ease_in", 0.0).max(0.0), fl("ease_out", 4.0).max(0.0));
    let rate = fl("rate", 2.0);
    let (start, key) = match trigger_times(e, markers) {
        Some(times) => {
            let i = times.partition_point(|t| *t <= lt);
            let start = snap(times.get(i.checked_sub(1)?)? * fps);
            (start, start.round() as i32)
        }
        // Together, every kind of hit lands on the first kind's moments.
        None if rate > 0.0 => {
            let timed = if e.bool_of("separate").unwrap_or(false) {
                op
            } else {
                0
            };
            random_start(seed, timed, frame, fps / rate)?
        }
        None => return None,
    };
    let env = jolt_envelope(frame - start, ease_in, ease_out);
    if env <= 0.0 {
        return None;
    }
    let draw = |n: i32| hash01(seed, op, key, n, 1);
    let strength = env * (0.5 + 0.5 * f64::from(draw(0)));
    Some((strength as f32, env as f32, [draw(1), draw(2), draw(3)]))
}

/// The rows that switch each kind of hit on, in the order their numbers are
/// drawn, with their defaults.
const KINDS: [(&str, bool); 6] = [
    ("blur_on", true),
    ("colour_on", false),
    ("light_on", true),
    ("scale_on", true),
    ("slide_on", true),
    ("time_on", false),
];

/// Which of [`KINDS`] the time hit is.
const TIME: u32 = 5;

/// Jolt's behaviour.
pub struct JoltDef;

impl EffectDef for JoltDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Jolt as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [EffectPreset] {
        &[
            EffectPreset {
                name: "Camera kick",
                values: &[
                    ("rate", PresetValue::Float(1.0)),
                    ("ease_out", PresetValue::Float(8.0)),
                    ("blur_amount", PresetValue::Float(30.0)),
                    ("blur_aspect", PresetValue::Float(-100.0)),
                    ("light_on", PresetValue::Bool(false)),
                    ("scale_amount", PresetValue::Float(4.0)),
                    ("slide_amount", PresetValue::Float(120.0)),
                    ("slide_direction", PresetValue::Float(90.0)),
                    ("spread", PresetValue::Float(20.0)),
                ],
            },
            EffectPreset {
                name: "Glitch hit",
                values: &[
                    ("rate", PresetValue::Float(4.0)),
                    ("ease_out", PresetValue::Float(2.0)),
                    ("blur_aspect", PresetValue::Float(100.0)),
                    ("colour_on", PresetValue::Bool(true)),
                    ("random_hue", PresetValue::Float(100.0)),
                    ("light_direction", PresetValue::Choice(2)),
                    ("scale_on", PresetValue::Bool(false)),
                    ("slide_amount", PresetValue::Float(80.0)),
                    ("spread", PresetValue::Float(20.0)),
                    ("rgb_split", PresetValue::Float(60.0)),
                ],
            },
            EffectPreset {
                name: "Light flicker",
                values: &[
                    ("rate", PresetValue::Float(6.0)),
                    ("ease_out", PresetValue::Float(3.0)),
                    ("blur_on", PresetValue::Bool(false)),
                    ("light_amount", PresetValue::Float(1.5)),
                    ("light_direction", PresetValue::Choice(2)),
                    ("scale_on", PresetValue::Bool(false)),
                    ("slide_on", PresetValue::Bool(false)),
                ],
            },
            EffectPreset {
                name: "Time stutter",
                values: &[
                    ("rate", PresetValue::Float(3.0)),
                    ("ease_out", PresetValue::Float(3.0)),
                    ("blur_on", PresetValue::Bool(false)),
                    ("light_on", PresetValue::Bool(false)),
                    ("scale_on", PresetValue::Bool(false)),
                    ("slide_on", PresetValue::Bool(false)),
                    ("time_on", PresetValue::Bool(true)),
                    ("time_amount", PresetValue::Float(6.0)),
                ],
            },
            EffectPreset {
                name: "Beat pulse",
                values: &[
                    ("trigger", PresetValue::Choice(2)),
                    ("ease_out", PresetValue::Float(8.0)),
                    ("blur_amount", PresetValue::Float(10.0)),
                    ("light_amount", PresetValue::Float(0.7)),
                    ("scale_amount", PresetValue::Float(8.0)),
                    ("slide_on", PresetValue::Bool(false)),
                ],
            },
        ]
    }

    /// When each kind of hit last fired and how hard, as unit-free numbers
    /// for [`Jolt::packed`].
    /// A time hit shows the layer that many whole frames away, back at the
    /// frame itself as the hit settles. No moment at all while none is on.
    fn own_moments(
        &self,
        e: &EffectInstance,
        lt: f64,
        markers: &MarkerContext,
    ) -> Option<OwnMoments> {
        if !e.bool_of("time_on").unwrap_or(false) {
            return None;
        }
        let fl = |id: &str, d: f64| e.float_at(id, lt).unwrap_or(d);
        let (strength, _, draws) = latest_hit(e, lt, markers, TIME, &fl)?;
        let master = (fl("amount", 100.0) / 100.0).clamp(0.0, 1.0);
        let way = match e.param("time_direction") {
            Some(EffectValue::Choice(1)) => 1.0,
            Some(EffectValue::Choice(2)) => -1.0,
            _ if draws[0] < 0.5 => -1.0,
            _ => 1.0,
        };
        let frames = (fl("time_amount", 4.0).max(0.0) * master * f64::from(strength)).round();
        let fps = if markers.fps > 0.0 { markers.fps } else { 30.0 };
        (frames != 0.0).then(|| OwnMoments {
            a: lt + way * frames / fps,
            b: None,
        })
    }

    fn shows_own_moments(&self) -> bool {
        true
    }

    /// Only while its time hits are on: otherwise a Timewarp below it in the
    /// stack keeps the moments.
    fn wants_own_moments(&self, e: &EffectInstance) -> bool {
        e.bool_of("time_on").unwrap_or(false)
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let (e, lt) = (cx.inst, cx.lt);
        let fl = |id: &str, d: f64| {
            e.float_at_with_context(id, lt, cx.context.clone())
                .unwrap_or(d)
        };
        let hit = |op: u32| latest_hit(e, lt, cx.markers, op, &fl);
        let mut hits = [None; 6];
        for (op, (slot, (id, on))) in hits.iter_mut().zip(KINDS).enumerate() {
            if e.bool_of(id).unwrap_or(on) {
                *slot = hit(op as u32);
            }
        }
        let [blur, colour, light, scale, slide, _] = hits;
        let k = |h: Option<(f32, f32, [f32; 3])>| h.map_or(0.0, |h| h.0);
        // A draw as −1..1.
        let aim = |h: Option<(f32, f32, [f32; 3])>, n: usize| {
            h.map_or(0.0, |h| h.2.get(n).map_or(0.0, |d| d * 2.0 - 1.0))
        };
        let either = |h| if aim(h, 0) < 0.0 { -1.0 } else { 1.0 };
        let light_way = match e.param("light_direction") {
            Some(EffectValue::Choice(1)) => -1.0,
            Some(EffectValue::Choice(2)) => either(light),
            _ => 1.0,
        };
        push(
            Jolt::DERIVED_HIT,
            Value::Vec4([
                k(blur),
                k(colour),
                k(light) * light_way,
                k(scale) * either(scale),
            ]),
        );
        push(
            Jolt::DERIVED_AIM,
            Value::Vec4([aim(colour, 0), aim(scale, 1), aim(scale, 2), aim(slide, 0)]),
        );
        let shown = hits.iter().flatten().map(|h| h.1).fold(0.0, f32::max);
        push(
            Jolt::DERIVED_SLIDE,
            Value::Vec4([k(slide), shown, 0.0, 0.0]),
        );
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        jolt(rgba, w, h, &Jolt::read(p).packed(p, w, h));
    }
}

/// The blur, `radius` across then down, each a separable gaussian pass as
/// `fx_blur.wgsl` runs them.
fn blur_axes(rgba: &mut [f32], w: u32, h: u32, radius: [f32; 2], edge: u32) {
    let (wi, hi) = (i64::from(w), i64::from(h));
    for (across, r) in [(true, radius[0]), (false, radius[1])] {
        if r <= 0.0 {
            continue;
        }
        let weights = cpu::gaussian_weights(r);
        let reach = (weights.len() / 2) as i64;
        let src = rgba.to_vec();
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let (x, y) = (i as i64 % wi, i as i64 / wi);
            let mut acc = [0.0f32; 4];
            for (j, wt) in weights.iter().enumerate() {
                let tap = j as i64 - reach;
                let at = if across {
                    cpu::edge_index(x + tap, wi, edge).map(|tx| y * wi + tx)
                } else {
                    cpu::edge_index(y + tap, hi, edge).map(|ty| ty * wi + x)
                };
                let Some(s) = at.and_then(|s| src.get(s as usize * 4..s as usize * 4 + 4)) else {
                    continue;
                };
                for (a, v) in acc.iter_mut().zip(s) {
                    *a += v * wt;
                }
            }
            px.copy_from_slice(&acc);
        }
    }
}

/// The CPU oracle for `fx_jolt.wgsl`: the blur, then each pixel read through
/// the scale and the slide, red and blue parted, the colour mixed in and the
/// light multiplied.
pub fn jolt(rgba: &mut [f32], w: u32, h: u32, p: &JoltParams) {
    if p.is_identity() {
        return;
    }
    let original = rgba.to_vec();
    blur_axes(rgba, w, h, p.blur, p.edge);
    let src = rgba.to_vec();
    let w1 = w.max(1);
    for (i, (px, o)) in rgba
        .chunks_exact_mut(4)
        .zip(original.chunks_exact(4))
        .enumerate()
    {
        let x = (i as u32 % w1) as f32 + 0.5;
        let y = (i as u32 / w1) as f32 + 0.5;
        let (qx, qy) = (x * p.zoom + p.offset[0], y * p.zoom + p.offset[1]);
        let mut v = cpu::bilinear_edge(&src, w, h, qx, qy, p.edge);
        if p.split != [0.0; 2] {
            let r = cpu::bilinear_edge(&src, w, h, qx - p.split[0], qy - p.split[1], p.edge);
            let b = cpu::bilinear_edge(&src, w, h, qx + p.split[0], qy + p.split[1], p.edge);
            v = [r[0], v[1], b[2], r[3].max(v[3]).max(b[3])];
        }
        // The colour is laid over the picture's own light, so its detail
        // stays.
        let k = p.tint[3];
        let luma = |c: &[f32]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        let lift = luma(&v) / luma(&p.tint).max(1e-3);
        for (x, t) in v.iter_mut().zip(p.tint).take(3) {
            *x = (*x * (1.0 - k) + t * lift * k) * p.gain;
        }
        for c in 0..4 {
            px[c] = o[c] * (1.0 - p.mix) + v[c] * p.opacity * p.mix;
        }
    }
}
