//! Audio level (node-graph.md §1.3): how loud the music is, as a number.
//!
//! **In plain terms.** It gives you the loudness at the moment being drawn —
//! one number for the whole sound, one for the low end alone, which is the one
//! that follows a kick drum rather than a hi-hat, one for the loudest single
//! sample in the window, and one for the top end, which follows a hi-hat. Wire
//! any of them into a scale, a glow or a brightness and the picture moves with
//! the track.
//!
//! **What it listens to is a choice of three.** The Source row picks between
//! the composition's own mix, one layer of it, and one clip of that layer. Left
//! on *This comp* it reads everything the mixer sums, at the layers' own
//! volumes, muted layers silent and a solo honoured, so a project whose music
//! arrives as four stems drives from the music rather than from whichever stem
//! was named. *Layer* reads the named layer's own contribution to that same
//! mix, post-fader, which is what a stem, a voice-over or a sound effect wants.
//! *Clip* narrows it once more, to one piece of sound on a Sequence layer,
//! with its own fades in what the reading gives.
//!
//! **An instance older than the Source row keeps what it had.** Before the row
//! existed a named layer was read straight off the file, raw and pre-fader, and
//! a parameter somebody drove with it must not change value because the schema
//! grew a control, so an instance still at version 1 and still saying *Layer*
//! goes on reading it that way (`backfill_builtin_params` writes its Source row
//! to say which of the two it was doing, and leaves the version alone). Pick
//! *This comp* or *Clip* on such an instance and it reads as it would anywhere
//! else: the pin is on the reading nobody chose, not on two pickers the panel
//! draws and lets you drag.
//!
//! **It reads a window, not an instant.** A single audio sample is a
//! meaningless number — sound is a wave, and a wave crosses zero constantly —
//! so the level is the root-mean-square over a short window centred on the
//! frame. That is a *temporal* read, declared as such
//! ([`EffectDef::driver_window`]) so the frame key folds the range in and a
//! cached frame cannot outlive the sound it was measured from.
//!
//! **Silence is the degrade.** No host tap, no footage, a comp nothing sounds
//! in, or a reference that names a layer or a clip somebody deleted: the level
//! reads nought, which is the same labelled no-op a dangling matte gives, never
//! a fault.

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, Port, PortType,
    Signature, Value,
};
use lumit_fx_macros::Effect;

/// Audio level's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_level",
    label = "Audio level",
    version = 2,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
    enabled_when = AUDIO_LEVEL_ENABLED_WHEN,
)]
pub struct AudioLevel {
    /// Which sound is measured: the composition's own mix, the layer the Audio
    /// row names, or one clip of that layer. Every one of the three is the same
    /// windowed mixdown over a different set of the mixer's own jobs
    /// (docs/impl/audio-nodes.md §2), so they cannot come apart from each other
    /// or from the sound a listener hears.
    #[choice(options = SOURCE_OPTIONS, default = 0)]
    pub source: u32,

    /// The layer *Layer* and *Clip* measure. An ordinary layer-reference
    /// parameter (docs/03 §8) with the usual degrade: a layer somebody deletes
    /// reads as silence, because it says which layer it wanted and the comp mix
    /// is not it. **Edges never cross layers**: the canvas draws the
    /// referenced layer as a derived source node and the wire from it renders
    /// this parameter, exactly as the image chain's wires render the stack.
    #[layer(label = "Audio", self_default = false)]
    pub audio: bool,

    /// The clip *Clip* measures, chosen from the clips of the layer above and
    /// no other layer's. Unset, or naming a clip that has gone, is silence:
    /// the row asked for one piece of sound, and the rest of the layer is not
    /// it.
    #[clip]
    pub clip: (),

    /// The width of the window the level is measured over, in seconds, centred
    /// on the frame. Short follows transients; long rides the tune.
    #[slider(
        min = 0.01,
        max = 1.0,
        default = 0.05,
        hard_min = 0.001,
        hard_max = 10.0,
        unit = Seconds
    )]
    pub window: f32,
}

/// The greyed rows: which picker the Source row has put in charge, said in the
/// panel rather than left to be found by dragging something inert.
pub const AUDIO_LEVEL_ENABLED_WHEN: &[EnabledWhen] = &[
    // The clip picker narrows a layer that is already being read, so on the
    // other two modes there is no layer for it to narrow.
    EnabledWhen {
        param: "clip",
        on: "source",
        cond: EnabledCond::ChoiceIs(SOURCE_CLIP),
    },
    // And the comp's own mix names no layer.
    EnabledWhen {
        param: "audio",
        on: "source",
        cond: EnabledCond::ChoiceIsNot(SOURCE_THIS_COMP),
    },
];

/// What the Source row offers, in the order it offers them.
pub const SOURCE_OPTIONS: [&str; 3] = ["This comp", "Layer", "Clip"];
/// Source: the composition's own mix, the whole of it.
pub const SOURCE_THIS_COMP: u32 = 0;
/// Source: one layer's contribution to that mix, post-fader.
pub const SOURCE_LAYER: u32 = 1;
/// Source: one clip of that layer, with its own fades.
pub const SOURCE_CLIP: u32 = 2;

/// The port the whole sound's level leaves by.
pub const AMPLITUDE_PORT: &str = "amplitude";
/// The port the low band's level leaves by.
pub const LOW_PORT: &str = "low";
/// The port the loudest sample in the window leaves by.
pub const PEAK_PORT: &str = "peak";
/// The port the top band's level leaves by.
pub const HIGH_PORT: &str = "high";

/// Where the low band stops, in hertz — a kick drum and a bass line, not a
/// snare. A constant rather than a control: it is the definition of "low", and
/// a second number to tune would only be a second thing to get wrong.
const LOW_BAND_HZ: f64 = 200.0;

/// Where the top band starts, in hertz: a hi-hat, a rim, the consonants of a
/// voice. A constant for the reason [`LOW_BAND_HZ`] is one.
const HIGH_BAND_HZ: f64 = 4000.0;

/// Audio level's behaviour.
pub struct AudioLevelDef;

impl EffectDef for AudioLevelDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioLevel as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: &[
                Port {
                    id: AMPLITUDE_PORT,
                    label: "Amplitude",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: LOW_PORT,
                    label: "Low",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: PEAK_PORT,
                    label: "Peak",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: HIGH_PORT,
                    label: "High",
                    ty: PortType::Number,
                    three_d: false,
                },
            ],
        }
    }

    fn driver_window(&self, p: crate::fx::Params<'_>) -> f64 {
        // Centred, so it reaches half the window either side of the frame.
        f64::from(AudioLevel::read(p).window.max(0.0)) / 2.0
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = AudioLevel::read(cx.params);
        let [amplitude, low, peak, high] = level(cx, f64::from(p.window.max(0.0)), p.source);
        push(AMPLITUDE_PORT, Value::Float(amplitude));
        push(LOW_PORT, Value::Float(low));
        push(PEAK_PORT, Value::Float(peak));
        push(HIGH_PORT, Value::Float(high));
    }
}

/// The window of samples the Source row asks for, and the rate they came at.
///
/// The three modes are the tap's one reading under its two filters
/// (docs/impl/audio-nodes.md §2). The fourth reading is the one an instance
/// older than the row keeps: a named layer straight off its file, raw and
/// pre-fader, which is what a parameter driven before the row existed was
/// following and must go on following.
fn read_window(
    cx: &DriverCx<'_>,
    tap: &dyn crate::fx::AudioTap,
    source: u32,
    half: f64,
    out: &mut Vec<f32>,
) -> Option<f64> {
    let layer = cx.inst.layer_ref("audio");
    // The pin is the version **and** the row: an instance older than the row
    // goes on reading a named layer raw for as long as its Source still says
    // Layer, and reads through the row the moment somebody picks another mode.
    // Pinning the whole instance instead would leave two pickers drawn, live
    // and inert for ever. The absent row counts as Layer here because a saved
    // graph node reaches the render without the backfill's row at all, and the
    // schema default would otherwise read as This comp.
    let pinned = cx.inst.effect.version < 2
        && cx
            .inst
            .param("source")
            .is_none_or(|v| matches!(v, crate::model::EffectValue::Choice(SOURCE_LAYER)));
    if pinned {
        if let Some(l) = layer {
            return tap.samples(l, cx.lt - half, cx.lt + half, out);
        }
    }
    match source {
        // A row that names no layer is a row that named nothing: silence, the
        // same degrade a dangling reference gives.
        SOURCE_LAYER => layer.and_then(|l| tap.strip(Some(l), None, half, out)),
        SOURCE_CLIP => layer
            .zip(cx.inst.clip_ref("clip"))
            .and_then(|(l, c)| tap.strip(Some(l), Some(c), half, out)),
        // The comp's mix, and the tap centres that window itself: the comp's
        // clock is the host's, not this layer's.
        _ => tap.mix(half, out),
    }
}

/// The windowed measurements of the chosen sound: amplitude, low band, peak
/// and high band, in the order the ports are declared.
///
/// Deterministic per (source, time, window): the same samples through the same
/// sums give the same four numbers, on any machine and in any render.
fn level(cx: &DriverCx<'_>, window: f64, source: u32) -> [f32; 4] {
    let Some(tap) = cx.audio else {
        return [0.0; 4];
    };
    if window <= 0.0 {
        return [0.0; 4];
    }
    // ponytail: one Vec per driver per frame. A window is milliseconds of
    // mono audio; give the tap a reusable buffer if a profile ever shows it.
    let mut samples = Vec::new();
    let Some(rate) = read_window(cx, tap, source, window / 2.0, &mut samples) else {
        return [0.0; 4];
    };
    if samples.is_empty() || rate <= 0.0 {
        return [0.0; 4];
    }

    // The whole sound, and the same sound through two one-pole filters: a
    // low-pass at LOW_BAND_HZ, and what a low-pass at HIGH_BAND_HZ leaves
    // behind, which is the top band. One pole rather than a proper filter bank
    // because the outputs are numbers a picture follows, not audio anybody
    // listens to.
    let alpha = |hz: f64| {
        let rc = 1.0 / (std::f64::consts::TAU * hz);
        let dt = 1.0 / rate;
        (dt / (rc + dt)) as f32
    };
    let (low_alpha, high_alpha) = (alpha(LOW_BAND_HZ), alpha(HIGH_BAND_HZ));
    let mut sum = 0.0f64;
    let mut low_sum = 0.0f64;
    let mut high_sum = 0.0f64;
    let mut low_pole = 0.0f32;
    let mut high_pole = 0.0f32;
    let mut peak = 0.0f32;
    for &s in &samples {
        sum += f64::from(s) * f64::from(s);
        // The loudest single sample, which is the transient an RMS smooths
        // away: a snare hit reads here and barely moves Amplitude.
        peak = peak.max(s.abs());
        low_pole += low_alpha * (s - low_pole);
        low_sum += f64::from(low_pole) * f64::from(low_pole);
        high_pole += high_alpha * (s - high_pole);
        let top = s - high_pole;
        high_sum += f64::from(top) * f64::from(top);
    }
    let n = samples.len() as f64;
    [
        (sum / n).sqrt() as f32,
        (low_sum / n).sqrt() as f32,
        peak,
        (high_sum / n).sqrt() as f32,
    ]
}
