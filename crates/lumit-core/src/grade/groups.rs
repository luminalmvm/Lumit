//! Grade groups: shots sharing a grade laid on before theirs and one laid on
//! after, the colourist's group pre-clip and post-clip.
//!
//! The document holds the groups. A member carries two `grade_group`
//! references in its own stack, a Pre at the start of its grade and a Post
//! after it ([`crate::fx::effects::grade_group`]), and the render puts the
//! group's effects where each one stands ([`worn_at`]). So the group's grade is
//! edited once, every member picks it up, and a group that has gone leaves its
//! references passing the picture over untouched.
//!
//! A group's keys are on the comp's clock, so a member reads them where its own
//! layer sits.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{edit, is_grade, Chain, GradeError, Shot};
use crate::fx::effects::grade_group::{
    group_of, is_reference, is_shared, part_of, reference, Part,
};
use crate::model::{Document, EffectInstance, Layer, LayerKind, ProjectItem};
use crate::ops::Op;
use crate::time::Rational;

/// One grade group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradeGroup {
    pub id: Uuid,
    pub name: String,
    /// What runs before each member's own grade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre: Vec<EffectInstance>,
    /// What runs after it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post: Vec<EffectInstance>,
    /// A shared node rather than a group: its Pre list is the one node
    /// ([`super::shared`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared: bool,
}

impl GradeGroup {
    /// The list `part` names.
    #[must_use]
    pub fn part(&self, part: Part) -> &[EffectInstance] {
        match part {
            Part::Pre => &self.pre,
            Part::Post => &self.post,
        }
    }

    /// The list `part` names, to change.
    pub fn part_mut(&mut self, part: Part) -> &mut Vec<EffectInstance> {
        match part {
            Part::Pre => &mut self.pre,
            Part::Post => &mut self.post,
        }
    }
}

/// `layer` as it draws at layer time `lt`: wearing the clip it shows
/// ([`crate::sequence::worn_at`]), and with each group reference in its stack
/// replaced by that group's effects. `None` where it draws as it is stored.
///
/// The render and the frame key both read a layer through this, so a change
/// to a group's effects changes the picture and the frame's name together.
#[must_use]
pub fn worn_at(doc: &Document, layer: &Layer, lt: f64) -> Option<Layer> {
    let worn = crate::sequence::worn_at(layer, lt);
    let drawn = worn.as_ref().unwrap_or(layer);
    if !drawn.effects.iter().any(is_reference) {
        return worn;
    }
    let mut effects = Vec::with_capacity(drawn.effects.len());
    for e in &drawn.effects {
        if !is_reference(e) {
            effects.push(e.clone());
            continue;
        }
        let Some(g) = group_of(e).and_then(|id| group(doc, id)) else {
            continue;
        };
        if !e.enabled {
            continue;
        }
        let mut list = g.part(part_of(e)).to_vec();
        if let Ok(back) = Rational::ZERO.checked_sub(drawn.start_offset.0) {
            crate::preset::shift_keys(&mut list, back);
        }
        effects.extend(list);
    }
    let mut worn = worn.unwrap_or_else(|| layer.clone());
    worn.effects = effects;
    Some(worn)
}

/// Group `id`, if the document has it.
#[must_use]
pub fn group(doc: &Document, id: Uuid) -> Option<&GradeGroup> {
    doc.grade_groups.iter().find(|g| g.id == id)
}

/// The group a stack's references name, if any. A shared node's reference
/// is not a group.
#[must_use]
pub fn group_in(stack: &[EffectInstance]) -> Option<Uuid> {
    stack.iter().filter(|e| !is_shared(e)).find_map(group_of)
}

/// Which group's list holds effect `id`, and which of its two.
#[must_use]
pub fn home_of(doc: &Document, id: Uuid) -> Option<(Uuid, Part)> {
    doc.grade_groups.iter().find_map(|g| {
        [Part::Pre, Part::Post]
            .into_iter()
            .find(|p| g.part(*p).iter().any(|e| e.id == id))
            .map(|p| (g.id, p))
    })
}

/// Where effect `id` sits when a reference in `stack` holds it: that
/// reference's place in `stack`, and the group and list it names.
#[must_use]
pub fn holder(doc: &Document, stack: &[EffectInstance], id: Uuid) -> Option<(usize, Uuid, Part)> {
    stack.iter().enumerate().find_map(|(at, e)| {
        let g = group(doc, group_of(e)?)?;
        let part = part_of(e);
        g.part(part)
            .iter()
            .any(|x| x.id == id)
            .then_some((at, g.id, part))
    })
}

/// Lay `staged` over the group list that holds its first effect, for a
/// preview. False when no group holds it.
pub fn stage(doc: &mut Document, staged: &[EffectInstance]) -> bool {
    let Some((id, part)) = staged.first().and_then(|f| home_of(doc, f.id)) else {
        return false;
    };
    let Some(g) = doc.grade_groups.iter_mut().find(|g| g.id == id) else {
        return false;
    };
    *g.part_mut(part) = staged.to_vec();
    true
}

/// The op that lets `edit` change one of group `id`'s two lists.
pub fn set_list(
    doc: &Document,
    id: Uuid,
    part: Part,
    list: Vec<EffectInstance>,
) -> Result<Option<Op>, GradeError> {
    let mut groups = doc.grade_groups.clone();
    let g = groups
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or(GradeError::UnknownGroup)?;
    *g.part_mut(part) = list;
    Ok((groups != doc.grade_groups).then_some(Op::SetGradeGroups { groups }))
}

/// Every shot in `doc` that is in group `id`, comp by comp, layer by layer.
#[must_use]
pub fn members(doc: &Document, id: Uuid) -> Vec<Shot> {
    let mut out = Vec::new();
    let comps = doc.items.iter().filter_map(|i| match i {
        ProjectItem::Composition(c) => Some(c),
        _ => None,
    });
    for comp in comps {
        for layer in &comp.layers {
            let shot = |clip| Shot {
                comp: comp.id,
                layer: layer.id,
                clip,
            };
            if group_in(&layer.effects) == Some(id) {
                out.push(shot(None));
            }
            if let LayerKind::Sequence { clips } = &layer.kind {
                for c in clips.iter().filter(|c| group_in(&c.effects) == Some(id)) {
                    out.push(shot(Some(c.id)));
                }
            }
        }
    }
    out
}

/// Take a stack's references out and, for `group`, put a Pre at the start of
/// its grade and a Post after it.
fn place(stack: &mut Vec<EffectInstance>, group: Option<Uuid>) {
    stack.retain(|e| !is_reference(e) || is_shared(e));
    let Some(group) = group else {
        return;
    };
    let start = stack.iter().position(is_grade).unwrap_or(stack.len());
    let end = stack.iter().rposition(is_grade).map_or(start, |i| i + 1);
    if let (Some(pre), Some(post)) = (reference(group, Part::Pre), reference(group, Part::Post)) {
        stack.insert(end, post);
        stack.insert(start, pre);
    }
}

/// Put `shot` in group `group`, move it there from another, or with `None`
/// take it out of any. A shot is in one group at a time.
pub fn set_group(
    doc: &Document,
    shot: Shot,
    group: Option<Uuid>,
) -> Result<Option<Op>, GradeError> {
    if group.is_some_and(|id| self::group(doc, id).is_none_or(|g| g.shared)) {
        return Err(GradeError::UnknownGroup);
    }
    edit(doc, shot, |stack, _| {
        if group_in(stack) != group {
            place(stack, group);
        }
        Ok(())
    })
    .map(|((), op)| op)
}

/// A new group, empty, with `shots` in it. Answers its id.
pub fn create(doc: &Document, shots: &[Shot]) -> Result<(Uuid, Op), GradeError> {
    let id = Uuid::now_v7();
    let mut n = doc.grade_groups.len() + 1;
    while doc
        .grade_groups
        .iter()
        .any(|g| g.name == format!("Group {n}"))
    {
        n += 1;
    }
    let mut groups = doc.grade_groups.clone();
    groups.push(GradeGroup {
        id,
        name: format!("Group {n}"),
        pre: Vec::new(),
        post: Vec::new(),
        shared: false,
    });
    let mut chain = Chain::new(doc);
    chain.push(Some(Op::SetGradeGroups { groups }))?;
    for shot in shots {
        let op = set_group(&chain.scratch, *shot, Some(id))?;
        chain.push(op)?;
    }
    let op = chain.finish().ok_or(GradeError::UnknownGroup)?;
    Ok((id, op))
}

/// Call group `id` `name`.
pub fn rename(doc: &Document, id: Uuid, name: &str) -> Result<Option<Op>, GradeError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(GradeError::EmptyName);
    }
    let mut groups = doc.grade_groups.clone();
    let g = groups
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or(GradeError::UnknownGroup)?;
    if g.name == name {
        return Ok(None);
    }
    g.name = name.to_owned();
    Ok(Some(Op::SetGradeGroups { groups }))
}

/// Delete group `id`, and its references from every member.
pub fn delete(doc: &Document, id: Uuid) -> Result<Op, GradeError> {
    if group(doc, id).is_none() {
        return Err(GradeError::UnknownGroup);
    }
    let mut chain = Chain::new(doc);
    for shot in members(doc, id) {
        let op = set_group(&chain.scratch, shot, None)?;
        chain.push(op)?;
    }
    let groups = doc
        .grade_groups
        .iter()
        .filter(|g| g.id != id)
        .cloned()
        .collect();
    chain.push(Some(Op::SetGradeGroups { groups }))?;
    chain.finish().ok_or(GradeError::UnknownGroup)
}
