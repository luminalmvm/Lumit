//! A shot's grade versions and its grade group: reading them, switching the
//! version showing, and moving the shot between groups.

use lumit_core::grade::{groups, versions, GradeError, Shot};
use lumit_core::model::{Document, LayerKind};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{commit, find_comp, find_layer, present, NoArgs};
use crate::{Host, ScriptError};

/// The reason as a sentence, as an op's is.
fn refused(e: GradeError) -> ScriptError {
    match e {
        GradeError::Op(e) => e.into(),
        e => {
            let reason = e.to_string();
            let mut letters = reason.chars();
            let first = letters.next().map(|c| c.to_uppercase().collect::<String>());
            ScriptError::Failed(format!(
                "{}{}.",
                first.unwrap_or_default(),
                letters.as_str()
            ))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ShotArgs {
    comp: String,
    layer: String,
    clip: Option<String>,
}

/// The shot the arguments name, checked against `doc`.
fn shot_of(
    doc: &Document,
    comp: &str,
    layer: &str,
    clip: Option<&str>,
) -> Result<Shot, ScriptError> {
    let c = find_comp(doc, comp)?;
    let l = find_layer(c, layer)?;
    let clip = match clip {
        None => None,
        Some(key) => {
            let id = Uuid::try_parse(key).ok();
            match &l.kind {
                LayerKind::Sequence { clips } if clips.iter().any(|c| Some(c.id) == id) => id,
                _ => {
                    return Err(ScriptError::NotFound(format!(
                        "There is no clip {key} on {}.",
                        l.name
                    )))
                }
            }
        }
    };
    Ok(Shot {
        comp: c.id,
        layer: l.id,
        clip,
    })
}

pub(super) fn get(host: &dyn Host, a: ShotArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let shot = shot_of(&doc, &a.comp, &a.layer, a.clip.as_deref())?;
    let (stack, set) = lumit_core::grade::read(&doc, shot).map_err(refused)?;
    let versions: Vec<Value> = set.map_or_else(Vec::new, |set| {
        set.versions
            .iter()
            .map(|v| json!({ "id": v.id, "name": v.name, "live": v.id == set.live }))
            .collect()
    });
    Ok(json!({ "versions": versions, "group": groups::group_in(stack) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetArgs {
    comp: String,
    layer: String,
    clip: Option<String>,
    version: Option<String>,
    #[serde(default, deserialize_with = "present")]
    group: Option<Option<String>>,
}

pub(super) fn set(host: &dyn Host, a: SetArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let shot = shot_of(&doc, &a.comp, &a.layer, a.clip.as_deref())?;
    let mut scratch = (*doc).clone();
    let mut ops = Vec::new();
    if let Some(key) = &a.version {
        let (_, set) = lumit_core::grade::read(&scratch, shot).map_err(refused)?;
        let id = set
            .and_then(|set| {
                set.versions
                    .iter()
                    .find(|v| Some(v.id) == Uuid::try_parse(key).ok() || v.name == *key)
            })
            .map(|v| v.id)
            .ok_or_else(|| {
                ScriptError::NotFound(format!("There is no version called \"{key}\"."))
            })?;
        if let Some(op) = versions::switch(&scratch, shot, id).map_err(refused)? {
            lumit_core::ops::apply(&mut scratch, &op)?;
            ops.push(op);
        }
    }
    if let Some(group) = &a.group {
        let group = match group {
            None => None,
            Some(key) => Some(
                scratch
                    .grade_groups
                    .iter()
                    .find(|g| Some(g.id) == Uuid::try_parse(key).ok() || g.name == *key)
                    .map(|g| g.id)
                    .ok_or_else(|| {
                        ScriptError::NotFound(format!("There is no grade group called \"{key}\"."))
                    })?,
            ),
        };
        if let Some(op) = groups::set_group(&scratch, shot, group).map_err(refused)? {
            ops.push(op);
        }
    }
    commit(store, ops)?;
    get(
        host,
        ShotArgs {
            comp: shot.comp.to_string(),
            layer: shot.layer.to_string(),
            clip: shot.clip.map(|c| c.to_string()),
        },
    )
}

pub(super) fn group_list(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    Ok(doc
        .grade_groups
        .iter()
        .filter(|g| !g.shared)
        .map(
            |g| json!({ "id": g.id, "name": g.name, "members": groups::members(&doc, g.id).len() }),
        )
        .collect())
}
