//! Particulate worked out step by step on the graphics card.
//!
//! The rows, births and what is hit are read here on the host, a frame at a
//! time, and the particles themselves only ever live on the card
//! (`lumit-render`'s `sim` and `fx_particulate_sim.wgsl`). A frame is the walk
//! from the layer's in point, as in [`super::stepped`]: kept states only make
//! it faster.

use super::stepped::{above, burst_at};
use super::{burst_base, Contact};
use crate::fx::effects::particulate::Particulate;
use crate::model::{Composition, Document, EffectInstance, EffectValue, LayerKind};

fn on(inst: &EffectInstance, id: &str) -> bool {
    matches!(inst.param(id), Some(EffectValue::Bool(true)))
}

/// Whether Particulate `inst` is simulated on the card. Simulate says so, and
/// so does anything only the card can do: Ground, All 3D layers, sparks from
/// anything but a death, and a collider or sparks in the scene.
#[must_use]
pub fn wanted(comp: &Composition, inst: &EffectInstance) -> bool {
    inst.effect.match_name == "particulate"
        && (on(inst, "simulate")
            || on(inst, "ground")
            || on(inst, "collide_all")
            || !matches!(
                inst.param("sparks_from"),
                None | Some(EffectValue::Choice(0))
            )
            || (on(inst, "in_scene")
                && (inst.layer_ref("collide_with").is_some() || above(comp, inst, "sparks"))))
}

/// Feeds everything a simulation reads beyond its own layer and Collide with:
/// every 3D layer for All 3D layers, and a Layer emitter's model. They are
/// read at every frame from the in point.
pub(super) fn feed_colliders(
    doc: &Document,
    comp: &Composition,
    inst: &EffectInstance,
    h: &mut blake3::Hasher,
) {
    let mut feed = |l: &crate::model::Layer| {
        let _ = serde_json::to_writer(&mut *h, l);
        if let LayerKind::Model { item, .. } = l.kind {
            let _ = serde_json::to_writer(&mut *h, &doc.item(item));
        }
    };
    if on(inst, "collide_all") {
        comp.layers
            .iter()
            .filter(|l| l.switches.three_d)
            .for_each(&mut feed);
    }
    let named = |id: &str| {
        let id = inst.layer_ref(id)?;
        comp.layers.iter().find(|l| l.id == id)
    };
    if let Some(l) = named("emitter_layer") {
        feed(l);
    }
    if let Some(l) = named("collide_with") {
        feed(l);
    }
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
}

/// `rows` reduced to what a step reads. Nought for anything not a number.
#[must_use]
pub fn physics(rows: &Particulate) -> Physics {
    let real = |v: f32| if v.is_finite() { v } else { 0.0 };
    let simulating = rows.simulate;
    let field = |v: f32| if simulating { real(v) } else { 0.0 };
    Physics {
        steps: if simulating {
            rows.sub_steps.clamp(1, 8) as u32
        } else {
            2
        },
        field: [
            real(rows.field_x),
            real(rows.field_y),
            real(rows.field_z),
            real(rows.field_radius).max(1.0),
        ],
        attractor: field(rows.attractor),
        vortex: field(rows.vortex),
        axis: rows.vortex_axis.min(2),
        sphere: field(rows.sphere).max(0.0),
        contact: Contact::from_code(rows.on_contact) as u32,
        keep: (real(rows.bounciness) / 100.0).clamp(0.0, 1.0),
        slip: 1.0 - (real(rows.friction) / 100.0).clamp(0.0, 1.0),
        sparks: real(rows.sparks).clamp(0.0, 1000.0),
        spark_speed: real(rows.spark_speed).max(0.0),
        spark_life: real(rows.spark_life).max(0.0),
        sparks_from: rows.sparks_from.min(2),
        spark_size: (real(rows.spark_size) / 100.0).max(0.0),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

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

    /// Anything only the card can do turns the card on, and a saved project
    /// with none of it stays on the old walk.
    #[test]
    fn what_turns_the_card_on() {
        use crate::time::{Duration, FrameRate, Rational};
        let comp = Composition {
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
        };
        let mut e = crate::fx::builtins::instantiate("particulate").unwrap();
        assert!(!wanted(&comp, &e));
        let set = |e: &mut EffectInstance, id: &str, v: EffectValue| {
            e.params.iter_mut().find(|p| p.id == id).unwrap().value = v;
        };
        set(&mut e, "ground", EffectValue::Bool(true));
        assert!(wanted(&comp, &e));
        set(&mut e, "ground", EffectValue::Bool(false));
        set(&mut e, "sparks_from", EffectValue::Choice(1));
        assert!(wanted(&comp, &e));
        set(&mut e, "sparks_from", EffectValue::Choice(0));
        assert!(!wanted(&comp, &e));
        set(&mut e, "simulate", EffectValue::Bool(true));
        assert!(wanted(&comp, &e));
    }
}
