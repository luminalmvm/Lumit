//! Shared nodes: one grade node used by several shots, so changing it in one
//! shot changes it in all of them.
//!
//! The node lives on the document as a grade group marked shared, its Pre
//! list holding the one effect. Each shot that uses it carries a Shared
//! reference (a `grade_group` whose part is Shared) where the node goes, in
//! any place in its stack. So the render and the frame key lay the node in
//! through the group machinery ([`super::groups::worn_at`]), it is edited as
//! a group's effect is, and a Shared reference never counts as the shot's
//! group. A reference whose node has gone passes the picture over.
//!
//! The node's keys are on the comp's clock, as a group's are, so moving it in
//! and out of a shot moves its keys by where the shot sits.

use uuid::Uuid;

use super::groups::GradeGroup;
use super::{edit, is_grade, read, Chain, GradeError, Shot};
use crate::fx::effects::grade_group::{group_of, is_reference, is_shared, shared_reference};
use crate::model::{Document, EffectInstance, LayerKind, ProjectItem};
use crate::ops::Op;
use crate::time::Rational;

/// Every shared node, in the order they were made.
pub fn nodes(doc: &Document) -> impl Iterator<Item = &GradeGroup> {
    doc.grade_groups.iter().filter(|g| g.shared)
}

/// Whether `e` is a reference to shared node `id`.
fn refers(e: &EffectInstance, id: Uuid) -> bool {
    is_shared(e) && group_of(e) == Some(id)
}

/// How far `shot`'s own keys sit from the comp's clock: its layer's start,
/// and for a clip where it is placed on the layer.
fn clock(doc: &Document, shot: Shot) -> Result<Rational, GradeError> {
    let layer = doc
        .comp(shot.comp)
        .and_then(|c| c.layers.iter().find(|l| l.id == shot.layer))
        .ok_or(GradeError::UnknownShot)?;
    let mut at = layer.start_offset.0;
    if let (Some(clip), LayerKind::Sequence { clips }) = (shot.clip, &layer.kind) {
        let c = clips
            .iter()
            .find(|c| c.id == clip)
            .ok_or(GradeError::UnknownShot)?;
        at = at
            .checked_add(c.place_start)
            .map_err(|_| GradeError::UnknownShot)?;
    }
    Ok(at)
}

/// `list` with its keys moved by `by`.
fn shifted(mut list: Vec<EffectInstance>, by: Rational) -> Vec<EffectInstance> {
    crate::preset::shift_keys(&mut list, by);
    list
}

/// Where a reference goes in a stack that doesn't name a place: after its
/// last grade node, or at the end.
fn after_grade(stack: &[EffectInstance]) -> usize {
    stack
        .iter()
        .rposition(is_grade)
        .map_or(stack.len(), |i| i + 1)
}

/// Every shot that uses shared node `id`, comp by comp, layer by layer.
#[must_use]
pub fn users(doc: &Document, id: Uuid) -> Vec<Shot> {
    let mut out = Vec::new();
    let uses = |stack: &[EffectInstance]| stack.iter().any(|e| refers(e, id));
    for comp in doc.items.iter().filter_map(|i| match i {
        ProjectItem::Composition(c) => Some(c),
        _ => None,
    }) {
        for layer in &comp.layers {
            let shot = |clip| Shot {
                comp: comp.id,
                layer: layer.id,
                clip,
            };
            if uses(&layer.effects) {
                out.push(shot(None));
            }
            if let LayerKind::Sequence { clips } = &layer.kind {
                for c in clips.iter().filter(|c| uses(&c.effects)) {
                    out.push(shot(Some(c.id)));
                }
            }
        }
    }
    out
}

/// The id of every reference to shared node `id`, in every shot's stack.
#[must_use]
pub fn references(doc: &Document, id: Uuid) -> Vec<Uuid> {
    let mut out = Vec::new();
    let mut take = |stack: &[EffectInstance]| {
        out.extend(stack.iter().filter(|e| refers(e, id)).map(|e| e.id));
    };
    for comp in doc.items.iter().filter_map(|i| match i {
        ProjectItem::Composition(c) => Some(c),
        _ => None,
    }) {
        for layer in &comp.layers {
            take(&layer.effects);
            if let LayerKind::Sequence { clips } = &layer.kind {
                for c in clips {
                    take(&c.effects);
                }
            }
        }
    }
    out
}

/// Share node `node` of shot `from` with `shots`: the node moves into a new
/// shared node, `from` keeps a reference where the node was, and each of
/// `shots` gets one after its last grade node. A group's reference or a
/// shared node's own can't be shared again. Answers the shared node's id.
pub fn share(
    doc: &Document,
    from: Shot,
    node: Uuid,
    shots: &[Shot],
) -> Result<(Uuid, Op), GradeError> {
    let (stack, _) = read(doc, from)?;
    let inst = stack
        .iter()
        .find(|e| e.id == node)
        .filter(|e| !is_reference(e))
        .ok_or(GradeError::UnknownNode)?
        .clone();
    let id = Uuid::now_v7();
    let name = inst.custom_name.clone().unwrap_or_else(|| {
        crate::fx::builtins::schema(&inst.effect.match_name)
            .map_or_else(|| inst.effect.match_name.clone(), |s| s.label.to_owned())
    });
    let mut groups = doc.grade_groups.clone();
    groups.push(GradeGroup {
        id,
        name,
        pre: shifted(vec![inst], clock(doc, from)?),
        post: Vec::new(),
        shared: true,
    });
    let reference = shared_reference(id).ok_or(GradeError::UnknownNode)?;
    let mut chain = Chain::new(doc);
    chain.push(Some(Op::SetGradeGroups { groups }))?;
    let ((), op) = edit(&chain.scratch, from, |stack, _| {
        if let Some(slot) = stack.iter_mut().find(|e| e.id == node) {
            *slot = reference.clone();
        }
        Ok(())
    })?;
    chain.push(op)?;
    for shot in shots.iter().filter(|s| **s != from) {
        let op = add_ref(&chain.scratch, *shot, id, None)?.1;
        chain.push(op)?;
    }
    let op = chain.finish().ok_or(GradeError::UnknownNode)?;
    Ok((id, op))
}

/// A reference to shared node `id` in `shot`, right after effect `after`, or
/// after its last grade node. A shot that already uses the node is left as it
/// is. Answers the reference's id.
pub fn add_ref(
    doc: &Document,
    shot: Shot,
    id: Uuid,
    after: Option<Uuid>,
) -> Result<(Uuid, Option<Op>), GradeError> {
    if !nodes(doc).any(|g| g.id == id) {
        return Err(GradeError::UnknownNode);
    }
    edit(doc, shot, |stack, _| {
        if let Some(e) = stack.iter().find(|e| refers(e, id)) {
            return Ok(e.id);
        }
        let at = match after {
            Some(after) => stack
                .iter()
                .position(|e| e.id == after)
                .map(|i| i + 1)
                .ok_or(GradeError::UnknownNode)?,
            None => after_grade(stack),
        };
        let reference = shared_reference(id).ok_or(GradeError::UnknownNode)?;
        let new = reference.id;
        stack.insert(at, reference);
        Ok(new)
    })
}

/// Stop sharing node `id`: every shot that uses it gets its own copy where
/// the reference was, and the shared node goes.
pub fn unshare(doc: &Document, id: Uuid) -> Result<Op, GradeError> {
    let g = nodes(doc)
        .find(|g| g.id == id)
        .ok_or(GradeError::UnknownNode)?;
    let mut chain = Chain::new(doc);
    for shot in users(doc, id) {
        let back = Rational::ZERO
            .checked_sub(clock(doc, shot)?)
            .map_err(|_| GradeError::UnknownShot)?;
        let own = shifted(g.pre.clone(), back);
        let ((), op) = edit(&chain.scratch, shot, |stack, _| {
            let mut out = Vec::with_capacity(stack.len());
            for e in stack.drain(..) {
                if !refers(&e, id) {
                    out.push(e);
                    continue;
                }
                out.extend(own.iter().map(|o| EffectInstance {
                    id: Uuid::now_v7(),
                    ..o.clone()
                }));
            }
            *stack = out;
            Ok(())
        })?;
        chain.push(op)?;
    }
    let groups = doc
        .grade_groups
        .iter()
        .filter(|g| g.id != id)
        .cloned()
        .collect();
    chain.push(Some(Op::SetGradeGroups { groups }))?;
    chain.finish().ok_or(GradeError::UnknownNode)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::builtins::instantiate;

    /// A comp of `layers` adjustment layers, each starting a second later
    /// than the last and graded by one Correction.
    fn doc_with(layers: usize) -> (Document, Vec<Shot>) {
        use crate::model::{Composition, LinearColour, MotionBlur, TransformGroup};
        use crate::time::{Duration, FrameRate};
        let comp_id = Uuid::now_v7();
        let mut shots = Vec::new();
        let mut list = Vec::new();
        for i in 0..layers {
            let mut layer = crate::edits::base_layer(
                "plate".into(),
                LayerKind::Adjustment,
                Rational::new(4, 1).unwrap(),
                TransformGroup::default(),
            );
            layer.start_offset.0 = Rational::new(i as i64, 1).unwrap();
            layer.effects.push(instantiate("correction").unwrap());
            shots.push(Shot {
                comp: comp_id,
                layer: layer.id,
                clip: None,
            });
            list.push(layer);
        }
        let comp = Composition {
            id: comp_id,
            name: "Comp".into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(25, 1).unwrap(),
            duration: Duration(Rational::new(4, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: list,
            groups: Vec::new(),
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            master_volume_db: 0.0,
            master_keys: None,
            master_automation: Default::default(),
            loudness_target: None,
            lighting: Default::default(),
            sound_mix: false,
            beat_grid: None,
            graph: None,
            extra: serde_json::Map::new(),
        };
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));
        (doc, shots)
    }

    fn stack(doc: &Document, shot: Shot) -> Vec<EffectInstance> {
        read(doc, shot).unwrap().0.to_vec()
    }

    /// A node shared from one shot reaches the others, an edit to it is one
    /// edit for all, a shared reference is not the shot's group, and
    /// unsharing hands each shot its own copy back.
    #[test]
    fn a_shared_node_is_one_node_in_every_shot() {
        let (mut doc, shots) = doc_with(3);
        let node = stack(&doc, shots[0])[0].id;
        let (id, op) = share(&doc, shots[0], node, &shots[1..]).unwrap();
        crate::ops::apply(&mut doc, &op).unwrap();
        for s in &shots {
            assert!(stack(&doc, *s).iter().any(|e| refers(e, id)));
            assert_eq!(super::super::groups::group_in(&stack(&doc, *s)), None);
        }
        assert_eq!(users(&doc, id), shots);
        let layer = doc.comp(shots[2].comp).unwrap().layers[2].clone();
        let worn = super::super::groups::worn_at(&doc, &layer, 0.0).unwrap();
        assert!(worn.effects.iter().any(|e| e.id == node));

        let op = unshare(&doc, id).unwrap();
        crate::ops::apply(&mut doc, &op).unwrap();
        assert_eq!(nodes(&doc).count(), 0);
        let ids: Vec<Uuid> = shots
            .iter()
            .map(|s| {
                let st = stack(&doc, *s);
                assert!(st.iter().all(|e| !is_shared(e)));
                st.iter()
                    .find(|e| e.effect.match_name == "correction")
                    .unwrap()
                    .id
            })
            .collect();
        assert!(ids[0] != ids[1] && ids[1] != ids[2]);
    }
}
