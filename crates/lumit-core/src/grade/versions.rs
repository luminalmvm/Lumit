//! Versions of a shot's grade: named alternatives a colourist keeps and
//! switches between, "Version 1", "Version 2".
//!
//! **Nothing is stored twice.** The live version's grade is the one in the
//! stack, so its own list stays empty; the others keep their effects here.
//! Switching takes the live grade out of the stack into its version and puts
//! the other's where it started, so the effects around the grade keep their
//! places, and the whole switch is one op.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{edit, is_grade, GradeError, Shot};
use crate::fx::effects::grade_group::{is_reference, part_of, Part};
use crate::model::{Document, EffectInstance};
use crate::ops::Op;

/// A shot's versions, and which one the stack holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradeVersions {
    /// The version the stack holds.
    pub live: Uuid,
    /// Every version, in the order they were made.
    pub versions: Vec<GradeVersion>,
}

/// One version of a grade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradeVersion {
    pub id: Uuid,
    pub name: String,
    /// The grade, in stack order. Empty on the live version, whose grade is
    /// the stack's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectInstance>,
}

impl GradeVersions {
    /// "Version n" for the next n no version is called yet.
    fn fresh_name(&self) -> String {
        let mut n = self.versions.len() + 1;
        while self
            .versions
            .iter()
            .any(|v| v.name == format!("Version {n}"))
        {
            n += 1;
        }
        format!("Version {n}")
    }
}

/// Fresh ids for every stored effect, for a copy of the layer or clip that
/// holds them. An effect is found by its id alone, so a copy that shared them
/// would answer for the original once a version was switched in.
pub fn respawn(versions: &mut Option<GradeVersions>) {
    for v in versions.iter_mut().flat_map(|s| s.versions.iter_mut()) {
        for e in &mut v.effects {
            e.id = Uuid::now_v7();
        }
    }
}

/// A copy of `versions` with fresh ids and every stored key moved by `delta`,
/// for a clip made from another: a duplicate, or the later piece of a cut,
/// whose keys count from its own start.
#[must_use]
pub fn respawned(
    versions: &Option<GradeVersions>,
    delta: crate::time::Rational,
) -> Option<GradeVersions> {
    let mut copy = versions.clone();
    respawn(&mut copy);
    for v in copy.iter_mut().flat_map(|s| s.versions.iter_mut()) {
        crate::preset::shift_keys(&mut v.effects, delta);
    }
    copy
}

/// Take the grade out of `stack`, and say where it started: where the first
/// grade effect was, or with none, where a group's Post sits, or the end.
fn lift(stack: &mut Vec<EffectInstance>) -> (Vec<EffectInstance>, usize) {
    let at = stack
        .iter()
        .position(is_grade)
        .or_else(|| {
            stack
                .iter()
                .position(|e| is_reference(e) && part_of(e) == Part::Post)
        })
        .unwrap_or(stack.len());
    let (grade, rest) = std::mem::take(stack).into_iter().partition(is_grade);
    *stack = rest;
    (grade, at)
}

/// Make `to` the live version: the live grade leaves the stack into its own
/// version, and `to`'s goes in where it started.
fn swap(
    stack: &mut Vec<EffectInstance>,
    set: &mut GradeVersions,
    to: Uuid,
) -> Result<(), GradeError> {
    let target = set
        .versions
        .iter()
        .position(|v| v.id == to)
        .ok_or(GradeError::UnknownVersion)?;
    if to == set.live {
        return Ok(());
    }
    let (grade, at) = lift(stack);
    let incoming = std::mem::take(&mut set.versions[target].effects);
    let live = set.live;
    match set.versions.iter_mut().find(|v| v.id == live) {
        Some(v) => v.effects = grade,
        // A file that names a live version it doesn't have still keeps the
        // grade that was showing.
        None => {
            let name = set.fresh_name();
            set.versions.push(GradeVersion {
                id: live,
                name,
                effects: grade,
            });
        }
    }
    let at = at.min(stack.len());
    stack.splice(at..at, incoming);
    set.live = to;
    Ok(())
}

/// Add a version to `shot` and make it the live one: a copy of the live
/// grade, or with `copy` off an empty grade. A shot with no versions yet gets
/// its first, holding the grade it has. Answers the new version's id.
pub fn add(doc: &Document, shot: Shot, copy: bool) -> Result<(Uuid, Option<Op>), GradeError> {
    edit(doc, shot, |stack, versions| {
        let set = versions.get_or_insert_with(|| {
            let id = Uuid::now_v7();
            GradeVersions {
                live: id,
                versions: vec![GradeVersion {
                    id,
                    name: "Version 1".to_owned(),
                    effects: Vec::new(),
                }],
            }
        });
        let id = Uuid::now_v7();
        let name = set.fresh_name();
        set.versions.push(GradeVersion {
            id,
            name,
            effects: Vec::new(),
        });
        if copy {
            // The stack is left as it is and the version it came from keeps
            // the copy, so the picture and the effects showing don't change.
            let kept: Vec<EffectInstance> = stack
                .iter()
                .filter(|e| is_grade(e))
                .map(|e| EffectInstance {
                    id: Uuid::now_v7(),
                    ..e.clone()
                })
                .collect();
            let live = set.live;
            if let Some(v) = set.versions.iter_mut().find(|v| v.id == live) {
                v.effects = kept;
            }
            set.live = id;
        } else {
            swap(stack, set, id)?;
        }
        Ok(id)
    })
}

/// Make `to` the live version of `shot`.
pub fn switch(doc: &Document, shot: Shot, to: Uuid) -> Result<Option<Op>, GradeError> {
    edit(doc, shot, |stack, versions| {
        swap(
            stack,
            versions.as_mut().ok_or(GradeError::UnknownVersion)?,
            to,
        )
    })
    .map(|((), op)| op)
}

/// Call version `id` of `shot` `name`.
pub fn rename(doc: &Document, shot: Shot, id: Uuid, name: &str) -> Result<Option<Op>, GradeError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(GradeError::EmptyName);
    }
    edit(doc, shot, |_, versions| {
        let v = versions
            .iter_mut()
            .flat_map(|s| s.versions.iter_mut())
            .find(|v| v.id == id)
            .ok_or(GradeError::UnknownVersion)?;
        v.name = name.to_owned();
        Ok(())
    })
    .map(|((), op)| op)
}

/// Delete version `id` of `shot`. Deleting the live one shows the version
/// before it (or after, for the first) in its place, in the same step. The
/// last one going leaves the shot with no versions: its grade stays in the
/// stack.
pub fn delete(doc: &Document, shot: Shot, id: Uuid) -> Result<Option<Op>, GradeError> {
    edit(doc, shot, |stack, versions| {
        let set = versions.as_mut().ok_or(GradeError::UnknownVersion)?;
        let index = set
            .versions
            .iter()
            .position(|v| v.id == id)
            .ok_or(GradeError::UnknownVersion)?;
        if id == set.live {
            if set.versions.len() == 1 {
                *versions = None;
                return Ok(());
            }
            let next = set.versions[if index > 0 { index - 1 } else { 1 }].id;
            swap(stack, set, next)?;
        }
        set.versions.retain(|v| v.id != id);
        Ok(())
    })
    .map(|((), op)| op)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::anim::{Animation, Keyframe, SideInterp};
    use crate::fx::effects::grade_group::reference;
    use crate::model::{
        Composition, EffectValue, LayerKind, LinearColour, MotionBlur, ProjectItem, TransformGroup,
    };
    use crate::time::{Duration, FrameRate, Rational};

    fn fx(name: &str) -> EffectInstance {
        crate::fx::builtins::instantiate(name).unwrap()
    }

    fn stack_of(doc: &Document, shot: Shot) -> Vec<EffectInstance> {
        crate::grade::read(doc, shot).unwrap().0.to_vec()
    }

    /// A shot's grade goes into a new empty version and comes back from it
    /// exactly as it was, keys and all, while the effects around it and a
    /// group's references stay where they are.
    #[test]
    fn switching_versions_and_back_loses_nothing() {
        let mut exposure = fx("exposure");
        let key = |t: i64, value: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let keyed = exposure
            .params
            .iter_mut()
            .find_map(|p| match &mut p.value {
                EffectValue::Float(prop) => Some(prop),
                _ => None,
            })
            .unwrap();
        keyed.animation = Animation::Keyframed(vec![key(0, 0.5), key(2, -1.0)]);
        let group = Uuid::now_v7();
        let original = vec![
            fx("blur"),
            reference(group, Part::Pre).unwrap(),
            exposure,
            fx("saturation"),
            reference(group, Part::Post).unwrap(),
            fx("glow"),
        ];
        let mut layer = crate::edits::base_layer(
            "plate".into(),
            LayerKind::Adjustment,
            Rational::new(4, 1).unwrap(),
            TransformGroup::default(),
        );
        layer.effects = original.clone();
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Comp".into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(25, 1).unwrap(),
            duration: Duration(Rational::new(4, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer.clone()],
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
        let shot = Shot {
            comp: comp.id,
            layer: layer.id,
            clip: None,
        };
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));

        let (second, op) = add(&doc, shot, false).unwrap();
        crate::ops::apply(&mut doc, &op.unwrap()).unwrap();
        let names: Vec<String> = stack_of(&doc, shot)
            .iter()
            .map(|e| e.effect.match_name.clone())
            .collect();
        assert_eq!(
            names,
            ["blur", "grade_group", "grade_group", "glow"],
            "the grade left and the rest stayed"
        );

        let first = crate::grade::read(&doc, shot).unwrap().1.unwrap().versions[0].id;
        let op = switch(&doc, shot, first).unwrap().unwrap();
        crate::ops::apply(&mut doc, &op).unwrap();
        assert_eq!(stack_of(&doc, shot), original, "every effect and key back");
        let set = crate::grade::read(&doc, shot).unwrap().1.unwrap().clone();
        assert_eq!(set.live, first);
        assert!(
            set.versions.iter().all(|v| v.effects.is_empty()),
            "nothing stored twice: the live grade is the stack's, the other is empty"
        );
        assert!(set.versions.iter().any(|v| v.id == second));
    }
}
