//! `lumit-colour` — OCIO colour management, implemented rather than linked
//! (the *how* is [docs/impl/ocio.md](../../../docs/impl/ocio.md)).
//!
//! ## In plain terms
//!
//! Professional colour work runs on a standard called **OpenColorIO**. A config
//! is a folder: one text file that names colour spaces — "this footage is
//! ACEScct", "this monitor shows sRGB" — plus the look-up-table files that text
//! points at. Studios publish configs, so a Lumit project can agree with a Nuke
//! or Resolve project about what its pixels *mean*.
//!
//! The official implementation is a C++ library that deliberately computes
//! differently on the processor and on the graphics card. Lumit's foundational
//! promise is that the preview **is** the export, bit for bit, and a
//! library with two answers cannot keep that promise — so Lumit implements the
//! format itself, the same way it hosts OpenFX itself and parses `.cube` files
//! itself. Everything a config describes is resolved once, on the processor,
//! into a small **baked table**, and that one table is what both the Viewer and
//! the export sample. One implementation, one answer.
//!
//! The honest risk of writing it ourselves is getting a transform subtly wrong,
//! and the answer to that risk is not care but proof: nothing is claimed until
//! its golden fixtures pass, and a config that uses anything outside the
//! implemented set is **refused by name** — "this config needs
//! `FixedFunctionTransform`, which Lumit does not support yet" — never
//! approximated. A wrong picture that looks plausible is the one failure this
//! crate refuses to ship.
//!
//! ## The shape of the crate
//!
//! - [`op`] — the arithmetic steps a config is made of, and the flat [`Chain`]
//!   that resolving anything produces.
//! - [`sample`] — the two samplers: tetrahedral for cubes, linear for curves.
//!   Binding maths: WP3's WGSL must match it byte for byte.
//! - [`bake`] — the chain baked into the one artefact the pipeline executes.
//! - [`spi`], [`clf`] — the look-up table file formats `lumit-core::lut` does
//!   not already own.
//! - [`config`] — the `config.ocio` grammar.
//! - [`resolve`] — a loaded config: roles, displays, views, and the bridge from
//!   the config's reference space to Lumit's fixed working space.
//! - [`builtin`] — `BuiltinTransform`, in its two tiers.
//! - [`error`] — the refusal taxonomy. Read this first to know what v1 is.
//!
//! Thread role: pure and deterministic. Parsing reads files it is handed paths
//! to; nothing else touches the world. Every value here is immutable once built,
//! so a loaded config and a baked artefact are `Send + Sync` and may be shared
//! across worker threads (docs/14 §1.2).

pub mod bake;
pub mod builtin;
pub mod clf;
pub mod config;
pub mod error;
pub mod file;
pub mod matrix;
pub mod op;
pub mod resolve;
pub mod sample;
pub mod spi;

pub use bake::{bake, Artefact, Shaper, Stage, VendoredArtefact};
pub use config::Config;
pub use error::{ColourError, Result};
pub use op::{Chain, Direction, Op};
pub use resolve::{Bridge, LoadedConfig};
pub use sample::{Cube, Curve};
