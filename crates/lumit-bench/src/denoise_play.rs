//! **B41: Denoise played forward over 1080p footage** (docs/13 §2).
//!
//! One footage layer of the reference clip with a Denoise at its defaults:
//! two frames either side, lined up along the measured motion. Played forward
//! from a renderer that has drawn one frame, so the shaders are built but no
//! frame or motion is cached, and reported as the 95th percentile of the
//! milliseconds each frame took, as docs/13 states its budgets. Each frame
//! decodes one new frame and measures one new pair, which is what playing a
//! denoised shot costs.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::comp::{add_footage, layer, rat};
use crate::scenarios::{elapsed_ms, p95, Measurement};
use lumit_core::anim::Property;
use lumit_core::model::{Composition, Document, EffectValue, LayerKind, LinearColour, ProjectItem};
use lumit_core::time::{Duration, FrameRate};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use uuid::Uuid;

/// A second of playback at the clip's rate.
const PLAY: u64 = 60;

/// Measure B41. `Err` when there is no graphics adapter or no media.
pub fn budgets(
    media_dir: &Path,
    progress: &mut dyn FnMut(Measurement),
) -> Result<Vec<Measurement>, String> {
    let media = crate::media::generate(media_dir)?;
    let mut doc = Document::new();
    let span = rat(10, 1);
    let clip = add_footage(&mut doc, "ref_a.mp4", &media.clip_a);
    let mut plate = layer("Plate", LayerKind::Footage { item: clip }, span);
    plate.transform = lumit_core::edits::centred_transform(1920.0, 1080.0, 1920, 1080);
    let mut e = lumit_core::fx::builtins::instantiate_for_raster("denoise", 1920.0, 1080.0)
        .ok_or_else(|| "denoise is not in the catalogue".to_string())?;
    for p in &mut e.params {
        if p.id == "luma_noise" {
            p.value = EffectValue::Float(Property::fixed(4.0));
        }
    }
    plate.effects = vec![e];
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "B41".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).map_err(|e| e.to_string())?,
        duration: Duration(span),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![plate],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);
    let mut r = HeadlessRenderer::new()?;
    let full = Quality {
        draft: false,
        auto_res: false,
        display_scale: 1.0,
        divisor: 1,
    };
    // Far from the frames played, so nothing it decodes or measures is reused.
    r.render_prepared(&doc, comp_id, 400, full, false, true)?;
    r.settle_gpu();
    let count = crate::scenarios::span_scaled(PLAY);
    let mut each = Vec::new();
    for f in 0..count {
        let t = Instant::now();
        r.render_prepared(&doc, comp_id, f, full, false, true)?;
        r.settle_gpu();
        each.push(elapsed_ms(t));
    }
    let ms = p95(&mut each);
    eprintln!("lumit-bench: B41 {ms:.2} ms a frame (95th percentile)");
    let m = Measurement {
        budget: "B41",
        value_ms: ms,
        frames: count,
    };
    progress(m);
    Ok(vec![m])
}
