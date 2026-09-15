//! `lumit-ml`: the model runtime, the model packs, and the tensor arithmetic
//! that feeds them (the whole mechanism is pinned in docs/impl/addons.md, and
//! this crate is §3, §4 and §5's engine half).
//!
//! # In plain terms
//!
//! Some things a compositor wants are best done by a trained model: guessing
//! how far away every pixel is, cutting a person out of their background,
//! tracing the thing you clicked on, and inventing the frame that sits between
//! two real ones. Those models are large files and they need a runtime library
//! to execute, so neither ships with Lumit. They are addons: a folder the user
//! installs from Settings, holding a manifest and the files it names.
//!
//! This crate is everything the engine does with that folder. It reads the
//! manifests, says what is installed and what is broken, unpacks a new install
//! without ever writing into a live one, opens the runtime library once, opens
//! one model at a time on top of it, and packs pictures into the shape a model
//! takes. It decides nothing about when a model runs: that is the effect's,
//! the Retime's or the Roto brush's business, and each of them names its model
//! on a control the user can see.
//!
//! Nothing generative lives here and nothing ever will: a model in Lumit makes
//! a plane of depth, a plane of coverage or an in-between frame, never a
//! picture from a description.
//!
//! # Thread role and contract
//!
//! Three halves with three different contracts, each stated at the top of its
//! own module. [`manifest`] and [`tensor`] are pure, so they run and are
//! tested on every machine. [`store`] does disk work on whichever thread asked
//! and allows one install at a time across the process. [`runtime`] opens the
//! library once behind a `OnceLock`, and [`session`] hands back a model owned
//! by one thread and never shared, because a run is an FFI call that may hold
//! the GPU (14-ENGINEERING-RULES §1.3). [`synthesis`] is one such model, wired
//! to the one task that runs inside a render rather than as a baked analysis,
//! [`depth`] and [`matte`] are two of the baked ones, and [`segment`] is the
//! one a tap on the picture asks a question of.

pub mod depth;
pub mod error;
pub mod manifest;
pub mod matte;
pub mod runtime;
pub mod segment;
pub mod session;
pub mod store;
pub mod synthesis;
pub mod tensor;

pub use depth::Depth;
pub use error::MlError;
pub use manifest::{Arch, Kind, Manifest, Task};
pub use matte::Matte;
pub use runtime::{no_runtime, RuntimeStatus, REQUIRE_RUNTIME_ENV};
pub use segment::Segment;
pub use session::{Prefer, Session};
pub use store::Installed;
pub use synthesis::Synthesis;

/// Where the tests find a real runtime and real packs, on the one machine
/// that has them.
#[cfg(test)]
mod test_support {
    use std::path::PathBuf;

    /// One test at a time wherever the addons folder is pointed somewhere of
    /// its own: the override and the install slot are process-wide, so two
    /// overlapping tests would read each other's addons.
    pub fn serially() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIAL.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// The folder holding the shared library, from `LUMIT_ML_RUNTIME_DIR`.
    pub fn runtime_dir() -> Option<PathBuf> {
        named("LUMIT_ML_RUNTIME_DIR")
    }

    /// The folder holding the `.onnx` files, from `LUMIT_ML_PACKS_DIR`.
    pub fn packs_dir() -> Option<PathBuf> {
        named("LUMIT_ML_PACKS_DIR")
    }

    /// A folder an environment variable names, if it is one.
    fn named(variable: &str) -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os(variable)?);
        dir.is_dir().then_some(dir)
    }
}
