//! Smoke and fire through the whole renderer: a frame comes out the same however
//! it was reached, a bake stands in for the walk, and a card that can't hold the
//! grid draws no smoke rather than falling over.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::edits::base_layer;
use lumit_core::fx::builtins::instantiate;
use lumit_core::model::{
    ColourDepth, Composition, Document, EffectInstance, EffectValue, LayerKind, LinearColour,
    ProjectItem, SolidDef, TransformGroup,
};
use lumit_core::time::{Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::track::{progress, Progress};
use std::sync::Arc;
use uuid::Uuid;

const W: u32 = 320;
const H: u32 = 180;

fn float(e: &mut EffectInstance, id: &str, v: f64) {
    e.params.iter_mut().find(|p| p.id == id).unwrap().value =
        EffectValue::Float(Property::fixed(v));
}

/// A small fire in a box, on a dark comp. What it returns names its comp, layer
/// and effect.
fn project(depth: ColourDepth, seconds: i32) -> (Arc<Document>, Uuid, Uuid, Uuid) {
    let mut doc = Document::new();
    doc.colour_depth = depth;
    let def = Uuid::now_v7();
    let (w, h) = (160, 170);
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Box".into(),
        colour: LinearColour([0.0; 4]),
        width: w,
        height: h,
        extra: serde_json::Map::new(),
    }));
    let span = Rational::new(i64::from(seconds), 1).unwrap();
    let mut layer = base_layer(
        "Fire".into(),
        LayerKind::Solid { def },
        span,
        TransformGroup {
            anchor_x: Property::fixed(f64::from(w) * 0.5),
            anchor_y: Property::fixed(f64::from(h) * 0.5),
            position_x: Property::fixed(f64::from(W) * 0.5),
            position_y: Property::fixed(f64::from(H) * 0.5),
            ..TransformGroup::default()
        },
    );
    layer.switches.three_d = true;
    let mut e = instantiate("smoke").unwrap();
    // The coarsest grid: this is about where frames come from, not how they look.
    e.params
        .iter_mut()
        .find(|p| p.id == "resolution")
        .unwrap()
        .value = EffectValue::Choice(0);
    float(&mut e, "temperature", 100.0);
    float(&mut e, "fuel", 100.0);
    float(&mut e, "emit_size", 60.0);
    let (layer_id, effect) = (layer.id, e.id);
    layer.effects = vec![e];
    let id = Uuid::now_v7();
    doc.items.push(ProjectItem::Composition(Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id,
        name: "Smoke".into(),
        width: W,
        height: H,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(span),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }));
    (Arc::new(doc), id, layer_id, effect)
}

fn walked_alone(doc: &Arc<Document>, comp: Uuid, frame: u64) -> Option<Vec<u8>> {
    let mut renderer = HeadlessRenderer::new().ok()?;
    renderer
        .render_rgba(doc, comp, frame, 1.0)
        .ok()
        .map(|f| f.0)
}

/// A frame played to, scrubbed back to from past it, or read off a kept state is
/// the frame walked from the in point, to the byte, which is what lets an export
/// match the Viewer. The fire draws at every colour depth a project can be.
#[test]
fn a_frame_is_the_same_however_it_is_reached() {
    let (doc, id, _, _) = project(ColourDepth::Sixteen, 4);
    let Some(walked) = walked_alone(&doc, id, 44) else {
        lumit_gpu::no_adapter();
        return;
    };
    assert!(walked.chunks_exact(4).any(|p| p[0] > 60), "the fire drew");
    let mut r = HeadlessRenderer::new().unwrap();
    for f in [10u64, 30, 60, 90] {
        r.render_rgba(&doc, id, f, 1.0).unwrap();
    }
    // Back past where it stopped, from states kept on the way, on a renderer that
    // has never drawn this frame.
    assert!(
        r.render_rgba(&doc, id, 44, 1.0).unwrap().0 == walked,
        "scrubbed back"
    );
    lumit_render::sim::clear_kept();
    lumit_render::sim::let_go();
    assert!(
        walked_alone(&doc, id, 44).unwrap() == walked,
        "from the start again"
    );
    for depth in [ColourDepth::Eight, ColourDepth::ThirtyTwo] {
        let (doc, id, _, _) = project(depth, 2);
        let drew = walked_alone(&doc, id, 20).unwrap();
        assert!(drew.chunks_exact(4).any(|p| p[0] > 60), "{depth:?} drew");
    }
}

/// A particle-fed walk stopped part way carries on to the frame walked whole: a
/// stop must never step on what was laid for another frame.
#[test]
fn a_stop_leaves_a_particle_fed_walk_right() {
    use lumit_eval::epoch::Epoch;
    use lumit_render::cancel::Scope;
    let (doc, id, layer, _) = project(ColourDepth::Sixteen, 2);
    let mut fed = Document::clone(&doc);
    let c = fed.comp_mut(id).unwrap();
    let mut parts = c.layers[0].clone();
    parts.id = Uuid::now_v7();
    parts.switches.three_d = true;
    let mut p = instantiate("particulate").unwrap();
    p.params
        .iter_mut()
        .find(|q| q.id == "simulate")
        .unwrap()
        .value = EffectValue::Bool(true);
    float(&mut p, "emit_rate", 2000.0);
    float(&mut p, "max_particles", 200_000.0);
    parts.effects = vec![p];
    let source = parts.id;
    c.layers.push(parts);
    let e = &mut c.layers.iter_mut().find(|l| l.id == layer).unwrap().effects[0];
    e.params
        .iter_mut()
        .find(|q| q.id == "emit_from")
        .unwrap()
        .value = EffectValue::Choice(1);
    e.params
        .iter_mut()
        .find(|q| q.id == "emit_layer")
        .unwrap()
        .value = EffectValue::Layer(Some(source));
    let doc = Arc::new(fed);
    let Some(whole) = walked_alone(&doc, id, 50) else {
        lumit_gpu::no_adapter();
        return;
    };
    lumit_render::sim::clear_kept();
    lumit_render::sim::let_go();
    let epoch = Epoch::new();
    let scope = Scope::new(epoch.token());
    let stopper = {
        let epoch = epoch.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(15));
            epoch.bump();
        })
    };
    let mut r = HeadlessRenderer::new().unwrap();
    let _ = r.render_rgba(&doc, id, 50, 1.0);
    stopper.join().unwrap();
    drop(scope);
    let _ = lumit_render::cancel::drawn_short();
    assert!(r.render_rgba(&doc, id, 50, 1.0).unwrap().0 == whole);
}

/// A bake stands in for the walk, and is the same frame. A bake with no room on
/// the card is refused in words, with nothing written.
#[test]
fn a_baked_frame_is_the_walked_frame() {
    let (doc, id, layer, effect) = project(ColourDepth::Sixteen, 2);
    let Some(walked) = walked_alone(&doc, id, 44) else {
        lumit_gpu::no_adapter();
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    lumit_render::sim::bake::set_root(Some(folder.path().to_path_buf()));
    let asked = lumit_render::sim::bake::press(Arc::clone(&doc), id, layer, effect);
    assert!(matches!(asked, lumit_render::track::Requested::Started));
    loop {
        match progress(effect) {
            Some(Progress::Done) => break,
            Some(Progress::Failed(why)) => panic!("the bake failed: {why:?}"),
            _ => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    assert_eq!(
        lumit_render::sim::bake::status(&doc, id, layer, effect),
        Some((60, 60))
    );
    use std::sync::atomic::Ordering;
    lumit_render::sim::clear_kept();
    lumit_render::sim::let_go();
    let reads = lumit_render::sim::bake::READS.load(Ordering::Relaxed);
    let baked = walked_alone(&doc, id, 44).unwrap();
    assert!(
        lumit_render::sim::bake::READS.load(Ordering::Relaxed) > reads,
        "the frame came off the bake"
    );
    assert!(baked == walked, "a baked frame is the walked frame");
    lumit_render::sim::bake::set_root(None);
}

/// The finest grid on a card with no room draws no smoke, says so, and comes
/// back once there is room.
#[test]
fn a_grid_too_big_for_the_card_is_refused() {
    let (doc, id, layer, effect) = project(ColourDepth::Sixteen, 2);
    let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = lumit_gpu::fx::FxEngine::new(&ctx);
    ctx.ledger().set_budget(lumit_budget::Tier::Vram, 1024);
    assert!(!lumit_render::sim::smoke::fits(
        &fx, &ctx, &doc, id, layer, effect
    ));
    assert!(matches!(
        progress(effect),
        Some(Progress::Failed(lumit_render::track::AnalysisError::TooBig))
    ));
    ctx.ledger().set_budget(lumit_budget::Tier::Vram, 8 << 30);
    assert!(lumit_render::sim::smoke::fits(
        &fx, &ctx, &doc, id, layer, effect
    ));
    assert!(progress(effect).is_none(), "the words go once it fits");
}
