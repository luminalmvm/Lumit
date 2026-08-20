//! Layers: kind, span, switches, blend, matte, parenting, masks, markers, and
//! the two things After Effects calls time — "time stretch" and "time remap" —
//! both of which are one Retime in Lumit
//! ([docs/11](../../../../docs/11-AE-IMPORT.md) §3, docs/04-RETIMING.md).
//!
//! # In plain terms
//!
//! A layer is the part of an import where the most small facts have to survive
//! at once, and most of them map straight across: the stacking order is the
//! order, the in and out points are the in and out points, the switches are
//! the switches. Three do not, and each gets its own care here.
//!
//! **The matte.** After Effects has had two ways of saying "use that layer's
//! alpha to cut this one out": the old way, where the matte is simply whatever
//! sits directly above, and the new way, where the layer is chosen from a
//! dropdown. Lumit has only the second, so the old form is resolved to the
//! layer above and both arrive as one thing.
//!
//! **Time stretch.** A layer stretched to 50% plays at double speed. Lumit has
//! no stretch switch — it has Retime, which says the same thing in a more
//! general way — so the stretch becomes the straight-line Retime that plays
//! the source at that rate. A *negative* stretch means "backwards", and comes
//! across as the same line reflected: the layer starts at the far end of its
//! source and walks back to the beginning.
//!
//! **Time remap.** This one is already a Retime — AE's time-remap value graph
//! and Lumit's Retime value graph are the same mathematical object — so it is
//! a value copy, and a hold key on it *is* a freeze without anything having to
//! translate it. When a layer has both, the remap wins, because in After
//! Effects it does too.

use std::collections::BTreeMap;

use lumit_core::anim::Property as LumProperty;
use lumit_core::markers::{Marker, MarkerKind};
use lumit_core::mask::{BezierPath, Mask, MaskMode, PathKeyframe, Vertex};
use lumit_core::model::{
    BlendMode, Layer, LayerInputSource, LayerKind, LightDef, LightKind, MatteChannel, MatteRef,
    Switches, TextDocument, TransformGroup,
};
use lumit_core::retime::Interpolation;
use lumit_core::time::{CompTime, Rational};
use uuid::Uuid;

use crate::capture::{Layer as AeLayer, Marker as AeMarker, Property, Shape};
use crate::report::{ItemPath, Outcome, Reason};

use super::effects::map_effect;
use super::props::{
    ae_map, child, display_name, from_node, group, match_name_of, ramp, scalar, still,
};
use super::{srgb_to_linear, Conv, ItemKind, Items};

/// One layer, whole. Never fails: a layer whose source has vanished still
/// arrives, keeps its slot and its transform, and says so in the report.
pub(crate) fn map_layer(
    conv: &mut Conv<'_>,
    comp: &ItemPath,
    ae: &AeLayer,
    items: &Items,
    ids: &BTreeMap<u32, Uuid>,
) -> Option<Layer> {
    let name = ae.name.clone().unwrap_or_else(|| "(unnamed)".to_string());
    // A layer with no stacking index cannot be parented to, cannot be a matte,
    // and cannot be placed. It is the one layer-level skip, and it is a row
    // rather than a failure: the rest of the composition still imports.
    let Some(id) = ae.index.and_then(|index| ids.get(&index).copied()) else {
        conv.report
            .row(comp.layer(&name), Outcome::Skipped, Reason::LayerUnreadable);
        return None;
    };
    let index = ae.index.unwrap_or(0);
    let path = comp.layer(&name);

    // Every key time in this layer's properties is measured against the same
    // origin, so the offset is set once, here.
    conv.offset = conv.tb.seconds(ae.start_time.unwrap_or(0.0));
    let props = &ae.properties;

    let in_point = CompTime(conv.tb.seconds(ae.in_point.unwrap_or(0.0)));
    let mut out_point = CompTime(conv.tb.seconds(ae.out_point.unwrap_or(0.0)));
    if out_point <= in_point {
        // The model's one span invariant. A zero-length or reversed bar is a
        // damaged capture, not a layer worth dropping.
        let frame = conv
            .tb
            .rate()
            .frame_duration()
            .unwrap_or(lumit_core::time::Duration(Rational::ONE));
        out_point = in_point.add_dur(frame).unwrap_or(out_point);
        conv.report
            .row(path.clone(), Outcome::Adjusted, Reason::LayerSpanRepaired);
    }
    let start_offset = CompTime(conv.offset);

    // The layer's own span, before anything under it is mapped: an effect
    // parameter that reads After Effects' clock becomes keyframes across it
    // (docs/08 §3.53), so the effect table needs it in hand.
    let local_in = conv.offset_from(in_point.0);
    let local_out = conv.offset_from(out_point.0);
    conv.span = (local_in, local_out);

    let kind = layer_kind(conv, &path, ae, items, props);
    let masks = masks(conv, &path, props);
    // Masks before effects, because an effect parameter can name one (K-408).
    conv.masks = super::fx_colour::mask_refs(&masks);
    let effects = group(props, "ADBE Effect Parade")
        .iter()
        .map(|node| map_effect(conv, &path, node).instance())
        .collect();

    let (retime, interpolation) = retime(conv, &path, ae, props, local_in, local_out);

    let layer = Layer {
        id,
        name,
        kind,
        in_point,
        out_point,
        start_offset,
        transform: transform(conv, &path, props),
        matte: matte(conv, &path, ae, index, ids),
        parent: parent(conv, &path, ae, ids),
        label: u8::try_from(ae.label.unwrap_or(0)).unwrap_or(0),
        markers: markers(conv, &ae.markers),
        volume_db: LumProperty::zero(),
        retime,
        interpolation,
        parked_flow: None,
        blend: blend(conv, &path, ae.blend.as_deref()),
        masks,
        paint: Vec::new(),
        effects,
        switches: switches(conv, &path, ae),
        extra: ae_map(vec![
            ("index", serde_json::json!(index)),
            ("kind", serde_json::json!(ae.kind)),
            ("stretch", serde_json::json!(ae.stretch)),
            ("auto_orient", serde_json::json!(ae.auto_orient)),
            (
                "preserve_transparency",
                serde_json::json!(ae.preserve_transparency),
            ),
        ]),
    };
    conv.report.imported();
    Some(layer)
}

/// The layer's kind, resolved through its source item where it has one.
fn layer_kind(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    ae: &AeLayer,
    items: &Items,
    props: &[Property],
) -> LayerKind {
    let source = ae.source_id.and_then(|id| items.get(id));
    let kind = ae.kind.as_deref().unwrap_or("");

    // The source item decides between Footage and Precomp whatever the walker
    // called the layer: a precomp layer is a layer whose source is a comp.
    if let Some((uuid, item_kind)) = source {
        match item_kind {
            ItemKind::Comp => return LayerKind::Precomp { comp: uuid },
            ItemKind::Solid => return LayerKind::Solid { def: uuid },
            ItemKind::Footage => {
                if kind == "audio" {
                    conv.report
                        .row(path.clone(), Outcome::Adjusted, Reason::AudioLayerAsFootage);
                }
                return LayerKind::Footage { item: uuid };
            }
            ItemKind::Folder => {}
        }
    }

    match kind {
        "null" => LayerKind::Null,
        "adjustment" => LayerKind::Adjustment,
        "camera" => LayerKind::Camera {
            zoom: scalar(conv, path, props, "ADBE Camera Zoom", 0, 1000.0),
        },
        "light" => LayerKind::Light {
            light: Box::new(light(conv, path, ae, props)),
        },
        "shape" => {
            conv.report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::ShapeContentsNotMapped,
            );
            LayerKind::Shape {
                contents: Vec::new(),
            }
        }
        "text" => {
            conv.report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::TextStylingNotMapped,
            );
            LayerKind::Text {
                document: text(props),
            }
        }
        other => {
            // A layer whose source went missing, or a kind this build has
            // never heard of. Either way it keeps its slot as a Null, so
            // parenting and stacking order survive around it.
            if ae.source_id.is_some() {
                conv.report.row(
                    path.clone(),
                    Outcome::Adjusted,
                    Reason::LayerSourceMissing {
                        id: ae.source_id.unwrap_or_default(),
                    },
                );
            } else {
                conv.report.row(
                    path.clone(),
                    Outcome::Adjusted,
                    Reason::LayerKindUnsupported {
                        ae_kind: other.to_string(),
                    },
                );
            }
            LayerKind::Null
        }
    }
}

/// v1 text: the words, the size and the fill colour, which is what Lumit's
/// text layer has. Everything else in AE's text document is reported by the
/// caller and kept in the bundle.
fn text(props: &[Property]) -> TextDocument {
    let doc = child(props, "ADBE Text Properties")
        .and_then(|group| child(group.children(), "ADBE Text Document"))
        .and_then(|node| node.value.clone())
        .unwrap_or(serde_json::Value::Null);
    let fill = doc.get("fillColor").and_then(|c| c.as_array()).map(|c| {
        let ch = |i: usize| c.get(i).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
        lumit_core::model::LinearColour([
            srgb_to_linear(ch(0)),
            srgb_to_linear(ch(1)),
            srgb_to_linear(ch(2)),
            1.0,
        ])
    });
    TextDocument {
        text: doc
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string(),
        expression: None,
        size: doc
            .get("fontSize")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(72.0),
        fill: fill.unwrap_or(lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0])),
        extra: serde_json::Map::new(),
    }
}

/// A Light layer's own properties. AE's intensity is a percentage where 100 is
/// unity, and its cone angle is the *full* angle where Lumit's is the half —
/// the two conversions this function makes.
fn light(conv: &mut Conv<'_>, path: &ItemPath, ae: &AeLayer, props: &[Property]) -> LightDef {
    let kind = match ae.light_type.as_deref() {
        Some("SPOT") => LightKind::Spot,
        Some("POINT") | None => LightKind::Point,
        Some(other) => {
            // Parallel and Ambient have no counterpart: a parallel light is a
            // point light infinitely far away, an ambient one is not a place
            // at all. Both import as points and say so.
            conv.report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::LightKindApproximated {
                    ae_kind: other.to_string(),
                },
            );
            LightKind::Point
        }
    };
    let colour_axis = |axis: usize| {
        let raw = still(props, "ADBE Light Color", axis).unwrap_or(1.0);
        LumProperty::fixed(f64::from(srgb_to_linear(raw)))
    };
    let intensity = still(props, "ADBE Light Intensity", 0).unwrap_or(100.0) / 100.0;
    let cone = still(props, "ADBE Light Cone Angle", 0).unwrap_or(90.0) / 2.0;
    LightDef {
        kind,
        colour: [colour_axis(0), colour_axis(1), colour_axis(2)],
        intensity: LumProperty::fixed(intensity),
        half_size: [LumProperty::zero(), LumProperty::zero()],
        cone_deg: LumProperty::fixed(cone),
        falloff_px: LumProperty::zero(),
    }
}

/// The transform group, axis by axis. Lumit separates every dimension, so a
/// coupled AE property and a separated one arrive the same way.
fn transform(conv: &mut Conv<'_>, path: &ItemPath, props: &[Property]) -> TransformGroup {
    let props = match child(props, "ADBE Transform Group") {
        Some(group) => group.children(),
        None => return TransformGroup::default(),
    };
    TransformGroup {
        anchor_x: scalar(conv, path, props, "ADBE Anchor Point", 0, 0.0),
        anchor_y: scalar(conv, path, props, "ADBE Anchor Point", 1, 0.0),
        position_x: scalar(conv, path, props, "ADBE Position", 0, 0.0),
        position_y: scalar(conv, path, props, "ADBE Position", 1, 0.0),
        position_z: scalar(conv, path, props, "ADBE Position", 2, 0.0),
        scale_x: scalar(conv, path, props, "ADBE Scale", 0, 100.0),
        scale_y: scalar(conv, path, props, "ADBE Scale", 1, 100.0),
        rotation: scalar(conv, path, props, "ADBE Rotate Z", 0, 0.0),
        rotation_x: scalar(conv, path, props, "ADBE Rotate X", 0, 0.0),
        rotation_y: scalar(conv, path, props, "ADBE Rotate Y", 0, 0.0),
        opacity: scalar(conv, path, props, "ADBE Opacity", 0, 100.0),
        extra: serde_json::Map::new(),
    }
}

/// The layer's switches. Lumit has no draft/wireframe quality and no guide
/// flag, so both are reported rather than dropped in silence.
fn switches(conv: &mut Conv<'_>, path: &ItemPath, ae: &AeLayer) -> Switches {
    let s = ae.switches.clone().unwrap_or_default();
    if s.guide == Some(true) {
        conv.report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::GuideLayerNotSupported,
        );
    }
    if let Some(quality) = s.quality.as_deref().filter(|q| *q != "BEST") {
        conv.report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::LayerQualityIgnored {
                quality: quality.to_string(),
            },
        );
    }
    if ae.preserve_transparency == Some(true) {
        conv.report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::PreserveTransparencyNotSupported,
        );
    }
    Switches {
        visible: s.enabled.unwrap_or(true),
        audible: s.audio.unwrap_or(true),
        locked: s.lock.unwrap_or(false),
        three_d: s.three_d.unwrap_or(false),
        collapse: s.collapse.unwrap_or(false),
        fx: s.effects_active.unwrap_or(true),
        solo: s.solo.unwrap_or(false),
        motion_blur: s.motion_blur.unwrap_or(false),
        shy: s.shy.unwrap_or(false),
        accepts_lights: true,
    }
}

/// The blend mode, and the documented fallback where there is none
/// (docs/11 §4's three "mapped" blend rows).
fn blend(conv: &mut Conv<'_>, path: &ItemPath, ae: Option<&str>) -> BlendMode {
    let Some(ae) = ae else {
        return BlendMode::Normal;
    };
    // AE's "Classic" modes are the 4.x maths of their modern namesakes; docs/11
    // §4 imports them as the modern one and flags the row.
    if let Some(modern) = ae.strip_prefix("CLASSIC_") {
        let mode = standard(modern).unwrap_or(BlendMode::Normal);
        conv.report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::BlendModeClassic {
                ae_mode: ae.to_string(),
            },
        );
        return mode;
    }
    match standard(ae) {
        Some(mode) => mode,
        None => {
            conv.report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::BlendModeUnavailable {
                    ae_mode: ae.to_string(),
                },
            );
            BlendMode::Normal
        }
    }
}

/// The standard set, one for one (docs/11 §4's "lossless" blend row). AE's
/// Linear Dodge and Add are the same operator, and both land on Add.
fn standard(ae: &str) -> Option<BlendMode> {
    Some(match ae {
        "NORMAL" => BlendMode::Normal,
        "DARKEN" => BlendMode::Darken,
        "MULTIPLY" => BlendMode::Multiply,
        "COLOR_BURN" => BlendMode::ColourBurn,
        "LINEAR_BURN" => BlendMode::LinearBurn,
        "DARKER_COLOR" => BlendMode::DarkerColour,
        "ADD" | "LINEAR_DODGE" => BlendMode::Add,
        "LIGHTEN" => BlendMode::Lighten,
        "SCREEN" => BlendMode::Screen,
        "COLOR_DODGE" => BlendMode::ColourDodge,
        "LIGHTER_COLOR" => BlendMode::LighterColour,
        "OVERLAY" => BlendMode::Overlay,
        "SOFT_LIGHT" => BlendMode::SoftLight,
        "HARD_LIGHT" => BlendMode::HardLight,
        "LINEAR_LIGHT" => BlendMode::LinearLight,
        "VIVID_LIGHT" => BlendMode::VividLight,
        "PIN_LIGHT" => BlendMode::PinLight,
        "HARD_MIX" => BlendMode::HardMix,
        "DIFFERENCE" => BlendMode::Difference,
        "EXCLUSION" => BlendMode::Exclusion,
        "SUBTRACT" => BlendMode::Subtract,
        "DIVIDE" => BlendMode::Divide,
        "HUE" => BlendMode::Hue,
        "SATURATION" => BlendMode::Saturation,
        "COLOR" => BlendMode::Colour,
        "LUMINOSITY" => BlendMode::Luminosity,
        _ => return None,
    })
}

/// Both generations of matte, normalised (docs/11 §3).
fn matte(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    ae: &AeLayer,
    index: u32,
    ids: &BTreeMap<u32, Uuid>,
) -> Option<MatteRef> {
    let matte = ae.matte.as_ref()?;
    let (channel, inverted) = match matte.kind.as_deref() {
        Some("ALPHA") => (MatteChannel::Alpha, false),
        Some("ALPHA_INVERTED") => (MatteChannel::Alpha, true),
        Some("LUMA") => (MatteChannel::Luma, false),
        Some("LUMA_INVERTED") => (MatteChannel::Luma, true),
        // NO_TRACK_MATTE, and anything a later AE invents.
        _ => return None,
    };
    // The 23.0+ form names the layer; the legacy form means the one above,
    // which in AE's 1-based top-first stack is this index minus one.
    let target = matte.layer_index.unwrap_or(index.saturating_sub(1));
    let Some(layer) = ids.get(&target).copied() else {
        conv.report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::MatteTargetMissing { index: target },
        );
        return None;
    };
    Some(MatteRef {
        layer,
        channel,
        inverted,
        // A track matte in After Effects samples the matte layer's finished
        // picture, effects and masks included.
        source: LayerInputSource::EffectsAndMasks,
    })
}

fn parent(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    ae: &AeLayer,
    ids: &BTreeMap<u32, Uuid>,
) -> Option<Uuid> {
    let index = ae.parent_index?;
    match ids.get(&index).copied() {
        Some(id) => Some(id),
        None => {
            conv.report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::ParentMissing { index },
            );
            None
        }
    }
}

/// Comp and layer markers alike. AE's chapter text makes a chapter marker;
/// the comment is the label a person reads.
pub(crate) fn markers(conv: &mut Conv<'_>, ae: &[AeMarker]) -> Vec<Marker> {
    ae.iter()
        .map(|m| {
            let duration = m.duration.filter(|d| *d > 0.0).map(|d| conv.tb.seconds(d));
            Marker {
                id: Uuid::now_v7(),
                time: CompTime(conv.tb.seconds(m.t.unwrap_or(0.0))),
                duration,
                label: m.comment.clone().unwrap_or_default(),
                kind: if m.chapter.as_deref().is_some_and(|c| !c.is_empty()) {
                    MarkerKind::Chapter
                } else {
                    MarkerKind::User
                },
                extra: serde_json::Map::new(),
            }
        })
        .collect()
}

/// The Retime and the frame-interpolation policy — AE's "time remap", its
/// "time stretch", and its frame-blending switch, which are three separate
/// controls there and two fields here.
fn retime(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    ae: &AeLayer,
    props: &[Property],
    local_in: Rational,
    local_out: Rational,
) -> (Option<LumProperty>, Interpolation) {
    let interpolation = match ae
        .switches
        .as_ref()
        .and_then(|s| s.frame_blending.as_deref())
    {
        Some("FRAME_MIX") => Interpolation::Blend,
        Some("PIXEL_MOTION") => {
            conv.report
                .row(path.clone(), Outcome::Adjusted, Reason::FlowEngineDiffers);
            Interpolation::Flow(lumit_core::retime::FlowParams::default())
        }
        _ => Interpolation::Nearest,
    };

    // The remap is already a Retime: same graph, same maths, value copy. Its
    // hold keys are freezes without anything having to convert them.
    if let Some(node) = child(props, "ADBE Time Remapping") {
        if node.keyframes.as_deref().is_some_and(|k| !k.is_empty()) {
            let map = from_node(conv, path, node, 0, 0.0);
            return (Some(map), interpolation);
        }
    }

    // Otherwise a stretch, if there is one, is the map.
    let stretch = ae.stretch.unwrap_or(100.0);
    if !stretch.is_finite() || stretch == 0.0 || (stretch - 100.0).abs() < 1e-9 {
        return (None, interpolation);
    }
    let rate = 100.0 / stretch;
    let (from, to) = (local_in.to_f64(), local_out.to_f64());
    let (v_from, v_to) = if rate >= 0.0 {
        (from * rate, to * rate)
    } else {
        // Reversed: the same line reflected inside the layer's own span, so
        // the layer opens on the last frame of the stretch and walks back.
        (to * -rate, from * -rate)
    };
    conv.report.row(
        path.clone(),
        Outcome::Adjusted,
        Reason::StretchAsRetime { percent: stretch },
    );
    (Some(ramp(local_in, v_from, local_out, v_to)), interpolation)
}

/// Every mask on the layer, in AE's own order.
fn masks(conv: &mut Conv<'_>, path: &ItemPath, props: &[Property]) -> Vec<Mask> {
    group(props, "ADBE Mask Parade")
        .iter()
        .filter_map(|node| mask(conv, path, node))
        .collect()
}

fn mask(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<Mask> {
    let facts = node.mask.clone()?;
    let name = display_name(node, match_name_of(node)).to_string();
    let here = path.property(&name);
    let props = node.children();

    let mode = match facts.mode.as_deref() {
        Some("NONE") => MaskMode::None,
        Some("SUBTRACT") => MaskMode::Subtract,
        Some("INTERSECT") => MaskMode::Intersect,
        Some("DIFFERENCE") => MaskMode::Difference,
        Some(other @ ("LIGHTEN" | "DARKEN")) => {
            // docs/06 §2: not built. Add is the honest neighbour.
            conv.report.row(
                here.clone(),
                Outcome::Adjusted,
                Reason::MaskModeUnavailable {
                    ae_mode: other.to_string(),
                },
            );
            MaskMode::Add
        }
        _ => MaskMode::Add,
    };

    if facts.roto_bezier == Some(true) {
        conv.report.row(
            here.clone(),
            Outcome::Adjusted,
            Reason::MaskRotoBezierFlattened,
        );
    }

    // AE feathers separately in x and y; Lumit has one width.
    let (fx, fy) = (
        still(props, "ADBE Mask Feather", 0).unwrap_or(0.0),
        still(props, "ADBE Mask Feather", 1).unwrap_or(0.0),
    );
    if (fx - fy).abs() > 1e-9 {
        conv.report.row(
            here.clone(),
            Outcome::Adjusted,
            Reason::MaskFeatherAxesDiffer { x: fx, y: fy },
        );
    }

    let shape = child(props, "ADBE Mask Shape");
    let path_keys: Vec<PathKeyframe> = shape
        .and_then(|node| node.keyframes.as_deref())
        .map(|keys| {
            keys.iter()
                .filter_map(|key| {
                    let shape: Shape = serde_json::from_value(key.v.clone()?).ok()?;
                    Some(PathKeyframe {
                        time: conv.layer_time(key.t?),
                        path: bezier(&shape),
                        interp_in: super::props::path_side(key.in_interp.as_deref()),
                        interp_out: super::props::path_side(key.out_interp.as_deref()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let still_path = shape
        .and_then(crate::capture::Property::shape)
        .as_ref()
        .map(bezier)
        .or_else(|| path_keys.first().map(|k| k.path.clone()))
        .unwrap_or(BezierPath {
            vertices: Vec::new(),
            closed: true,
        });

    conv.report.imported();
    Some(Mask {
        id: Uuid::now_v7(),
        name,
        path: still_path,
        path_keys,
        inverted: facts.inverted.unwrap_or(false),
        opacity: scalar(conv, &here, props, "ADBE Mask Opacity", 0, 100.0),
        mode,
        feather: LumProperty::fixed((fx + fy) / 2.0),
        expansion: scalar(conv, &here, props, "ADBE Mask Offset", 0, 0.0),
        extra: ae_map(vec![
            ("mode", serde_json::json!(facts.mode)),
            ("feather", serde_json::json!([fx, fy])),
            ("colour", serde_json::json!(facts.colour)),
        ]),
    })
}

/// A capture path as Lumit's own — parallel arrays become one vertex each.
fn bezier(shape: &Shape) -> BezierPath {
    let point = |list: &[Vec<f64>], i: usize| {
        list.get(i).map_or((0.0, 0.0), |p| {
            (
                p.first().copied().unwrap_or(0.0),
                p.get(1).copied().unwrap_or(0.0),
            )
        })
    };
    BezierPath {
        vertices: (0..shape.vertices.len())
            .map(|i| Vertex {
                pos: point(&shape.vertices, i),
                tan_in: point(&shape.in_tangents, i),
                tan_out: point(&shape.out_tangents, i),
            })
            .collect(),
        closed: shape.closed.unwrap_or(true),
    }
}
