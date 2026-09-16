//! The entry table: what the loader finds, and what it reaches from there.
//!
//! # In plain terms
//!
//! A bundle exports one symbol. Through it the host asks how many effects are
//! inside, reads a descriptor for each without creating anything, and then
//! makes one instance per in-flight frame. This module is that machinery,
//! written once: an author lists their types in [`crate::bundle`] and never
//! sees a function pointer.
//!
//! Three of the header's rules live here rather than in the author's code.
//!
//! **A descriptor stays valid and unchanged until `deinit`.** So the table is
//! built once, on the first ask, and nothing ever edits it - a host may
//! already have read the pointer it holds.
//!
//! **Every struct carries its own `size_of`**, so a host built against a later
//! header reads the fields it knows and ignores the tail, and one built
//! against an earlier header is refused rather than read past its end.
//!
//! **A panic must not cross the boundary.** Unwinding into C is undefined, so
//! `describe` and `process` are wrapped: a panic becomes a refused frame and a
//! badged layer, which is the worst thing it should ever be.
//!
//! # Thread role and contract
//!
//! `init`, `describe`, `create` and `destroy` run on the host's control
//! thread; `process` runs on any worker thread and on different instances
//! concurrently. One instance is never re-entered, which is what makes the
//! `&mut self` in [`crate::Effect`] sound.

use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::describe::Describe;
use crate::request::Request;
use crate::{sys, Effect, Spec, Status};

/// One effect, behind a trait object, so a bundle may hold several types.
trait Instance {
    fn describe(&mut self, sink: &mut Describe<'_>) -> bool;
    fn process(&mut self, call: &Request<'_>) -> Status;
}

impl<E: Effect> Instance for E {
    fn describe(&mut self, sink: &mut Describe<'_>) -> bool {
        Effect::describe(self, sink)
    }

    fn process(&mut self, call: &Request<'_>) -> Status {
        Effect::process(self, call)
    }
}

/// One row of a bundle's table: what the effect is, and how to make one.
///
/// Built by [`registration`] and handed to [`Table::of`]; there is nothing to
/// fill in by hand.
pub struct Registration {
    spec: Spec,
    make: fn() -> Box<dyn Instance>,
}

/// The registration for one [`Effect`] type.
#[must_use]
pub fn registration<E: Effect>() -> Registration {
    Registration {
        spec: E::SPEC,
        make: || Box::new(E::default()) as Box<dyn Instance>,
    }
}

/// A bundle's descriptors, built once and never changed after.
///
/// It owns everything the descriptors point at - the trait blocks and the
/// required-extension arrays - because the header says a descriptor stays
/// valid until `deinit`, and a pointer into something that has been dropped is
/// the one fault a host cannot defend itself against.
pub struct Table {
    /// A **boxed slice** rather than a vector, and that is the point: the
    /// descriptors below point at these blocks, the header says a descriptor
    /// stays valid until `deinit`, and a slice that cannot grow cannot move
    /// what it holds. Never read again - it is here to be pointed at.
    #[allow(dead_code)]
    traits: Box<[sys::LfxTraits]>,
    /// One array of `const char *` per effect, pointed at by its descriptor,
    /// each boxed for the reason the trait blocks are.
    #[allow(dead_code)]
    required: Vec<Box<[*const c_char]>>,
    /// What the host reads.
    descriptors: Vec<sys::LfxDescriptor>,
    /// The ids, in the same order, for `create` to match against.
    ids: Vec<&'static CStr>,
    /// How to make one, in the same order.
    makers: Vec<fn() -> Box<dyn Instance>>,
}

// SAFETY: every pointer in the table is either into this library's own
// read-only statics or into memory the table itself owns and never frees or
// mutates, so sharing it across threads shares immutable bytes that outlive
// every reader.
unsafe impl Sync for Table {}
// SAFETY: as above.
unsafe impl Send for Table {}

impl Table {
    /// Build the table from the bundle's registrations.
    #[must_use]
    pub fn of(registrations: Vec<Registration>) -> Self {
        // The two owned arrays are filled in completely before a descriptor
        // points at either of them: a vector that grew afterwards would move
        // its elements out from under a pointer the host may already hold.
        let traits: Box<[sys::LfxTraits]> = registrations
            .iter()
            .map(|row| row.spec.traits.lowered())
            .collect();
        let required: Vec<Box<[*const c_char]>> = registrations
            .iter()
            .map(|row| {
                row.spec
                    .required_extensions
                    .iter()
                    .map(|id| id.as_ptr())
                    .collect()
            })
            .collect();
        let descriptors = registrations
            .iter()
            .enumerate()
            .map(|(index, row)| sys::LfxDescriptor {
                struct_size: size_of::<sys::LfxDescriptor>() as u32,
                id: row.spec.id.as_ptr(),
                name: row.spec.name.as_ptr(),
                vendor: row.spec.vendor.as_ptr(),
                major: row.spec.version.0,
                minor: row.spec.version.1,
                patch: row.spec.version.2,
                categories: row.spec.categories.as_ptr().cast::<sys::LfxCategory>(),
                category_count: row.spec.categories.len() as u32,
                traits: traits
                    .get(index)
                    .map_or(std::ptr::null(), std::ptr::from_ref),
                required_extensions: required
                    .get(index)
                    .map_or(std::ptr::null(), |list| list.as_ptr()),
                required_extension_count: required.get(index).map_or(0, |list| list.len() as u32),
            })
            .collect();
        Self {
            traits,
            required,
            descriptors,
            ids: registrations.iter().map(|row| row.spec.id).collect(),
            makers: registrations.iter().map(|row| row.make).collect(),
        }
    }

    /// How many effects this bundle holds.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.descriptors.len() as u32
    }

    /// The descriptor at `index`, or null past the end.
    #[must_use]
    pub fn descriptor(&self, index: u32) -> *const sys::LfxDescriptor {
        self.descriptors
            .get(index as usize)
            .map_or(std::ptr::null(), std::ptr::from_ref)
    }

    /// One instance of the effect named `id`, or null.
    ///
    /// # Safety
    ///
    /// `id` must be null or a NUL-terminated string valid for the call, and
    /// `host` null or a host table valid until `deinit` - which is the host's
    /// own contract at `lfx_entry.create`.
    #[must_use]
    pub unsafe fn create(
        &self,
        host: *const sys::LfxHost,
        id: *const c_char,
    ) -> *mut sys::LfxPlugin {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's contract: a NUL-terminated string valid for the
        // call.
        let asked = unsafe { CStr::from_ptr(id) };
        let Some(index) = self.ids.iter().position(|known| *known == asked) else {
            return std::ptr::null_mut();
        };
        let Some(make) = self.makers.get(index).copied() else {
            return std::ptr::null_mut();
        };
        // An effect whose own constructor panics is an effect that could not
        // be made, which is a null here rather than an unwind through C.
        let Ok(instance) = catch_unwind(AssertUnwindSafe(make)) else {
            return std::ptr::null_mut();
        };
        let held = Box::new(Held {
            table: sys::LfxPlugin {
                struct_size: size_of::<sys::LfxPlugin>() as u32,
                plugin_data: std::ptr::null_mut(),
                init: Some(plugin_init),
                destroy: Some(plugin_destroy),
                describe: Some(plugin_describe),
                process: Some(plugin_process),
                get_extension: Some(plugin_extension),
            },
            instance,
            host,
        });
        let raw = Box::into_raw(held);
        // SAFETY: `raw` is the box just leaked, so it is live and unaliased.
        unsafe {
            (*raw).table.plugin_data = raw.cast::<c_void>();
            &raw mut (*raw).table
        }
    }
}

/// The C table in front of one Rust effect.
///
/// The `LfxPlugin` is first so that the pointer the host holds is this
/// allocation's - but `plugin_data` is what everything here reads, because one
/// of those two facts is a promise and the other is a coincidence of layout.
#[repr(C)]
struct Held {
    table: sys::LfxPlugin,
    instance: Box<dyn Instance>,
    /// The host table handed to `create`, kept because the header says it may
    /// be: it is the only pointer in the ABI with that lifetime.
    #[allow(dead_code)]
    host: *const sys::LfxHost,
}

/// What `plugin_data` points at, or `None` for a table this library did not
/// make.
///
/// # Safety
///
/// `plugin` must be null or a table this library's `create` minted, valid for
/// the call.
unsafe fn held<'a>(plugin: *mut sys::LfxPlugin) -> Option<&'a mut Held> {
    if plugin.is_null() {
        return None;
    }
    // SAFETY: the caller's contract: a table valid for the call.
    let data = unsafe { (*plugin).plugin_data };
    if data.is_null() {
        return None;
    }
    // SAFETY: `plugin_data` is the allocation `create` leaked, and the host
    // never re-enters one instance, so this reference is unaliased for the
    // length of the call.
    Some(unsafe { &mut *data.cast::<Held>() })
}

/// Prepare this instance. Non-zero accepts it.
///
/// # Safety
///
/// As [`held`].
unsafe extern "C" fn plugin_init(plugin: *mut sys::LfxPlugin) -> u32 {
    // SAFETY: the caller's contract, passed straight on.
    u32::from(unsafe { held(plugin) }.is_some())
}

/// # Safety
///
/// As [`held`], and never called while a `process` is running.
unsafe extern "C" fn plugin_destroy(plugin: *mut sys::LfxPlugin) {
    if plugin.is_null() {
        return;
    }
    // SAFETY: the caller's contract: a table valid for the call.
    let data = unsafe { (*plugin).plugin_data };
    if data.is_null() {
        return;
    }
    // SAFETY: the allocation `create` leaked, handed back exactly once - the
    // header says `destroy` is called once and never while a frame is in
    // flight.
    drop(unsafe { Box::from_raw(data.cast::<Held>()) });
}

/// # Safety
///
/// As [`held`]; `sink` must be null or a sink valid for the call.
unsafe extern "C" fn plugin_describe(
    plugin: *mut sys::LfxPlugin,
    sink: *mut sys::LfxDescribeSink,
) -> u32 {
    // SAFETY: the caller's contract, passed straight on.
    let Some(self_) = (unsafe { held(plugin) }) else {
        return 0;
    };
    if sink.is_null() {
        return 0;
    }
    // SAFETY: the caller's contract: a sink valid for the call.
    let sink = unsafe { &mut *sink };
    let mut declaring = Describe::new(sink);
    if !declaring.complete() {
        return 0;
    }
    // A panic here is an effect that did not describe, which the host reports
    // by name - never an unwind through C.
    catch_unwind(AssertUnwindSafe(|| self_.instance.describe(&mut declaring))).map_or(0, u32::from)
}

/// # Safety
///
/// As [`held`]; `call` must be null or a request valid for the call.
unsafe extern "C" fn plugin_process(
    plugin: *mut sys::LfxPlugin,
    call: *const sys::LfxProcess,
) -> i32 {
    // SAFETY: the caller's contract, passed straight on.
    let Some(self_) = (unsafe { held(plugin) }) else {
        return Status::Failed as i32;
    };
    if call.is_null() {
        return Status::Failed as i32;
    }
    // SAFETY: the caller's contract: a request valid for the call.
    let request = Request::new(unsafe { &*call });
    catch_unwind(AssertUnwindSafe(|| self_.instance.process(&request))).unwrap_or(Status::Failed)
        as i32
}

/// This library offers no extension table, which is a null and never a status.
///
/// # Safety
///
/// As [`held`].
unsafe extern "C" fn plugin_extension(
    _plugin: *mut sys::LfxPlugin,
    _id: *const c_char,
    _version: u32,
) -> *const c_void {
    std::ptr::null()
}
