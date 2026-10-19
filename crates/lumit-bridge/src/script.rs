//! File > Run script: Lumit's own [`Host`] for `lumit_script`, and the thread
//! a script runs from.
//!
//! The thread that owns the host is the one `lumit_script::run` blocks, and
//! every `lumit` call the script makes comes back to it. Edits commit through
//! the project's store the way a shared project's threads do, so the
//! interface hears them as ordinary changes. This project's lock is only
//! taken inside a host call, never across one (docs/14 §1, `api::state`).
//!
//! While a script runs the interface sits behind a card with Stop on it, so
//! nothing else commits into the run's undo step.

use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};

use lumit_core::model::{Composition, Layer};
use lumit_core::time::CompTime;
use lumit_core::DocumentStore;
use lumit_script::{ExportState, ExportStatus, Host, RunOptions, ScriptError};
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference, export::BridgeExportState, project::ProjectReference,
    script::BridgeScriptEvent, streams::BridgeStreamsArrival, BridgeError,
};

/// The longest script read, far past any written by hand.
pub(crate) const MAX_SOURCE_BYTES: u64 = 16 << 20;

/// The script that is running, if one is: how to stop it, and where the
/// file it is waiting for goes.
struct Running {
    stop: Arc<AtomicBool>,
    answer: Option<SyncSender<Option<PathBuf>>>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Run the script at `path` on `project`, on a thread of its own, with
/// `comp` open in the Viewer. False when one is running already. `tell` hears
/// what it does, and answers false when nobody is listening.
pub(crate) fn start(
    project: ProjectReference,
    path: PathBuf,
    comp: Option<Uuid>,
    tell: impl Fn(BridgeScriptEvent) -> bool + Send + 'static,
) -> Result<bool, BridgeError> {
    let store = {
        let state = project.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.clone()
    };
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut running = RUNNING.lock().map_err(|_| BridgeError::WriteFailed)?;
        if running.is_some() {
            return Ok(false);
        }
        *running = Some(Running {
            stop: stop.clone(),
            answer: None,
        });
    }
    let spawned = std::thread::Builder::new()
        .name("script".into())
        .spawn(move || {
            let end = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_file(project, store, &path, comp, stop, &tell)
            }))
            .unwrap_or_else(|_| finished(Vec::new(), "The script stopped unexpectedly.", None));
            // Free before saying so, so the next run can start straight away.
            if let Ok(mut running) = RUNNING.lock() {
                *running = None;
            }
            tell(end);
        });
    if spawned.is_err() {
        if let Ok(mut running) = RUNNING.lock() {
            *running = None;
        }
        return Ok(false);
    }
    Ok(true)
}

/// Stop the running script at its next step, and let go of a file it is
/// waiting for.
pub(crate) fn stop() {
    if let Ok(mut running) = RUNNING.lock() {
        if let Some(running) = running.as_mut() {
            running.stop.store(true, Ordering::Relaxed);
            running.answer = None;
        }
    }
}

/// Take the script's place for one request from a program driving Lumit, so
/// File > Run script waits for it, it never lands inside a script's undo
/// step, and Stop reaches it through `stop`. `None` while a script runs.
pub(crate) fn hold(stop: Arc<AtomicBool>) -> Option<Holding> {
    let mut running = RUNNING.lock().ok()?;
    if running.is_some() {
        return None;
    }
    *running = Some(Running { stop, answer: None });
    Some(Holding)
}

/// The place [`hold`] took, given back when this goes.
pub(crate) struct Holding;

impl Drop for Holding {
    fn drop(&mut self) {
        if let Ok(mut running) = RUNNING.lock() {
            *running = None;
        }
    }
}

/// The file the running script asked for, or `None` when the person
/// cancelled.
pub(crate) fn answer(path: Option<PathBuf>) {
    let waiting = RUNNING
        .lock()
        .ok()
        .and_then(|mut running| running.as_mut()?.answer.take());
    if let Some(waiting) = waiting {
        let _ = waiting.send(path);
    }
}

fn run_file(
    project: ProjectReference,
    store: Arc<DocumentStore>,
    path: &Path,
    comp: Option<Uuid>,
    stop: Arc<AtomicBool>,
    tell: &dyn Fn(BridgeScriptEvent) -> bool,
) -> BridgeScriptEvent {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut source = String::new();
    let read = std::fs::File::open(path)
        .and_then(|f| f.take(MAX_SOURCE_BYTES + 1).read_to_string(&mut source));
    if read.is_err() || source.len() as u64 > MAX_SOURCE_BYTES {
        return finished(Vec::new(), "The script could not be read.", None);
    }
    let mut host = AppHost::new(project, store, stop.clone(), tell);
    host.active = comp;
    let choose = |title: &str, extensions: &[String]| host.choose_file(title, extensions);
    let alert = |message: &str| {
        host.say(BridgeScriptEvent::Alert {
            message: message.into(),
        });
    };
    let report = lumit_script::run(
        &host,
        &source,
        RunOptions {
            name,
            stop: stop.clone(),
            choose_file: &choose,
            path: Some(path.to_path_buf()),
            alert: Some(&alert),
            ..Default::default()
        },
    );
    match report.result {
        Ok(_) => BridgeScriptEvent::Finished {
            log: report.log,
            stopped: false,
            error: None,
            line: None,
        },
        Err(e) => BridgeScriptEvent::Finished {
            log: report.log,
            stopped: e.error == ScriptError::Stopped,
            error: Some(e.error.to_string()),
            line: e.line.and_then(|l| u32::try_from(l).ok()),
        },
    }
}

fn finished(log: Vec<String>, error: &str, line: Option<u32>) -> BridgeScriptEvent {
    BridgeScriptEvent::Finished {
        log,
        stopped: false,
        error: Some(error.into()),
        line,
    }
}

/// The open project, as a script sees it.
pub(crate) struct AppHost<'a> {
    pub(crate) project: ProjectReference,
    store: Arc<DocumentStore>,
    /// The export this run started last. Lumit exports one thing at a time.
    export: RefCell<Option<Export>>,
    stop: Arc<AtomicBool>,
    /// The interface, which hears [`BridgeScriptEvent`]s.
    tell: &'a dyn Fn(BridgeScriptEvent) -> bool,
    /// The comp open in the Viewer when the run started.
    pub(crate) active: Option<Uuid>,
}

impl<'a> AppHost<'a> {
    /// The open `project`, whose store is `store`, for a run that `stop`
    /// stops and that tells the interface through `tell`.
    pub(crate) fn new(
        project: ProjectReference,
        store: Arc<DocumentStore>,
        stop: Arc<AtomicBool>,
        tell: &'a dyn Fn(BridgeScriptEvent) -> bool,
    ) -> Self {
        Self {
            project,
            store,
            export: RefCell::new(None),
            stop,
            tell,
            active: None,
        }
    }

    /// Tell the interface something: a message to show, a comp to open in
    /// the Viewer, a file to ask for. False when nobody is listening.
    fn say(&self, event: BridgeScriptEvent) -> bool {
        (self.tell)(event)
    }

    /// Ask the interface for a file and wait for the answer, which Stop also
    /// gives, as `None`.
    fn choose_file(&self, title: &str, extensions: &[String]) -> Option<PathBuf> {
        let (answer, answered) = mpsc::sync_channel(1);
        {
            let mut running = RUNNING.lock().ok()?;
            // Under the lock, so a Stop cannot land between this and the wait.
            if self.stop.load(Ordering::Relaxed) {
                return None;
            }
            running.as_mut()?.answer = Some(answer);
        }
        let asked = self.say(BridgeScriptEvent::ChooseFile {
            title: title.into(),
            extensions: extensions.to_vec(),
        });
        if !asked {
            return None;
        }
        answered.recv().ok().flatten()
    }
}

struct Export {
    job: u64,
    path: PathBuf,
    /// Whether the script asked it to stop.
    cancelled: bool,
    /// Frames to write, once the exporter has said.
    total: u64,
}

fn failed(message: &str) -> ScriptError {
    ScriptError::Failed(message.into())
}

impl Host for AppHost<'_> {
    fn store(&self) -> &DocumentStore {
        &self.store
    }

    fn path(&self) -> Option<PathBuf> {
        self.project.path().ok().flatten().map(PathBuf::from)
    }

    fn save(&self, path: Option<&Path>) -> Result<PathBuf, ScriptError> {
        let to = path.map(|p| p.to_string_lossy().into_owned());
        match self.project.save(to.unwrap_or_default()) {
            Ok(written) => Ok(PathBuf::from(written)),
            Err(BridgeError::NoProjectPath) => Err(ScriptError::BadArgs(
                "The project has not been saved yet, so it needs a path.".into(),
            )),
            Err(_) => Err(failed("The project could not be saved.")),
        }
    }

    fn import_footage(&self, paths: &[PathBuf]) -> Result<Vec<Uuid>, ScriptError> {
        // Each file is its own commit, and the call is one undo step.
        self.store.begin_undo_group();
        let ids = paths
            .iter()
            .map(|path| {
                if !path.exists() {
                    return Err(ScriptError::NotFound(format!(
                        "{} could not be found.",
                        path.display()
                    )));
                }
                self.project
                    .import_footage(path.to_string_lossy().into_owned())
                    .map(|footage| footage.id())
                    .map_err(|_| {
                        ScriptError::Failed(format!("{} could not be imported.", path.display()))
                    })
            })
            .collect();
        self.store.end_undo_group();
        ids
    }

    fn footage_layers(&self, comp: &Composition, item: Uuid) -> Result<Vec<Layer>, ScriptError> {
        let state = self
            .project
            .state()
            .map_err(|_| failed("The project is closed."))?;
        // Probed first and kept, so reading the file is not done under the
        // project's lock.
        #[cfg(feature = "media")]
        {
            let source = {
                let state = state.read().map_err(|_| failed("The project is closed."))?;
                match state.store.snapshot().item(item) {
                    Some(lumit_core::model::ProjectItem::Footage(f)) => {
                        crate::api::footage::FootageReference::resolve_source(&state, f)
                    }
                    _ => None,
                }
            };
            if let Some(source) = source {
                let _ = crate::probe::ensure_probed(source);
            }
        }
        let state = state.read().map_err(|_| failed("The project is closed."))?;
        let doc = state.store.snapshot();
        let (layer, beside) = CompositionReference::footage_layers(
            &state,
            &doc,
            comp,
            item,
            false,
            false,
            BridgeStreamsArrival::First,
        )
        .map_err(|_| failed("That footage cannot be placed."))?;
        Ok(std::iter::once(layer).chain(beside).collect())
    }

    fn export_start(&self, comp: Uuid, out: Option<&Path>) -> Result<u64, ScriptError> {
        let spec = crate::export::from_export_spec(&lumit_render::export::ExportSpec::default())
            .ok_or_else(|| failed("This build of Lumit cannot export."))?;
        let reference = CompositionReference::new(self.project.id, comp);
        let out = match out {
            Some(out) => out.to_string_lossy().into_owned(),
            None => {
                let project = self.path().ok_or_else(|| {
                    ScriptError::BadArgs(
                        "The project has not been saved yet, so the export needs a path.".into(),
                    )
                })?;
                let name = reference
                    .composition()
                    .map(|c| c.name)
                    .map_err(|_| failed("That comp has gone."))?;
                let file = crate::api::export::export_preset(String::new(), name, "{comp}".into())
                    .default_name;
                project.with_file_name(file).to_string_lossy().into_owned()
            }
        };
        // Never over a file that is there already.
        let out = crate::export::with_format_extension(&out, &spec.codec);
        let out = if crate::export::path_taken(&out, &spec.codec) {
            crate::export::free_path(&out, &spec.codec)
        } else {
            out
        };
        reference
            .start_export(spec, out.clone())
            .map_err(|e| match e {
                BridgeError::ExportFailed(why) => ScriptError::Failed(why),
                BridgeError::AddonMissing => {
                    failed("The comp needs an addon that is not installed.")
                }
                _ => failed("The export could not start."),
            })?;
        let mut export = self.export.borrow_mut();
        let job = export.as_ref().map_or(1, |e| e.job + 1);
        *export = Some(Export {
            job,
            path: PathBuf::from(out),
            cancelled: false,
            total: 0,
        });
        Ok(job)
    }

    fn export_status(&self, job: u64) -> Result<ExportStatus, ScriptError> {
        let mut export = self.export.borrow_mut();
        let export = export
            .as_mut()
            .filter(|e| e.job == job)
            .ok_or_else(|| ScriptError::NotFound(format!("There is no export {job}.")))?;
        let (state, done, message) = match crate::api::export::export_poll() {
            BridgeExportState::Idle => (ExportState::Running, 0, None),
            BridgeExportState::Running { frame, total, .. } => {
                export.total = total;
                (ExportState::Running, frame, None)
            }
            BridgeExportState::Done { path } => {
                export.path = PathBuf::from(path);
                (ExportState::Done, export.total, None)
            }
            BridgeExportState::Failed { .. } if export.cancelled => {
                (ExportState::Cancelled, 0, None)
            }
            BridgeExportState::Failed { error } => (ExportState::Failed, 0, Some(error)),
        };
        Ok(ExportStatus {
            state,
            done,
            total: export.total,
            path: export.path.clone(),
            message,
        })
    }

    fn export_cancel(&self, job: u64) -> Result<(), ScriptError> {
        let mut export = self.export.borrow_mut();
        let export = export
            .as_mut()
            .filter(|e| e.job == job)
            .ok_or_else(|| ScriptError::NotFound(format!("There is no export {job}.")))?;
        export.cancelled = true;
        crate::api::export::export_cancel();
        Ok(())
    }

    fn open_comp(&self, comp: Uuid) -> Result<(), ScriptError> {
        self.say(BridgeScriptEvent::OpenComp {
            comp: CompositionReference::new(self.project.id, comp),
        });
        Ok(())
    }

    fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    fn active_comp(&self) -> Option<Uuid> {
        self.active
    }

    fn parent_ops(
        &self,
        comp: Uuid,
        layer: Uuid,
        parent: Option<Uuid>,
        at: CompTime,
    ) -> Vec<lumit_core::Op> {
        let doc = self.store().snapshot();
        lumit_render::build::parent_ops(&doc, comp, layer, parent, at.0.to_f64())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::api::state::LumitBridgeState;

    // Stop pressed while a script waits for a file must end the run, not
    // leave it waiting for an answer that never comes.
    #[test]
    #[allow(clippy::expect_used)]
    fn stop_ends_a_script_waiting_for_a_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("wait.js");
        std::fs::write(
            &path,
            "lumit.chooseFile(\"Pick\", [\"json\"]);
while (true) {}
",
        )
        .expect("script written");
        let project = LumitBridgeState::new_project(None).expect("a new project");
        let (heard, hearing) = mpsc::channel();
        let started =
            start(project, path, None, move |event| heard.send(event).is_ok()).expect("started");
        assert!(started);
        let wait = Duration::from_secs(20);
        let asked = hearing.recv_timeout(wait).expect("asked for a file");
        assert!(matches!(asked, BridgeScriptEvent::ChooseFile { .. }));
        stop();
        let ended = hearing.recv_timeout(wait).expect("the run ended");
        assert!(matches!(
            ended,
            BridgeScriptEvent::Finished { stopped: true, .. }
        ));
    }
}
