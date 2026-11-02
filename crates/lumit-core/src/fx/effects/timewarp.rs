//! Timewarp: shows the layer at another time, as an effect, so it retimes a
//! precomp or everything under an adjustment layer as readily as footage.
//!
//! The time it shows is set by a Speed, read from the start of the layer, or
//! keyframed straight as a Source frame. Between two frames of its input it
//! holds the earlier one, mixes the two, or makes the frame in between from the
//! motion it measures (Pixel motion), and that motion can smear the result as a
//! shutter would.
//!
//! The input is the layer as the effects above Timewarp leave it, built again
//! at the moments [`TimewarpDef::own_moments`] names
//! ([`crate::fx::moments`]). Frames are counted at the composition's rate, or
//! at Input rate when one is set, so footage shot faster than the composition
//! can show every frame it has.

use crate::anim::{Animation, Property};
use crate::fx::markers::MarkerContext;
use crate::fx::{
    moments::OwnMoments, params::ParamId, params::Params, params::Value, presets::EffectPreset,
    presets::PresetValue, registry::EffectDef, registry::EffectMetadata, registry::ResolveCx,
    schema::EffectSchema, schema::ParamGroup,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// Timewarp's groups: the Speed or the Source frame, whichever drives it, the
/// measurement, the motion blur and the input's rate.
pub const TIMEWARP_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "",
        params: &["speed"],
        collapsed: false,
        visible_when: Some(("adjust", &[0])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["source_frame"],
        collapsed: false,
        visible_when: Some(("adjust", &[1])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Motion",
        params: &["quality", "smoothness"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Motion blur",
        params: &["motion_blur", "shutter_angle", "samples"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Input",
        params: &["input_rate"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Timewarp's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "timewarp",
    label = "Timewarp",
    version = 1,
    category = Temporal,
    // Two renders of its input a frame, and the motion between them.
    cost = Heavy,
    roi = FullFrame,
    // What it shows moves with the clock under fixed controls.
    seeded = true,
    groups = TIMEWARP_GROUPS,
)]
pub struct Timewarp {
    /// Whether a Speed or a Source frame says what time is shown.
    #[choice(label = "Adjust time by", options = ["Speed", "Source frame"], default = 0)]
    pub adjust: u32,

    /// Per cent of real time, read from the start of the layer. 50 by default,
    /// so the effect shows something when it is added. Negative runs back.
    #[slider(
        min = -400.0,
        max = 400.0,
        default = 50.0,
        hard_min = -10000.0,
        hard_max = 10000.0,
        unit = Percent
    )]
    pub speed: f32,

    /// The frame of the input shown now, at the composition's rate. Keyframed,
    /// it is a time remap curve.
    #[slider(label = "Source frame", min = 0.0, max = 300.0, default = 0.0, unit = Frames)]
    pub source_frame: f32,

    /// How a time between two frames is shown.
    #[choice(options = ["Whole frames", "Frame mix", "Pixel motion"], default = 2)]
    pub method: u32,

    /// How closely the motion is measured, for Pixel motion and the motion blur.
    #[choice(options = ["Draft", "Normal", "High"], default = 1)]
    pub quality: u32,

    /// How smooth the measured motion is kept: high tears less, low keeps
    /// edges between moving things sharper.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub smoothness: f32,

    /// Smear along the measured motion, as long as the shutter is open.
    #[toggle(label = "Motion blur", default = false)]
    pub motion_blur: bool,

    /// How long the shutter is open, degrees of one frame.
    #[slider(
        label = "Shutter angle",
        min = 0.0,
        max = 720.0,
        default = 180.0,
        hard_min = 0.0,
        hard_max = 3600.0,
        unit = Degrees
    )]
    pub shutter_angle: f32,

    /// The most taps along a streak.
    #[slider(min = 2.0, max = 64.0, default = 16.0, hard_min = 2.0, hard_max = 64.0, unit = Raw)]
    pub samples: f32,

    /// Frames a second the input is counted in. 0 is the composition's rate;
    /// footage shot faster than the composition shows all its frames at its
    /// own.
    #[slider(
        label = "Input rate",
        min = 0.0,
        max = 120.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 1000.0,
        unit = Raw
    )]
    pub input_rate: f32,

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

/// How the in-between frame is shown, as the kernel reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimewarpMethod {
    WholeFrames,
    FrameMix,
    PixelMotion,
}

/// One resolved Timewarp, reduced to what the GPU pass reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimewarpParams {
    pub method: TimewarpMethod,
    /// How far from the first moment towards the second the picture sits.
    pub phase: f32,
    /// Streak length as a share of the motion between the two moments, 0 for
    /// no motion blur.
    pub shutter: f32,
    pub samples: i32,
    /// Divide the picture by this before measuring its motion.
    pub divisor: u32,
    /// The motion measurement's detail, as `retime::VectorDetail` codes.
    pub detail: u32,
    pub smoothness: f32,
    /// 0..1, blended against the input at the frame's own time.
    pub mix: f32,
}

/// The layer time Timewarp shows at layer time `lt`, never before the layer's
/// start. Speed is read from the start of the layer, so a keyframed Speed is a
/// ramp. `None` when the instance carries neither row.
#[must_use]
pub fn source_time(e: &EffectInstance, lt: f64, comp_fps: f64) -> Option<f64> {
    let fps = comp_fps.max(1.0);
    let t = match e.param("adjust") {
        Some(EffectValue::Choice(1)) => e.float_at("source_frame", lt)? / fps,
        _ => match e.param("speed") {
            Some(EffectValue::Float(p)) => integral(p, lt) / 100.0,
            _ => return None,
        },
    };
    Some(if t.is_finite() { t.max(0.0) } else { 0.0 })
}

/// How fast the shown time moves at `lt`, input seconds a second.
#[must_use]
pub fn rate_of_change(e: &EffectInstance, lt: f64, comp_fps: f64) -> f64 {
    let h = 0.5 / comp_fps.max(1.0);
    let at = |t| source_time(e, t, comp_fps).unwrap_or(0.0);
    // One-sided at the very start, where the time is held at the layer's start.
    let (t0, t1) = if lt - h < 0.0 {
        (lt, lt + h)
    } else {
        (lt - h, lt + h)
    };
    (at(t1) - at(t0)) / (t1 - t0)
}

/// The area under `p` from 0 to `lt`: exact for a still value and for an
/// expression, which is read as one value here, and Simpson's rule between
/// each pair of keys otherwise.
fn integral(p: &Property, lt: f64) -> f64 {
    let (from, to, sign) = if lt >= 0.0 {
        (0.0, lt, 1.0)
    } else {
        (lt, 0.0, -1.0)
    };
    let mut cuts = vec![from];
    match &p.animation {
        Animation::Static(_) | Animation::Expression(_) => return p.value_at(lt) * lt,
        Animation::Keyframed(keys) => cuts.extend(
            keys.iter()
                .map(|k| k.time.to_f64())
                .filter(|&k| k > from && k < to),
        ),
    }
    cuts.push(to);
    let simpson = |a: f64, b: f64| {
        const N: usize = 16;
        let h = (b - a) / N as f64;
        (0..=N)
            .map(|i| {
                let w = if i == 0 || i == N {
                    1.0
                } else if i % 2 == 1 {
                    4.0
                } else {
                    2.0
                };
                w * p.value_at(a + h * i as f64)
            })
            .sum::<f64>()
            * h
            / 3.0
    };
    sign * cuts.windows(2).map(|w| simpson(w[0], w[1])).sum::<f64>()
}

impl Timewarp {
    /// How far between its two moments the picture sits, worked out at resolve
    /// time from the clock rather than declared.
    pub const DERIVED_PHASE: ParamId = ParamId::new("derived.phase");
    /// How many frames of the input pass a frame, for the motion blur.
    pub const DERIVED_ADVANCE: ParamId = ParamId::new("derived.advance");

    /// [`Self::packed`] with its two derived numbers read out of a resolved bag.
    #[must_use]
    pub fn packed_from(p: Params<'_>) -> TimewarpParams {
        Self::read(p).packed(
            p.float(Self::DERIVED_PHASE, 0.0),
            f64::from(p.float(Self::DERIVED_ADVANCE, 0.0)),
        )
    }

    /// The numbers the GPU pass reads, with the moments' phase and how fast the
    /// shown time moves through the input, input frames a frame.
    #[must_use]
    pub fn packed(self, phase: f32, advance: f64) -> TimewarpParams {
        // Draft, Normal and Best: how small the picture is measured at and how
        // hard the search works.
        let (divisor, detail) = match self.quality {
            0 => (4, 0),
            2 => (1, 2),
            _ => (2, 1),
        };
        let shutter = if self.motion_blur {
            (f64::from(self.shutter_angle.max(0.0)) / 360.0 * advance.abs()) as f32
        } else {
            0.0
        };
        TimewarpParams {
            method: match self.method {
                0 => TimewarpMethod::WholeFrames,
                1 => TimewarpMethod::FrameMix,
                _ => TimewarpMethod::PixelMotion,
            },
            phase,
            shutter,
            samples: self.samples.round().clamp(2.0, 64.0) as i32,
            divisor,
            detail,
            smoothness: self.smoothness.clamp(0.0, 100.0),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// The CPU half of Frame mix, and of Pixel motion where neither frame can say
/// what a pixel is: the two moments crossfaded. The WGSL twin is the same
/// expression.
pub fn timewarp_mix(a: &mut [f32], b: &[f32], phase: f32) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += (y - *x) * phase;
    }
}

/// One `w` by `h` RGBA picture or field, read as the kernel reads it.
struct Plane<'a> {
    px: &'a [f32],
    w: usize,
    h: usize,
}

impl Plane<'_> {
    fn load(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        let i = (y * self.w + x) * 4;
        match self.px.get(i..i + 4) {
            Some(p) => [p[0], p[1], p[2], p[3]],
            None => [0.0; 4],
        }
    }

    /// Bilinear, edge-clamped, on pixel-centre coordinates.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = x.clamp(0.0, (self.w - 1) as f32);
        let y = y.clamp(0.0, (self.h - 1) as f32);
        let (fx, fy) = (x.floor(), y.floor());
        let (tx, ty) = (x - fx, y - fy);
        let (x0, y0) = (fx as i64, fy as i64);
        let lerp = |p: [f32; 4], q: [f32; 4], t: f32| -> [f32; 4] {
            std::array::from_fn(|c| p[c] * (1.0 - t) + q[c] * t)
        };
        let top = lerp(self.load(x0, y0), self.load(x0 + 1, y0), tx);
        let bot = lerp(self.load(x0, y0 + 1), self.load(x0 + 1, y0 + 1), tx);
        lerp(top, bot, ty)
    }
}

/// Pixel motion, the CPU oracle `fx_timewarp.wgsl` is held to: the frame at
/// `phase` between `a` and `b` (`w` by `h`), each moment carried along its
/// measured motion and the two blended, each trusted where the other cannot
/// see. `fwd` is the motion from `a` to `b` and `bwd` back, `fw` by `fh` as
/// measured, as x, y in measured pixels and how far it is trusted.
#[must_use]
pub fn timewarp_synth(
    a: &[f32],
    b: &[f32],
    fwd: &[f32],
    bwd: &[f32],
    (w, h): (usize, usize),
    (fw, fh): (usize, usize),
    phase: f32,
) -> Vec<f32> {
    const OCC_ABS: f32 = 1.5;
    const OCC_REL: f32 = 0.05;
    const EPS: f32 = 1e-4;
    if w == 0 || h == 0 || fw == 0 || fh == 0 {
        return Vec::new();
    }
    let (pa, pb) = (Plane { px: a, w, h }, Plane { px: b, w, h });
    let (pf, pw) = (
        Plane {
            px: fwd,
            w: fw,
            h: fh,
        },
        Plane {
            px: bwd,
            w: fw,
            h: fh,
        },
    );
    let len = |x: f32, y: f32| (x * x + y * y).sqrt();
    // How far each field has no partner in the other, or is not trusted.
    let unmatched = |f: [f32; 4], other: &Plane<'_>, x: f32, y: f32| -> f32 {
        let g = other.sample(x + f[0], y + f[1]);
        let thr = (OCC_REL * (len(f[0], f[1]) + len(g[0], g[1]))).max(OCC_ABS);
        let fail: f32 = if len(f[0] + g[0], f[1] + g[1]) > thr {
            1.0
        } else {
            0.0
        };
        fail.max(1.0 - f[2].clamp(0.0, 1.0))
    };
    let mut raw = vec![0.0f32; fw * fh * 4];
    for y in 0..fh {
        for x in 0..fw {
            let i = (y * fw + x) * 4;
            let (xi, yi) = (x as i64, y as i64);
            raw[i] = unmatched(pf.load(xi, yi), &pw, x as f32, y as f32);
            raw[i + 1] = unmatched(pw.load(xi, yi), &pf, x as f32, y as f32);
        }
    }
    // Grown by a pixel, the test missing a boundary's own pixels, then
    // averaged over 3 by 3 so the hand-over between the moments is gradual.
    let raw = Plane {
        px: &raw,
        w: fw,
        h: fh,
    };
    let mut grown = vec![0.0f32; fw * fh * 4];
    for y in 0..fh as i64 {
        for x in 0..fw as i64 {
            let mut sum = [0.0f32; 2];
            for cy in -1..=1 {
                for cx in -1..=1 {
                    let mut m = [0.0f32; 2];
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let v = raw.load(x + cx + dx, y + cy + dy);
                            m = [m[0].max(v[0]), m[1].max(v[1])];
                        }
                    }
                    sum = [sum[0] + m[0], sum[1] + m[1]];
                }
            }
            let i = (y as usize * fw + x as usize) * 4;
            grown[i] = sum[0] / 9.0;
            grown[i + 1] = sum[1] / 9.0;
        }
    }
    let grown = Plane {
        px: &grown,
        w: fw,
        h: fh,
    };
    let s = w as f32 / fw as f32;
    let motion = |field: &Plane<'_>, x: f32, y: f32| -> [f32; 2] {
        let f = field.sample(x / s, y / s);
        [f[0] * s, f[1] * s]
    };
    let mut out = vec![0.0f32; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32, y as f32);
            let f0 = motion(&pf, px, py);
            let f1 = motion(&pf, px - phase * f0[0], py - phase * f0[1]);
            let b0 = motion(&pw, px, py);
            let b1 = motion(&pw, px - (1.0 - phase) * b0[0], py - (1.0 - phase) * b0[1]);
            let sa = pa.sample(px - phase * f1[0], py - phase * f1[1]);
            let sb = pb.sample(px - (1.0 - phase) * b1[0], py - (1.0 - phase) * b1[1]);
            let m = grown.sample(px / s, py / s);
            let (la, lb) = (pa.load(x as i64, y as i64), pb.load(x as i64, y as i64));
            let wa = (1.0 - phase) * (1.0 - m[1]) + EPS;
            let wb = phase * (1.0 - m[0]) + EPS;
            let both = m[0] * m[1];
            let i = (y * w + x) * 4;
            for c in 0..4 {
                let plain = la[c] * (1.0 - phase) + lb[c] * phase;
                let warped = (wa * sa[c] + wb * sb[c]) / (wa + wb);
                out[i + c] = warped * (1.0 - both) + plain * both;
            }
        }
    }
    out
}

/// Timewarp's behaviour.
pub struct TimewarpDef;

impl EffectDef for TimewarpDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Timewarp as EffectMetadata>::SCHEMA
    }

    /// The frame of the input at or before the shown time, and the one after
    /// it when the picture sits between them or the motion blur needs the
    /// motion. Whole frames holds the earlier frame.
    fn own_moments(
        &self,
        e: &EffectInstance,
        lt: f64,
        markers: &MarkerContext,
    ) -> Option<OwnMoments> {
        let comp_fps = markers.fps;
        let t = source_time(e, lt, comp_fps)?;
        let rate = match e.float_at("input_rate", lt) {
            Some(r) if r >= 1.0 => r,
            _ => comp_fps.max(1.0),
        };
        let v = t * rate;
        // A whole frame often lands a hair under itself after the sums above,
        // and must still be that frame.
        let first = (v + 1e-6).floor();
        let a = first / rate;
        let phase = (v - first).max(0.0) as f32;
        let whole = matches!(e.param("method"), Some(EffectValue::Choice(0)));
        let blur = matches!(e.param("motion_blur"), Some(EffectValue::Bool(true)));
        let between = !whole && phase > 1e-4;
        Some(OwnMoments {
            a,
            b: (between || blur).then(|| ((first + 1.0) / rate, if whole { 0.0 } else { phase })),
        })
    }

    fn shows_own_moments(&self) -> bool {
        true
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let fps = cx.markers.fps.max(1.0);
        let phase = self
            .own_moments(cx.inst, cx.lt, cx.markers)
            .and_then(|m| m.b)
            .map_or(0.0, |(_, phase)| phase);
        let rate = match cx.inst.float_at("input_rate", cx.lt) {
            Some(r) if r >= 1.0 => r,
            _ => fps,
        };
        let advance = rate_of_change(cx.inst, cx.lt, fps) * rate / fps;
        push(Timewarp::DERIVED_PHASE, Value::Float(phase));
        push(Timewarp::DERIVED_ADVANCE, Value::Float(advance as f32));
    }

    fn presets(&self) -> &'static [EffectPreset] {
        &[
            EffectPreset {
                name: "Half speed",
                values: &[("speed", PresetValue::Float(50.0))],
            },
            EffectPreset {
                name: "Quarter speed",
                values: &[
                    ("speed", PresetValue::Float(25.0)),
                    ("quality", PresetValue::Choice(2)),
                ],
            },
            EffectPreset {
                name: "Slow motion with blur",
                values: &[
                    ("speed", PresetValue::Float(30.0)),
                    ("motion_blur", PresetValue::Bool(true)),
                ],
            },
            EffectPreset {
                name: "Speed ramp",
                values: &[(
                    "speed",
                    PresetValue::Keys(&[(0.0, 100.0), (0.5, 20.0), (1.5, 20.0), (2.0, 100.0)]),
                )],
            },
            EffectPreset {
                name: "Fast forward blur",
                values: &[
                    ("speed", PresetValue::Float(400.0)),
                    ("method", PresetValue::Choice(0)),
                    ("motion_blur", PresetValue::Bool(true)),
                ],
            },
        ]
    }

    /// One picture alone has no other moment to show, so the CPU rung shows
    /// the frame as it is. The moments reach the GPU pass as layer pictures.
    fn apply_cpu(&self, _rgba: &mut [f32], _w: u32, _h: u32, _p: Params<'_>) {}
}
