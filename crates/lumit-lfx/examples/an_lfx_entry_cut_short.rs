//! A shared library whose `lfx_entry_point` really is shorter than this
//! header's entry table, rather than merely saying so.
//!
//! # In plain terms
//!
//! This is the bundle a vendor ships when they built against an earlier,
//! smaller `lfx_entry`: two words and one hook, honestly prefixed with its own
//! length. Nothing here misbehaves - the fault is the shape, and the shape is
//! what a growing struct costs.
//!
//! It exists because `an_lfx_entry_that_lies` cannot be it. That library's
//! static is the whole of an `lfx_entry` and only claims to be short, so a host
//! that formed a `&LfxEntry` over it before reading the prefix would be reading
//! its own bundle's bytes and no tool would object. Here the object stops after
//! sixteen bytes or so, so the same host reads past the end of a static and
//! Miri or a sanitiser says which line it was. The rule `LocalHost::open` is
//! held to - **the size prefix is read before the struct it prefixes** - is
//! about the allocation, and this is the allocation
//! (docs/impl/lfx.md §2.1, §10).
//!
//! `init` is exported and counted so the refusal can be asked what it ran. The
//! honest answer is nothing: a function pointer past the end of the prefix is
//! not a function pointer the host may read at all, let alone call.

// `#[no_mangle]` and a raw entry table, as `an_lfx_entry_that_lies` takes them.
// This crate denies `unsafe_code` everywhere but `src/local.rs`, and a fixture
// whose whole job is to be the wrong shape at a C boundary cannot be written
// without it.
#![allow(unsafe_code)]

use std::ffi::c_char;
use std::sync::atomic::{AtomicU32, Ordering};

use lumit_lfx_abi::LFX_ABI_VERSION;

/// An `lfx_entry` as an earlier version of the header had it: the two words
/// every struct in this ABI opens with, and the one hook a host cannot start a
/// bundle without.
///
/// `#[repr(C)]`, so the first word is at offset nought and is this object's own
/// length, which is the one field a host may read before it trusts the rest.
#[repr(C)]
pub struct ShortEntry {
    struct_size: u32,
    abi_version: u32,
    init: Option<unsafe extern "C" fn(*const c_char) -> u32>,
}

/// The one exported object, under the name the header gives the loader, and
/// genuinely this long.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static lfx_entry_point: ShortEntry = ShortEntry {
    struct_size: size_of::<ShortEntry>() as u32,
    abi_version: LFX_ABI_VERSION,
    init: Some(entry_init),
};

/// How many times a host has called `init`. Nought is the only right answer.
static INITS: AtomicU32 = AtomicU32::new(0);

/// How long this bundle's entry table really is, so the test asserting the
/// refusal's number need not re-spell a layout.
#[no_mangle]
pub extern "C" fn LumitLfxCutShortBytes() -> u32 {
    size_of::<ShortEntry>() as u32
}

/// How many times a host has called `init` on this bundle.
#[no_mangle]
pub extern "C" fn LumitLfxCutShortInits() -> u32 {
    INITS.load(Ordering::SeqCst)
}

/// Start the bundle. A host reading this table's prefix never reaches it.
///
/// # Safety
///
/// `bundle_path` must be null or a NUL-terminated string valid for the call. It
/// is not read: what this bundle is for is the shape, not the argument.
unsafe extern "C" fn entry_init(_bundle_path: *const c_char) -> u32 {
    INITS.fetch_add(1, Ordering::SeqCst);
    1
}
