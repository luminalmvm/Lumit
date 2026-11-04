use std::path::{Path, PathBuf};

use lumit_core::edits;
use lumit_core::model::{Layer, LayerKind, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::time::{CompTime, FrameRate, Rational};
use serde::Deserialize;
use serde_json::{json, Value};

use super::items::find_folder;
use super::keys::markers_json;
use super::{bad, commit, comp_json, find_comp, find_layer, layer_json, present, NoArgs, Time};
use crate::reach::{self, Reach};
use crate::{Host, ScriptError};

pub(super) fn comp_list(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comps: Vec<Value> = doc
        .items
        .iter()
        .filter_map(|item| match item {
            ProjectItem::Composition(c) => Some(comp_json(c)),
            _ => None,
        })
        .collect();
    Ok(json!({ "comps": comps }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompArgs {
    comp: String,
}

pub(super) fn comp_get(host: &dyn Host, a: CompArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let mut answer = comp_json(comp);
    answer["layers"] = comp.layers.iter().map(|l| layer_json(comp, l)).collect();
    answer["markers"] = markers_json(&comp.markers, comp.frame_rate, Rational::ZERO);
    answer["comment"] = json!(doc.item_comments.get(&comp.id).map_or("", String::as_str));
    Ok(answer)
}

/// A frame rate on the wire: whole frames a second, or an exact fraction.
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
pub(super) enum Rate {
    Whole(u32),
    Exact { num: u32, den: u32 },
}

impl Rate {
    pub(super) fn checked(self) -> Result<FrameRate, ScriptError> {
        let (num, den) = match self {
            Rate::Whole(fps) => (fps, 1),
            Rate::Exact { num, den } => (num, den),
        };
        // After Effects' range, which is plenty for anything a comp shows.
        let fps = (u64::from(num), u64::from(den));
        if fps.0 < fps.1 || fps.0 > 999 * fps.1 {
            return Err(bad("A frame rate is 1 to 999 frames a second."));
        }
        FrameRate::new(num, den).map_err(|_| bad("That frame rate is nought."))
    }
}

/// What `comp.new` and `comp.set` can change. `comp` names the comp to change
/// and is left out for a new one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompSettingsArgs {
    comp: Option<String>,
    name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    frame_rate: Option<Rate>,
    duration: Option<Time>,
    background: Option<[f32; 4]>,
    #[serde(default, deserialize_with = "present")]
    folder: Option<Option<String>>,
}

impl CompSettingsArgs {
    /// `base` with what these arguments change. A duration given in frames is
    /// counted on the rate the comp will have.
    fn over(self, base: edits::CompSettings) -> Result<edits::CompSettings, ScriptError> {
        let frame_rate = match self.frame_rate {
            None => base.frame_rate,
            Some(rate) => rate.checked()?,
        };
        let seconds = match self.duration {
            Some(t) => t.at(frame_rate)?,
            None => base.duration.0,
        };
        // The longest comp a command makes is 3 hours, as After Effects allows.
        if i128::from(seconds.num()) > 3 * 60 * 60 * i128::from(seconds.den()) {
            return Err(bad("A comp is at most 3 hours long."));
        }
        Ok(edits::CompSettings {
            name: self.name.unwrap_or(base.name),
            width: self.width.unwrap_or(base.width),
            height: self.height.unwrap_or(base.height),
            frame_rate,
            duration: edits::comp_duration(frame_rate, seconds)
                .ok_or_else(|| bad("That duration is out of range."))?,
            background: self.background.unwrap_or(base.background),
            ..base
        })
    }
}

pub(super) fn comp_new(host: &dyn Host, mut a: CompSettingsArgs) -> Result<Value, ScriptError> {
    if a.comp.is_some() {
        return Err(bad("comp.new makes a comp, so it takes no comp to change."));
    }
    let store = host.store();
    let doc = store.snapshot();
    let folder = a.folder.take();
    let settings = a.over(edits::CompSettings::default())?;
    let (id, mut ops) = edits::new_comp_ops(&doc, settings, None);
    if let Some(folder) = folder {
        // Filed where asked, so of the ops only the comp itself is kept and
        // the Compositions folder is neither made nor used.
        let folder = folder.map(|key| find_folder(&doc, &key)).transpose()?;
        ops = ops
            .into_iter()
            .filter_map(|op| match op {
                Op::AddItem { item, .. } if matches!(*item, ProjectItem::Composition(_)) => {
                    Some(Op::AddItem {
                        index: doc.items.len(),
                        item,
                    })
                }
                _ => None,
            })
            .collect();
        if let Some(f) = folder.and_then(|f| doc.folder(f)) {
            let mut children = f.children.clone();
            children.push(id);
            ops.push(Op::SetFolderChildren {
                folder: f.id,
                children,
            });
        }
    }
    commit(store, ops)?;
    let doc = store.snapshot();
    let comp = doc
        .comp(id)
        .ok_or_else(|| ScriptError::Failed("The comp was not made.".into()))?;
    Ok(comp_json(comp))
}

pub(super) fn comp_set(host: &dyn Host, a: CompSettingsArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let key = a
        .comp
        .clone()
        .ok_or_else(|| bad("comp.set needs the comp to change."))?;
    if a.folder.is_some() {
        return Err(bad("comp.set does not file a comp. item.set does."));
    }
    let comp = find_comp(&doc, &key)?;
    let base = edits::CompSettings {
        name: comp.name.clone(),
        width: comp.width,
        height: comp.height,
        frame_rate: comp.frame_rate,
        duration: comp.duration,
        background: comp.background.0,
        shutter_angle: comp.motion_blur.shutter_angle,
        motion_blur_samples: comp.motion_blur.samples,
        motion_blur_limit: comp.motion_blur.limit,
    };
    commit(store, edits::comp_settings_ops(comp, a.over(base)?))?;
    let doc = store.snapshot();
    let comp = doc
        .comp(comp.id)
        .ok_or_else(|| ScriptError::Failed("The comp went away.".into()))?;
    Ok(comp_json(comp))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrecomposeArgs {
    comp: String,
    layers: Vec<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    leave_attributes: bool,
}

/// Pre-compose as Layer > Pre-compose does, with the new comp as long as this
/// one, as After Effects' own `precompose` makes it.
pub(super) fn comp_precompose(host: &dyn Host, a: PrecomposeArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layers = a
        .layers
        .iter()
        .map(|key| find_layer(comp, key).map(|l| l.id))
        .collect::<Result<Vec<_>, _>>()?;
    if layers.is_empty() {
        return Err(bad("comp.precompose needs a layer to move."));
    }
    if a.leave_attributes && layers.len() > 1 {
        return Err(bad("Only a single layer can leave its attributes behind."));
    }
    let length = |layer: &Layer| {
        host.source_length(comp.id, layer)
            .or_else(|| match layer.kind {
                LayerKind::Precomp { comp } => doc.comp(comp).map(|c| c.duration),
                _ => None,
            })
    };
    let (ops, id) = edits::precompose_ops(
        &doc,
        comp,
        edits::Precompose {
            layers: &layers,
            name: a.name,
            leave_attributes: a.leave_attributes,
            adjust_duration: false,
            group: None,
            source_length: &length,
        },
    )?;
    commit(store, ops)?;
    let doc = store.snapshot();
    let comp = doc
        .comp(comp.id)
        .ok_or_else(|| ScriptError::Failed("The comp went away.".into()))?;
    let layer = find_layer(comp, &id.to_string())?;
    let LayerKind::Precomp { comp: made } = layer.kind else {
        return Err(ScriptError::Failed("The new comp was not made.".into()));
    };
    let made = doc
        .comp(made)
        .ok_or_else(|| ScriptError::Failed("The new comp was not made.".into()))?;
    Ok(json!({ "comp": comp_json(made), "layer": layer_json(comp, layer) }))
}

pub(super) fn comp_open(host: &dyn Host, a: CompArgs) -> Result<Value, ScriptError> {
    let comp = find_comp(&host.store().snapshot(), &a.comp)?.id;
    host.open_comp(comp)?;
    Ok(json!({}))
}

pub(super) fn comp_active(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = host.active_comp().and_then(|id| doc.comp(id));
    Ok(json!({ "comp": comp.map(comp_json) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FrameArgs {
    comp: String,
    time: Option<Time>,
    scale: Option<f32>,
    out: Option<PathBuf>,
}

pub(super) fn comp_frame(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: FrameArgs,
) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let rate = comp.frame_rate;
    let at = a.time.unwrap_or(Time::Frame(0)).at(rate)?;
    let frames = rate.frame_at(CompTime(comp.duration.0)).max(1);
    let frame = u64::try_from(rate.frame_at(CompTime(at)))
        .ok()
        .filter(|f| i64::try_from(*f).is_ok_and(|f| f < frames))
        .ok_or_else(|| bad(format!("{} has frames 0 to {}.", comp.name, frames - 1)))?;
    let scale = a.scale.unwrap_or(1.0);
    if !(scale > 0.0 && scale <= 1.0) {
        return Err(bad("The scale is above 0 and at most 1."));
    }
    let out = a.out.as_deref().map(reach::plain).transpose()?;
    let out = match reach {
        Some(reach) => reach.frame(host, out)?,
        None => out.map(|out| {
            let mut out = out.to_path_buf();
            if out.extension().is_none() {
                out.set_extension("png");
            }
            out
        }),
    };
    let out = match out {
        Some(out) => out,
        None => frame_path(host, reach, &comp.name, frame)?,
    };
    let (width, height) = host.render_frame(comp.id, frame, scale, &out)?;
    Ok(json!({ "path": out, "width": width, "height": height }))
}

/// A new file for frame `frame` of comp `name`, numbered, beside the project
/// or else in the script's folder.
fn frame_path(
    host: &dyn Host,
    reach: Option<&Reach>,
    name: &str,
    frame: u64,
) -> Result<PathBuf, ScriptError> {
    let dir = host
        .path()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| reach.and_then(|r| r.folder.clone()))
        .ok_or_else(|| bad("The project has not been saved yet, so the frame needs a path."))?;
    // A comp's name may hold anything, a separator included, and the file
    // must stay in `dir`.
    let safe: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let safe = safe.trim().trim_end_matches('.');
    let safe = if safe.is_empty() { "Frame" } else { safe };
    let stem = format!("{safe}_{frame:05}");
    let mut path = dir.join(format!("{stem}.png"));
    let mut n = 2;
    while path.symlink_metadata().is_ok() {
        path = dir.join(format!("{stem}_{n}.png"));
        n += 1;
    }
    Ok(path)
}
