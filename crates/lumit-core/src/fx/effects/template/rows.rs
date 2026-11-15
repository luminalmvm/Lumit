//! The rows a copy of a graphics template derives from its control list,
//! each with the template's own value as its default while the copy is
//! handed out to be shown.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use super::{held, rows_of_control, RowKind, EXTRA_KEY};
use crate::fx::builtins::CHOICE_UNGROUPED;
use crate::fx::params::Unit;
use crate::fx::schema::{ParamKind, ParamSchema};
use crate::fx::shader::hash64;
use crate::model::{EffectInstance, EffectValue};
use crate::template::{ControlKind, PLAY_INTRO, PLAY_OUTRO};

/// Every control list this process has derived rows from, by its hash.
type RowCache = RwLock<HashMap<u64, &'static [ParamSchema]>>;

fn cache() -> &'static RowCache {
    static CACHE: OnceLock<RowCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn leaked(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// The rows the instance's control list derives, built once per distinct
/// list and kept until Lumit closes, as the Custom controls rows are.
///
/// A row's default is the template's own value when the instance was handed
/// out with them ([`offer`]). So the list is distinct for each state of the
/// template a copy has been looked at in, which is as often as a hand edits a
/// template and then looks at a copy. A stored instance carries no such
/// values and derives the one list its controls give.
pub(super) fn rows_of(inst: &EffectInstance) -> &'static [ParamSchema] {
    if !inst.extra.contains_key(EXTRA_KEY) {
        return &[];
    }
    let kept = held(inst);
    // Only what the rows are made from, so a copy setting a value, which
    // moves `own`, derives the rows it already had.
    let key_of = (&kept.controls, kept.intro, kept.outro, &kept.live);
    let key = hash64(&serde_json::to_vec(&key_of).unwrap_or_default());
    if let Ok(map) = cache().read() {
        if let Some(hit) = map.get(&key) {
            return hit;
        }
    }
    let live = |row: &str| kept.live.iter().find(|(r, _)| r == row).map(|(_, v)| v);
    let number = |row: &str| match live(row) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => 0.0,
    };
    let tint = |row: &str| match live(row) {
        Some(EffectValue::Colour(c)) => [
            c[0].value_at(0.0),
            c[1].value_at(0.0),
            c[2].value_at(0.0),
            c[3].value_at(0.0),
        ],
        _ => [0.0, 0.0, 0.0, 1.0],
    };
    let words = |row: &str| match live(row) {
        Some(EffectValue::Text(t)) => leaked(t),
        _ => "",
    };
    let mut rows: Vec<ParamSchema> = Vec::new();
    for control in &kept.controls {
        if control.id.is_empty() {
            continue;
        }
        let label = leaked(&control.label);
        for (id, kind) in rows_of_control(control) {
            if rows.iter().any(|r| r.id == id) {
                continue;
            }
            let (travel, hard) = match control.range {
                Some((min, max)) => ((min, max), (Some(min), Some(max))),
                None => (control.slider.unwrap_or((0.0, 100.0)), (None, None)),
            };
            let (kind, unit, label) = match kind {
                RowKind::Number => (
                    ParamKind::Float {
                        default: number(&id),
                        slider: travel,
                        hard,
                    },
                    control.unit,
                    label,
                ),
                RowKind::Angle => (
                    ParamKind::Angle {
                        default: number(&id),
                        dial_step: 15.0,
                    },
                    Unit::Degrees,
                    label,
                ),
                RowKind::Axis => (
                    ParamKind::Float {
                        default: number(&id),
                        slider: control.slider.unwrap_or((0.0, 1920.0)),
                        hard: (None, None),
                    },
                    control.unit,
                    label,
                ),
                RowKind::Colour => (
                    ParamKind::Colour {
                        default: tint(&id),
                        range: (0.0, 1.0),
                    },
                    Unit::Raw,
                    label,
                ),
                RowKind::Tick => (
                    ParamKind::Bool {
                        default: matches!(live(&id), Some(EffectValue::Bool(true))),
                    },
                    Unit::Raw,
                    label,
                ),
                RowKind::Menu => (
                    ParamKind::Choice {
                        options: &[],
                        default: match live(&id) {
                            Some(EffectValue::Choice(c)) => *c,
                            _ => 0,
                        },
                        dividers_after: CHOICE_UNGROUPED,
                    },
                    Unit::Raw,
                    label,
                ),
                RowKind::Words | RowKind::Picture => (
                    ParamKind::Text {
                        default: words(&id),
                    },
                    Unit::Raw,
                    label,
                ),
                RowKind::Font => (
                    ParamKind::Text {
                        default: words(&id),
                    },
                    Unit::Raw,
                    if control.kind == ControlKind::Font {
                        label
                    } else {
                        "Font"
                    },
                ),
                RowKind::Size => (
                    ParamKind::Float {
                        default: number(&id),
                        slider: (1.0, 400.0),
                        hard: (Some(1.0), Some(2000.0)),
                    },
                    Unit::Px,
                    "Size",
                ),
                RowKind::TextColour => (
                    ParamKind::Colour {
                        default: tint(&id),
                        range: (0.0, 1.0),
                    },
                    Unit::Raw,
                    "Colour",
                ),
                RowKind::Fit => (
                    ParamKind::Choice {
                        options: &["Fill", "Fit", "Stretch"],
                        default: 0,
                        dividers_after: CHOICE_UNGROUPED,
                    },
                    Unit::Raw,
                    "Fit",
                ),
            };
            rows.push(ParamSchema {
                id: leaked(&id),
                label,
                kind,
                unit,
            });
        }
    }
    for (row, there, label) in [
        (PLAY_INTRO, kept.intro, "Intro"),
        (PLAY_OUTRO, kept.outro, "Outro"),
    ] {
        if there {
            rows.push(ParamSchema {
                id: row,
                label,
                kind: ParamKind::Bool { default: true },
                unit: Unit::Raw,
            });
        }
    }
    let built: &'static [ParamSchema] = Box::leak(rows.into_boxed_slice());
    if let Ok(mut map) = cache().write() {
        map.insert(key, built);
    }
    built
}
