//! Data for a graphics template: the file linked to it, what each column
//! feeds, the copies that follow a row, and placing a copy for each row
//! (`lumit_core::template::feed`).
//!
//! Reading a file is off the UI thread: [`CompositionReference::link_data`],
//! `reload_data` and `save_starting_data` are not `sync`. Everything else is
//! a read of the project or one edit, like any other.

use std::path::Path;

use flutter_rust_bridge::frb;
use lumit_core::fx::effects::template as values;
use lumit_core::model::{Document, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::template::data::{self, Format, Table};
use lumit_core::template::feed::{self, Data, Feeds};
use lumit_core::template::version::{cell_file, feed_copy, refeed_ops};
use uuid::Uuid;

use crate::api::{composition::CompositionReference, layer::LayerReference, BridgeError};

/// The most bytes of a data file that are read: far past any table a hand
/// keeps, and short of one that would stall the project it is kept in.
const DATA_FILE_MAX_BYTES: u64 = 64 << 20;

/// What a column feeds.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFeeds {
    Nothing,
    /// The control named beside it.
    Control,
    /// That text control's font, size or colour.
    Font,
    Size,
    Colour,
    /// How long the row's version runs.
    Length,
    /// The name of the row's exported file.
    FileName,
}

/// One column of a template's data.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDataColumn {
    pub name: String,
    pub feeds: BridgeFeeds,
    /// The control it feeds, by id, when it feeds one.
    pub control: Option<String>,
}

/// One cell that could not be read as what its column feeds.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeDataFault {
    pub row: u32,
    pub column: u32,
}

/// One row of the data linked to a template: a cell for each column.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDataRow {
    pub cells: Vec<String>,
}

/// The data linked to a template.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeData {
    /// The file the rows came from.
    pub file: String,
    pub columns: Vec<BridgeDataColumn>,
    pub rows: Vec<BridgeDataRow>,
    /// The cells that will not read. Each uses the template's own value.
    pub faults: Vec<BridgeDataFault>,
}

fn bridge_feeds(feeds: &Feeds) -> (BridgeFeeds, Option<String>) {
    match feeds {
        Feeds::Nothing => (BridgeFeeds::Nothing, None),
        Feeds::Control(id) => (BridgeFeeds::Control, Some(id.clone())),
        Feeds::Font(id) => (BridgeFeeds::Font, Some(id.clone())),
        Feeds::Size(id) => (BridgeFeeds::Size, Some(id.clone())),
        Feeds::Colour(id) => (BridgeFeeds::Colour, Some(id.clone())),
        Feeds::Length => (BridgeFeeds::Length, None),
        Feeds::FileName => (BridgeFeeds::FileName, None),
    }
}

fn core_feeds(feeds: BridgeFeeds, control: Option<String>) -> Result<Feeds, BridgeError> {
    let named = || control.clone().ok_or(BridgeError::InvalidParam);
    Ok(match feeds {
        BridgeFeeds::Nothing => Feeds::Nothing,
        BridgeFeeds::Control => Feeds::Control(named()?),
        BridgeFeeds::Font => Feeds::Font(named()?),
        BridgeFeeds::Size => Feeds::Size(named()?),
        BridgeFeeds::Colour => Feeds::Colour(named()?),
        BridgeFeeds::Length => Feeds::Length,
        BridgeFeeds::FileName => Feeds::FileName,
    })
}

/// Read the table in the file at `path`: by its name's ending, and for any
/// other ending as CSV.
#[frb(ignore)]
pub(crate) fn read_table(path: &Path) -> Result<Table, BridgeError> {
    let text = lumit_ingress::read_to_string_capped(path, DATA_FILE_MAX_BYTES)
        .map_err(|e| BridgeError::ExportFailed(e.to_string()))?;
    let format = path
        .file_name()
        .and_then(|n| Format::of_name(&n.to_string_lossy()))
        .unwrap_or(Format::Csv);
    data::parse(&text, format).map_err(|e| BridgeError::ExportFailed(e.to_string()))
}

/// The footage item in `doc` that reads the file at `path`, when there is one.
fn footage_at(doc: &Document, path: &Path) -> Option<Uuid> {
    doc.items.iter().find_map(|item| match item {
        ProjectItem::Footage(f) if Path::new(&f.media.absolute_path) == path => Some(f.id),
        _ => None,
    })
}

impl CompositionReference {
    /// The footage item and its size for a file a cell names, when the
    /// project already holds that file.
    #[frb(ignore)]
    pub(crate) fn held_picture(&self, doc: &Document, file: &Path) -> Option<(Uuid, (u32, u32))> {
        let item = footage_at(doc, file)?;
        let comp = doc.comp(self.id)?;
        let size = super::copy::picture_size(self.project, doc, comp, item).ok()?;
        Some((item, size))
    }

    /// Bring every file the data's footage columns name into the project,
    /// filed in a folder called `folder`, so a row that names one finds it.
    /// A file that is not there is left out, and its cells feed nothing.
    #[frb(ignore)]
    pub(crate) fn bring_data_files(&self, data: &Data, folder: &str) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let Some(template) = doc.comp(self.id).and_then(|c| c.template()) else {
            return Ok(());
        };
        let project = crate::api::project::ProjectReference { id: self.project };
        let mut brought: Vec<Uuid> = Vec::new();
        for row in 0..data.table.rows.len() {
            let Some(read) = feed::row_values(&template, data, row) else {
                continue;
            };
            for (_, cell) in read.files {
                let file = cell_file(&data.file, &cell);
                if !file.is_file() || footage_at(&*self.document()?, &file).is_some() {
                    continue;
                }
                if let Ok(footage) = project.import_footage(file.to_string_lossy().into_owned()) {
                    brought.push(footage.id);
                }
            }
        }
        if brought.is_empty() || folder.trim().is_empty() {
            return Ok(());
        }
        let doc = self.document()?;
        let mut ops = Vec::new();
        let home = match lumit_core::template::author::library_folder(&doc, folder) {
            Some(id) => id,
            None => {
                let id = Uuid::now_v7();
                ops.push(Op::AddItem {
                    index: doc.items.len(),
                    item: Box::new(ProjectItem::Folder(lumit_core::model::Folder {
                        id,
                        name: folder.to_owned(),
                        children: Vec::new(),
                        extra: serde_json::Map::new(),
                    })),
                });
                id
            }
        };
        // Out of whichever folder the import filed them in, and into this one.
        for item in &doc.items {
            if let ProjectItem::Folder(f) = item {
                if f.id != home && f.children.iter().any(|c| brought.contains(c)) {
                    ops.push(Op::SetFolderChildren {
                        folder: f.id,
                        children: f
                            .children
                            .iter()
                            .copied()
                            .filter(|c| !brought.contains(c))
                            .collect(),
                    });
                }
            }
        }
        let mut children = doc
            .folder(home)
            .map(|f| f.children.clone())
            .unwrap_or_default();
        children.extend(brought);
        ops.push(Op::SetFolderChildren {
            folder: home,
            children,
        });
        self.commit(Op::Batch { ops })
    }

    /// Set this template's data and feed every copy that follows a row again,
    /// as one undo step.
    #[frb(ignore)]
    fn commit_data(&self, data: Option<Data>) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let template = doc
            .comp(self.id)
            .and_then(|c| c.template())
            .ok_or(BridgeError::InvalidComp)?;
        let pictures = |file: &Path| self.held_picture(&doc, file);
        let mut ops = vec![Op::SetCompTemplateData {
            comp: self.id,
            data: data.clone().map(Box::new),
        }];
        ops.extend(refeed_ops(
            &doc,
            self.id,
            &template,
            data.as_ref(),
            &pictures,
        ));
        self.commit(Op::Batch { ops })
    }

    /// **Open file**: link the CSV, TSV or JSON file at `path` to this
    /// template. Its rows are kept in the project. Each column feeds the
    /// control of its name unless it fed something before under that name.
    /// Footage the rows name is brought into a folder called `folder`.
    pub fn link_data(&self, path: String, folder: String) -> Result<(), BridgeError> {
        let table = read_table(Path::new(&path))?;
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let template = comp.template().ok_or(BridgeError::InvalidComp)?;
        let mut feeds = feed::matched(&template, &table.columns);
        // A column fed by hand before keeps what it fed, by its name.
        if let Some(before) = comp.template_data() {
            for (at, name) in table.columns.iter().enumerate() {
                let was = before
                    .table
                    .columns
                    .iter()
                    .position(|c| c == name)
                    .and_then(|i| before.feeds.get(i));
                if let (Some(was), Some(slot)) = (was, feeds.get_mut(at)) {
                    slot.clone_from(was);
                }
            }
        }
        let data = Data {
            file: path,
            table,
            feeds,
            extra: serde_json::Map::new(),
        };
        self.bring_data_files(&data, &folder)?;
        self.commit_data(Some(data))
    }

    /// **Reload**: read the linked file again.
    pub fn reload_data(&self, folder: String) -> Result<(), BridgeError> {
        let file = self
            .composition()?
            .template_data()
            .ok_or(BridgeError::InvalidComp)?
            .file;
        self.link_data(file, folder)
    }

    /// Take this template's data away. Copies that followed a row keep what
    /// they set themselves and follow the template in the rest.
    #[frb(sync)]
    pub fn unlink_data(&self) -> Result<(), BridgeError> {
        self.commit_data(None)
    }

    /// This template's data, or `None` when none is linked.
    #[frb(sync)]
    pub fn get_data(&self) -> Result<Option<BridgeData>, BridgeError> {
        let comp = self.composition()?;
        let (Some(template), Some(data)) = (comp.template(), comp.template_data()) else {
            return Ok(None);
        };
        let columns = data
            .table
            .columns
            .iter()
            .enumerate()
            .map(|(at, name)| {
                let (feeds, control) = bridge_feeds(data.feeds.get(at).unwrap_or(&Feeds::Nothing));
                BridgeDataColumn {
                    name: name.clone(),
                    feeds,
                    control,
                }
            })
            .collect();
        let mut faults = Vec::new();
        for row in 0..data.table.rows.len() {
            if let Some(read) = feed::row_values(&template, &data, row) {
                faults.extend(read.faults.into_iter().map(|f| BridgeDataFault {
                    row: row as u32,
                    column: f.column as u32,
                }));
            }
        }
        Ok(Some(BridgeData {
            file: data.file.clone(),
            columns,
            rows: data
                .table
                .rows
                .iter()
                .map(|cells| BridgeDataRow {
                    cells: cells.clone(),
                })
                .collect(),
            faults,
        }))
    }

    /// Say what column `column` feeds.
    #[frb(sync)]
    pub fn set_data_feed(
        &self,
        column: u32,
        feeds: BridgeFeeds,
        control: Option<String>,
    ) -> Result<(), BridgeError> {
        let mut data = self
            .composition()?
            .template_data()
            .ok_or(BridgeError::InvalidComp)?;
        let width = data.table.columns.len();
        if column as usize >= width {
            return Err(BridgeError::InvalidParam);
        }
        data.feeds.resize(width, Feeds::Nothing);
        let next = core_feeds(feeds, control)?;
        // One column feeds a thing, so another that fed it stops.
        if next != Feeds::Nothing {
            for other in &mut data.feeds {
                if *other == next {
                    *other = Feeds::Nothing;
                }
            }
        }
        data.feeds[column as usize] = next;
        self.commit_data(Some(data))
    }

    /// **Save a starting file**: a CSV with a column for each control and the
    /// template's own values as its first row, so the names always match.
    pub fn save_starting_data(&self, path: String) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let template = comp.template().ok_or(BridgeError::InvalidComp)?;
        let table = feed::starting_table(&doc, comp, &template);
        std::fs::write(path, data::to_csv(&table)).map_err(|_| BridgeError::WriteFailed)
    }
}

impl LayerReference {
    /// Let this copy, or its clip `clip`, follow row `row` of its template's
    /// data, or with `None` stop following one. Its controls then come from
    /// that row, except the ones it sets itself.
    #[frb(sync)]
    pub fn follow_data_row(&self, row: Option<u32>, clip: Option<Uuid>) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let holder = CompositionReference {
            project: self.project_id,
            id: self.comp_id,
        };
        let pictures = |file: &Path| holder.held_picture(&doc, file);
        self.edit_copy(clip, |doc, template, inst| {
            let comp = values::comp_of(inst).and_then(|id| doc.comp(id));
            let data = comp.and_then(|c| c.template_data());
            match (row, data) {
                (Some(row), Some(data)) => feed_copy(template, &data, row, inst, &pictures),
                _ => {
                    values::unfeed(inst);
                    values::follow(inst, None);
                }
            }
            Ok(())
        })
    }
}
