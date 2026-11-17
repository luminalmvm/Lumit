//! A Lottie's comps and layers as Lumit's: kinds, spans, transforms,
//! parenting, masks, mattes, blend modes, markers and precomps.
//!
//! # In plain terms
//!
//! A Lottie layer is a Lumit layer almost field for field. Its times are in
//! frames of the comp it sits in: `ip`, `op` and its keys alike. `st` is
//! where the layer's own clock starts, which is Lumit's start offset, so the
//! keys are moved back by it onto the clock Lumit keeps them on. A precomp's
//! `tm` is
//! Lumit's Retime, the same map from the layer's time to the precomp's. A
//! matte names its layer by `tp`, or is the layer directly above, and the
//! matte layer itself (`td`) is not drawn, so its eye is turned off.
//!
//! Two kinds measure from a different place. A Lottie shape layer's art is
//! measured from the layer's own zero, where Lumit measures it from the
//! corner of the box round the art, and a text layer from its first
//! baseline. So the anchor, the masks and the children of those layers move
//! by the difference, and everything stays where the Lottie drew it.
//!
//! Runs on the import's thread.

use std::collections::BTreeMap;

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::edits::base_layer;
use lumit_core::markers::Marker;
use lumit_core::mask::{Mask, MaskMode};
use lumit_core::model::{
    BlendMode, Composition, Layer, LayerInputSource, LayerKind, LinearColour, MatteChannel,
    MatteRef, MotionBlur, ProjectItem, SolidDef, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, Rational};
use lumit_import::report::{ItemPath, Outcome, Reason};
use serde_json::Value;
use uuid::Uuid;

use super::read::{self, num, on, text};
use super::shapes::Split;
use super::{assets, extras, keys, shapes, Cx, MAX_DEPTH};
use crate::LottieError;

/// One Lottie layer as it lands: the layer, and the layers a moving group of
/// its art became, which go directly above it.
struct Built {
    layer: Layer,
    above: Vec<Layer>,
    /// The layer's own zero within Lumit's, for a shape or a text layer.
    origin: Option<(f64, f64)>,
}

/// A comp made from a list of Lottie layers, `span` its first and last
/// frames.
pub(super) fn comp(
    cx: &mut Cx<'_>,
    name: &str,
    size: (u32, u32),
    layers: &[Value],
    span: (f64, f64),
) -> Result<Composition, LottieError> {
    cx.budget.take_items(layers.len() as u64)?;
    let place = ItemPath::item(name);
    let ids: Vec<Uuid> = layers.iter().map(|_| Uuid::now_v7()).collect();
    let by_ind: BTreeMap<i64, Uuid> = layers
        .iter()
        .zip(&ids)
        .filter_map(|(l, id)| Some((num(l.get("ind"))? as i64, *id)))
        .collect();

    let mut built = Vec::with_capacity(layers.len());
    for (n, (v, id)) in layers.iter().zip(&ids).enumerate() {
        built.push(layer(cx, &place, v, n, *id, &by_ind)?);
    }

    // Mattes: the layer `tp` names, or the one directly above.
    for n in 0..built.len() {
        let Some(v) = layers.get(n) else { continue };
        let mode = num(v.get("tt")).unwrap_or(0.0) as i64;
        if !(1..=4).contains(&mode) {
            continue;
        }
        let source = match num(v.get("tp")) {
            Some(tp) => by_ind.get(&(tp as i64)).copied(),
            None => n.checked_sub(1).and_then(|above| ids.get(above).copied()),
        };
        let Some(b) = built.get_mut(n) else { continue };
        match source.filter(|s| *s != b.layer.id) {
            Some(layer) => {
                b.layer.matte = Some(MatteRef {
                    layer,
                    channel: if mode >= 3 {
                        MatteChannel::Luma
                    } else {
                        MatteChannel::Alpha
                    },
                    inverted: mode == 2 || mode == 4,
                    source: LayerInputSource::default(),
                });
            }
            None => {
                let index = num(v.get("tp")).unwrap_or(0.0).max(0.0) as u32;
                let path = place.layer(&b.layer.name);
                cx.row(
                    path,
                    Outcome::Adjusted,
                    Reason::MatteTargetMissing { index },
                );
            }
        }
    }

    // A shape or text layer's children sit in its space, whose zero moved.
    let origins: BTreeMap<Uuid, (f64, f64)> = built
        .iter()
        .filter_map(|b| Some((b.layer.id, b.origin?)))
        .collect();
    let mut out = Vec::with_capacity(built.len());
    for b in built {
        for mut layer in b.above.into_iter().chain(std::iter::once(b.layer)) {
            if let Some(&(x, y)) = layer.parent.and_then(|p| origins.get(&p)) {
                shift(&mut layer.transform.position_x, x);
                shift(&mut layer.transform.position_y, y);
            }
            out.push(layer);
        }
    }

    let fps = cx.clock.fps;
    let end = if span.1 > span.0 {
        span.1
    } else {
        span.0 + fps
    };
    let duration = cx.clock.time(end);
    let duration = if duration > Rational::ZERO {
        duration
    } else {
        cx.clock.time(fps)
    };
    let work_area = (span.0 > 0.0 && span.0 < end)
        .then(|| (CompTime(cx.clock.time(span.0)), CompTime(duration)));
    Ok(Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.to_owned(),
        width: size.0,
        height: size.1,
        frame_rate: cx.clock.rate,
        duration: Duration(duration),
        background: LinearColour::BLACK,
        work_area,
        layers: out,
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        extra: serde_json::Map::new(),
    })
}

/// One layer, and what its moving groups became.
fn layer(
    cx: &mut Cx<'_>,
    comp: &ItemPath,
    v: &Value,
    n: usize,
    id: Uuid,
    by_ind: &BTreeMap<i64, Uuid>,
) -> Result<Built, LottieError> {
    let name = text(v, "nm")
        .filter(|n| !n.trim().is_empty())
        .map_or_else(|| format!("Layer {}", n + 1), str::to_owned);
    let path = comp.layer(&name);
    cx.layers = cx.layers.saturating_add(1);

    let clock = cx.clock;
    let mut in_point = clock.time(num(v.get("ip")).unwrap_or(0.0));
    let mut out_point = clock.time(num(v.get("op")).unwrap_or(0.0));
    if out_point < in_point {
        std::mem::swap(&mut in_point, &mut out_point);
    }
    if out_point <= in_point {
        out_point = clock.time(num(v.get("ip")).unwrap_or(0.0) + 1.0);
        cx.row(path.clone(), Outcome::Adjusted, Reason::LayerSpanRepaired);
    }
    let st = num(v.get("st")).unwrap_or(0.0);
    let start = clock.time(st);
    cx.offset = st;

    let mut origin = None;
    let mut splits = Vec::new();
    let ref_id = v.get("refId").and_then(super::id_text).unwrap_or_default();
    let kind = match num(v.get("ty")).map(|t| t as i64) {
        Some(0) => {
            let size = (super::dimension(v.get("w")), super::dimension(v.get("h")));
            match precomp(cx, &ref_id, size)? {
                Some(comp) => LayerKind::Precomp { comp },
                None => {
                    cx.row(path.clone(), Outcome::Skipped, Reason::CompMissing);
                    LayerKind::Null
                }
            }
        }
        Some(1) => {
            let colour = text(v, "sc").and_then(read::hex).unwrap_or([0.0; 3]);
            let def = Uuid::now_v7();
            cx.made.push(ProjectItem::Solid(SolidDef {
                id: def,
                name: name.clone(),
                colour: linear(colour),
                width: super::dimension(v.get("sw")),
                height: super::dimension(v.get("sh")),
                extra: serde_json::Map::new(),
            }));
            LayerKind::Solid { def }
        }
        Some(2) => match assets::image(cx, &path, &ref_id)? {
            Some(item) => LayerKind::Footage { item },
            None => {
                cx.row(path.clone(), Outcome::Adjusted, Reason::MediaNotFound);
                LayerKind::Null
            }
        },
        Some(3) => LayerKind::Null,
        Some(4) => {
            let (contents, split) = shapes::contents(cx, &path, read::list(v, "shapes"))?;
            splits = split;
            origin = lumit_core::shape::contents_bounds(&contents, 0.0).map(|(x, y, ..)| (-x, -y));
            LayerKind::Shape { contents }
        }
        Some(5) => {
            let (document, at) = extras::text_layer(cx, &path, v.get("t").unwrap_or(&Value::Null))?;
            origin = Some(at);
            LayerKind::Text { document }
        }
        other => {
            let ae_kind = match other {
                Some(6) => String::from("Audio"),
                Some(13) => String::from("Camera"),
                Some(15) => String::from("Data"),
                Some(t) => format!("Layer type {t}"),
                None => String::from("A layer with no type"),
            };
            cx.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::LayerKindUnsupported { ae_kind },
            );
            LayerKind::Null
        }
    };

    let mut transform = transform(cx, &path, v.get("ks"))?;
    if on(v.get("ao")) {
        transform.orient = lumit_core::orient::Orient::AlongPath;
    }
    let mut masks = masks(cx, &path, v)?;
    if let Some((x, y)) = origin {
        shift(&mut transform.anchor_x, x);
        shift(&mut transform.anchor_y, y);
        for mask in &mut masks {
            let keyed = mask.path_keys.iter_mut().map(|k| &mut k.path);
            for vertex in std::iter::once(&mut mask.path)
                .chain(keyed)
                .flat_map(|p| &mut p.vertices)
            {
                vertex.pos.0 += x;
                vertex.pos.1 += y;
            }
        }
    }
    let effects = extras::effects(cx, &path, v)?;
    if !read::list(v, "sy").is_empty() {
        cx.left_out(path.property("Layer styles"), "Layer styles");
    }
    if on(v.get("ddd")) {
        cx.approximated(path.clone(), "3D layer");
    }

    let mut layer = base_layer(name.clone(), kind, out_point, transform);
    layer.id = id;
    layer.in_point = CompTime(in_point);
    layer.start_offset = CompTime(start);
    layer.masks = masks;
    layer.effects = effects;
    layer.blend = blend(num(v.get("bm")).unwrap_or(0.0) as i64);
    layer.switches.visible = !on(v.get("hd")) && !on(v.get("td"));
    if let Some(parent) = num(v.get("parent")) {
        match by_ind.get(&(parent as i64)).filter(|p| **p != id) {
            Some(p) => layer.parent = Some(*p),
            None => cx.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::ParentMissing {
                    index: parent.max(0.0) as u32,
                },
            ),
        }
    }
    if matches!(layer.kind, LayerKind::Precomp { .. }) {
        let local = (
            in_point.checked_sub(start).unwrap_or(in_point),
            out_point.checked_sub(start).unwrap_or(out_point),
        );
        layer.retime = retime(cx, &path, v, local)?;
    }

    // A group that moves of its own became layers of its own; anything the
    // layer does to its whole picture no longer reaches them.
    let whole = !matches!(layer.transform.opacity.animation, Animation::Static(o) if o >= 100.0)
        || !layer.masks.is_empty()
        || num(v.get("tt")).unwrap_or(0.0) > 0.0
        || layer.blend != BlendMode::Normal
        || !layer.effects.is_empty();
    let mut above = Vec::new();
    for split in splits {
        if whole {
            cx.approximated(path.property(&split.name), &split.name);
        }
        place_split(split, &layer, layer.id, &mut above);
    }
    Ok(Built {
        layer,
        above,
        origin,
    })
}

/// A moving group's layers: a Null for each group it sits in, its own last,
/// and the Shape layer of its art parented to that, with what moves inside
/// it parented to the Shape layer in turn.
fn place_split(split: Split, owner: &Layer, parent: Uuid, out: &mut Vec<Layer>) {
    let mut parent = parent;
    let mut nulls = Vec::with_capacity(split.nulls.len());
    for (n, transform) in split.nulls.into_iter().enumerate() {
        let mut null = base_layer(
            if n == 0 {
                split.name.clone()
            } else {
                format!("{} {}", split.name, n + 1)
            },
            LayerKind::Null,
            owner.out_point.0,
            transform,
        );
        null.in_point = owner.in_point;
        null.start_offset = owner.start_offset;
        null.parent = Some(parent);
        parent = null.id;
        nulls.push(null);
    }
    let origin =
        lumit_core::shape::contents_bounds(&split.contents, 0.0).map(|(x, y, ..)| (-x, -y));
    let mut transform = TransformGroup {
        opacity: split.opacity,
        ..TransformGroup::default()
    };
    if let Some((x, y)) = origin {
        transform.anchor_x = Property::fixed(x);
        transform.anchor_y = Property::fixed(y);
    }
    let mut art = base_layer(
        split.name.clone(),
        LayerKind::Shape {
            contents: split.contents,
        },
        owner.out_point.0,
        transform,
    );
    art.in_point = owner.in_point;
    art.start_offset = owner.start_offset;
    art.parent = Some(parent);
    let art_id = art.id;
    let mut inner = Vec::new();
    for child in split.inner {
        place_split(child, &art, art_id, &mut inner);
    }
    // The children sit in the Shape layer's space, whose zero moved.
    if let Some((x, y)) = origin {
        for layer in inner.iter_mut().filter(|l| l.parent == Some(art_id)) {
            shift(&mut layer.transform.position_x, x);
            shift(&mut layer.transform.position_y, y);
        }
    }
    out.extend(inner);
    out.push(art);
    out.extend(nulls);
}

/// The comp for precomp asset `id`, made the first time a layer places it.
/// `None` when there is no such asset, or when it places itself.
fn precomp(cx: &mut Cx<'_>, id: &str, size: (u32, u32)) -> Result<Option<Uuid>, LottieError> {
    if let Some(made) = cx.comps.get(id) {
        return Ok(*made);
    }
    let Some(asset) = cx.assets.get(id).copied() else {
        return Ok(None);
    };
    let Some(layers) = asset.get("layers").and_then(Value::as_array) else {
        return Ok(None);
    };
    if cx.depth >= MAX_DEPTH {
        return Err(lumit_ingress::IngressError::Depth { limit: MAX_DEPTH }.into());
    }
    let end = layers
        .iter()
        .filter_map(|l| num(l.get("op")))
        .fold(0.0, f64::max);
    let name = text(asset, "nm")
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(id)
        .to_owned();
    cx.comps.insert(id.to_owned(), None);
    // The precomp's layers set their own starts; the placing layer's comes
    // back for the rest of its properties.
    let offset = cx.offset;
    cx.depth += 1;
    let made = comp(cx, &name, size, layers, (0.0, end));
    cx.depth -= 1;
    cx.offset = offset;
    let made = made?;
    let comp_id = made.id;
    cx.comps.insert(id.to_owned(), Some(comp_id));
    cx.made.push(ProjectItem::Composition(made));
    Ok(Some(comp_id))
}

/// A precomp layer's Retime: its `tm`, or the straight map a stretch (`sr`)
/// makes, across the layer's own span `local`.
fn retime(
    cx: &mut Cx<'_>,
    path: &ItemPath,
    v: &Value,
    local: (Rational, Rational),
) -> Result<Option<Property>, LottieError> {
    let stretch = num(v.get("sr")).filter(|s| *s > 0.0).unwrap_or(1.0);
    let stretched = (stretch - 1.0).abs() > 1e-9;
    if let Some(tm) = cx.read(path, "Retime", v.get("tm"))? {
        if stretched {
            cx.approximated(path.property("Time stretch"), "Time stretch");
        }
        return Ok(Some(keys::property(&tm.track, 0, &cx.clock, 0.0)));
    }
    if !stretched {
        return Ok(None);
    }
    cx.row(
        path.clone(),
        Outcome::Adjusted,
        Reason::StretchAsRetime {
            percent: stretch * 100.0,
        },
    );
    let key = |time: Rational| Keyframe {
        time,
        value: time.to_f64() / stretch,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    Ok(Some(Property {
        animation: Animation::Keyframed(vec![key(local.0), key(local.1)]),
        extra: serde_json::Map::new(),
    }))
}

/// A transform: the layer's `ks`, or a shape group's `tr`.
pub(super) fn transform(
    cx: &mut Cx<'_>,
    path: &ItemPath,
    ks: Option<&Value>,
) -> Result<TransformGroup, LottieError> {
    let mut t = TransformGroup::default();
    let Some(ks) = ks else {
        return Ok(t);
    };
    let clock = cx.clock;
    let anchor = cx.read(path, "Anchor point", ks.get("a"))?;
    [t.anchor_x, t.anchor_y] = keys::pair(anchor.as_ref(), &clock, [0.0, 0.0]);
    let p = ks.get("p");
    if p.is_some_and(|p| on(p.get("s"))) {
        t.position_x = cx.one(path, "Position", p.and_then(|p| p.get("x")), 0.0)?;
        t.position_y = cx.one(path, "Position", p.and_then(|p| p.get("y")), 0.0)?;
    } else {
        let position = cx.read(path, "Position", p)?;
        [t.position_x, t.position_y] = keys::pair(position.as_ref(), &clock, [0.0, 0.0]);
    }
    let scale = cx.read(path, "Scale", ks.get("s"))?;
    t.scale_x = keys::one(scale.as_ref(), 0, &clock, 100.0);
    t.scale_y = keys::one(scale.as_ref(), 1, &clock, 100.0);
    t.rotation = cx.one(path, "Rotation", ks.get("r").or_else(|| ks.get("rz")), 0.0)?;
    t.opacity = cx.one(path, "Opacity", ks.get("o"), 100.0)?;
    if let Some(skew) = cx.read(path, "Skew", ks.get("sk"))? {
        if skew.is_animated() || skew.first().first().is_some_and(|s| s.abs() > 1e-9) {
            cx.left_out(path.property("Skew"), "Skew");
        }
    }
    Ok(t)
}

/// The layer's masks, read only when the layer says it has them, as
/// lottie-web reads them.
fn masks(cx: &mut Cx<'_>, path: &ItemPath, v: &Value) -> Result<Vec<Mask>, LottieError> {
    if !on(v.get("hasMask")) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (n, m) in read::list(v, "masksProperties").iter().enumerate() {
        let name = text(m, "nm")
            .filter(|n| !n.trim().is_empty())
            .map_or_else(|| format!("Mask {}", n + 1), str::to_owned);
        let here = path.property(&name);
        let mode = match text(m, "mode") {
            Some("n") => MaskMode::None,
            Some("s") => MaskMode::Subtract,
            Some("i") => MaskMode::Intersect,
            Some("l") => MaskMode::Lighten,
            Some("d") => MaskMode::Darken,
            Some("f") => MaskMode::Difference,
            _ => MaskMode::Add,
        };
        let (still, path_keys) = cx.path(m.get("pt"), m.get("cl").and_then(Value::as_bool))?;
        out.push(Mask {
            id: Uuid::now_v7(),
            name,
            path: still,
            path_keys,
            inverted: on(m.get("inv")),
            opacity: cx.one(&here, "Opacity", m.get("o"), 100.0)?,
            mode,
            feather: Property::zero(),
            vertex_feather: Vec::new(),
            expansion: cx.one(&here, "Expansion", m.get("x"), 0.0)?,
            extra: serde_json::Map::new(),
        });
    }
    Ok(out)
}

/// The file's markers: a name, a time and a length, in frames.
pub(super) fn markers(cx: &Cx<'_>, list: &[Value]) -> Vec<Marker> {
    list.iter()
        .filter_map(|m| {
            let mut marker = Marker::user(Uuid::now_v7(), cx.clock.time(num(m.get("tm"))?));
            marker.label = text(m, "cm").unwrap_or_default().to_owned();
            marker.duration = num(m.get("dr"))
                .filter(|d| *d > 0.0)
                .map(|d| cx.clock.time(d));
            Some(marker)
        })
        .collect()
}

/// A blend mode by its Lottie number.
fn blend(code: i64) -> BlendMode {
    match code {
        1 => BlendMode::Multiply,
        2 => BlendMode::Screen,
        3 => BlendMode::Overlay,
        4 => BlendMode::Darken,
        5 => BlendMode::Lighten,
        6 => BlendMode::ColourDodge,
        7 => BlendMode::ColourBurn,
        8 => BlendMode::HardLight,
        9 => BlendMode::SoftLight,
        10 => BlendMode::Difference,
        11 => BlendMode::Exclusion,
        12 => BlendMode::Hue,
        13 => BlendMode::Saturation,
        14 => BlendMode::Colour,
        15 => BlendMode::Luminosity,
        16 => BlendMode::Add,
        17 => BlendMode::HardMix,
        _ => BlendMode::Normal,
    }
}

/// A display colour as Lumit's linear light, opaque.
pub(super) fn linear(c: [f64; 3]) -> LinearColour {
    let [r, g, b] = c.map(|v| crate::colour::srgb_decode(v) as f32);
    LinearColour([r, g, b, 1.0])
}

/// Every value of a property moved by `by`, keys and all.
pub(super) fn shift(p: &mut Property, by: f64) {
    match &mut p.animation {
        Animation::Static(v) => *v += by,
        Animation::Keyframed(keys) => keys.iter_mut().for_each(|k| k.value += by),
        Animation::Expression(_) => {}
    }
}
