//! Audio level (K-471 §1.3): how loud the music is, as a number.
//!
//! **In plain terms.** It gives you the loudness at the moment being drawn —
//! one number for the whole sound, and a second for the low end alone, which
//! is the one that follows a kick drum rather than a hi-hat. Wire either into
//! a scale, a glow or a brightness and the picture moves with the track.
//!
//! **What it listens to is a choice of two shapes.** Left on *This comp*, it
//! reads the composition's own mix — everything the mixer sums, at the layers'
//! own volumes, muted layers silent and a solo honoured — so a project whose
//! music arrives as four stems drives from the music rather than from whichever
//! stem was named. Point it at a layer instead and it reads that layer alone,
//! which is what a stem, a voice-over or a sound effect wants.
//!
//! **It reads a window, not an instant.** A single audio sample is a
//! meaningless number — sound is a wave, and a wave crosses zero constantly —
//! so the level is the root-mean-square over a short window centred on the
//! frame. That is a *temporal* read, declared as such
//! ([`EffectDef::driver_window`]) so the frame key folds the range in and a
//! cached frame cannot outlive the sound it was measured from.
//!
//! **Silence is the degrade.** No host tap, no footage, a comp nothing sounds
//! in, or a reference that names a layer somebody deleted: the level reads
//! nought, which is the same labelled no-op a dangling matte gives, never a
//! fault.

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Audio level's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_level",
    label = "Audio level",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioLevel {
    /// What is measured: **unset is the composition's own mix**, and naming a
    /// layer reads that layer alone. An ordinary layer-reference parameter
    /// (docs/03 §8) whose empty entry carries a meaning rather than nothing —
    /// the picker reads *This comp* — so the driver as dropped already follows
    /// the track. A named layer somebody then deletes still degrades to
    /// silence: it says which layer it wanted, and the comp mix is not it.
    /// **Edges never cross layers** (K-471): the canvas draws the
    /// referenced layer as a derived source node and the wire from it renders
    /// this parameter, exactly as the image chain's wires render the stack.
    #[layer(label = "Audio", self_default = false)]
    pub audio: bool,

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

/// The port the whole sound's level leaves by.
pub const AMPLITUDE_PORT: &str = "amplitude";
/// The port the low band's level leaves by.
pub const LOW_PORT: &str = "low";

/// Where the low band stops, in hertz — a kick drum and a bass line, not a
/// snare. A constant rather than a control: it is the definition of "low", and
/// a second number to tune would only be a second thing to get wrong.
const LOW_BAND_HZ: f64 = 200.0;

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
            ],
        }
    }

    fn driver_window(&self, p: crate::fx::Params<'_>) -> f64 {
        // Centred, so it reaches half the window either side of the frame.
        f64::from(AudioLevel::read(p).window.max(0.0)) / 2.0
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = AudioLevel::read(cx.params);
        let (amplitude, low) = level(cx, f64::from(p.window.max(0.0)));
        push(AMPLITUDE_PORT, Value::Float(amplitude));
        push(LOW_PORT, Value::Float(low));
    }
}

/// The windowed RMS of the chosen sound — the comp's mix, or one layer's —
/// whole and low-band.
///
/// Deterministic per (audio, time, window): the same samples through the same
/// two sums give the same two numbers, on any machine and in any render.
fn level(cx: &DriverCx<'_>, window: f64) -> (f32, f32) {
    let Some(tap) = cx.audio else {
        return (0.0, 0.0);
    };
    if window <= 0.0 {
        return (0.0, 0.0);
    }
    // ponytail: one Vec per driver per frame. A window is milliseconds of
    // mono audio; give the tap a reusable buffer if a profile ever shows it.
    let mut samples = Vec::new();
    // Unset is the comp's mix, and the tap centres that window itself: the
    // comp's clock is the host's, not this layer's (K-657).
    let read = match cx.inst.layer_ref("audio") {
        Some(layer) => tap.samples(
            layer,
            cx.lt - window / 2.0,
            cx.lt + window / 2.0,
            &mut samples,
        ),
        None => tap.mix(window / 2.0, &mut samples),
    };
    let Some(rate) = read else {
        return (0.0, 0.0);
    };
    if samples.is_empty() || rate <= 0.0 {
        return (0.0, 0.0);
    }

    // The whole sound, and the same sound through a one-pole low-pass at
    // LOW_BAND_HZ. One pole rather than a proper filter bank because the output
    // is one number a picture follows, not audio anybody listens to.
    let alpha = {
        let rc = 1.0 / (std::f64::consts::TAU * LOW_BAND_HZ);
        let dt = 1.0 / rate;
        (dt / (rc + dt)) as f32
    };
    let mut sum = 0.0f64;
    let mut low_sum = 0.0f64;
    let mut lp = 0.0f32;
    for &s in &samples {
        sum += f64::from(s) * f64::from(s);
        lp += alpha * (s - lp);
        low_sum += f64::from(lp) * f64::from(lp);
    }
    let n = samples.len() as f64;
    ((sum / n).sqrt() as f32, (low_sum / n).sqrt() as f32)
}
