//! lumit-core: rational time, the document model, operations, and the
//! snapshot store. Engine root — depends on nothing above it
//! (docs/05-ARCHITECTURE.md dependency rules).

// The `#[derive(Effect)]` macro writes `::lumit_core::…` paths, which have to
// resolve inside this crate as well as outside it (docs/impl/effect-registry.md).
extern crate self as lumit_core;

pub mod anim;
pub mod camera;
pub mod captions;
pub mod comp_graph;
pub mod edits;
pub mod environment;
pub mod expression;
pub mod extrude;
pub mod fx;
pub mod grade;
pub mod grade_solve;
pub mod graph;
pub mod group;
pub mod haze;
pub mod interpretation;
pub mod largest_picture;
pub mod lighting;
pub mod linalg;
pub mod lut;
pub mod markers;
pub mod mask;
pub mod mesh_shape;
pub mod mixer;
pub mod model;
pub mod model_layer;
pub mod model_material;
pub mod model_part;
pub mod multicam;
pub mod occlusion;
pub mod ops;
pub mod orient;
pub mod paint;
pub mod pixels;
pub mod planes;
pub mod precomp_flow;
pub mod preset;
pub mod preset_parts;
pub mod puppet;
pub mod retime;
pub mod roto;
pub mod routes;
pub mod sequence;
pub mod shape;
pub mod shared;
pub mod shown_size;
pub mod stabilise;
pub mod store;
pub mod template;
pub mod text;
pub mod time;
pub mod track;
pub mod transcript;
pub mod view_camera;
pub mod wire_tie;
