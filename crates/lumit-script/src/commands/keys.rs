use lumit_core::anim::{Animation, Keyframe, Property, SideInterp, TangentMode};
use lumit_core::edits;
use lumit_core::expression::{script::Language, script::Slot};
use lumit_core::markers::{Marker, MarkerEdit, MarkerKind};
use lumit_core::model::{LayerKind, TransformProp};
use lumit_core::ops::Op;
use lumit_core::time::{CompTime, FrameRate, Rational};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{bad, commit, exact, find_comp, find_layer, layer_now, Time};
use crate::{Host, ScriptError};

/// Markers as `comp.get` and `layer.get` answer them, on the comp's clock as
/// `markers.set` takes them. A layer keeps its own on its clock, `offset`
/// behind the comp's.
pub(super) fn markers_json(markers: &[Marker], rate: FrameRate, offset: Rational) -> Value {
    markers
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "time": exact(m.time.0.checked_add(offset).unwrap_or(m.time.0)),
                "label": m.label,
                "colour": m.colour,
                "duration": m.duration.map(|d| rate.frame_at(CompTime(d))),
                "chapter": matches!(m.kind, MarkerKind::Chapter),
            })
        })
        .collect()
}

/// The transform and camera properties `layer.keys` and `layer.get` know, and
/// the channels each is stored as.
pub(super) const PROPERTIES: &[(&str, &[TransformProp])] = &[
    ("anchor", &[TransformProp::AnchorX, TransformProp::AnchorY]),
    (
        "position",
        &[
            TransformProp::PositionX,
            TransformProp::PositionY,
            TransformProp::PositionZ,
        ],
    ),
    ("scale", &[TransformProp::ScaleX, TransformProp::ScaleY]),
    ("rotation", &[TransformProp::Rotation]),
    ("rotation_x", &[TransformProp::RotationX]),
    ("rotation_y", &[TransformProp::RotationY]),
    (
        "orientation",
        &[
            TransformProp::OrientationX,
            TransformProp::OrientationY,
            TransformProp::OrientationZ,
        ],
    ),
    ("opacity", &[TransformProp::Opacity]),
    (
        "point_of_interest",
        &[
            TransformProp::PoiX,
            TransformProp::PoiY,
            TransformProp::PoiZ,
        ],
    ),
    ("zoom", &[TransformProp::Zoom]),
    ("focus_distance", &[TransformProp::FocusDistance]),
    ("aperture", &[TransformProp::Aperture]),
];

/// One channel's animation as `layer.get` answers it: a number, keys on the
/// comp's clock, or an expression.
pub(super) fn animation_json(property: &Property, offset: Rational) -> Value {
    match &property.animation {
        Animation::Static(value) => json!(value),
        Animation::Keyframed(keys) => keys
            .iter()
            .map(|k| {
                let time = k.time.checked_add(offset).unwrap_or(k.time);
                json!({
                    "time": exact(time),
                    "value": k.value,
                    "hold": k.is_hold(),
                    "in": side_json(k.interp_in),
                    "out": side_json(k.interp_out),
                })
            })
            .collect(),
        Animation::Expression(text) => json!({ "expression": text }),
    }
}

/// One side of a key as `layer.keys` takes it, an ease in After Effects'
/// terms.
fn side_json(side: SideInterp) -> Value {
    match side {
        SideInterp::Hold => json!("hold"),
        SideInterp::Linear => json!("linear"),
        SideInterp::Auto { clamped: false, .. } => json!("auto"),
        SideInterp::Auto { clamped: true, .. } => json!("clamped"),
        SideInterp::Bezier { speed, influence } => {
            json!({ "speed": speed, "influence": influence * 100.0 })
        }
    }
}

/// A property's value on the wire: one number, or one per axis.
#[derive(Deserialize, Clone)]
#[serde(untagged)]
pub(super) enum Values {
    One(f64),
    Many(Vec<f64>),
}

impl Values {
    pub(super) fn list(self) -> Vec<f64> {
        match self {
            Values::One(v) => vec![v],
            Values::Many(v) => v,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub(super) enum Interp {
    #[default]
    Linear,
    Hold,
}

/// How a key meets the curve on one side: by name, or eased as After
/// Effects eases it, with one ease for every value or one for each.
#[derive(Deserialize, Clone)]
#[serde(untagged)]
pub(super) enum Side {
    Named(Named),
    Ease(Ease),
    Eases(Vec<Ease>),
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(super) enum Named {
    Linear,
    Hold,
    Auto,
    Clamped,
}

/// A speed in units a second and an influence in per cent.
#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub(super) struct Ease {
    pub(super) speed: Option<f64>,
    pub(super) influence: Option<f64>,
}

impl Side {
    /// This side on value `axis` of a key, through the mapping importing an
    /// After Effects project uses.
    pub(super) fn on(&self, axis: usize) -> SideInterp {
        let eased = |e: Option<&Ease>| {
            lumit_core::anim::ae_ease(e.and_then(|e| e.speed), e.and_then(|e| e.influence))
        };
        match self {
            Side::Named(Named::Linear) => SideInterp::Linear,
            Side::Named(Named::Hold) => SideInterp::Hold,
            Side::Named(Named::Auto) => SideInterp::Linear.with_tangent_mode(TangentMode::Auto),
            Side::Named(Named::Clamped) => SideInterp::Linear.with_tangent_mode(TangentMode::Clamp),
            Side::Ease(e) => eased(Some(e)),
            Side::Eases(list) => eased(list.get(axis).or_else(|| list.first())),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeyArgs {
    pub(super) time: Time,
    pub(super) value: Values,
    #[serde(default)]
    pub(super) interp: Interp,
    #[serde(rename = "in")]
    pub(super) into: Option<Side>,
    pub(super) out: Option<Side>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeysArgs {
    comp: String,
    layer: String,
    property: String,
    value: Option<Values>,
    keys: Option<Vec<KeyArgs>>,
    expression: Option<String>,
}

/// `keys` as one keyframed animation per value, on the layer's clock,
/// `offset` behind the comp's. `axes` says how many values a key's value
/// is, or why it is wrong.
pub(super) fn keyed(
    keys: Vec<KeyArgs>,
    rate: FrameRate,
    offset: Rational,
    axes: impl Fn(&Values) -> Result<usize, ScriptError>,
) -> Result<Vec<Animation>, ScriptError> {
    // One list of keys per axis, all at the same times.
    let mut keyed: Vec<Vec<Keyframe>> = Vec::new();
    for key in keys {
        let time = key.time.at(rate)?;
        let n = axes(&key.value)?;
        let values = key.value.list();
        if keyed.is_empty() {
            keyed.resize_with(n, Vec::new);
        } else if keyed.len() != n {
            return Err(bad("Every key needs the same number of values."));
        }
        for (n, (axis, value)) in keyed.iter_mut().zip(values).enumerate() {
            let made = Keyframe {
                time,
                value,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            };
            let made = match key.interp {
                Interp::Linear => made,
                Interp::Hold => made.to_hold(),
            };
            axis.push(Keyframe {
                interp_in: key.into.as_ref().map_or(made.interp_in, |s| s.on(n)),
                interp_out: key.out.as_ref().map_or(made.interp_out, |s| s.on(n)),
                ..made
            });
        }
    }
    keyed
        .into_iter()
        .map(|keys| {
            keys.into_iter()
                .map(|k| edits::key_on_layer_clock(k, offset))
                .collect::<Option<Vec<_>>>()
                .and_then(edits::keyframed)
                .ok_or_else(|| bad("Each key needs a time after the one before it."))
        })
        .collect()
}

/// What `property` holds before an expression takes it over.
pub(super) fn underneath(property: &Property) -> f64 {
    match &property.animation {
        Animation::Expression(_) => Slot::read(&property.extra).own().unwrap_or(0.0),
        _ => property.value_at(0.0),
    }
}

/// `held` driven by JavaScript expression `text`. A property with several
/// values is a channel each in Lumit, so each is told all of `values` and
/// which of them it is, as importing an After Effects expression does.
pub(super) fn expressed(held: &Property, text: &str, values: &[f64], axis: usize) -> Property {
    let mut next = held.clone();
    next.set_animation(Animation::Expression(text.to_owned()));
    let seed = Slot::read(&next.extra).seed;
    Slot::new(values, axis, seed)
        .in_language(Language::JavaScript)
        .write(&mut next.extra);
    next
}

pub(super) fn layer_keys(host: &dyn Host, a: KeysArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let channels = PROPERTIES
        .iter()
        .find(|(name, _)| *name == a.property)
        .map(|(_, channels)| *channels)
        .ok_or_else(|| bad(format!("Lumit has no property called {}.", a.property)))?;
    if channels.iter().any(|c| layer.prop(*c).is_none()) {
        return Err(bad(format!("{} is not on this layer.", a.property)));
    }
    // One number for a one-axis property, a list with one for each axis
    // otherwise, though position can leave Z alone.
    let axes = |values: &Values| -> Result<usize, ScriptError> {
        match (channels.len(), values) {
            (1, Values::One(_)) => Ok(1),
            (n, Values::Many(v)) if n > 1 && (2..=n).contains(&v.len()) => Ok(v.len()),
            (1, _) => Err(bad(format!("{} takes one number.", a.property))),
            (2, _) => Err(bad(format!("{} takes 2 values.", a.property))),
            _ => Err(bad(format!("{} takes 2 or 3 values.", a.property))),
        }
    };

    let animations: Vec<Animation> = match (a.value, a.keys, a.expression) {
        (Some(value), None, None) => {
            axes(&value)?;
            value.list().into_iter().map(Animation::Static).collect()
        }
        (None, Some(keys), None) => keyed(keys, comp.frame_rate, layer.start_offset.0, axes)?,
        (None, None, Some(text)) => {
            // A 2D layer's point has no Z in After Effects, so its expression
            // answers two values and Z is left as it is.
            let flat = !layer.switches.three_d && !matches!(layer.kind, LayerKind::Camera { .. });
            let used = if flat {
                channels.len().min(2)
            } else {
                channels.len()
            };
            let mut ops = Vec::new();
            for channel in channels.iter().take(used) {
                ops.extend(edits::transform_ops(
                    comp.id,
                    layer,
                    *channel,
                    Animation::Expression(text.clone()),
                    Some(Language::JavaScript),
                ));
            }
            let id = layer.id;
            commit(store, ops)?;
            return layer_now(store, comp.id, id);
        }
        _ => return Err(bad("Give a value, keys or an expression.")),
    };

    let mut ops = Vec::new();
    for (channel, animation) in channels.iter().zip(animations) {
        ops.extend(edits::transform_ops(
            comp.id, layer, *channel, animation, None,
        ));
    }
    let id = layer.id;
    commit(store, ops)?;
    layer_now(store, comp.id, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerArgs {
    id: Option<Uuid>,
    time: Time,
    #[serde(default)]
    label: String,
    colour: Option<u8>,
    duration: Option<i64>,
    chapter: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MarkersArgs {
    comp: String,
    layer: Option<String>,
    markers: Vec<MarkerArgs>,
}

pub(super) fn markers_set(host: &dyn Host, a: MarkersArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let rate = comp.frame_rate;
    let layer = a.layer.map(|key| find_layer(comp, &key)).transpose()?;
    // Times come in on the comp's clock, and a layer keeps its markers on
    // its own.
    let offset = layer.map_or(Rational::ZERO, |l| l.start_offset.0);
    let chapters: Vec<Option<bool>> = a.markers.iter().map(|m| m.chapter).collect();
    let edits = a
        .markers
        .into_iter()
        .map(|m| {
            let time = m.time.at(rate)?.checked_sub(offset);
            Ok(MarkerEdit {
                id: m.id.unwrap_or_else(Uuid::now_v7),
                time: CompTime(time.map_err(|_| bad("That time is out of range."))?),
                label: m.label,
                colour: m.colour,
                duration_frames: m.duration,
            })
        })
        .collect::<Result<Vec<_>, ScriptError>>()?;
    let count = edits.len();
    // Merged onto what is there, so a marker named by id keeps what a script
    // cannot say, such as being a beat.
    let held = layer.map_or(&comp.markers, |l| &l.markers);
    let mut markers = lumit_core::markers::merge_edits(edits, held, rate);
    for (marker, chapter) in markers.iter_mut().zip(chapters) {
        match chapter {
            Some(true) => marker.kind = MarkerKind::Chapter,
            Some(false) if matches!(marker.kind, MarkerKind::Chapter) => {
                marker.kind = MarkerKind::User;
            }
            _ => {}
        }
    }
    let op = match layer {
        Some(layer) => Op::SetLayerMarkers {
            comp: comp.id,
            layer: layer.id,
            markers,
        },
        None => Op::SetCompMarkers {
            comp: comp.id,
            markers,
        },
    };
    commit(store, vec![op])?;
    Ok(json!({ "markers": count }))
}
