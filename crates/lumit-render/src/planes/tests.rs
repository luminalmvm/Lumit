//! The planes job, the `planes/` tier and the store, asserted against a
//! **synthetic clip and a model the test wrote down** (docs/impl/addons.md §11
//! test 6).
//!
//! No runtime, no pack and no asset: the frames arrive through [`RotoFrames`],
//! which is the seam the tracking and roto tiers already have, and the model
//! arrives through [`PlaneModel`], which is why every claim below is an
//! assertion on a CI runner that has never seen an addon.

use super::*;
use lumit_core::model::EffectParam;

const W: u32 = 32;
const H: u32 = 24;
/// What the written-down model answers at, which is deliberately **not** the
/// frame's own raster: a depth model answers at its own size and the record has
/// to carry that rather than the clip's.
const PW: u32 = 14;
const PH: u32 = 14;

/// A clip of `frames` frames, each a flat picture whose brightness is the frame
/// number, so a run that reads the wrong frame cannot pass.
struct Clip {
    frames: usize,
}

impl RotoFrames for Clip {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, W, H, 24.0)
    }

    fn rgba(&mut self, n: usize) -> Option<Vec<u8>> {
        if n >= self.frames {
            return None;
        }
        let v = (n * 10) as u8;
        Some(
            (0..(W as usize) * (H as usize))
                .flat_map(|_| [v, v, v, 255u8])
                .collect(),
        )
    }
}

/// A model whose answer is a written-down function of the frame it was handed,
/// so two runs are the same bytes and a frame's record can be checked against
/// the frame it came from.
struct Written {
    reads: usize,
}

impl PlaneModel for Written {
    fn run(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<PlaneOut, PlaneFailure> {
        assert_eq!((w, h), (W, H), "the model was handed the wrong raster");
        self.reads += 1;
        let seed = u16::from(rgba.first().copied().unwrap_or(0)) * 257;
        let mut data = Vec::with_capacity((PW as usize) * (PH as usize) * 2);
        for i in 0..(PW as usize) * (PH as usize) {
            let v = seed.wrapping_add(i as u16);
            data.extend_from_slice(&v.to_le_bytes());
        }
        Ok(PlaneOut {
            width: PW,
            height: PH,
            kind: PlaneKind::Depth,
            data,
        })
    }

    fn made_with(&self) -> String {
        "Written; depth-test 1.0; ONNX Runtime none".into()
    }
}

/// A matte model whose coverage is a box in the middle of the frame, filled
/// with the frame's own brightness, so a run that boxed the wrong rows or kept
/// the wrong frame cannot pass. At the frame's own raster, which is where a
/// matte model answers.
struct Cut {
    reads: usize,
}

/// The box [`Cut`] fills: a quarter of the frame, well inside it.
const BOX: [u32; 4] = [W / 4, H / 4, W / 2, H / 2];

impl PlaneModel for Cut {
    fn run(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<PlaneOut, PlaneFailure> {
        assert_eq!((w, h), (W, H), "the model was handed the wrong raster");
        self.reads += 1;
        let value = rgba.first().copied().unwrap_or(0).max(1);
        let mut data = vec![0u8; (w as usize) * (h as usize)];
        let [bx, by, bw, bh] = BOX;
        for y in by..by + bh {
            for x in bx..bx + bw {
                data[(y * w + x) as usize] = value;
            }
        }
        Ok(PlaneOut {
            width: w,
            height: h,
            kind: PlaneKind::Matte,
            data,
        })
    }

    fn made_with(&self) -> String {
        "Written; rvm-test 1.0; ONNX Runtime none".into()
    }
}

/// A model that will not run at all.
struct Refuses;

impl PlaneModel for Refuses {
    fn run(&mut self, _rgba: &[u8], _w: u32, _h: u32) -> Result<PlaneOut, PlaneFailure> {
        Err(PlaneFailure::ModelFailed)
    }

    fn made_with(&self) -> String {
        String::new()
    }
}

/// Tests share one process-wide cache override and one running slot, so they
/// take a lock rather than racing each other.
fn serially() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

fn settings() -> PlaneSettings {
    PlaneSettings {
        task: PlaneTask::Depth,
        arch: 0,
        identity: [7u8; 32],
        provider: 1,
        downsample: 0,
    }
}

fn fingerprint() -> Fingerprint {
    Fingerprint {
        size: 4096,
        head_tail_hash: "planes-test".into(),
        mtime_secs: 0,
    }
}

fn job(frames: usize) -> PlaneJob {
    PlaneJob {
        instance: Uuid::from_u128(11),
        key: Some(PlaneKey::new(&fingerprint(), settings())),
        settings: settings(),
        open: Box::new(move || Some(Box::new(Clip { frames }) as Box<dyn RotoFrames>)),
        analyse: true,
        owns_slot: false,
        stop_after: None,
    }
}

/// **A run reads every frame, reports as it goes, and files one record each.**
/// The whole job in one assertion: the progress the card polls, the span the
/// panel reads, and a plane per frame at the model's own raster.
#[test]
fn an_analysis_reads_the_clip_and_files_a_plane_a_frame() {
    let _guard = serially();
    set_test_cache_dir(None);
    let seen = std::sync::Mutex::new(Vec::new());
    let mut model = Written { reads: 0 };
    let (run, cancelled) = analyse(job(5), None, &mut model, &never(), &|step| {
        if let Ok(mut seen) = seen.lock() {
            seen.push(step);
        }
    })
    .expect("a run");

    assert!(!cancelled);
    assert_eq!(model.reads, 5, "one model run a frame");
    assert_eq!((run.first_frame, run.last_frame), (0, 4));
    assert_eq!(run.clip_frames, 5);
    assert!(!run.is_partial(), "the whole clip was read");
    assert_eq!(
        run.provider(),
        "Written",
        "the provenance names its provider"
    );

    let steps = seen.lock().expect("the log").clone();
    assert_eq!(
        steps,
        (1..=5)
            .map(|done| Progress::Solving { done, total: 5 })
            .collect::<Vec<_>>(),
        "the reading the card polls walks the clip"
    );

    for frame in 0..5 {
        let (w, h, kind, data) = run.plane(frame).expect("a plane a frame");
        assert_eq!((w, h), (PW, PH), "the model's own raster, not the clip's");
        assert_eq!(kind, PlaneKind::Depth);
        assert_eq!(data.len(), (PW as usize) * (PH as usize) * 2);
        // The plane of frame n is the plane the model made of frame n.
        let expect = u16::from((frame * 10) as u8) * 257;
        assert_eq!(
            u16::from_le_bytes([data[0], data[1]]),
            expect,
            "frame {frame} kept another frame's plane"
        );
    }
}

/// **Outside the analysed span there is no plane at all**, so the effect
/// passes through rather than holding a neighbour's answer (§6.1).
#[test]
fn outside_the_span_there_is_no_plane_to_hold() {
    let _guard = serially();
    set_test_cache_dir(None);
    let (run, _) =
        analyse(job(3), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a run");
    assert!(run.plane(0).is_some());
    assert!(run.plane(2).is_some());
    assert!(run.plane(3).is_none(), "a frame past the span has no plane");
    assert!(run.plane(-1).is_none());
}

/// **Two runs of the same clip through the same model are the same bytes.**
/// The model itself is not bit-reproducible across cards and drivers, which is
/// why the pack's identity is in the key and the sidecar is what an export
/// reads (§7); what *is* reproducible is everything this file does with what
/// the model answered, and that is what this pins.
#[test]
fn two_runs_of_one_clip_are_the_same_bytes() {
    let _guard = serially();
    set_test_cache_dir(None);
    let key = PlaneKey::new(&fingerprint(), settings());
    let once = analyse(job(4), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a run");
    let twice = analyse(job(4), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a run");
    assert_eq!(
        encode(key, &once.0).expect("bytes"),
        encode(key, &twice.0).expect("bytes"),
    );
}

/// **A cancel keeps the prefix it finished** (§6.1, the Roto brush's stance).
/// Every frame reached is correct and correctly named, so they are kept, the
/// span says how far it got, and a later Analyse carries on rather than
/// starting again.
#[test]
fn a_cancel_keeps_the_prefix_it_finished() {
    let _guard = serially();
    set_test_cache_dir(None);
    let stop = AtomicBool::new(false);
    let mut model = Written { reads: 0 };
    let (run, cancelled) = analyse(job(9), None, &mut model, &stop, &|step| {
        if let Progress::Solving { done, .. } = step {
            if done >= 3 {
                stop.store(true, Ordering::Relaxed);
            }
        }
    })
    .expect("a run");

    assert!(cancelled, "the run says it was stopped");
    assert_eq!((run.first_frame, run.last_frame), (0, 2));
    assert!(run.is_partial(), "and that it did not reach the end");
    assert_eq!(model.reads, 3, "nothing was read after the flag went up");
    for frame in 0..3 {
        assert!(run.plane(frame).is_some(), "frame {frame} was thrown away");
    }
    assert!(run.plane(3).is_none());
}

/// **A later Analyse carries on from a cancelled run's prefix** rather than
/// reading the whole shot again (§6.1, and the module note's own promise).
/// Every frame the first run reached is correct and correctly named, so the
/// second one starts after the last of them and the finished run is the same
/// planes either way.
#[test]
fn a_later_analyse_carries_on_from_the_prefix() {
    let _guard = serially();
    set_test_cache_dir(None);
    let stop = AtomicBool::new(false);
    let (prefix, cancelled) = analyse(job(9), None, &mut Written { reads: 0 }, &stop, &|step| {
        if let Progress::Solving { done, .. } = step {
            if done >= 3 {
                stop.store(true, Ordering::Relaxed);
            }
        }
    })
    .expect("a run");
    assert!(cancelled);
    assert_eq!((prefix.first_frame, prefix.last_frame), (0, 2));

    let mut model = Written { reads: 0 };
    let (whole, cancelled) =
        analyse(job(9), Some(prefix), &mut model, &never(), &|_| {}).expect("a run");
    assert!(!cancelled);
    assert_eq!(
        model.reads, 6,
        "the frames the first run finished were read through the model again"
    );
    assert_eq!((whole.first_frame, whole.last_frame), (0, 8));
    assert!(!whole.is_partial());

    // And every frame holds its own plane, the kept ones included.
    let (fresh, _) =
        analyse(job(9), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a whole run");
    let key = PlaneKey::new(&fingerprint(), settings());
    assert_eq!(
        encode(key, &whole).expect("bytes"),
        encode(key, &fresh).expect("bytes"),
        "a resumed run is not the run one pass would have made"
    );

    // A prefix something else made is read again rather than carried on from:
    // the provenance names one run, and this one would be naming the wrong
    // half of itself.
    let mut elsewhere = fresh;
    elsewhere.made_with = "Another runtime entirely".into();
    let mut model = Written { reads: 0 };
    let (again, _) =
        analyse(job(9), Some(elsewhere), &mut model, &never(), &|_| {}).expect("a run of its own");
    assert_eq!(model.reads, 9, "a prefix from another run was carried on");
    assert_eq!((again.first_frame, again.last_frame), (0, 8));
}

/// The same job, asking for a matte instead: `arch` is the Model row, 0 for
/// Robust Video Matting and 1 for BiRefNet.
fn matte_job(frames: usize, arch: u32) -> PlaneJob {
    let settings = PlaneSettings {
        task: PlaneTask::Matte,
        arch,
        ..settings()
    };
    PlaneJob {
        key: Some(PlaneKey::new(&fingerprint(), settings)),
        settings,
        ..job(frames)
    }
}

/// **A matte is kept as the box it covers, and comes back whole.** A coverage
/// is a subject with nothing around it, so only the box is written; what the
/// render path asks for is the plane at its own raster, and every byte outside
/// the box has to be nought when it gets there.
#[test]
fn a_matte_is_boxed_on_the_way_in_and_whole_on_the_way_out() {
    let _guard = serially();
    set_test_cache_dir(None);
    let mut model = Cut { reads: 0 };
    let (run, cancelled) =
        analyse(matte_job(3, 0), None, &mut model, &never(), &|_| {}).expect("a run");
    assert!(!cancelled);
    assert_eq!(model.reads, 3, "one model run a frame");

    let boxed = &run.records[1];
    assert_eq!(boxed.bbox, BOX, "the box a matte is kept as");
    assert!(
        boxed.lz4.len() < (W as usize) * (H as usize),
        "a boxed, compressed matte is smaller than the plane it covers"
    );

    for frame in 0..3 {
        let (w, h, kind, plane) = run.plane(frame).expect("a matte a frame");
        assert_eq!((w, h), (W, H), "a matte is at the frame's own raster");
        assert_eq!(kind, PlaneKind::Matte);
        assert_eq!(plane.len(), (W as usize) * (H as usize), "one byte a pixel");
        let value = ((frame * 10) as u8).max(1);
        let [bx, by, _, _] = BOX;
        assert_eq!(
            plane[((by + 1) * W + bx + 1) as usize],
            value,
            "frame {frame} kept another frame's coverage"
        );
        assert_eq!(plane[0], 0, "and outside the box there is no subject");
        assert_eq!(plane[plane.len() - 1], 0);
    }
}

/// **A Robust Video Matting run starts at the first frame and copies nothing,
/// and a BiRefNet one carries on** (§13's "Robust Video Matting is a sequence,
/// not a set").
///
/// Its state rides from one frame to the next and the record does not keep it,
/// so a prefix read under a state this run never had is not a prefix this run
/// may carry on from. Depth and the other matte model read each frame on its
/// own and do carry on, which is the other half of the same rule: the question
/// is the model's, not the task's.
#[test]
fn a_matte_run_reads_the_shot_again_rather_than_resuming_it() {
    let _guard = serially();
    set_test_cache_dir(None);
    let (prefix, _) = analyse(
        matte_job(3, 0),
        None,
        &mut Cut { reads: 0 },
        &never(),
        &|_| {},
    )
    .expect("a prefix");

    let mut model = Cut { reads: 0 };
    let (whole, _) = analyse(matte_job(6, 0), Some(prefix), &mut model, &never(), &|_| {})
        .expect("a run of its own");
    assert_eq!(model.reads, 6, "a matte run carried on from a kept prefix");
    assert_eq!((whole.first_frame, whole.last_frame), (0, 5));
    assert!(resumes(settings()), "depth reads each frame on its own");
    assert!(!resumes(matte_job(1, 0).settings));
    assert!(
        resumes(matte_job(1, 1).settings),
        "BiRefNet reads each frame on its own, which the manual promises"
    );

    // And the model that does read each frame on its own carries on from the
    // prefix rather than spending the shot again to reach the same planes.
    let (prefix, _) = analyse(
        matte_job(3, 1),
        None,
        &mut Cut { reads: 0 },
        &never(),
        &|_| {},
    )
    .expect("a prefix");
    let mut model = Cut { reads: 0 };
    let (whole, _) = analyse(matte_job(6, 1), Some(prefix), &mut model, &never(), &|_| {})
        .expect("a run of its own");
    assert_eq!(model.reads, 3, "BiRefNet read the frames it already had");
    assert_eq!((whole.first_frame, whole.last_frame), (0, 5));

    // And a depth prefix that starts part way through a shot is read again
    // too, because carrying on from it would leave the run with a hole.
    let (mut middle, _) =
        analyse(job(3), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a prefix");
    middle.first_frame = 1;
    let mut model = Written { reads: 0 };
    let (again, _) =
        analyse(job(5), Some(middle), &mut model, &never(), &|_| {}).expect("a run of its own");
    assert_eq!(
        model.reads, 5,
        "a prefix with no first frame was carried on"
    );
    assert_eq!((again.first_frame, again.last_frame), (0, 4));
}

/// **Only the job that claimed the slot may give it back** (§9's "two model
/// jobs never run at once").
///
/// Reopening a project part-way through an analysis warms the very instance
/// being read, because the ids come out of the file. A warm pass that answered
/// for it used to hand the running job's slot away and delete its reading, and
/// the next Analyse then opened a second model on the same card.
#[test]
fn a_warm_pass_leaves_a_running_job_holding_the_slot() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    set_test_cache_dir(Some(dir.path().to_path_buf()));
    clear();

    let instance = Uuid::from_u128(61);
    let reading = Progress::Solving { done: 1, total: 9 };
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut held) = jobs().lock() {
        // Exactly what `request` claims before it spawns its thread.
        held.running = Some((instance, Arc::clone(&flag)));
        held.progress.insert(instance, reading.clone());
    }

    let mut warm_job = job(3);
    warm_job.instance = instance;
    warm_job.analyse = false;
    run(warm_job, &never());

    assert_eq!(
        progress(instance),
        Some(reading),
        "the warm pass wiped the running job's reading"
    );
    let mut second = job(3);
    second.instance = instance;
    assert_eq!(
        request(second),
        Requested::Refused(PlaneFailure::Busy),
        "a second model job was let past the one-at-a-time slot"
    );

    if let Ok(mut held) = jobs().lock() {
        held.running = None;
    }
    clear();
    set_test_cache_dir(None);
}

/// **A model that will not run is a refusal, not a fault.** Nothing is filed
/// and the reason has a name the bridge can carry.
#[test]
fn a_model_that_will_not_run_is_a_named_refusal() {
    let _guard = serially();
    set_test_cache_dir(None);
    assert_eq!(
        analyse(job(3), None, &mut Refuses, &never(), &|_| {}).unwrap_err(),
        PlaneFailure::ModelFailed
    );
    // And a clip with nothing in it says so rather than filing an empty run.
    let empty = PlaneJob {
        open: Box::new(|| Some(Box::new(Clip { frames: 0 }) as Box<dyn RotoFrames>)),
        ..job(0)
    };
    assert_eq!(
        analyse(empty, None, &mut Written { reads: 0 }, &never(), &|_| {}).unwrap_err(),
        PlaneFailure::NoFrames
    );
    // And media that will not open at all.
    let offline = PlaneJob {
        open: Box::new(|| None),
        ..job(3)
    };
    assert_eq!(
        analyse(offline, None, &mut Written { reads: 0 }, &never(), &|_| {}).unwrap_err(),
        PlaneFailure::Unreadable
    );
}

/// **The sidecar round trips, refuses what it cannot vouch for, and a deleted
/// file rebuilds to what was deleted** (§11 test 6).
#[test]
fn the_sidecar_round_trips_and_refuses_what_it_cannot_vouch_for() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    set_test_cache_dir(Some(dir.path().to_path_buf()));
    let key = PlaneKey::new(&fingerprint(), settings());
    let (run, _) =
        analyse(job(4), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a run");

    write_sidecar(dir.path(), key, &run);
    let read = read_sidecar(dir.path(), key).expect("the file reads back");
    assert_eq!((read.first_frame, read.last_frame), (0, 3));
    assert_eq!(read.made_with, run.made_with, "the provenance survives");
    assert_eq!(
        encode(key, &read).expect("bytes"),
        encode(key, &run).expect("bytes"),
        "a cache hit and the run it was made from are the same bytes"
    );

    // A key that is not the one asked for.
    let elsewhere = PlaneSettings {
        arch: 1,
        ..settings()
    };
    let other = PlaneKey::new(&fingerprint(), elsewhere);
    assert_ne!(other, key, "a settings change is a different name");
    assert!(read_sidecar(dir.path(), other).is_none());

    // A file a newer Lumit wrote, and one that is not ours at all.
    let path = dir.path().join(key.file_name());
    let mut bytes = std::fs::read(&path).expect("the file");
    bytes[7] = 0xff;
    bytes[8] = 0xff;
    assert!(decode(&bytes, key).is_none(), "a newer version is refused");
    assert!(decode(b"nothing of the sort", key).is_none());
    assert!(decode(&[], key).is_none());

    // Deleted, and rebuilt to what was deleted.
    std::fs::remove_file(&path).expect("the file goes");
    assert!(read_sidecar(dir.path(), key).is_none());
    let (again, _) =
        analyse(job(4), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a run");
    write_sidecar(dir.path(), key, &again);
    assert_eq!(
        std::fs::read(&path).expect("the file is back"),
        encode(key, &run).expect("bytes"),
        "the rebuild is the file that was deleted"
    );
    set_test_cache_dir(None);
}

/// **The store answers for the instance it was filed under, and forgets on
/// demand.** Two effects on one clip are two answers, and closing a project
/// takes its own and nobody else's.
#[test]
fn the_store_answers_per_instance_and_forgets_on_demand() {
    let _guard = serially();
    set_test_cache_dir(None);
    clear();
    let mine = Uuid::from_u128(21);
    let yours = Uuid::from_u128(22);
    let (run, _) =
        analyse(job(2), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a run");
    let (other, _) =
        analyse(job(2), None, &mut Written { reads: 0 }, &never(), &|_| {}).expect("a run");
    publish(mine, run);
    publish(yours, other);

    assert_eq!(span(mine), Some((0, 1)));
    assert!(plane(mine, 0).is_some());
    assert!(plane(Uuid::from_u128(23), 0).is_none(), "nobody else's");

    forget(&[mine]);
    assert!(analysed(mine).is_none(), "mine went");
    assert!(analysed(yours).is_some(), "and yours stayed");
    clear();
    assert!(analysed(yours).is_none());
}

/// **A refusal a press answered without spawning a thread is readable.** A
/// press is an event with nothing to poll against, so the reason is left where
/// the next status read finds it.
#[test]
fn a_refusal_with_no_thread_is_still_a_reading() {
    let _guard = serially();
    clear();
    let instance = Uuid::from_u128(31);
    assert_eq!(progress(instance), None, "nothing has been asked");
    note_refusal(instance, PlaneFailure::PackMissing);
    assert_eq!(
        progress(instance),
        Some(Progress::Failed(PlaneFailure::PackMissing))
    );
    clear();
}

/// A footage layer wearing one Depth effect, and the comp around it, so the
/// real frame key can be asked about it.
fn project(
    model: u32,
    with_depth: bool,
) -> (
    std::sync::Arc<lumit_core::model::Document>,
    lumit_core::model::Composition,
    Uuid,
) {
    use lumit_core::model::{
        Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, ProjectItem,
        Switches, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};

    let item = Uuid::from_u128(41);
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: None,
        id: item,
        name: "clip.mp4".into(),
        media: MediaRef {
            relative_path: "clip.mp4".into(),
            absolute_path: "/media/clip.mp4".into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
    }));

    let mut effects = Vec::new();
    if with_depth {
        let mut depth = lumit_core::fx::instantiate(lumit_core::planes::DEPTH).expect("a built-in");
        depth.id = Uuid::from_u128(42);
        depth.params.push(EffectParam {
            id: "model".into(),
            value: lumit_core::model::EffectValue::Choice(model),
            extra: serde_json::Map::new(),
        });
        effects.push(depth);
    }

    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::from_u128(43),
        name: "clip".into(),
        kind: LayerKind::Footage { item },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(4, 1).expect("a duration")),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects,
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::from_u128(44),
        name: "Scene".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(30, 1).expect("a rate"),
        duration: Duration(Rational::new(4, 1).expect("a duration")),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    doc.items.push(ProjectItem::Composition(comp.clone()));
    (std::sync::Arc::new(doc), comp, item)
}

/// **A settings change renames every frame, and a layer without the effect is
/// renamed by nothing** (§13's "the frame key is the easiest thing to forget",
/// and its mirror).
///
/// Asked through the real frame key rather than through the stamp, because the
/// stamp reaching the key is the half that fails silently.
#[test]
fn a_settings_change_renames_the_frames_and_a_plain_layer_none() {
    // The addons folder and the store are both process-wide and both reach
    // this key, so this asks its questions with nothing else moving them.
    let _guard = serially();
    let (doc_a, comp_a, item) = project(0, true);
    let (doc_b, comp_b, _) = project(1, true);
    let (doc_none, comp_none, _) = project(0, false);

    let mut probes = std::collections::HashMap::new();
    probes.insert(
        item,
        crate::source::SourceProbe::Video {
            fps: 30.0,
            width: 64,
            height: 64,
            frames: 120,
            audio: false,
        },
    );
    let quality = crate::plan::Quality::default();
    let key = |doc: &std::sync::Arc<lumit_core::model::Document>,
               comp: &lumit_core::model::Composition,
               frame: usize| {
        crate::cache::frame_key(doc, comp, frame, quality, &probes).expect("a named frame")
    };

    for frame in 0..4 {
        assert_ne!(
            key(&doc_a, &comp_a, frame),
            key(&doc_b, &comp_b, frame),
            "frame {frame} kept its name across a model change"
        );
        assert_ne!(
            key(&doc_a, &comp_a, frame),
            key(&doc_none, &comp_none, frame),
            "frame {frame} is named the same with the effect and without it"
        );
        assert_eq!(
            key(&doc_a, &comp_a, frame),
            key(&doc_a, &comp_a, frame),
            "and the same document twice is the same name"
        );
    }
}

/// **An analysis landing renames the frames it changes** (§13's "the frame key
/// is the easiest thing to forget", the other half of the test above).
///
/// Nothing else in a frame's name moves when a run is published: the document
/// is untouched and the installed pack is the one it always was. Without the
/// store's own turn in the key, every frame looked at during the minutes an
/// analysis took would be served back with no depth in it, for as long as the
/// cache held it.
#[test]
fn a_landed_analysis_renames_the_frames_and_forgetting_it_renames_them_again() {
    let _guard = serially();
    clear();
    let (doc, comp, item) = project(0, true);
    let mut probes = std::collections::HashMap::new();
    probes.insert(
        item,
        crate::source::SourceProbe::Video {
            fps: 30.0,
            width: 64,
            height: 64,
            frames: 120,
            audio: false,
        },
    );
    let quality = crate::plan::Quality::default();
    let key = |frame: usize| {
        crate::cache::frame_key(&doc, &comp, frame, quality, &probes).expect("a named frame")
    };

    let before = key(0);
    assert_eq!(before, key(0), "the same document twice is the same name");

    let instance = Uuid::from_u128(42);
    let (run, _) = analyse(job(4), None, &mut Written { reads: 0 }, &never(), &|_| {})
        .expect("a run to publish");
    publish(instance, run);
    let after = key(0);
    assert_ne!(before, after, "the frame kept its name across an analysis");

    // The same planes published again are the same name, which is what a
    // project reopened tomorrow does: the name is taken off what the run holds
    // rather than counted, and the disk tier keeps a frame across restarts.
    let (same, _) = analyse(job(4), None, &mut Written { reads: 0 }, &never(), &|_| {})
        .expect("the same run again");
    publish(instance, same);
    assert_eq!(after, key(0), "the same analysis twice is two names");

    forget(&[instance]);
    assert_eq!(
        before,
        key(0),
        "dropping the analysis did not name the frames back"
    );
    clear();
}

/// **A machine with the model wears no badge, and one without it says which
/// addon is missing** (§11 test 8).
///
/// Both endings are written down here rather than left to whatever the machine
/// running the suite happens to have installed, which is the half a real
/// machine can never assert.
#[test]
fn the_badge_names_what_is_missing_and_says_nothing_when_it_is_not() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = dir.path();
    lumit_ml::store::with_dir(Some(root.to_path_buf()));

    let depth = lumit_core::fx::instantiate(lumit_core::planes::DEPTH).expect("a built-in");

    // Nothing installed at all: the runtime is the first thing missing, and
    // the badge sends the user to the runtime's own row.
    assert_eq!(refusal_for(&depth), Some(PlaneFailure::RuntimeMissing));
    assert_eq!(
        addon_missing(&depth).as_deref(),
        Some(lumit_ml::runtime::RUNTIME_ID)
    );

    // The runtime and nothing else: the pack is what is missing now, and the
    // badge names the pack the effect's own model row asks for.
    let library = lumit_ml::runtime::LIBRARY;
    an_addon(root, "runtime", &a_runtime_manifest(library), &[library]);
    assert_eq!(lumit_ml::store::scan().len(), 1, "the runtime is installed");
    assert_eq!(refusal_for(&depth), Some(PlaneFailure::PackMissing));
    assert_eq!(
        addon_missing(&depth).as_deref(),
        Some(lumit_core::fx::effects::depth::model_pack(0))
    );

    // Both: there is nothing to say and the effect wears no badge.
    an_addon(
        root,
        "depth-anything-v2-small",
        &a_depth_manifest(),
        &["model.onnx"],
    );
    assert_eq!(lumit_ml::store::scan().len(), 2, "and the pack beside it");
    assert_eq!(refusal_for(&depth), None);
    assert_eq!(addon_missing(&depth), None);

    lumit_ml::store::with_dir(None);
}

/// **The badge asks for the pack the Model row names, not for any pack that
/// does the task** (§11 test 8, for the second effect on this tier).
///
/// Two packs do matting and they are two different answers. With one of them
/// installed and the other chosen, the effect still cannot run, and a badge that
/// asked only "is anything installed for matting" would say nothing while the
/// effect sat there doing nothing.
#[test]
fn the_matte_badge_names_the_pack_the_model_row_asks_for() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = dir.path();
    lumit_ml::store::with_dir(Some(root.to_path_buf()));

    let rvm =
        lumit_core::fx::instantiate(lumit_core::planes::REMOVE_BACKGROUND).expect("a built-in");
    assert_eq!(refusal_for(&rvm), Some(PlaneFailure::RuntimeMissing));
    assert_eq!(
        addon_missing(&rvm).as_deref(),
        Some(lumit_ml::runtime::RUNTIME_ID)
    );

    let library = lumit_ml::runtime::LIBRARY;
    an_addon(root, "runtime", &a_runtime_manifest(library), &[library]);
    an_addon(
        root,
        "rvm",
        &a_matte_manifest(lumit_ml::matte::MatteArch::Rvm),
        &["model.onnx"],
    );
    assert_eq!(lumit_ml::store::scan().len(), 2, "the runtime and one pack");
    assert_eq!(refusal_for(&rvm), None, "the pack it asks for is installed");
    assert_eq!(addon_missing(&rvm), None);

    // The other model, which nothing installed does.
    let mut birefnet = rvm.clone();
    set(
        &mut birefnet,
        "model",
        lumit_core::model::EffectValue::Choice(1),
    );
    assert_eq!(refusal_for(&birefnet), Some(PlaneFailure::PackMissing));
    assert_eq!(
        addon_missing(&birefnet).as_deref(),
        Some(lumit_core::fx::effects::remove_background::model_pack(1)),
        "the badge named a pack the Model row did not ask for"
    );

    // And with both installed, each model's runs are named after its own pack.
    // The key used to read whichever pack of the task sorted first, so
    // installing the second one renamed the first one's finished analysis and
    // updating it renamed nothing.
    an_addon(
        root,
        "birefnet-lite",
        &a_matte_manifest(lumit_ml::matte::MatteArch::Birefnet),
        &["model.onnx"],
    );
    assert_eq!(lumit_ml::store::scan().len(), 3, "and the second pack");
    let named = |fx| PlaneSettings::of(fx).expect("a planes effect").identity;
    assert_ne!(
        named(&rvm),
        named(&birefnet),
        "both models' runs are named after one pack"
    );
    assert_ne!(named(&rvm), [0u8; 32], "the named pack is installed");
    assert_ne!(named(&birefnet), [0u8; 32]);

    lumit_ml::store::with_dir(None);
}

/// Write one row on an instance, adding it when the declaration's default left
/// it out.
fn set(fx: &mut EffectInstance, id: &str, value: lumit_core::model::EffectValue) {
    match fx.params.iter_mut().find(|p| p.id == id) {
        Some(param) => param.value = value,
        None => fx.params.push(EffectParam {
            id: id.to_owned(),
            value,
            extra: serde_json::Map::new(),
        }),
    }
}

/// A matte pack's manifest, per family. Two of them do this one task, which is
/// why the tests below have to be able to put both on the machine at once.
fn a_matte_manifest(arch: lumit_ml::matte::MatteArch) -> String {
    let (id, name, family, digest) = match arch {
        lumit_ml::matte::MatteArch::Rvm => (
            "rvm",
            "Robust Video Matting",
            "rvm",
            "0000000000000000000000000000000000000000000000000000000000000005",
        ),
        lumit_ml::matte::MatteArch::Birefnet => (
            "birefnet-lite",
            "BiRefNet Lite",
            "birefnet",
            "0000000000000000000000000000000000000000000000000000000000000006",
        ),
    };
    let bytes = ADDON_FILE_BYTES;
    format!(
        r#"{{"format":1,"id":"{id}","kind":"model",
           "name":"{name}","version":"1.0","licence":"GPL-3.0",
           "platforms":{{"any":{{"downloads":[
             {{"url":"https://example.invalid/model.onnx","sha256":"{digest}",
               "size":{bytes},"unpack":"file","dest":"model.onnx"}}]}}}},
           "model":{{"task":"matte","arch":"{family}","file":"model.onnx"}}}}"#
    )
}

/// How long each file an addon written here is: the store checks that a listed
/// file is there and is the length the manifest says, so a dozen bytes of
/// nothing is a working install.
const ADDON_FILE_BYTES: usize = 12;

/// Write one addon folder: its manifest, and each file the manifest lists.
fn an_addon(root: &Path, id: &str, manifest: &str, files: &[&str]) {
    let home = root.join(id);
    std::fs::create_dir_all(&home).expect("the addon's folder");
    std::fs::write(home.join(lumit_ml::store::MANIFEST_FILE), manifest).expect("the manifest");
    for name in files {
        std::fs::write(home.join(name), vec![0u8; ADDON_FILE_BYTES]).expect("a listed file");
    }
}

fn a_runtime_manifest(library: &str) -> String {
    let digest = "0000000000000000000000000000000000000000000000000000000000000003";
    let bytes = ADDON_FILE_BYTES;
    format!(
        r#"{{"format":1,"id":"runtime","kind":"runtime","name":"Model runtime",
           "version":"1.24.4","licence":"MIT","platforms":{{"any":{{"downloads":[
             {{"url":"https://example.invalid/runtime.zip","sha256":"{digest}",
               "size":{bytes},"unpack":"file","dest":"{library}"}}]}}}}}}"#
    )
}

fn a_depth_manifest() -> String {
    let digest = "0000000000000000000000000000000000000000000000000000000000000004";
    let bytes = ADDON_FILE_BYTES;
    format!(
        r#"{{"format":1,"id":"depth-anything-v2-small","kind":"model",
           "name":"Depth Anything V2 Small","version":"1.0","licence":"Apache-2.0",
           "platforms":{{"any":{{"downloads":[
             {{"url":"https://example.invalid/model.onnx","sha256":"{digest}",
               "size":{bytes},"unpack":"file","dest":"model.onnx"}}]}}}},
           "model":{{"task":"depth","arch":"depth-anything","file":"model.onnx",
                     "input":"pixel_values","output":"predicted_depth",
                     "size":518,"multiple":14}}}}"#
    )
}

/// **Every refusal has a name and none is a fault** (§9). The list is closed,
/// so the bridge's own mapping is a compile error when one is added.
#[test]
fn every_refusal_has_a_name_and_none_is_a_fault() {
    use lumit_ml::MlError;
    for (was, is) in [
        (MlError::RuntimeMissing, PlaneFailure::RuntimeMissing),
        (
            MlError::RuntimeFailed("no".into()),
            PlaneFailure::RuntimeMissing,
        ),
        (
            MlError::PackMissing(lumit_ml::Task::Depth),
            PlaneFailure::PackMissing,
        ),
        (MlError::PackUnreadable, PlaneFailure::ModelFailed),
        (MlError::ShapeMismatch, PlaneFailure::ModelFailed),
        (MlError::ModelFailed("no".into()), PlaneFailure::ModelFailed),
        (MlError::Cancelled, PlaneFailure::Cancelled),
    ] {
        assert_eq!(PlaneFailure::of(&was), is, "{was:?}");
    }
    for failure in [
        PlaneFailure::RuntimeMissing,
        PlaneFailure::PackMissing,
        PlaneFailure::ModelFailed,
        PlaneFailure::Busy,
        PlaneFailure::Unreadable,
        PlaneFailure::NoFrames,
        PlaneFailure::Cancelled,
    ] {
        assert!(
            !failure.to_string().is_empty(),
            "{failure:?} has nothing to say"
        );
    }
}

/// **A key is the media and the settings, and nothing else.** Two projects on
/// the same rushes with the same rows find the same file; one row apart and
/// they do not.
#[test]
fn the_key_is_the_media_and_the_settings() {
    let same = PlaneKey::new(&fingerprint(), settings());
    assert_eq!(same, PlaneKey::new(&fingerprint(), settings()));
    assert!(same.file_name().ends_with(".lpln"));
    assert!(same.file_name().starts_with(&same.prefix()));

    for other in [
        PlaneSettings {
            arch: 1,
            ..settings()
        },
        PlaneSettings {
            identity: [9u8; 32],
            ..settings()
        },
        PlaneSettings {
            provider: 0,
            ..settings()
        },
        PlaneSettings {
            downsample: 1,
            ..settings()
        },
        PlaneSettings {
            task: PlaneTask::Matte,
            ..settings()
        },
    ] {
        assert_ne!(
            same,
            PlaneKey::new(&fingerprint(), other),
            "{other:?} shares a name with the settings beside it"
        );
    }
    let elsewhere = Fingerprint {
        size: 8192,
        head_tail_hash: "another clip".into(),
        mtime_secs: 0,
    };
    let apart = PlaneKey::new(&elsewhere, settings());
    assert_ne!(
        same.prefix(),
        apart.prefix(),
        "another file, another prefix"
    );
}
