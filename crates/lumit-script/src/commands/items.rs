use std::path::{Path, PathBuf};

use lumit_core::model::{Document, ProjectItem};
use lumit_core::ops::Op;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::comps::Rate;
use super::{bad, commit, exact, find_comp, present, NoArgs};
use crate::reach::{self, Reach};
use crate::{Host, ScriptError};

fn find_item<'a>(doc: &'a Document, key: &str) -> Result<&'a ProjectItem, ScriptError> {
    let id = Uuid::try_parse(key).ok();
    doc.items
        .iter()
        .find(|i| Some(i.id()) == id)
        .or_else(|| doc.items.iter().find(|i| i.name() == key))
        .ok_or_else(|| {
            ScriptError::NotFound(format!("There is nothing called \"{key}\" in the project."))
        })
}

pub(super) fn find_folder(doc: &Document, key: &str) -> Result<Uuid, ScriptError> {
    match find_item(doc, key)? {
        ProjectItem::Folder(f) => Ok(f.id),
        _ => Err(bad(format!("\"{key}\" is not a folder."))),
    }
}

/// The folder `id` is filed in, if any.
fn folder_of(doc: &Document, id: Uuid) -> Option<Uuid> {
    doc.items.iter().find_map(|i| match i {
        ProjectItem::Folder(f) if f.children.contains(&id) => Some(f.id),
        _ => None,
    })
}

fn item_json(host: &dyn Host, doc: &Document, item: &ProjectItem) -> Value {
    let kind = match item {
        ProjectItem::Footage(_) => "footage",
        ProjectItem::Folder(_) => "folder",
        ProjectItem::Composition(_) => "comp",
        ProjectItem::Solid(_) => "solid",
    };
    let mut answer = json!({
        "id": item.id(),
        "name": item.name(),
        "kind": kind,
        "folder": folder_of(doc, item.id()),
        "comment": doc.item_comments.get(&item.id()).map_or("", String::as_str),
    });
    if let ProjectItem::Footage(f) = item {
        let rate = f
            .sequence
            .as_ref()
            .map(|s| s.frame_rate)
            .or(f.interpretation.frame_rate);
        answer["sequence"] = json!(f.sequence.is_some());
        answer["frame_rate"] = json!(rate.map(|r| json!({ "num": r.num(), "den": r.den() })));
        answer["path"] = json!(f.media.absolute_path);
        // The file as it is read, or null when it will not read.
        answer["media"] = json!(host.footage_info(f.id).map(|m| json!({
            "width": m.width,
            "height": m.height,
            "frame_rate": m.frame_rate.map(|r| json!({ "num": r.num(), "den": r.den() })),
            "duration": m.duration.map(|d| exact(d.0)),
        })));
    }
    if let ProjectItem::Solid(s) = item {
        // A still of its own size, as After Effects has a solid.
        answer["media"] = json!({
            "width": s.width,
            "height": s.height,
            "frame_rate": null,
            "duration": null,
        });
    }
    answer
}

pub(super) fn item_list(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let items: Vec<Value> = doc.items.iter().map(|i| item_json(host, &doc, i)).collect();
    Ok(json!({ "items": items }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ItemSetArgs {
    item: String,
    name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    folder: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    frame_rate: Option<Option<Rate>>,
    comment: Option<String>,
}

pub(super) fn item_set(host: &dyn Host, a: ItemSetArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let item = find_item(&doc, &a.item)?;
    let id = item.id();
    let mut ops = Vec::new();
    if let Some(name) = a.name {
        ops.push(Op::RenameItem { id, name });
    }
    if let Some(comment) = a.comment {
        ops.push(Op::SetItemComment { id, comment });
    }
    if let Some(rate) = a.frame_rate {
        let ProjectItem::Footage(f) = item else {
            return Err(bad("Only footage has a frame rate of its own."));
        };
        let rate = rate.map(Rate::checked).transpose()?;
        // A run of stills keeps its rate on the run, a video on how it is read.
        if f.sequence.is_some() {
            ops.push(Op::SetSequenceRate {
                id,
                frame_rate: rate
                    .ok_or_else(|| bad("A run of stills has no rate of its own to go back to."))?,
            });
        } else {
            let mut interpretation = f.interpretation.clone();
            interpretation.frame_rate = rate;
            ops.push(Op::SetFootageInterpretation {
                id,
                interpretation: Box::new(interpretation),
            });
        }
    }
    match a.folder {
        Some(Some(key)) => {
            let folder = find_folder(&doc, &key)?;
            ops.extend(
                lumit_core::ops::move_to_folder_ops(&doc, id, folder)
                    .ok_or_else(|| bad("A folder cannot go inside itself."))?,
            );
        }
        Some(None) => {
            // Out of whatever folder holds it, to the top of the panel.
            if let Some(ProjectItem::Folder(f)) = folder_of(&doc, id).and_then(|f| doc.item(f)) {
                ops.push(Op::SetFolderChildren {
                    folder: f.id,
                    children: f.children.iter().copied().filter(|c| *c != id).collect(),
                });
            }
        }
        None => {}
    }
    commit(store, ops)?;
    let doc = store.snapshot();
    let item = doc
        .item(id)
        .ok_or_else(|| ScriptError::Failed("The item went away.".into()))?;
    Ok(item_json(host, &doc, item))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FolderArgs {
    #[serde(default)]
    name: String,
    parent: Option<String>,
}

pub(super) fn folder_new(host: &dyn Host, a: FolderArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let parent = a.parent.map(|key| find_folder(&doc, &key)).transpose()?;
    let (id, ops) = lumit_core::ops::new_folder_ops(&doc, &a.name, parent);
    commit(store, ops)?;
    let doc = store.snapshot();
    let item = doc
        .item(id)
        .ok_or_else(|| ScriptError::Failed("The folder was not made.".into()))?;
    Ok(item_json(host, &doc, item))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportArgs {
    paths: Vec<PathBuf>,
}

pub(super) fn footage_import(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: ImportArgs,
) -> Result<Value, ScriptError> {
    let paths = a
        .paths
        .iter()
        .map(|path| match (reach::plain(path)?, reach) {
            (path, Some(reach)) => reach.footage(host, path),
            (path, None) => Ok(path.to_path_buf()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ids = host.import_footage(&paths)?;
    let doc = host.store().snapshot();
    let items: Vec<Value> = ids
        .iter()
        .map(|id| {
            let name = match doc.item(*id) {
                Some(ProjectItem::Footage(f)) => Some(f.name.clone()),
                _ => None,
            };
            json!({ "id": id, "name": name })
        })
        .collect();
    Ok(json!({ "items": items }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportArgs {
    comp: String,
    out: Option<PathBuf>,
}

pub(super) fn export_start(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: ExportArgs,
) -> Result<Value, ScriptError> {
    let comp = find_comp(&host.store().snapshot(), &a.comp)?.id;
    let out = a.out.as_deref().map(reach::plain).transpose()?;
    let out = match reach {
        Some(reach) => reach.export(host, out)?,
        None => out.map(Path::to_path_buf),
    };
    let job = host.export_start(comp, out.as_deref())?;
    Ok(json!({ "job": job }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LottieArgs {
    path: PathBuf,
}

pub(super) fn lottie_import(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: LottieArgs,
) -> Result<Value, ScriptError> {
    let path = match (reach::plain(&a.path)?, reach) {
        (path, Some(reach)) => reach.lottie(host, path)?,
        (path, None) => path.to_path_buf(),
    };
    let (comp, left_out) = host.import_lottie(&path)?;
    let doc = host.store().snapshot();
    let name = doc.comp(comp).map(|c| c.name.clone());
    Ok(json!({ "comp": comp, "name": name, "left_out": left_out }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VectorArgs {
    comp: String,
    out: PathBuf,
    format: Option<String>,
    text: Option<String>,
    minify: Option<bool>,
}

pub(super) fn export_vector(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: VectorArgs,
) -> Result<Value, ScriptError> {
    use lumit_lottie::settings::{Format, Settings, TextMode};
    let comp = find_comp(&host.store().snapshot(), &a.comp)?.id;
    let by_name = a.out.extension().and_then(|e| e.to_str()).and_then(|e| {
        match e.to_ascii_lowercase().as_str() {
            "json" => Some(Format::Lottie),
            "lottie" => Some(Format::DotLottie),
            "svg" => Some(Format::Svg),
            _ => None,
        }
    });
    let format = match a.format.as_deref() {
        Some(key) => Format::from_key(key)
            .ok_or_else(|| bad(format!("There is no format called \"{key}\".")))?,
        None => by_name
            .ok_or_else(|| bad("Name the format, or end the file in .json, .lottie or .svg."))?,
    };
    let out = reach::plain(&a.out)?;
    let out = match reach {
        Some(reach) => reach.vector(host, out, format.extension())?,
        None => out.to_path_buf(),
    };
    let text = match a.text.as_deref() {
        None | Some("shapes") => TextMode::Shapes,
        Some("text") => TextMode::Text,
        Some(other) => return Err(bad(format!("text is shapes or text, not {other}."))),
    };
    let settings = Settings {
        format,
        text,
        minify: a.minify.unwrap_or(false),
        file_stem: out
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "animation".into()),
        ..Settings::default()
    };
    let left_out = host.export_vector(comp, &out, &settings)?;
    Ok(json!({ "file": out, "left_out": left_out }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JobArgs {
    job: u64,
}

pub(super) fn export_status(host: &dyn Host, a: JobArgs) -> Result<Value, ScriptError> {
    serde_json::to_value(host.export_status(a.job)?).map_err(|e| ScriptError::Failed(e.to_string()))
}

pub(super) fn export_cancel(host: &dyn Host, a: JobArgs) -> Result<Value, ScriptError> {
    host.export_cancel(a.job)?;
    Ok(json!({}))
}
