//! A shared library whose `lfx_entry_point` lies about itself, built so the
//! host's four refusals at the front door have something real to refuse.
//!
//! # In plain terms
//!
//! Four of the answers `LocalHost::open` gives are about the entry table itself
//! rather than about anything inside it: a `struct_size` shorter than this
//! header's, an `abi_version` this host does not speak, an `init` hook that is
//! not there at all, and an `init` that declines to load. No honest bundle can
//! be any of them, and `lumit-lfx-testplug` is an honest bundle - every other
//! test in the suite opens it, so a personality that made it short at the front
//! door would take the suite with it. So the lie is told from here, by a
//! library of its own, and the twelve personalities stay what they are.
//!
//! It is the shape `lumit-lfx-testplug`'s own misbehaving probes take, one
//! struct further out: the table is whole and what is wrong with it is the
//! bytes a host reads **before** it trusts the rest. A host that formed a
//! `&LfxEntry` over the symbol without reading the prefix first would find
//! every function pointer where it expects one and never notice, which is why
//! the fault has to be told rather than caught by inspection
//! (docs/impl/lfx.md §2.1, §10). What this library cannot tell is the fault
//! itself: its static is the whole of an `lfx_entry` and only says otherwise,
//! so the sanitiser sees nothing either way. `an_lfx_entry_cut_short` is that
//! half - a static that really is two words and a hook - and the two are read
//! by the same test.
//!
//! `lfx_entry_point` is mutable here, which a vendor's is not - the header
//! declares it `const` and every honest bundle spells it that way. Nothing in
//! the loader or in the host requires the symbol to sit in read-only memory,
//! and one library that can be made to lie three different ways is cheaper than
//! three libraries that each lie once. Two of those three come out of the
//! mutable static - the prefix and the version - and the third does not:
//! declining to load is `INIT_ANSWERS`, an ordinary atomic, which would work
//! as well with a `const` entry.
//!
//! **The fixture depends on this test binary exporting no dynamic symbols.**
//! `lumit-lfx-testplug` is a dev-dependency of `lumit-lfx` and is linked into
//! the test executable as an rlib, so that executable defines an
//! `lfx_entry_point` of its own; the writes below reach this library's copy
//! through the GOT, which the global scope may interpose. Today it does not,
//! because the test binary's `.dynsym` is empty. A later `-C
//! link-args=-rdynamic`, a `-C prefer-dynamic` build or a dylib test harness
//! would silently redirect these writes into the testplug's entry table,
//! leaving the liar honest and the twelve personalities corrupt - a failure
//! that would read as a regression in the host's own prefix check. Whoever adds
//! such a flag is breaking this.

// `#[no_mangle]`, a mutable static and a raw entry table: the same allow
// `not_an_lfx_bundle` takes, for the same reason. This crate denies
// `unsafe_code` everywhere but `src/local.rs`, and a fixture whose whole job is
// to be the wrong shape at a C boundary cannot be written without it.
#![allow(unsafe_code)]

use std::ffi::c_char;
use std::sync::atomic::{AtomicU32, Ordering};

use lumit_lfx_abi::{LfxDescriptor, LfxEntry, LfxHost, LfxPlugin, LFX_ABI_VERSION};

/// How many times the host has called `init` since the last reset.
static INITS: AtomicU32 = AtomicU32::new(0);
/// How many times it has called `deinit`.
static DEINITS: AtomicU32 = AtomicU32::new(0);
/// How many times it has asked what this bundle holds.
static COUNTS: AtomicU32 = AtomicU32::new(0);
/// What `init` answers. Nought declines; anything else loads, because the
/// answer is a `uint32_t` in which non-zero is true and not a C `bool`
/// (docs/impl/lfx.md §2.1).
static INIT_ANSWERS: AtomicU32 = AtomicU32::new(1);

/// The one exported object, under the name the header gives the loader -
/// mutable, so [`LumitLfxLiarShape`] can make it the wrong shape.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut lfx_entry_point: LfxEntry = LfxEntry {
    struct_size: size_of::<LfxEntry>() as u32,
    abi_version: LFX_ABI_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    count: Some(entry_count),
    descriptor: Some(entry_descriptor),
    create: Some(entry_create),
};

/// Tell the entry what to say about itself the next time a host opens it.
///
/// `struct_size` and `abi_version` are taken as given, so a caller asks for the
/// honest numbers by passing them; `init_answers` is what `init` returns, where
/// nought declines to load; and `init_present` nought unsets the hook
/// altogether, which is a bundle that exports an entry point and gives the host
/// nothing to start it with.
#[no_mangle]
pub extern "C" fn LumitLfxLiarShape(
    struct_size: u32,
    abi_version: u32,
    init_answers: u32,
    init_present: u32,
) {
    // The fields are reached as raw places rather than through a `&mut
    // LfxEntry`, so no reference to a mutable static is ever formed: the host
    // may be reading this table from another thread's point of view, and the
    // suite's own lock rather than the type system is what keeps the two apart.
    //
    // SAFETY: the static is this library's own, and a `u32` field of a
    // `#[repr(C)]` struct is a plain place to write.
    unsafe {
        (&raw mut lfx_entry_point.struct_size).write(struct_size);
        (&raw mut lfx_entry_point.abi_version).write(abi_version);
        (&raw mut lfx_entry_point.init).write(match init_present {
            0 => None,
            _ => Some(entry_init),
        });
    }
    INIT_ANSWERS.store(init_answers, Ordering::SeqCst);
}

/// Put the entry back the way an honest one is, and forget the counts.
#[no_mangle]
pub extern "C" fn LumitLfxLiarReset() {
    LumitLfxLiarShape(size_of::<LfxEntry>() as u32, LFX_ABI_VERSION, 1, 1);
    INITS.store(0, Ordering::SeqCst);
    DEINITS.store(0, Ordering::SeqCst);
    COUNTS.store(0, Ordering::SeqCst);
}

/// How many times the host has called `init` since the last reset. Nought is
/// the answer a refusal read from the size prefix leaves behind.
#[no_mangle]
pub extern "C" fn LumitLfxLiarInits() -> u32 {
    INITS.load(Ordering::SeqCst)
}

/// How many times the host has called `deinit`. The header pairs it with an
/// `init` that succeeded, so a bundle that declined to load is never told to
/// stop.
#[no_mangle]
pub extern "C" fn LumitLfxLiarDeinits() -> u32 {
    DEINITS.load(Ordering::SeqCst)
}

/// How many times the host has asked what this bundle holds. Nought is the
/// answer to a bundle that declined to load: nothing else in it may be called.
#[no_mangle]
pub extern "C" fn LumitLfxLiarCounts() -> u32 {
    COUNTS.load(Ordering::SeqCst)
}

/// Start the bundle, or decline to.
///
/// # Safety
///
/// `bundle_path` must be null or a NUL-terminated string valid for the call. It
/// is not read: what this bundle is for is the answer, not the argument.
unsafe extern "C" fn entry_init(_bundle_path: *const c_char) -> u32 {
    INITS.fetch_add(1, Ordering::SeqCst);
    INIT_ANSWERS.load(Ordering::SeqCst)
}

/// Stop the bundle. Once, last, and only after an `init` that succeeded.
unsafe extern "C" fn entry_deinit() {
    DEINITS.fetch_add(1, Ordering::SeqCst);
}

/// How many effects this bundle holds: none, and the count is what proves the
/// host asked at all.
unsafe extern "C" fn entry_count() -> u32 {
    COUNTS.fetch_add(1, Ordering::SeqCst);
    0
}

/// The descriptor at `index`. There are none, so this is always null.
unsafe extern "C" fn entry_descriptor(_index: u32) -> *const LfxDescriptor {
    std::ptr::null()
}

/// One instance of the effect named `id`. There are none, so this is always
/// null.
///
/// # Safety
///
/// As the header declares it; neither argument is read.
unsafe extern "C" fn entry_create(_host: *const LfxHost, _id: *const c_char) -> *mut LfxPlugin {
    std::ptr::null_mut()
}
