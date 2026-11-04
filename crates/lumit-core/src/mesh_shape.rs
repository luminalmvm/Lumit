//! 3D shapes: a cube, sphere, cylinder, cone, torus or plane that Lumit makes
//! itself, sized by rows that key like any other.
//!
//! A shape keeps its rows where a solid Text or Shape layer keeps its Geometry
//! options ([`crate::extrude`]): its own first, then its one Surface material,
//! so the panels, keys, expressions and presets all come with them.

pub mod build;

use crate::fx::registry::{EffectDef, EffectMetadata};
use crate::fx::schema::EffectSchema;
use crate::model::{Composition, EffectInstance, EffectValue, Layer, LayerKind, ShadowMode};
use lumit_fx_macros::Effect;
use serde::{Deserialize, Serialize};

/// Which shape a [`LayerKind::Mesh`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshShape {
    #[default]
    Cube,
    Sphere,
    Cylinder,
    Cone,
    Torus,
    Plane,
}

impl MeshShape {
    /// What its Geometry options are known by.
    #[must_use]
    pub fn options(self) -> &'static str {
        match self {
            Self::Cube => "cube_geometry",
            Self::Sphere => "sphere_geometry",
            Self::Cylinder => "cylinder_geometry",
            Self::Cone => "cone_geometry",
            Self::Torus => "torus_geometry",
            Self::Plane => "plane_geometry",
        }
    }
}

/// A menu entry: a name, the shape, and the rows that differ from its own.
pub type Entry = (&'static str, MeshShape, &'static [(&'static str, f64)]);

/// What a menu offers, in its order. A pyramid is a cone with four sides and
/// a point, its base 200 pixels across.
pub const MENU: [Entry; 7] = [
    ("Cube", MeshShape::Cube, &[]),
    ("Sphere", MeshShape::Sphere, &[]),
    ("Cylinder", MeshShape::Cylinder, &[]),
    ("Cone", MeshShape::Cone, &[]),
    ("Torus", MeshShape::Torus, &[]),
    ("Plane", MeshShape::Plane, &[]),
    (
        "Pyramid",
        MeshShape::Cone,
        &[("segments", 4.0), ("bottom_radius", 141.421_356)],
    ),
];

/// A fresh layer of menu entry `entry` ([`MENU`]) in `comp`: always 3D, at the
/// comp's centre, as long as the comp, and casting shadows, as a Model layer
/// starts.
#[must_use]
pub fn mesh_layer(comp: &Composition, entry: usize) -> Option<Layer> {
    let (name, shape, rows) = MENU.get(entry)?;
    let mut layer = crate::edits::base_layer(
        (*name).to_owned(),
        LayerKind::Mesh { shape: *shape },
        comp.duration.0,
        crate::edits::centred_transform(0.0, 0.0, comp.width, comp.height),
    );
    layer.switches.three_d = true;
    layer.transform.material.casts_shadows = ShadowMode::On;
    if !rows.is_empty() {
        let mut options = crate::fx::builtins::instantiate(shape.options())?;
        for (id, v) in *rows {
            if let Some(p) = options.params.iter_mut().find(|p| p.id == *id) {
                p.value = EffectValue::Float(crate::anim::Property::fixed(*v));
            }
        }
        layer.transform.material.geometry = vec![Some(options)];
    }
    Some(layer)
}

/// The rows a layer of `shape` reads, kept or else its defaults.
#[must_use]
pub fn options_of(shape: MeshShape, kept: Option<&EffectInstance>) -> Option<EffectInstance> {
    kept.cloned()
        .or_else(|| crate::fx::builtins::instantiate(shape.options()))
}

macro_rules! def {
    ($def:ident, $rows:ident) => {
        pub struct $def;

        impl EffectDef for $def {
            fn schema(&self) -> &'static EffectSchema {
                &<$rows as EffectMetadata>::SCHEMA
            }

            fn is_image_op(&self) -> bool {
                false
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(match_name = "cube_geometry", label = "Geometry options", version = 1, category = Utility, cost = Trivial, roi = Exact, matte = false)]
pub struct CubeGeometry {
    #[slider(min = 0.0, max = 1000.0, default = 200.0, hard_min = 0.0, unit = Px)]
    pub width: f32,
    #[slider(min = 0.0, max = 1000.0, default = 200.0, hard_min = 0.0, unit = Px)]
    pub height: f32,
    #[slider(min = 0.0, max = 1000.0, default = 200.0, hard_min = 0.0, unit = Px)]
    pub depth: f32,
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub bevel: f32,
    #[counter(min = 1, max = 16, default = 4, hard_min = 1, hard_max = 32)]
    pub bevel_segments: i32,
}
def!(CubeGeometryDef, CubeGeometry);

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(match_name = "sphere_geometry", label = "Geometry options", version = 1, category = Utility, cost = Trivial, roi = Exact, matte = false)]
pub struct SphereGeometry {
    #[slider(min = 0.0, max = 500.0, default = 100.0, hard_min = 0.0, unit = Px)]
    pub radius: f32,
    #[counter(min = 3, max = 128, default = 48, hard_min = 3, hard_max = 512)]
    pub segments: i32,
    #[slider(min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub slice_start: f32,
    #[slider(min = 0.0, max = 360.0, default = 360.0, unit = Degrees)]
    pub slice_end: f32,
}
def!(SphereGeometryDef, SphereGeometry);

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(match_name = "cylinder_geometry", label = "Geometry options", version = 1, category = Utility, cost = Trivial, roi = Exact, matte = false)]
pub struct CylinderGeometry {
    #[slider(min = 0.0, max = 500.0, default = 100.0, hard_min = 0.0, unit = Px)]
    pub radius: f32,
    #[slider(min = 0.0, max = 1000.0, default = 200.0, hard_min = 0.0, unit = Px)]
    pub height: f32,
    #[counter(min = 3, max = 128, default = 48, hard_min = 3, hard_max = 512)]
    pub segments: i32,
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub bevel: f32,
    #[counter(min = 1, max = 16, default = 4, hard_min = 1, hard_max = 32)]
    pub bevel_segments: i32,
    #[slider(min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub slice_start: f32,
    #[slider(min = 0.0, max = 360.0, default = 360.0, unit = Degrees)]
    pub slice_end: f32,
}
def!(CylinderGeometryDef, CylinderGeometry);

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(match_name = "cone_geometry", label = "Geometry options", version = 1, category = Utility, cost = Trivial, roi = Exact, matte = false)]
pub struct ConeGeometry {
    #[slider(min = 0.0, max = 500.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub top_radius: f32,
    #[slider(min = 0.0, max = 500.0, default = 100.0, hard_min = 0.0, unit = Px)]
    pub bottom_radius: f32,
    #[slider(min = 0.0, max = 1000.0, default = 200.0, hard_min = 0.0, unit = Px)]
    pub height: f32,
    #[counter(min = 3, max = 128, default = 48, hard_min = 3, hard_max = 512)]
    pub segments: i32,
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub bevel: f32,
    #[counter(min = 1, max = 16, default = 4, hard_min = 1, hard_max = 32)]
    pub bevel_segments: i32,
    #[slider(min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub slice_start: f32,
    #[slider(min = 0.0, max = 360.0, default = 360.0, unit = Degrees)]
    pub slice_end: f32,
}
def!(ConeGeometryDef, ConeGeometry);

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(match_name = "torus_geometry", label = "Geometry options", version = 1, category = Utility, cost = Trivial, roi = Exact, matte = false)]
pub struct TorusGeometry {
    #[slider(min = 0.0, max = 500.0, default = 100.0, hard_min = 0.0, unit = Px)]
    pub radius: f32,
    #[slider(min = 0.0, max = 200.0, default = 30.0, hard_min = 0.0, unit = Px)]
    pub pipe_radius: f32,
    #[counter(min = 3, max = 128, default = 64, hard_min = 3, hard_max = 512)]
    pub segments: i32,
    #[counter(min = 3, max = 64, default = 24, hard_min = 3, hard_max = 256)]
    pub pipe_segments: i32,
    #[slider(min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub slice_start: f32,
    #[slider(min = 0.0, max = 360.0, default = 360.0, unit = Degrees)]
    pub slice_end: f32,
}
def!(TorusGeometryDef, TorusGeometry);

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(match_name = "plane_geometry", label = "Geometry options", version = 1, category = Utility, cost = Trivial, roi = Exact, matte = false)]
pub struct PlaneGeometry {
    #[slider(min = 0.0, max = 2000.0, default = 400.0, hard_min = 0.0, unit = Px)]
    pub width: f32,
    #[slider(min = 0.0, max = 2000.0, default = 400.0, hard_min = 0.0, unit = Px)]
    pub height: f32,
    #[slider(min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub corner_radius: f32,
    #[counter(min = 1, max = 32, default = 8, hard_min = 1, hard_max = 64)]
    pub corner_segments: i32,
}
def!(PlaneGeometryDef, PlaneGeometry);
