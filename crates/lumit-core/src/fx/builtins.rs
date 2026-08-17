use super::*;
use crate::anim::Property;
use crate::model::{
    EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue, FileParam,
};

/// Edge-policy option labels shared by the blur family (docs/08 §3.8) and
/// Shake (§3.4). Backed by the reusable [`EdgesMode`] enum (P3, K-145), so the
/// labels and the 0/1/2 codes stay in one place.
pub const EDGE_OPTIONS: &[&str] = EdgesMode::OPTIONS;

/// "No group dividers" for a [`ParamKind::Choice`]'s `dividers_after` (T21) —
/// the common case, spelled once so every ungrouped Choice reads the same.
pub const CHOICE_UNGROUPED: &[u32] = &[];

/// Which channel of an auxiliary picture an effect reads as a single number —
/// the depth out of a depth pass, the weight out of a custom aperture image.
///
/// One list, shared, so that every effect naming a channel of an auxiliary
/// picture names it from the same short list rather than declaring its own and
/// letting them drift. The index order is the wire form the resolved ops carry,
/// so entries are appended, never reordered.
///
/// **Every entry has to be able to explain itself.** A depth pass or a dirt
/// plate arrives as a picture, and the question is only which number in it is
/// the one the effect wants:
///
/// - **Luminance** — the default, and right for the overwhelmingly common case:
///   a grey map, whatever combination of channels it was written to. Weighted
///   (Rec.709) rather than a plain mean, so a pass that is only *nearly* grey
///   still reads sensibly.
/// - **Alpha** — some renderers put depth in the alpha of the beauty pass.
/// - **Red / Green / Blue** — a packed pass, where several AOVs were flattened
///   into one image and this one landed in a particular channel. Red is also the
///   historical convention for a depth pass on its own.
///
/// Hue, saturation and lightness are deliberately **not** here. Nothing encodes
/// a depth or a density as a hue, and offering the option only invites someone
/// to find out.
pub const CHANNEL_OPTIONS: &[&str] = &["Luminance", "Alpha", "Red", "Green", "Blue"];

/// Look a schema up by its match name. `None` for a name this build does not
/// know — an unknown effect is preserved as an inert placeholder (K-065).
pub fn schema(match_name: &str) -> Option<&'static EffectSchema> {
    super::BUILTIN_DEFS.get(match_name).map(|d| d.schema())
}

/// A fresh random seed value — the per-instance Seed default (docs/08
/// §3.4) and the Effect Controls "reseed" button (§2.4) both draw from
/// here. Taken from a new UUID's random tail, so it needs no extra
/// dependency; the value becomes stored project data the moment it is
/// chosen, so evaluation determinism (§2.4) is untouched.
pub fn fresh_seed() -> u32 {
    let b = uuid::Uuid::now_v7().into_bytes();
    u32::from_le_bytes([b[12], b[13], b[14], b[15]])
}

/// A new instance of a built-in, carrying the declared defaults.
/// [`instantiate`], then centre any raster-anchored defaults on the target
/// raster (owner T23): the Transform effect's Anchor and Position default to
/// the raster's centre so a fresh instance rotates and scales about the
/// middle, not the 0,0 corner — a schema constant cannot know the raster, so
/// the apply site passes it. Every UI apply path calls this; plain
/// [`instantiate`] keeps the pure schema defaults (tests, presets).
pub fn instantiate_for_raster(match_name: &str, w: f64, h: f64) -> Option<EffectInstance> {
    let mut inst = instantiate(match_name)?;
    if match_name == "transform" {
        for p in &mut inst.params {
            let v = match p.id.as_str() {
                "anchor_x" | "position_x" => w * 0.5,
                "anchor_y" | "position_y" => h * 0.5,
                _ => continue,
            };
            p.value = EffectValue::Float(Property::fixed(v));
        }
    }
    // The flare's light is px@comp (K-260), so its tasteful default — the
    // upper-left third (§1.2) — needs the actual raster.
    if match_name == "lens_flare" {
        for p in &mut inst.params {
            let v = match p.id.as_str() {
                "light_x" => w * 0.33,
                "light_y" => h * 0.30,
                _ => continue,
            };
            p.value = EffectValue::Float(Property::fixed(v));
        }
    }
    // Depth of field's Focus point is the same shape of default: a fresh
    // instance should focus on the middle of the frame, which is where the
    // subject usually is and is the only guess that is never absurd. The schema
    // declares (0, 0) because it cannot know the raster; landing focus in the
    // top-left corner is exactly the "drop it on and it already looks right"
    // failure (§1.2).
    if match_name == "dof" {
        for p in &mut inst.params {
            let v = match p.id.as_str() {
                "focus_point_x" => w * 0.5,
                "focus_point_y" => h * 0.5,
                _ => continue,
            };
            p.value = EffectValue::Float(Property::fixed(v));
        }
    }
    Some(inst)
}

/// The value a parameter kind starts at (docs/08 §1.2): what `instantiate`
/// fills a fresh instance with, and what [`backfill_builtin_params`] appends
/// for a parameter the saved instance predates.
pub fn default_param_value(kind: &ParamKind) -> EffectValue {
    match *kind {
        ParamKind::Float { default, .. } => EffectValue::Float(Property::fixed(default)),
        // Int is a display/rounding kind; the value is a Float like any
        // other scalar (see the schema's Int docs).
        ParamKind::Int { default, .. } => EffectValue::Float(Property::fixed(default as f64)),
        // An angle is a number of degrees, so it stores as a plain Float
        // (docs/08 §1.1) — the dial is a control, not a value type, and
        // keyframes and expressions see nothing new.
        ParamKind::Angle { default, .. } => EffectValue::Float(Property::fixed(default)),
        ParamKind::Choice { default, .. } => EffectValue::Choice(default),
        ParamKind::Bool { default } => EffectValue::Bool(default),
        ParamKind::Colour { default, .. } => EffectValue::Colour(default.map(Property::fixed)),
        ParamKind::Seed => EffectValue::Seed(fresh_seed()),
        ParamKind::File { .. } => EffectValue::File(FileParam::empty()),
        // A fresh layer reference is unset (docs/impl/layer-input.md): the
        // effect is a labelled no-op until the owner picks a layer, the same
        // sanctioned exception the File parameter takes to the "no no-op
        // default" rule.
        ParamKind::Layer { .. } => EffectValue::Layer(None),
    }
}

/// Forward-migrate a stack loaded from disk (K-258): a built-in instance
/// saved before its schema grew a parameter simply lacks it, which left the
/// panel drawing a dash and `set_value` refusing the id. Append every
/// missing declared parameter at its default. Never touches present values,
/// unknown effects, or plugin namespaces — a project round-trips untouched
/// unless its schema really did grow.
pub fn backfill_builtin_params(effects: &mut [EffectInstance]) {
    for e in effects.iter_mut() {
        if e.effect.namespace != EffectNamespace::Builtin {
            continue;
        }
        let Some(s) = schema(&e.effect.match_name) else {
            continue;
        };
        migrate_lens_flare_background(e);
        for p in s.params {
            if !e.params.iter().any(|have| have.id == p.id) {
                e.params.push(EffectParam {
                    id: p.id.to_owned(),
                    value: default_param_value(&p.kind),
                    extra: serde_json::Map::new(),
                });
            }
        }
    }
}

/// Carry a saved Lens flare's Background choice over to the Blend menu that
/// replaced it (K-289, superseding K-258). The old parameter had two values:
/// Transparent (the flare added to the layer's own alpha) and Black (the
/// output forced opaque — the flare-element-over-black export). Transparent
/// *is* the new Add, bit for bit, and it was the default, so almost every
/// saved flare needs nothing beyond the ordinary backfill. Black becomes
/// Normal: on the empty layer that option was for, "the flare on opaque
/// black" is what both produce.
///
/// The legacy parameter is dropped once read — the schema no longer declares
/// it, so leaving it would be a row `set_value` refuses and the panel cannot
/// draw. Runs before the backfill appends `blend`, so a project saved with
/// Black never briefly reads as Add.
fn migrate_lens_flare_background(e: &mut EffectInstance) {
    if e.effect.match_name != "lens_flare" {
        return;
    }
    let Some(old) = e.params.iter().position(|p| p.id == "background") else {
        return;
    };
    let was_black = matches!(e.params[old].value, EffectValue::Choice(1));
    e.params.remove(old);
    if e.params.iter().any(|p| p.id == "blend") {
        return;
    }
    e.params.push(EffectParam {
        id: "blend".to_owned(),
        value: EffectValue::Choice(if was_black {
            crate::fx::lens_flare::BLEND_NORMAL
        } else {
            crate::fx::lens_flare::BLEND_ADD
        }),
        extra: serde_json::Map::new(),
    });
}

/// Point every `self_default` Layer parameter in `inst` at `layer` — the
/// layer the effect is being added to (K-288, docs/impl/layer-input.md).
///
/// A schema constant cannot know which layer it will land on, so the apply
/// site passes it, exactly as [`instantiate_for_raster`] passes the raster.
/// Today that is the Lens flare's Matte layer: adding the effect and
/// switching Source to Matte should flare the lights in the picture the
/// effect is already looking at, not sit there doing nothing until a layer
/// is picked. Presets and plain [`instantiate`] leave the reference unset,
/// which stays the labelled no-op it always was.
pub fn point_self_layer_params_at(inst: &mut EffectInstance, layer: uuid::Uuid) {
    let Some(s) = schema(&inst.effect.match_name) else {
        return;
    };
    for p in s.params {
        if !matches!(p.kind, ParamKind::Layer { self_default: true }) {
            continue;
        }
        if let Some(slot) = inst.params.iter_mut().find(|have| have.id == p.id) {
            slot.value = EffectValue::Layer(Some(layer));
        }
    }
}

/// Whether the parameter `id` is editable given what `inst` currently holds —
/// the greyed-row rule of [`EnabledWhen`], evaluated.
///
/// **In plain terms.** Ticking "Use focus point" hands focus over to the point,
/// so the focus *distance* number stops being what decides anything; this
/// answers `false` for it while that tick is on, and the panel draws the row
/// greyed. A parameter with no rule against it is always editable, which is
/// nearly all of them.
///
/// This is the single authority on the question. The panel greys from it, and a
/// write to a disabled parameter is still accepted — greying is an affordance
/// telling you which control is in charge, not a lock. The resolve step
/// implements the real branch independently and never calls this, so the two
/// cannot drift into disagreeing about pixels: at worst a missing rule leaves a
/// live control that does nothing, which is a panel bug, not a render bug.
pub fn param_enabled(inst: &EffectInstance, id: &str) -> bool {
    let Some(s) = schema(&inst.effect.match_name) else {
        // No built-in schema (an OFX or placeholder instance) means no rules,
        // so nothing is greyed.
        return true;
    };
    s.enabled_when
        .iter()
        .filter(|rule| rule.param == id)
        .all(|rule| {
            // A rule naming a parameter the instance does not carry cannot be
            // judged, so it does not grey anything: an older instance that
            // predates the deciding parameter stays fully editable rather than
            // locking a row it can never unlock (the `fill_missing_params`
            // trap, from the other side).
            let Some(value) = inst.param(rule.on) else {
                return true;
            };
            match (rule.cond, value) {
                (EnabledCond::BoolIs(want), EffectValue::Bool(got)) => *got == want,
                (EnabledCond::ChoiceIs(want), EffectValue::Choice(got)) => *got == want,
                (EnabledCond::ChoiceIsNot(no), EffectValue::Choice(got)) => *got != no,
                (EnabledCond::LayerSet, EffectValue::Layer(layer)) => layer.is_some(),
                // A rule pointed at the wrong kind of parameter is a schema
                // mistake, not a reason to grey a row the owner can then never
                // reach. `every_enablement_rule_names_a_parameter_of_its_kind`
                // fails the build for it instead.
                _ => true,
            }
        })
}

pub fn instantiate(match_name: &str) -> Option<EffectInstance> {
    let s = schema(match_name)?;
    Some(EffectInstance {
        id: uuid::Uuid::now_v7(),
        effect: EffectKey {
            namespace: EffectNamespace::Builtin,
            match_name: s.match_name.to_owned(),
            version: s.version,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: s
            .params
            .iter()
            .map(|p| EffectParam {
                id: p.id.to_owned(),
                value: default_param_value(&p.kind),
                extra: serde_json::Map::new(),
            })
            .collect(),
        sample_temporally: true,
        custom_name: None,
        extra: serde_json::Map::new(),
    })
}
