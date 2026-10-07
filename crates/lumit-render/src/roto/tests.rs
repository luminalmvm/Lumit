//! The propagation job, the `roto/` tier and the store, asserted against a
//! **synthetic shot with a known matte** (docs/impl/roto.md §10) — a bright disc
//! translating over textured ground, rendered by the test so every claim is an
//! assertion rather than a look.
//!
//! No encoder and no asset: the frames arrive through [`RotoFrames`], which is
//! the seam `LumaFrames` already is on the tracking side.

use super::*;
use lumit_core::roto::{RotoStroke as DocStroke, RotoStrokeKind};

/// The shot: a 96×72 frame, spatially smooth texture, a bright disc that moves
/// four pixels a frame. Smooth texture on purpose — γ prices *every* colour
/// step, so per-pixel noise would make the walk crawl (docs/impl/roto.md §12).
struct Disc {
    frames: usize,
    width: u32,
    height: u32,
}

const W: u32 = 96;
const H: u32 = 72;
const R: f32 = 14.0;

impl Disc {
    fn new(frames: usize) -> Self {
        Disc {
            frames,
            width: W,
            height: H,
        }
    }

    /// Where the disc's centre is on frame `n`.
    fn centre(n: i64) -> (f32, f32) {
        (24.0 + 4.0 * n as f32, 36.0)
    }

    /// The matte the test knows to be right.
    fn truth(n: i64) -> Vec<bool> {
        let (cx, cy) = Self::centre(n);
        (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .map(|(x, y)| {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                dx * dx + dy * dy <= R * R
            })
            .collect()
    }
}

impl RotoFrames for Disc {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, self.width, self.height, 24.0)
    }

    fn rgba(&mut self, n: usize) -> Option<Vec<u8>> {
        if n >= self.frames {
            return None;
        }
        let inside = Disc::truth(n as i64);
        let mut out = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            for x in 0..W {
                // A slow ramp both ways: texture the flow can lock on to, with
                // no step anywhere for the geodesic walk to trip over.
                let ground = 40 + ((x / 8 + y / 8) % 3) as u8 * 6;
                let (r, g, b) = if inside[(y * W + x) as usize] {
                    (235u8, 225u8, 210u8)
                } else {
                    (ground, ground + 4, ground + 8)
                };
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Some(out)
    }
}

fn stroke(frame: i64, kind: RotoStrokeKind, from: (f32, f32), to: (f32, f32)) -> DocStroke {
    DocStroke {
        id: uuid::Uuid::now_v7(),
        points: vec![from, to],
        radius: 2.0,
        kind,
        frame,
    }
}

/// A block that cuts the disc out on frame `base`: one stroke through it, and
/// the border ring answering for the background.
fn block_at(base: i64) -> RotoBlock {
    let (cx, cy) = Disc::centre(base);
    RotoBlock {
        base_frame: Some(base),
        strokes: vec![stroke(
            base,
            RotoStrokeKind::Foreground,
            (cx - 6.0, cy),
            (cx + 6.0, cy),
        )],
        prompts: Vec::new(),
    }
}

/// The same shot seeded by a tap in the middle of the disc instead: no stroke
/// at all, one prompt, and the seed row saying so.
fn prompted_at(base: i64) -> (RotoBlock, RotoSettings) {
    let (cx, cy) = Disc::centre(base);
    (
        RotoBlock {
            base_frame: Some(base),
            strokes: Vec::new(),
            prompts: vec![lumit_core::roto::RotoPrompt {
                id: uuid::Uuid::now_v7(),
                frame: base,
                points: vec![(cx, cy)],
                labels: vec![1],
            }],
        },
        RotoSettings {
            seed: lumit_core::fx::effects::roto_brush::SEED_SEGMENT,
            identity: [3; 32],
            ..RotoSettings::default()
        },
    )
}

/// A model the test wrote down: it answers with the analytic disc of whichever
/// frame the tap landed on, so every claim about the seeding is an assertion
/// rather than a reading of somebody's weights.
///
/// It is deliberately soft at the rim, because a real model is: the band
/// between the two thresholds is what [`lumit_roto::mask_seeds`] declines to
/// seed and what the solve then decides from the frame's own colours.
struct FakeModel {
    /// Where the tap the model was asked about landed, so the test can assert
    /// the prompt reached it in source pixels.
    asked: std::sync::Arc<std::sync::Mutex<Vec<(f32, f32)>>>,
    /// How many frames it was handed, which is what makes "the encoder runs
    /// once" an assertion.
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl RotoModel for FakeModel {
    fn embed(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), RotoFailure> {
        assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn mask(&mut self, points: &[(f32, f32)], labels: &[u8]) -> Result<Vec<f32>, RotoFailure> {
        if let Ok(mut held) = self.asked.lock() {
            held.extend_from_slice(points);
        }
        let positive = points
            .iter()
            .zip(labels)
            .find(|(_, label)| **label == 1)
            .map(|(point, _)| *point)
            .ok_or(RotoFailure::ModelFailed)?;
        // Which frame's disc the tap is inside, by where its centre would be.
        let frame = ((positive.0 - 24.0) / 4.0).round() as i64;
        let (cx, cy) = Disc::centre(frame);
        Ok((0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .map(|(x, y)| {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let away = (dx * dx + dy * dy).sqrt();
                // Sure inside, sure outside, unsure across the rim.
                (1.0 - (away - (R - 1.0)) / 2.0).clamp(0.0, 1.0)
            })
            .collect())
    }

    fn made_with(&self) -> String {
        "Test; sam2-written-down; 03; ONNX Runtime 0".into()
    }
}

fn job(block: RotoBlock, frames: usize) -> RotoJob {
    RotoJob {
        instance: uuid::Uuid::now_v7(),
        key: None,
        settings: RotoSettings::default(),
        block,
        open: Box::new(move || Some(Box::new(Disc::new(frames)) as Box<dyn RotoFrames>)),
        // Nothing seeded by its strokes ever opens one, so the default refuses:
        // a test that reaches for a model without saying so fails loudly.
        model: Box::new(|| Err(RotoFailure::ModelMissing)),
        propagate: true,
        stop_after: None,
    }
}

/// Intersection over union of a stored matte against the analytic disc.
fn iou(run: &RotoRun, frame: i64) -> f64 {
    let truth = Disc::truth(frame);
    let plane = run.matte(frame).expect("a matte for this frame");
    let (mut inter, mut union) = (0usize, 0usize);
    for (i, &t) in truth.iter().enumerate() {
        let got = plane.get(i).copied().unwrap_or(0) > 127;
        if t && got {
            inter += 1;
        }
        if t || got {
            union += 1;
        }
    }
    inter as f64 / union.max(1) as f64
}

/// Tests share one process-wide cache override and one running slot, so they
/// take a lock rather than racing each other — `crate::track`'s arrangement.
fn serially() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// The same claim for a **prompted** base (docs/impl/addons.md §6.2, §11 test
/// 10): the model proposes the subject, the seeds come off its answer, and the
/// solve cuts the same disc out of the same frame.
///
/// What is asserted about the model is that it was handed the frame once and
/// the tap in source pixels, which is the whole of the seam; what its answer
/// looks like is the test's own arithmetic, so nothing here depends on a pack
/// being installed.
#[test]
fn the_base_frame_is_cut_from_a_prompt_when_the_seed_row_asks() {
    let _guard = serially();
    set_test_cache_dir(None);
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let (block, settings) = prompted_at(0);
    let mut j = job(block, 1);
    j.settings = settings;
    j.model = {
        let (asked, reads) = (asked.clone(), reads.clone());
        Box::new(move || Ok(Box::new(FakeModel { asked, reads }) as Box<dyn RotoModel>))
    };
    let (run, cancelled) = propagate(j, &never(), &|_| {}).expect("a run");

    assert!(!cancelled);
    assert!(iou(&run, 0) >= 0.95, "prompted base IoU {}", iou(&run, 0));
    assert_eq!(
        asked.lock().expect("the taps").as_slice(),
        &[Disc::centre(0)],
        "the tap reached the model in source pixels"
    );
    assert_eq!(
        reads.load(Ordering::Relaxed),
        1,
        "the encoder read the frame more than once"
    );
    assert!(
        run.made_with.starts_with("Test; sam2-written-down;"),
        "the record does not say what seeded it: {}",
        run.made_with
    );

    // With no model on the machine the run is a refusal that names the reason,
    // and never a matte cut some other way (docs/impl/addons.md §9).
    let (block, settings) = prompted_at(0);
    let mut j = job(block, 1);
    j.settings = settings;
    assert_eq!(
        propagate(j, &never(), &|_| {}).unwrap_err(),
        RotoFailure::ModelMissing
    );

    // And a brush carrying a prompt with the seed row back on its strokes never
    // opens one at all: the job's opener refuses, and the run comes back
    // anyway, cut from the strokes as it always was.
    let (prompted, _) = prompted_at(0);
    let mut block = block_at(0);
    block.prompts = prompted.prompts;
    let (run, _) = propagate(job(block, 1), &never(), &|_| {}).expect("a run");
    assert!(
        run.made_with.is_empty(),
        "nothing seeded it but the strokes"
    );
    assert!(iou(&run, 0) >= 0.95, "stroked base IoU {}", iou(&run, 0));
}

/// §10 item 2: the disc translating, strokes on the base only, both directions
/// from a base in the middle.
#[test]
fn the_matte_is_carried_both_ways_from_the_base() {
    let _guard = serially();
    set_test_cache_dir(None);
    let frames = 9;
    let (run, _) = propagate(job(block_at(4), frames), &never(), &|_| {}).expect("a run");
    assert_eq!((run.first_frame, run.last_frame), (0, 8));
    for f in 0..frames as i64 {
        let got = iou(&run, f);
        assert!(got >= 0.85, "frame {f} IoU {got}");
    }
}

/// §10 item 6: the sidecar's whole contract — round trip, a rebuild identical
/// to the hit, a wrong key refused, a newer version refused, a deleted file
/// rebuilt to the identical bytes.
#[test]
fn the_sidecar_round_trips_and_refuses_what_it_cannot_vouch_for() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    set_test_cache_dir(Some(dir.path().to_path_buf()));

    let fingerprint = lumit_core::model::Fingerprint {
        size: 512,
        head_tail_hash: "roto-sidecar".into(),
        mtime_secs: 0,
    };
    let block = block_at(0);
    let key = RotoKey::new(&fingerprint, &block, RotoSettings::default());
    let mut j = job(block.clone(), 4);
    j.key = Some(key);
    let (run, _) = propagate(j, &never(), &|_| {}).expect("a run");
    let bytes = encode(key, &run).expect("encodes");

    // Round trip.
    let back = decode(&bytes, Some(key)).expect("decodes");
    assert_eq!(back.frames, run.records);

    // A wrong key is refused rather than believed.
    let other = RotoKey::new(&fingerprint, &block_at(1), RotoSettings::default());
    assert!(decode(&bytes, Some(other)).is_none());

    // A version from the future is refused before the body is parsed.
    let mut newer = bytes.clone();
    newer[7] = FORMAT_VERSION.saturating_add(1) as u8;
    assert!(decode(&newer, Some(key)).is_none());

    // A file that is not one of ours never reaches a deserialiser.
    let mut alien = bytes.clone();
    alien[0] = b'X';
    assert!(decode(&alien, Some(key)).is_none());

    // Delete-safe, and the rebuild is byte-identical to what was deleted: the
    // whole determinism claim, asserted rather than assumed (§8).
    write_sidecar(dir.path(), key, &run);
    let path = dir.path().join(key.file_name());
    assert!(path.exists());
    std::fs::remove_file(&path).expect("removes");
    assert!(read_sidecar(dir.path(), key).is_none());
    let mut again = job(block, 4);
    again.key = Some(key);
    let (rebuilt, _) = propagate(again, &never(), &|_| {}).expect("a run");
    assert_eq!(
        encode(key, &rebuilt).expect("encodes"),
        bytes,
        "a rebuild is byte-identical to the file it replaces"
    );
}

/// A sidecar is Lumit's own writing, but it lives in a cache directory on an
/// ordinary disk: it can be truncated by a full disk, corrupted by a failing
/// drive, carried between machines in a project folder, or simply edited. Every
/// number in it decides an allocation, so every number is checked.
///
/// A refused sidecar costs a re-propagation, which is what happens when there is
/// no sidecar at all — so the whole file is refused rather than salvaged.
#[test]
fn a_sidecar_whose_numbers_do_not_hold_together_is_refused() {
    /// One valid record, then whatever the caller wants changed about it.
    fn framed(edit: impl FnOnce(&mut Record)) -> Vec<u8> {
        let boxed = vec![7_u8; 4];
        let mut record = Record {
            key: [0_u8; 32],
            width: 8,
            height: 8,
            fps: 25.0,
            clip_frames: 4,
            made_with: String::new(),
            frames: vec![FrameRecord {
                frame: 0,
                chain: [1_u8; 32],
                bbox: [0, 0, 2, 2],
                lz4: lz4_flex::compress_prepend_size(&boxed),
            }],
        };
        edit(&mut record);
        let body = bincode::serialize(&record).expect("serialises");
        crate::sidecar::frame(MAGIC, FORMAT_VERSION, &body)
    }

    // The shape itself is sound, or none of the rest would mean anything.
    assert!(
        decode(&framed(|_| {}), None).is_some(),
        "the fixture decodes"
    );

    // A raster no picture has. `expand` would allocate width x height from
    // these, which is the whole reason they are checked here instead.
    for (what, edit) in [
        (
            "zero width",
            (|r: &mut Record| r.width = 0) as fn(&mut Record),
        ),
        ("zero height", |r: &mut Record| r.height = 0),
        ("an absurd width", |r: &mut Record| r.width = u32::MAX),
        ("an absurd height", |r: &mut Record| r.height = u32::MAX),
    ] {
        assert!(
            decode(&framed(edit), None).is_none(),
            "{what} must be refused"
        );
    }

    // A rate that is not a positive finite number reaches the retime maths,
    // where an infinity or a NaN spreads quietly rather than failing.
    for (what, edit) in [
        (
            "a NaN rate",
            (|r: &mut Record| r.fps = f64::NAN) as fn(&mut Record),
        ),
        ("an infinite rate", |r: &mut Record| r.fps = f64::INFINITY),
        ("a zero rate", |r: &mut Record| r.fps = 0.0),
        ("a negative rate", |r: &mut Record| r.fps = -25.0),
    ] {
        assert!(
            decode(&framed(edit), None).is_none(),
            "{what} must be refused"
        );
    }

    // A box that leaves the raster: the row arithmetic in `expand` would be
    // writing somewhere the picture is not.
    for (what, edit) in [
        (
            "a box past the right edge",
            (|r: &mut Record| {
                if let Some(f) = r.frames.first_mut() {
                    f.bbox = [7, 0, 2, 2];
                }
            }) as fn(&mut Record),
        ),
        ("a box past the bottom", |r: &mut Record| {
            if let Some(f) = r.frames.first_mut() {
                f.bbox = [0, 7, 2, 2];
            }
        }),
        ("a box whose origin overflows", |r: &mut Record| {
            if let Some(f) = r.frames.first_mut() {
                f.bbox = [u32::MAX, 0, 2, 2];
            }
        }),
    ] {
        assert!(
            decode(&framed(edit), None).is_none(),
            "{what} must be refused"
        );
    }

    // The payload's own announcement. `decompress_size_prepended` reads this
    // four-byte prefix and allocates it before it decompresses anything, so a
    // ten-byte payload announcing four gigabytes is four gigabytes — unless the
    // announcement is checked against the box first, which is what this is.
    let lying = framed(|r| {
        if let Some(f) = r.frames.first_mut() {
            if let Some(head) = f.lz4.get_mut(..4) {
                head.copy_from_slice(&u32::MAX.to_le_bytes());
            }
        }
    });
    assert!(
        decode(&lying, None).is_none(),
        "an LZ4 payload claiming more than its box holds must be refused"
    );

    // An empty box carries no pixels; one that does is not the file it says.
    let stowaway = framed(|r| {
        if let Some(f) = r.frames.first_mut() {
            f.bbox = [0, 0, 0, 0];
        }
    });
    assert!(
        decode(&stowaway, None).is_none(),
        "an empty box with a payload must be refused"
    );

    // Ascending by frame is what makes a lookup a binary search. Out of order
    // is a wrong matte rather than a slow one, which is worse.
    let unsorted = framed(|r| {
        let Some(first) = r.frames.first().cloned() else {
            return;
        };
        r.frames.push(FrameRecord { frame: -1, ..first });
    });
    assert!(
        decode(&unsorted, None).is_none(),
        "records out of frame order must be refused"
    );
}

/// §10 item 8, and §6's fifth step: a cancel **finalises rather than discards**.
/// The frames already solved are kept, correctly named, and the span says how
/// far it got.
#[test]
fn a_cancel_keeps_the_prefix_it_finished() {
    let _guard = serially();
    set_test_cache_dir(None);
    // Raised from the start: the base frame is solved before the loop reads the
    // flag, so the run keeps exactly that one frame and stops.
    let flag = AtomicBool::new(true);
    let (run, cancelled) = propagate(job(block_at(2), 8), &flag, &|_| {}).expect("a run");
    assert!(cancelled);
    assert_eq!((run.first_frame, run.last_frame), (2, 2));
    assert!(run.matte(2).is_some(), "the finished frame was kept");
    assert!(run.matte(3).is_none(), "nothing was invented past it");
    assert!(run.is_partial());
}

/// A Propagate over a partial run **carries on from it** rather than
/// reading it back as the whole answer — the resume §6 promises. The partial
/// run here is a release-time solo; a cancelled run resumes the same way, the
/// two being the same shape of sidecar.
#[test]
fn a_propagate_resumes_from_its_own_partial_run() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    set_test_cache_dir(Some(dir.path().to_path_buf()));

    let fingerprint = lumit_core::model::Fingerprint {
        size: 2048,
        head_tail_hash: "roto-resume".into(),
        mtime_secs: 0,
    };
    let frames = 6;
    let block = block_at(0);
    let key = RotoKey::new(&fingerprint, &block, RotoSettings::default());
    let instance = uuid::Uuid::now_v7();

    // The release-time solo, through the whole job path: solved, filed in the
    // sidecar, published as the one-frame span it honestly is.
    let mut solo = job(block.clone(), frames);
    solo.instance = instance;
    solo.key = Some(key);
    solo.stop_after = Some(0);
    run(solo, &never());
    let partial = propagated(instance).expect("the solo run is in the store");
    assert_eq!((partial.first_frame, partial.last_frame), (0, 0));

    // The same key lends its own file back (the `lendable` half of the
    // resume): the base is **copied** out of it rather than solved again —
    // counted, never timed (§5).
    let mut counted = job(block.clone(), frames);
    counted.key = Some(key);
    let last = std::sync::Mutex::new(Progress::Queued);
    let (_, _) = propagate(counted, &never(), &|p| {
        if let Ok(mut held) = last.lock() {
            *held = p;
        }
    })
    .expect("a run");
    let Progress::Solving { reused, .. } = last.into_inner().expect("the reporter never panicked")
    else {
        panic!("the run never reported progress");
    };
    assert_eq!(reused, 1, "the solo base was lent, not re-solved");

    // The press of Propagate, through the whole job path: the same key finds
    // the partial file and falls through instead of answering Done at it.
    let mut full = job(block, frames);
    full.instance = instance;
    full.key = Some(key);
    run(full, &never());
    let whole = propagated(instance).expect("the resumed run replaced it");
    assert_eq!(
        (whole.first_frame, whole.last_frame),
        (0, frames as i64 - 1),
        "the partial run resumed to the whole clip instead of answering Done"
    );
    assert_eq!(
        whole.matte(0).expect("the base")[..],
        partial.matte(0).expect("the solo base")[..],
        "the lent base is byte-identical to the one the solo filed"
    );
}

/// The store's own contract: a published run answers by frame, the warm cache
/// hands back the same plane, and the per-frame read stays inside the 1 ms bound
/// the render path is budgeted at (§7, docs/13).
#[test]
fn the_store_answers_one_frame_quickly_and_forgets_on_clear() {
    let _guard = serially();
    set_test_cache_dir(None);
    let instance = uuid::Uuid::now_v7();
    let chain = [7u8; 32];
    let plane: Vec<u8> = (0..(W * H)).map(|i| (i % 251) as u8).collect();
    let run = run_from_planes(W, H, 24.0, 40, &[(3, chain, plane.clone())]).expect("a run");
    publish(instance, run);

    assert_eq!(span(instance), Some((3, 3)));
    assert_eq!(stored_chain(instance, 3), Some(chain));
    let (w, h, got) = matte(instance, 3).expect("a matte");
    assert_eq!((w, h), (W, H));
    assert_eq!(&got[..], &plane[..], "the plane round-trips through LZ4");
    assert!(matte(instance, 4).is_none());

    // Budgeted, not measured for a headline: a hundred reads of one frame, the
    // shape the frame walk makes, well inside a millisecond each.
    let started = std::time::Instant::now();
    for _ in 0..100 {
        assert!(matte(instance, 3).is_some());
    }
    let each = started.elapsed().as_secs_f64() * 1000.0 / 100.0;
    assert!(each < 1.0, "a store read took {each:.3} ms, budget is 1 ms");

    clear();
    assert!(propagated(instance).is_none());
}

/// One comp of one footage layer wearing one Roto brush carrying `block`, at
/// the media's own rate, so comp frame n is source frame n and the assertions
/// above can talk about one number. Shared by the two tests below, which ask
/// the same question of a stroke edit and of a prompt edit.
fn a_document(
    block: RotoBlock,
    seed: u32,
) -> (
    std::sync::Arc<lumit_core::model::Document>,
    lumit_core::model::Composition,
    uuid::Uuid,
) {
    use lumit_core::model::{
        Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, ProjectItem,
        Switches, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};

    {
        let item = uuid::Uuid::from_u128(7);
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
        let mut brush =
            lumit_core::fx::instantiate("roto_brush").expect("roto brush is a built-in");
        brush.id = uuid::Uuid::from_u128(9);
        brush.roto = Some(block);
        if let Some(row) = brush.params.iter_mut().find(|p| p.id == "seed") {
            row.value = lumit_core::model::EffectValue::Choice(seed);
        }
        let mut layer = Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: uuid::Uuid::from_u128(8),
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
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        layer.effects = vec![brush];
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: uuid::Uuid::from_u128(6),
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
}

/// A block on frame 0 carrying `strokes` and `prompts` and nothing else.
fn block_of(strokes: Vec<DocStroke>, prompts: Vec<lumit_core::roto::RotoPrompt>) -> RotoBlock {
    RotoBlock {
        base_frame: Some(0),
        strokes,
        prompts,
    }
}

/// The two names the same frame of two documents is filed under, asserted the
/// same way for a stroke edit and for a prompt edit.
fn renames_from(before: RotoBlock, after: RotoBlock, edited: usize) {
    let (doc_a, comp_a, item) = a_document(before, 0);
    let (doc_b, comp_b, _) = a_document(after, 0);

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

    for frame in 0..edited {
        assert_eq!(
            key(&doc_a, &comp_a, frame),
            key(&doc_b, &comp_b, frame),
            "frame {frame} was renamed by a correction it cannot depend on"
        );
    }
    for frame in edited..edited + 10 {
        assert_ne!(
            key(&doc_a, &comp_a, frame),
            key(&doc_b, &comp_b, frame),
            "frame {frame} kept its name across a correction that changes it"
        );
    }
}

/// §10 item 7's other half, and the whole point of the chain hash: **a stroke
/// edit renames exactly the frames it invalidated**, through the real frame key
/// the disk cache files pictures under.
///
/// Without this, a corrected shot would serve back the frames it banked before
/// the correction - a wrong picture with a right name, which is the one failure
/// a content-addressed cache exists to make impossible.
#[test]
fn a_correction_renames_exactly_the_frames_it_spoiled() {
    let base = stroke(0, RotoStrokeKind::Foreground, (10.0, 10.0), (20.0, 10.0));
    renames_from(
        block_of(vec![base.clone()], Vec::new()),
        block_of(
            vec![
                base,
                stroke(10, RotoStrokeKind::Foreground, (30.0, 30.0), (40.0, 30.0)),
            ],
            Vec::new(),
        ),
        10,
    );
}
