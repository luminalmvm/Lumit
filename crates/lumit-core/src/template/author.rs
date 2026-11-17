//! Making and changing a template: a control from the row it names, a write
//! to the template's own rows as the ops that make it, and what a menu's
//! option sets, recorded from the rows a hand changed.
//!
//! Everything here answers values or ops. Nothing commits.

use uuid::Uuid;

use super::{
    rows, Control, ControlKind, MaskRow, Setting, ShapeRow, Target, Template, TextRow, Value,
};
use crate::fx::params::Unit;
use crate::fx::schema::ParamKind;
use crate::model::{Composition, Document, Layer, LayerKind, ProjectItem, TransformProp};
use crate::ops::Op;

/// Whether a transform row is an angle.
fn is_angle(prop: TransformProp) -> bool {
    matches!(
        prop,
        TransformProp::Rotation
            | TransformProp::RotationX
            | TransformProp::RotationY
            | TransformProp::OrientationX
            | TransformProp::OrientationY
            | TransformProp::OrientationZ
            | TransformProp::ConeAngle
    )
}

/// What a transform row is counted in and how far its slider travels in a
/// composition `width` wide.
fn transform_scale(prop: TransformProp, width: f64) -> (Unit, (f64, f64)) {
    use TransformProp as P;
    match prop {
        P::Opacity | P::ConeFeather | P::ShadowDarkness => (Unit::Percent, (0.0, 100.0)),
        P::ScaleX | P::ScaleY | P::ScaleZ => (Unit::Percent, (0.0, 200.0)),
        P::LightIntensity => (Unit::Percent, (0.0, 200.0)),
        P::AnchorX | P::AnchorY | P::AnchorZ | P::PositionX | P::PositionY | P::PositionZ => {
            (Unit::Px, (0.0, width))
        }
        P::PoiX | P::PoiY | P::PoiZ | P::Zoom | P::FocusDistance => (Unit::Px, (0.0, width)),
        _ => (Unit::Raw, (0.0, 100.0)),
    }
}

/// A control for the row `target` names in `comp`, labelled `label`, with the
/// kind, range and options the row itself has. `None` when the row is not
/// there, or holds something no control sets.
///
/// The id is the caller's to give ([`Template::free_id`]): it has to be free
/// among the controls the template already has.
#[must_use]
pub fn control_for(
    doc: &Document,
    comp: &Composition,
    target: Target,
    label: &str,
) -> Option<Control> {
    let value = rows::read(doc, comp, &target)?;
    let layer = comp.layers.iter().find(|l| l.id == target.layer())?;
    let width = f64::from(comp.width);
    let mut control = Control::new(String::new(), label.to_owned(), ControlKind::Number, target);
    match &control.target {
        Target::Transform { props, .. } => {
            let first = *props.first()?;
            let (unit, slider) = transform_scale(first, width);
            control.unit = unit;
            control.slider = Some(slider);
            control.kind = match value {
                Value::Colour(_) => ControlKind::Colour,
                Value::Point(_) => {
                    control.axes = u8::try_from(props.len()).ok()?;
                    ControlKind::Point
                }
                _ if is_angle(first) => ControlKind::Angle,
                _ => ControlKind::Number,
            };
        }
        Target::Param { effect, param, .. } => {
            let inst = layer
                .effects
                .iter()
                .chain(&layer.styles)
                .find(|e| e.id == *effect)?;
            let def = crate::fx::builtins::def(&inst.effect.match_name)?;
            let declared = def
                .schema()
                .params
                .iter()
                .chain(def.derived(inst))
                .find(|p| p.id == param.as_str())?;
            control.unit = declared.unit;
            control.kind = match &declared.kind {
                ParamKind::Float { slider, hard, .. } => {
                    control.slider = Some(*slider);
                    if let (Some(min), Some(max)) = hard {
                        control.range = Some((*min, *max));
                    }
                    ControlKind::Number
                }
                ParamKind::Slider { range, .. } => {
                    control.range = Some(*range);
                    ControlKind::Number
                }
                ParamKind::Int { slider, .. } => {
                    control.slider = Some((slider.0 as f64, slider.1 as f64));
                    ControlKind::Number
                }
                ParamKind::Angle { .. } => ControlKind::Angle,
                ParamKind::Choice { options, .. } => {
                    // The words this instance lists, where it lists its own.
                    let own = def
                        .row_state(inst)
                        .options
                        .into_iter()
                        .find(|(row, _)| *row == declared.id)
                        .map(|(_, words)| words);
                    control.options =
                        own.unwrap_or_else(|| options.iter().map(|o| (*o).to_owned()).collect());
                    ControlKind::Menu
                }
                ParamKind::Bool { .. } => ControlKind::Checkbox,
                ParamKind::Colour { .. } => ControlKind::Colour,
                ParamKind::Text { .. } => ControlKind::Text,
                _ => return None,
            };
        }
        Target::ParamPoint { .. } => {
            control.kind = ControlKind::Point;
            control.axes = 2;
            control.unit = Unit::Px;
            control.slider = Some((0.0, width));
        }
        Target::EffectOn { .. } | Target::Visible { .. } => control.kind = ControlKind::Checkbox,
        Target::Text { .. } => control.kind = ControlKind::Text,
        Target::Font { .. } => control.kind = ControlKind::Font,
        Target::TextRow { value: row, .. } => match row {
            TextRow::Fill | TextRow::Stroke | TextRow::Background => {
                control.kind = ControlKind::Colour;
            }
            TextRow::Size => {
                control.unit = Unit::Px;
                control.slider = Some((1.0, 400.0));
                control.range = Some((1.0, 2000.0));
            }
            TextRow::StrokeWidth => {
                control.unit = Unit::Px;
                control.slider = Some((0.0, 50.0));
            }
            TextRow::Tracking => control.slider = Some((-100.0, 500.0)),
        },
        Target::Shape { value: row, .. } => match row {
            ShapeRow::Fill | ShapeRow::Stroke => control.kind = ControlKind::Colour,
            ShapeRow::Rotation => control.kind = ControlKind::Angle,
            ShapeRow::Opacity => {
                control.unit = Unit::Percent;
                control.slider = Some((0.0, 100.0));
            }
            ShapeRow::StrokeWidth => {
                control.unit = Unit::Px;
                control.slider = Some((0.0, 100.0));
            }
            ShapeRow::Width | ShapeRow::Height | ShapeRow::Roundness => {
                control.unit = Unit::Px;
                control.slider = Some((0.0, width));
            }
        },
        Target::Mask { value: row, .. } => match row {
            MaskRow::Opacity => {
                control.unit = Unit::Percent;
                control.slider = Some((0.0, 100.0));
            }
            MaskRow::Feather => {
                control.unit = Unit::Px;
                control.slider = Some((0.0, 200.0));
            }
            MaskRow::Expansion => {
                control.unit = Unit::Px;
                control.slider = Some((-200.0, 200.0));
            }
        },
        Target::Volume { .. } => control.slider = Some((-48.0, 12.0)),
        Target::Source { .. } => control.kind = ControlKind::Footage,
        Target::Copy {
            control: inner_id, ..
        } => {
            let LayerKind::Precomp { comp: nested } = &layer.kind else {
                return None;
            };
            let inner = doc.comp(*nested)?.template()?;
            let inner = inner.control(inner_id)?;
            let (target, label) = (control.target.clone(), control.label.clone());
            control = Control {
                id: String::new(),
                label,
                target,
                group: None,
                looks: Vec::new(),
                ..inner.clone()
            };
        }
    }
    Some(control)
}

/// The ops that write `settings` into the rows of composition `comp`, each
/// row fixed at its value. What setting a control in the template itself is,
/// and writing a copy's value back as the template's.
///
/// One op for each part of a layer that changed, so the edit undoes and
/// merges with other people's as the same change made by hand would.
#[must_use]
pub fn ops(doc: &Document, comp: Uuid, settings: &[Setting]) -> Vec<Op> {
    let mut after = doc.clone();
    for setting in settings {
        rows::write(&mut after, comp, &setting.target, &setting.value);
    }
    let (Some(was), Some(now)) = (doc.comp(comp), after.comp(comp)) else {
        return Vec::new();
    };
    let mut made: Vec<Op> = Vec::new();
    // A picture brings a composition of its own for it to be fitted in.
    let mut index = doc.items.len();
    for item in &after.items {
        if doc.item(item.id()).is_none() {
            made.push(Op::AddItem {
                index,
                item: Box::new(item.clone()),
            });
            index += 1;
        }
    }
    for layer in &now.layers {
        let Some(before) = was.layers.iter().find(|l| l.id == layer.id) else {
            continue;
        };
        if before != layer {
            layer_ops(comp, settings, before, layer, &mut made);
        }
    }
    made
}

/// The ops that turn layer `before` into `after`, for the parts a row write
/// can change.
fn layer_ops(comp: Uuid, settings: &[Setting], before: &Layer, after: &Layer, made: &mut Vec<Op>) {
    let layer = after.id;
    for setting in settings {
        let Target::Transform { layer: on, props } = &setting.target else {
            continue;
        };
        if *on != layer {
            continue;
        }
        for prop in props {
            if let (Some(a), Some(b)) = (before.prop(*prop), after.prop(*prop)) {
                if a != b {
                    made.push(Op::SetTransformProperty {
                        comp,
                        layer,
                        prop: *prop,
                        animation: b.animation.clone(),
                    });
                }
            }
        }
    }
    if before.effects != after.effects {
        made.push(Op::SetLayerEffects {
            comp,
            layer,
            effects: after.effects.clone(),
        });
    }
    if before.styles != after.styles {
        made.push(Op::SetLayerStyles {
            comp,
            layer,
            styles: after.styles.clone(),
        });
    }
    if before.masks != after.masks {
        made.push(Op::SetLayerMasks {
            comp,
            layer,
            masks: after.masks.clone(),
        });
    }
    if before.kind != after.kind {
        made.push(match &after.kind {
            LayerKind::Text { document } => Op::SetTextDocument {
                comp,
                layer,
                document: document.clone(),
            },
            LayerKind::Shape { contents } => Op::SetShapeContents {
                comp,
                layer,
                contents: contents.clone(),
            },
            kind => Op::SetLayerKind {
                comp,
                layer,
                kind: Box::new(kind.clone()),
            },
        });
    }
    if before.switches.visible != after.switches.visible {
        made.push(Op::SetLayerVisible {
            comp,
            layer,
            visible: after.switches.visible,
        });
    }
    if before.volume_db != after.volume_db {
        made.push(Op::SetLayerVolume {
            comp,
            layer,
            animation: after.volume_db.animation.clone(),
        });
    }
    if before.graph_inputs != after.graph_inputs {
        made.push(Op::SetLayerGraphInputs {
            comp,
            layer,
            inputs: after.graph_inputs.clone().map(Box::new),
        });
    }
}

/// A template with what a hand changed recorded as the look of option
/// `option` of its menu `menu`: every row that holds another value in `now`
/// than it did in `was`, the composition as it stood when recording began.
///
/// Every other option then takes the value those rows had before, unless it
/// already sets them, so picking any option says what each of those rows
/// holds and picking the first one back undoes the rest. `None` when `menu`
/// is not a menu of the template.
#[must_use]
pub fn recorded(
    doc: &Document,
    template: &Template,
    menu: &str,
    option: usize,
    was: &Composition,
    now: &Composition,
) -> Option<Template> {
    let mut next = template.clone();
    let control = next.controls.iter_mut().find(|c| c.id == menu)?;
    if control.kind != ControlKind::Menu {
        return None;
    }
    // The menu's own row moving is choosing an option, not part of a look.
    let own = control.target.clone();
    let changed: Vec<Setting> = rows::changed(doc, was, now)
        .into_iter()
        .filter(|s| s.target != own)
        .collect();
    let count = control.options.len().max(option + 1);
    control.looks.resize(count, Vec::new());
    for setting in changed {
        let before = rows::read(doc, was, &setting.target);
        for (n, look) in control.looks.iter_mut().enumerate() {
            let held = look.iter_mut().find(|s| s.target == setting.target);
            match (n == option, held, &before) {
                (true, Some(held), _) => held.value = setting.value.clone(),
                (true, None, _) => look.push(setting.clone()),
                (false, None, Some(before)) => look.push(Setting {
                    target: setting.target.clone(),
                    value: before.clone(),
                }),
                _ => {}
            }
        }
    }
    Some(next)
}

/// A name for a new folder or layer that nothing among `taken` has: `base`,
/// or `base 2`, `base 3` and on.
#[must_use]
pub fn free_name(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_owned();
    }
    (2u32..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken.contains(&name.as_str()))
        .unwrap_or_else(|| base.to_owned())
}

/// The folder in the Project panel that templates brought from the library
/// are filed in, by its name, when the project has one.
#[must_use]
pub fn library_folder(doc: &Document, name: &str) -> Option<Uuid> {
    doc.items.iter().find_map(|item| match item {
        ProjectItem::Folder(folder) if folder.name == name => Some(folder.id),
        _ => None,
    })
}
