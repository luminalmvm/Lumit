//! One layer as a Lottie layer: its timing, transform, masks, matte, blend
//! mode and effects around its own content.
//!
//! Runs on the export's thread.

use lumit_core::anim::{Animation, Property};
use lumit_core::model::{art_shift, placement, BlendMode, Composition, Layer, LayerKind};
use lumit_core::orient::{flat_turn, Orient};
use serde_json::{json, Map, Value};

use super::value::{moved, Class, Clock};
use super::{assets, effects, shapes, text, Ctx};
use crate::check::{arg, Feature, Place, Report};
use crate::fit::{self, tolerance};
use crate::keys::Track;
use crate::settings::TextMode;
use crate::time::Timebase;
use crate::walk::{Out, Role};
use crate::LottieError;

/// Lottie layer kinds.
const PRECOMP: u8 = 0;
const SOLID: u8 = 1;
const IMAGE: u8 = 2;
const NULL: u8 = 3;
const SHAPE: u8 = 4;

/// `out` as a Lottie layer, in a comp whose clock is `tb` and whose layers
/// go out as `plan`.
pub fn layer(
    cx: &mut Ctx,
    comp: &Composition,
    tb: &Timebase,
    out: &Out,
    plan: &[Out],
) -> Result<Value, LottieError> {
    let l = out.layer;
    let place = &out.place;
    let clock = Clock::new(cx.doc, comp, l, tb, cx.host, cx.fmt);
    let mut m = Map::new();

    // The content first: it decides the kind, and how far the layer's own
    // pixels sit from the coordinates the content is written in.
    let (masks_on, effects_on) = match out.role {
        Role::Drawn => (true, true),
        Role::ParentOnly => (false, false),
        Role::MatteCopy { masks, effects } => (masks, effects),
    };
    // Kept for a hidden parent too: its children follow the whole transform.
    let shift = shift(cx, &clock, l);
    let mut ty = NULL;
    let mut hidden = matches!(out.role, Role::ParentOnly);
    if !hidden {
        match &l.kind {
            LayerKind::Shape { contents } => {
                ty = SHAPE;
                let shapes = shapes::contents(&clock, contents, &mut cx.report, place);
                m.insert("shapes".into(), Value::Array(shapes));
            }
            LayerKind::Text { document } => match cx.settings.text {
                TextMode::Shapes => {
                    ty = SHAPE;
                    let shapes = text::shapes(&clock, document, &mut cx.report, place);
                    m.insert("shapes".into(), Value::Array(shapes));
                }
                TextMode::Text => {
                    ty = 5;
                    let t = text::document(&clock, document, &mut cx.fonts, &mut cx.report, place);
                    m.insert("t".into(), t);
                }
            },
            LayerKind::Solid { def } => match cx.doc.solid(*def) {
                Some(s) => {
                    ty = SOLID;
                    let c = s.colour.0.map(f64::from);
                    let shown = cx.host.display([c[0], c[1], c[2]]);
                    m.insert("sc".into(), Value::from(crate::colour::hex(shown)));
                    m.insert("sw".into(), Value::from(s.width));
                    m.insert("sh".into(), Value::from(s.height));
                }
                None => hidden = true,
            },
            LayerKind::Footage { item } => match assets::image(cx, *item, place) {
                Some(id) => {
                    ty = IMAGE;
                    m.insert("refId".into(), Value::from(id));
                }
                None => hidden = true,
            },
            LayerKind::Precomp { comp: inner } => match assets::precomp(cx, *inner, place)? {
                Some(id) => {
                    ty = PRECOMP;
                    m.insert("refId".into(), Value::from(id));
                    if let Some(c) = cx.doc.comp(*inner) {
                        m.insert("w".into(), Value::from(c.width));
                        m.insert("h".into(), Value::from(c.height));
                    }
                    if let (Some(retime), Some(c)) = (&l.retime, cx.doc.comp(*inner)) {
                        m.insert("tm".into(), retimed(&clock, retime, c));
                    }
                }
                None => hidden = true,
            },
            _ => {}
        }
    }

    m.insert("ty".into(), Value::from(ty));
    cx.fmt.name(&mut m, &l.name);
    m.insert("ind".into(), Value::from(out.ind));
    if let Some(parent) = out.parent {
        m.insert("parent".into(), Value::from(parent));
    }
    m.insert("ddd".into(), Value::from(0));
    let (ip, op) = if matches!(out.role, Role::ParentOnly) {
        (tb.start, tb.end)
    } else {
        (l.in_point.0.to_f64(), l.out_point.0.to_f64())
    };
    m.insert("ip".into(), cx.fmt.num(tb.frame(ip), Class::Time));
    m.insert("op".into(), cx.fmt.num(tb.frame(op), Class::Time));
    m.insert(
        "st".into(),
        cx.fmt.num(tb.frame(clock.offset()), Class::Time),
    );
    m.insert("sr".into(), Value::from(1));
    // The transform is read wherever a child needs it, not only while the
    // layer itself shows.
    let mut moving = Clock::new(cx.doc, comp, l, tb, cx.host, cx.fmt);
    moving.span = out.reach;
    let lift = lift(cx, comp, tb, l);
    m.insert("ks".into(), transform(&moving, l, shift, lift));
    clock.baked.set(clock.baked.get() || moving.baked.get());
    clock.split.set(clock.split.get() || moving.split.get());
    if hidden {
        m.insert("hd".into(), Value::from(true));
    }
    if matches!(out.role, Role::MatteCopy { .. }) {
        m.insert("td".into(), Value::from(1));
    }
    if let Some(matte) = out.matte {
        let tt = match (matte.luma, matte.inverted) {
            (false, false) => 1,
            (false, true) => 2,
            (true, false) => 3,
            (true, true) => 4,
        };
        m.insert("tt".into(), Value::from(tt));
        m.insert("tp".into(), Value::from(matte.ind));
        if matte.luma {
            cx.report.players(
                place,
                Feature::LumaMatte,
                "luma_matte",
                "Luma matte".into(),
                Vec::new(),
            );
        }
    }

    if !hidden {
        m.insert(
            "bm".into(),
            Value::from(blend(l.blend, &mut cx.report, place)),
        );
        if masks_on && !l.masks.is_empty() {
            let masks = masks(&clock, l, shift, &mut cx.report, place);
            m.insert("hasMask".into(), Value::from(true));
            m.insert("masksProperties".into(), Value::Array(masks));
        }
        if effects_on {
            let drawn = |id: uuid::Uuid| {
                plan.iter()
                    .find(|o| o.layer.id == id && matches!(o.role, Role::Drawn))
                    .map(|o| o.ind)
            };
            let matted = out.matte.is_some();
            let ef = effects::effects(&clock, l, shift, &drawn, matted, &mut cx.report, place);
            if !ef.is_empty() {
                m.insert("ef".into(), Value::Array(ef));
            }
        }
        if matches!(out.role, Role::Drawn) {
            left_out(cx, comp, l, place);
        }
    }
    if clock.baked.get() {
        cx.report.changed(
            place,
            "expression_baked",
            "Expressions are baked into keys".into(),
            Vec::new(),
        );
    }
    if clock.split.get() {
        cx.report.players(
            place,
            Feature::SplitEasing,
            "split_easing",
            "Separate easing for each number".into(),
            Vec::new(),
        );
    }
    Ok(Value::Object(m))
}

/// What a drawn layer carries that no file can: said once per layer.
fn left_out(cx: &mut Ctx, comp: &Composition, l: &Layer, place: &Place) {
    let r = &mut cx.report;
    if l.switches.three_d {
        r.changed(place, "three_d_flat", "Drawn flat".into(), Vec::new());
    }
    if l.switches.motion_blur && comp.motion_blur.enabled {
        r.left_out(
            place,
            "motion_blur",
            "Motion blur is left out".into(),
            Vec::new(),
        );
    }
    if !l.paint.is_empty() {
        r.left_out(
            place,
            "paint",
            "Paint strokes are left out".into(),
            Vec::new(),
        );
    }
    if l.puppet.is_some() {
        r.left_out(
            place,
            "puppet",
            "Puppet pins are left out".into(),
            Vec::new(),
        );
    }
    for style in l.styles.iter().filter(|s| s.enabled) {
        let label = effects::label(style);
        r.left_out(
            &place.part(&label),
            "style",
            format!("{label} is left out"),
            vec![arg("label", label.clone())],
        );
    }
}

/// Where `l`'s own pixel (0, 0) sits in the coordinates its content is
/// written in, at the head of the layer: a Shape layer's art box corner, a
/// Lottie text layer's origin turned round.
fn shift(cx: &Ctx, c: &Clock, l: &Layer) -> (f64, f64) {
    match &l.kind {
        LayerKind::Shape { contents } => lumit_core::shape::contents_bounds(contents, 0.0)
            .map_or((0.0, 0.0), |(x0, y0, ..)| (x0, y0)),
        LayerKind::Text { document } if cx.settings.text == TextMode::Text => {
            let (x, y) = text::origin(c, document);
            (-x, -y)
        }
        _ => (0.0, 0.0),
    }
}

/// `l`'s parent with its [`shift`]. A child's position is in its parent's
/// pixels, which a player reads in the parent's content coordinates.
fn lift<'a>(cx: &Ctx, comp: &'a Composition, tb: &Timebase, l: &Layer) -> Lift<'a> {
    let parent = comp.layers.iter().find(|p| Some(p.id) == l.parent)?;
    let c = Clock::new(cx.doc, comp, parent, tb, cx.host, cx.fmt);
    Some((parent, shift(cx, &c, parent)))
}

type Lift<'a> = Option<(&'a Layer, (f64, f64))>;

/// The layer's transform. `shift` is added to the anchor: where the
/// content's own coordinates put the layer's pixel corner (a Shape layer's
/// art box, a text layer's baseline). `lift` is added to the position.
fn transform(c: &Clock, l: &Layer, shift: (f64, f64), lift: Lift) -> Value {
    let tr = &l.transform;
    // A camera or a light has no anchor or scale, as Lumit places it.
    let eye = matches!(l.kind, LayerKind::Camera { .. } | LayerKind::Light { .. });
    let anchor = if eye {
        Track::still(vec![0.0, 0.0])
    } else {
        c.track(&[&tr.anchor_x, &tr.anchor_y], &[tolerance::PIXELS; 2])
            .map_affine(|d, v| v + if d == 0 { shift.0 } else { shift.1 })
    };
    let scale = if eye {
        Track::still(vec![100.0, 100.0])
    } else {
        c.track(&[&tr.scale_x, &tr.scale_y], &[tolerance::PERCENT; 2])
    };

    // The parent's pixel corner moves with its art.
    let offset = c.offset();
    let base = lift.map_or((0.0, 0.0), |(_, s)| s);
    let lifted = |lt: f64| match lift {
        Some((p, s)) => {
            let (dx, dy) = art_shift(p, lt + offset - p.start_offset.0.to_f64());
            (s.0 + dx, s.1 + dy)
        }
        None => base,
    };
    let lift_moves = lift.is_some()
        && c.times()
            .into_iter()
            .take_while(|_| !c.host.cancelled())
            .any(|lt| lifted(lt) != base);
    let (px, py) = (&tr.position_x, &tr.position_y);
    let position = if lift_moves {
        let axis = |p: &Property, d: usize| {
            if matches!(p.animation, Animation::Expression(_)) {
                c.baked.set(true);
            }
            let track = c.sampled(
                |lt| {
                    let by = lifted(lt);
                    vec![c.value(p, lt) + if d == 0 { by.0 } else { by.1 }]
                },
                &[tolerance::PIXELS],
            );
            c.write(&track, Class::Px, false, &[])
        };
        json!({"s": true, "x": axis(px, 0), "y": axis(py, 1)})
    } else if Clock::moves(px) || Clock::moves(py) {
        let axis = |p: &Property, by: f64| {
            let track = c
                .track(&[p], &[tolerance::PIXELS])
                .map_affine(|_, v| v + by);
            c.write(&track, Class::Px, false, &[])
        };
        json!({"s": true, "x": axis(px, base.0), "y": axis(py, base.1)})
    } else {
        let (x, y) = (c.value(px, 0.0) + base.0, c.value(py, 0.0) + base.1);
        json!({"a": 0, "k": c.fmt.nums(&[x, y, 0.0], Class::Px)})
    };

    // A turn Lumit adds (along the path, or a camera's aim) is baked from
    // Lumit's own placement, so every player turns the same.
    let rotation = if eye {
        turned(c, |lt| {
            let read = |_: &Layer, p: &Property| c.value(p, lt);
            (0.0, placement(l, lt + offset, &read).rotation_deg.2)
        })
    } else if tr.orient == Orient::AlongPath {
        turned(c, |lt| (c.value(&tr.rotation, lt), flat_turn(tr, lt)))
    } else {
        c.prop(&[&tr.rotation], Class::Deg, tolerance::DEGREES)
    };
    json!({
        "a": c.write(&anchor, Class::Px, true, &[0.0]),
        "p": position,
        "s": c.write(&scale, Class::Px, true, &[100.0]),
        "r": rotation,
        "o": c.prop(&[&tr.opacity], Class::Px, tolerance::PERCENT),
    })
}

/// Degrees read at every sampled time. `f` gives a part kept as it is and a
/// heading, which is kept turning the short way from frame to frame so a
/// player never spins round between two of them.
fn turned(c: &Clock, f: impl Fn(f64) -> (f64, f64)) -> Value {
    if matches!(
        c.layer.transform.rotation.animation,
        Animation::Expression(_)
    ) {
        c.baked.set(true);
    }
    let mut last: Option<f64> = None;
    let samples: Vec<(f64, Vec<f64>)> = c
        .times()
        .into_iter()
        .take_while(|_| !c.host.cancelled())
        .map(|lt| {
            let (kept, mut heading) = f(lt);
            if let Some(p) = last {
                heading -= ((heading - p) / 360.0).round() * 360.0;
            }
            last = Some(heading);
            (lt, vec![kept + heading])
        })
        .collect();
    c.write(
        &fit::linear(&samples, &[tolerance::DEGREES]),
        Class::Deg,
        false,
        &[],
    )
}

/// A Retime as Lottie's `tm`, in source seconds. Lumit holds the precomp's
/// last frame once the map runs past it (as `nested_source_time` does),
/// where a player draws nothing, so a map that does is written as the
/// times Lumit shows, frame by frame.
fn retimed(c: &Clock, retime: &Property, inner: &Composition) -> Value {
    let last = (inner.duration.0.to_f64() - 1.0 / inner.frame_rate.fps().max(1.0)).max(0.0);
    let held = |lt: f64| c.value(retime, lt).clamp(0.0, last);
    let past = c
        .times()
        .into_iter()
        .take_while(|_| !c.host.cancelled())
        .any(|lt| c.value(retime, lt) != held(lt));
    if !past {
        return c.prop(&[retime], Class::Unit, tolerance::SECONDS);
    }
    if matches!(retime.animation, Animation::Expression(_)) {
        c.baked.set(true);
    }
    let track = c.sampled(|lt| vec![held(lt)], &[tolerance::SECONDS]);
    c.write(&track, Class::Unit, false, &[])
}

/// A blend mode's Lottie code, or Normal with a word in the check for the
/// modes a Lottie has no code for.
fn blend(mode: BlendMode, report: &mut Report, place: &Place) -> u8 {
    let code = match mode {
        BlendMode::Normal => Some(0),
        BlendMode::Multiply => Some(1),
        BlendMode::Screen => Some(2),
        BlendMode::Overlay => Some(3),
        BlendMode::Darken => Some(4),
        BlendMode::Lighten => Some(5),
        BlendMode::ColourDodge => Some(6),
        BlendMode::ColourBurn => Some(7),
        BlendMode::HardLight => Some(8),
        BlendMode::SoftLight => Some(9),
        BlendMode::Difference => Some(10),
        BlendMode::Exclusion => Some(11),
        BlendMode::Hue => Some(12),
        BlendMode::Saturation => Some(13),
        BlendMode::Colour => Some(14),
        BlendMode::Luminosity => Some(15),
        BlendMode::Add => Some(16),
        BlendMode::HardMix => Some(17),
        _ => None,
    };
    let name = mode.name();
    match code {
        Some(0) => 0,
        Some(code) => {
            report.players(
                place,
                Feature::BlendMode(code),
                "blend_mode",
                format!("{name} blend"),
                vec![arg("mode", name)],
            );
            code
        }
        None => {
            report.left_out(
                place,
                "blend_mode",
                format!("{name} draws as Normal"),
                vec![arg("mode", name)],
            );
            0
        }
    }
}

/// The layer's masks, `shift` added to their points.
fn masks(
    c: &Clock,
    l: &Layer,
    shift: (f64, f64),
    report: &mut Report,
    place: &Place,
) -> Vec<Value> {
    use lumit_core::mask::MaskMode;
    let still = |p: &Property, v: f64| matches!(p.animation, Animation::Static(x) if x == v);
    // A mask is in the layer's pixels, whose corner moves with a Shape
    // layer's art.
    let art_moves = c
        .times()
        .into_iter()
        .take_while(|_| !c.host.cancelled())
        .any(|lt| art_shift(l, lt) != (0.0, 0.0));
    l.masks
        .iter()
        .map(|mask| {
            let here = place.part(&mask.name);
            // A mask Lumit ignores (off, open, or still at 0%) cuts nothing.
            let closed = mask
                .path_keys
                .first()
                .map_or(mask.path.closed, |k| k.path.closed);
            let off = matches!(mask.opacity.animation, Animation::Static(v) if v <= 0.0);
            let ignored = mask.mode == MaskMode::None || !closed || off;
            let (mode, feature) = match mask.mode {
                _ if ignored => ("n", None),
                MaskMode::None => ("n", None),
                MaskMode::Add => ("a", None),
                MaskMode::Subtract => ("s", Some(Feature::MaskSubtract)),
                MaskMode::Intersect => ("i", Some(Feature::MaskIntersect)),
                MaskMode::Lighten => ("l", Some(Feature::MaskLightenDarken)),
                MaskMode::Darken => ("d", Some(Feature::MaskLightenDarken)),
                MaskMode::Difference => ("f", Some(Feature::MaskDifference)),
            };
            if let Some(feature) = feature {
                let name = format!("{:?}", mask.mode);
                report.players(
                    &here,
                    feature,
                    "mask_mode",
                    format!("{name} masks draw as Add"),
                    vec![arg("mode", name)],
                );
            }
            if !ignored && (!still(&mask.feather, 0.0) || !mask.vertex_feather.is_empty()) {
                report.left_out(
                    &here,
                    "mask_feather",
                    "Mask feather is left out".into(),
                    Vec::new(),
                );
            }
            if !ignored && !still(&mask.expansion, 0.0) {
                report.players(
                    &here,
                    Feature::MaskExpansion,
                    "mask_expansion",
                    "Mask expansion".into(),
                    Vec::new(),
                );
            }
            // Lumit shows the layer whole where a keyed opacity reaches 0,
            // and a player hides it.
            let through_zero = matches!(
                &mask.opacity.animation,
                Animation::Keyframed(k) if k.iter().any(|k| k.value <= 0.0)
            );
            if !ignored && through_zero {
                report.changed(&here, "mask_opacity", "Mask opacity".into(), Vec::new());
            } else if !ignored && !still(&mask.opacity, 100.0) {
                report.players(
                    &here,
                    Feature::MaskOpacity,
                    "mask_opacity",
                    "Mask opacity".into(),
                    Vec::new(),
                );
            }
            let mut m = Map::new();
            c.fmt.name(&mut m, &mask.name);
            m.insert("inv".into(), Value::from(mask.inverted));
            m.insert("mode".into(), Value::from(mode));
            let pt = if art_moves {
                c.path_sampled(
                    |lt| moved(mask.path_at(lt).into_owned(), art_shift(l, lt)),
                    shift,
                )
            } else {
                c.path(&mask.path, &mask.path_keys, shift)
            };
            m.insert("pt".into(), pt);
            m.insert(
                "o".into(),
                c.prop(&[&mask.opacity], Class::Px, tolerance::PERCENT),
            );
            m.insert(
                "x".into(),
                c.prop(&[&mask.expansion], Class::Px, tolerance::PIXELS),
            );
            Value::Object(m)
        })
        .collect()
}
