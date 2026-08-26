//! The built-in effect registry and CPU reference implementations
//! (docs/08-EFFECTS.md §1). The WGSL production path lives in `lumit-gpu`
//! (docs/05 crate table); this module is the engine-pure side: what each
//! effect *is* (schema, parameters, traits), how an instance is born with
//! tasteful defaults, how a stack resolves to plain evaluated numbers at a
//! frame, and the CPU maths that serve as the test oracle (§1.6) and the
//! degradation ladder's fallback rung (K-019).
//!
//! In plain terms: this file is the effects catalogue. Each entry declares
//! its parameters (names, defaults, slider ranges) and its cost/behaviour
//! traits; dropping one on a layer copies the declared defaults into the
//! project. At render time the animatable parameters are evaluated at the
//! frame's time into a flat list of numbers — the same list the GPU kernels
//! and these CPU functions both consume, which is what makes "the GPU must
//! agree with the CPU" a testable promise.

mod builtins;
/// Registration: the list of effects this build has (§2.6).
mod catalogue;
/// Spectral colour tables for the Lens flare (docs/impl/lens-flare.md §5).
pub mod cie;
/// The drivers and the driver-graph evaluation (K-471): one module per driver,
/// plus the demand-driven walk that works out what every wire carries.
pub mod drivers;
/// One module per migrated built-in: its declaration and its behaviour (§2.1).
pub mod effects;
/// The in-house FFT / fractional Fourier transform the Lens flare bakes use.
pub mod fft;
/// The Lens flare optics core, bake, and CPU reference (docs/08 §3.27).
pub mod labels;
pub mod lens_flare;
/// The bundled lens prescription library (K-261): 1303 .lens files as
/// embedded text, generated from the FlareSim / PhotonsToPhotos collection.
pub mod lens_library;
mod markers;
mod maths;
/// The shared procedural noise core (docs/08 §3.37): seeded 3-D value and
/// Perlin noise and the fractal sum over them, reused by the displacement
/// family.
pub mod noise;
/// The resolved key/value parameter form a frame renders from
/// (docs/impl/effect-registry.md §2.3).
mod params;
/// The points stream and its closed forms (K-474, K-495): one frame's
/// particles, worked out from their birth indices and nothing else.
pub mod points;
/// The catalogue as JSON, for the manual's per-effect parameter tables.
pub mod reference;
/// What an effect is, as a value rather than a variant (§2.4).
mod registry;
mod resolved;
mod schema;
mod temporal;

/// CPU reference implementations (docs/08 §1.6): identical semantics to the
/// WGSL kernels, plain and readable — the oracle the GPU must agree with.
pub mod cpu;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub use builtins::*;
pub use catalogue::*;
pub use drivers::{resolve_drivers, resolve_drivers_projected, temporal_window, ResolvedDrivers};
pub use markers::*;
pub use maths::*;
pub use params::*;
pub use registry::*;
pub use resolved::*;
pub use schema::*;
pub use temporal::*;
