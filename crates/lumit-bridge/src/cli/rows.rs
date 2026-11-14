//! `lumit-cli export --data <file>`: one export of a graphics template for
//! each row of a CSV, TSV or JSON file, through the same row versions the
//! Data window's **Export rows** makes.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use lumit_core::model::{Document, ProjectItem};
use lumit_core::template::feed::{self, Data};
use lumit_core::template::version;
use lumit_script::Host;
use uuid::Uuid;

use super::Export;
use crate::api::export::BridgeExportSpec;

/// The footage item in `doc` that reads `file`, when there is one.
fn footage_at(doc: &Document, file: &Path) -> Option<Uuid> {
    doc.items.iter().find_map(|item| match item {
        ProjectItem::Footage(f) if Path::new(&f.media.absolute_path) == file => Some(f.id),
        _ => None,
    })
}

/// Export the template `how` names once for each row of its `--data` file.
pub(super) fn export(
    project: &Path,
    how: &Export,
    data_file: &Path,
    settings: &BridgeExportSpec,
) -> Result<(), String> {
    let stop = super::interrupt::stop_flag();
    let data_file = std::path::absolute(data_file).unwrap_or_else(|_| data_file.to_path_buf());
    let table = crate::api::template::data::read_table(&data_file)
        .map_err(|_| format!("{} could not be read as rows.", data_file.display()))?;
    let host = super::open(project)?;
    crate::api::extensions::register_effects();
    let comps = super::chosen_comps(&host, how)?;
    let [(comp, name)] = comps.as_slice() else {
        return Err("--data takes one comp: the graphics template to fill in.".into());
    };
    let comp = *comp;
    let doc = host.store().snapshot();
    let template = doc
        .comp(comp)
        .and_then(|c| c.template())
        .filter(|t| !t.controls.is_empty())
        .ok_or_else(|| format!("{name} has no controls for the rows to set."))?;
    // What each column feeds: as the project has it for a column of that
    // name, and otherwise the control of that name.
    let mut feeds = feed::matched(&template, &table.columns);
    if let Some(linked) = doc.comp(comp).and_then(|c| c.template_data()) {
        for (at, column) in table.columns.iter().enumerate() {
            let was = linked
                .table
                .columns
                .iter()
                .position(|c| c == column)
                .and_then(|i| linked.feeds.get(i));
            if let (Some(was), Some(slot)) = (was, feeds.get_mut(at)) {
                slot.clone_from(was);
            }
        }
    }
    let data = Data {
        file: data_file.to_string_lossy().into_owned(),
        table,
        feeds,
        extra: serde_json::Map::new(),
    };
    let count = data.table.rows.len();
    let (first, last) = match how.rows {
        Some((first, last)) => (first.max(1), last.min(count as u32)),
        None => (1, count as u32),
    };
    if first > last {
        return Err(format!("The file has {count} rows."));
    }

    // The files the rows name, brought in once.
    let mut wanted: Vec<PathBuf> = Vec::new();
    for row in first - 1..last {
        let Some(read) = feed::row_values(&template, &data, row as usize) else {
            continue;
        };
        for (_, cell) in read.files {
            let file = version::cell_file(&data.file, &cell);
            if file.is_file() && footage_at(&doc, &file).is_none() && !wanted.contains(&file) {
                wanted.push(file);
            }
        }
    }
    if !wanted.is_empty() {
        host.import_footage(&wanted).map_err(|e| e.to_string())?;
    }

    // Where the files go and what they are called: `--out` is a name that
    // may hold `{Column}`, `{row}`, `{comp}` and `{date}`, in the folder it
    // names. Without it, the template's name and the row's number, beside
    // the project.
    // Beside the project, or beside the template file it was opened from.
    let beside = host
        .path()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| project.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let (folder, pattern) = match &how.out {
        Some(out) if out.is_dir() => (out.clone(), "{comp} {row}".to_owned()),
        Some(out) => (
            out.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or(beside.clone(), Path::to_path_buf),
            out.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "{comp} {row}".to_owned()),
        ),
        None => (beside, "{comp} {row}".to_owned()),
    };
    let folder = std::path::absolute(&folder).unwrap_or(folder);
    let today = crate::export::today();

    let mut failed = 0;
    let total = last - first + 1;
    for row in first - 1..last {
        if stop.load(Ordering::Relaxed) {
            return Err("Export stopped.".into());
        }
        let doc = host.store().snapshot();
        let pictures = |file: &Path| {
            let item = footage_at(&doc, file)?;
            let info = host.footage_info(item)?;
            Some((item, (info.width, info.height)))
        };
        let Some(made) = version::row_version(&doc, comp, &template, &data, row, None, &pictures)
        else {
            continue;
        };
        if version::faulty(&data, &made.read, &pictures) {
            note!(
                "Row {}: a cell could not be read, so the template's own value stands there.",
                row + 1
            );
        }
        let file = version::row_name(
            &pattern,
            name,
            &data,
            row as usize,
            made.read.file_name.as_deref(),
            &today,
        );
        let path = folder.join(file);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("{} could not be made: {e}.", dir.display()))?;
        }
        let path = PathBuf::from(crate::export::taken::untaken_path(
            &path.to_string_lossy(),
            &settings.codec,
            settings.captions,
            &settings.also,
        ));
        // The row's version is made in this run's own copy of the project,
        // which is never saved.
        host.store()
            .commit(lumit_core::ops::Op::Batch { ops: made.ops })
            .map_err(|e| e.to_string())?;
        match super::export_one(&host, &stop, made.comp, settings, how.range, path) {
            Ok(path) => out!("{}", path.display()),
            Err(why) if total == 1 => return Err(why),
            Err(why) => {
                note!("Row {}: {why}", row + 1);
                failed += 1;
            }
        }
    }
    super::note_unreadable_frames();
    match failed {
        0 => Ok(()),
        n => Err(format!("{n} of {total} rows failed.")),
    }
}
