//! Turning a [`Document`] into the snapshot JSON the panels read.
//!
//! # In plain terms
//!
//! A *snapshot* is the whole document written out as JSON for the Flutter side
//! to draw. "Snapshot v2" keeps every field v1 had (the item tree, undo flags,
//! path) and *adds* detail the Viewer, Timeline and editors need: for a
//! composition the `comp` block (its size, frame rate, frame count, layers and
//! markers); for a footage item its `media` metadata and probe `status`. The
//! rule is strictly additive — an older reader still finds everything it knew.
//!
//! Frames are integers derived from the composition's *own* frame rate the way
//! the egui frontend derives them (rational time, never threaded f64): a layer's
//! in/out frame is the frame containing its in/out point, and the frame count is
//! the comp's duration divided by one frame, rounded to the nearest whole frame.

use crate::media::MediaCache;
use crate::state::Bridge;
use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::markers::MarkerKind;
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectNamespace, EffectValue, Layer, LayerInputSource,
    LayerKind, LinearColour, MatteChannel, ProjectItem, TransformProp,
};
use lumit_core::time::{CompTime, Rational};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

/// The transform properties the read-back exposes, in a stable order, paired
/// with their snake_case names (the same vocabulary `set_transform` and the
/// keyframe ops speak). `position` seeds to the comp centre for a fresh layer
/// (`lumit-ui`'s `centred_transform`), so the read-back `value` already carries
/// that true current value — no separate defaults block is needed.
const TRANSFORM_PROPS: &[(&str, TransformProp)] = &[
    ("anchor_x", TransformProp::AnchorX),
    ("anchor_y", TransformProp::AnchorY),
    ("position_x", TransformProp::PositionX),
    ("position_y", TransformProp::PositionY),
    ("position_z", TransformProp::PositionZ),
    ("scale_x", TransformProp::ScaleX),
    ("scale_y", TransformProp::ScaleY),
    ("rotation", TransformProp::Rotation),
    ("rotation_x", TransformProp::RotationX),
    ("rotation_y", TransformProp::RotationY),
    ("opacity", TransformProp::Opacity),
];

/// The document tree as the Project panel reads it, plus the v2 detail. Walks
/// [`Document::root_items`] and nests each folder's children, so the JSON mirrors
/// the panel's real nesting rather than the flat storage. A malformed folder
/// cycle is broken by the `seen` set, never looped.
pub(crate) fn snapshot_value(bridge: &Bridge) -> Value {
    let doc = bridge.store.snapshot();
    let mut seen = HashSet::new();
    let items: Vec<Value> = doc
        .root_items()
        .into_iter()
        .filter_map(|id| item_value(&doc, &bridge.media, id, &mut seen))
        .collect();
    json!({
        "ok": true,
        "items": items,
        "can_undo": bridge.store.can_undo(),
        "can_redo": bridge.store.can_redo(),
        "path": bridge.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
    })
}

/// One item as `{id, name, kind, children, …}`. `children` is populated only for
/// folders (recursively). A composition additionally carries a `comp` block; a
/// footage item carries `status` and, once probed, a `media` block. Returns
/// `None` for an id already visited (cycle guard) or absent from the document.
fn item_value(
    doc: &Document,
    media: &MediaCache,
    id: Uuid,
    seen: &mut HashSet<Uuid>,
) -> Option<Value> {
    if !seen.insert(id) {
        return None;
    }
    let item = doc.item(id)?;
    let children: Vec<Value> = match item {
        ProjectItem::Folder(f) => f
            .children
            .iter()
            .filter_map(|child| item_value(doc, media, *child, seen))
            .collect(),
        _ => Vec::new(),
    };
    let mut obj = json!({
        "id": id.to_string(),
        "name": item.name(),
        "kind": item_kind(item),
        "children": children,
    });
    match item {
        ProjectItem::Composition(c) => {
            obj["comp"] = comp_value(doc, c);
        }
        ProjectItem::Footage(f) => {
            let (status, detail) = media.snapshot_for(f.id);
            obj["status"] = json!(status);
            if let Some(detail) = detail {
                obj["media"] = detail;
            }
        }
        _ => {}
    }
    Some(obj)
}

fn item_kind(item: &ProjectItem) -> &'static str {
    match item {
        ProjectItem::Footage(_) => "footage",
        ProjectItem::Folder(_) => "folder",
        ProjectItem::Composition(_) => "composition",
        ProjectItem::Solid(_) => "solid",
    }
}

/// A composition's v2 detail plus the v0.3 `work_area`: size, frame rate (as
/// the model stores it, `{num, den}`), the derived frame count, every layer, the
/// marker frames, and the work-area span as `[in_frame, out_frame]` (or null
/// for the full comp).
fn comp_value(doc: &Document, c: &Composition) -> Value {
    // FrameRate serialises to `{"num":…,"den":…}` — exactly what Dart expects,
    // and exact (no f64 rounding of the rate itself).
    let fps = serde_json::to_value(c.frame_rate).unwrap_or(json!({ "num": 0, "den": 1 }));
    let layers: Vec<Value> = c
        .layers
        .iter()
        .enumerate()
        .map(|(index, l)| layer_value(doc, c, index, l))
        .collect();
    // Markers as comp-frame indices (the frame containing each marker's time).
    let markers: Vec<i64> = c
        .markers
        .iter()
        .map(|m| c.frame_rate.frame_at(m.time))
        .collect();
    // v0.9: the same markers with their kind, so beat markers can be drawn
    // apart from user/chapter cues (`MarkerKind` — the model already
    // distinguishes them; the bare `markers` array flattened the distinction).
    // Additive: an older reader keeps reading `markers`.
    let marker_details: Vec<Value> = c
        .markers
        .iter()
        .map(|m| marker_detail_value(c, m))
        .collect();
    // Work area as `[in_frame, out_frame]`, or null when the comp has none
    // (the whole span plays). The B/N keys set it via `set_work_area_edge`.
    let work_area = match c.work_area {
        Some((a, b)) => json!([c.frame_rate.frame_at(a), c.frame_rate.frame_at(b)]),
        None => Value::Null,
    };
    json!({
        "width": c.width,
        "height": c.height,
        "fps": fps,
        "frame_count": comp_frame_count(c),
        "layers": layers,
        "markers": markers,
        "marker_details": marker_details,
        "work_area": work_area,
        // v0.4: the comp motion-blur master (read back; set via set_motion_blur).
        "motion_blur": {
            "enabled": c.motion_blur.enabled,
            "shutter_angle": c.motion_blur.shutter_angle,
            "shutter_phase": c.motion_blur.shutter_phase,
            "samples": c.motion_blur.samples,
        },
    })
}

/// One marker as `{frame, kind, confidence?, label, duration_frames?}` (v0.9).
/// `kind` is `user`/`beat`/`chapter` (the [`MarkerKind`] the model stores);
/// `confidence` is present only for a beat marker (its 0..1 onset prominence,
/// which `detect_beats` sets and `clear_beat_markers` selects on). `frame` is
/// the comp frame the marker's time lands on — the same value the bare
/// `markers` array carries, so the two agree.
fn marker_detail_value(c: &Composition, m: &lumit_core::markers::Marker) -> Value {
    let (kind, confidence) = match m.kind {
        MarkerKind::User => ("user", None),
        MarkerKind::Beat { confidence } => ("beat", Some(confidence)),
        MarkerKind::Chapter => ("chapter", None),
    };
    let mut obj = json!({
        "frame": c.frame_rate.frame_at(m.time),
        "kind": kind,
        "label": m.label,
    });
    if let Some(conf) = confidence {
        obj["confidence"] = json!(conf);
    }
    if let Some(dur) = m.duration {
        obj["duration_frames"] = json!(c.frame_rate.frame_at(CompTime(dur)));
    }
    obj
}

/// One layer's v2 detail plus the v0.3 read-back: `in_frame`/`out_frame` are the
/// comp frames containing the layer's in/out points; `switches` mirrors the
/// model's [`lumit_core::model::Switches`] field names verbatim; `transform`
/// carries every property's current value and keyframes; and the identity links
/// (`source_item_id`, `source_comp_id`, `colour`) name what a layer references.
fn layer_value(doc: &Document, c: &Composition, index: usize, l: &Layer) -> Value {
    let switches = serde_json::to_value(l.switches).unwrap_or(json!({}));
    let mut obj = json!({
        "id": l.id.to_string(),
        "index": index,
        "name": l.name,
        "kind": layer_kind(&l.kind),
        "in_frame": c.frame_rate.frame_at(l.in_point),
        "out_frame": c.frame_rate.frame_at(l.out_point),
        // v0.9: the layer's start offset (where layer time 0 sits on the comp
        // timeline) and its in/out points in seconds — the ingredients the
        // overrun HOLD hatch needs (`overrun_span_secs` in speed_rows.rs) that
        // the frame-only read-back could not give: start offset plus the local
        // in/out. `start_offset` seeds to 0 for a fresh layer.
        "start_offset_frame": c.frame_rate.frame_at(l.start_offset),
        "start_offset_secs": l.start_offset.0.to_f64(),
        "in_secs": l.in_point.0.to_f64(),
        "out_secs": l.out_point.0.to_f64(),
        "label": l.label,
        "switches": switches,
        "transform": transform_value(c, l),
        "effects": effects_value(c, l),
        // v0.4 columns: the blend mode (serde variant name, round-trip stable),
        // the matte, and the transform parent (a layer id or null).
        "blend_mode": serde_json::to_value(l.blend).unwrap_or(json!("Normal")),
        "matte": matte_value(l),
        "parent": l.parent.map(|p| json!(p.to_string())).unwrap_or(Value::Null),
    });
    // Identity links, mirroring the real `LayerKind` variant fields.
    match &l.kind {
        LayerKind::Footage { item, retime } => {
            obj["source_item_id"] = json!(item.to_string());
            if let Some(r) = retime {
                obj["retime"] = retime_value(c, l, r);
            }
        }
        LayerKind::Precomp { comp } => {
            obj["source_comp_id"] = json!(comp.to_string());
        }
        LayerKind::Solid { def } => {
            if let Some(solid) = doc.solid(*def) {
                let LinearColour([r, g, b, a]) = solid.colour;
                obj["colour"] = json!([r, g, b, a]);
                // v0.9: the solid's size read back (colour already crossed).
                obj["solid_size"] = json!([solid.width, solid.height]);
            }
        }
        // v0.9: text content/size/fill, camera zoom, and sequence clips read
        // back from the model — the editors held these in a session-edit map
        // because the snapshot did not carry them.
        LayerKind::Text { document } => {
            let LinearColour([r, g, b, a]) = document.fill;
            obj["text"] = json!({
                "content": document.text,
                "size": document.size,
                "fill": [r, g, b, a],
            });
        }
        LayerKind::Camera { zoom } => {
            // Zoom is a Property (animatable); read it back like a transform
            // property so the editor sees its value and any keyframes.
            obj["camera"] = property_value(c, l, zoom);
        }
        LayerKind::Sequence { clips } => {
            let clips: Vec<Value> = clips
                .iter()
                .map(|clip| crate::sequence::clip_value(c, l, clip))
                .collect();
            obj["clips"] = Value::Array(clips);
        }
        _ => {}
    }
    obj
}

/// A layer's matte as `{source, channel, inverted, source_mode}`, or null when
/// the layer has none (v0.4). `source` is the matte layer's id; `channel` is
/// `alpha`/`luma`; `source_mode` is how the matte samples its source
/// (`none`/`masks`/`effects_and_masks`), mirroring [`MatteRef`].
fn matte_value(l: &Layer) -> Value {
    match &l.matte {
        None => Value::Null,
        Some(m) => json!({
            "source": m.layer.to_string(),
            "channel": match m.channel {
                MatteChannel::Alpha => "alpha",
                MatteChannel::Luma => "luma",
            },
            "inverted": m.inverted,
            "source_mode": match m.source {
                LayerInputSource::None => "none",
                LayerInputSource::Masks => "masks",
                LayerInputSource::EffectsAndMasks => "effects_and_masks",
            },
        }),
    }
}

/// A footage layer's Retime store as the Timeline/graph reads it (v0.4). Chosen
/// shape: `reverse`, `interpolation` (`nearest`/`blend`/`flow`), the `boundaries`
/// (each `{t_frame, t_seconds, s_seconds, smooth}` — `t_frame` is the boundary's
/// local time as a comp frame, the durable seconds kept alongside), and the
/// `segments` (each tagged `rate` with `{v0, v1, ease}` or `map` with
/// `{m0, m1, b0, b1}`), mirroring [`lumit_core::retime`]'s own types. Segment `i`
/// spans `boundaries[i]..boundaries[i+1]`.
fn retime_value(c: &Composition, l: &Layer, r: &lumit_core::retime::Retime) -> Value {
    use lumit_core::retime::{Ease, RetimeSegment};
    let ease_name = |e: Ease| match e {
        Ease::Linear => "Linear",
        Ease::Slow => "Slow",
        Ease::Fast => "Fast",
        Ease::Smooth => "Smooth",
        Ease::Sharp => "Sharp",
    };
    // A boundary's local time in comp frames: layer-local seconds + the layer's
    // start offset, then the comp's own rate (the same map keyframes use).
    let boundaries: Vec<Value> = r
        .boundaries
        .iter()
        .map(|b| {
            let comp_time =
                b.t.checked_add(l.start_offset.0)
                    .map(CompTime)
                    .unwrap_or(CompTime(b.t));
            json!({
                "t_frame": c.frame_rate.frame_at(comp_time),
                "t_seconds": b.t.to_f64(),
                "s_seconds": b.s.to_f64(),
                "smooth": b.smooth,
            })
        })
        .collect();
    let segments: Vec<Value> = r
        .segments
        .iter()
        .map(|s| match s {
            RetimeSegment::Rate(seg) => json!({
                "kind": "rate",
                "v0": seg.v0.to_f64(),
                "v1": seg.v1.to_f64(),
                "ease": ease_name(seg.ease),
            }),
            RetimeSegment::Map(seg) => json!({
                "kind": "map",
                "m0": seg.m0.to_f64(),
                "m1": seg.m1.to_f64(),
                "b0": seg.b0.to_f64(),
                "b1": seg.b1.to_f64(),
            }),
        })
        .collect();
    let interp = match &r.interpolation {
        lumit_core::retime::Interpolation::Nearest => "nearest",
        lumit_core::retime::Interpolation::Blend => "blend",
        lumit_core::retime::Interpolation::Flow(_) => "flow",
    };
    json!({
        "reverse": r.allow_reverse,
        "interpolation": interp,
        "boundaries": boundaries,
        "segments": segments,
    })
}

/// A layer's whole transform as `{ prop: {value, animated, keys?} }`, one entry
/// per [`TRANSFORM_PROPS`] name. See [`property_value`] for a property's shape.
fn transform_value(c: &Composition, l: &Layer) -> Value {
    let mut map = serde_json::Map::new();
    for (name, prop) in TRANSFORM_PROPS {
        map.insert(
            (*name).to_owned(),
            property_value(c, l, l.transform.get(*prop)),
        );
    }
    Value::Object(map)
}

/// One property's read-back: its current `value` (the static value, or the
/// value evaluated at layer time 0 when keyframed), whether it is `animated`,
/// and — only when animated — its `keys`. Mirrors `lumit-core`'s
/// [`Property`]/[`Animation`] faithfully.
fn property_value(c: &Composition, l: &Layer, p: &Property) -> Value {
    let value = p.value_at(0.0);
    match &p.animation {
        Animation::Keyframed(keys) if !keys.is_empty() => {
            let keys: Vec<Value> = keys.iter().map(|k| keyframe_value(c, l, k)).collect();
            json!({ "value": value, "animated": true, "keys": keys })
        }
        _ => json!({ "value": value, "animated": false }),
    }
}

/// One keyframe as `{frame, value, interp_in, interp_out}`, plus `bezier_in`/
/// `bezier_out` (`{speed, influence}`) on whichever side carries a Bezier tangent
/// (v0.4, the graph editor read-back). The keyframe time is layer-local; the
/// reported `frame` is the comp frame it lands on (layer time plus the layer's
/// start offset, then the comp's own rate), so the Timeline draws it under the
/// right column. `interp_in`/`interp_out` are the [`SideInterp`] variant names.
fn keyframe_value(c: &Composition, l: &Layer, k: &Keyframe) -> Value {
    let comp_time = k
        .time
        .checked_add(l.start_offset.0)
        .map(CompTime)
        .unwrap_or(CompTime(k.time));
    let mut obj = json!({
        "frame": c.frame_rate.frame_at(comp_time),
        "value": k.value,
        "interp_in": side_interp_name(k.interp_in),
        "interp_out": side_interp_name(k.interp_out),
    });
    if let Some(b) = side_bezier(k.interp_in) {
        obj["bezier_in"] = b;
    }
    if let Some(b) = side_bezier(k.interp_out) {
        obj["bezier_out"] = b;
    }
    obj
}

/// The [`SideInterp`] variant name (`Hold`/`Linear`/`Bezier`) — the interp
/// vocabulary the graph editor speaks and `set_keyframe_interp` accepts.
fn side_interp_name(s: SideInterp) -> &'static str {
    match s {
        SideInterp::Hold => "Hold",
        SideInterp::Linear => "Linear",
        SideInterp::Bezier { .. } => "Bezier",
    }
}

/// A Bezier side's `{speed, influence}` (v0.4), or `None` for a Hold/Linear
/// side — the tangent the graph editor draws and `set_keyframe_interp` sets.
fn side_bezier(s: SideInterp) -> Option<Value> {
    match s {
        SideInterp::Bezier { speed, influence } => {
            Some(json!({ "speed": speed, "influence": influence }))
        }
        _ => None,
    }
}

/// A layer's effect stack as `[{id, name, enabled, params}]` (v0.3). `name` is
/// the effect's stable match name; each param carries its `kind` tag and a
/// read-back `value` (scalar/colour evaluated at layer time 0, enums/bools as
/// stored). Exotic kinds carry a null value — the Dart side shows the row but
/// leaves the control to a later phase.
fn effects_value(c: &Composition, l: &Layer) -> Value {
    let effects: Vec<Value> = l.effects.iter().map(|e| effect_value(c, l, e)).collect();
    Value::Array(effects)
}

fn effect_value(c: &Composition, l: &Layer, e: &EffectInstance) -> Value {
    // The declaring schema, for the parameter ranges the drag controls clamp to
    // (v0.5). A schema a newer/older build does not know simply omits the range.
    let schema = lumit_core::fx::schema(&e.effect.match_name);
    let params: Vec<Value> = e
        .params
        .iter()
        .map(|p| {
            let mut obj = effect_param_value(c, l, &p.value);
            obj["name"] = json!(p.id);
            if let Some(range) = schema
                .and_then(|s| s.params.iter().find(|ps| ps.id == p.id))
                .and_then(|ps| param_range(&ps.kind))
            {
                obj["range"] = range;
            }
            obj
        })
        .collect();
    json!({
        "id": e.id.to_string(),
        "name": e.effect.match_name,
        // v0.9: the full EffectKey identity, so a `.lumfx` preset round-trips
        // byte-faithfully — the match name alone dropped the namespace and
        // version the frame key and plugin resolver rely on.
        "namespace": namespace_name(e.effect.namespace),
        "version": e.effect.version,
        "sample_temporally": e.sample_temporally,
        "enabled": e.enabled,
        "params": params,
    })
}

/// The [`EffectNamespace`] tag as a stable string (`builtin`/`ofx`/`lfx`/
/// `placeholder`), so a preset carries which implementation an effect came from.
fn namespace_name(ns: EffectNamespace) -> &'static str {
    match ns {
        EffectNamespace::Builtin => "builtin",
        EffectNamespace::Ofx => "ofx",
        EffectNamespace::Lfx => "lfx",
        EffectNamespace::Placeholder => "placeholder",
    }
}

/// A parameter's declared edit range for the read-back (v0.5): a `Float`'s hard
/// bounds (`min`/`max`, either nullable) and its `slider` soft range; a
/// `Colour`'s per-channel `min`/`max`; a `Choice`'s `options`. Other kinds carry
/// no range (the drag control is unbounded or has no numeric range). The schema
/// declares no step, so none is reported — the control chooses its own.
fn param_range(kind: &lumit_core::fx::ParamKind) -> Option<Value> {
    use lumit_core::fx::ParamKind;
    match kind {
        ParamKind::Float { slider, hard, .. } => Some(json!({
            "min": hard.0,
            "max": hard.1,
            "slider_min": slider.0,
            "slider_max": slider.1,
        })),
        ParamKind::Colour { range, .. } => Some(json!({
            "min": range.0,
            "max": range.1,
        })),
        ParamKind::Choice { options, .. } => Some(json!({ "options": options })),
        _ => None,
    }
}

/// A parameter's read-back object `{kind, value, ...}`. Scalars and colours
/// evaluate at layer time 0; enums/bools/seeds read their stored value; the
/// file/layer kinds report their tag with a null value (no editor yet).
///
/// v0.9: the animatable kinds (scalar/colour/point) also carry their animation
/// state — an `animated` flag and, when animated, their `keys` — exactly as the
/// transform read-back does, so the Effect controls can draw a stopwatch and a
/// keyframe navigator on an effect parameter. A scalar carries one `keys` list;
/// a point/colour carry per-axis/channel lists (`keys_x`/`keys_y`,
/// `keys_r`/`keys_g`/`keys_b`/`keys_a`).
fn effect_param_value(c: &Composition, l: &Layer, v: &EffectValue) -> Value {
    match v {
        EffectValue::Float(p) => {
            let mut obj = json!({ "kind": "scalar", "value": p.value_at(0.0) });
            add_animation(&mut obj, "keys", c, l, p);
            obj
        }
        EffectValue::Colour(ch) => {
            let mut obj = json!({
                "kind": "colour",
                "value": [
                    ch[0].value_at(0.0),
                    ch[1].value_at(0.0),
                    ch[2].value_at(0.0),
                    ch[3].value_at(0.0),
                ],
            });
            let channels = ["keys_r", "keys_g", "keys_b", "keys_a"];
            let animated = ch.iter().any(|p| p.is_animated());
            obj["animated"] = json!(animated);
            for (name, p) in channels.iter().zip(ch.iter()) {
                add_channel_keys(&mut obj, name, c, l, p);
            }
            obj
        }
        EffectValue::Point(x, y) => {
            let mut obj = json!({
                "kind": "point",
                "value": [x.value_at(0.0), y.value_at(0.0)],
            });
            obj["animated"] = json!(x.is_animated() || y.is_animated());
            add_channel_keys(&mut obj, "keys_x", c, l, x);
            add_channel_keys(&mut obj, "keys_y", c, l, y);
            obj
        }
        EffectValue::Choice(i) => json!({ "kind": "enum", "value": i }),
        EffectValue::Bool(b) => json!({ "kind": "bool", "value": b }),
        EffectValue::Seed(s) => json!({ "kind": "seed", "value": s }),
        EffectValue::File(_) => json!({ "kind": "file", "value": Value::Null }),
        EffectValue::Layer(_) => json!({ "kind": "layer", "value": Value::Null }),
    }
}

/// Set `obj["animated"]` and, when the property is keyframed, `obj[keys_field]`
/// to its keys (mirroring [`property_value`]'s single-property read-back).
fn add_animation(obj: &mut Value, keys_field: &str, c: &Composition, l: &Layer, p: &Property) {
    match &p.animation {
        Animation::Keyframed(keys) if !keys.is_empty() => {
            obj["animated"] = json!(true);
            let keys: Vec<Value> = keys.iter().map(|k| keyframe_value(c, l, k)).collect();
            obj[keys_field] = Value::Array(keys);
        }
        _ => {
            obj["animated"] = json!(false);
        }
    }
}

/// Set `obj[keys_field]` to a channel/axis property's keys when it is keyframed
/// (the per-channel/per-axis sibling of [`add_animation`], which set the shared
/// `animated` flag once for the whole colour/point).
fn add_channel_keys(obj: &mut Value, keys_field: &str, c: &Composition, l: &Layer, p: &Property) {
    if let Animation::Keyframed(keys) = &p.animation {
        if !keys.is_empty() {
            let keys: Vec<Value> = keys.iter().map(|k| keyframe_value(c, l, k)).collect();
            obj[keys_field] = Value::Array(keys);
        }
    }
}

/// The layer-kind tag, mirroring the [`LayerKind`] variant names.
fn layer_kind(kind: &LayerKind) -> &'static str {
    match kind {
        LayerKind::Footage { .. } => "footage",
        LayerKind::Solid { .. } => "solid",
        LayerKind::Precomp { .. } => "precomp",
        LayerKind::Text { .. } => "text",
        LayerKind::Camera { .. } => "camera",
        LayerKind::Sequence { .. } => "sequence",
        LayerKind::Adjustment => "adjustment",
    }
}

/// The comp's frame count: duration ÷ one-frame, rounded to the nearest whole
/// frame — the same quantity `lumit-ui`'s `comp_frame_count` computes, but kept
/// on rational time (no f64 threading) and at least one frame.
fn comp_frame_count(c: &Composition) -> i64 {
    let Ok(frame_dur) = c.frame_rate.frame_duration() else {
        return 1;
    };
    let Ok(frames) = c.duration.0.checked_div(frame_dur.0) else {
        return 1;
    };
    round_rational(frames).max(1)
}

/// Round a rational to the nearest integer (ties toward +∞), in i128 so the
/// doubling cannot overflow a well-formed rate. Frame counts are non-negative,
/// where this agrees with f64 rounding.
fn round_rational(r: Rational) -> i64 {
    let num = i128::from(r.num());
    let den = i128::from(r.den()); // invariant: > 0
    let doubled = num * 2 + den; // round-half-up numerator over 2·den
    i64::try_from(doubled.div_euclid(den * 2)).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::markers::Marker;
    use lumit_core::model::{
        FootageItem, Layer, LayerKind, LinearColour, MediaRef, MotionBlur, Switches, TransformGroup,
    };
    use lumit_core::ops::Op;
    use lumit_core::store::DocumentStore;
    use lumit_core::time::{CompTime, Duration, FrameRate};

    fn ct(n: i64) -> CompTime {
        CompTime(Rational::new(n, 1).unwrap())
    }

    #[test]
    fn round_rational_rounds_to_nearest() {
        assert_eq!(round_rational(Rational::new(7, 2).unwrap()), 4); // 3.5 → 4
        assert_eq!(round_rational(Rational::new(5, 2).unwrap()), 3); // 2.5 → 3
        assert_eq!(round_rational(Rational::new(10, 3).unwrap()), 3); // 3.33 → 3
        assert_eq!(round_rational(Rational::new(300, 1).unwrap()), 300);
    }

    /// Snapshot v2: a comp with two layers, switches, markers and a footage
    /// item's probe status all serialise into the expected shape. Built through
    /// the real store so the JSON is exactly what the bridge would emit.
    #[test]
    fn comp_with_layers_serialises_v2_shape() {
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![
                sample_layer("top", ct(1), ct(4)),
                sample_layer("bottom", ct(0), ct(5)),
            ],
            markers: vec![Marker::user(Uuid::now_v7(), Rational::new(2, 1).unwrap())],
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let bridge = Bridge {
            store,
            path: None,
            media: MediaCache::default(),
            journal: None,
            preview: None,
        };
        let snap = snapshot_value(&bridge);

        let comp_json = &snap["items"][0];
        assert_eq!(comp_json["kind"], json!("composition"));
        let comp_block = &comp_json["comp"];
        assert_eq!(comp_block["width"], json!(1920));
        assert_eq!(comp_block["height"], json!(1080));
        assert_eq!(comp_block["fps"], json!({ "num": 60, "den": 1 }));
        // 5 s at 60 fps = 300 frames.
        assert_eq!(comp_block["frame_count"], json!(300));

        let layers = comp_block["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 2);
        // Index 0 is the top layer; in/out frames derive from 60 fps.
        assert_eq!(layers[0]["index"], json!(0));
        assert_eq!(layers[0]["name"], json!("top"));
        assert_eq!(layers[0]["kind"], json!("footage"));
        assert_eq!(layers[0]["in_frame"], json!(60)); // 1 s
        assert_eq!(layers[0]["out_frame"], json!(240)); // 4 s
        let sw = &layers[0]["switches"]; // switches mirror the model's field names
        assert_eq!(sw["visible"], json!(true));
        assert_eq!(sw["audible"], json!(true));
        assert_eq!(sw["locked"], json!(false));
        assert_eq!(sw["solo"], json!(false));
        assert_eq!(sw["motion_blur"], json!(false));
        assert_eq!(sw["fx"], json!(true));
        assert_eq!(sw["three_d"], json!(false));
        assert_eq!(sw["collapse"], json!(false));

        // Markers are comp-frame indices: 2 s → frame 120.
        assert_eq!(comp_block["markers"], json!([120]));
        assert!(bridge.store.can_undo());
    }

    /// Snapshot v3: a footage layer carries its transform read-back (each
    /// property `{value, animated}`, the position seeded to the comp centre) and
    /// its `source_item_id`; the comp carries `work_area` (null here). Built
    /// through the real store so the JSON is exactly what the bridge emits.
    #[test]
    fn layer_carries_transform_readback_and_identity_link() {
        let item = Uuid::now_v7();
        let mut layer = sample_layer("clip", ct(0), ct(5));
        layer.kind = LayerKind::Footage { item, retime: None };
        // Seed position at the comp centre, as `centred_transform` would.
        layer.transform.position_x = lumit_core::anim::Property::fixed(960.0);
        layer.transform.position_y = lumit_core::anim::Property::fixed(540.0);
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let bridge = Bridge {
            store,
            path: None,
            media: MediaCache::default(),
            journal: None,
            preview: None,
        };
        let snap = snapshot_value(&bridge);
        let l = &snap["items"][0]["comp"]["layers"][0];
        // Identity link and the effects array (empty here).
        assert_eq!(l["source_item_id"], json!(item.to_string()));
        assert_eq!(l["effects"], json!([]));
        // Transform read-back: position reads back the seeded centre, static.
        let tr = &l["transform"];
        assert_eq!(tr["position_x"]["value"], json!(960.0));
        assert_eq!(tr["position_x"]["animated"], json!(false));
        assert!(tr["position_x"].get("keys").is_none(), "static has no keys");
        assert_eq!(tr["opacity"]["value"], json!(100.0));
        assert_eq!(tr["scale_x"]["value"], json!(100.0));
        // Work area is null (full comp).
        assert_eq!(snap["items"][0]["comp"]["work_area"], Value::Null);
    }

    /// A footage item without a cache entry reports status "unprobed" and no
    /// media block — the shape a `--no-default-features` build always produces.
    #[test]
    fn footage_without_a_probe_is_unprobed() {
        let footage = FootageItem {
            id: Uuid::now_v7(),
            name: "clip.mp4".into(),
            media: MediaRef {
                relative_path: "clip.mp4".into(),
                absolute_path: String::new(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
        };
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(footage)),
            })
            .unwrap();
        let bridge = Bridge {
            store,
            path: None,
            media: MediaCache::default(),
            journal: None,
            preview: None,
        };
        let snap = snapshot_value(&bridge);
        assert_eq!(snap["items"][0]["status"], json!("unprobed"));
        assert!(snap["items"][0].get("media").is_none());
    }

    /// Build a one-layer 60 fps comp and its bridge, so a test can inspect the
    /// single layer's snapshot. The layer spans comp [0, 5] s.
    fn bridge_with_layer(layer: Layer) -> Bridge {
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        Bridge {
            store,
            path: None,
            media: MediaCache::default(),
            journal: None,
            preview: None,
        }
    }

    /// v0.9: `marker_details` carries each marker's kind, and a beat marker its
    /// 0..1 confidence — so beat markers can be drawn apart from user cues. The
    /// bare `markers` frame array is unchanged (additive).
    #[test]
    fn marker_details_carry_kind_and_confidence() {
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 640,
            height: 480,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: vec![
                Marker::user(Uuid::now_v7(), Rational::new(1, 1).unwrap()),
                Marker::beat(Uuid::now_v7(), Rational::new(2, 1).unwrap(), 0.75),
            ],
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let bridge = Bridge {
            store,
            path: None,
            media: MediaCache::default(),
            journal: None,
            preview: None,
        };
        let snap = snapshot_value(&bridge);
        let block = &snap["items"][0]["comp"];
        // The bare array is unchanged: 1 s → 60, 2 s → 120.
        assert_eq!(block["markers"], json!([60, 120]));
        let details = block["marker_details"].as_array().unwrap();
        assert_eq!(details.len(), 2);
        assert_eq!(details[0]["kind"], json!("user"));
        assert_eq!(details[0]["frame"], json!(60));
        assert!(details[0].get("confidence").is_none());
        assert_eq!(details[1]["kind"], json!("beat"));
        assert_eq!(details[1]["frame"], json!(120));
        assert_eq!(details[1]["confidence"], json!(0.75_f32));
    }

    /// v0.9: a layer carries its `start_offset` (frame + seconds) and its in/out
    /// points in seconds — the overrun-hatch ingredients the frame-only
    /// read-back lacked.
    #[test]
    fn layer_carries_start_offset_and_local_in_out() {
        let mut layer = sample_layer("clip", ct(2), ct(4));
        layer.start_offset = ct(1);
        let bridge = bridge_with_layer(layer);
        let snap = snapshot_value(&bridge);
        let l = &snap["items"][0]["comp"]["layers"][0];
        assert_eq!(l["start_offset_frame"], json!(60)); // 1 s at 60 fps
        assert_eq!(l["start_offset_secs"], json!(1.0));
        assert_eq!(l["in_secs"], json!(2.0));
        assert_eq!(l["out_secs"], json!(4.0));
    }

    /// v0.9: an effect reads back its full `EffectKey` identity (namespace +
    /// version, alongside the match name) and each animatable parameter its
    /// animation state — an `animated` flag and, when animated, its `keys`.
    #[test]
    fn effect_identity_and_param_animation_read_back() {
        use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
        use lumit_core::model::EffectValue;
        let mut layer = sample_layer("fx", ct(0), ct(5));
        let mut effect = lumit_core::fx::instantiate("blur").expect("blur exists");
        // Animate the first Float parameter with two keys at layer time 0 and 1.
        let pi = effect
            .params
            .iter()
            .position(|p| matches!(p.value, EffectValue::Float(_)))
            .expect("blur has a float param");
        let keys = vec![
            Keyframe {
                time: Rational::new(0, 1).unwrap(),
                value: 5.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
            Keyframe {
                time: Rational::new(1, 1).unwrap(),
                value: 20.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
        ];
        effect.params[pi].value = EffectValue::Float(Property {
            animation: Animation::Keyframed(keys),
            extra: serde_json::Map::new(),
        });
        let pid = effect.params[pi].id.clone();
        layer.effects = vec![effect];
        let bridge = bridge_with_layer(layer);
        let snap = snapshot_value(&bridge);
        let e = &snap["items"][0]["comp"]["layers"][0]["effects"][0];
        assert_eq!(e["namespace"], json!("builtin"));
        assert_eq!(e["version"], json!(1));
        assert_eq!(e["name"], json!("blur"));
        // Find the animated param and check its animation state.
        let params = e["params"].as_array().unwrap();
        let p = params
            .iter()
            .find(|p| p["name"] == json!(pid))
            .expect("the animated param is present");
        assert_eq!(p["kind"], json!("scalar"));
        assert_eq!(p["animated"], json!(true));
        let keys = p["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0]["frame"], json!(0));
        assert_eq!(keys[1]["frame"], json!(60)); // 1 s at 60 fps
        assert_eq!(keys[1]["value"], json!(20.0));
        // A non-animated param reports animated:false with no keys.
        let other = params
            .iter()
            .find(|p| p["kind"] == json!("scalar") && p["name"] != json!(pid));
        if let Some(other) = other {
            assert_eq!(other["animated"], json!(false));
            assert!(other.get("keys").is_none());
        }
    }

    /// v0.9: text, solid and camera layers read their asset back from the model
    /// (text content/size/fill, solid size, camera zoom) rather than a
    /// session-edit map.
    #[test]
    fn text_solid_camera_read_back() {
        use lumit_core::anim::Property;
        use lumit_core::model::{SolidDef, TextDocument};
        // Text layer.
        let mut text = sample_layer("title", ct(0), ct(5));
        text.kind = LayerKind::Text {
            document: TextDocument {
                text: "Hello".into(),
                size: 72.0,
                fill: LinearColour([1.0, 0.5, 0.25, 1.0]),
                extra: serde_json::Map::new(),
            },
        };
        let snap = snapshot_value(&bridge_with_layer(text));
        let l = &snap["items"][0]["comp"]["layers"][0];
        assert_eq!(l["text"]["content"], json!("Hello"));
        assert_eq!(l["text"]["size"], json!(72.0));
        assert_eq!(l["text"]["fill"], json!([1.0, 0.5, 0.25, 1.0]));

        // Camera layer: zoom reads back like a property.
        let mut cam = sample_layer("cam", ct(0), ct(5));
        cam.kind = LayerKind::Camera {
            zoom: Property::fixed(1200.0),
        };
        let snap = snapshot_value(&bridge_with_layer(cam));
        let l = &snap["items"][0]["comp"]["layers"][0];
        assert_eq!(l["camera"]["value"], json!(1200.0));
        assert_eq!(l["camera"]["animated"], json!(false));

        // Solid layer: size reads back alongside the colour. The solid def must
        // live in the document for the read-back to resolve.
        let def = SolidDef {
            id: Uuid::now_v7(),
            name: "Red".into(),
            colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
            width: 800,
            height: 600,
            extra: serde_json::Map::new(),
        };
        let mut solid = sample_layer("solid", ct(0), ct(5));
        solid.kind = LayerKind::Solid { def: def.id };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![solid],
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Solid(def)),
            })
            .unwrap();
        store
            .commit(Op::AddItem {
                index: 1,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let bridge = Bridge {
            store,
            path: None,
            media: MediaCache::default(),
            journal: None,
            preview: None,
        };
        let snap = snapshot_value(&bridge);
        // The comp is at root index 1 (solid at 0).
        let comp_item = snap["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["kind"] == json!("composition"))
            .unwrap();
        let l = &comp_item["comp"]["layers"][0];
        assert_eq!(l["colour"], json!([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(l["solid_size"], json!([800, 600]));
    }

    fn sample_layer(name: &str, in_point: CompTime, out_point: CompTime) -> Layer {
        Layer {
            id: Uuid::now_v7(),
            name: name.into(),
            kind: LayerKind::Footage {
                item: Uuid::now_v7(),
                retime: None,
            },
            in_point,
            out_point,
            start_offset: ct(0),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }
}
