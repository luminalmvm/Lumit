//! A copy's document: the project with the copy's values written into the
//! template it places.
//!
//! Every reader of a nested composition (the draw builder, the frame key, the
//! decode planner, the mix) asks here for the document to read a copy's
//! template from, and reads the template's composition out of it by id as it
//! always does. A copy that sets nothing gets `None` and reads the project
//! itself, which is every Precomp layer and clip of every composition that is
//! not a template.
//!
//! A made document is kept while its project is, by the values that made it,
//! so a still copy costs one document for as long as nothing is edited and a
//! thousand frames of it share that one.
//!
//! Runs wherever the document is read.

use std::sync::{Arc, Mutex, Weak};

use uuid::Uuid;

use super::{Fit, Picture, Setting};
use crate::expression::ExpressionContext;
use crate::model::{
    Composition, Document, EffectInstance, Layer, LayerKind, LinearColour, MotionBlur, ProjectItem,
};
use crate::sequence::Clip;
use crate::time::{CompTime, Duration, Rational};

/// How many made documents are kept. A frame shows a handful of copies, and
/// a miss costs one copy of the project.
// ponytail: a made document is the whole project copied. Copying only the
// template and what it places is the upgrade, if a large project with many
// keyed copies ever scrubs slowly.
const KEPT: usize = 16;

/// One kept document: the project it was made from, the template, the values
/// written, and the document.
type Kept = (Weak<Document>, Uuid, Vec<u8>, Arc<Document>);

/// The document the template in composition `nested` is read from for a copy
/// holding `inst`, at the copy's own time `at`.
///
/// `None` when the copy sets nothing, or `nested` is not a template: the
/// project itself is what to read.
#[must_use]
pub fn document(
    doc: &Arc<Document>,
    nested: Uuid,
    inst: Option<&EffectInstance>,
    at: f64,
    context: Option<&Arc<ExpressionContext>>,
    drivers: Option<&crate::fx::drivers::ResolvedDrivers>,
) -> Option<Arc<Document>> {
    let inst = inst?;
    if crate::fx::effects::template::comp_of(inst) != Some(nested) {
        return None;
    }
    let template = doc.comp(nested)?.template()?;
    let settings = crate::fx::effects::template::settings_at(inst, &template, at, context, drivers);
    with(doc, nested, &settings)
}

/// The project with `settings` written into composition `comp`. `None` when
/// there is nothing to write.
#[must_use]
pub fn with(doc: &Arc<Document>, comp: Uuid, settings: &[Setting]) -> Option<Arc<Document>> {
    if settings.is_empty() {
        return None;
    }
    static MEMO: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
    let written = serde_json::to_vec(settings).ok()?;
    let same = |kept: &Kept| {
        kept.1 == comp && kept.2 == written && std::ptr::eq(kept.0.as_ptr(), Arc::as_ptr(doc))
    };
    if let Ok(mut memo) = MEMO.lock() {
        if let Some(at) = memo.iter().position(same) {
            // The one just asked for goes last, so the one dropped for room
            // is the one asked for longest ago.
            let kept = memo.remove(at);
            let made = kept.3.clone();
            memo.push(kept);
            return Some(made);
        }
    }
    let mut made = Document::clone(doc);
    for setting in settings {
        super::rows::write(&mut made, comp, &setting.target, &setting.value);
    }
    let made = Arc::new(made);
    if let Ok(mut memo) = MEMO.lock() {
        memo.retain(|kept| kept.0.strong_count() > 0);
        if memo.len() >= KEPT {
            memo.remove(0);
        }
        memo.push((Arc::downgrade(doc), comp, written, made.clone()));
    }
    Some(made)
}

/// The document a Precomp `layer` of composition `owner` reads its nested
/// composition from at layer time `lt`, when the layer is a copy of a
/// template that sets something.
#[must_use]
pub fn for_layer(
    doc: &Arc<Document>,
    owner: &Composition,
    layer: &Layer,
    lt: f64,
    comp_time: f64,
    drivers: Option<&crate::fx::drivers::ResolvedDrivers>,
) -> Option<Arc<Document>> {
    let inst = layer.graph_inputs.as_ref()?;
    let LayerKind::Precomp { comp: nested } = &layer.kind else {
        return None;
    };
    let context = context_of(doc, owner, layer, comp_time);
    document(doc, *nested, Some(inst), lt, Some(&context), drivers)
}

/// The document a `clip` on Sequence `layer` reads its composition from at
/// layer time `lt`, when the clip is a copy of a template that sets
/// something. A clip's values are keyed on the clip's own clock, as its
/// effects are.
#[must_use]
pub fn for_clip(
    doc: &Arc<Document>,
    owner: &Composition,
    layer: &Layer,
    clip: &Clip,
    lt: f64,
    comp_time: f64,
) -> Option<Arc<Document>> {
    let inst = clip.inputs.as_ref()?;
    let crate::sequence::ClipSource::Comp(nested) = clip.source else {
        return None;
    };
    let context = context_of(doc, owner, layer, comp_time);
    let at = lt - clip.place_start.to_f64();
    document(doc, nested, Some(inst), at, Some(&context), None)
}

/// The context a copy's own rows are read under: the composition and the
/// layer placing it, so an expression on a copy's row reads what any row of
/// that layer would.
fn context_of(
    doc: &Arc<Document>,
    owner: &Composition,
    layer: &Layer,
    comp_time: f64,
) -> Arc<ExpressionContext> {
    Arc::new(ExpressionContext {
        document: doc.clone(),
        comp: Some(owner.id),
        layer: Some(layer.id),
        comp_time,
        current_depth: 0,
        inputs: None,
    })
}

/// An id made from other ids, the same every time. A copy's picture is
/// fitted inside a composition and a layer the project never held, and they
/// must be the same two for every frame and for every reader, or the decoded
/// picture and the layer asking for it would not meet.
fn made_id(tag: &[u8], parts: &[Uuid], fit: Fit) -> Uuid {
    let mut hasher = blake3::Hasher::new();
    hasher.update(tag);
    for part in parts {
        hasher.update(part.as_bytes());
    }
    hasher.update(&fit.to_choice().to_le_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    Uuid::from_bytes(bytes)
}

/// Put a copy's `picture` in the place of what `layer` of composition `comp`
/// shows, fitted into `frame`, the size of the picture it replaces.
///
/// The layer keeps everything it has: its transform, masks, effects and
/// place in the stack were all made for a picture of that size. Its source
/// becomes a composition of that size holding the copy's picture filled,
/// fitted or stretched, so nothing on the layer needs to know the picture
/// changed, and one that covers more than the frame is cut at it.
///
/// Answers whether the layer was there and the picture is something a layer
/// can show.
pub(crate) fn fitted(
    doc: &mut Document,
    comp: Uuid,
    layer: Uuid,
    frame: (u32, u32),
    picture: &Picture,
) -> bool {
    let (fw, fh) = (f64::from(frame.0), f64::from(frame.1));
    let (pw, ph) = (f64::from(picture.size.0), f64::from(picture.size.1));
    if fw <= 0.0 || fh <= 0.0 || pw <= 0.0 || ph <= 0.0 {
        return false;
    }
    let kind = match doc.item(picture.item) {
        Some(ProjectItem::Footage(item)) => LayerKind::Footage { item: item.id },
        Some(ProjectItem::Composition(c)) if c.id != comp => LayerKind::Precomp { comp: c.id },
        _ => return false,
    };
    let Some(owner) = doc.comp(comp) else {
        return false;
    };
    let Some(slot) = owner.layers.iter().find(|l| l.id == layer) else {
        return false;
    };
    if !matches!(
        slot.kind,
        LayerKind::Footage { .. } | LayerKind::Precomp { .. }
    ) {
        return false;
    }
    // As long as the slot ever reads: its own out point on its own clock, and
    // never shorter than the template.
    let reach = slot
        .out_point
        .0
        .checked_sub(slot.start_offset.0)
        .ok()
        .filter(|r| *r > owner.duration.0)
        .unwrap_or(owner.duration.0);
    let (sx, sy) = match picture.fit {
        Fit::Stretch => (fw / pw, fh / ph),
        Fit::Fit => {
            let s = (fw / pw).min(fh / ph);
            (s, s)
        }
        Fit::Fill => {
            let s = (fw / pw).max(fh / ph);
            (s, s)
        }
    };
    let mut transform = crate::edits::centred_transform(pw, ph, frame.0, frame.1);
    transform.scale_x = crate::anim::Property::fixed(sx * 100.0);
    transform.scale_y = crate::anim::Property::fixed(sy * 100.0);
    let parts = [layer, picture.item];
    let mut inner = crate::edits::base_layer(slot.name.clone(), kind, reach, transform);
    inner.id = made_id(b"template-picture-layer/", &parts, picture.fit);
    inner.in_point = CompTime(Rational::ZERO);
    let made = Composition {
        id: made_id(b"template-picture/", &parts, picture.fit),
        name: slot.name.clone(),
        width: frame.0,
        height: frame.1,
        frame_rate: owner.frame_rate,
        duration: Duration(reach),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![inner],
        groups: Vec::new(),
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        beat_grid: None,
        graph: None,
        extra: serde_json::Map::new(),
    };
    let made_id = made.id;
    if doc.comp(made_id).is_none() {
        doc.items.push(ProjectItem::Composition(made));
    }
    let Some(slot) = doc
        .comp_mut(comp)
        .and_then(|c| c.layers.iter_mut().find(|l| l.id == layer))
    else {
        return false;
    };
    slot.kind = LayerKind::Precomp { comp: made_id };
    // Values a Precomp layer kept for the composition it showed before are
    // not this one's.
    slot.graph_inputs = None;
    true
}
