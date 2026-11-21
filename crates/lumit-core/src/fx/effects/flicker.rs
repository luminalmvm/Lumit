//! Flicker: the light on the layer wavers from frame to frame, as a neon tube,
//! a failing bulb, a film projector or an old screen does.
//!
//! One stream of seeded noise over time drives the light, the contrast and the
//! opacity together, so they dip and flash as one. The colour has three streams
//! of its own, one a channel, so the tint wanders apart from the light. The
//! noise is read once a frame, when the effect is applied, and both render
//! paths are handed the same few gains, so each pixel is only a multiply or two.

use crate::fx::{
    noise::value3, params::ParamId, params::Params, params::Value, presets::EffectPreset,
    presets::PresetValue, registry::EffectDef, registry::EffectMetadata, registry::ResolveCx,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// The grey Contrast turns about, scene-linear.
pub const FLICKER_PIVOT: f32 = 0.18;

/// The furthest the pattern is read along, in flickers.
const PHASE_LIMIT: f64 = 1.0e7;

/// Motion's Hold: each value is held for one flicker, then jumps.
const MOTION_HOLD: u32 = 1;

/// Flicker's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "flicker",
    label = "Flicker",
    version = 1,
    category = Stylise,
    // A multiply or two a pixel.
    cost = Trivial,
    roi = Exact,
    // The light moves with time under fixed controls.
    seeded = true,
    premultiplied = true,
)]
pub struct Flicker {
    /// How far the light moves, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub amount: f32,

    /// Flickers a second.
    #[slider(min = 0.0, max = 60.0, default = 12.0, hard_min = 0.0, unit = Raw)]
    pub speed: f32,

    /// Per instance, so two layers do not flicker together.
    #[seed]
    pub seed: u32,

    /// Smooth eases from one value to the next. Hold keeps each value for one
    /// flicker, then jumps.
    #[choice(options = ["Smooth", "Hold"], default = 0)]
    pub motion: u32,

    /// Moves the pattern in time, seconds, so two layers on one seed differ.
    #[slider(label = "Time offset", min = -10.0, max = 10.0, default = 0.0, unit = Seconds)]
    pub time_offset: f32,

    /// Below 0 the light mostly dips under the layer's own, above 0 it mostly
    /// flashes over it. 0 goes both ways evenly.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub bias: f32,

    /// How much the brightness flickers, per cent of Amount.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub light: f32,

    /// How much each channel flickers on its own, per cent of Amount.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub colour: f32,

    /// How much the contrast flickers about a mid grey, per cent of Amount.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub contrast: f32,

    /// How much the opacity dips, per cent of Amount.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub opacity: f32,

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

/// One frame of Flicker, as both render paths draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlickerParams {
    /// Light times Colour, per channel.
    pub gain: [f32; 3],
    /// The contrast about [`FLICKER_PIVOT`]. 1 leaves it.
    pub contrast: f32,
    /// 0..1, times the whole pixel.
    pub opacity: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl FlickerParams {
    /// Nothing moves this frame: the picture goes through untouched, bit for bit.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.mix == 0.0 || (self.gain == [1.0; 3] && self.contrast == 1.0 && self.opacity == 1.0)
    }
}

impl Flicker {
    /// Flickers so far, this frame. Never a panel row.
    pub const DERIVED_PHASE: ParamId = ParamId::new("derived.phase");
    /// The whole flickers so far, worked out in f64 so Hold never slips a frame.
    pub const DERIVED_STEP: ParamId = ParamId::new("derived.step");

    /// The noise on stream `channel` at this frame, -1..1. Hold reads the same
    /// lattice at the start of the flicker, so both motions follow one pattern.
    fn wave(self, phase: f32, step: f32, channel: u32) -> f32 {
        let x = if self.motion == MOTION_HOLD {
            step
        } else {
            phase
        };
        value3(self.seed, channel, x, 0.0, 0.0, 0)
    }

    /// The gains both kernels read, from the rows and the time in `p`.
    #[must_use]
    pub fn packed(self, p: Params<'_>) -> FlickerParams {
        let phase = p.float(Self::DERIVED_PHASE, 0.0);
        let step = p.float(Self::DERIVED_STEP, 0.0);
        let amount = self.amount.max(0.0) / 100.0;
        let share = |pct: f32| amount * pct.max(0.0) / 100.0;
        // Bias folds the swing onto one side and keeps its size, so at -100
        // every flicker is a dip and at 100 every one is a flash.
        let b = (self.bias / 100.0).clamp(-1.0, 1.0);
        let n = self.wave(phase, step, 0);
        let s = n + (b.signum() * n.abs() - n) * b.abs();
        let light = (1.0 + share(self.light) * s).max(0.0);
        let tint = share(self.colour);
        let gain = [1, 2, 3].map(|c| light * (1.0 + tint * self.wave(phase, step, c)).max(0.0));
        FlickerParams {
            gain,
            contrast: (1.0 + share(self.contrast) * s).max(0.0),
            opacity: (1.0 + share(self.opacity) * s).clamp(0.0, 1.0),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// The CPU oracle, on premultiplied pixels. The WGSL twin is the same
/// expression in the same order.
pub fn flicker(rgba: &mut [f32], p: &FlickerParams) {
    if p.is_identity() {
        return;
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let grey = FLICKER_PIVOT * a;
        let mut out = [0.0f32; 4];
        for (c, o) in out.iter_mut().take(3).enumerate() {
            *o = (grey + (px[c] * p.gain[c] - grey) * p.contrast).max(0.0) * p.opacity;
        }
        out[3] = a * p.opacity;
        for (v, o) in px.iter_mut().zip(out) {
            *v += p.mix * (o - *v);
        }
    }
}

/// Flicker's looks.
pub const FLICKER_LOOKS: &[EffectPreset] = &[
    EffectPreset {
        name: "Neon tube",
        values: &[
            ("amount", PresetValue::Float(70.0)),
            ("speed", PresetValue::Float(20.0)),
            ("motion", PresetValue::Choice(MOTION_HOLD)),
            ("bias", PresetValue::Float(-70.0)),
        ],
    },
    EffectPreset {
        name: "Failing light",
        values: &[
            ("amount", PresetValue::Float(100.0)),
            ("speed", PresetValue::Float(4.0)),
            ("bias", PresetValue::Float(-90.0)),
        ],
    },
    EffectPreset {
        name: "Film projector",
        values: &[
            ("amount", PresetValue::Float(12.0)),
            ("speed", PresetValue::Float(24.0)),
            ("contrast", PresetValue::Float(40.0)),
        ],
    },
    EffectPreset {
        name: "CRT",
        values: &[
            ("amount", PresetValue::Float(10.0)),
            ("speed", PresetValue::Float(50.0)),
            ("motion", PresetValue::Choice(MOTION_HOLD)),
            ("colour", PresetValue::Float(30.0)),
        ],
    },
];

/// Flicker's behaviour.
pub struct FlickerDef;

impl EffectDef for FlickerDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Flicker as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [EffectPreset] {
        FLICKER_LOOKS
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let fl = |id: &str, or: f64| {
            cx.inst
                .float_at_with_context(id, cx.lt, cx.context.clone())
                .unwrap_or(or)
        };
        // Held well inside what the noise lattice can count.
        let phase = ((cx.lt + fl("time_offset", 0.0)) * fl("speed", 12.0).max(0.0))
            .clamp(-PHASE_LIMIT, PHASE_LIMIT);
        push(Flicker::DERIVED_PHASE, Value::Float(phase as f32));
        push(Flicker::DERIVED_STEP, Value::Float(phase.floor() as f32));
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        flicker(rgba, &Flicker::read(p).packed(p));
    }
}
