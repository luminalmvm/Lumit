//! Graphics templates: a comp's controls, what a copy has set for itself,
//! the data that feeds them, and the library.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lumit_core::edits;
use lumit_core::fx::effects::template as values;
use lumit_core::model::{Composition, Document, EffectInstance, Layer, LayerKind, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::sequence::ClipSource;
use lumit_core::template::data::{self, Table};
use lumit_core::template::feed::{self, Data, Feeds};
use lumit_core::template::version::{self, Host as CopyAt};
use lumit_core::template::Value as ControlValue;
use lumit_core::template::{author, rows, Control, ControlKind, Fit, Picture, Setting, Template};
use lumit_core::time::CompTime;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::layers::holds;
use super::{bad, commit, comp_json, find_comp, find_layer, layer_now, present, NoArgs, Time};
use crate::reach::{self, Reach};
use crate::{Host, ScriptError};

/// The most bytes of a data file that are read, as Lumit's own Data window
/// reads one.
const DATA_FILE_MAX_BYTES: u64 = 64 << 20;
/// What a row's file is called when nobody says.
const ROW_NAME: &str = "{comp} {row}";

/// The comp called `key` and its controls. A comp with none is refused.
fn find_template<'a>(
    doc: &'a Document,
    key: &str,
) -> Result<(&'a Composition, Arc<Template>), ScriptError> {
    let comp = find_comp(doc, key)?;
    let template = comp
        .template()
        .ok_or_else(|| bad(format!("{} has no controls.", comp.name)))?;
    Ok((comp, template))
}

/// The control of `comp` called `key`, by its id or else its label.
fn find_control<'a>(
    template: &'a Template,
    comp: &Composition,
    key: &str,
) -> Result<&'a Control, ScriptError> {
    template
        .control(key)
        .or_else(|| template.controls.iter().find(|c| c.label == key))
        .ok_or_else(|| {
            ScriptError::NotFound(format!(
                "There is no control called \"{key}\" in {}.",
                comp.name
            ))
        })
}

/// The footage item or the comp called `key` as a picture, with its size.
fn picture(host: &dyn Host, doc: &Document, key: &str) -> Result<Picture, ScriptError> {
    let id = Uuid::try_parse(key).ok();
    let shows =
        |i: &&ProjectItem| matches!(i, ProjectItem::Footage(_) | ProjectItem::Composition(_));
    let item = doc
        .items
        .iter()
        .filter(shows)
        .find(|i| Some(i.id()) == id)
        .or_else(|| doc.items.iter().filter(shows).find(|i| i.name() == key))
        .ok_or_else(|| {
            ScriptError::NotFound(format!("There is no footage or comp called \"{key}\"."))
        })?;
    let size = match item {
        ProjectItem::Composition(c) => Some((c.width, c.height)),
        _ => host.footage_info(item.id()).map(|f| (f.width, f.height)),
    };
    let size = size
        .filter(|(w, h)| *w > 0 && *h > 0)
        .ok_or_else(|| ScriptError::Failed(format!("{} could not be read.", item.name())))?;
    Ok(Picture {
        item: item.id(),
        size,
        fit: Fit::default(),
    })
}

/// `value` as control `control` keeps it.
fn value_in(
    host: &dyn Host,
    doc: &Document,
    control: &Control,
    value: &Value,
) -> Result<ControlValue, ScriptError> {
    let wrong = |takes: &str| bad(format!("{} takes {takes}.", control.label));
    let numbers: Option<Vec<f64>> = value
        .as_array()
        .and_then(|list| list.iter().map(Value::as_f64).collect());
    Ok(match control.kind {
        ControlKind::Number | ControlKind::Angle => {
            ControlValue::Number(value.as_f64().ok_or_else(|| wrong("a number"))?)
        }
        ControlKind::Point => {
            let axes = usize::from(control.axes.max(2));
            match numbers {
                Some(point) if point.len() == axes => ControlValue::Point(point),
                _ => return Err(wrong(&format!("{axes} numbers"))),
            }
        }
        ControlKind::Colour => {
            let colour = match (value.as_str(), numbers.as_deref()) {
                (Some(hex), _) => feed::row_colour(hex),
                (_, Some([r, g, b])) => Some([*r, *g, *b, 1.0]),
                (_, Some([r, g, b, a])) => Some([*r, *g, *b, *a]),
                _ => None,
            };
            ControlValue::Colour(
                colour.ok_or_else(|| wrong("a colour, as hex text or as [r, g, b, a]"))?,
            )
        }
        ControlKind::Checkbox => {
            ControlValue::Tick(value.as_bool().ok_or_else(|| wrong("true or false"))?)
        }
        ControlKind::Menu => {
            let option = match value.as_str() {
                Some(name) => control
                    .options
                    .iter()
                    .position(|o| o.trim().eq_ignore_ascii_case(name.trim())),
                None => value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n < control.options.len()),
            };
            ControlValue::Choice(
                option
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| wrong("one of its options, by name or by number from 0"))?,
            )
        }
        ControlKind::Text => {
            ControlValue::Text(value.as_str().ok_or_else(|| wrong("text"))?.to_owned())
        }
        ControlKind::Font => {
            let family = value.get("family").and_then(Value::as_str);
            let face = value.get("face").and_then(Value::as_str);
            ControlValue::Font {
                family: family
                    .ok_or_else(|| wrong("a font, as {family, face}"))?
                    .to_owned(),
                face: face.unwrap_or_default().to_owned(),
            }
        }
        ControlKind::Footage => {
            let key = value
                .as_str()
                .ok_or_else(|| wrong("a footage item's or a comp's id or name"))?;
            ControlValue::Picture(picture(host, doc, key)?)
        }
    })
}

/// A control's value in the shape [`value_in`] reads. A menu's option is its
/// name, or its number where the template has no name for it.
fn value_out(control: &Control, value: &ControlValue) -> Value {
    match value {
        ControlValue::Number(v) => json!(v),
        ControlValue::Point(point) => json!(point),
        ControlValue::Colour(colour) => json!(colour),
        ControlValue::Tick(on) => json!(on),
        ControlValue::Choice(n) => control
            .options
            .get(*n as usize)
            .map_or_else(|| json!(n), |name| json!(name)),
        ControlValue::Text(text) => json!(text),
        ControlValue::Font { family, face } => json!({ "family": family, "face": face }),
        ControlValue::Picture(picture) => json!(picture.item),
    }
}

/// The footage item of `doc` that reads `file`, with its size, when the
/// project holds that file and the host can measure it.
fn held_picture(host: &dyn Host, doc: &Document, file: &Path) -> Option<(Uuid, (u32, u32))> {
    let item = doc.items.iter().find_map(|item| match item {
        ProjectItem::Footage(f) if Path::new(&f.media.absolute_path) == file => Some(f.id),
        _ => None,
    })?;
    let info = host.footage_info(item)?;
    Some((item, (info.width, info.height)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ControlsArgs {
    comp: String,
    set: Option<serde_json::Map<String, Value>>,
}

pub(super) fn comp_controls(host: &dyn Host, a: ControlsArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let id = comp.id;
    if let Some(set) = &a.set {
        let template = comp.template().unwrap_or_default();
        // What a menu's option sets goes first, so a control set here wins
        // over a look that sets the same row.
        let mut looks: Vec<Setting> = Vec::new();
        let mut own: Vec<Setting> = Vec::new();
        for (key, value) in set {
            let control = find_control(&template, comp, key)?;
            let value = value_in(host, &doc, control, value)?;
            if let ControlValue::Choice(option) = value {
                looks.extend(
                    control
                        .looks
                        .get(option as usize)
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
            own.push(Setting {
                target: control.target.clone(),
                value,
            });
        }
        looks.extend(own);
        commit(store, author::ops(&doc, id, &looks))?;
    }
    let doc = store.snapshot();
    let comp = doc
        .comp(id)
        .ok_or_else(|| ScriptError::Failed("The comp went away.".into()))?;
    let template = comp.template().unwrap_or_default();
    let controls: Vec<Value> = template
        .controls
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "label": c.label,
                "kind": c.kind,
                "group": c.group,
                "value": rows::read(&doc, comp, &c.target).map(|v| value_out(c, &v)),
                "options": c.options,
                "range": c.range,
            })
        })
        .collect();
    Ok(json!({ "controls": controls }))
}

/// A copy of a graphics template: where it sits, the template it places and
/// what it has stored for itself.
struct Copy {
    at: CopyAt,
    template: Uuid,
    stored: Option<EffectInstance>,
}

/// The copy that `layer` of `comp` is, or that its clip `clip` is. One that
/// places no graphics template is refused.
fn copy_of(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    clip: Option<&str>,
) -> Result<Copy, ScriptError> {
    let (placed, stored, clip) = match (clip, &layer.kind) {
        (None, LayerKind::Precomp { comp }) => (Some(*comp), layer.graph_inputs.clone(), None),
        (None, _) => (None, None, None),
        (Some(key), LayerKind::Sequence { clips }) => {
            let id = Uuid::try_parse(key).ok();
            let clip = clips.iter().find(|c| Some(c.id) == id).ok_or_else(|| {
                ScriptError::NotFound(format!("There is no clip \"{key}\" on {}.", layer.name))
            })?;
            let placed = match clip.source {
                ClipSource::Comp(comp) => Some(comp),
                _ => None,
            };
            (placed, clip.inputs.clone(), Some(clip.id))
        }
        (Some(_), _) => {
            return Err(bad(format!(
                "{} is not a Sequence layer, so it has no clips.",
                layer.name
            )))
        }
    };
    let template = placed
        .filter(|id| doc.comp(*id).is_some_and(Composition::is_template))
        .ok_or_else(|| {
            let what = if clip.is_some() {
                "That clip"
            } else {
                layer.name.as_str()
            };
            bad(format!("{what} places no graphics template."))
        })?;
    Ok(Copy {
        at: CopyAt {
            comp: comp.id,
            layer: layer.id,
            clip,
        },
        template,
        stored: stored.filter(|inst| values::comp_of(inst) == Some(template)),
    })
}

/// The controls that follow the template again.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Reset {
    All(bool),
    Named(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValuesArgs {
    comp: String,
    layer: String,
    clip: Option<String>,
    set: Option<serde_json::Map<String, Value>>,
    reset: Option<Reset>,
    #[serde(default, deserialize_with = "present")]
    row: Option<Option<u32>>,
}

pub(super) fn layer_values(host: &dyn Host, a: ValuesArgs) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let layer = find_layer(comp, &a.layer)?;
    let (comp_id, layer_id) = (comp.id, layer.id);
    if a.set.is_some() || a.reset.is_some() || a.row.is_some() {
        let copy = copy_of(&doc, comp, layer, a.clip.as_deref())?;
        let (inner, template) = find_template(&doc, &copy.template.to_string())?;
        let mut inst = copy
            .stored
            .unwrap_or_else(|| values::instance_for(copy.template, &template));
        values::refresh(&mut inst, &doc);
        let reset: Vec<String> = match &a.reset {
            Some(Reset::All(true)) => template
                .controls
                .iter()
                .flat_map(values::row_ids)
                .chain(
                    lumit_core::template::RESERVED_IDS
                        .iter()
                        .map(|r| (*r).to_owned()),
                )
                .collect(),
            Some(Reset::Named(keys)) => {
                let mut rows = Vec::new();
                for key in keys {
                    rows.extend(values::row_ids(find_control(&template, inner, key)?));
                }
                rows
            }
            _ => Vec::new(),
        };
        values::clear_rows(&mut inst, &reset);
        for (key, value) in a.set.iter().flatten() {
            let control = find_control(&template, inner, key)?;
            let mut value = value_in(host, &doc, control, value)?;
            // A range is a wall on a copy.
            if let (ControlValue::Number(v), Some((min, max))) = (&mut value, control.range) {
                *v = v.clamp(min.min(max), max.max(min));
            }
            values::set_value(&mut inst, control, &value);
        }
        match a.row {
            Some(Some(row)) => {
                let data = inner
                    .template_data()
                    .filter(|d| row >= 1 && row as usize <= d.table.rows.len())
                    .ok_or_else(|| bad(format!("{} has no row {row} of data.", inner.name)))?;
                let pictures = |file: &Path| held_picture(host, &doc, file);
                version::feed_copy(&template, &data, row - 1, &mut inst, &pictures);
            }
            Some(None) => {
                values::unfeed(&mut inst);
                values::follow(&mut inst, None);
            }
            None => {}
        }
        // A copy that has set nothing stores nothing.
        let inputs = (!values::is_bare(&inst)).then_some(inst);
        let op = match (copy.at.clip, &layer.kind) {
            (Some(clip), LayerKind::Sequence { clips }) => {
                let mut clips = clips.clone();
                if let Some(clip) = clips.iter_mut().find(|c| c.id == clip) {
                    clip.inputs = inputs;
                }
                Op::SetSequenceClips {
                    comp: comp_id,
                    layer: layer_id,
                    clips,
                }
            }
            _ => Op::SetLayerGraphInputs {
                comp: comp_id,
                layer: layer_id,
                inputs: inputs.map(Box::new),
            },
        };
        commit(store, vec![op])?;
    }
    let doc = store.snapshot();
    let comp = doc
        .comp(comp_id)
        .ok_or_else(|| ScriptError::Failed("The comp went away.".into()))?;
    let layer = find_layer(comp, &layer_id.to_string())?;
    let copy = copy_of(&doc, comp, layer, a.clip.as_deref())?;
    let (inner, template) = find_template(&doc, &copy.template.to_string())?;
    let kept = copy.stored.as_ref().map(values::held).unwrap_or_default();
    let controls: Vec<Value> = template
        .controls
        .iter()
        .map(|c| {
            let rows = values::row_ids(c);
            let own = copy
                .stored
                .as_ref()
                .and_then(|inst| values::value_of(inst, c, 0.0, None));
            let value = own.or_else(|| rows::read(&doc, inner, &c.target));
            json!({
                "id": c.id,
                "label": c.label,
                "kind": c.kind,
                "value": value.map(|v| value_out(c, &v)),
                "own": rows.iter().any(|r| kept.own.contains(r)),
                "fed": rows.iter().any(|r| kept.fed.contains(r)),
            })
        })
        .collect();
    Ok(json!({ "controls": controls, "row": kept.row.map(|row| row + 1) }))
}

/// The table in the file at `file`: by its name's ending, and for any other
/// ending as CSV.
fn read_table(file: &Path) -> Result<Table, ScriptError> {
    let could_not = |why: String| {
        ScriptError::Failed(format!(
            "{} could not be read as rows: {why}.",
            file.display()
        ))
    };
    let text = lumit_ingress::read_to_string_capped(file, DATA_FILE_MAX_BYTES)
        .map_err(|e| could_not(e.to_string()))?;
    let format = file
        .file_name()
        .and_then(|n| data::Format::of_name(&n.to_string_lossy()))
        .unwrap_or(data::Format::Csv);
    data::parse(&text, format).map_err(|e| could_not(e.to_string()))
}

/// What a column said to feed `what` feeds: a control by its id, or whatever
/// a column of that name would feed.
fn feeds_named(template: &Template, comp: &Composition, what: &str) -> Result<Feeds, ScriptError> {
    if template.control(what).is_some() {
        return Ok(Feeds::Control(what.to_owned()));
    }
    match feed::matched(template, &[what.to_owned()]).pop() {
        Some(Feeds::Nothing) | None => Err(ScriptError::NotFound(format!(
            "There is no control called \"{what}\" in {}.",
            comp.name
        ))),
        Some(feeds) => Ok(feeds),
    }
}

/// Link `data` to template `comp`. The footage its rows name is brought in
/// first, where it may be read, and every copy that follows a row is fed
/// again.
fn link(
    host: &dyn Host,
    reach: Option<&Reach>,
    comp: Uuid,
    template: &Template,
    data: Data,
) -> Result<(), ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let mut wanted: Vec<PathBuf> = Vec::new();
    for row in 0..data.table.rows.len() {
        let files = feed::row_values(template, &data, row).map(|read| read.files);
        for (_, cell) in files.unwrap_or_default() {
            let file = version::cell_file(&data.file, &cell);
            let held = doc.items.iter().any(|item| {
                matches!(item, ProjectItem::Footage(f) if Path::new(&f.media.absolute_path) == file)
            });
            // Asked of the reach before the file system, as any path is.
            let allowed =
                reach.is_none_or(|r| reach::plain(&file).is_ok() && r.footage(host, &file).is_ok());
            if !held && allowed && !wanted.contains(&file) && file.is_file() {
                wanted.push(file);
            }
        }
    }
    for file in wanted {
        // A file that will not import is left out, and its cells feed nothing.
        let _ = host.import_footage(&[file]);
    }
    let doc = store.snapshot();
    let pictures = |file: &Path| held_picture(host, &doc, file);
    let mut ops = version::refeed_ops(&doc, comp, template, Some(&data), &pictures);
    ops.insert(
        0,
        Op::SetCompTemplateData {
            comp,
            data: Some(Box::new(data)),
        },
    );
    commit(store, ops)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RowsArgs {
    comp: String,
    file: Option<PathBuf>,
    feeds: Option<BTreeMap<String, Option<String>>>,
}

pub(super) fn data_rows(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: RowsArgs,
) -> Result<Value, ScriptError> {
    let store = host.store();
    let doc = store.snapshot();
    let (comp, template) = find_template(&doc, &a.comp)?;
    let id = comp.id;
    if a.file.is_some() || a.feeds.is_some() {
        let linked = comp.template_data();
        let mut data = match &a.file {
            Some(file) => {
                let file = match (reach::plain(file)?, reach) {
                    (file, Some(reach)) => reach.footage(host, file)?,
                    (file, None) => std::path::absolute(file).unwrap_or_else(|_| file.into()),
                };
                let table = read_table(&file)?;
                let mut feeds = feed::matched(&template, &table.columns);
                // A column fed by hand before keeps what it fed, by its name.
                for (at, name) in table.columns.iter().enumerate() {
                    let was = linked.as_ref().and_then(|before| {
                        let i = before.table.columns.iter().position(|c| c == name)?;
                        before.feeds.get(i)
                    });
                    if let (Some(was), Some(slot)) = (was, feeds.get_mut(at)) {
                        slot.clone_from(was);
                    }
                }
                Data {
                    file: file.to_string_lossy().into_owned(),
                    table,
                    feeds,
                    extra: serde_json::Map::new(),
                }
            }
            None => linked.ok_or_else(|| bad(format!("{} has no data linked.", comp.name)))?,
        };
        data.feeds.resize(data.table.columns.len(), Feeds::Nothing);
        for (column, what) in a.feeds.iter().flatten() {
            let at = data
                .table
                .columns
                .iter()
                .position(|c| c == column)
                .ok_or_else(|| {
                    ScriptError::NotFound(format!("The data has no column called \"{column}\"."))
                })?;
            let next = match what {
                Some(what) => feeds_named(&template, comp, what)?,
                None => Feeds::Nothing,
            };
            // One column feeds a thing, so another that fed it stops.
            for other in data.feeds.iter_mut().filter(|f| **f != Feeds::Nothing) {
                if *other == next {
                    *other = Feeds::Nothing;
                }
            }
            data.feeds[at] = next;
        }
        // The footage brought in and the link are one undo step.
        store.begin_undo_group();
        let linked = link(host, reach, id, &template, data);
        store.end_undo_group();
        linked?;
    }
    let doc = store.snapshot();
    let (comp, template) = find_template(&doc, &id.to_string())?;
    let Some(data) = comp.template_data() else {
        return Ok(json!({ "file": null, "columns": [], "rows": 0, "faults": [] }));
    };
    let columns: Vec<Value> = data
        .table
        .columns
        .iter()
        .enumerate()
        .map(|(at, name)| {
            let (feeds, control) = match data.feeds.get(at).unwrap_or(&Feeds::Nothing) {
                Feeds::Nothing => (None, None),
                Feeds::Control(id) => (Some("control"), Some(id)),
                Feeds::Font(id) => (Some("font"), Some(id)),
                Feeds::Size(id) => (Some("size"), Some(id)),
                Feeds::Colour(id) => (Some("colour"), Some(id)),
                Feeds::Length => (Some("length"), None),
                Feeds::FileName => (Some("file name"), None),
            };
            json!({ "name": name, "feeds": feeds, "control": control })
        })
        .collect();
    let mut faults: Vec<Value> = Vec::new();
    for row in 0..data.table.rows.len() {
        let read = feed::row_values(&template, &data, row).unwrap_or_default();
        faults.extend(read.faults.iter().map(
            |fault| json!({ "row": row + 1, "column": data.table.columns.get(fault.column) }),
        ));
    }
    Ok(json!({
        "file": data.file,
        "columns": columns,
        "rows": data.table.rows.len(),
        "faults": faults,
    }))
}

/// Today's date in UTC, as `{date}` writes it.
fn today() -> String {
    let days = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    // Days since 1970 to a date (Howard Hinnant's civil-from-days).
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RowsExportArgs {
    comp: String,
    rows: Option<(u32, u32)>,
    out: Option<PathBuf>,
    layer: Option<String>,
    clip: Option<String>,
}

pub(super) fn data_export(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: RowsExportArgs,
) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let (comp, template) = find_template(&doc, &a.comp)?;
    let data = comp
        .template_data()
        .ok_or_else(|| bad(format!("{} has no data linked.", comp.name)))?;
    let count = u32::try_from(data.table.rows.len()).unwrap_or(u32::MAX);
    let (first, last) = a.rows.unwrap_or((1, count));
    let last = last.min(count);
    if first < 1 || first > last {
        return Err(bad(format!("The data has {count} rows, counted from 1.")));
    }
    // The copy that follows each row, in whichever comp it sits.
    let copy = match &a.layer {
        Some(key) => {
            let id = Uuid::try_parse(key).ok();
            let found = doc
                .items
                .iter()
                .filter_map(|item| match item {
                    ProjectItem::Composition(c) => Some(c),
                    _ => None,
                })
                .flat_map(|c| c.layers.iter().map(move |l| (c, l)))
                .filter(|(_, l)| Some(l.id) == id || l.name == *key)
                .find_map(|(c, l)| {
                    copy_of(&doc, c, l, a.clip.as_deref())
                        .ok()
                        .filter(|copy| copy.template == comp.id)
                });
            Some(found.map(|copy| copy.at).ok_or_else(|| {
                ScriptError::NotFound(format!("No layer called \"{key}\" places {}.", comp.name))
            })?)
        }
        None if a.clip.is_some() => return Err(bad("A clip needs the layer it is on.")),
        None => None,
    };
    let out = a.out.as_deref().map(reach::plain).transpose()?;
    let out = match reach {
        Some(reach) => reach.export(host, out)?,
        None => out.map(Path::to_path_buf),
    };
    let named = out.as_deref().and_then(|out| {
        let folder = out.parent().filter(|dir| !dir.as_os_str().is_empty())?;
        Some((folder.to_path_buf(), out.file_stem()?.to_string_lossy()))
    });
    let (folder, pattern) = match named {
        Some((folder, pattern)) => (folder, pattern.into_owned()),
        None => {
            let beside = host.path().and_then(|p| p.parent().map(Path::to_path_buf));
            let folder = beside.ok_or_else(|| {
                bad("The project has not been saved yet, so the export needs a folder.")
            })?;
            let pattern = out
                .as_deref()
                .and_then(Path::file_stem)
                .map_or_else(|| ROW_NAME.into(), |s| s.to_string_lossy().into_owned());
            (folder, pattern)
        }
    };
    let today = today();
    let pictures = |file: &Path| held_picture(host, &doc, file);
    let mut exports: Vec<Value> = Vec::new();
    for row in first - 1..last {
        let Some(made) =
            version::row_version(&doc, comp.id, &template, &data, row, copy, &pictures)
        else {
            continue;
        };
        let name = version::row_name(
            &pattern,
            &comp.name,
            &data,
            row as usize,
            made.read.file_name.as_deref(),
            &today,
        );
        let (job, file) = host.export_version(made.comp, made.ops, &folder.join(name))?;
        exports.push(json!({ "job": job, "row": row + 1, "file": file }));
    }
    Ok(json!({ "exports": exports }))
}

pub(super) fn template_list(host: &dyn Host, _: NoArgs) -> Result<Value, ScriptError> {
    Ok(json!({ "templates": host.template_library() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlaceArgs {
    template: String,
    comp: Option<String>,
    at: Option<Time>,
}

pub(super) fn template_place(
    host: &dyn Host,
    reach: Option<&Reach>,
    a: PlaceArgs,
) -> Result<Value, ScriptError> {
    let store = host.store();
    // Before anything is brought in, so a comp misspelt brings nothing.
    if let Some(key) = &a.comp {
        find_comp(&store.snapshot(), key)?;
    }
    let named = Path::new(&a.template);
    let listed = host
        .template_library()
        .into_iter()
        .find(|t| t.name == a.template);
    let file = match listed {
        Some(listed) => listed.path,
        None if named
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lumt")) =>
        {
            match (reach::plain(named)?, reach) {
                (file, Some(reach)) => reach.footage(host, file)?,
                (file, None) => file.to_path_buf(),
            }
        }
        None => {
            return Err(ScriptError::NotFound(format!(
                "There is no graphics template called \"{}\" in the library.",
                a.template
            )))
        }
    };
    // Bringing it in and placing it are one undo step.
    store.begin_undo_group();
    let placed = place(host, &file, a.comp.as_deref(), a.at);
    store.end_undo_group();
    placed
}

/// Bring the template in `file` into the project, and place it in the comp
/// called `comp` when there is one, from `at` for as long as it runs.
fn place(
    host: &dyn Host,
    file: &Path,
    comp: Option<&str>,
    at: Option<Time>,
) -> Result<Value, ScriptError> {
    let store = host.store();
    let id = host.template_bring(file)?;
    let doc = store.snapshot();
    let inner = doc
        .comp(id)
        .ok_or_else(|| ScriptError::Failed("The graphics template was not brought in.".into()))?;
    let Some(key) = comp else {
        return Ok(json!({ "comp": comp_json(inner) }));
    };
    let outer = find_comp(&doc, key)?;
    if outer.id == inner.id || holds(&doc, inner, outer.id, &mut Vec::new()) {
        return Err(bad(format!(
            "{} is inside {} already, so {} cannot go in it.",
            outer.name, inner.name, inner.name
        )));
    }
    let at = at.unwrap_or(Time::Frame(0)).at(outer.frame_rate)?;
    let out = at
        .checked_add(inner.duration.0)
        .map_err(|_| bad("That time is out of range."))?;
    let mut layer = edits::precomp_layer(&doc, outer, inner);
    layer.in_point = CompTime(at);
    layer.start_offset = CompTime(at);
    layer.out_point = CompTime(out);
    edits::solo_on_arrival(&mut layer, outer.layers.iter());
    let layer_id = layer.id;
    commit(
        store,
        vec![Op::AddLayer {
            comp: outer.id,
            index: 0,
            layer: Box::new(layer),
        }],
    )?;
    Ok(json!({ "comp": comp_json(inner), "layer": layer_now(store, outer.id, layer_id)? }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SaveArgs {
    comp: String,
    name: Option<String>,
    #[serde(default)]
    group: String,
    #[serde(default)]
    pack_fonts: bool,
}

pub(super) fn template_save(host: &dyn Host, a: SaveArgs) -> Result<Value, ScriptError> {
    let doc = host.store().snapshot();
    let comp = find_comp(&doc, &a.comp)?;
    let name = a.name.as_deref().unwrap_or(&comp.name);
    let file = host.template_save(comp.id, name, &a.group, a.pack_fonts)?;
    Ok(json!({ "file": file }))
}
