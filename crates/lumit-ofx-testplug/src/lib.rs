//! `lumit-ofx-testplug` — a minimal OFX plugin, for testing the host.
//!
//! # In plain terms
//!
//! A host cannot be tested against nothing, and it should not be tested only
//! against somebody else's plugin: a commercial plugin cannot be shipped in a
//! repository, and a free one changes underneath the tests. So this is a
//! plugin of our own — the smallest thing that is genuinely an OFX plugin. It
//! exports the two functions a bundle must export, accepts the host, loads,
//! unloads, and records what happened so a test can check the order.
//!
//! It also carries a few extra exports of its own — names beginning
//! `LumitTestPlug` — which no real plugin has. They are how a test asks it
//! what it saw: how many times it was given a host, whether it had one before
//! it was told to load, and which suites the host actually gave it.
//!
//! Describe and render answer `kOfxStatErrMissingHostFeature` for now. They
//! are where the one double parameter and the copy-the-input render go, and
//! they land with the image effect and parameter suites, because there is
//! nothing to describe a parameter *to* until the host has those.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use lumit_ofx::ffi::{
    actions, OfxHost, OfxMessageSuiteV1, OfxPlugin, K_OFX_IMAGE_EFFECT_PLUGIN_API_VERSION,
};
use lumit_ofx::status::{OfxStatus, Status};

/// How many times `setHost` has been called.
static SET_HOST_CALLS: AtomicU32 = AtomicU32::new(0);
/// How many times `kOfxActionLoad` has been dispatched.
static LOAD_CALLS: AtomicU32 = AtomicU32::new(0);
/// Whether a host was already in hand at the first load.
static HOST_SEEN_BEFORE_LOAD: AtomicU32 = AtomicU32::new(0);
/// Which suites the host gave us, as a bit per suite (see [`SUITE_PROPERTY`]).
static SUITE_MASK: AtomicU32 = AtomicU32::new(0);
/// The host, as an address; the plugin never dereferences it except through
/// [`with_host`].
static HOST: AtomicUsize = AtomicUsize::new(0);

/// Bit for `OfxPropertySuiteV1` in [`SUITE_MASK`].
pub const SUITE_PROPERTY: u32 = 1;
/// Bit for `OfxMemorySuiteV1`.
pub const SUITE_MEMORY: u32 = 2;
/// Bit for `OfxMessageSuiteV1`.
pub const SUITE_MESSAGE: u32 = 4;
/// Bit for `OfxInteractSuiteV1`, which an honest host does not hand out yet.
pub const SUITE_INTERACT: u32 = 8;

/// The message this plugin sends at load, so a test can see the message suite
/// carry something end to end.
pub const LOAD_MESSAGE: &CStr = c"lumit-ofx-testplug loaded";

/// A wrapper that lets an `OfxPlugin` full of raw pointers be a `static`.
struct PluginStatic(OfxPlugin);

// SAFETY: the struct is built at compile time from string literals and
// function pointers and is never written to. Sharing it is sharing constants.
unsafe impl Sync for PluginStatic {}

static PLUGIN: PluginStatic = PluginStatic(OfxPlugin {
    plugin_api: c"OfxImageEffectPluginAPI".as_ptr(),
    api_version: K_OFX_IMAGE_EFFECT_PLUGIN_API_VERSION,
    plugin_identifier: c"com.lumitlab.testplug".as_ptr(),
    plugin_version_major: 1,
    plugin_version_minor: 0,
    set_host: Some(set_host),
    main_entry: Some(main_entry),
});

/// `OfxGetNumberOfPlugins`.
#[no_mangle]
pub extern "C" fn OfxGetNumberOfPlugins() -> c_int {
    1
}

/// `OfxGetPlugin`.
#[no_mangle]
pub extern "C" fn OfxGetPlugin(index: c_int) -> *const OfxPlugin {
    if index == 0 {
        std::ptr::from_ref(&PLUGIN.0)
    } else {
        std::ptr::null()
    }
}

unsafe extern "C" fn set_host(host: *const OfxHost) {
    SET_HOST_CALLS.fetch_add(1, Ordering::SeqCst);
    HOST.store(host as usize, Ordering::SeqCst);
}

/// Do something with the host, if there is one.
fn with_host<R>(body: impl FnOnce(&OfxHost) -> R) -> Option<R> {
    let host = HOST.load(Ordering::SeqCst) as *const OfxHost;
    if host.is_null() {
        return None;
    }
    // SAFETY: the pointer came from `setHost`, and OFX guarantees the host
    // outlives the plugin. Nothing here keeps the borrow.
    Some(body(unsafe { &*host }))
}

/// Ask the host for a suite by name.
fn fetch(host: &OfxHost, name: &CStr) -> *const c_void {
    let Some(fetch_suite) = host.fetch_suite else {
        return std::ptr::null();
    };
    // SAFETY: `name` outlives the call, and the host's own function is being
    // called with the arguments it declares.
    unsafe { fetch_suite(host.host, name.as_ptr(), 1) }
}

unsafe extern "C" fn main_entry(
    action: *const c_char,
    _handle: *const c_void,
    _in_args: *mut c_void,
    _out_args: *mut c_void,
) -> OfxStatus {
    if action.is_null() {
        return Status::ErrValue.code();
    }
    // SAFETY: OFX passes a NUL-terminated action name that outlives the call.
    let Ok(action) = (unsafe { CStr::from_ptr(action) }).to_str() else {
        return Status::ErrValue.code();
    };

    match action {
        actions::LOAD => on_load(),
        actions::UNLOAD => Status::Ok.code(),
        // The parameter and the copy live here, and arrive with the suites
        // that make them expressible.
        actions::DESCRIBE | actions::DESCRIBE_IN_CONTEXT | actions::RENDER => {
            Status::ErrMissingHostFeature.code()
        }
        // An action a plugin does not implement is not an error.
        _ => Status::ReplyDefault.code(),
    }
}

/// `kOfxActionLoad`: note the order, fetch the suites, and say hello through
/// the message suite so the whole path is exercised.
fn on_load() -> OfxStatus {
    if LOAD_CALLS.fetch_add(1, Ordering::SeqCst) == 0 && HOST.load(Ordering::SeqCst) != 0 {
        HOST_SEEN_BEFORE_LOAD.store(1, Ordering::SeqCst);
    }

    let fetched = with_host(|host| {
        let mut mask = 0;
        if !fetch(host, c"OfxPropertySuite").is_null() {
            mask |= SUITE_PROPERTY;
        }
        if !fetch(host, c"OfxMemorySuite").is_null() {
            mask |= SUITE_MEMORY;
        }
        let message = fetch(host, c"OfxMessageSuite");
        if !message.is_null() {
            mask |= SUITE_MESSAGE;
            // SAFETY: the host promised a suite of this shape at version 1;
            // the pointer is used only for the duration of this call.
            let suite = unsafe { &*message.cast::<OfxMessageSuiteV1>() };
            // SAFETY: the host's own function, given strings that outlive it.
            unsafe {
                (suite.message)(
                    std::ptr::null_mut(),
                    c"OfxMessageLog".as_ptr(),
                    c"".as_ptr(),
                    LOAD_MESSAGE.as_ptr(),
                );
            }
        }
        if !fetch(host, c"OfxInteractSuite").is_null() {
            mask |= SUITE_INTERACT;
        }
        mask
    });

    let Some(mask) = fetched else {
        // No host means the load order was broken, which is worth failing on:
        // it is the one thing this plugin exists to catch.
        return Status::ErrMissingHostFeature.code();
    };
    SUITE_MASK.store(mask, Ordering::SeqCst);
    // The property suite is the one suite no plugin can work without.
    if mask & SUITE_PROPERTY == 0 {
        return Status::ErrMissingHostFeature.code();
    }
    Status::Ok.code()
}

/// How many times the plugin was given a host.
#[no_mangle]
pub extern "C" fn LumitTestPlugSetHostCalls() -> c_int {
    SET_HOST_CALLS.load(Ordering::SeqCst) as c_int
}

/// Whether the plugin already had a host when it was first told to load.
#[no_mangle]
pub extern "C" fn LumitTestPlugHostSeenBeforeLoad() -> c_int {
    HOST_SEEN_BEFORE_LOAD.load(Ordering::SeqCst) as c_int
}

/// Which suites the host handed over, as [`SUITE_PROPERTY`] and friends.
#[no_mangle]
pub extern "C" fn LumitTestPlugSuiteMask() -> c_int {
    SUITE_MASK.load(Ordering::SeqCst) as c_int
}
