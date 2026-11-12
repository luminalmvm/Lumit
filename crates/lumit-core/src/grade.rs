//! A shot's grade as a colourist keeps it: [`versions`] of it, and [`groups`]
//! of shots that share a grade laid on before and after their own.
//!
//! A shot is a picture layer, or one clip on a Sequence layer, and its grade is
//! the Colour effects in its own stack, in order, less any group's references.
//! The same definition Copy grade and Paste grade use.
//!
//! Runs on whichever thread edits the document. Nothing here renders.

pub mod groups;
pub mod matte_view;
pub mod memories;
pub mod shared;
pub mod versions;

use uuid::Uuid;

use crate::model::{Document, EffectInstance, LayerKind};
use crate::ops::{Op, OpError};
use versions::GradeVersions;

/// A shot: a layer, or one clip on a Sequence layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shot {
    pub comp: Uuid,
    pub layer: Uuid,
    pub clip: Option<Uuid>,
}

/// Why a grade edit was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GradeError {
    #[error("no such layer or clip")]
    UnknownShot,
    #[error("no such version")]
    UnknownVersion,
    #[error("no such grade group")]
    UnknownGroup,
    #[error("no such node")]
    UnknownNode,
    #[error("a name can't be blank")]
    EmptyName,
    #[error(transparent)]
    Op(#[from] OpError),
}

/// Whether `e` is part of a grade, the one definition versions, copied and
/// pasted grades and looks all keep to ([`is_grade_name`]).
#[must_use]
pub fn is_grade(e: &EffectInstance) -> bool {
    is_grade_name(&e.effect.match_name)
}

/// Whether an effect called `match_name` is part of a grade: a Colour effect
/// that is not a group's reference. Being in a group belongs to the shot, so a
/// version, a copied grade or a look never carries it.
#[must_use]
pub fn is_grade_name(match_name: &str) -> bool {
    match_name != crate::fx::effects::grade_group::GRADE_GROUP
        && crate::fx::builtins::schema(match_name)
            .is_some_and(|s| s.category == crate::fx::schema::FxCategory::Colour)
}

/// `shot`'s own stack and stored versions, as the document holds them.
pub fn read(
    doc: &Document,
    shot: Shot,
) -> Result<(&[EffectInstance], Option<&GradeVersions>), GradeError> {
    let layer = doc
        .comp(shot.comp)
        .and_then(|c| c.layers.iter().find(|l| l.id == shot.layer))
        .ok_or(GradeError::UnknownShot)?;
    let Some(clip) = shot.clip else {
        return Ok((&layer.effects, layer.grade_versions.as_ref()));
    };
    let LayerKind::Sequence { clips } = &layer.kind else {
        return Err(GradeError::UnknownShot);
    };
    let clip = clips
        .iter()
        .find(|c| c.id == clip)
        .ok_or(GradeError::UnknownShot)?;
    Ok((&clip.effects, clip.grade_versions.as_ref()))
}

/// The op that leaves `shot` with `effects` and `versions`, or `None` when it
/// already has them. A clip rides the whole clip list, as every clip edit does.
pub fn write(
    doc: &Document,
    shot: Shot,
    effects: Vec<EffectInstance>,
    versions: Option<GradeVersions>,
) -> Result<Option<Op>, GradeError> {
    let layer = doc
        .comp(shot.comp)
        .and_then(|c| c.layers.iter().find(|l| l.id == shot.layer))
        .ok_or(GradeError::UnknownShot)?;
    if let Some(clip) = shot.clip {
        let LayerKind::Sequence { clips } = &layer.kind else {
            return Err(GradeError::UnknownShot);
        };
        let mut clips = clips.clone();
        let c = clips
            .iter_mut()
            .find(|c| c.id == clip)
            .ok_or(GradeError::UnknownShot)?;
        if c.effects == effects && c.grade_versions == versions {
            return Ok(None);
        }
        c.effects = effects;
        c.grade_versions = versions;
        return Ok(Some(Op::SetSequenceClips {
            comp: shot.comp,
            layer: shot.layer,
            clips,
        }));
    }
    let mut ops = Vec::new();
    if layer.effects != effects {
        ops.push(Op::SetLayerEffects {
            comp: shot.comp,
            layer: shot.layer,
            effects,
        });
    }
    if layer.grade_versions != versions {
        ops.push(Op::SetLayerGradeVersions {
            comp: shot.comp,
            layer: shot.layer,
            versions: versions.map(Box::new),
        });
    }
    Ok(match ops.len() {
        0 => None,
        1 => ops.pop(),
        _ => Some(Op::Batch { ops }),
    })
}

/// Read `shot`, let `edit` change copies of its stack and versions, and
/// answer what `edit` answered with the op that writes the change.
pub fn edit<T>(
    doc: &Document,
    shot: Shot,
    edit: impl FnOnce(&mut Vec<EffectInstance>, &mut Option<GradeVersions>) -> Result<T, GradeError>,
) -> Result<(T, Option<Op>), GradeError> {
    let (effects, versions) = read(doc, shot)?;
    let (mut effects, mut versions) = (effects.to_vec(), versions.cloned());
    let answer = edit(&mut effects, &mut versions)?;
    Ok((answer, write(doc, shot, effects, versions)?))
}

/// Several edits made one after another on a copy of the document, so each is
/// made against what the ones before it left: two clips on one row both keep
/// their change. Then one op for the lot.
struct Chain {
    scratch: Document,
    ops: Vec<Op>,
}

impl Chain {
    fn new(doc: &Document) -> Self {
        Self {
            scratch: doc.clone(),
            ops: Vec::new(),
        }
    }

    fn push(&mut self, op: Option<Op>) -> Result<(), GradeError> {
        if let Some(op) = op {
            crate::ops::apply(&mut self.scratch, &op)?;
            self.ops.push(op);
        }
        Ok(())
    }

    fn finish(mut self) -> Option<Op> {
        match self.ops.len() {
            0 => None,
            1 => self.ops.pop(),
            _ => Some(Op::Batch { ops: self.ops }),
        }
    }
}
