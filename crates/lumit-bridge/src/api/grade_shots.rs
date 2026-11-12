//! A comp's shots for the Colour workspace: every picture clip on a picture
//! Sequence layer and every other layer with a picture to grade, in time
//! order, and a still of one shot on its own.
//!
//! A shot's grade is the Colour effects in its own stack, the clip's for a
//! clip and the layer's for a layer.

use std::sync::Arc;

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::layer::LayerReference;
use crate::api::state::BridgeRenderedFrame;
use crate::api::BridgeError;
use lumit_core::model::{Composition, Document, EffectInstance, Layer, LayerKind};
use lumit_core::sequence::Clip;
use lumit_core::time::{CompTime, Rational};

/// One shot, as the shot list draws it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShot {
    pub layer: LayerReference,
    /// The clip, for a shot on a Sequence layer.
    pub clip: Option<Uuid>,
    /// The layer's name, or the name of what the clip plays.
    pub name: String,
    /// The comp frames the shot covers, the end not included.
    pub start_frame: i64,
    pub end_frame: i64,
    /// The layer's row, the top one 0.
    pub row: u32,
    /// The clip's own label colour, else the layer's.
    pub label: u8,
    /// Whether a Colour effect in the shot's stack is switched on.
    pub graded: bool,
    /// How many Colour effects the shot's stack holds, on or off.
    pub colour_effects: u32,
}

impl CompositionReference {
    /// Every shot in this comp, in time order, the top row first where two
    /// start together. Read on every edit, so it reads the snapshot once and
    /// copies nothing but the list.
    #[frb(sync)]
    pub fn shots(&self) -> Result<Vec<BridgeShot>, BridgeError> {
        let doc = self.snapshot()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;

        // What each clip plays, named once rather than searched for per clip.
        let mut names: Vec<(Uuid, &str)> = doc.items.iter().map(|i| (i.id(), i.name())).collect();
        names.sort_unstable_by_key(|(id, _)| *id);
        let source_name = |id: Uuid| {
            names
                .binary_search_by_key(&id, |(id, _)| *id)
                .map_or("", |i| names[i].1)
        };

        let mut shots = Vec::new();
        for (row, layer) in comp.layers.iter().enumerate() {
            if !gradeable(layer) {
                continue;
            }
            let reference = LayerReference::new(self.project, self.id, layer.id);
            let mut push = |clip: Option<&Clip>, name: &str, effects: &[EffectInstance]| {
                let Some((start_frame, end_frame)) = span(comp, layer, clip) else {
                    return;
                };
                let grade = effects
                    .iter()
                    .filter(|e| lumit_render::grade_lut::in_grade(e));
                shots.push(BridgeShot {
                    layer: reference,
                    clip: clip.map(|c| c.id),
                    name: name.to_owned(),
                    start_frame,
                    end_frame,
                    row: row as u32,
                    label: clip.and_then(|c| c.label).unwrap_or(layer.label),
                    graded: grade.clone().any(|e| e.enabled),
                    colour_effects: grade.count() as u32,
                });
            };
            match &layer.kind {
                LayerKind::Sequence { clips } => {
                    for clip in clips {
                        push(Some(clip), source_name(clip.source.id()), &clip.effects);
                    }
                }
                _ => push(None, &layer.name, &layer.effects),
            }
        }
        shots.sort_by_key(|s| (s.start_frame, s.row));
        Ok(shots)
    }

    /// A small still of one shot on its own at `frame`, its longest edge
    /// `max_edge` pixels: only that layer drawn, and for a clip only that
    /// clip, with its grade. A frame outside the shot is moved into it.
    ///
    /// Made the way [`Self::thumbnail`] is, by the same renderer, and `None`
    /// where that one is.
    pub fn shot_thumbnail(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: u64,
        max_edge: u32,
    ) -> Result<Option<BridgeRenderedFrame>, BridgeError> {
        self.shot_still(layer, clip, None, None, frame, max_edge)
    }

    /// [`Self::shot_thumbnail`] with the shot's stack stopping after grade
    /// node `node`: what that node puts out. A node in a group's grade or a
    /// shared node is cut in that list. Nothing is written.
    pub fn shot_node_thumbnail(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        node: Uuid,
        frame: u64,
        max_edge: u32,
    ) -> Result<Option<BridgeRenderedFrame>, BridgeError> {
        self.shot_still(layer, clip, None, Some(node), frame, max_edge)
    }

    /// [`Self::shot_thumbnail`] with the shot's grade version `version` in
    /// place of the one it shows. Nothing is written.
    pub fn shot_version_thumbnail(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        version: Uuid,
        frame: u64,
        max_edge: u32,
    ) -> Result<Option<BridgeRenderedFrame>, BridgeError> {
        self.shot_still(layer, clip, Some(version), None, frame, max_edge)
    }

    #[frb(ignore)]
    fn shot_still(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        version: Option<Uuid>,
        after: Option<Uuid>,
        frame: u64,
        max_edge: u32,
    ) -> Result<Option<BridgeRenderedFrame>, BridgeError> {
        let doc = self.snapshot()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let (shown, clip_shown) = shot_in(comp, layer, clip)?;
        let (start, end) = span(comp, shown, clip_shown).ok_or(BridgeError::NoClipThere)?;
        let frame = (frame as i64).clamp(start, end - 1).max(0) as u64;
        let longest = comp.width.max(comp.height).max(1);
        let held = after.and_then(|node| {
            lumit_core::grade::groups::holder(&doc, shot_effects(shown, clip_shown), node)
        });

        // A patched copy, never the document: the shot alone, shown, and the
        // version asked for live in it.
        let mut alone = Document::clone(&doc);
        if let Some(version) = version {
            let shot = lumit_core::grade::Shot {
                comp: self.id,
                layer,
                clip,
            };
            let switched = lumit_core::grade::versions::switch(&alone, shot, version)
                .map_err(crate::api::grade_versions::refused)?;
            if let Some(op) = switched {
                lumit_core::ops::apply(&mut alone, &op).map_err(BridgeError::OpError)?;
            }
        }
        let comp = alone.comp_mut(self.id).ok_or(BridgeError::InvalidComp)?;
        for l in &mut comp.layers {
            l.switches.solo = l.id == layer;
            if l.id != layer {
                continue;
            }
            l.switches.visible = true;
            if let (Some(clip), LayerKind::Sequence { clips }) = (clip, &mut l.kind) {
                clips.retain(|c| c.id == clip);
            }
            let Some(node) = after else { continue };
            let stack = match (clip, &mut l.kind) {
                (Some(_), LayerKind::Sequence { clips }) => {
                    clips.first_mut().map(|c| &mut c.effects)
                }
                _ => Some(&mut l.effects),
            }
            .ok_or(BridgeError::InvalidItem)?;
            match held {
                Some((at, ..)) => stack.truncate(at + 1),
                None => cut_after(stack, node)?,
            }
            // The layer's own stack comes after every clip's.
            if clip.is_some() {
                l.effects.clear();
            }
        }
        if let (Some(node), Some((_, group, part))) = (after, held) {
            let g = alone
                .grade_groups
                .iter_mut()
                .find(|g| g.id == group)
                .ok_or(BridgeError::InvalidEffect)?;
            cut_after(g.part_mut(part), node)?;
        }
        let scale = (f64::from(max_edge.max(1)) / f64::from(longest)).clamp(1e-4, 1.0) as f32;
        Ok(
            crate::render::thumbnail(&Arc::new(alone), self.id, frame, scale).map(
                |(rgba, width, height)| BridgeRenderedFrame {
                    frame,
                    width,
                    height,
                    rgba,
                },
            ),
        )
    }

    /// The project as it stands, read under the lock and let go of at once.
    #[frb(ignore)]
    pub(crate) fn snapshot(&self) -> Result<Arc<Document>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(proj.store.snapshot())
    }
}

/// `effects` stopping after `node`.
fn cut_after(effects: &mut Vec<EffectInstance>, node: Uuid) -> Result<(), BridgeError> {
    let at = effects
        .iter()
        .position(|e| e.id == node)
        .ok_or(BridgeError::InvalidEffect)?;
    effects.truncate(at + 1);
    Ok(())
}

/// Whether a layer is a shot, or holds shots: one with a picture of its own.
/// Sound, adjustment layers and the layers that draw nothing are not.
fn gradeable(layer: &Layer) -> bool {
    !layer.audio_only
        && !layer.is_adjustment()
        && matches!(
            layer.kind,
            LayerKind::Footage { .. }
                | LayerKind::Precomp { .. }
                | LayerKind::Solid { .. }
                | LayerKind::Text { .. }
                | LayerKind::Shape { .. }
                | LayerKind::Model { .. }
                | LayerKind::Mesh { .. }
                | LayerKind::Sequence { .. }
        )
}

/// The comp frames a shot covers, the end not included: the layer's span, or
/// the clip's place inside it. `None` for one that covers no whole frame.
fn span(comp: &Composition, layer: &Layer, clip: Option<&Clip>) -> Option<(i64, i64)> {
    let (mut start, mut end) = (layer.in_point.0, layer.out_point.0);
    if let Some(clip) = clip {
        let offset = layer.start_offset.0;
        start = start.max(offset.checked_add(clip.place_start).ok()?);
        end = end.min(offset.checked_add(clip.place_end()).ok()?);
    }
    let frame = |t: Rational| comp.frame_rate.frame_at(CompTime(t));
    let (start, end) = (frame(start), frame(end));
    (start < end).then_some((start, end))
}

/// The layer and clip a shot names, refused when either is gone or the layer
/// is no shot.
#[frb(ignore)]
pub(crate) fn shot_in(
    comp: &Composition,
    layer: Uuid,
    clip: Option<Uuid>,
) -> Result<(&Layer, Option<&Clip>), BridgeError> {
    let layer = comp
        .layers
        .iter()
        .find(|l| l.id == layer)
        .filter(|l| gradeable(l))
        .ok_or(BridgeError::InvalidLayer)?;
    let clip = match (clip, &layer.kind) {
        (None, LayerKind::Sequence { .. }) => return Err(BridgeError::InvalidItem),
        (None, _) => None,
        (Some(id), LayerKind::Sequence { clips }) => Some(
            clips
                .iter()
                .find(|c| c.id == id)
                .ok_or(BridgeError::InvalidItem)?,
        ),
        (Some(_), _) => return Err(BridgeError::NotSequence),
    };
    Ok((layer, clip))
}

/// A shot's name: the layer's, or what the clip plays.
#[frb(ignore)]
pub(crate) fn shot_name(doc: &Document, layer: &Layer, clip: Option<&Clip>) -> String {
    match clip {
        Some(clip) => doc
            .item(clip.source.id())
            .map(|i| i.name().to_owned())
            .unwrap_or_default(),
        None => layer.name.clone(),
    }
}

/// The stack a shot's grade lives in.
#[frb(ignore)]
pub(crate) fn shot_effects<'a>(layer: &'a Layer, clip: Option<&'a Clip>) -> &'a [EffectInstance] {
    clip.map_or(&layer.effects, |c| &c.effects)
}

/// The layer time a shot's own stack is read at, at comp `frame`: a layer's
/// clock, or a clip's, which starts at the clip.
#[frb(ignore)]
pub(crate) fn shot_time(
    comp: &Composition,
    layer: &Layer,
    clip: Option<&Clip>,
    frame: i64,
) -> Result<f64, BridgeError> {
    let at = comp
        .frame_rate
        .time_of_frame(frame)
        .map_err(|_| BridgeError::InvalidTime)?;
    let mut local =
        at.0.checked_sub(layer.start_offset.0)
            .map_err(|_| BridgeError::InvalidTime)?;
    if let Some(clip) = clip {
        local = local
            .checked_sub(clip.place_start)
            .map_err(|_| BridgeError::InvalidTime)?;
    }
    Ok(local.to_f64())
}

/// The frame a shot's grade is read at: `frame`, moved inside the shot.
#[frb(ignore)]
pub(crate) fn frame_in_shot(
    comp: &Composition,
    layer: &Layer,
    clip: Option<&Clip>,
    frame: i64,
) -> Result<i64, BridgeError> {
    let (start, end) = span(comp, layer, clip).ok_or(BridgeError::NoClipThere)?;
    Ok(frame.clamp(start, end - 1))
}

/// The stack that holds `ids`: the layer's own, or the stack of the clip on
/// it that holds the first of them, so a picked clip's grade is copied by its
/// ids the way a layer's is.
#[frb(ignore)]
pub(crate) fn stack_holding(
    effects: Vec<EffectInstance>,
    kind: LayerKind,
    ids: &[Uuid],
) -> Vec<EffectInstance> {
    if ids.is_empty() || effects.iter().any(|e| ids.contains(&e.id)) {
        return effects;
    }
    match kind {
        LayerKind::Sequence { clips } => clips
            .into_iter()
            .find(|c| c.effects.iter().any(|e| ids.contains(&e.id)))
            .map_or(effects, |c| c.effects),
        _ => effects,
    }
}
