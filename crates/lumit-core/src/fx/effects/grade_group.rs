//! Group grade: where a grade group's pre-grade or post-grade runs in a
//! member shot's stack ([`crate::grade::groups`]).
//!
//! **In plain terms.** A grade group is a set of shots sharing a grade laid on
//! before theirs and one laid on after. Each member carries two of these: a
//! Pre at the start of its own grade and a Post after it. The render puts the
//! group's effects where each one stands, so the group's grade is edited once
//! and every member picks it up.
//!
//! Which group is kept in `extra`, as the Node graph effect keeps the comp it
//! names, because a group is not a value anybody keyframes. The effect draws
//! nothing of its own: one that names no group, or a group that has gone, is
//! simply passed over.

use crate::fx::{registry::EffectDef, registry::EffectMetadata, schema::EffectSchema};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// The match name.
pub const GRADE_GROUP: &str = "grade_group";

/// The `extra` key the group's id lives under.
pub const EXTRA_KEY: &str = "grade_group";

/// Group grade's one row.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "grade_group",
    label = "Group grade",
    version = 1,
    category = Colour,
    cost = Trivial,
    roi = Exact,
    // No picture of its own, so no matte to dissolve by.
    matte = false,
)]
pub struct GradeGroup {
    /// The group's grade that runs here: the one before the shot's own, the
    /// one after, or a shared node.
    #[choice(options = ["Pre", "Post", "Shared"], default = 0)]
    pub part: u32,
}

/// Which of a group's two grades a reference stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Pre,
    Post,
}

/// Whether `inst` is a group's reference.
#[must_use]
pub fn is_reference(inst: &EffectInstance) -> bool {
    inst.effect.match_name == GRADE_GROUP
}

/// The group `inst` names, or `None` for one that names none.
#[must_use]
pub fn group_of(inst: &EffectInstance) -> Option<Uuid> {
    if !is_reference(inst) {
        return None;
    }
    Uuid::parse_str(inst.extra.get(EXTRA_KEY)?.as_str()?).ok()
}

/// Whether `inst` stands for a shared node ([`crate::grade::shared`]) rather
/// than a group's grade.
#[must_use]
pub fn is_shared(inst: &EffectInstance) -> bool {
    is_reference(inst) && matches!(inst.param("part"), Some(EffectValue::Choice(2)))
}

/// Which grade `inst` stands for. Anything but Post reads as Pre, so a shared
/// node is its group's Pre list.
#[must_use]
pub fn part_of(inst: &EffectInstance) -> Part {
    match inst.param("part") {
        Some(EffectValue::Choice(1)) => Part::Post,
        _ => Part::Pre,
    }
}

/// A fresh reference to `group`'s `part`.
#[must_use]
pub fn reference(group: Uuid, part: Part) -> Option<EffectInstance> {
    referring(group, u32::from(part == Part::Post))
}

/// A fresh reference to shared node `node`.
#[must_use]
pub fn shared_reference(node: Uuid) -> Option<EffectInstance> {
    referring(node, 2)
}

fn referring(group: Uuid, part: u32) -> Option<EffectInstance> {
    let mut inst = crate::fx::builtins::instantiate(GRADE_GROUP)?;
    for p in &mut inst.params {
        if p.id == "part" {
            p.value = EffectValue::Choice(part);
        }
    }
    inst.extra.insert(
        EXTRA_KEY.to_owned(),
        serde_json::Value::String(group.to_string()),
    );
    Some(inst)
}

/// Group grade's behaviour: none of its own.
pub struct GradeGroupDef;

impl EffectDef for GradeGroupDef {
    fn schema(&self) -> &'static EffectSchema {
        &<GradeGroup as EffectMetadata>::SCHEMA
    }

    /// The render puts the group's effects in its place before the stack is
    /// resolved, so one that reaches the stack is a reference to nothing.
    fn is_image_op(&self) -> bool {
        false
    }
}
