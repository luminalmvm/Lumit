//! **B37: a million particles, one step** (docs/13 §2).
//!
//! Particulate's Simulate at its hard cap, full, with the flow on and a 3D card
//! to hit, stepped on the card with the stopwatch around the wait for each step.
//! The draw is B14's and B31's, so it is not in here.

use crate::comp::{layer, rat};
use crate::scenarios::Measurement;
use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, EffectValue, LayerKind, LinearColour, ProjectItem, SolidDef,
};
use lumit_core::time::{Duration, FrameRate};
use std::sync::Arc;
use uuid::Uuid;

/// Frames to fill the cloud: its life, at the rate below.
const WARM: i64 = 60;
const RUNS: i64 = 30;

/// Measure B37. `Err` when there is no graphics adapter.
pub fn budgets(progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
    let mut doc = Document::new();
    let span = rat(10, 1);
    let solid = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: solid,
        name: "Card".into(),
        colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
        width: 1500,
        height: 900,
        extra: serde_json::Map::new(),
    }));
    let mut card = layer("Card", LayerKind::Solid { def: solid }, span);
    card.switches.three_d = true;
    card.transform.position_x = Property::fixed(960.0);
    card.transform.position_y = Property::fixed(800.0);
    card.transform.rotation_x = Property::fixed(90.0);
    let mut host = layer("Particles", LayerKind::Null, span);
    host.switches.three_d = true;
    let mut e = lumit_core::fx::builtins::instantiate("particulate")
        .ok_or_else(|| "particulate is not in the catalogue".to_string())?;
    for p in &mut e.params {
        let v = match p.id.as_str() {
            "simulate" => {
                p.value = EffectValue::Bool(true);
                continue;
            }
            "sub_steps" => 1.0,
            "max_particles" => 1_000_000.0,
            "emit_rate" => 2_000_000.0,
            "life" => 2.0,
            "life_jitter" => 0.0,
            "gravity" => 100.0,
            "turbulence_amount" => 200.0,
            "width" => 800.0,
            "height" => 300.0,
            "depth" => 800.0,
            "position_x" => 960.0,
            "position_y" => 400.0,
            _ => continue,
        };
        p.value = EffectValue::Float(Property::fixed(v));
    }
    for p in &mut e.params {
        if p.id == "shape" {
            p.value = EffectValue::Choice(7);
        }
        if p.id == "collide_with" {
            p.value = EffectValue::Layer(Some(card.id));
        }
    }
    let effect = e.id;
    let host_id = host.id;
    host.effects = vec![e];
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Sim".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).map_err(|e| e.to_string())?,
        duration: Duration(span),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![host, card],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);
    let each = lumit_render::sim::timed_steps(&doc, comp_id, host_id, effect, WARM, RUNS)
        .ok_or_else(|| "no graphics adapter".to_string())?;
    let m = Measurement {
        budget: "B37",
        value_ms: each,
        frames: RUNS as u64,
    };
    progress(m);
    Ok(vec![m])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    #[ignore = "a timing run: cargo test --release -p lumit-bench -- --ignored sim_step"]
    fn sim_step_budget() {
        super::budgets(&mut |m| eprintln!("{m:?}")).unwrap();
    }
}
