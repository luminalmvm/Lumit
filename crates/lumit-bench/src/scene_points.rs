//! **B31 — a million particles in the 3D scene** (docs/13 §2).
//!
//! A whole frame through the renderer: Particulate in the scene at its hard
//! cap, sorted, with a 3D card in the way and a light on, so the depth the
//! particles meet and the light they take are timed with them. The same
//! frame with the particles drawn flat on their layer is the floor, so what is
//! left is what the scene costs over B14.
//!
//! **B38 — a 1080p shot's Depth points**: one a pixel at Density full, through a
//! camera off to the side and turned, over the same layer drawn flat.

use crate::comp::{layer, rat};
use crate::scenarios::{elapsed_ms, Measurement};
use lumit_core::anim::Property;
use lumit_core::model::{
    CompLighting, Composition, Document, EffectValue, LayerKind, LightDef, LightKind, LinearColour,
    ProjectItem, SolidDef,
};
use lumit_core::time::{Duration, FrameRate};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::planes::{publish, run_from_planes, PlaneKind, PlaneOut};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// Frames behind the stopwatch, after the warm-up.
const RUNS: u32 = 10;
const WARMUP: u32 = 3;
/// The fastest of these is kept, as for Particulate's three.
const REPEATS: u32 = 3;
/// Four seconds in at 60 fps: two whole lifetimes, so the cloud is full.
const FRAME: u64 = 240;

/// The comp, its particles in the scene or flat on their layer, and its id.
fn comp(in_scene: bool) -> Result<(Document, Uuid), String> {
    let mut doc = Document::new();
    let span = rat(10, 1);
    let solid = |doc: &mut Document, name: &str, colour: [f32; 4], w: u32, h: u32| {
        let id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id,
            name: name.into(),
            colour: LinearColour(colour),
            width: w,
            height: h,
            extra: serde_json::Map::new(),
        }));
        LayerKind::Solid { def: id }
    };
    let at = |l: &mut lumit_core::model::Layer, x: f64, y: f64, z: f64, w: f64, h: f64| {
        l.transform.position_x = Property::fixed(x);
        l.transform.position_y = Property::fixed(y);
        l.transform.position_z = Property::fixed(z);
        l.transform.anchor_x = Property::fixed(w / 2.0);
        l.transform.anchor_y = Property::fixed(h / 2.0);
        l.switches.three_d = true;
    };
    let kind = solid(&mut doc, "Card", [0.5, 0.5, 0.5, 1.0], 900, 600);
    let mut card = layer("Card", kind, span);
    at(&mut card, 1100.0, 540.0, -200.0, 900.0, 600.0);
    card.transform.rotation_y = Property::fixed(30.0);
    let light = LightDef {
        kind: LightKind::Point,
        intensity: Property::fixed(100.0),
        ..LightDef::default()
    };
    let mut light = layer(
        "Light",
        LayerKind::Light {
            light: Box::new(light),
        },
        span,
    );
    light.transform.position_x = Property::fixed(500.0);
    light.transform.position_y = Property::fixed(200.0);
    light.transform.position_z = Property::fixed(-800.0);
    let kind = solid(&mut doc, "Particles", [0.0; 4], 1920, 1080);
    let mut cloud = layer("Particles", kind, span);
    at(&mut cloud, 960.0, 540.0, 0.0, 1920.0, 1080.0);
    let mut fx = lumit_core::fx::builtins::instantiate("particulate")
        .ok_or_else(|| "particulate is not in the catalogue".to_string())?;
    for p in &mut fx.params {
        let float = |v: f64| EffectValue::Float(Property::fixed(v));
        p.value = match p.id.as_str() {
            "in_scene" => EffectValue::Bool(in_scene),
            "shape" => EffectValue::Choice(7),
            "width" => float(1600.0),
            "height" => float(900.0),
            "depth" => float(900.0),
            "emit_rate" => float(500_000.0),
            "size" => float(2.0),
            "life" => float(2.0),
            "max_particles" => float(1_000_000.0),
            _ => continue,
        };
    }
    cloud.effects = vec![fx];
    let layers = vec![light, card, cloud];
    let c = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "B31".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).map_err(|e| format!("{e:?}"))?,
        duration: Duration(span),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        lighting: CompLighting::AfterEffects { linear: true },
        extra: serde_json::Map::new(),
    };
    let id = c.id;
    doc.items.push(ProjectItem::Composition(c));
    Ok((doc, id))
}

/// Milliseconds per frame of one of the two comps, the fastest of the repeats.
fn time(r: &mut HeadlessRenderer, in_scene: bool) -> Result<f64, String> {
    let (doc, id) = comp(in_scene)?;
    let doc = Arc::new(doc);
    for _ in 0..WARMUP {
        r.render_rgba(&doc, id, FRAME, 1.0)?;
    }
    let mut best = f64::INFINITY;
    for _ in 0..REPEATS {
        let started = Instant::now();
        for _ in 0..RUNS {
            r.render_rgba(&doc, id, FRAME, 1.0)?;
        }
        best = best.min(elapsed_ms(started) / f64::from(RUNS));
    }
    Ok(best)
}

/// A 1080p comp of one 3D solid seen through a camera turned `off` degrees from
/// square on, its Depth points on it if `points`, and its id.
fn shot(points: bool, off: f64) -> Result<(Document, Uuid), String> {
    let mut doc = Document::new();
    let def = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Shot".into(),
        colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
        width: 1920,
        height: 1080,
        extra: serde_json::Map::new(),
    }));
    let span = rat(10, 1);
    let mut plate = layer("Shot", LayerKind::Solid { def }, span);
    plate.transform.position_x = Property::fixed(960.0);
    plate.transform.position_y = Property::fixed(540.0);
    plate.transform.anchor_x = Property::fixed(960.0);
    plate.transform.anchor_y = Property::fixed(540.0);
    plate.switches.three_d = true;
    if points {
        let fx = lumit_core::fx::builtins::instantiate(lumit_core::planes::DEPTH_POINTS)
            .ok_or_else(|| "Depth points is not in the catalogue".to_string())?;
        // A depth with some shape to it, the size a model hands back.
        let (w, h) = (518_u32, 294_u32);
        let data = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let d = 0.5 + 0.5 * (f64::from(x) * 0.03).sin() * (f64::from(y) * 0.04).cos();
                ((d * 65535.0) as u16).to_le_bytes()
            })
            .collect();
        let plane = PlaneOut {
            width: w,
            height: h,
            kind: PlaneKind::Depth,
            data,
        };
        let run = run_from_planes(60.0, 1, "B38", &[(0, plane)])
            .ok_or_else(|| "no run from the plane".to_string())?;
        publish(fx.id, run);
        plate.effects = vec![fx];
    }
    let mut comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "B38".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).map_err(|e| format!("{e:?}"))?,
        duration: Duration(span),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: Default::default(),
        lighting: Default::default(),
        extra: serde_json::Map::new(),
    };
    // Round the comp's middle, at the same distance a fresh camera stands.
    let mut camera = lumit_core::edits::fresh_camera(&comp, "Camera".into(), None);
    let far = lumit_core::camera::default_zoom(1920.0);
    let (s, c) = off.to_radians().sin_cos();
    camera.transform.position_x = Property::fixed(960.0 - far * s);
    camera.transform.position_z = Property::fixed(-far * c);
    if let LayerKind::Camera { options, .. } = &mut camera.kind {
        options.two_node = false;
    }
    camera.transform.rotation_y = Property::fixed(off);
    comp.layers = vec![camera, plate];
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    Ok((doc, id))
}

/// Milliseconds per frame of a comp, the fastest of the repeats.
fn timed(r: &mut HeadlessRenderer, (doc, id): (Document, Uuid)) -> Result<f64, String> {
    let doc = Arc::new(doc);
    for _ in 0..WARMUP {
        r.render_rgba(&doc, id, 0, 1.0)?;
    }
    let mut best = f64::INFINITY;
    for _ in 0..REPEATS {
        let started = Instant::now();
        for _ in 0..RUNS {
            r.render_rgba(&doc, id, 0, 1.0)?;
        }
        best = best.min(elapsed_ms(started) / f64::from(RUNS));
    }
    Ok(best)
}

/// Measure B31 and B38. `Err` when there is no graphics adapter.
pub fn budgets(progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
    let mut r = HeadlessRenderer::new()?;
    let square = timed(&mut r, shot(true, 0.0)?)? - timed(&mut r, shot(false, 0.0)?)?;
    eprintln!("lumit-bench: Depth points square on {square:.3} ms over flat");
    let floor = timed(&mut r, shot(false, 40.0)?)?;
    let each = timed(&mut r, shot(true, 40.0)?)?;
    eprintln!("lumit-bench: B38 flat floor {floor:.3} ms");
    let b36 = Measurement {
        budget: "B38",
        value_ms: (each - floor).max(0.0),
        frames: u64::from(RUNS),
    };
    progress(b36);
    let floor = time(&mut r, false)?;
    let each = time(&mut r, true)?;
    eprintln!("lumit-bench: B31 flat floor {floor:.3} ms");
    let m = Measurement {
        budget: "B31",
        value_ms: (each - floor).max(0.0),
        frames: u64::from(RUNS),
    };
    progress(m);
    Ok(vec![m, b36])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    #[ignore = "a timing run: cargo test --release -p lumit-bench -- --ignored scene_points"]
    fn scene_points_budget() {
        super::budgets(&mut |m| eprintln!("{m:?}")).unwrap();
    }
}
