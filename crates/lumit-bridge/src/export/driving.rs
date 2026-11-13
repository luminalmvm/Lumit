use super::{err_json, to_export_spec, with_format_extension, BridgeExportSpec};
use lumit_render::export::{ExportEvent, ExportHandle};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// The terminal/progress state a poll reports, held between polls.
enum State {
    Idle,
    Running {
        frame: usize,
        total: usize,
        encoder: Option<String>,
    },
    Done {
        path: String,
    },
    Failed {
        error: String,
    },
}

/// What an item exports.
enum Source {
    /// A composition of this document, snapshotted when the item was added
    /// (docs/06 §7.1), so later edits never alter what it renders.
    Document(std::sync::Arc<lumit_core::model::Document>),
    /// A remote export another computer sent: its job, and the folder its
    /// files are gathered in. It runs in a `lumit-cli` of its own, and only
    /// once `ready` says the files are all here.
    Job {
        job: Uuid,
        folder: PathBuf,
        ready: bool,
    },
}

/// One item waiting its turn.
struct Item {
    id: u32,
    comp_name: String,
    source: Source,
    comp: Uuid,
    /// The dialogue's fields as they stood when this was queued — the
    /// *when done* ticks included, which is why the queue can honour them
    /// after the dialogue that set them has closed.
    spec: BridgeExportSpec,
    out_path: String,
    state: ItemState,
    /// Held by someone. Waiting, it is passed over; running, it waits
    /// between two frames.
    paused: bool,
    /// When it started encoding, for how long it took.
    started: Option<std::time::Instant>,
    /// Edits made to `doc` when the item starts, and not before: one row's
    /// version of a graphics template. So many rows queued together share
    /// one snapshot, and are not each a project in memory. Empty for every
    /// other item.
    version: Vec<lumit_core::ops::Op>,
    /// The run of rows this item was queued with, which the queue folds
    /// under one heading.
    group: Option<u32>,
}

/// Where an item is in its life. Progress is not held here: the in-flight
/// item's frame count is [`State::Running`], one place rather than two that
/// could disagree.
pub(super) enum ItemState {
    Waiting,
    Running,
    /// How long it took, and the file's size where it is one file.
    Done {
        seconds: u32,
        bytes: u64,
    },
    Failed(String),
}

/// The one in-flight export: its state plus the handle whose receiver a poll
/// drains, or the child process a job runs in. Either is dropped once its
/// export has ended.
///
/// The queue lives here too rather than in a slot of its own, because "is
/// anything running" is the question both halves ask: one lock answers it
/// for the export in flight and for the items waiting behind it.
struct Run {
    state: State,
    handle: Option<ExportHandle>,
    child: Option<super::child::Child>,
    queue: Vec<Item>,
    /// What each run of rows is called, by its number.
    groups: Vec<(u32, String)>,
    next_id: u32,
    /// Whether the queue may start things. Adding holds by default —
    /// "Add to queue" is a list of work, not a start button — and Export
    /// (and the queue window's own action) sets it.
    running: bool,
    /// The queued item the handle belongs to, when the export in flight
    /// came from the queue rather than from a direct start.
    current: Option<u32>,
    /// A direct start whose end nobody has polled yet. The queue starts
    /// nothing meanwhile, or the next item would take over the state its
    /// poll is waiting to read.
    direct_unread: bool,
    /// Keeps the computer awake while an export is in flight.
    awake: Option<crate::awake::Awake>,
}

impl Run {
    fn in_flight(&self) -> bool {
        self.handle.is_some() || self.child.is_some()
    }
}

static EXPORT: OnceLock<Mutex<Run>> = OnceLock::new();

fn slot() -> &'static Mutex<Run> {
    EXPORT.get_or_init(|| {
        Mutex::new(Run {
            state: State::Idle,
            handle: None,
            child: None,
            queue: Vec::new(),
            groups: Vec::new(),
            next_id: 1,
            running: false,
            current: None,
            direct_unread: false,
            awake: None,
        })
    })
}

/// The export itself, given the document to render.
///
/// Split out from [`start`] so the frb path can drive the same exporter: v0
/// reads its document from the process-wide bridge, and an frb project is
/// not in it. Everything after this point is shared.
pub(super) fn start_with_document(
    doc: std::sync::Arc<lumit_core::model::Document>,
    comp: Uuid,
    spec: &BridgeExportSpec,
    out_path: &str,
) -> String {
    if out_path.trim().is_empty() {
        return err_json("export: no output path");
    }

    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    // Drain first so a just-finished export frees the slot for a new one.
    drain(&mut guard);
    if guard.in_flight() {
        return err_json("an export is already running");
    }

    let out_path = with_format_extension(out_path, &spec.codec);
    match launch(&mut guard, &doc, comp, spec, &out_path) {
        Ok(()) => {
            guard.current = None;
            guard.direct_unread = true;
            keep_awake(&mut guard);
            json!({ "ok": true }).to_string()
        }
        Err(e) => err_json(e),
    }
}

/// Start one export against the slot: resolve its spec, build its inputs
/// over the headless seam, and hand it to the exporter. The caller has
/// already established that nothing is in flight.
///
/// Split out so the queue and a direct start cannot drift apart — there is
/// one way to begin an export, whoever asked for it.
fn launch(
    run: &mut Run,
    doc: &std::sync::Arc<lumit_core::model::Document>,
    comp: Uuid,
    spec: &BridgeExportSpec,
    out_path: &str,
) -> Result<(), String> {
    // The comp's own size, which the crop is resolved against.
    let (cw, ch) = doc
        .comp(comp)
        .map(|c| (c.width, c.height))
        .ok_or("export: unknown composition")?;
    #[cfg(target_arch = "wasm32")]
    let codec = spec.codec.clone();
    let spec = to_export_spec(spec, cw, ch)?;

    // Build the audio inputs through the headless seam, then hand
    // off to the exporter, which drives the same render walk the Viewer
    // uses on its own thread and device.
    //
    // The inputs are gathered from the *delivery* snapshot, not the
    // project: a guide layer is reference-only, so its sound is no more
    // delivered than its picture — and "solo ignored" reaches the mixdown
    // for the same reason, in an export that has a picture at all. The
    // options come through `render_options` so a sound-only export takes
    // the defaults instead: it draws nothing, so it takes no picture
    // settings. The exporter asks the same question of the same spec and
    // applies the same overrides again, a no-op the second time.
    let delivery = lumit_render::export::apply_render_overrides(doc, &spec.render_options());
    let doc = delivery.as_ref().unwrap_or(doc);
    let inputs = crate::render::with_export_inputs(doc, comp)
        .ok_or("export: the GPU pipeline is unavailable")?;
    #[cfg(target_arch = "wasm32")]
    let handle = super::web::start(doc.clone(), comp, &inputs.audio, out_path, spec, &codec)?;
    #[cfg(not(target_arch = "wasm32"))]
    let handle = lumit_render::export::start(
        doc.clone(),
        comp,
        inputs.audio,
        std::path::PathBuf::from(out_path),
        spec,
    );

    run.state = State::Running {
        frame: 0,
        total: 0,
        encoder: None,
    };
    run.handle = Some(handle);
    Ok(())
}

pub(super) fn queue_add(
    doc: std::sync::Arc<lumit_core::model::Document>,
    comp: Uuid,
    comp_name: String,
    spec: &BridgeExportSpec,
    out_path: &str,
    start: bool,
) -> Result<u32, String> {
    queue_add_version(
        doc,
        comp,
        comp_name,
        spec,
        out_path,
        Vec::new(),
        None,
        start,
    )
}

/// A number for a run of rows queued together, called `name`.
pub(super) fn queue_group(name: String) -> u32 {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let id = guard.next_id;
    guard.next_id += 1;
    guard.groups.push((id, name));
    id
}

/// [`queue_add`] for one row's version of a template: `version` is applied
/// to `doc` when the item starts.
#[allow(clippy::too_many_arguments)]
pub(super) fn queue_add_version(
    doc: std::sync::Arc<lumit_core::model::Document>,
    comp: Uuid,
    comp_name: String,
    spec: &BridgeExportSpec,
    out_path: &str,
    version: Vec<lumit_core::ops::Op>,
    group: Option<u32>,
    start: bool,
) -> Result<u32, String> {
    if out_path.trim().is_empty() {
        return Err("export: no output path".to_owned());
    }
    let out_path = with_format_extension(out_path, &spec.codec);
    Ok(add(
        Source::Document(doc),
        comp,
        comp_name,
        spec,
        out_path,
        version,
        group,
        start,
    ))
}

/// Add the remote export `job`, whose files are gathered in `folder`, as an
/// item that waits its turn and is passed over until [`job_ready`]. The queue
/// is let run, as nobody may be at this computer to start it.
pub(super) fn queue_add_job(
    job: Uuid,
    folder: PathBuf,
    comp: Uuid,
    comp_name: String,
    spec: &BridgeExportSpec,
    out_path: String,
) -> u32 {
    let source = Source::Job {
        job,
        folder,
        ready: false,
    };
    add(
        source,
        comp,
        comp_name,
        spec,
        out_path,
        Vec::new(),
        None,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn add(
    source: Source,
    comp: Uuid,
    comp_name: String,
    spec: &BridgeExportSpec,
    out_path: String,
    version: Vec<lumit_core::ops::Op>,
    group: Option<u32>,
    start: bool,
) -> u32 {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let id = guard.next_id;
    guard.next_id += 1;
    guard.queue.push(Item {
        id,
        comp_name,
        source,
        comp,
        spec: spec.clone(),
        out_path,
        state: ItemState::Waiting,
        paused: false,
        started: None,
        version,
        group,
    });
    if start {
        guard.running = true;
    }
    pump(&mut guard);
    id
}

/// The job item `id` has all its files and may run.
pub(super) fn job_ready(id: u32) {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    for item in guard.queue.iter_mut().filter(|item| item.id == id) {
        if let Source::Job { ready, .. } = &mut item.source {
            *ready = true;
        }
    }
    pump(&mut guard);
}

pub(super) fn queue_list() -> Vec<super::QueueRow> {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    drain(&mut guard);
    pump(&mut guard);
    let running = match &guard.state {
        State::Running {
            frame,
            total,
            encoder,
        } => Some((*frame as u64, *total as u64, encoder.clone())),
        _ => None,
    };
    guard
        .queue
        .iter()
        .map(|item| super::QueueRow {
            id: item.id,
            group: item.group,
            group_name: item
                .group
                .and_then(|g| guard.groups.iter().find(|(id, _)| *id == g))
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            comp: item.comp,
            comp_name: item.comp_name.clone(),
            out_path: item.out_path.clone(),
            preset: item.spec.preset.clone(),
            codec: item.spec.codec.clone(),
            range: (item.spec.range_start_frame >= 0
                && item.spec.range_end_frame > item.spec.range_start_frame)
                .then_some((
                    item.spec.range_start_frame as u64,
                    item.spec.range_end_frame as u64,
                )),
            state: match &item.state {
                ItemState::Waiting => super::QueueRowState::Waiting,
                ItemState::Running => {
                    let (frame, total, encoder) = running.clone().unwrap_or_default();
                    super::QueueRowState::Running {
                        frame,
                        total,
                        encoder: encoder.unwrap_or_default(),
                        paused: item.paused,
                    }
                }
                ItemState::Done { seconds, bytes } => super::QueueRowState::Done {
                    seconds: *seconds,
                    bytes: *bytes,
                },
                ItemState::Failed(error) => super::QueueRowState::Failed(error.clone()),
            },
            job: match &item.source {
                Source::Job { job, .. } => Some(*job),
                Source::Document(_) => None,
            },
        })
        .collect()
}

pub(super) fn queue_start() {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    guard.running = true;
    drain(&mut guard);
    pump(&mut guard);
}

/// The pump only starts an item while the queue runs, so holding it is all
/// it takes: the export in flight carries on, and nothing follows it.
pub(super) fn queue_stop_after() {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    guard.running = false;
}

pub(super) fn queue_goes_on() -> bool {
    slot().lock().unwrap_or_else(|p| p.into_inner()).running
}

/// Stop what is in flight: the export at its next frame, or a job's child.
fn stop_current(run: &mut Run) {
    if let Some(handle) = &run.handle {
        handle.cancel();
    }
    if let Some(child) = run.child.as_mut() {
        child.stop();
    }
}

pub(super) fn queue_cancel(id: u32) {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    if guard.current == Some(id) {
        stop_current(&mut guard);
        return;
    }
    // A waiting item has nothing to stop and nothing to report, so it
    // simply leaves the list rather than sitting there as a failure.
    guard
        .queue
        .retain(|item| item.id != id || !matches!(item.state, ItemState::Waiting));
}

/// Hold item `id` where it is, or let it go on.
pub(super) fn queue_pause(id: u32, paused: bool) {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let Some(item) = guard.queue.iter_mut().find(|item| item.id == id) else {
        return;
    };
    if !matches!(item.state, ItemState::Waiting | ItemState::Running) {
        return;
    }
    item.paused = paused;
    if guard.current == Some(id) {
        if let Some(handle) = &guard.handle {
            handle.hold(paused);
        }
        if let Some(child) = guard.child.as_mut() {
            child.hold(paused);
        }
    }
    pump(&mut guard);
}

/// What [`queue_move`] says about an item it will not move, or `None` when
/// it will. Its own function so the four answers can be checked without a
/// process-wide queue — and without starting one, which on a machine with a
/// graphics card would launch a real encoder.
pub(super) fn move_refusal(state: &ItemState) -> Option<&'static str> {
    match state {
        ItemState::Waiting => None,
        ItemState::Running => Some("that export is already running"),
        ItemState::Done { .. } | ItemState::Failed(_) => Some("that export has already run"),
    }
}

/// What it says about an id the list no longer holds.
pub(super) const GONE: &str = "that export is no longer in the queue";

/// Move a waiting item to `index`. The pump is not turned here: reordering
/// changes which item is *next*, and starting one is what `queue_list`
/// already does on the interface's own tick.
pub(super) fn queue_move(id: u32, index: usize) -> Result<(), String> {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let Some(from) = guard.queue.iter().position(|item| item.id == id) else {
        return Err(GONE.to_owned());
    };
    if let Some(refusal) = move_refusal(&guard.queue[from].state) {
        return Err(refusal.to_owned());
    }
    // Past the end lands it last, which is what dragging a row off the
    // bottom of the list means.
    let to = index.min(guard.queue.len() - 1);
    let item = guard.queue.remove(from);
    guard.queue.insert(to, item);
    Ok(())
}

pub(super) fn queue_spec(id: u32) -> Option<BridgeExportSpec> {
    let guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    guard
        .queue
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.spec.clone())
}

pub(super) fn queue_update(id: u32, spec: &BridgeExportSpec, out_path: &str) -> Result<(), String> {
    if out_path.trim().is_empty() {
        return Err("export: no output path".to_owned());
    }
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let Some(item) = guard.queue.iter_mut().find(|item| item.id == id) else {
        return Err(GONE.to_owned());
    };
    if let Some(refusal) = move_refusal(&item.state) {
        return Err(refusal.to_owned());
    }
    // A remote export's settings are the computer's that sent it.
    if matches!(item.source, Source::Job { .. }) {
        return Err(GONE.to_owned());
    }
    item.spec = spec.clone();
    item.out_path = with_format_extension(out_path, &spec.codec);
    Ok(())
}

pub(super) fn queue_remove(id: u32) {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    if guard.current == Some(id) {
        stop_current(&mut guard);
        guard.current = None;
    }
    guard.queue.retain(|item| item.id != id);
}

/// Start the next waiting item when the queue is running and nothing is in
/// flight. An item that cannot start at all is marked failed and the one
/// behind it is tried, so a bad path cannot stall the whole queue. A held
/// item, and a remote export whose files are not all here, are passed over.
///
/// The pump is turned by the interface's own polling rather than by a
/// thread of its own: every caller here already holds the lock, and the
/// queue moves on the same 250 ms tick the progress does.
fn pump(run: &mut Run) {
    while !run.in_flight() && run.running && !run.direct_unread {
        let next = run.queue.iter().position(|item| {
            matches!(item.state, ItemState::Waiting)
                && !item.paused
                && !matches!(item.source, Source::Job { ready: false, .. })
        });
        let Some(index) = next else {
            break;
        };
        let item = &run.queue[index];
        let (comp, spec, out_path, id) =
            (item.comp, item.spec.clone(), item.out_path.clone(), item.id);
        let launched = match &item.source {
            Source::Document(doc) => versioned(doc, &item.version)
                .and_then(|doc| launch(run, &doc, comp, &spec, &out_path)),
            Source::Job { folder, .. } => super::child::start(folder).map(|child| {
                run.child = Some(child);
                run.state = State::Running {
                    frame: 0,
                    total: 0,
                    encoder: None,
                };
            }),
        };
        match launched {
            Ok(()) => {
                run.queue[index].state = ItemState::Running;
                run.queue[index].started = Some(std::time::Instant::now());
                run.current = Some(id);
            }
            Err(error) => run.queue[index].state = ItemState::Failed(error),
        }
    }
    keep_awake(run);
}

/// Hold the computer awake while an export is in flight, and only then.
fn keep_awake(run: &mut Run) {
    if !run.in_flight() {
        run.awake = None;
    } else if run.awake.is_none() {
        run.awake = Some(crate::awake::hold());
    }
}

/// The document an item renders: its snapshot, with its row's edits made.
fn versioned(
    doc: &std::sync::Arc<lumit_core::model::Document>,
    version: &[lumit_core::ops::Op],
) -> Result<std::sync::Arc<lumit_core::model::Document>, String> {
    if version.is_empty() {
        return Ok(doc.clone());
    }
    let mut doc = lumit_core::model::Document::clone(doc);
    for op in version {
        lumit_core::ops::apply(&mut doc, op)
            .map_err(|e| format!("export: the row could not be made: {e}"))?;
    }
    Ok(std::sync::Arc::new(doc))
}

/// Queue a finished or failed item again as it was: the same snapshot, the
/// same row. Answers the new item's id, or `None` for an id that has gone or
/// a remote export, which only the computer that sent it can send again.
pub(super) fn queue_again(id: u32) -> Option<u32> {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let at = guard.queue.iter().position(|item| item.id == id)?;
    let Source::Document(doc) = &guard.queue[at].source else {
        return None;
    };
    let doc = doc.clone();
    let new = guard.next_id;
    guard.next_id += 1;
    let was = &guard.queue[at];
    let again = Item {
        id: new,
        comp_name: was.comp_name.clone(),
        source: Source::Document(doc),
        comp: was.comp,
        spec: was.spec.clone(),
        out_path: was.out_path.clone(),
        state: ItemState::Waiting,
        paused: false,
        started: None,
        version: was.version.clone(),
        group: was.group,
    };
    guard.queue.insert(at + 1, again);
    Some(new)
}

pub(super) fn poll() -> String {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    drain(&mut guard);
    // Its end is read now, so the queue may go on.
    if matches!(guard.state, State::Done { .. } | State::Failed { .. }) {
        guard.direct_unread = false;
    }
    match &guard.state {
        State::Idle => json!({ "ok": true, "state": "idle" }).to_string(),
        State::Running {
            frame,
            total,
            encoder,
        } => json!({
            "ok": true,
            "state": "running",
            "frame": frame,
            "total": total,
            "encoder": encoder,
        })
        .to_string(),
        State::Done { path } => json!({ "ok": true, "state": "done", "path": path }).to_string(),
        State::Failed { error } => {
            json!({ "ok": true, "state": "failed", "error": error }).to_string()
        }
    }
}

pub(super) fn cancel() -> String {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    stop_current(&mut guard);
    json!({ "ok": true }).to_string()
}

/// Drain every pending exporter event into the held state. A terminal event
/// (Done/Failed) drops the handle so the slot is free for the next export.
fn drain(run: &mut Run) {
    let events: Vec<ExportEvent> = if let Some(handle) = &run.handle {
        handle.events.try_iter().collect()
    } else if let Some(child) = run.child.as_mut() {
        let out = run
            .current
            .and_then(|id| run.queue.iter().find(|item| item.id == id))
            .map(|item| PathBuf::from(&item.out_path))
            .unwrap_or_default();
        child
            .heard()
            .into_iter()
            .map(|heard| match heard {
                super::child::Heard::Progress {
                    frame,
                    total,
                    encoder,
                } => {
                    // The child names its encoder in words of its own.
                    if let (Some(label), State::Running { encoder: e, .. }) =
                        (encoder, &mut run.state)
                    {
                        *e = Some(label);
                    }
                    ExportEvent::Progress { frame, total }
                }
                super::child::Heard::Done => ExportEvent::Done(out.clone()),
                super::child::Heard::Failed(why) => ExportEvent::Failed(why),
            })
            .collect()
    } else {
        return;
    };
    let mut terminal: Option<State> = None;
    for ev in events {
        match ev {
            ExportEvent::Encoder(label) => {
                if let State::Running { encoder, .. } = &mut run.state {
                    *encoder = Some(label.to_string());
                }
            }
            ExportEvent::Progress { frame, total } => {
                if let State::Running {
                    frame: f, total: t, ..
                } = &mut run.state
                {
                    *f = frame;
                    *t = total;
                }
            }
            ExportEvent::Done(path) => {
                terminal = Some(State::Done {
                    path: path.to_string_lossy().into_owned(),
                });
            }
            ExportEvent::Failed(error) => {
                terminal = Some(State::Failed { error });
            }
        }
    }
    if let Some(state) = terminal {
        // The queued item this handle belonged to takes the same outcome:
        // the row in the queue window and the progress line in the status
        // strip are two readings of one export, never two answers.
        if let Some(id) = run.current.take() {
            if let Some(item) = run.queue.iter_mut().find(|item| item.id == id) {
                item.paused = false;
                item.state = match &state {
                    State::Failed { error } => ItemState::Failed(error.clone()),
                    // A sequence's path names no one file, so it has no size.
                    _ => ItemState::Done {
                        seconds: item.started.map_or(0, |at| {
                            u32::try_from(at.elapsed().as_secs()).unwrap_or(u32::MAX)
                        }),
                        bytes: std::fs::metadata(&item.out_path).map_or(0, |m| m.len()),
                    },
                };
                // A remote export says how it ended through its own news,
                // and its file goes where its job says.
                if matches!(item.source, Source::Document(_)) {
                    landed(item);
                }
            }
        }
        run.state = state;
        run.handle = None;
        run.child = None;
        keep_awake(run);
    }
}

/// An export of this computer's own has ended: the ticks the dialogue set,
/// and the news of it.
fn landed(item: &Item) {
    // The ticks the dialogue set, honoured here rather than by whatever
    // window happens to be watching: an export that lands after its dialogue
    // closed still makes its noise and opens its folder. Both are independent,
    // as a long export left running wants the sound *and* the folder, and the
    // noise is silent, never an error, when no sound has been supplied.
    match &item.state {
        ItemState::Done { .. } => {
            if item.spec.make_a_noise {
                lumit_render::export::play_done_sound();
            }
            if item.spec.open_folder {
                super::reveal_in_folder(&item.out_path);
            }
            crate::remote::exported(&item.comp_name, &item.out_path, None);
        }
        ItemState::Failed(error) => {
            crate::remote::exported(&item.comp_name, &item.out_path, Some(error.as_str()));
        }
        _ => {}
    }
}
