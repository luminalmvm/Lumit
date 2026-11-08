//! **B40: smoke and fire at 128 cells a side** (docs/13 §2).
//!
//! A cube of gas filled by a hot, burning emitter, stepped on the card with the
//! stopwatch around the wait for each step, then marched at 1080p with
//! it filling most of the frame. The worse of the two is the number.

use crate::comp::{layer, rat};
use crate::scenarios::Measurement;
use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, EffectValue, LayerKind, LinearColour, ProjectItem, SolidDef,
};
use lumit_core::time::{Duration, FrameRate};
use std::sync::Arc;
use uuid::Uuid;

/// Frames for the gas to fill the box.
const WARM: i64 = 90;
const RUNS: i64 = 30;

/// Measure B40. `Err` when there is no graphics adapter.
pub fn budgets(progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
    let mut doc = Document::new();
    let span = rat(10, 1);
    let solid = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: solid,
        name: "Box".into(),
        colour: LinearColour([0.0; 4]),
        width: 1000,
        height: 1000,
        extra: serde_json::Map::new(),
    }));
    let mut host = layer("Smoke", LayerKind::Solid { def: solid }, span);
    host.switches.three_d = true;
    host.transform.anchor_x = Property::fixed(500.0);
    host.transform.anchor_y = Property::fixed(500.0);
    host.transform.position_x = Property::fixed(960.0);
    host.transform.position_y = Property::fixed(540.0);
    let mut e = lumit_core::fx::builtins::instantiate("smoke")
        .ok_or_else(|| "smoke is not in the catalogue".to_string())?;
    for p in &mut e.params {
        let v = match p.id.as_str() {
            "resolution" => {
                p.value = EffectValue::Choice(2);
                continue;
            }
            "temperature" => 100.0,
            "fuel" => 100.0,
            "emit_size" => 60.0,
            "dissipation" => 5.0,
            _ => continue,
        };
        p.value = EffectValue::Float(Property::fixed(v));
    }
    let effect = e.id;
    host.effects = vec![e];
    let host_id = host.id;
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "B40".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).map_err(|e| e.to_string())?,
        duration: Duration(span),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![host],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);
    let warm = crate::scenarios::span_scaled(WARM as u64) as i64;
    let runs = crate::scenarios::span_scaled(RUNS as u64) as i64;
    let (step, march) = lumit_render::sim::smoke::timed(&doc, comp_id, host_id, effect, warm, runs)
        .ok_or_else(|| "no graphics adapter".to_string())?;
    eprintln!("lumit-bench: B40 step {step:.2} ms, march {march:.2} ms");
    let m = Measurement {
        budget: "B40",
        value_ms: step.max(march),
        frames: runs as u64,
    };
    progress(m);
    Ok(vec![m])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    #[ignore = "a timing run: cargo test --release -p lumit-bench -- --ignored smoke_step"]
    fn smoke_step_budget() {
        super::budgets(&mut |m| eprintln!("{m:?}")).unwrap();
    }
}
