//! The walk: which of a comp's layers go out, in what order, under which
//! numbers, and the copies the mattes need. Both emitters read it, so a
//! Lottie and an SVG of one comp always agree on what is in them.
//!
//! # In plain terms
//!
//! - A layer goes out when the Viewer would draw it: its eye on, not a guide,
//!   soloed when anything is, inside the span, and of a kind a player can
//!   draw. A kind it can't (a camera, a light, a 3D model, an adjustment
//!   layer, a Sequence layer) is left out and the check says so.
//! - A layer that doesn't go out but has children that do still goes out,
//!   hidden, so they keep following it.
//! - **Mattes.** In Lumit any layer can be another's matte and still draw
//!   itself. In a Lottie the matte is the layer directly above, drawn only
//!   through the layer below it, and some players (Android, Flutter) find it
//!   only there. So every matted layer gets its own copy of its matte source
//!   placed directly above it, and the source itself goes out or not by its
//!   own eye. Every player then finds the same matte.
//!
//! Runs on the export's thread.

use lumit_core::anim::Animation;
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, Layer, LayerInputSource, LayerKind,
    MatteChannel,
};
use uuid::Uuid;

use crate::check::{arg, Place, Report};
use crate::time::Timebase;

/// What a layer is in the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Drawn as itself.
    Drawn,
    /// Not drawn; kept, hidden, so the layers parented to it follow it.
    ParentOnly,
    /// A copy of a matte source, drawn only through the layer below it.
    /// `masks` and `effects` say what of the source the matte reads.
    MatteCopy { masks: bool, effects: bool },
}

/// The matte a layer is drawn through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Matte {
    /// The number of the matte copy directly above.
    pub ind: u32,
    pub luma: bool,
    pub inverted: bool,
}

/// One layer as it goes out.
#[derive(Debug, Clone)]
pub struct Out<'a> {
    pub layer: &'a Layer,
    /// Its number, from 1, unique in its comp (Lottie `ind`).
    pub ind: u32,
    /// Its parent's number.
    pub parent: Option<u32>,
    pub role: Role,
    pub matte: Option<Matte>,
    pub place: Place,
    /// The comp seconds its transform is read across: its own span, and
    /// every span of what follows it, which Lumit places it for.
    pub reach: (f64, f64),
}

/// The layers of `comp` that go out, top of the stack first, with each matte
/// copy directly above the layer it mattes.
pub fn walk<'a>(
    doc: &Document,
    comp: &'a Composition,
    at: &Place,
    tb: &Timebase,
    report: &mut Report,
) -> Vec<Out<'a>> {
    let any_solo = comp
        .layers
        .iter()
        .any(|l| l.switches.solo && l.switches.visible && !l.audio_only);
    let drawn: Vec<bool> = comp
        .layers
        .iter()
        .map(|l| {
            let place = at.layer(l.id, &l.name);
            let viewer_draws = l.switches.visible
                && !l.switches.guide
                && !l.audio_only
                && (!any_solo || l.switches.solo);
            if !viewer_draws {
                return false;
            }
            if !overlaps(l, tb) {
                return false;
            }
            match kind_left_out(doc, l) {
                Some((reason, english)) => {
                    report.left_out(&place, reason, english.to_owned(), Vec::new());
                    false
                }
                None => true,
            }
        })
        .collect();

    // A matte source left out on its own account still draws its matte, so
    // only a kind that can't be drawn at all loses it.
    let by_id = |id: Uuid| comp.layers.iter().position(|l| l.id == id);
    let mut mattes: Vec<Option<(usize, MatteChannel, bool, LayerInputSource)>> =
        vec![None; comp.layers.len()];
    for (i, layer) in comp.layers.iter().enumerate() {
        let (Some(true), Some(m)) = (drawn.get(i).copied(), layer.matte) else {
            continue;
        };
        let Some(source) = by_id(m.layer).filter(|s| *s != i) else {
            continue;
        };
        let Some(src) = comp.layers.get(source) else {
            continue;
        };
        if let Some((_, english)) = kind_left_out(doc, src) {
            report.left_out(
                &at.layer(layer.id, &layer.name),
                "matte_source",
                format!("The matte is left out: {english}"),
                vec![arg("layer", src.name.clone())],
            );
            continue;
        }
        if src.matte.is_some() {
            report.changed(
                &at.layer(layer.id, &layer.name),
                "matte_chain",
                "The matte's own matte is left out".to_owned(),
                vec![arg("layer", src.name.clone())],
            );
        }
        if let Some(slot) = mattes.get_mut(i) {
            *slot = Some((source, m.channel, m.inverted, m.source));
        }
    }

    // Every layer something going out is parented to, all the way up.
    let mut needed = drawn.clone();
    let mut stack: Vec<usize> = (0..comp.layers.len())
        .filter(|i| drawn.get(*i).copied().unwrap_or(false))
        .collect();
    stack.extend(mattes.iter().flatten().map(|m| m.0));
    while let Some(i) = stack.pop() {
        let Some(parent) = comp.layers.get(i).and_then(|l| l.parent).and_then(by_id) else {
            continue;
        };
        if let Some(slot) = needed.get_mut(parent) {
            if !*slot {
                *slot = true;
                stack.push(parent);
            }
        }
    }

    // Numbers: one per layer that goes out, and one per matte copy, in the
    // order they are written.
    let mut ind_of: Vec<Option<u32>> = vec![None; comp.layers.len()];
    let mut next = 1u32;
    for (i, slot) in ind_of.iter_mut().enumerate() {
        if mattes.get(i).is_some_and(Option::is_some) {
            next += 1; // the copy above takes the number before
        }
        if needed.get(i).copied().unwrap_or(false) {
            *slot = Some(next);
            next += 1;
        }
    }
    let parent_ind = |layer: &Layer| {
        layer
            .parent
            .and_then(by_id)
            .and_then(|p| ind_of.get(p).copied().flatten())
    };

    let mut out = Vec::with_capacity(comp.layers.len());
    for (i, layer) in comp.layers.iter().enumerate() {
        let Some(ind) = ind_of.get(i).copied().flatten() else {
            continue;
        };
        let place = at.layer(layer.id, &layer.name);
        let matte = match mattes.get(i).copied().flatten() {
            Some((source, channel, inverted, reads)) => {
                let Some(src) = comp.layers.get(source) else {
                    continue;
                };
                let copy = ind - 1;
                out.push(Out {
                    layer: src,
                    ind: copy,
                    parent: parent_ind(src),
                    role: Role::MatteCopy {
                        masks: !matches!(reads, LayerInputSource::None),
                        effects: matches!(reads, LayerInputSource::EffectsAndMasks),
                    },
                    matte: None,
                    place: at.layer(src.id, &src.name),
                    reach: span(src, tb),
                });
                Some(Matte {
                    ind: copy,
                    luma: matches!(channel, MatteChannel::Luma),
                    inverted,
                })
            }
            None => None,
        };
        out.push(Out {
            layer,
            ind,
            parent: parent_ind(layer),
            role: if drawn.get(i).copied().unwrap_or(false) {
                Role::Drawn
            } else {
                Role::ParentOnly
            },
            matte,
            place,
            reach: span(layer, tb),
        });
    }

    // A parent is read wherever anything below it is drawn. One that isn't
    // drawn itself starts with nothing of its own.
    let mut reach: Vec<(f64, f64)> = out
        .iter()
        .map(|o| match o.role {
            Role::ParentOnly => (f64::INFINITY, f64::NEG_INFINITY),
            _ => o.reach,
        })
        .collect();
    for (i, child) in out.iter().enumerate() {
        let Some((from, to)) = reach.get(i).copied() else {
            continue;
        };
        let mut parent = child.parent;
        // Bounded, so a parent loop in a damaged file can't hang the export.
        for _ in 0..out.len() {
            let Some(p) = parent.and_then(|ind| out.iter().position(|o| o.ind == ind)) else {
                break;
            };
            if let Some(r) = reach.get_mut(p) {
                *r = (r.0.min(from), r.1.max(to));
            }
            parent = out.get(p).and_then(|o| o.parent);
        }
    }
    for (o, r) in out.iter_mut().zip(reach) {
        if r.0 <= r.1 {
            o.reach = r;
        }
    }
    out
}

/// The part of the span written that `layer` shows in.
fn span(layer: &Layer, tb: &Timebase) -> (f64, f64) {
    let from = layer.in_point.0.to_f64().max(tb.start);
    (from, layer.out_point.0.to_f64().min(tb.end).max(from))
}

/// Whether `layer` shows at any moment of the span written.
fn overlaps(layer: &Layer, tb: &Timebase) -> bool {
    layer.in_point.0.to_f64() < tb.end && layer.out_point.0.to_f64() > tb.start
}

/// What a Set matte effect comes across as.
#[derive(Debug, Clone, Copy)]
pub enum SetMatte<'a> {
    /// No matte layer is set, so it does nothing in Lumit either.
    Unset,
    /// The layer's alpha multiplied by `source`'s alpha, or by its opposite
    /// when `inverted`. Lumit reads the source's own pixels, without its
    /// transform, stretched from the source's box `from` onto the layer's
    /// box `to` (both x, y, width, height in the coordinates each one's
    /// content is drawn in).
    Alpha {
        source: &'a Layer,
        inverted: bool,
        from: [f64; 4],
        to: [f64; 4],
    },
    /// No file draws it as Lumit does: the reason id.
    LeftOut(&'static str),
}

/// How `e`, a Set matte on `layer` in `comp`, comes across. A file draws it
/// as a mask of the source's alpha, so it has to read the alpha at full Mix,
/// and either combine with the layer's own alpha or sit on a solid, which is
/// opaque all over. It has to be the layer's first effect and the source
/// must have none, so both pictures are their own boxes, and the boxes have
/// to stand still to be known. The source has to show whenever the layer
/// does.
#[must_use]
pub fn set_matte<'a>(
    doc: &Document,
    comp: &'a Composition,
    layer: &Layer,
    e: &EffectInstance,
) -> SetMatte<'a> {
    let Some(EffectValue::Layer(Some(id))) = e.param("matte") else {
        return SetMatte::Unset;
    };
    let Some(source) = comp.layers.iter().find(|l| l.id == *id && l.id != layer.id) else {
        return SetMatte::Unset;
    };
    let full = matches!(e.param("mix"), Some(EffectValue::Float(p))
        if matches!(p.animation, Animation::Static(v) if v >= 100.0));
    let first = layer.effects.iter().find(|x| x.enabled).map(|x| x.id) == Some(e.id);
    let combines =
        e.bool_of("combine") == Some(true) || matches!(layer.kind, LayerKind::Solid { .. });
    let plain = !source.switches.fx || source.effects.iter().all(|x| !x.enabled);
    let covers = source.in_point <= layer.in_point && source.out_point >= layer.out_point;
    if !matches!(e.param("channel"), Some(EffectValue::Choice(1))) {
        return SetMatte::LeftOut("set_matte_channel");
    }
    let readable = full && first && combines && plain && covers;
    match (readable, own_box(doc, layer), own_box(doc, source)) {
        (true, Some(to), Some(from)) => SetMatte::Alpha {
            source,
            inverted: e.bool_of("matte_invert") == Some(true),
            from,
            to,
        },
        _ => SetMatte::LeftOut("effect"),
    }
}

/// The box a layer's own picture fills, in the coordinates its content is
/// drawn in, when it stands still: a solid's or a precomp's frame, or a
/// Shape layer's art whose box never moves.
fn own_box(doc: &Document, layer: &Layer) -> Option<[f64; 4]> {
    match &layer.kind {
        LayerKind::Solid { def } => {
            let s = doc.solid(*def)?;
            Some([0.0, 0.0, f64::from(s.width), f64::from(s.height)])
        }
        LayerKind::Precomp { comp } => {
            let c = doc.comp(*comp)?;
            Some([0.0, 0.0, f64::from(c.width), f64::from(c.height)])
        }
        LayerKind::Shape { contents } => {
            let still = lumit_core::shape::modifier::lowered(contents)
                .iter()
                .all(|a| !a.item.path_is_animated());
            let (x0, y0, x1, y1) = lumit_core::shape::contents_bounds(contents, 0.0)?;
            still.then_some([x0, y0, x1 - x0, y1 - y0])
        }
        _ => None,
    }
}

/// The English of a Set matte's reason id, for the effect called `label`.
#[must_use]
pub fn set_matte_english(reason: &str, label: &str) -> String {
    match reason {
        "set_matte_channel" => format!("{label} is left out: only an alpha matte comes across"),
        "set_matte_hidden" => format!("{label} is left out: its matte layer is hidden"),
        _ => format!("{label} is left out"),
    }
}

/// Why `layer`'s kind can't be drawn by a player, when it can't: the reason
/// id and its English.
#[must_use]
pub fn kind_left_out(doc: &Document, layer: &Layer) -> Option<(&'static str, &'static str)> {
    if layer.is_adjustment() {
        return Some(("adjustment_layer", "Adjustment layers are left out"));
    }
    match &layer.kind {
        LayerKind::Camera { .. } => Some(("camera_layer", "Cameras are left out")),
        LayerKind::Light { .. } => Some(("light_layer", "Lights are left out")),
        LayerKind::Model { .. } => Some(("model_layer", "3D models are left out")),
        LayerKind::Mesh { .. } => Some(("shape_3d_layer", "3D shapes are left out")),
        LayerKind::Sequence { .. } => Some(("sequence_layer", "Sequence layers are left out")),
        LayerKind::Precomp { comp } => doc
            .comp(*comp)
            .filter(|c| c.graph.is_some())
            .map(|_| ("node_graph", "Node graph compositions are left out")),
        _ => None,
    }
}
