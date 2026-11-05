//! A Text or Shape layer's Geometry options: how its outline becomes a solid
//! the mesh pass draws, with Front, Bevel, Side and Back materials.
//!
//! The rows are declared the way an effect's are and kept on the layer as a
//! Model layer's are, only where they differ from their defaults. A layer with
//! no depth and no bevel keeps none, and draws on the plane it always did.

pub mod mesh;

use crate::anim::Property;
use crate::fx::registry::{EffectDef, EffectMetadata};
use crate::fx::schema::EffectSchema;
use crate::mask::BezierPath;
use crate::model::{EffectInstance, EffectValue, Layer, LayerKind, ShadowMode};
use crate::ops::Op;
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// What [`GeometryOptions`] is known by.
pub const MATCH_NAME: &str = "geometry_options";

/// The names of a solid's faces, in the order the layer keeps their rows,
/// after the options' own.
pub const FACES: [&str; 4] = ["Front", "Bevel", "Side", "Back"];

/// How far a curve can stray from the straight steps it is walked in, in
/// layer pixels.
const FLATNESS: f64 = 0.08;
/// The most straight steps one curve is walked in.
const MOST_STEPS: u32 = 48;

/// The rows themselves.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "geometry_options",
    label = "Geometry options",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct GeometryOptions {
    #[slider(min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub extrusion_depth: f32,
    #[choice(label = "Bevel style", options = ["Angular", "Convex", "Concave"], default = 0)]
    pub bevel_style: u32,
    #[slider(min = 0.0, max = 60.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub bevel_depth: f32,
    #[counter(min = 1, max = 16, default = 4, hard_min = 1, hard_max = 32)]
    pub bevel_segments: i32,
    /// A hole's bevel, as a share of the bevel depth.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub hole_bevel_depth: f32,
}

/// The rows' behaviour: none of their own, the mesh pass reads them.
pub struct GeometryOptionsDef;

impl EffectDef for GeometryOptionsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<GeometryOptions as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}

/// The shape a bevel's slope takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BevelStyle {
    /// A flat slope.
    Angular,
    /// Rounded over.
    Convex,
    /// Scooped out.
    Concave,
}

/// What the rows come to at a moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub depth: f64,
    pub bevel: f64,
    pub style: BevelStyle,
    pub segments: u32,
    /// A hole's bevel as a share of the outline's, nought to one.
    pub hole: f64,
}

impl Profile {
    /// The rows `rows` at layer time `lt`, each number read by `read`.
    #[must_use]
    pub fn of(rows: &EffectInstance, read: &dyn Fn(&Property) -> f64) -> Self {
        let float = |id: &str, or: f64| match rows.param(id) {
            Some(EffectValue::Float(p)) => read(p),
            _ => or,
        };
        let style = match rows.param("bevel_style") {
            Some(EffectValue::Choice(c)) => *c,
            _ => 0,
        };
        Self {
            depth: float("extrusion_depth", 0.0).max(0.0),
            bevel: float("bevel_depth", 0.0).max(0.0),
            style: match style {
                1 => BevelStyle::Convex,
                2 => BevelStyle::Concave,
                _ => BevelStyle::Angular,
            },
            segments: float("bevel_segments", 4.0).round().clamp(1.0, 32.0) as u32,
            hole: float("hole_bevel_depth", 100.0).clamp(0.0, 100.0) / 100.0,
        }
    }
}

/// Whether `layer` is a kind that can have Geometry options: a Text or Shape
/// layer, or a 3D shape, whose options are its size.
#[must_use]
pub fn takes_geometry(layer: &Layer) -> bool {
    matches!(
        layer.kind,
        LayerKind::Text { .. } | LayerKind::Shape { .. } | LayerKind::Mesh { .. }
    )
}

/// The rows `layer` keeps: its options, then its faces', each `None` for rows
/// left as they were.
#[must_use]
pub fn kept(layer: &Layer) -> &[Option<EffectInstance>] {
    match takes_geometry(layer) {
        true => &layer.transform.material.geometry,
        false => &[],
    }
}

/// Whether `rows` give a solid at any moment. A row nought all the way through
/// does nothing, and a keyed or driven one may not be nought for long, so
/// animating up from nought never jumps from one way of drawing to the other.
#[must_use]
pub fn is_solid(rows: Option<&EffectInstance>) -> bool {
    rows.is_some_and(|rows| {
        rows.enabled
            && ["extrusion_depth", "bevel_depth"]
                .iter()
                .any(|id| match rows.param(id) {
                    Some(EffectValue::Float(p)) => !crate::paint::is_static_zero(p),
                    _ => false,
                })
    })
}

/// Whether the mesh pass draws `layer`: a Model layer always, and a Text or
/// Shape layer once it's in 3D with depth or a bevel to show.
#[must_use]
pub fn draws_mesh(layer: &Layer) -> bool {
    match layer.kind {
        LayerKind::Model { .. } | LayerKind::Mesh { .. } => true,
        _ => layer.switches.three_d && is_extruded(layer),
    }
}

/// Whether `layer`'s Geometry options give it any depth. A 3D shape always
/// has its own.
#[must_use]
pub fn is_extruded(layer: &Layer) -> bool {
    matches!(layer.kind, LayerKind::Mesh { .. })
        || is_solid(kept(layer).first().and_then(Option::as_ref))
}

/// The op that gives `layer` in comp `comp` the kept rows `geometry`. Depth
/// arriving by any route puts a 2D layer in 3D in the same step and has it
/// cast shadows, so its switch, gizmo and views agree with what it now is.
#[must_use]
pub fn set_geometry(comp: Uuid, layer: &Layer, geometry: Vec<Option<EffectInstance>>) -> Op {
    let arrives = !is_extruded(layer) && is_solid(geometry.first().and_then(Option::as_ref));
    let set = Op::SetLayerGeometry {
        comp,
        layer: layer.id,
        geometry,
    };
    if !arrives {
        return set;
    }
    let mut ops = vec![set];
    if !layer.switches.three_d {
        ops.push(Op::SetLayerThreeD {
            comp,
            layer: layer.id,
            three_d: true,
        });
    }
    let material = &layer.transform.material;
    if material.casts_shadows == ShadowMode::Off {
        ops.push(Op::SetMaterialSettings {
            comp,
            layer: layer.id,
            settings: crate::model::MaterialSettings {
                casts_shadows: ShadowMode::On,
                ..material.settings()
            },
        });
    }
    Op::Batch { ops }
}

/// `path` as straight steps, each curve in as few as keep it within
/// [`FLATNESS`], a straight edge in one. The same path gives the same points.
#[must_use]
pub fn flattened(path: &BezierPath) -> Vec<[f64; 2]> {
    let n = path.vertices.len();
    let mut out = Vec::new();
    let Some(first) = path.vertices.first() else {
        return out;
    };
    out.push([first.pos.0, first.pos.1]);
    let last = if path.closed { n } else { n.saturating_sub(1) };
    for i in 0..last {
        let (a, b) = (&path.vertices[i], &path.vertices[(i + 1) % n]);
        let p0 = a.pos;
        let p1 = (p0.0 + a.tan_out.0, p0.1 + a.tan_out.1);
        let p3 = b.pos;
        let p2 = (p3.0 + b.tan_in.0, p3.1 + b.tan_in.1);
        // How far the curve bends sets how many steps it needs. A straight
        // edge's handles sit on it, whichever way they are spaced.
        let d1 = (p0.0 - 2.0 * p1.0 + p2.0, p0.1 - 2.0 * p1.1 + p2.1);
        let d2 = (p1.0 - 2.0 * p2.0 + p3.0, p1.1 - 2.0 * p2.1 + p3.1);
        let bend = d1.0.hypot(d1.1).max(d2.0.hypot(d2.1));
        let chord = (p3.0 - p0.0, p3.1 - p0.1);
        let off = |p: (f64, f64)| (chord.0 * (p.1 - p0.1) - chord.1 * (p.0 - p0.0)).abs();
        let straight = off(p1).max(off(p2)) <= 1e-6 * chord.0.hypot(chord.1);
        let steps = (0.75 * bend / FLATNESS).sqrt().ceil();
        let steps = if straight {
            1
        } else if steps.is_finite() {
            (steps as u32).clamp(1, MOST_STEPS)
        } else {
            1
        };
        for k in 1..=steps {
            let t = f64::from(k) / f64::from(steps);
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            out.push([
                w[0] * p0.0 + w[1] * p1.0 + w[2] * p2.0 + w[3] * p3.0,
                w[0] * p0.1 + w[1] * p1.1 + w[2] * p2.1 + w[3] * p3.1,
            ]);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::LayerKind;

    fn logo() -> Layer {
        crate::edits::base_layer(
            "Logo".into(),
            LayerKind::Shape {
                contents: Vec::new(),
            },
            crate::time::Rational::new(1, 1).unwrap(),
            Default::default(),
        )
    }

    fn with_depth(depth: f64) -> EffectInstance {
        let mut rows = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        let p = rows
            .params
            .iter_mut()
            .find(|p| p.id == "extrusion_depth")
            .unwrap();
        p.value = EffectValue::Float(Property::fixed(depth));
        rows
    }

    /// Depth arriving puts a 2D layer in 3D as one undo step, and undo puts
    /// back both. Nought does nothing at all, and a saved layer with no rows
    /// reads and writes as it always did.
    #[test]
    fn depth_arrives_in_one_step() {
        let mut layer = logo();
        let comp = Uuid::now_v7();
        let flat = set_geometry(comp, &layer, vec![Some(with_depth(0.0))]);
        assert!(matches!(flat, Op::SetLayerGeometry { .. }));

        let Op::Batch { ops } = set_geometry(comp, &layer, vec![Some(with_depth(40.0))]) else {
            panic!("a 2D layer was left flat");
        };
        assert!(matches!(ops[1], Op::SetLayerThreeD { three_d: true, .. }));
        assert!(matches!(ops[2], Op::SetMaterialSettings { .. }));

        assert!(!draws_mesh(&layer));
        layer.transform.material.geometry = vec![Some(with_depth(40.0))];
        assert!(!draws_mesh(&layer), "its 3D switch is off");
        layer.switches.three_d = true;
        assert!(draws_mesh(&layer));

        // Already solid, a change of depth takes nothing else with it.
        assert!(matches!(
            set_geometry(comp, &layer, vec![Some(with_depth(60.0))]),
            Op::SetLayerGeometry { .. }
        ));

        let plain = serde_json::to_value(logo().transform).unwrap();
        assert!(plain.get("geometry").is_none());
    }

    #[test]
    fn a_straight_edge_is_one_step_and_a_curve_many() {
        use crate::mask::Vertex;
        let line = BezierPath {
            vertices: vec![
                Vertex {
                    pos: (0.0, 0.0),
                    tan_in: (0.0, 0.0),
                    tan_out: (0.0, 0.0),
                },
                Vertex {
                    pos: (100.0, 0.0),
                    tan_in: (0.0, 0.0),
                    tan_out: (0.0, 0.0),
                },
            ],
            closed: false,
        };
        assert_eq!(flattened(&line).len(), 2);
        let mut curve = line;
        curve.vertices[0].tan_out = (40.0, 60.0);
        curve.vertices[1].tan_in = (-40.0, 60.0);
        assert!(flattened(&curve).len() > 8);
    }
}
