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
        source_layer: None,
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
