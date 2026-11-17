//! Denoise reads the frames either side and the motion to each, all of it
//! cached by the decode worker. What a frame comes out as must not depend on
//! which frames were asked for before it: an export walks forward from its
//! first frame, the Viewer jumps about and plays backwards, and both have to
//! give the same pixels.
//!
//! Nor may it average in a frame that is no second look at this one's noise:
//! the same picture again, a frame an effect ahead of it has not touched, or
//! another part of the footage across a cut.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::edits::{base_layer, centred_transform, new_comp_ops, CompSettings};
use lumit_core::model::{
    Document, EffectInstance, EffectValue, FootageItem, Layer, LayerKind, MediaRef, ProjectItem,
    SequenceRef,
};
use lumit_core::ops::{apply, Op};
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::time::{FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

const W: u32 = 96;
const H: u32 = 64;
const FRAMES: u32 = 8;

/// Frame `n` of the run `name`: a pattern moving `slide` px a frame right (and
/// a third of that down), under noise that is different every frame.
fn frame(dir: &Path, name: &str, n: u32, slide: f32) {
    let file = std::fs::File::create(dir.join(format!("{name}_{n:04}.png"))).unwrap();
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    let px: Vec<u8> = (0..W * H)
        .map(|i| {
            let (x, y) = (
                (i % W) as f32 - slide * n as f32,
                (i / W) as f32 - slide / 3.0 * n as f32,
            );
            let base = 120.0 + 70.0 * (x * 0.21).sin() * (y * 0.13).cos();
            let mut s = (i ^ (n.wrapping_mul(0x9E37_79B9))).wrapping_mul(0x85EB_CA6B);
            let mut grain = || {
                s ^= s >> 13;
                s = s.wrapping_mul(0xC2B2_AE35);
                s ^= s >> 16;
                (s % 17) as f32 - 8.0
            };
            [base + grain(), base * 0.8 + grain(), 90.0 + grain()]
                .map(|v| v.clamp(0.0, 255.0) as u8)
        })
        .flat_map(|[r, g, b]| [r, g, b, 255])
        .collect();
    png.write_header().unwrap().write_image_data(&px).unwrap();
}

/// A comp the size of the frames, at 25 fps.
fn comp_doc() -> (Document, Uuid) {
    let mut doc = Document::new();
    let settings = CompSettings {
        width: W,
        height: H,
        frame_rate: FrameRate::new(25, 1).unwrap(),
        background: [0.0; 4],
        ..CompSettings::default()
    };
    let (comp, ops) = new_comp_ops(&doc, settings, None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    (doc, comp)
}

/// A footage item for `file` in `dir`: the run it starts, or with `run` off
/// that one picture alone.
fn footage(doc: &mut Document, dir: &Path, file: &str, run: bool) -> Uuid {
    let item = Uuid::now_v7();
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: run.then(SequenceRef::default),
        id: item,
        name: file.to_owned(),
        media: MediaRef {
            relative_path: file.to_owned(),
            absolute_path: dir.join(file).to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
        source_layer: None,
        interpretation: Default::default(),
    }));
    item
}

/// A layer of `kind` over the whole comp, with `effects`.
fn layer(doc: &Document, comp: Uuid, kind: LayerKind, effects: Vec<EffectInstance>) -> Layer {
    let mut layer = base_layer(
        "Layer".into(),
        kind,
        doc.comp(comp).unwrap().duration.0,
        centred_transform(f64::from(W), f64::from(H), W, H),
    );
    layer.effects = effects;
    layer
}

/// `layer` put on top of `comp`.
fn add(doc: &mut Document, comp: Uuid, layer: Layer) {
    let op = Op::AddLayer {
        comp,
        index: 0,
        layer: Box::new(layer),
    };
    apply(doc, &op).unwrap();
}

fn scene(dir: &Path, denoise: EffectInstance) -> (Arc<Document>, Uuid) {
    let (mut doc, comp) = comp_doc();
    let item = footage(&mut doc, dir, "plate_0000.png", true);
    let plate = layer(&doc, comp, LayerKind::Footage { item }, vec![denoise]);
    add(&mut doc, comp, plate);
    (Arc::new(doc), comp)
}

fn denoise(view: u32) -> EffectInstance {
    denoise_over(view, 2.0)
}

/// Denoise showing `view`, reading `frames` either side.
fn denoise_over(view: u32, frames: f64) -> EffectInstance {
    let mut e = lumit_core::fx::builtins::instantiate("denoise").unwrap();
    for p in &mut e.params {
        match p.id.as_str() {
            "view" => p.value = EffectValue::Choice(view),
            "luma_noise" => p.value = EffectValue::Float(Property::fixed(5.0)),
            "chroma_noise" => p.value = EffectValue::Float(Property::fixed(4.0)),
            "frames" => p.value = EffectValue::Float(Property::fixed(frames)),
            _ => {}
        }
    }
    e
}

/// Frame `f` of `comp`, from a renderer of its own, so nothing one scene
/// cached is found by another of the same footage.
fn shown(doc: &Arc<Document>, comp: Uuid, f: u64) -> Vec<u8> {
    let mut r = HeadlessRenderer::new().unwrap();
    r.render_preview(doc, comp, f, Quality::default(), 1.0)
        .unwrap()
        .0
}

#[test]
fn a_denoised_frame_is_the_same_whichever_frames_came_before_it() {
    let Ok(mut export) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut viewer = HeadlessRenderer::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    for n in 0..FRAMES {
        frame(dir.path(), "plate", n, 1.5);
    }
    let q = Quality::default();
    for view in [0, 2] {
        let (doc, comp) = scene(dir.path(), denoise(view));
        // As an export runs: forward from the first frame, through the
        // pipelined path it reads its bytes from.
        let walked: Vec<Vec<u8>> = (0..u64::from(FRAMES))
            .map(|f| {
                let pending = export.start_preview(&doc, comp, f, q).unwrap();
                export.finish_preview(pending).unwrap()
            })
            .collect();
        // As the Viewer might: the middle first, cold, then backwards.
        let mut order: Vec<u64> = vec![4];
        order.extend((0..u64::from(FRAMES)).rev());
        for f in order {
            let shown = viewer.render_preview(&doc, comp, f, q, 1.0).unwrap().0;
            assert!(
                shown == walked[f as usize],
                "view {view}, frame {f}: the Viewer and the export disagree"
            );
        }
        if view == 2 {
            // The neighbours really were averaged in the middle of the run,
            // and at the held first frame only the frames after it were.
            let grey =
                |px: &[u8]| px.chunks_exact(4).map(|p| u32::from(p[0])).sum::<u32>() / (W * H);
            let (start, middle) = (grey(&walked[0]), grey(&walked[4]));
            assert!(middle > 0, "nothing was blended in the middle of the run");
            assert!(
                start < middle,
                "the held first frame was counted as its own neighbour"
            );
        }
    }
}

/// Over a still, a Precomp or adjustment layer's neighbours are its own
/// picture built again, the same pixels to the bit. They are the same noise,
/// so Frames blended must show that nothing was blended.
#[test]
fn a_rebuilt_neighbour_that_is_the_same_picture_is_not_blended() {
    if HeadlessRenderer::new().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    frame(dir.path(), "still", 0, 0.0);
    // Denoise on an adjustment layer over the still.
    let (mut doc, comp) = comp_doc();
    let item = footage(&mut doc, dir.path(), "still_0000.png", false);
    let still = layer(&doc, comp, LayerKind::Footage { item }, Vec::new());
    add(&mut doc, comp, still);
    let adjust = layer(&doc, comp, LayerKind::Adjustment, vec![denoise(2)]);
    add(&mut doc, comp, adjust);
    let over_adjustment = (Arc::new(doc), comp);
    // And on a Precomp layer of a comp holding it.
    let (mut doc, nested) = comp_doc();
    let item = footage(&mut doc, dir.path(), "still_0000.png", false);
    let still = layer(&doc, nested, LayerKind::Footage { item }, Vec::new());
    add(&mut doc, nested, still);
    let (outer_doc, outer) = comp_doc();
    doc.items.extend(outer_doc.items);
    let precomp = layer(
        &doc,
        outer,
        LayerKind::Precomp { comp: nested },
        vec![denoise(2)],
    );
    add(&mut doc, outer, precomp);
    let over_precomp = (Arc::new(doc), outer);
    let blended: Vec<&str> = [("adjustment", over_adjustment), ("Precomp", over_precomp)]
        .into_iter()
        .filter(|(_, (doc, comp))| {
            let px = shown(doc, *comp, 4);
            !px.chunks_exact(4).all(|p| p[..3] == [0, 0, 0])
        })
        .map(|(what, _)| what)
        .collect();
    assert!(
        blended.is_empty(),
        "a repeat of the frame was blended in on {blended:?}"
    );
}

/// The neighbours are the raw frames, so a Denoise after a correction must
/// not average them in, or most of the correction is undone where the frames
/// agree. Not first, it works as it does at Frames 0.
#[test]
fn a_denoise_after_a_correction_leaves_the_correction_alone() {
    if HeadlessRenderer::new().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for n in 0..FRAMES {
        frame(dir.path(), "plate", n, 0.0);
    }
    let graded = |frames: f64| {
        let mut exposure = lumit_core::fx::builtins::instantiate("exposure").unwrap();
        for p in &mut exposure.params {
            if p.id == "stops" {
                p.value = EffectValue::Float(Property::fixed(0.05));
            }
        }
        let (mut doc, comp) = comp_doc();
        let item = footage(&mut doc, dir.path(), "plate_0000.png", true);
        let effects = vec![exposure, denoise_over(0, frames)];
        let plate = layer(&doc, comp, LayerKind::Footage { item }, effects);
        add(&mut doc, comp, plate);
        shown(&Arc::new(doc), comp, 4)
    };
    assert!(
        graded(2.0) == graded(0.0),
        "the raw frames either side were averaged into the graded one"
    );
}

/// A Sequence layer cut inside one source, with the middle taken out: the
/// frames across the cut are another part of the footage, so the frame
/// before it is denoised as if the next clip were another file's.
#[test]
fn a_cut_inside_one_source_is_not_averaged_across() {
    if HeadlessRenderer::new().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for n in 0..16 {
        frame(dir.path(), "plate", n, 0.0);
    }
    let at = |n: i64| Rational::new(n, 25).unwrap();
    // Frames 0 to 3, then 10 to 13: of the same item, or of a second item
    // reading the same files, which is another shot whatever it shows.
    let cut = |same: bool| {
        let (mut doc, comp) = comp_doc();
        let first = footage(&mut doc, dir.path(), "plate_0000.png", true);
        let second = if same {
            first
        } else {
            footage(&mut doc, dir.path(), "plate_0000.png", true)
        };
        let clip = |item, from| {
            Clip::new(
                ClipSource::Footage(item),
                at(from),
                at(from + 4),
                at(0),
                at(4),
            )
        };
        let mut later = clip(second, 10);
        later.place_start = at(4);
        let clips = vec![clip(first, 0), later];
        let row = layer(&doc, comp, LayerKind::Sequence { clips }, vec![denoise(0)]);
        add(&mut doc, comp, row);
        shown(&Arc::new(doc), comp, 3)
    };
    assert!(
        cut(true) == cut(false),
        "the frames after the cut were averaged into the frame before it"
    );
}
