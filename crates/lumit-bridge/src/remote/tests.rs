//! Two computers in one process, each with its own folders, saying to each
//! other what the room would pass on, over a link on this machine.

use super::*;
use lumit_media::encode::{ColourTags, Encoder, Metadata, VideoCodec, VideoSettings};

fn until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(90), "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A clip of `frames` frames of noise, which no encoder makes small.
fn clip(path: &Path, frames: u32, seed: u32) {
    let (width, height) = (320, 240);
    let settings = VideoSettings {
        codec: VideoCodec::H264,
        width,
        height,
        fps_num: 30,
        fps_den: 1,
        bit_rate: None,
        max_rate: None,
        colour: ColourTags::default(),
        depth: lumit_media::encode::BitDepth::Eight,
    };
    let mut encoder = Encoder::open(path, Some(&settings), None, &Metadata::new()).unwrap();
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    for _ in 0..frames {
        let frame: Vec<u8> = (0..width * height * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 24) as u8
            })
            .collect();
        encoder.write_rgba(&frame).unwrap();
    }
    encoder.finish().unwrap();
}

/// Where what one computer says goes: the other, while it is there.
type Slot = Arc<Mutex<Option<Arc<Remote>>>>;

/// What a computer said, kept for a test to read.
type Heard = Arc<Mutex<Vec<BridgeRemoteSay>>>;

struct Rig {
    dir: tempfile::TempDir,
    sender: Slot,
    worker: Slot,
    said_by_worker: Heard,
    project: crate::api::project::ProjectReference,
    comp: CompositionReference,
    /// The second clip, which the worker has already.
    second: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        for slot in [&self.sender, &self.worker] {
            if let Some(remote) = slot.lock().unwrap().take() {
                remote.shut();
            }
        }
        let _ = self.project.close();
        crate::footage::LIMITS.set(0, 0);
    }
}

impl Rig {
    fn sender(&self) -> Arc<Remote> {
        self.sender.lock().unwrap().clone().unwrap()
    }

    fn worker(&self) -> Arc<Remote> {
        self.worker.lock().unwrap().clone().unwrap()
    }

    fn send(&self) -> Uuid {
        let out = self.dir.path().join("out.mp4");
        let spec = BridgeExportSpec {
            range_start_frame: 0,
            range_end_frame: 6,
            ..BridgeExportSpec::default()
        };
        let job = self
            .sender()
            .send(
                &self.comp,
                "W",
                &spec,
                &out.to_string_lossy(),
                job::Lands::Sender,
            )
            .unwrap();
        job.parse().unwrap()
    }

    /// Send `N` jobs that make the worker take the first clip. The sender is
    /// away until it has worked out their files, and the clip then goes
    /// where the worker won't find it at the sender's place, as it would on
    /// one computer. [`Rig::put_back`] puts it back.
    fn send_unseen<const N: usize>(&self) -> [Uuid; N] {
        let sender = self.sender();
        listed(&sender, "S", "W", false);
        let jobs: [Uuid; N] = std::array::from_fn(|_| self.send());
        until("the files are worked out", || {
            let done = |j: &Uuid| sender.job(*j).is_some_and(|j| !j.manifest.hash.is_empty());
            jobs.iter().all(done)
        });
        let moved = self.dir.path().join("moved.mp4");
        std::fs::rename(self.dir.path().join("clip.mp4"), &moved).unwrap();
        for held in sender.holding.lock().unwrap().values_mut() {
            let mut files = (**held).clone();
            for (path, _) in files.values_mut() {
                if path.file_name() == Some("clip.mp4".as_ref()) {
                    path.clone_from(&moved);
                }
            }
            *held = Arc::new(files);
        }
        listed(&sender, "S", "W", true);
        jobs
    }

    /// The first clip back where the project has it.
    fn put_back(&self) {
        let path = self.dir.path();
        std::fs::rename(path.join("moved.mp4"), path.join("clip.mp4")).unwrap();
    }

    fn stage(&self, of: &Arc<Remote>, job: Uuid) -> Stage {
        of.job(job).map_or(Stage::Checking, |j| j.stage)
    }

    /// Hold the worker's item in the queue once it has one, so nothing here
    /// starts an export.
    fn hold_item(&self, job: Uuid) -> u32 {
        let worker = self.worker();
        until("the worker takes the job", || {
            worker.job(job).is_some_and(|j| j.item != 0)
        });
        let item = worker.job(job).unwrap().item;
        crate::export::queue_pause(item, true);
        item
    }
}

/// Start one computer, kept under `root`, as the account's computer `me`.
fn computer(root: &Path, me: &str, other: &str, takes: bool) -> Arc<Remote> {
    let remote = Remote::start(root.to_path_buf());
    *remote.account.lock().unwrap() = "account".into();
    remote.takes.store(takes, Ordering::Relaxed);
    listed(&remote, me, other, true);
    remote
}

/// What the room says to `remote`: it is `me`, and `other` is online or not.
fn listed(remote: &Remote, me: &str, other: &str, online: bool) {
    remote.hear(&serde_json::json!({
        "type": "computers",
        "computers": [
            { "id": me, "me": true, "online": true },
            { "id": other, "name": other, "online": online, "takes": true, "near": true,
              "key": "ab".repeat(32) },
        ],
    }));
}

/// Pass what `from` says to whoever is in `to`.
fn wire(from: &Arc<Remote>, me: &str, to: &Slot, heard: Option<Heard>) {
    let (to, me) = (to.clone(), me.to_owned());
    from.set_outbox(Box::new(move |say: BridgeRemoteSay| {
        if let Some(heard) = &heard {
            heard.lock().unwrap().push(say.clone());
        }
        let Some(to) = to.lock().unwrap().clone() else {
            return;
        };
        if say.reply {
            to.reply(&say.id, &say.body, &say.error);
        } else {
            to.command(&say.id, &me, &say.body);
        }
    }));
}

fn rig(takes: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (dir.path().join("clip.mp4"), dir.path().join("second.mp4"));
    clip(&first, 30, 1);
    clip(&second, 6, 2);
    let project = crate::api::state::LumitBridgeState::new_project(None).unwrap();
    let comp = project.new_composition("Scene".into(), None).unwrap();
    for file in [&first, &second] {
        let placed = project
            .import_footage(file.to_string_lossy().into_owned())
            .unwrap();
        comp.add_footage_layer(&placed, false, None, None).unwrap();
    }
    let (sender, worker): (Slot, Slot) = (Arc::default(), Arc::default());
    let s = computer(&dir.path().join("sender"), "S", "W", true);
    let w = computer(&dir.path().join("worker"), "W", "S", takes);
    let said_by_worker = Heard::default();
    wire(&s, "S", &worker, None);
    wire(&w, "W", &sender, Some(said_by_worker.clone()));
    *sender.lock().unwrap() = Some(s);
    *worker.lock().unwrap() = Some(w);
    Rig {
        dir,
        sender,
        worker,
        said_by_worker,
        project,
        comp,
        second,
    }
}

/// The worker takes only the file it lacks, checks it, and lays the job out
/// for its export, whose item then waits its turn in the queue. Two jobs of
/// one project sent at once take the same file without spoiling it.
#[test]
fn a_job_sends_only_what_the_worker_lacks() {
    let _queue = crate::api::tests::export_queue_test();
    let rig = rig(true);
    // The worker has the second clip in its store already.
    let worker = rig.worker();
    let hash = |path: &Path| files::hash_file(path, &|| false, &mut |_| {}).unwrap();
    let kept = files::stored(&worker.root, &hash(&rig.second), "second.mp4");
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::copy(&rig.second, &kept).unwrap();

    let [job, twin] = rig.send_unseen::<2>();
    let clip = rig.dir.path().join("moved.mp4");
    let items = [rig.hold_item(job), rig.hold_item(twin)];
    until("the worker has every file", || {
        rig.stage(&worker, job) == Stage::Queued && rig.stage(&worker, twin) == Stage::Queued
    });

    // What each took is the first clip, or nothing when the other had it
    // in the store by then, and never the second.
    let first = std::fs::metadata(&clip).unwrap().len();
    let wanted = [hash(&clip), hash(&rig.second)];
    for (job, item) in [job, twin].into_iter().zip(items) {
        assert!([0, first].contains(&worker.job(job).unwrap().size));
        let folder = job::folder(&worker.root, job);
        let order = std::fs::read(folder.join("job.json")).unwrap();
        let order: Value = serde_json::from_slice(&order).unwrap();
        let references = order["references"].as_object().unwrap();
        let mut laid: Vec<String> = references
            .values()
            .map(|path| {
                let path = Path::new(path.as_str().unwrap());
                assert!(path.starts_with(&folder), "{path:?}");
                hash(path)
            })
            .collect();
        laid.sort();
        let mut expected = wanted.to_vec();
        expected.sort();
        assert_eq!(laid, expected, "the files laid out are the ones sent");
        let row = crate::export::queue_list()
            .into_iter()
            .find(|r| r.id == item)
            .unwrap();
        assert_eq!(row.job, Some(job));
        assert!(matches!(row.state, crate::export::QueueRowState::Waiting));
    }
    assert!(files::in_store(&worker.root, &wanted[0], first).is_some());
    until("the sender hears so", || {
        rig.stage(&rig.sender(), job) == Stage::Queued
    });
    for item in items {
        crate::export::queue_remove(item);
    }
}

/// The whole of it: the worker's own `lumit-cli` exports the job and the
/// file comes back to where the sender asked for it, as big as the same
/// export made here. It needs a graphics card and the command line built:
/// `cargo build -p lumit-cli -j 6`, then
/// `cargo test -p lumit_bridge --lib remote::tests::a_job_comes_back -- --ignored`.
#[test]
#[ignore = "runs a real export in lumit-cli"]
fn a_job_comes_back_as_the_same_file() {
    let _queue = crate::api::tests::export_queue_test();
    let rig = rig(true);
    let sender = rig.sender();
    let job = rig.send();
    until("the file comes back", || {
        let stage = rig.stage(&sender, job);
        assert!(
            !matches!(stage, Stage::Failed | Stage::Cancelled),
            "{:?}",
            sender.job(job)
        );
        stage == Stage::Done
    });
    let landed = PathBuf::from(sender.job(job).unwrap().path);
    assert_eq!(landed, rig.dir.path().join("out.mp4"));

    // The same export, here.
    let here = rig.dir.path().join("here.mp4");
    let doc = {
        let state = rig.comp.project().unwrap();
        let state = state.read().unwrap();
        state.store.snapshot()
    };
    let spec = BridgeExportSpec {
        range_start_frame: 0,
        range_end_frame: 6,
        ..BridgeExportSpec::default()
    };
    let id = crate::export::queue_add(
        doc,
        rig.comp.id,
        "Scene".into(),
        &spec,
        &here.to_string_lossy(),
        true,
    )
    .unwrap();
    until("the export here is done", || {
        let rows = crate::export::queue_list();
        let row = rows.iter().find(|r| r.id == id).unwrap();
        assert!(
            !matches!(row.state, crate::export::QueueRowState::Failed(_)),
            "{row:?}"
        );
        matches!(row.state, crate::export::QueueRowState::Done { .. })
    });
    let size = |p: &Path| std::fs::metadata(p).unwrap().len();
    assert_eq!(size(&landed), size(&here));
    // The worker let its copy go once the sender had it.
    let worker = rig.worker();
    until("the worker hears it landed", || {
        rig.stage(&worker, job) == Stage::Done
    });
}

/// Nothing of a cancelled job is left in its folder on the worker.
fn left(worker: &Remote, job: Uuid) -> Vec<String> {
    let folder = job::folder(&worker.root, job);
    let names = std::fs::read_dir(folder).into_iter().flatten().flatten();
    let names = names.map(|e| e.file_name().to_string_lossy().into_owned());
    names.filter(|n| n != "record.json").collect()
}

/// A cancel from the sender while files move, and from the worker's queue
/// while the job waits there, ends the job on both computers.
#[test]
fn a_cancel_on_either_computer_ends_the_job_on_both() {
    let _queue = crate::api::tests::export_queue_test();
    let rig = rig(true);
    let (sender, worker) = (rig.sender(), rig.worker());
    // Slow, so the cancel comes while the files move.
    crate::footage::LIMITS.set(0, 64 << 10);
    let [job] = rig.send_unseen();
    rig.hold_item(job);
    until("files move", || {
        worker.job(job).is_some_and(|j| j.bytes > 0)
    });
    sender.cancel_here(job, true);
    until("both have cancelled", || {
        rig.stage(&sender, job) == Stage::Cancelled && rig.stage(&worker, job) == Stage::Cancelled
    });
    until("the links stop", || {
        sender.link(job).is_none() && worker.link(job).is_none()
    });
    until("the worker's folder is empty", || {
        left(&worker, job).is_empty()
    });

    crate::footage::LIMITS.set(0, 0);
    rig.put_back();
    let job = rig.send();
    let item = rig.hold_item(job);
    until("the job waits in the queue", || {
        rig.stage(&worker, job) == Stage::Queued
    });
    // The queue window's cross.
    crate::export::queue_cancel(item);
    until("both have cancelled", || {
        rig.stage(&sender, job) == Stage::Cancelled && rig.stage(&worker, job) == Stage::Cancelled
    });
    until("the worker's folder is empty", || {
        left(&worker, job).is_empty()
    });
}

/// A sender that quits while its files move takes the job up again from its
/// record when it starts, and the worker gets every file.
#[test]
fn a_job_carries_on_after_the_sender_starts_again() {
    let _queue = crate::api::tests::export_queue_test();
    let rig = rig(true);
    let worker = rig.worker();
    crate::footage::LIMITS.set(0, 64 << 10);
    let [job] = rig.send_unseen();
    let item = rig.hold_item(job);
    until("files move", || {
        worker.job(job).is_some_and(|j| j.bytes > 0)
    });
    // The sender quits, and comes back with the clip where its project has
    // it.
    let gone = rig.sender.lock().unwrap().take().unwrap();
    gone.write_down();
    gone.shut();
    drop(gone);
    std::thread::sleep(Duration::from_millis(500));
    crate::footage::LIMITS.set(0, 0);
    rig.put_back();
    let back = computer(&rig.dir.path().join("sender"), "S", "W", true);
    wire(&back, "S", &rig.worker, None);
    *rig.sender.lock().unwrap() = Some(back.clone());
    assert!(back
        .job(job)
        .is_some_and(|j| j.accepted && !j.stage.ended()));
    until("the worker has every file", || {
        rig.stage(&worker, job) == Stage::Queued
    });
    // The clip carried on across the two links is the clip, whole.
    let hash = |path: &Path| files::hash_file(path, &|| false, &mut |_| {}).unwrap();
    let clip = hash(&rig.dir.path().join("clip.mp4"));
    assert!(files::in_store(
        &worker.root,
        &clip,
        std::fs::metadata(rig.dir.path().join("clip.mp4"))
            .unwrap()
            .len()
    )
    .is_some());
    until("the sender hears so", || {
        rig.stage(&back, job) == Stage::Queued
    });
    crate::export::queue_remove(item);
}

/// A worker that doesn't take exports says so, and so does one that can't
/// read the project.
#[test]
fn a_worker_refuses_what_it_cannot_take() {
    let _queue = crate::api::tests::export_queue_test();
    let rig = rig(false);
    let sender = rig.sender();
    let job = rig.send();
    until("the offer is refused", || {
        rig.stage(&sender, job) == Stage::Failed
    });
    assert_eq!(sender.job(job).unwrap().error, "turned_off");

    let worker = rig.worker();
    worker.takes.store(true, Ordering::Relaxed);
    let offer = serde_json::json!({
        "lumit": "remote-offer", "job": Uuid::now_v7(), "key": "cd".repeat(32),
        "schema": "99.0.0", "needs": {}, "bytes": 1,
    });
    worker.command("asked", "S", &offer.to_string());
    until("the offer is answered", || {
        rig.said_by_worker
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.id == "asked" && s.reply)
    });
    let said = rig.said_by_worker.lock().unwrap();
    let answer = said.iter().find(|s| s.id == "asked").unwrap();
    assert_eq!(answer.error, "too_old");
}
