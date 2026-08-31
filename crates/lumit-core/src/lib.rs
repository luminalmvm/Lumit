//! lumit-core: rational time, the document model, operations, and the
//! snapshot store. Engine root — depends on nothing above it
//! (docs/05-ARCHITECTURE.md dependency rules).

// The `#[derive(Effect)]` macro writes `::lumit_core::…` paths, which have to
// resolve inside this crate as well as outside it (docs/impl/effect-registry.md).
extern crate self as lumit_core;

pub mod anim;
pub mod expression;
pub mod fx;
/// The layer driver graph: the additive wiring beside the effect stack (K-471).
pub mod graph;
/// Layer groups: the Timeline's organisational fold over a run of layers (K-700).
pub mod group;
pub mod lighting;
pub mod lut;
pub mod markers;
pub mod mask;
pub mod model;
/// Occlusion culling: the full-frame opaque layer that hides what is under it (K-423).
pub mod occlusion;
pub mod ops;
pub mod paint;
pub mod pixels;
pub mod preset;
/// Puppet: the mesh over a layer's own silhouette and the as-rigid-as-possible
/// deformer over it (K-704).
pub mod puppet;
pub mod retime;
pub mod roto;
pub mod sequence;
pub mod shape;
pub mod store;
/// Text animators: a Text layer's words moved a letter at a time (K-609).
pub mod text;
pub mod time;
/// The solve link: a Camera layer driven by a tracked layer (K-417).
pub mod track;

pub use model::Document;
pub use ops::{Op, OpError};
pub use store::DocumentStore;
pub use time::{CompTime, Duration, FrameRate, Rational, TimeError};
