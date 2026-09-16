//! `lfx-sys` - the LFX C ABI's declarations, and nothing else.
//!
//! # In plain terms
//!
//! `include/lfx.h` is the agreement between Lumit and an effect somebody else
//! wrote. This crate is that agreement in `#[repr(C)]` Rust: every constant,
//! every enumeration, every struct and every function-pointer table, with no
//! behaviour anywhere. A plugin written in Rust either uses this crate
//! directly, which is the C header with Rust's spelling, or uses the safe
//! `lfx` wrapper beside it, which is this crate with the unsafe parts written
//! once.
//!
//! **The declarations are a verbatim copy.** They live in `src/abi.rs`, which
//! is byte for byte the module Lumit itself reads a bundle's memory with
//! (`crates/lumit-lfx-abi/src/lib.rs` in the editor's own repository), and
//! Lumit's CI fails if the two files ever differ. So a vendor compiling
//! against this crate is compiling against the host's own declarations rather
//! than against somebody's reading of them - which is the same promise
//! `include/lfx.h` carries, made on the other side of the language boundary.
//!
//! That is also why the copy is a module rather than this file: a verbatim
//! copy cannot carry a crate doc of its own, and a crate that introduced
//! itself with the host's own words would be describing a repository a vendor
//! has not got.
//!
//! # Licence
//!
//! MIT, deliberately more permissive than Lumit's own GPLv3, so a proprietary
//! vendor may adopt it without licence anxiety.

mod abi;

pub use abi::*;
