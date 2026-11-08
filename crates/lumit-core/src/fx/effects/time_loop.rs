//! Time loop plays a stretch of another layer over and over.
//!
//! Source layer is drawn as it is at a moment inside the loop, with its own
//! effects, the drivers wired into them and, under Map placement As placed in
//! comp, its transform at that moment, so everything about it loops
//! together. The loop runs on the clock of the layer the effect is on: from
//! Start, Duration long. Type says how it comes round. **Cycle** starts
//! again, with a cut where the end and the start differ. **Ping-pong** plays
//! forwards and then backwards. **Crossfade Bastiuni** never cuts: for the
//! first Fade length of each lap it fades from the source one lap later into
//! the source now, so the motion keeps going forwards across the join. The
//! source then has to be animated for Duration and Fade length together, and
//! is drawn twice while it fades.
//!
//! The looped picture is laid over the layer's own, so the effect is usually
//! put on a clear solid. Hide the Source layer so only the loop shows.

use crate::fx::{
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, registry::Signature, schema::EffectSchema, schema::EnabledCond,
    schema::EnabledWhen,
};
use crate::model::{Composition, EffectInstance, EffectValue};
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// The ways round, in Choice order.
pub const LOOP_TYPE_OPTIONS: &[&str] = &["Cycle", "Ping-pong", "Crossfade Bastiuni"];
const PING_PONG: u32 = 1;
const CROSSFADE: u32 = 2;

/// The two fade curves, in Choice order.
pub const FADE_CURVE_OPTIONS: &[&str] = &["Linear", "Smooth"];

/// The fade rows are read only by Crossfade Bastiuni.
pub const TIME_LOOP_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "fade_length",
        on: "loop_type",
        cond: EnabledCond::ChoiceIs(CROSSFADE),
    },
    EnabledWhen {
        param: "fade_curve",
        on: "loop_type",
        cond: EnabledCond::ChoiceIs(CROSSFADE),
    },
];

/// Time loop's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "time_loop",
    label = "Time loop",
    version = 1,
    category = Temporal,
    cost = Heavy,
    roi = FullFrame,
    premultiplied = true,
    enabled_when = TIME_LOOP_ENABLED_WHEN,
)]
pub struct TimeLoop {
    /// The layer whose stretch is looped.
    #[layer(label = "Source layer")]
    pub source: bool,

    /// Where the source sits on this layer. As placed in comp follows the
    /// source's own transform at the looped moment, so its movement loops
    /// too.
    #[choice(
        label = "Map placement",
        options = *crate::fx::builtins::MAP_PLACEMENT_OPTIONS,
        default = 0
    )]
    pub source_placement: u32,

    /// How the loop comes round.
    #[choice(label = "Type", options = LOOP_TYPE_OPTIONS, default = 0)]
    pub loop_type: u32,

    /// Where the looped stretch begins, seconds on this layer's clock.
    #[slider(label = "Start", min = 0.0, max = 60.0, default = 0.0, unit = Seconds)]
    pub start: f32,

    /// How long the looped stretch is, seconds.
    #[slider(
        label = "Duration",
        min = 0.1,
        max = 60.0,
        default = 2.0,
        hard_min = 0.01,
        unit = Seconds
    )]
    pub duration: f32,

    /// Crossfade Bastiuni: how long each lap fades from the next lap in,
    /// seconds, up to Duration.
    #[slider(
        label = "Fade length",
        min = 0.0,
        max = 10.0,
        default = 0.5,
        hard_min = 0.0,
        unit = Seconds
    )]
    pub fade_length: f32,

    /// Crossfade Bastiuni: the fade at an even pace, or eased at both ends.
    #[choice(label = "Fade curve", options = FADE_CURVE_OPTIONS, default = 0)]
    pub fade_curve: u32,

    /// The Mix every effect ends with, per cent.
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

/// The moments a frame shows: the source at `now`, and while a crossfade
/// runs the source at `next` too, mixed in by `next_share`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moments {
    pub now: f64,
    pub next: Option<f64>,
    pub next_share: f32,
}

impl TimeLoop {
    /// How much of the lap after this one shows, read back by the draw.
    pub const DERIVED_NEXT_SHARE: ParamId = ParamId::new("derived.next_share");

    /// The moments, on this layer's clock, that the frame at layer time `lt`
    /// shows.
    #[must_use]
    pub fn moments(self, lt: f64) -> Moments {
        let start = f64::from(self.start);
        let d = f64::from(self.duration.max(0.01));
        let plain = |at: f64| Moments {
            now: at,
            next: None,
            next_share: 0.0,
        };
        match self.loop_type {
            PING_PONG => {
                let u = (lt - start).rem_euclid(2.0 * d);
                plain(start + if u < d { u } else { 2.0 * d - u })
            }
            CROSSFADE => {
                let tau = (lt - start).rem_euclid(d);
                let fade = f64::from(self.fade_length).clamp(0.0, d);
                if tau >= fade || fade <= 0.0 {
                    return plain(start + tau);
                }
                let x = (tau / fade) as f32;
                let into = if self.fade_curve == 1 {
                    x * x * (3.0 - 2.0 * x)
                } else {
                    x
                };
                Moments {
                    now: start + tau,
                    next: Some(start + tau + d),
                    next_share: 1.0 - into,
                }
            }
            // Cycle.
            _ => plain(start + (lt - start).rem_euclid(d)),
        }
    }

    /// The source layer and the composition times its pictures are made at,
    /// for the frame at comp time `t` on a layer whose time is `lt`: one, or
    /// two while a crossfade runs. Empty with no source. The planner, the
    /// builder and the frame key all ask here, so they agree.
    #[must_use]
    pub fn planned(e: &EffectInstance, comp: &Composition, t: f64, lt: f64) -> Vec<(Uuid, f64)> {
        let Some(source) = e.layer_ref("source") else {
            return Vec::new();
        };
        if !comp.layers.iter().any(|l| l.id == source) {
            return Vec::new();
        }
        let m = Self::stored(e, lt).moments(lt);
        std::iter::once(m.now)
            .chain(m.next)
            .map(|at| (source, t + (at - lt)))
            .collect()
    }

    /// The rows that decide the moments, read off the stored instance at
    /// layer time `lt`.
    #[must_use]
    pub fn stored(e: &EffectInstance, lt: f64) -> Self {
        let mut c = Self::read(Params::EMPTY);
        let choice = |id: &str, or: u32| match e.param(id) {
            Some(EffectValue::Choice(v)) => *v,
            _ => or,
        };
        let number = |id: &str, or: f32| {
            e.float_at(id, lt)
                .filter(|v| v.is_finite())
                .map_or(or, |v| v as f32)
        };
        c.loop_type = choice("loop_type", c.loop_type);
        c.fade_curve = choice("fade_curve", c.fade_curve);
        c.start = number("start", c.start);
        c.duration = number("duration", c.duration);
        c.fade_length = number("fade_length", c.fade_length);
        c
    }
}

/// Time loop's behaviour: the pictures come from the builder, laid over the
/// layer's own on the card.
pub struct TimeLoopDef;

impl EffectDef for TimeLoopDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TimeLoop as EffectMetadata>::SCHEMA
    }

    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: &[],
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let m = TimeLoop::stored(cx.inst, cx.lt).moments(cx.lt);
        push(TimeLoop::DERIVED_NEXT_SHARE, Value::Float(m.next_share));
    }
}
