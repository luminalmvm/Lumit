//! Beat trigger: the music as events other effects react to.
//!
//! **In plain terms.** Audio level says how loud the sound is. This says
//! *when something happens*: a beat marker passing, a beat found in the sound,
//! or the level of one band rising past a threshold. Each event fires four
//! outputs. **Pulse** is 1 on the frame of an event and 0 on every other,
//! which is what a burst listens for (it fires on a rising edge past 0.5).
//! **Count** is how many events there have been. **Envelope** jumps to 1 at
//! each event and falls away over Decay, Flash's own fall, for a pop that
//! settles. **Time since last** is seconds since the last one.
//!
//! **Every answer is worked out from the start each frame**, never carried
//! from the frame before, so scrubbing to a moment and playing up to it reach
//! the same numbers, and so does the export. The sound is read once and kept by
//! the host ([`crate::fx::AudioTap::onsets`]); what each frame pays for is the
//! picking, at the Sensitivity, Minimum gap and Threshold each moment was keyed
//! to. A wire into one of those rows is read at this frame only, and the
//! moments before it read their keyframes.
//!
//! **No sound, no markers or no host is no events**: Pulse, Count and Envelope
//! read nought and Time since last reads the layer's own time.

use crate::fx::drivers::audio_level::{
    SOURCE_CLIP, SOURCE_LAYER, SOURCE_OPTIONS, SOURCE_THIS_COMP,
};
use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, MarkerContext,
    Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// This driver's catalogue name.
pub const MATCH_NAME: &str = "beat_trigger";

/// What the Events row offers.
pub const EVENT_OPTIONS: [&str; 3] = ["Beat markers", "Detected beats", "Level crossing"];
/// Events: the comp's beat markers.
pub const EVENTS_MARKERS: u32 = 0;
/// Events: beats found in the sound.
pub const EVENTS_DETECTED: u32 = 1;
/// Events: the band's level rising past Threshold.
pub const EVENTS_LEVEL: u32 = 2;

/// What the Band row offers: the whole sound, below 200 Hz, above 4 kHz.
pub const BAND_OPTIONS: [&str; 3] = ["All", "Low", "High"];

const OUTPUTS: &[Port] = &[
    Port::new("pulse", "Pulse", PortType::Number),
    Port::new("count", "Count", PortType::Number),
    Port::new("envelope", "Envelope", PortType::Number),
    Port::new("since", "Time since last", PortType::Number),
];

/// Beat trigger's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "beat_trigger",
    label = "Beat trigger",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
    enabled_when = BEAT_TRIGGER_ENABLED_WHEN,
)]
pub struct BeatTrigger {
    /// What counts as an event.
    #[choice(label = "Events", options = EVENT_OPTIONS, default = 0)]
    pub events: u32,

    /// Which sound is listened to, as Audio level's Source row.
    #[choice(options = SOURCE_OPTIONS, default = 0)]
    pub source: u32,

    /// The layer *Layer* and *Clip* listen to.
    #[layer(label = "Audio", self_default = false)]
    pub audio: bool,

    /// The clip *Clip* listens to.
    #[clip]
    pub clip: (),

    /// The part of the sound listened to.
    #[choice(options = BAND_OPTIONS, default = 0)]
    pub band: u32,

    /// How readily a beat is found, as the Beats panel's slider.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub sensitivity: f32,

    /// Of two beats closer than this, the stronger stands.
    #[slider(
        label = "Minimum gap",
        min = 0.0,
        max = 1.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Seconds
    )]
    pub min_gap: f32,

    /// The level the band has to rise past.
    #[slider(min = 0.0, max = 1.0, default = 0.2, hard_min = 0.0, unit = Raw)]
    pub threshold: f32,

    /// Frames after a crossing before the next can fire.
    #[slider(
        label = "Hold off",
        min = 0.0,
        max = 30.0,
        default = 4.0,
        hard_min = 0.0,
        unit = Frames
    )]
    pub hold_off: f32,

    /// How long Envelope takes to fall to about a third.
    #[slider(
        min = 0.01,
        max = 2.0,
        default = 0.25,
        hard_min = 0.0,
        unit = Seconds
    )]
    pub decay: f32,
}

/// The rows each Events choice leaves live.
pub const BEAT_TRIGGER_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "source",
        on: "events",
        cond: EnabledCond::ChoiceIsNot(EVENTS_MARKERS),
    },
    EnabledWhen {
        param: "audio",
        on: "source",
        cond: EnabledCond::ChoiceIsNot(SOURCE_THIS_COMP),
    },
    EnabledWhen {
        param: "clip",
        on: "source",
        cond: EnabledCond::ChoiceIs(SOURCE_CLIP),
    },
    EnabledWhen {
        param: "band",
        on: "events",
        cond: EnabledCond::ChoiceIsNot(EVENTS_MARKERS),
    },
    EnabledWhen {
        param: "sensitivity",
        on: "events",
        cond: EnabledCond::ChoiceIs(EVENTS_DETECTED),
    },
    EnabledWhen {
        param: "min_gap",
        on: "events",
        cond: EnabledCond::ChoiceIs(EVENTS_DETECTED),
    },
    EnabledWhen {
        param: "threshold",
        on: "events",
        cond: EnabledCond::ChoiceIs(EVENTS_LEVEL),
    },
    EnabledWhen {
        param: "hold_off",
        on: "events",
        cond: EnabledCond::ChoiceIs(EVENTS_LEVEL),
    },
];

/// Beat trigger's behaviour.
pub struct BeatTriggerDef;

impl EffectDef for BeatTriggerDef {
    fn schema(&self) -> &'static EffectSchema {
        &<BeatTrigger as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: OUTPUTS,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = BeatTrigger::read(cx.params);
        let (fps, events) = events(cx, &p);
        let lt = cx.lt;
        let last = events.last().copied();
        // An event belongs to the one frame whose span it falls in.
        let frame = if fps > 0.0 { 1.0 / fps } else { 0.0 };
        let pulse = last.is_some_and(|e| lt - e < frame || e == lt);
        let envelope = last.map_or(0.0, |e| {
            crate::fx::decayed_hit(lt - e, f64::from(p.decay.max(0.0)))
        });
        let out = [
            f32::from(u8::from(pulse)),
            events.len() as f32,
            envelope as f32,
            last.map_or(lt.max(0.0), |e| lt - e) as f32,
        ];
        for (port, v) in OUTPUTS.iter().zip(out) {
            push(port.id, Value::Float(v));
        }
    }
}

/// The comp's frame rate, and every event up to `cx.lt`, in this layer's time,
/// in order.
fn events(cx: &DriverCx<'_>, p: &BeatTrigger) -> (f64, Vec<f64>) {
    let ctx = cx.context;
    let Some(comp) = ctx.comp.and_then(|id| ctx.document.comp(id)) else {
        return (0.0, Vec::new());
    };
    let fps = comp.frame_rate.fps();
    let own = ctx
        .layer
        .and_then(|id| comp.layers.iter().find(|l| l.id == id));
    let lt = cx.lt;
    if p.events == EVENTS_MARKERS {
        let markers = match own {
            Some(layer) => MarkerContext::for_layer(comp, layer),
            None => MarkerContext::for_comp(comp),
        };
        let seen = markers.beats.partition_point(|b| *b <= lt);
        return (fps, markers.beats[..seen].to_vec());
    }
    let Some(tap) = cx.audio else {
        return (fps, Vec::new());
    };
    let layer = cx.inst.layer_ref("audio");
    let (layer, clip) = match p.source {
        SOURCE_LAYER => (layer, None),
        SOURCE_CLIP => (layer, cx.inst.clip_ref("clip")),
        _ => (None, None),
    };
    // A row that names nothing is listening to nothing.
    if p.source != SOURCE_THIS_COMP
        && (layer.is_none() || (p.source == SOURCE_CLIP && clip.is_none()))
    {
        return (fps, Vec::new());
    }
    // The sound runs on the comp's clock, this layer on its own.
    let offset = own.map_or(0.0, |l| l.start_offset.0.to_f64());
    let at = |id: &str, comp_t: f64, or: f32| {
        cx.inst
            .float_at(id, comp_t - offset)
            .unwrap_or(f64::from(or))
    };
    let mut found = Vec::new();
    let mut events = Vec::new();
    if p.events == EVENTS_DETECTED {
        let sensitivity = |t: f64| at("sensitivity", t, p.sensitivity);
        if tap.onsets(layer, clip, p.band, lt + offset, &sensitivity, &mut found) {
            // Of two beats closer than the gap, the stronger stands: the
            // Beats panel's own rule.
            let mut kept: Vec<(f64, f32)> = Vec::with_capacity(found.len());
            for beat in found {
                match kept.last_mut() {
                    Some(last) if beat.0 - last.0 < at("min_gap", beat.0, p.min_gap) => {
                        if beat.1 > last.1 {
                            *last = beat;
                        }
                    }
                    _ => kept.push(beat),
                }
            }
            events.extend(kept.iter().map(|(t, _)| t - offset));
        }
    } else if tap.levels(layer, clip, p.band, lt + offset, &mut found) {
        let mut below = true;
        let mut last: Option<f64> = None;
        for (t, level) in found {
            let over = f64::from(level) >= at("threshold", t, p.threshold);
            let ready =
                last.is_none_or(|l| fps <= 0.0 || t - l >= at("hold_off", t, p.hold_off) / fps);
            if over && below && ready {
                last = Some(t);
                events.push(t - offset);
            }
            below = !over;
        }
    }
    events.retain(|e| *e <= lt);
    (fps, events)
}
