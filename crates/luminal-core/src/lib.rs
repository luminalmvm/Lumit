//! luminal-core: rational time, the document model, operations, and the
//! snapshot store. Engine root — depends on nothing above it
//! (docs/05-ARCHITECTURE.md dependency rules).

pub mod anim;
pub mod markers;
pub mod mask;
pub mod model;
pub mod ops;
pub mod retime;
pub mod sequence;
pub mod store;
pub mod time;

pub use model::Document;
pub use ops::{Op, OpError};
pub use store::DocumentStore;
pub use time::{CompTime, Duration, FrameRate, Rational, TimeError};
