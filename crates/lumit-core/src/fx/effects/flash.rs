//! Flash (docs/08 §3.7): the beat-aware strobe.
//!
//! **In plain terms.** Every other migrated effect is fully described by its
//! controls — you can read its sliders and know what it will draw. This one
//! cannot be: how bright the flash is *right now* depends on what time it is,
//! which beats the composition carries, and the shape of a whole keyframed
//! Trigger track. None of those is a control, so the strength is worked out at
//! resolve time through the one hook that sees them ([`EffectDef::
//! resolve_derived`]) and handed to the kernel as a single number, exactly
//! as the hand-written resolve arm handed it over before.

use crate::fx::markers::flash_nth;
use crate::fx::{
    cpu, flash_beat_envelope, flash_envelope, EffectDef, EffectMetadata, EffectSchema, ParamId,
    Params, ResolveCx, Value,
};
use crate::model::EffectValue;
use lumit_fx_macros::Effect;

/// Flash's controls.
///
/// Manual mode is the original manual form: each keyframe on Trigger is a hit
/// (its value = how hard, 0..1) that decays exponentially over Decay; a static
/// Trigger holds a constant flash. Trigger mode fires the §1.4 envelope from the
/// comp's beat markers; Strobe fires every Nth beat only. Instances saved before
/// the marker modes existed carry no `mode` parameter and resolve as Manual,
/// byte-identically. The default is a no-op by design: §1.2 exempts inherently
/// trigger-driven effects.
///
/// Nothing here is spatial — a flash is a per-pixel blend toward a colour — so
/// every parameter is `Raw` and the old `rescale_px` listed this effect among the
/// arms that did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "flash",
    label = "Flash",
    version = 1,
    category = Stylise,
    cost = Trivial,
    roi = Exact,
    beat_input = true, // binds to comp beat markers per §1.4
)]
pub struct Flash {
    /// Manual = keyframed hits on Trigger (the original form); Trigger = the
    /// §1.4 beat envelope; Strobe = every Nth beat only.
    #[choice(options = ["Manual", "Trigger", "Strobe"], default = 0)]
    pub mode: u32,

    /// Manual mode's hit track: each keyframe is a hit of that strength.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub trigger: f32,

    /// Frames (comp-rate, §2.3) a marker-driven flash lasts. Hard floor 0,
    /// unbounded above (a one-sided clamp); 0 is honestly a flash zero
    /// frames long — never shown.
    #[slider(min = 0.0, max = 12.0, default = 2.0, hard_min = 0.0, unit = Frames)]
    pub duration: f32,

    /// Hard holds full strength for Duration then cuts; Fade decays linearly to
    /// zero across it.
    #[choice(options = ["Hard", "Fade"], default = 0)]
    pub shape: u32,

    /// Strobe mode: fire beats 0, N, 2N, … of the comp's beat list. The spec's
    /// integer ≥ 1, carried as a Float row — the resolver rounds and clamps at 1.
    #[slider(
        label = "Every Nth beat",
        min = 1.0,
        max = 8.0,
        default = 1.0,
        hard_min = 1.0,
        unit = Raw
    )]
    pub every_nth: f32,

    /// Frames a marker-driven flash trails (> 0) or leads (< 0) its beat.
    #[slider(label = "Phase offset", min = -8.0, max = 8.0, default = 0.0, unit = Frames)]
    pub phase: f32,

    /// Per cent scale on the trigger envelope.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub intensity: f32,

    /// The flash colour, scene-linear (alpha unused: the flash respects the
    /// layer's own footprint). Linear light, so HDR flashes are legal.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// Milliseconds for a Manual hit to fall to 1/e.
    #[slider(
        min = 10.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 0.0,
        hard_max = 10000.0,
        unit = Raw
    )]
    pub decay: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
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

impl Flash {
    /// The envelope × intensity this frame, 0..1, derived at resolve time rather
    /// than declared. Never a panel row: it is what the controls and the
    /// clock *produce*.
    pub const DERIVED_STRENGTH: ParamId = ParamId::new("derived.strength");

    /// The strength, colour and mix the kernel wants (docs/impl/
    /// effect-registry.md §2.4). `strength` comes from the bag rather than from a
    /// declared row, because it is a function of time and markers as well as of
    /// controls — [`FlashDef::resolve_derived`] computed it, already clamped, in
    /// the `f64` the old arm used throughout. Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self, strength: f32) -> (f32, [f32; 4], f32) {
        (strength, self.colour, (self.mix / 100.0).clamp(0.0, 1.0))
    }

    /// This instance's strength read back out of a resolved bag: `packed`'s
    /// missing argument, so no caller has to know the id.
    pub fn strength_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_STRENGTH, 0.0)
    }
}

/// Flash's behaviour.
pub struct FlashDef;

impl EffectDef for FlashDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Flash as EffectMetadata>::SCHEMA
    }

    /// The trigger envelope, scaled by Intensity and clamped — the whole of what
    /// the old resolve arm did, moved unchanged.
    ///
    /// Every read is the one the arm made, in the order it made it and in `f64`
    /// throughout, so the number reaching the kernel is bit-identical: the mode
    /// from the stored Choice (**absent resolves as Manual**, which is what makes
    /// a project saved before the marker modes render as it always did), the
    /// duration/shape/nth/phase of the marker envelope, or Manual's whole Trigger
    /// *track* decaying over Decay.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let (e, lt) = (cx.inst, cx.lt);
        let fl = |id: &str| e.float_at_with_context(id, lt, cx.context.clone());
        let mode = match e.param("mode") {
            Some(EffectValue::Choice(c)) => *c,
            _ => 0,
        };
        let envelope = match mode {
            // Trigger (1) and Strobe (2): the §3.7 beat envelope from the §1.4
            // context; Strobe thins the beat list to every Nth.
            1 | 2 => {
                let duration = fl("duration").unwrap_or(2.0).max(0.0);
                let fade = matches!(e.param("shape"), Some(EffectValue::Choice(1)));
                let nth = if mode == 2 { flash_nth(e, lt) } else { 1 };
                let phase = fl("phase").unwrap_or(0.0);
                flash_beat_envelope(cx.markers, lt, duration, fade, nth, phase)
            }
            // Manual: keyframed hits on Trigger, decaying over Decay — the
            // original form, untouched.
            _ => {
                let decay_s = (fl("decay").unwrap_or(120.0) / 1000.0).max(0.0);
                match e.param("trigger") {
                    Some(EffectValue::Float(p)) => flash_envelope(p, lt, decay_s),
                    _ => 0.0,
                }
            }
        };
        let intensity = fl("intensity").unwrap_or(100.0).max(0.0) / 100.0;
        push(
            Flash::DERIVED_STRENGTH,
            Value::Float((envelope * intensity).clamp(0.0, 1.0) as f32),
        );
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (strength, colour, mix) = Flash::read(p).packed(Flash::strength_of(p));
        cpu::flash(rgba, strength, colour, mix);
    }
}
