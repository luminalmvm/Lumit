//! Custom controls: a named set of controls, held for expressions to read.
//!
//! **In plain terms.** A rig usually wants more than one dial. This effect
//! draws nothing and holds as many controls as its owner gave it, each with a
//! name of its own: "Amplitude", "Frames per flip", "Seed". Expressions on
//! other properties read them, `effect("Shake")("Amplitude")`, and the person
//! using the rig keyframes those and never opens the expressions.
//!
//! It is what After Effects calls a pseudo effect, and it is how one arrives
//! when a preset is brought across.
//!
//! **The controls are the instance's own.** Which controls there are is a
//! fact about one rig and not about the effect, so the list lives on the
//! instance under `extra["controls"]` and the rows are derived from it, the
//! way an Expression box derives its inputs. [`controls_of`] and
//! [`set_controls`] are the two ways in.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::anim::Property;
use crate::fx::shader::hash64;
use crate::fx::{EffectDef, EffectMetadata, EffectSchema, ParamKind, ParamSchema, Unit};
use crate::model::{EffectInstance, EffectParam, EffectValue};
use lumit_fx_macros::Effect;

/// The `extra` key the list of controls lives under.
pub const EXTRA_KEY: &str = "controls";

/// The most controls one instance holds. A ceiling rather than a design: the
/// list comes out of the document, and a hand-edited one must not ask for a
/// million rows.
pub const MAX_CONTROLS: usize = 256;

/// What one control is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Number,
    Angle,
    Checkbox,
    Colour,
}

/// One control of a set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Control {
    /// The row's id, which is what the document stores its value under.
    pub id: String,
    /// The name it is shown under, and the name an expression reads it by.
    pub label: String,
    pub kind: ControlKind,
    /// What a fresh one holds. A number uses the first, a colour all four.
    #[serde(default)]
    pub default: [f64; 4],
    /// Where the slider's travel starts and ends. Typing goes past either.
    #[serde(default)]
    pub min: f64,
    #[serde(default = "hundred")]
    pub max: f64,
}

fn hundred() -> f64 {
    100.0
}

/// Custom controls declares nothing of its own. Every row it shows is one of
/// the instance's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "custom_controls",
    label = "Custom controls",
    version = 1,
    category = Controls,
    cost = Trivial,
    roi = Exact,
    // No picture, so no matte, as every other control declares.
    matte = false,
)]
pub struct CustomControls {}

/// The controls this instance holds, in the order they are shown.
#[must_use]
pub fn controls_of(inst: &EffectInstance) -> Vec<Control> {
    inst.extra
        .get(EXTRA_KEY)
        .and_then(|list| serde_json::from_value::<Vec<Control>>(list.clone()).ok())
        .map(|mut list| {
            list.truncate(MAX_CONTROLS);
            list
        })
        .unwrap_or_default()
}

/// The id of the control shown as `label`, read straight off the stored list.
///
/// An expression asks this for every control it reads, on every frame, which
/// is why it does not go through [`controls_of`] or the derived rows.
#[must_use]
pub fn id_of<'a>(inst: &'a EffectInstance, label: &str) -> Option<&'a str> {
    inst.extra
        .get(EXTRA_KEY)?
        .as_array()?
        .iter()
        .find(|control| {
            control
                .get("label")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|shown| shown.eq_ignore_ascii_case(label))
        })?
        .get("id")?
        .as_str()
}

/// Give `inst` this list of controls. A control it has no value for yet gets
/// its default, and a value whose control is gone is dropped.
pub fn set_controls(inst: &mut EffectInstance, controls: &[Control]) {
    let controls = controls
        .get(..controls.len().min(MAX_CONTROLS))
        .unwrap_or(controls);
    inst.params
        .retain(|p| controls.iter().any(|c| c.id == p.id));
    for control in controls {
        if inst.params.iter().any(|p| p.id == control.id) {
            continue;
        }
        let d = control.default;
        inst.params.push(EffectParam {
            id: control.id.clone(),
            value: match control.kind {
                ControlKind::Number | ControlKind::Angle => {
                    EffectValue::Float(Property::fixed(d[0]))
                }
                ControlKind::Checkbox => EffectValue::Bool(d[0] != 0.0),
                ControlKind::Colour => EffectValue::Colour(d.map(Property::fixed)),
            },
            extra: serde_json::Map::new(),
        });
    }
    match serde_json::to_value(controls) {
        Ok(list) if !controls.is_empty() => {
            inst.extra.insert(EXTRA_KEY.to_owned(), list);
        }
        _ => {
            inst.extra.remove(EXTRA_KEY);
        }
    }
}

/// Every list of controls this process has derived rows from, by its hash.
type RowCache = RwLock<HashMap<u64, &'static [ParamSchema]>>;

fn cache() -> &'static RowCache {
    static CACHE: OnceLock<RowCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The rows a list of controls derives, built once per distinct list and kept
/// until Lumit closes.
///
/// The render path asks for these once per effect per frame, so this is a
/// hash and a lookup rather than a rebuild. The rows are `&'static`, which for
/// names somebody typed means leaked, as the Node graph effect's are and for
/// the same reason: it is bounded by how many rigs a hand can make.
fn rows_of(inst: &EffectInstance) -> &'static [ParamSchema] {
    let Some(list) = inst.extra.get(EXTRA_KEY) else {
        return &[];
    };
    let key = hash64(&serde_json::to_vec(list).unwrap_or_default());
    if let Ok(map) = cache().read() {
        if let Some(hit) = map.get(&key) {
            return hit;
        }
    }
    let mut rows: Vec<ParamSchema> = Vec::new();
    for control in controls_of(inst) {
        // Two controls under one id would be one value shown twice.
        if control.id.is_empty() || rows.iter().any(|row| row.id == control.id) {
            continue;
        }
        let d = control.default;
        let (kind, unit) = match control.kind {
            ControlKind::Number => (
                ParamKind::Float {
                    default: d[0],
                    slider: (control.min, control.max),
                    // What the number means is the rig's business, as a
                    // Slider control's is.
                    hard: (None, None),
                },
                Unit::Raw,
            ),
            ControlKind::Angle => (
                ParamKind::Angle {
                    default: d[0],
                    dial_step: 15.0,
                },
                Unit::Degrees,
            ),
            ControlKind::Checkbox => (
                ParamKind::Bool {
                    default: d[0] != 0.0,
                },
                Unit::Raw,
            ),
            ControlKind::Colour => (
                ParamKind::Colour {
                    default: d,
                    range: (0.0, 1.0),
                },
                Unit::Raw,
            ),
        };
        rows.push(ParamSchema {
            id: Box::leak(control.id.into_boxed_str()),
            label: Box::leak(control.label.into_boxed_str()),
            kind,
            unit,
        });
    }
    let built: &'static [ParamSchema] = Box::leak(rows.into_boxed_slice());
    if let Ok(mut map) = cache().write() {
        map.insert(key, built);
    }
    built
}

/// Custom controls' behaviour: none, by design.
pub struct CustomControlsDef;

impl EffectDef for CustomControlsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CustomControls as EffectMetadata>::SCHEMA
    }

    /// It holds values. It does not draw.
    fn is_image_op(&self) -> bool {
        false
    }

    fn derived(&self, inst: &EffectInstance) -> &'static [ParamSchema] {
        rows_of(inst)
    }
}
