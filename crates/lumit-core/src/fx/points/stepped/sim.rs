//! Particulate worked out step by step on the graphics card.
//!
//! The rows, births and what is hit are read here on the host, a frame at a
//! time, and the particles themselves only ever live on the card
//! (`lumit-render`'s `sim` and `fx_particulate_sim.wgsl`). A frame is the walk
//! from the layer's in point, as in [`super::stepped`]: kept states only make
//! it faster.

use super::burst_at;
use crate::fx::effects::particulate::Particulate;
use crate::fx::points::{burst_base, Contact};
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer, LayerKind};

fn on(inst: &EffectInstance, id: &str) -> bool {
    matches!(inst.param(id), Some(EffectValue::Bool(true)))
}

/// Whether Particulate `inst` is simulated on the card. Only Simulate says so,
/// so nothing saved before it changes look.
#[must_use]
pub fn wanted(inst: &EffectInstance) -> bool {
    inst.effect.match_name == "particulate" && on(inst, "simulate")
}

/// Rows that only change how particles are drawn or that a walk never reads,
/// so a change to them keeps every state and bake. A row not named here renames
/// the walk, which is the safe way round for a new one.
pub const LOOK_ROWS: &[&str] = &[
    "size",
    "size_jitter",
    "size_over_life",
    "opacity_over_life",
    "colour",
    "end_colour",
    "rotation",
    "rotation_jitter",
    "spin",
    "align_to_motion",
    "size_by",
    "opacity_by",
    "colour_by",
    "speed_min",
    "speed_max",
    "spark_size",
    "mode",
    "feather",
    "sprite_layer",
    "streak_length",
    "particle_blend",
    "mix",
    "density_layer",
    "density_layer_placement",
    "persistence",
    "zoom",
    "turn",
];

/// The layers a simulation hits at any time: Collide with, and every 3D layer
/// that shows for All 3D layers.
pub fn hit_layers<'a>(
    comp: &'a Composition,
    inst: &EffectInstance,
    owner: uuid::Uuid,
) -> impl Iterator<Item = &'a Layer> {
    let named = inst.layer_ref("collide_with");
    let every = on(inst, "collide_all");
    comp.layers.iter().filter(move |l| {
        l.id != owner
            && (Some(l.id) == named
                || every
                    && l.switches.three_d
                    && l.switches.visible
                    && !matches!(
                        l.kind,
                        LayerKind::Camera { .. }
                            | LayerKind::Light { .. }
                            | LayerKind::Null
                            | LayerKind::Adjustment
                    ))
    })
}

/// Everything a layer's place is read from, and nothing else about it, so a
/// rename or a label colour keeps the walk.
fn feed_place(comp: &Composition, l: &Layer, h: &mut blake3::Hasher) {
    h.update(&[u8::from(l.switches.three_d), u8::from(l.switches.visible)]);
    let _ = serde_json::to_writer(&mut *h, &l.transform);
    let _ = serde_json::to_writer(&mut *h, &l.start_offset);
    for id in crate::model::layer_parent_chain(comp, l.id) {
        if let Some(parent) = comp.layers.iter().find(|p| p.id == id) {
            // A parent is placed in 3D by its switch, and a camera or a light
            // by its aim too.
            h.update(&[u8::from(parent.switches.three_d)]);
            if matches!(
                parent.kind,
                LayerKind::Camera { .. } | LayerKind::Light { .. }
            ) {
                let _ = serde_json::to_writer(&mut *h, &parent.kind);
            }
            let _ = serde_json::to_writer(&mut *h, &parent.transform);
            let _ = serde_json::to_writer(&mut *h, &parent.start_offset);
        }
    }
}

/// A layer's shape as a thing to hit or give birth on: its outlines, its size,
/// and a model's content wherever it has been moved to.
fn feed_shape(doc: &Document, l: &Layer, h: &mut blake3::Hasher) {
    let _ = serde_json::to_writer(&mut *h, &l.masks);
    match &l.kind {
        LayerKind::Model { item, settings } => {
            let _ = serde_json::to_writer(&mut *h, settings);
            if let Some(crate::model::ProjectItem::Footage(f)) = doc.item(*item) {
                match &f.media.fingerprint {
                    Some(print) => {
                        let _ = serde_json::to_writer(&mut *h, print);
                    }
                    None => {
                        h.update(f.media.relative_path.as_bytes());
                    }
                }
            }
        }
        LayerKind::Solid { def } => {
            if let Some(solid) = doc.solid(*def) {
                h.update(&solid.width.to_le_bytes());
                h.update(&solid.height.to_le_bytes());
            }
        }
        other => {
            let _ = serde_json::to_writer(&mut *h, other);
        }
    }
}

/// Everything a simulation reads, as one name: its kept states and its bake are
/// found by it. What only changes how it looks is left out, so a change to Size
/// or Colour draws again rather than walking again.
#[must_use]
pub fn definition(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
) -> [u8; 32] {
    use crate::graph::{InputRef, NodeRef};
    let mut h = blake3::Hasher::new_derive_key("lumit card walk 2");
    h.update(&comp.width.to_le_bytes());
    h.update(&comp.height.to_le_bytes());
    let _ = serde_json::to_writer(&mut h, &comp.frame_rate);
    for p in inst
        .params
        .iter()
        .filter(|p| !LOOK_ROWS.contains(&p.id.as_str()))
    {
        let _ = serde_json::to_writer(&mut h, p);
    }
    // A wire into a look row keeps the walk too. The rest name the whole graph
    // and what feeds it from above.
    let wired = layer.graph.edges.iter().any(|e| {
        matches!(&e.to, InputRef::Param { node: NodeRef::Effect(n), port }
            if *n == inst.id && !LOOK_ROWS.contains(&port.as_str()))
    });
    if wired {
        let _ = serde_json::to_writer(&mut h, &layer.graph);
        for above in layer.effects.iter().take_while(|e| e.id != inst.id) {
            let _ = serde_json::to_writer(&mut h, above);
        }
    }
    feed_shape(doc, layer, &mut h);
    let _ = serde_json::to_writer(&mut h, &layer.start_offset);
    // The layer's place only matters against what the particles hit.
    let hits_any =
        on(inst, "ground") || on(inst, "collide_all") || inst.layer_ref("collide_with").is_some();
    if hits_any {
        feed_place(comp, layer, &mut h);
    }
    for l in hit_layers(comp, inst, layer.id) {
        feed_shape(doc, l, &mut h);
        feed_place(comp, l, &mut h);
    }
    let emitter = inst
        .layer_ref("emitter_layer")
        .and_then(|id| comp.layers.iter().find(|l| l.id == id));
    if let Some(l) = emitter {
        feed_shape(doc, l, &mut h);
        feed_place(comp, l, &mut h);
    }
    *h.finalize().as_bytes()
}

/// What the host carries beside the particles from frame to frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ledger {
    /// The frame, from the layer's in point.
    pub frame: i64,
    /// The part of a birth the steady flow owes, as the scan carries it.
    pub carry: f64,
    /// Steady births so far: the next one's index.
    pub births: u64,
    /// Whether Trigger was over a half on this frame.
    pub high: bool,
}

/// One frame's steady births: `n` of them from index `first`, spread through
/// `dt` seconds, of which only the last `cap` could survive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steady {
    pub first: u64,
    pub n: u32,
    /// The first one worth making.
    pub from: u32,
    pub dt: f64,
}

impl Steady {
    /// When birth `j` happens, seconds into the frame.
    #[must_use]
    pub fn off(&self, j: u32) -> f64 {
        (f64::from(j) + 0.5) * self.dt / f64::from(self.n.max(1))
    }

    /// The births from `a` to `b` seconds into the frame, as `first..last`.
    #[must_use]
    pub fn between(&self, a: f64, b: f64) -> (u32, u32) {
        let at = |s: f64| {
            let n = f64::from(self.n);
            ((s / self.dt * n - 0.5).ceil().clamp(0.0, n) as u32).max(self.from)
        };
        (at(a), at(b))
    }
}

impl Ledger {
    /// The steady births the frame after this one owes at Emit rate `rate`.
    #[must_use]
    pub fn steady(&self, rate: f32, dt: f64, cap: u32) -> Steady {
        let rate = f64::from(rate);
        let rate = if rate.is_finite() { rate.max(0.0) } else { 0.0 };
        let n = (self.carry + rate * dt)
            .floor()
            .clamp(0.0, f64::from(u32::MAX)) as u32;
        Steady {
            first: self.births,
            n,
            from: n.saturating_sub(cap),
            dt,
        }
    }

    /// On to the next frame, having made `steady`.
    pub fn advance(&mut self, rate: f32, steady: &Steady) {
        let rate = f64::from(rate);
        let rate = if rate.is_finite() { rate.max(0.0) } else { 0.0 };
        self.carry = (self.carry + rate * steady.dt - f64::from(steady.n)).max(0.0);
        self.births += u64::from(steady.n);
        self.frame += 1;
    }

    /// The burst `rows` throw on this frame: its first index and how many.
    pub fn burst(&mut self, rows: &Particulate) -> (u64, u32) {
        let n = burst_at(rows, self.frame, &mut self.high);
        (burst_base(self.frame), n)
    }
}

/// The numbers a step reads from `rows`, each row reduced once for the kernel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Physics {
    /// How many times a frame the particles move.
    pub steps: u32,
    /// The field's middle, and how far it reaches.
    pub field: [f32; 4],
    pub attractor: f32,
    pub vortex: f32,
    /// 0 x, 1 y, 2 z.
    pub axis: u32,
    pub sphere: f32,
    /// The [`Contact`] code, and how much speed is kept off and along a surface.
    pub contact: u32,
    pub keep: f32,
    pub slip: f32,
    /// Sparks a death or a hit throws, or each second of flight.
    pub sparks: f32,
    pub spark_speed: f32,
    pub spark_life: f32,
    /// 0 death, 1 bounce, 2 flight.
    pub sparks_from: u32,
    /// A spark's size, as a share of its parent's.
    pub spark_size: f32,
    /// How far off another particle still counts as a neighbour, nought for no
    /// flock, and how strongly a particle steers each of the three ways.
    pub flock_radius: f32,
    pub apart: f32,
    pub along: f32,
    pub together: f32,
}

/// `rows` reduced to what a step reads. Nought for anything not a number.
#[must_use]
pub fn physics(rows: &Particulate) -> Physics {
    let real = |v: f32| if v.is_finite() { v } else { 0.0 };
    let share = |v: f32| (real(v) / 100.0).clamp(0.0, 1.0);
    let (apart, along, together) = (
        share(rows.separation),
        share(rows.alignment),
        share(rows.cohesion),
    );
    let radius = real(rows.flock_radius).max(0.0);
    Physics {
        steps: rows.sub_steps.clamp(1, 8) as u32,
        field: [
            real(rows.field_x),
            real(rows.field_y),
            real(rows.field_z),
            real(rows.field_radius).max(1.0),
        ],
        attractor: real(rows.attractor),
        vortex: real(rows.vortex),
        axis: rows.vortex_axis.min(2),
        sphere: real(rows.sphere).max(0.0),
        contact: Contact::from_code(rows.on_contact) as u32,
        keep: (real(rows.bounciness) / 100.0).clamp(0.0, 1.0),
        slip: 1.0 - (real(rows.friction) / 100.0).clamp(0.0, 1.0),
        sparks: real(rows.sparks).clamp(0.0, 1000.0),
        spark_speed: real(rows.spark_speed).max(0.0),
        spark_life: real(rows.spark_life).max(0.0),
        sparks_from: rows.sparks_from.min(2),
        spark_size: (real(rows.spark_size) / 100.0).max(0.0),
        flock_radius: if apart + along + together > 0.0 {
            radius
        } else {
            0.0
        },
        apart,
        along,
        together,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn comp() -> Composition {
        use crate::time::{Duration, FrameRate, Rational};
        Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: uuid::Uuid::now_v7(),
            name: "Sim".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(30, 1).unwrap()),
            background: crate::model::LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// Births never repeat or get lost, however a frame is cut into steps.
    #[test]
    fn a_frame_of_births_is_cut_into_steps_without_a_gap() {
        for (rate, steps) in [(5.0, 2u32), (1000.0, 3), (60_000.0, 8), (1.0, 1)] {
            let dt = 1.0 / 30.0;
            let mut l = Ledger::default();
            let (mut made, mut owed) = (0u64, 0.0f64);
            for _ in 0..60 {
                let s = l.steady(rate, dt, u32::MAX);
                let mut next = 0;
                for k in 0..steps {
                    let h = dt / f64::from(steps);
                    let (a, b) = s.between(f64::from(k) * h, f64::from(k + 1) * h);
                    assert_eq!(a, next, "no gap between steps");
                    next = b;
                    made += u64::from(b - a);
                }
                assert_eq!(next, s.n, "every birth of the frame is made");
                owed += f64::from(rate) * dt;
                l.advance(rate, &s);
                assert_eq!(l.births, made);
            }
            assert!((owed - made as f64).abs() < 1.001);
        }
    }

    /// Over the cap, only the newest births of a frame are made.
    #[test]
    fn only_the_newest_births_are_made_past_the_cap() {
        let l = Ledger::default();
        let s = l.steady(30_000.0, 1.0 / 30.0, 100);
        assert_eq!((s.n, s.from), (1000, 900));
        assert_eq!(s.between(0.0, 1.0 / 30.0), (900, 1000));
        assert!(s.off(1) > s.off(0));
    }

    /// Rows reach the kernel reduced, and a row that is not a number is nought.
    #[test]
    fn rows_are_reduced_for_a_step() {
        use crate::fx::params::Params;
        let mut e = crate::fx::builtins::instantiate("particulate").unwrap();
        let mut set = |id: &str, v: EffectValue| {
            e.params.iter_mut().find(|p| p.id == id).unwrap().value = v;
        };
        let fixed = |v: f64| EffectValue::Float(crate::anim::Property::fixed(v));
        set("simulate", EffectValue::Bool(true));
        set("sub_steps", fixed(40.0));
        set("attractor", fixed(50.0));
        set("bounciness", fixed(250.0));
        set("spark_size", fixed(25.0));
        let bag = crate::fx::tests::resolve_bag(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &crate::fx::markers::MarkerContext::NONE,
        );
        let mut rows =
            <Particulate as crate::fx::registry::EffectMetadata>::read(Params::new(&bag));
        let p = physics(&rows);
        assert_eq!(p.steps, 8);
        assert_eq!(p.attractor, 50.0);
        assert_eq!(p.keep, 1.0);
        assert_eq!(p.spark_size, 0.25);
        rows.attractor = f32::NAN;
        assert_eq!(physics(&rows).attractor, 0.0);
    }

    /// The floor is where its own z is nought, and up is its z.
    #[test]
    fn the_floor_is_a_surface_seen_from_the_layer() {
        use super::super::collider::Collider;
        use crate::model::{Document, LayerKind};
        use crate::time::Rational;
        let layer = crate::edits::base_layer(
            "Particles".into(),
            LayerKind::Null,
            Rational::new(1, 1).unwrap(),
            Default::default(),
        );
        let mut comp = comp();
        comp.layers = vec![layer.clone()];
        let id = comp.id;
        let mut doc = Document::new();
        doc.items
            .push(crate::model::ProjectItem::Composition(comp.clone()));
        let context = crate::expression::ExpressionContext {
            document: std::sync::Arc::new(doc.clone()),
            comp: Some(id),
            layer: Some(layer.id),
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        };
        let floor = Collider::ground(&comp, &layer, 0.0, &context, 700.0);
        assert!(floor.is_surface() && floor.edges().is_empty());
        let back = floor.from_layer();
        let above = back.point((0.0, 600.0, 0.0));
        let below = back.point((0.0, 800.0, 0.0));
        assert!(above.2 > 0.0 && below.2 < 0.0, "{above:?} {below:?}");
        let there = floor.to_layer().point(above);
        assert!((there.1 - 600.0).abs() < 1e-6);
        // Every 3D layer feeds the name of a walk that hits them all.
        let mut e = crate::fx::builtins::instantiate("particulate").unwrap();
        for id in ["simulate", "collide_all"] {
            e.params.iter_mut().find(|p| p.id == id).unwrap().value = EffectValue::Bool(true);
        }
        let named = super::super::definition(&doc, &comp, &layer, &e, None);
        let mut moved = comp.clone();
        let mut other = layer.clone();
        other.id = uuid::Uuid::now_v7();
        other.switches.three_d = true;
        moved.layers.push(other);
        assert_ne!(
            named,
            super::super::definition(&doc, &moved, &layer, &e, None)
        );
    }

    /// Only Simulate turns the card on, so nothing saved before it looks new.
    #[test]
    fn only_simulate_turns_the_card_on() {
        let mut e = crate::fx::builtins::instantiate("particulate").unwrap();
        let set = |e: &mut EffectInstance, id: &str, v: EffectValue| {
            e.params.iter_mut().find(|p| p.id == id).unwrap().value = v;
        };
        assert!(!wanted(&e));
        set(&mut e, "ground", EffectValue::Bool(true));
        set(&mut e, "collide_all", EffectValue::Bool(true));
        set(&mut e, "in_scene", EffectValue::Bool(true));
        set(&mut e, "sparks_from", EffectValue::Choice(1));
        assert!(!wanted(&e));
        set(&mut e, "simulate", EffectValue::Bool(true));
        assert!(wanted(&e));
    }

    /// A row in the list is a row, and the walk's name follows what it reads and
    /// not how it looks.
    #[test]
    fn looks_keep_the_walk_and_physics_rename_it() {
        let mut e = crate::fx::builtins::instantiate("particulate").unwrap();
        for id in LOOK_ROWS {
            assert!(e.params.iter().any(|p| p.id == *id), "no row {id}");
        }
        let simulate = e.params.iter_mut().find(|p| p.id == "simulate").unwrap();
        simulate.value = EffectValue::Bool(true);
        let layer = crate::edits::base_layer(
            "Particles".into(),
            LayerKind::Null,
            crate::time::Rational::new(1, 1).unwrap(),
            Default::default(),
        );
        let mut comp = comp();
        comp.layers = vec![layer.clone()];
        let doc = Document::new();
        let name = |comp: &Composition, e: &EffectInstance, l: &Layer| {
            super::super::definition(&doc, comp, l, e, None)
        };
        let walk = name(&comp, &e, &layer);
        let set = |e: &EffectInstance, id: &str, v: f64| {
            let mut e = e.clone();
            e.params.iter_mut().find(|p| p.id == id).unwrap().value =
                EffectValue::Float(crate::anim::Property::fixed(v));
            e
        };
        assert_eq!(walk, name(&comp, &set(&e, "size", 9.0), &layer));
        assert_eq!(walk, name(&comp, &set(&e, "mix", 50.0), &layer));
        assert_ne!(walk, name(&comp, &set(&e, "gravity", 9.0), &layer));
        // A taller comp moves the floor, the model's fit and the Edge frame.
        let mut taller = comp.clone();
        taller.height += 1;
        assert_ne!(walk, name(&taller, &e, &layer));
        // Moving a layer with nothing to hit keeps it, as particles are in its pixels.
        let mut moved = layer.clone();
        moved.transform.position_x = crate::anim::Property::fixed(400.0);
        assert_eq!(walk, name(&comp, &e, &moved));
    }
}
