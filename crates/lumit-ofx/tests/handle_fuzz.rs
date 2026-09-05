//! Every suite entry point, handed a handle that is not one
//! (docs/impl/ofx-host.md §5 item 2).
//!
//! # In plain terms
//!
//! A handle is a number the host invents and the plugin hands back
//! ([`lumit_ofx::handles`]). Plugins get this wrong constantly: they keep a
//! handle after the thing it named was destroyed, pass an effect where a
//! parameter was meant, or hand back memory they never got from us. Every one
//! of those must be an error code the plugin is required to expect — never a
//! followed pointer, never a crash.
//!
//! The unit tests seed that with a handful of hand-written bad handles. This is
//! the same idea run properly: a corpus of forged, stale, wrong-kind, freed and
//! **randomly mutated** handles, put through *every* entry point of all six
//! suites, thousands of times. It is deterministic by default — same seed, same
//! calls, same answers — and the seed and the length are environment variables
//! so a long soak is the same test with a bigger number.
//!
//! It is also the target CI runs under AddressSanitizer, which is the half of
//! the promise assertions cannot make: a status code says the host *answered*,
//! and only a sanitiser says it did not read a byte it should not have on the
//! way.
//!
//! | | |
//! |---|---|
//! | `LUMIT_OFX_FUZZ_ITERS` | how many random handles to add to the corpus (default 2000) |
//! | `LUMIT_OFX_FUZZ_SEED` | the seed for those (default fixed, so a run is reproducible) |
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::ffi::{c_char, c_int, c_uint, c_void};

use lumit_ofx::ffi::{OfxRectD, OfxTime};
use lumit_ofx::handles::{Handle, HandleKind};
use lumit_ofx::host::{host_props_handle, state};
use lumit_ofx::props::PropertySet;
use lumit_ofx::status::{OfxStatus, Status};
use lumit_ofx::suites::{image_effect, memory, message, multi_thread, parameter, property};

/// How many random handles to try, unless the environment says otherwise.
const DEFAULT_ITERATIONS: usize = 2_000;

/// The default seed. A fixed number, because a test that fails only on Tuesdays
/// is a test nobody can bisect (docs/14 §8).
const DEFAULT_SEED: u64 = 0x0F0C_5EED_1F0C_5EED;

/// What an entry point is allowed to answer when the handle it was given is
/// not one of ours.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// It must say so: `kOfxStatErrBadHandle`.
    BadHandle,
    /// It never looks at the handle (the memory and message suites take one
    /// only for the host's convenience, and the OFX spec lets it be null), so
    /// any defined status is a legal answer — what is under test is that it
    /// does not crash or invent a number.
    AnyStatus,
    /// `abort` and `multiThreadIsSpawnedThread` answer a flag, not a status.
    Flag,
}

/// One entry point under test.
struct Entry {
    name: &'static str,
    expect: Expect,
    call: fn(*mut c_void) -> OfxStatus,
}

/// A scratch out-parameter of each shape the suites write into. Every call is
/// given a **valid** place to put its answer, so a rejection can only be about
/// the handle.
mod out {
    use super::{c_char, c_int, c_uint, c_void, OfxRectD, OfxTime};

    pub fn handle_slot() -> *mut *mut c_void {
        std::ptr::addr_of_mut!(SLOT_HANDLE)
    }
    pub fn int_slot() -> *mut c_int {
        std::ptr::addr_of_mut!(SLOT_INT)
    }
    pub fn uint_slot() -> *mut c_uint {
        std::ptr::addr_of_mut!(SLOT_UINT)
    }
    pub fn double_slot() -> *mut f64 {
        std::ptr::addr_of_mut!(SLOT_DOUBLE)
    }
    pub fn time_slot() -> *mut OfxTime {
        std::ptr::addr_of_mut!(SLOT_TIME)
    }
    pub fn string_slot() -> *mut *mut c_char {
        std::ptr::addr_of_mut!(SLOT_STRING)
    }
    pub fn rect_slot() -> *mut OfxRectD {
        std::ptr::addr_of_mut!(SLOT_RECT)
    }

    // Statics rather than locals so every entry in the table is a plain `fn`
    // with nothing borrowed. The test is single-threaded (one test in this
    // binary) and every write to them is a write the host was asked to make.
    static mut SLOT_HANDLE: *mut c_void = std::ptr::null_mut();
    static mut SLOT_INT: c_int = 0;
    static mut SLOT_UINT: c_uint = 0;
    static mut SLOT_DOUBLE: f64 = 0.0;
    static mut SLOT_TIME: OfxTime = 0.0;
    static mut SLOT_STRING: *mut c_char = std::ptr::null_mut();
    static mut SLOT_RECT: OfxRectD = OfxRectD {
        x1: 0.0,
        y1: 0.0,
        x2: 0.0,
        y2: 0.0,
    };
}

/// The property name every call is made with. Which name it is does not matter:
/// on a handle that is not one of ours the name is never looked at, and that is
/// the whole assertion.
const KEY: &std::ffi::CStr = c"OfxPropName";

/// Every entry point of every suite, in the order the headers declare them.
///
/// The list is written out rather than derived: a suite is a C struct of
/// function pointers with no reflection to walk, and writing them out is what
/// makes a missing one visible in review. The four image-memory entry points
/// are [`Expect::AnyStatus`] because this host has no image-memory arena at
/// all — there is no registry for their handles to be absent from, and
/// `kOfxStatErrUnsupported` is the true answer to every one of them, valid
/// handle or not.
fn entries() -> Vec<Entry> {
    macro_rules! entry {
        ($name:literal, $expect:expr, $body:expr) => {
            Entry {
                name: $name,
                expect: $expect,
                call: $body,
            }
        };
    }

    vec![
        // ------------------------------------------------ property suite --
        entry!("propSetPointer", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_set_pointer)(h, KEY.as_ptr(), 0, std::ptr::null_mut())
        }),
        entry!("propSetString", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_set_string)(h, KEY.as_ptr(), 0, c"value".as_ptr())
        }),
        entry!("propSetDouble", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_set_double)(h, KEY.as_ptr(), 0, 1.0)
        }),
        entry!("propSetInt", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_set_int)(h, KEY.as_ptr(), 0, 1)
        }),
        entry!("propSetPointerN", Expect::BadHandle, |h| unsafe {
            let values: [*mut c_void; 1] = [std::ptr::null_mut()];
            (property::SUITE.prop_set_pointer_n)(h, KEY.as_ptr(), 1, values.as_ptr())
        }),
        entry!("propSetStringN", Expect::BadHandle, |h| unsafe {
            let values: [*const c_char; 1] = [c"value".as_ptr()];
            (property::SUITE.prop_set_string_n)(h, KEY.as_ptr(), 1, values.as_ptr())
        }),
        entry!("propSetDoubleN", Expect::BadHandle, |h| unsafe {
            let values: [f64; 1] = [1.0];
            (property::SUITE.prop_set_double_n)(h, KEY.as_ptr(), 1, values.as_ptr())
        }),
        entry!("propSetIntN", Expect::BadHandle, |h| unsafe {
            let values: [c_int; 1] = [1];
            (property::SUITE.prop_set_int_n)(h, KEY.as_ptr(), 1, values.as_ptr())
        }),
        entry!("propGetPointer", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_pointer)(h, KEY.as_ptr(), 0, out::handle_slot())
        }),
        entry!("propGetString", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_string)(h, KEY.as_ptr(), 0, out::string_slot())
        }),
        entry!("propGetDouble", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_double)(h, KEY.as_ptr(), 0, out::double_slot())
        }),
        entry!("propGetInt", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_int)(h, KEY.as_ptr(), 0, out::int_slot())
        }),
        entry!("propGetPointerN", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_pointer_n)(h, KEY.as_ptr(), 1, out::handle_slot())
        }),
        entry!("propGetStringN", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_string_n)(h, KEY.as_ptr(), 1, out::string_slot())
        }),
        entry!("propGetDoubleN", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_double_n)(h, KEY.as_ptr(), 1, out::double_slot())
        }),
        entry!("propGetIntN", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_int_n)(h, KEY.as_ptr(), 1, out::int_slot())
        }),
        entry!("propReset", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_reset)(h, KEY.as_ptr())
        }),
        entry!("propGetDimension", Expect::BadHandle, |h| unsafe {
            (property::SUITE.prop_get_dimension)(h, KEY.as_ptr(), out::int_slot())
        }),
        // -------------------------------------------- image effect suite --
        entry!("getPropertySet", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.get_property_set)(h, out::handle_slot())
        }),
        entry!("getParamSet", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.get_param_set)(h, out::handle_slot())
        }),
        entry!("clipDefine", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.clip_define)(h, c"Source".as_ptr(), out::handle_slot())
        }),
        entry!("clipGetHandle", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.clip_get_handle)(
                h,
                c"Source".as_ptr(),
                out::handle_slot(),
                out::handle_slot(),
            )
        }),
        entry!("clipGetPropertySet", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.clip_get_property_set)(h, out::handle_slot())
        }),
        entry!("clipGetImage", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.clip_get_image)(h, 0.0, std::ptr::null(), out::handle_slot())
        }),
        entry!("clipReleaseImage", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.clip_release_image)(h)
        }),
        entry!("clipGetRegionOfDefinition", Expect::BadHandle, |h| unsafe {
            (image_effect::SUITE.clip_get_region_of_definition)(h, 0.0, out::rect_slot())
        }),
        entry!("abort", Expect::Flag, |h| unsafe {
            (image_effect::SUITE.abort)(h)
        }),
        entry!("imageMemoryAlloc", Expect::AnyStatus, |h| unsafe {
            (image_effect::SUITE.image_memory_alloc)(h, 16, out::handle_slot())
        }),
        entry!("imageMemoryFree", Expect::AnyStatus, |h| unsafe {
            (image_effect::SUITE.image_memory_free)(h)
        }),
        entry!("imageMemoryLock", Expect::AnyStatus, |h| unsafe {
            (image_effect::SUITE.image_memory_lock)(h, out::handle_slot())
        }),
        entry!("imageMemoryUnlock", Expect::AnyStatus, |h| unsafe {
            (image_effect::SUITE.image_memory_unlock)(h)
        }),
        // ----------------------------------------------- parameter suite --
        entry!("paramDefine", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_define)(
                h,
                c"OfxParamTypeDouble".as_ptr(),
                c"amount".as_ptr(),
                out::handle_slot(),
            )
        }),
        entry!("paramGetHandle", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_handle)(
                h,
                c"amount".as_ptr(),
                out::handle_slot(),
                out::handle_slot(),
            )
        }),
        entry!("paramSetGetPropertySet", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_set_get_property_set)(h, out::handle_slot())
        }),
        entry!("paramGetPropertySet", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_property_set)(h, out::handle_slot())
        }),
        entry!("paramGetValue", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_value)(h, out::double_slot().cast::<c_void>())
        }),
        entry!("paramGetValueAtTime", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_value_at_time)(h, 0.0, out::double_slot().cast::<c_void>())
        }),
        entry!("paramGetDerivative", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_derivative)(h, 0.0)
        }),
        entry!("paramGetIntegral", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_integral)(h, 0.0, 1.0)
        }),
        entry!("paramSetValue", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_set_value)(h, 0_i32)
        }),
        entry!("paramSetValueAtTime", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_set_value_at_time)(h, 0.0, 0_i32)
        }),
        entry!("paramGetNumKeys", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_num_keys)(h, out::uint_slot())
        }),
        entry!("paramGetKeyTime", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_key_time)(h, 0, out::time_slot())
        }),
        entry!("paramGetKeyIndex", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_get_key_index)(h, 0.0, 0, out::int_slot())
        }),
        entry!("paramDeleteKey", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_delete_key)(h, 0.0)
        }),
        entry!("paramDeleteAllKeys", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_delete_all_keys)(h)
        }),
        entry!("paramCopy", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_copy)(h, h, 0.0, std::ptr::null())
        }),
        entry!("paramEditBegin", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_edit_begin)(h, c"edit".as_ptr())
        }),
        entry!("paramEditEnd", Expect::BadHandle, |h| unsafe {
            (parameter::SUITE.param_edit_end)(h)
        }),
        // -------------------------------------------- multi-thread suite --
        entry!("mutexDestroy", Expect::BadHandle, |h| unsafe {
            (multi_thread::SUITE.mutex_destroy)(h)
        }),
        entry!("mutexLock", Expect::BadHandle, |h| unsafe {
            (multi_thread::SUITE.mutex_lock)(h)
        }),
        entry!("mutexUnLock", Expect::BadHandle, |h| unsafe {
            (multi_thread::SUITE.mutex_un_lock)(h)
        }),
        entry!("mutexTryLock", Expect::BadHandle, |h| unsafe {
            (multi_thread::SUITE.mutex_try_lock)(h)
        }),
        entry!("multiThreadNumCPUs", Expect::AnyStatus, |_h| unsafe {
            (multi_thread::SUITE.multi_thread_num_cpus)(out::uint_slot())
        }),
        entry!("multiThreadIndex", Expect::AnyStatus, |_h| unsafe {
            (multi_thread::SUITE.multi_thread_index)(out::uint_slot())
        }),
        entry!("multiThreadIsSpawnedThread", Expect::Flag, |_h| unsafe {
            (multi_thread::SUITE.multi_thread_is_spawned_thread)()
        }),
        // ---------------------------------------- memory and message suites --
        entry!("memoryFree", Expect::AnyStatus, |h| unsafe {
            (memory::SUITE.memory_free)(h)
        }),
        entry!("message", Expect::AnyStatus, |h| unsafe {
            (message::SUITE.message)(
                h,
                c"OfxMessageError".as_ptr(),
                c"id".as_ptr(),
                c"text".as_ptr(),
            )
        }),
    ]
}

/// A small deterministic generator. Not cryptography — a spread of bit patterns
/// that is the same on every machine and every run.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// A number from the environment, or the default.
fn from_env<T: std::str::FromStr>(name: &str, fallback: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(fallback)
}

/// The handles the host really minted, which are the only ones a call is
/// allowed to accept. Everything else in the corpus must be refused.
fn live_handles() -> Vec<usize> {
    let mut live = Vec::new();
    if let Ok(handle) = host_props_handle() {
        live.push(handle.bits());
        // The host's own set is reachable under both faces of an effect, so the
        // recasts of every live handle count as live too.
        for kind in [HandleKind::ImageEffect, HandleKind::ParamSet] {
            if let Some(recast) = handle.recast(kind) {
                live.push(recast.bits());
            }
        }
    }
    live
}

/// Forged, stale, wrong-kind, freed and mutated handles: everything a plugin
/// can hand back that is not a handle.
fn corpus(live: &[usize]) -> Vec<(String, *mut c_void)> {
    let mut corpus: Vec<(String, *mut c_void)> = vec![
        ("null".to_owned(), std::ptr::null_mut()),
        ("garbage".to_owned(), 0xdead_beef_usize as *mut c_void),
        ("one".to_owned(), std::ptr::dangling_mut::<c_void>()),
        ("all ones".to_owned(), usize::MAX as *mut c_void),
    ];

    // Stale: minted, then destroyed. The index is never issued again, so this
    // is the shape of a handle a plugin kept too long.
    let stale = {
        let mut state = state();
        let handle = state
            .props
            .insert(PropertySet::new())
            .expect("room for one more");
        let _ = state.props.remove(handle);
        handle
    };
    corpus.push(("stale".to_owned(), stale.as_ptr()));

    // Wrong kind and past the end, for every kind of handle there is.
    for (name, kind) in [
        ("property set", HandleKind::PropertySet),
        ("image effect", HandleKind::ImageEffect),
        ("parameter", HandleKind::Param),
        ("clip", HandleKind::Clip),
        ("param set", HandleKind::ParamSet),
        ("mutex", HandleKind::Mutex),
    ] {
        for index in [0_usize, 1, 7, 1 << 30, (1 << 40) - 1] {
            let Some(handle) = Handle::encode(kind, index) else {
                continue;
            };
            if live.contains(&handle.bits()) {
                continue;
            }
            corpus.push((format!("{name} at {index}"), handle.as_ptr()));
        }
    }

    // And the mutations: real handle bits with the magic, the kind or the index
    // pulled about, plus plain random words.
    let mut rng = Xorshift(from_env("LUMIT_OFX_FUZZ_SEED", DEFAULT_SEED));
    let seeds: Vec<usize> = corpus.iter().map(|(_, ptr)| *ptr as usize).collect();
    for _ in 0..from_env("LUMIT_OFX_FUZZ_ITERS", DEFAULT_ITERATIONS) {
        let word = rng.next() as usize;
        let bits = match rng.next() % 3 {
            // A whole random word.
            0 => word,
            // One bit of a handle we have, flipped.
            1 => {
                let seed = seeds[(word >> 8) % seeds.len()];
                seed ^ (1_usize << (word % usize::BITS as usize))
            }
            // A handle whose fields are each random.
            _ => word & ((1 << 56) - 1),
        };
        if live.contains(&bits) {
            continue;
        }
        corpus.push((format!("mutated {bits:#x}"), bits as *mut c_void));
    }
    corpus
}

/// The whole point: no entry point of any suite may accept a handle the host
/// never minted, and none may do anything but answer.
#[test]
fn no_suite_entry_point_accepts_a_handle_that_is_not_one() {
    let live = live_handles();
    let corpus = corpus(&live);
    let entries = entries();
    assert!(corpus.len() > 1_000, "the corpus is the point of the test");

    for (name, handle) in &corpus {
        for entry in &entries {
            let status = (entry.call)(*handle);
            match entry.expect {
                Expect::BadHandle => assert_eq!(
                    status,
                    Status::ErrBadHandle.code(),
                    "{} answered {status} to a {name} handle",
                    entry.name
                ),
                Expect::AnyStatus => assert!(
                    (0..=14).contains(&status),
                    "{} answered {status}, which is not an OFX status, to a {name} handle",
                    entry.name
                ),
                Expect::Flag => assert!(
                    status == 0 || status == 1,
                    "{} answered {status}, which is not a flag, to a {name} handle",
                    entry.name
                ),
            }
        }
    }

    // The host is still itself afterwards: its own property set is live, holds
    // what it held, and nothing in the corpus reached it.
    let handle = host_props_handle().expect("the host has a property set");
    let state = state();
    let set = state.props.get(handle).expect("the host's set is live");
    assert!(
        set.get_string("OfxPropName", 0).is_ok(),
        "the host's own property set survived {} forged calls",
        corpus.len() * entries.len()
    );
}
