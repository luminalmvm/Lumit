//! A shared library that is not an LFX bundle, built so the host's refusal has
//! something real to refuse.
//!
//! # In plain terms
//!
//! A `.lfx` is an ordinary shared library, so "this file is not a plugin" comes
//! in two shapes and the host answers them differently: a file the loader will
//! not load at all, and a library that loads perfectly well and simply does not
//! export `lfx_entry_point`. The first is a text file in a temporary folder;
//! the second has to be a real library, which is this one.
//!
//! It exports one thing, under a name nothing looks for.
// `#[no_mangle]` is an unsafe attribute, and this crate denies `unsafe_code`
// everywhere but `src/local.rs`. An export that exists to be looked for and not
// found is the narrowest possible use of it.
#![allow(unsafe_code)]

/// A symbol, so the library is not empty. Nothing calls it.
#[no_mangle]
pub extern "C" fn lumit_not_an_lfx_bundle() -> u32 {
    0
}
