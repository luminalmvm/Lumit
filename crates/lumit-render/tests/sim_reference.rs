//! The CPU reference for Particulate's simulation (docs/13 section 6): one step
//! in plain Rust, field for field what `fx_particulate_sim.wgsl` does, held
//! against the card over 60 frames. Flock cells and sparks stay card only.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::fx::builtins::instantiate;
use lumit_core::fx::effects::particulate::Particulate;
use lumit_core::fx::noise::hash01;
use lumit_core::fx::points::emitter::{birth_point, outline_polyline};
use lumit_core::fx::points::stepped::sim::{physics, Ledger};
use lumit_core::fx::points::{attr, draw, integrate, jitter, launch};
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::{
    Composition, Document, EffectValue, Layer, LayerKind, LinearColour, ProjectItem, SolidDef,
    Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use std::sync::Arc;
use uuid::Uuid;

const FPS: u32 = 30;
const FRAMES: i64 = 60;
const FLOOR: f32 = 400.0;
type V = [f32; 3];

fn add(a: V, b: V) -> V {
    std::array::from_fn(|k| a[k] + b[k])
}
fn scale(a: V, s: f32) -> V {
    a.map(|v| v * s)
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// == `sim_grad`: the gradient of one channel of value noise.
fn grad(seed: u32, ch: u32, q: V) -> V {
    let f = q.map(f32::floor);
    let x: V = std::array::from_fn(|k| q[k] - f[k]);
    let i = f.map(|v| v as i32);
    let u = x.map(|v| v * v * v * (v * (v * 6.0 - 15.0) + 10.0));
    let du = x.map(|v| 30.0 * v * v * (v * (v - 2.0) + 1.0));
    let h = |a: i32, b: i32, c: i32| hash01(seed, ch, i[0] + a, i[1] + b, i[2] + c);
    let (a, b, c, d) = (h(0, 0, 0), h(1, 0, 0), h(0, 1, 0), h(1, 1, 0));
    let (e, g, m, k) = (h(0, 0, 1), h(1, 0, 1), h(0, 1, 1), h(1, 1, 1));
    let (kx, ky, kz) = (b - a, c - a, e - a);
    let (kxy, kyz, kzx) = (a - b - c + d, a - c - e + m, a - b - e + g);
    let kxyz = -a + b + c - d + e - g - m + k;
    [
        2.0 * du[0] * (kx + kxy * u[1] + kzx * u[2] + kxyz * u[1] * u[2]),
        2.0 * du[1] * (ky + kyz * u[2] + kxy * u[0] + kxyz * u[2] * u[0]),
        2.0 * du[2] * (kz + kzx * u[0] + kyz * u[1] + kxyz * u[0] * u[1]),
    ]
}

/// == `sim_curl`.
fn curl(rows: &Particulate, seed: u32, at: V, clock: f32) -> V {
    let q = scale(at, 1.0 / rows.turbulence_scale.max(1e-3));
    let s = clock * rows.turbulence_speed;
    let a = grad(seed, 64, add(q, [s, 0.0, 31.7]));
    let b = grad(seed, 65, add(q, [11.3, s, 0.0]));
    let c = grad(seed, 66, add(q, [0.0, 47.1, s]));
    [c[1] - b[2], a[2] - c[0], b[0] - a[1]]
}

/// == `sim_push`.
fn push(rows: &Particulate, seed: u32, at: V, vel: V, clock: f32) -> V {
    let phys = physics(rows);
    let force = rows.turbulence_amount * rows.drag.max(0.5) * 2.0;
    let mut a = scale(curl(rows, seed, at, clock), force);
    let d: V = std::array::from_fn(|k| at[k] - phys.field[k]);
    let r = dot(d, d).sqrt();
    if r > 1e-3 {
        let out = scale(d, 1.0 / r);
        let reach = r / phys.field[3];
        let fade = 1.0 / (1.0 + reach * reach);
        a = add(a, scale(out, -phys.attractor * fade));
        if phys.sphere > 0.0 && reach < 1.0 {
            a = add(a, scale(out, phys.sphere * (1.0 - reach)));
        }
        let axis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]][phys.axis as usize];
        let side = cross(axis, d);
        let len = dot(side, side).sqrt();
        if phys.vortex != 0.0 && len > 1e-3 {
            let round = scale(side, 1.0 / len);
            let along = dot(vel, round);
            let wants = phys.vortex * fade;
            let inward = cross(round, axis);
            a = add(a, scale(round, (wants - along) * 4.0));
            a = add(a, scale(inward, along * along.abs() / len));
        }
    }
    a
}

#[derive(Clone, Copy)]
struct Part {
    id: u64,
    at: V,
    vel: V,
    age: f32,
    life: f32,
}

/// One walk's worth of [`Part`]s, each frame cut into the same steps as the card's.
fn walk(rows: &Particulate, dt: f64) -> Vec<Part> {
    let p = rows.points().framed(1920.0, 1080.0);
    let phys = physics(rows);
    let path = outline_polyline(&p.emitter);
    let mut ledger = Ledger::default();
    let mut parts: Vec<Part> = Vec::new();
    for f in 0..FRAMES {
        let steady = ledger.steady(rows.emit_rate, dt, p.cap);
        let each = dt / f64::from(phys.steps);
        for k in 0..phys.steps {
            let end = f64::from(k + 1) * each;
            let (h, clock) = (each as f32, (f as f64 * dt + end) as f32);
            parts.retain_mut(|q| {
                let age = q.age + h;
                if age >= q.life {
                    return false;
                }
                let v0 = add(q.vel, scale(push(rows, p.seed, q.at, q.vel, clock), h));
                let (mut to, mut vel) = integrate(q.at, v0, &p.forces, h);
                // == the floor in `sim_first` and `sim_move`, Bounce with no friction.
                if q.at[1] < FLOOR && to[1] >= FLOOR {
                    let s = (FLOOR - q.at[1]) / (to[1] - q.at[1]);
                    let rest = (to[1] - FLOOR) * phys.keep;
                    let on: V = std::array::from_fn(|k| q.at[k] + (to[k] - q.at[k]) * s);
                    to = [to[0], FLOOR - 1e-3 - rest, to[2]];
                    let _ = on;
                    vel[1] = -vel[1] * phys.keep;
                }
                (q.at, q.vel, q.age) = (to, vel, age);
                true
            });
            let (a, b) = steady.between(f64::from(k) * each, end);
            for j in a..b {
                let id = steady.first + u64::from(j);
                let off0 = steady.off(a) as f32;
                let spacing = (steady.dt / f64::from(steady.n.max(1))) as f32;
                let age = (end as f32 - (off0 + (j - a) as f32 * spacing)).max(0.0);
                let dice = [attr::EMIT_U, attr::EMIT_V, attr::EMIT_W].map(|a| draw(p.seed, id, a));
                let p0 = birth_point(&p.emitter, &path, p.frame, dice).unwrap();
                let (at, vel) = integrate(p0, launch(&p, id), &p.forces, age);
                let life = jitter(
                    p.particle.life,
                    p.particle.life_jitter,
                    draw(p.seed, id, attr::LIFE),
                );
                parts.push(Part {
                    id,
                    at,
                    vel,
                    age,
                    life,
                });
            }
        }
        ledger.advance(rows.emit_rate, &steady);
    }
    parts
}

fn layer(def: Uuid) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "Particles".into(),
        kind: LayerKind::Solid { def },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(30, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 2,
        volume_db: Property::zero(),
        pan: Property::zero(),
        audio_only: false,
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
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
    }
}

/// A thousand particles under every field at once, and a floor to bounce on: the
/// card lands where the reference does.
#[test]
fn the_card_walks_where_the_reference_walks() {
    let mut e = instantiate("particulate").unwrap();
    let mut float = |id: &str, v: f64| {
        let row = e.params.iter_mut().find(|p| p.id == id).unwrap();
        row.value = EffectValue::Float(Property::fixed(v));
    };
    float("emit_rate", 500.0);
    float("max_particles", 1000.0);
    float("life", 2.0);
    float("gravity", 300.0);
    float("wind_x", 30.0);
    float("drag", 0.7);
    float("turbulence_amount", 60.0);
    float("turbulence_scale", 150.0);
    float("position_x", 480.0);
    float("position_y", 100.0);
    float("field_x", 480.0);
    float("field_y", 200.0);
    float("field_radius", 200.0);
    float("attractor", 150.0);
    float("vortex", 60.0);
    float("sphere", 300.0);
    float("bounciness", 50.0);
    float("friction", 0.0);
    float("ground_height", 540.0 - f64::from(FLOOR));
    for id in ["simulate", "ground"] {
        e.params.iter_mut().find(|p| p.id == id).unwrap().value = EffectValue::Bool(true);
    }
    let effect = e.id;
    let ops = lumit_core::fx::resolved::resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &lumit_core::fx::markers::MarkerContext::NONE,
        Arc::new(lumit_core::expression::ExpressionContext::detached()),
    );
    let rows = Particulate::read(ops.get(0).unwrap().params);

    let def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Solid".into(),
        colour: LinearColour([0.0, 0.0, 0.0, 1.0]),
        width: 1920,
        height: 1080,
        extra: serde_json::Map::new(),
    }));
    let mut host = layer(def);
    let host_id = host.id;
    host.effects = vec![e];
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Sim".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(FPS, 1).unwrap(),
        duration: Duration(Rational::new(30, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![host],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);
    let Some(card) = lumit_render::sim::state_after(&doc, id, host_id, effect, FRAMES) else {
        return;
    };
    let reference = walk(&rows, 1.0 / f64::from(FPS));
    assert!(reference.len() > 500, "a real cloud: {}", reference.len());
    assert_eq!(card.len(), reference.len(), "the same particles live");
    let mut worst = 0.0f32;
    for ((state, gpu_id), q) in card.iter().zip(&reference) {
        assert_eq!(*gpu_id, q.id, "in the same order");
        for (a, b) in state.iter().zip(&q.at) {
            worst = worst.max((a - b).abs());
        }
    }
    // Cards round their floats their own way and each step carries on from the
    // last, so this drifts by a different amount on each: 0.005 px here, up to
    // 0.59 on CI's Macs. Two pixels leaves room for that, and a step that moved
    // particles differently would pass it in the first few frames. Whether one
    // card gives the same frame twice is a_frame_is_the_same_however_it_is_reached.
    assert!(worst < 2.0, "worst place {worst} px");
}
