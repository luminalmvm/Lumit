//! Built-in presets: a few named settings an effect ships with, picked from
//! the effect's own menu. Each effect lists its own in its file
//! ([`EffectDef::presets`](crate::fx::registry::EffectDef::presets)), so a new
//! one is a line beside the controls it sets.
//!
//! Applying one sets the effect it is picked on. Every control goes back to
//! its default first, so a preset looks the same whatever was there before,
//! except the layers it reads, which the preset knows nothing about. Keyframes
//! are laid from the playhead, so a reveal or a hide lands where it is wanted.

use crate::anim::{Animation, Keyframe, Property, SideInterp, EASY_EASE};
use crate::model::{EffectInstance, EffectValue};
use crate::time::Rational;

/// One built-in preset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectPreset {
    /// What the menu says. An engine label, so it is translated.
    pub name: &'static str,
    /// The controls it sets, by id. Anything left out is at its default.
    pub values: &'static [(&'static str, PresetValue)],
}

/// One control's value in a preset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PresetValue {
    Float(f64),
    Choice(u32),
    Bool(bool),
    /// Scene-linear RGBA.
    Colour([f64; 4]),
    /// Keyframes eased in and out, each a time in seconds after the playhead
    /// and a value: something moving from one state to another.
    Keys(&'static [(f64, f64)]),
    /// The same, eased out only: each step starts at once and settles, for
    /// something arriving or leaving.
    KeysOut(&'static [(f64, f64)]),
}

/// The presets `match_name` ships with. Empty for an effect with none, and
/// for one this build does not know.
#[must_use]
pub fn presets(match_name: &str) -> &'static [EffectPreset] {
    crate::fx::builtins::def(match_name).map_or(&[], |d| d.presets())
}

/// Set `inst` to `preset`, with any keyframes starting at layer time `at`, on
/// a comp `raster` (width, height) so positions start where a fresh effect's
/// would. The layers its rows read, those rows' own switches, and the seed are
/// kept.
pub fn apply(inst: &mut EffectInstance, preset: &EffectPreset, at: Rational, raster: (f64, f64)) {
    if let Some(fresh) =
        crate::fx::builtins::instantiate_for_raster(&inst.effect.match_name, raster.0, raster.1)
    {
        apply_over(inst, preset, at, &fresh);
    }
}

/// [`apply`] with every control first going back to `fresh` in place of a
/// new effect's: a 3D model's material starts from its file's own values.
pub fn apply_over(
    inst: &mut EffectInstance,
    preset: &EffectPreset,
    at: Rational,
    fresh: &EffectInstance,
) {
    // A transition's look leaves which clip it is on as it was.
    let kept = crate::sequence::transition::kept_rows(&inst.effect.match_name);
    let keeps = |id: &str, v: &EffectValue| {
        matches!(v, EffectValue::Layer(_) | EffectValue::Seed(_))
            || id.ends_with("_placement")
            || id == crate::fx::schema::MATTE_INVERT_PARAM
            || id == crate::fx::schema::MATTE_CHANNEL_PARAM
            || kept.contains(&id)
    };
    for p in &mut inst.params {
        if keeps(&p.id, &p.value) {
            continue;
        }
        if let Some(d) = fresh.params.iter().find(|d| d.id == p.id) {
            p.value = d.value.clone();
        }
    }
    for (id, value) in preset.values {
        let Some(p) = inst.params.iter_mut().find(|p| p.id == *id) else {
            continue;
        };
        p.value = match *value {
            PresetValue::Float(v) => EffectValue::Float(Property::fixed(v)),
            PresetValue::Choice(c) => EffectValue::Choice(c),
            PresetValue::Bool(b) => EffectValue::Bool(b),
            PresetValue::Colour(c) => EffectValue::Colour(c.map(Property::fixed)),
            PresetValue::Keys(keys) | PresetValue::KeysOut(keys) => {
                let out = matches!(value, PresetValue::KeysOut(_));
                let mut keys: Vec<Keyframe> = keys
                    .iter()
                    .filter_map(|&(t, value)| {
                        let after = Rational::new((t * 1000.0).round() as i64, 1000).ok()?;
                        Some(Keyframe {
                            time: at.checked_add(after).ok()?,
                            value,
                            interp_in: EASY_EASE,
                            interp_out: EASY_EASE,
                        })
                    })
                    .collect();
                if out {
                    ease_out(&mut keys);
                }
                EffectValue::Float(Property {
                    animation: Animation::Keyframed(keys),
                    extra: serde_json::Map::new(),
                })
            }
        };
    }
}

/// An ease-out on every step, `cubic-bezier(0.33, 1, 0.68, 1)` in After
/// Effects' terms: the outgoing handle covers the whole step in its first
/// third, and the incoming one arrives flat over the last third. Gentler than
/// an interface's ease-out, so a one-second reveal still reads as one.
fn ease_out(keys: &mut [Keyframe]) {
    for i in 1..keys.len() {
        let span = (keys[i].time.to_f64() - keys[i - 1].time.to_f64()).max(1e-6);
        let step = keys[i].value - keys[i - 1].value;
        keys[i - 1].interp_out = SideInterp::Bezier {
            speed: step / (0.33 * span),
            influence: 0.33,
        };
        keys[i].interp_in = SideInterp::Bezier {
            speed: 0.0,
            influence: 0.32,
        };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::schema::ParamKind;

    /// A preset naming a row its effect hasn't got, or the wrong kind of value
    /// for it, would quietly do nothing.
    #[test]
    fn every_preset_sets_rows_its_effect_has() {
        let materials = crate::model_material::MATERIAL_DEFS.builtins();
        // And every look a transition kind offers.
        let looks = crate::sequence::transition::TABLE.iter().filter_map(|k| {
            let def = crate::fx::catalogue::BUILTIN_DEFS.get(k.name)?;
            Some((def, k.looks))
        });
        let defs = crate::fx::catalogue::BUILTIN_DEFS.iter().chain(materials);
        for (def, presets) in defs.map(|d| (d, d.presets())).chain(looks) {
            let schema = def.schema();
            for preset in presets {
                for (id, value) in preset.values {
                    let row = schema.params.iter().find(|p| p.id == *id);
                    let fits = match (row.map(|r| &r.kind), value) {
                        (
                            Some(
                                ParamKind::Float { .. }
                                | ParamKind::Slider { .. }
                                | ParamKind::Int { .. }
                                | ParamKind::Angle { .. },
                            ),
                            PresetValue::Float(_) | PresetValue::Keys(_) | PresetValue::KeysOut(_),
                        ) => true,
                        (Some(ParamKind::Choice { options, .. }), PresetValue::Choice(c)) => {
                            (*c as usize) < options.len()
                        }
                        (Some(ParamKind::Bool { .. }), PresetValue::Bool(_)) => true,
                        (Some(ParamKind::Colour { .. }), PresetValue::Colour(_)) => true,
                        _ => false,
                    };
                    assert!(
                        fits,
                        "{} preset {:?} sets {id} wrongly",
                        schema.match_name, preset.name
                    );
                }
            }
        }
    }
}
