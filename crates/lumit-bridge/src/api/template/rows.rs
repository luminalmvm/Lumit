//! One version of a graphics template for each row of its data: placed as
//! copies on a timeline, queued as exports, or written as stills.
//!
//! A row's version is the project with a few edits made to it, and those
//! edits are what an export item keeps ([`row_version`]), applied when the
//! item starts. So five hundred rows of one template share one snapshot of
//! the project and are not five hundred projects in memory.
//!
//! Queueing and placing are single edits on the UI thread. Stills render, so
//! [`CompositionReference::export_data_stills`] is not `sync`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::fx::effects::template as values;
use lumit_core::model::{Document, EffectInstance};
use lumit_core::ops::Op;
use lumit_core::template::feed::{self, Data};
use lumit_core::template::version::{self, Host};
use lumit_core::template::Template;
use lumit_core::time::{CompTime, Rational};
use uuid::Uuid;

use crate::api::export::BridgeExportSpec;
use crate::api::{composition::CompositionReference, layer::LayerReference, BridgeError};

/// What the rows asked for come to, before anything is queued.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRowsCheck {
    /// The rows with a cell that will not read as what its column feeds, or
    /// that name a file that is not there. Counting from 0.
    pub faulty: Vec<u32>,
    /// The file each row asked for would be written as, in the order asked,
    /// without its ending.
    pub names: Vec<String>,
}

/// How far a run of stills has got.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeStillsProgress {
    pub done: u32,
    pub total: u32,
    pub running: bool,
}

static STILLS_DONE: AtomicU32 = AtomicU32::new(0);
static STILLS_TOTAL: AtomicU32 = AtomicU32::new(0);
static STILLS_RUNNING: AtomicBool = AtomicBool::new(false);
static STILLS_STOP: AtomicBool = AtomicBool::new(false);

/// How far the stills being written have got.
#[frb(sync)]
pub fn data_stills_progress() -> BridgeStillsProgress {
    BridgeStillsProgress {
        done: STILLS_DONE.load(Ordering::Relaxed),
        total: STILLS_TOTAL.load(Ordering::Relaxed),
        running: STILLS_RUNNING.load(Ordering::Relaxed),
    }
}

/// Stop the stills being written after the one in hand.
#[frb(sync)]
pub fn cancel_data_stills() {
    STILLS_STOP.store(true, Ordering::Relaxed);
}

impl CompositionReference {
    /// The template and its data, for the calls below.
    #[frb(ignore)]
    fn template_and_data(&self, doc: &Document) -> Result<(Arc<Template>, Data), BridgeError> {
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        match (comp.template(), comp.template_data()) {
            (Some(template), Some(data)) => Ok((template, data)),
            _ => Err(BridgeError::InvalidComp),
        }
    }

    /// The copy a run of rows is exported through, when one was picked.
    #[frb(ignore)]
    fn host_of(host: Option<&LayerReference>, clip: Option<Uuid>) -> Option<Host> {
        host.map(|layer| Host {
            comp: layer.comp_id,
            layer: layer.layer_id,
            clip,
        })
    }

    /// Look at `rows` before exporting them: which have a fault, and the file
    /// each would be written as under `pattern`.
    #[frb(sync)]
    pub fn check_data_rows(
        &self,
        rows: Vec<u32>,
        pattern: String,
    ) -> Result<BridgeRowsCheck, BridgeError> {
        let doc = self.document()?;
        let (template, data) = self.template_and_data(&doc)?;
        let pictures = |file: &Path| self.held_picture(&doc, file);
        let name = doc
            .comp(self.id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let mut check = BridgeRowsCheck {
            faulty: Vec::new(),
            names: Vec::new(),
        };
        for row in rows {
            let Some(read) = feed::row_values(&template, &data, row as usize) else {
                continue;
            };
            if version::faulty(&data, &read, &pictures) {
                check.faulty.push(row);
            }
            check.names.push(version::row_name(
                &pattern,
                &name,
                &data,
                row as usize,
                read.file_name.as_deref(),
                &crate::export::today(),
            ));
        }
        Ok(check)
    }

    /// **Export rows**: queue one export for each of `rows`, written into
    /// `folder` under `pattern`, folded under one heading in the queue. With
    /// `host` the composition that copy sits in is exported with the copy
    /// following each row in turn; without, the template on its own.
    ///
    /// A row with a fault is left out when `skip_faulty` is on, and otherwise
    /// exported with the template's own value where the fault is. A name two
    /// rows share is numbered. Answers how many were queued.
    #[frb(sync)]
    #[allow(clippy::too_many_arguments)]
    pub fn queue_data_rows(
        &self,
        spec: BridgeExportSpec,
        rows: Vec<u32>,
        pattern: String,
        folder: String,
        skip_faulty: bool,
        host: Option<LayerReference>,
        host_clip: Option<Uuid>,
        start: bool,
    ) -> Result<u32, BridgeError> {
        let doc = self.document()?;
        if !lumit_render::decode::addon_needs(&doc).is_empty() {
            return Err(BridgeError::AddonMissing);
        }
        let (template, data) = self.template_and_data(&doc)?;
        let pictures = |file: &Path| self.held_picture(&doc, file);
        let name = doc
            .comp(self.id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let host = Self::host_of(host.as_ref(), host_clip);
        let group = crate::export::queue_group(name.clone());
        let mut queued = 0;
        for row in rows {
            let Some(made) =
                version::row_version(&doc, self.id, &template, &data, row, host, &pictures)
            else {
                continue;
            };
            if skip_faulty && version::faulty(&data, &made.read, &pictures) {
                continue;
            }
            let file = version::row_name(
                &pattern,
                &name,
                &data,
                row as usize,
                made.read.file_name.as_deref(),
                &crate::export::today(),
            );
            let path = Path::new(&folder).join(file);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|_| BridgeError::WriteFailed)?;
            }
            let path =
                crate::export::untaken_path(&path.to_string_lossy(), &spec.codec, spec.captions);
            crate::export::queue_add_version(
                doc.clone(),
                made.comp,
                name.clone(),
                &spec,
                &path,
                made.ops,
                group,
            )
            .map_err(BridgeError::ExportFailed)?;
            queued += 1;
        }
        if start && queued > 0 {
            crate::export::queue_start();
        }
        Ok(queued)
    }

    /// **Export rows** as stills: one PNG for each of `rows` at the
    /// template's poster frame, written into `folder` under `pattern`, never
    /// over a file that is there. Answers how many were written. Stops after
    /// the one in hand when [`cancel_data_stills`] is called.
    #[allow(clippy::too_many_arguments)]
    pub fn export_data_stills(
        &self,
        rows: Vec<u32>,
        pattern: String,
        folder: String,
        skip_faulty: bool,
        host: Option<LayerReference>,
        host_clip: Option<Uuid>,
        frame: u64,
    ) -> Result<u32, BridgeError> {
        let doc = self.document()?;
        let (template, data) = self.template_and_data(&doc)?;
        let pictures = |file: &Path| self.held_picture(&doc, file);
        let name = doc
            .comp(self.id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let host = Self::host_of(host.as_ref(), host_clip);
        STILLS_STOP.store(false, Ordering::Relaxed);
        STILLS_DONE.store(0, Ordering::Relaxed);
        STILLS_TOTAL.store(rows.len() as u32, Ordering::Relaxed);
        STILLS_RUNNING.store(true, Ordering::Relaxed);
        let mut written = 0;
        for row in rows {
            if STILLS_STOP.load(Ordering::Relaxed) {
                break;
            }
            STILLS_DONE.fetch_add(1, Ordering::Relaxed);
            let Some(made) =
                version::row_version(&doc, self.id, &template, &data, row, host, &pictures)
            else {
                continue;
            };
            if skip_faulty && version::faulty(&data, &made.read, &pictures) {
                continue;
            }
            let mut still = Document::clone(&doc);
            if made
                .ops
                .iter()
                .any(|op| lumit_core::ops::apply(&mut still, op).is_err())
            {
                continue;
            }
            let file = version::row_name(
                &pattern,
                &name,
                &data,
                row as usize,
                made.read.file_name.as_deref(),
                &crate::export::today(),
            );
            let mut path = PathBuf::from(&folder).join(format!("{file}.png"));
            if let Some(dir) = path.parent() {
                if std::fs::create_dir_all(dir).is_err() {
                    continue;
                }
            }
            // Keep both: a name that is taken is numbered.
            let mut n = 2;
            while path.exists() {
                path = PathBuf::from(&folder).join(format!("{file}_{n}.png"));
                n += 1;
            }
            if crate::render::save_frame(&Arc::new(still), made.comp, frame, 1.0, &path).is_ok() {
                written += 1;
            }
        }
        STILLS_RUNNING.store(false, Ordering::Relaxed);
        Ok(written)
    }

    /// **Place rows**: one copy of template `template` for each of `rows` in
    /// this composition's Timeline, as Precomp layers one after another from
    /// `at_frame`, each following its row and as long as its row says. One
    /// undo step.
    #[frb(sync)]
    pub fn place_data_rows(
        &self,
        template: &CompositionReference,
        rows: Vec<u32>,
        at_frame: i64,
    ) -> Result<(), BridgeError> {
        if template.id == self.id {
            return Err(BridgeError::InvalidComp);
        }
        let doc = self.document()?;
        let (controls, data) = template.template_and_data(&doc)?;
        let outer = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let inner = doc.comp(template.id).ok_or(BridgeError::InvalidComp)?;
        let mut at = outer
            .frame_rate
            .time_of_frame(at_frame.max(0))
            .map_err(|_| BridgeError::InvalidTime)?
            .0;
        let mut ops = Vec::new();
        for row in rows {
            let Some(read) = feed::row_values(&controls, &data, row as usize) else {
                continue;
            };
            let length = copy_length(outer, inner, read.length)?;
            let out = at
                .checked_add(length)
                .map_err(|_| BridgeError::InvalidTime)?;
            let mut layer = lumit_core::edits::precomp_layer(&doc, outer, inner);
            layer.in_point = CompTime(at);
            layer.start_offset = CompTime(at);
            layer.out_point = CompTime(out);
            let pictures = |file: &Path| template.held_picture(&doc, file);
            layer.graph_inputs = version::fed(template.id, &controls, &data, row, None, &pictures);
            // Each above the last, so the stack reads down in time.
            ops.push(Op::AddLayer {
                comp: self.id,
                index: 0,
                layer: Box::new(layer),
            });
            at = out;
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.commit(Op::Batch { ops })
    }
}

/// A data row the Viewer shows without the project changing: on the copy
/// that was picked, or on the template itself.
#[frb(ignore)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowLook {
    project: Uuid,
    template: Uuid,
    row: u32,
    host: Option<(Uuid, Uuid, Option<Uuid>)>,
}

static ROW_LOOK: std::sync::Mutex<Option<RowLook>> = std::sync::Mutex::new(None);

/// The data row the Viewer is showing, when it is showing one.
#[frb(ignore)]
pub(crate) fn row_look() -> Option<RowLook> {
    ROW_LOOK.lock().ok().and_then(|look| *look)
}

/// `document` as the Viewer shows it under `look`: the picked copy following
/// the row, or the template's own rows holding the row's values. A look at a
/// template another project holds, or one that has gone, changes nothing.
#[frb(ignore)]
pub(crate) fn looked(look: RowLook, document: Arc<Document>) -> Arc<Document> {
    let holder = CompositionReference {
        project: look.project,
        id: look.template,
    };
    let Some((template, data)) = document
        .comp(look.template)
        .and_then(|c| Some((c.template()?, c.template_data()?)))
    else {
        return document;
    };
    let pictures = |file: &Path| holder.held_picture(&document, file);
    match look.host {
        Some((comp, layer, clip)) => {
            let host = Host { comp, layer, clip };
            let Some(version) = version::row_version(
                &document,
                look.template,
                &template,
                &data,
                look.row,
                Some(host),
                &pictures,
            ) else {
                return document;
            };
            let mut made = Document::clone(&document);
            for op in &version.ops {
                if lumit_core::ops::apply(&mut made, op).is_err() {
                    return document;
                }
            }
            Arc::new(made)
        }
        None => {
            let Some(inst) =
                version::fed(look.template, &template, &data, look.row, None, &pictures)
            else {
                return document;
            };
            let settings = values::settings_at(&inst, &template, 0.0, None, None);
            lumit_core::template::placed::with(&document, look.template, &settings)
                .unwrap_or(document)
        }
    }
}

impl CompositionReference {
    /// Show row `row` of this template's data in the Viewer without changing
    /// the project: on the copy `host` when one is given, else on the
    /// template itself. `None` shows the project as it is again. The caller
    /// asks the Viewer for its frame afresh.
    #[frb(sync)]
    pub fn show_data_row(
        &self,
        row: Option<u32>,
        host: Option<LayerReference>,
        host_clip: Option<Uuid>,
    ) -> Result<(), BridgeError> {
        let mut look = ROW_LOOK.lock().map_err(|_| BridgeError::WriteFailed)?;
        *look = row.map(|row| RowLook {
            project: self.project,
            template: self.id,
            row,
            host: host.map(|layer| (layer.comp_id, layer.layer_id, host_clip)),
        });
        Ok(())
    }
}

/// How long a copy placed for a row runs in composition `outer`: what the
/// row says, on `outer`'s frames, or the template's own length.
#[frb(ignore)]
pub(crate) fn copy_length(
    outer: &lumit_core::model::Composition,
    inner: &lumit_core::model::Composition,
    seconds: Option<f64>,
) -> Result<Rational, BridgeError> {
    match seconds {
        Some(seconds) => {
            let frames = (seconds * outer.frame_rate.fps()).round().max(1.0) as i64;
            outer
                .frame_rate
                .time_of_frame(frames)
                .map(|t| t.0)
                .map_err(|_| BridgeError::InvalidTime)
        }
        None => Ok(inner.duration.0),
    }
}

/// The fed rows of a copy placed for row `row` of `template`'s data, and how
/// long the row says the copy runs. What placing rows as clips asks.
#[frb(ignore)]
pub(crate) fn row_copy(
    template: &CompositionReference,
    doc: &Document,
    row: u32,
) -> Option<(Option<EffectInstance>, Option<f64>)> {
    let comp = doc.comp(template.id)?;
    let (controls, data) = (comp.template()?, comp.template_data()?);
    let read = feed::row_values(&controls, &data, row as usize)?;
    let pictures = |file: &Path| template.held_picture(doc, file);
    Some((
        version::fed(template.id, &controls, &data, row, None, &pictures),
        read.length,
    ))
}
