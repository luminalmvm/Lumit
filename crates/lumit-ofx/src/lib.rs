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
//! The ground floor is opening a bundle, telling the plugin honestly what sort
//! of host it has landed in, and answering the three simplest of its
//! questions — read and write properties, allocate memory, say something to the
//! user. On top of that sits **describe** ([`describe`]): the conversation
//! where a plugin says what it is called, what shapes of effect it can be, and
//! what controls it has, and where Lumit writes that answer down as the same
//! declaration a built-in effect carries ([`schema`]).
//!
//! On top of *that* sits the frame. An [`instance`] is one live copy of the
//! effect, with a value in every control; an [`image`] is the picture crossing
//! the boundary, widened to float and possibly upside-down on purpose; and
//! [`render`] is the fixed order of actions that turns the two into a frame.
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
pub mod def;
pub mod describe;
pub mod discover;
pub mod ffi;
pub mod handles;
pub mod host;
pub mod image;
pub mod instance;
pub mod ipc;
pub mod props;
pub mod quirks;
pub mod render;
pub mod schema;
pub mod status;
pub mod suites;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub use bundle::{Bundle, BundleError};
pub use def::{BrokerHost, LocalHost, OfxEffectDef, PluginHost, Rendering};
pub use describe::{describe, describe_bundle, Context, PluginDescriptor, Rejection, ScanReport};
pub use discover::{
    scan, DiscoveredPlugin, Hosting, ScanOptions, ScanOutcome, DISABLED_REASON, SCAN_FRAME,
};
pub use handles::{Handle, HandleKind, HandleRegistry};
pub use image::{Frame16, Image, RectI, RowOrder};
pub use instance::{Instance, ParamSnapshot, ThreadSafety};
pub use ipc::broker::{Broker, BrokerConfig, BrokerError, BrokerRender};
pub use props::{Element, PropValue, PropertySet};
pub use quirks::{Quirks, QuirksTable};
pub use render::{render, RenderError, RenderRequest, Rendered};
pub use schema::schema_of;
pub use status::{OfxStatus, Status};
