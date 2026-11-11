//! One version of a graphics template for one row of its data: a copy fed
//! from the row, the edits that make that row's version of the project, and
//! the file it is written as.
//!
//! The interface, the command line and scripts all make a row's version
//! here, so a row exported from any of them is the same picture under the
//! same name.
//!
//! Nothing here opens a file. A footage cell names one, and the caller says
//! which footage item reads it ([`Pictures`]).

use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::data::matched_name;
use super::feed::{self, Data, RowValues};
use super::{Fit, Picture, Template, Value};
use crate::fx::effects::template as values;
use crate::model::{Document, EffectInstance, LayerKind, ProjectItem};
use crate::ops::Op;
use crate::time::{CompTime, Duration};

/// The footage item that reads a file a cell names, with its size in pixels,
/// or `None` when the project holds no item for that file. Bringing files in
/// and measuring them is the caller's.
pub type Pictures<'a> = &'a dyn Fn(&Path) -> Option<(Uuid, (u32, u32))>;

/// Where a file a cell names is: as written when that is a whole path, and
/// beside the data file otherwise.
#[must_use]
pub fn cell_file(data_file: &str, cell: &str) -> PathBuf {
    let named = PathBuf::from(cell);
    if named.is_absolute() {
        return named;
    }
    Path::new(data_file)
        .parent()
        .map_or(named.clone(), |dir| dir.join(&named))
}

/// Write row `row` of `data` into a copy's rows as the values its data row
/// feeds, in place of what an earlier row fed. Rows the copy set itself stay.
pub fn feed_copy(
    template: &Template,
    data: &Data,
    row: u32,
    inst: &mut EffectInstance,
    pictures: Pictures<'_>,
) {
    values::unfeed(inst);
    values::follow(inst, Some(row));
    let Some(read) = feed::row_values(template, data, row as usize) else {
        return;
    };
    for (id, value) in &read.values {
        if let Some(control) = template.control(id) {
            values::feed_value(inst, control, value);
        }
    }
    for (row_id, value) in &read.extras {
        values::feed_row(inst, row_id, value);
    }
    for (id, cell) in &read.files {
        let Some(control) = template.control(id) else {
            continue;
        };
        if let Some((item, size)) = pictures(&cell_file(&data.file, cell)) {
            let picture = Value::Picture(Picture {
                item,
                size,
                fit: Fit::default(),
            });
            values::feed_value(inst, control, &picture);
        }
    }
}

/// A copy's rows fed from row `row`, on top of `base` when the copy has
/// stored some. `None` when the copy would store nothing.
#[must_use]
pub fn fed(
    template_comp: Uuid,
    template: &Template,
    data: &Data,
    row: u32,
    base: Option<&EffectInstance>,
    pictures: Pictures<'_>,
) -> Option<EffectInstance> {
    let mut inst = match base.filter(|b| values::comp_of(b) == Some(template_comp)) {
        Some(base) => base.clone(),
        None => values::instance_for(template_comp, template),
    };
    feed_copy(template, data, row, &mut inst, pictures);
    (!values::is_bare(&inst)).then_some(inst)
}

/// The ops that feed every copy of template `comp` that follows a row from
/// `data` again: what linking other data, changing what a column feeds and
/// reloading the file all end with.
#[must_use]
pub fn refeed_ops(
    doc: &Document,
    comp: Uuid,
    template: &Template,
    data: Option<&Data>,
    pictures: Pictures<'_>,
) -> Vec<Op> {
    let again = |inst: &EffectInstance| -> Option<Option<EffectInstance>> {
        let row = values::held(inst).row?;
        if values::comp_of(inst) != Some(comp) {
            return None;
        }
        let mut next = inst.clone();
        match data {
            Some(data) => feed_copy(template, data, row, &mut next, pictures),
            None => {
                values::unfeed(&mut next);
                values::follow(&mut next, None);
            }
        }
        (next != *inst).then(|| (!values::is_bare(&next)).then_some(next))
    };
    let mut ops = Vec::new();
    for item in &doc.items {
        let ProjectItem::Composition(owner) = item else {
            continue;
        };
        for layer in &owner.layers {
            if let Some(next) = layer.graph_inputs.as_ref().and_then(again) {
                ops.push(Op::SetLayerGraphInputs {
                    comp: owner.id,
                    layer: layer.id,
                    inputs: next.map(Box::new),
                });
            }
            let LayerKind::Sequence { clips } = &layer.kind else {
                continue;
            };
            let mut changed = false;
            let mut next_clips = clips.clone();
            for clip in &mut next_clips {
                if let Some(next) = clip.inputs.as_ref().and_then(again) {
                    clip.inputs = next;
                    changed = true;
                }
            }
            if changed {
                ops.push(Op::SetSequenceClips {
                    comp: owner.id,
                    layer: layer.id,
                    clips: next_clips,
                });
            }
        }
    }
    ops
}

/// Where a copy that follows each row in turn sits: a Precomp layer, or a
/// clip of a Sequence layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Host {
    /// The composition the copy is in.
    pub comp: Uuid,
    /// The Precomp layer, or the Sequence layer holding the clip.
    pub layer: Uuid,
    /// The clip, for a copy that is one.
    pub clip: Option<Uuid>,
}

/// One row's version of the project.
#[derive(Debug, Clone, PartialEq)]
pub struct RowVersion {
    /// The edits that make it.
    pub ops: Vec<Op>,
    /// The composition to export.
    pub comp: Uuid,
    /// What the row read as.
    pub read: RowValues,
}

/// The edits that make row `row`'s version of the project, for the template
/// in composition `template_comp`.
///
/// With a `host` it is that copy following the row, and the composition to
/// export is the one the copy sits in: this is how one video is made again
/// for each row. Without, it is the template on its own, placed once in a
/// composition made for it that runs as long as the row says, so the intro
/// and the outro keep their speed whatever the length.
#[must_use]
pub fn row_version(
    doc: &Document,
    template_comp: Uuid,
    template: &Template,
    data: &Data,
    row: u32,
    host: Option<Host>,
    pictures: Pictures<'_>,
) -> Option<RowVersion> {
    let inner = doc.comp(template_comp)?;
    let read = feed::row_values(template, data, row as usize)?;
    if let Some(host) = host {
        let owner = doc.comp(host.comp)?;
        let layer = owner.layers.iter().find(|l| l.id == host.layer)?;
        let op = match host.clip {
            None => Op::SetLayerGraphInputs {
                comp: host.comp,
                layer: host.layer,
                inputs: fed(
                    template_comp,
                    template,
                    data,
                    row,
                    layer.graph_inputs.as_ref(),
                    pictures,
                )
                .map(Box::new),
            },
            Some(clip_id) => {
                let LayerKind::Sequence { clips } = &layer.kind else {
                    return None;
                };
                let mut clips = clips.clone();
                let clip = clips.iter_mut().find(|c| c.id == clip_id)?;
                clip.inputs = fed(
                    template_comp,
                    template,
                    data,
                    row,
                    clip.inputs.as_ref(),
                    pictures,
                );
                Op::SetSequenceClips {
                    comp: host.comp,
                    layer: host.layer,
                    clips,
                }
            }
        };
        return Some(RowVersion {
            ops: vec![op],
            comp: host.comp,
            read,
        });
    }
    let length = read
        .length
        .and_then(|seconds| {
            let frames = (seconds * inner.frame_rate.fps()).round().max(1.0) as i64;
            inner.frame_rate.time_of_frame(frames).ok().map(|t| t.0)
        })
        .unwrap_or(inner.duration.0);
    let settings = crate::edits::CompSettings {
        name: inner.name.clone(),
        width: inner.width,
        height: inner.height,
        frame_rate: inner.frame_rate,
        duration: Duration(length),
        background: inner.background.0,
        ..Default::default()
    };
    let (made, mut ops) = crate::edits::new_comp_ops(doc, settings, None);
    // The layer is made for a composition of that length, which the ops have
    // yet to add: a scratch copy stands in for it.
    let mut scratch = doc.clone();
    for op in &ops {
        crate::ops::apply(&mut scratch, op).ok()?;
    }
    let outer = scratch.comp(made)?;
    let mut layer = crate::edits::precomp_layer(&scratch, outer, inner);
    layer.out_point = CompTime(length);
    layer.graph_inputs = fed(template_comp, template, data, row, None, pictures);
    ops.push(Op::AddLayer {
        comp: made,
        index: 0,
        layer: Box::new(layer),
    });
    Some(RowVersion {
        ops,
        comp: made,
        read,
    })
}

/// Whether a row has a fault: a cell that will not read, or a file a cell
/// names that the project does not hold.
#[must_use]
pub fn faulty(data: &Data, read: &RowValues, pictures: Pictures<'_>) -> bool {
    !read.faults.is_empty()
        || read
            .files
            .iter()
            .any(|(_, cell)| pictures(&cell_file(&data.file, cell)).is_none())
}

/// One piece of a file's name: what a file system refuses becomes an
/// underscore, and a name that would be nothing is `row`.
fn plain(piece: &str) -> String {
    let made: String = piece
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let made = made.trim().trim_matches('.').trim().to_owned();
    if made.is_empty() {
        "row".to_owned()
    } else {
        made
    }
}

/// The file a row is written as, without its ending: `pattern` with each
/// `{Column}` put in from the row, `{row}` its number counting from 1,
/// `{comp}` the composition's name and `{date}` what `today` says. A row
/// whose file-name column says a name (`named`) uses that instead. Either may
/// hold folders, parted by a slash, and never leaves the folder it is given.
#[must_use]
pub fn row_name(
    pattern: &str,
    comp_name: &str,
    data: &Data,
    row: usize,
    named: Option<&str>,
    today: &str,
) -> String {
    let cells = data.table.rows.get(row).map_or(&[][..], Vec::as_slice);
    let mut made = String::new();
    let mut rest = named.filter(|n| !n.trim().is_empty()).unwrap_or(pattern);
    while let Some(open) = rest.find('{') {
        made.push_str(&rest[..open]);
        rest = &rest[open..];
        let Some(close) = rest.find('}') else {
            break;
        };
        let open = 0;
        let wanted = matched_name(&rest[open + 1..open + close]);
        let cell = data
            .table
            .columns
            .iter()
            .position(|c| matched_name(c) == wanted)
            .and_then(|at| cells.get(at));
        match (wanted.as_str(), cell) {
            (_, Some(cell)) => made.push_str(cell.trim()),
            ("row", None) => made.push_str(&(row + 1).to_string()),
            ("comp", None) => made.push_str(comp_name),
            ("date", None) => made.push_str(today),
            // Not a token of ours: left as it was written.
            _ => made.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    made.push_str(rest);
    made.split(['/', '\\'])
        .filter(|piece| !piece.trim().is_empty() && piece.trim() != "..")
        .map(plain)
        .collect::<Vec<_>>()
        .join("/")
}
