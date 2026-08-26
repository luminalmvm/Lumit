//! `lumit-ofx` — Lumit's OpenFX host (K-061; docs/12 §2 says what it must do,
//! docs/impl/ofx-host.md says how).
//!
//! # In plain terms
//!
//! An OFX plugin is somebody else's compiled code — Twixtor, Sapphire, one of
//! the eighty free ones — shipped as a shared library that Lumit loads while
//! it is running. It cannot be inspected, it was written against other hosts,
//! and it will be handed pictures and asked for pictures back. This crate is
//! the side of that conversation Lumit speaks.
//!
//! What this package builds is the ground floor: opening a bundle, telling
//! the plugin honestly what sort of host it has landed in, and answering the
//! three simplest of its questions — read and write properties, allocate
//! memory, say something to the user. Effects, parameters, clips and rendering
//! come next, and everything they add stands on the two ideas here.
//!
//! **Honesty.** The host describes itself in a table of properties, and every
//! answer in it is one the rest of Lumit keeps. Claiming to support tiled
//! rendering when the pipeline is full-frame is the classic way to break a
//! plugin that believed you.
//!
//! **Handles.** Everything the plugin refers to, it refers to by an opaque
//! number the host invents. Plugins forge them, keep them too long, and pass
//! the wrong sort. None of ours is a real pointer; each is checked three ways
//! on every use, and the answer to a bad one is an error code the plugin is
//! required to expect, not a crash. See [`handles`].
//!
//! # Thread role and contract
//!
//! Plugin-facing, and therefore not free of shared state: the C API gives a
//! suite function no context but its arguments, so the host's property sets
//! live behind one process-wide mutex ([`host::state`]). It is taken and
//! released inside a single suite call and **never held across a call into a
//! plugin**, which would deadlock the moment that plugin read a property
//! (docs/14 §7). Nothing here blocks, sleeps, or touches the GPU.
//!
//! No Rust panic may cross back into C, so every entry point a plugin can
//! reach runs its body inside a catch (see [`suites`]).

pub mod bundle;
pub mod ffi;
pub mod handles;
pub mod host;
pub mod props;
pub mod quirks;
pub mod status;
pub mod suites;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub use bundle::{Bundle, BundleError};
pub use handles::{Handle, HandleKind, HandleRegistry};
pub use props::{Element, PropValue, PropertySet};
pub use quirks::{Quirks, QuirksTable};
pub use status::{OfxStatus, Status};
