//! `lumit-ofx-testplug` — minimal OFX plugins, for testing the host.
//!
//! # In plain terms
//!
//! A host cannot be tested against nothing, and it should not be tested only
//! against somebody else's plugin: a commercial plugin cannot be shipped in a
//! repository, and a free one changes underneath the tests. So these are
//! plugins of our own — the smallest things that are genuinely OFX plugins.
//! They export the two functions a bundle must export, accept the host, load,
//! unload, describe themselves, and record what happened so a test can check
//! the order.
//!
//! There are eight, because the host has eight kinds of answer to give and each
//! needs something to give it to (see [`Variant`]): a plugin with one parameter
//! of every standard kind, a second version of the same plugin, a plugin that
//! works only in a context this host does not drive, a plugin that fails to
//! describe, a plugin whose parameters collide, a plugin that hands its input
//! back untouched, a plugin that says it is a no-op, and a plugin that says two
//! of its renders may never run at once.
//!
//! They also carry a few extra exports of their own — names beginning
//! `LumitTestPlug` — which no real plugin has. They are how a test asks what
//! was seen: how many times a host was handed over, whether one was in hand
//! before the load, which suites the host actually gave, **the exact sequence
//! of actions the host dispatched**, and how many renders were ever in flight
//! at one moment.
//!
//! The renders are real. They fetch images through `clipGetImage`, read the
//! data pointer, the bounds and the row bytes out of the property set they get
//! back, **honour the sign of the row bytes**, write float RGBA, and release
//! what they fetched. That is the whole of what a plugin does, and a host that
//! survives it is a host.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use lumit_ofx::ffi::{
    actions, OfxHost, OfxImageClipHandle, OfxImageEffectSuiteV1, OfxMessageSuiteV1,
    OfxMultiThreadSuiteV1, OfxParamHandle, OfxParamSetHandle, OfxParameterSuiteV1, OfxPlugin,
    OfxPropertySetHandle, OfxPropertySuiteV1, OfxRectD, K_OFX_IMAGE_EFFECT_PLUGIN_API_VERSION,
};
use lumit_ofx::status::{OfxStatus, Status};

/// How many times `setHost` has been called, across every plugin here.
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
/// Bit for `OfxImageEffectSuiteV1`.
pub const SUITE_IMAGE_EFFECT: u32 = 16;
/// Bit for `OfxParameterSuiteV1`.
pub const SUITE_PARAMETER: u32 = 32;

/// The message this plugin sends at load, so a test can see the message suite
/// carry something end to end.
pub const LOAD_MESSAGE: &CStr = c"lumit-ofx-testplug loaded";

/// How many plugins the bundle declares.
pub const PLUGIN_COUNT: c_int = 8;

/// Every action the host has dispatched since the log was last reset, comma
/// separated. Read through [`LumitTestPlugActionLog`].
static ACTION_LOG: Mutex<String> = Mutex::new(String::new());

/// Renders in flight at this moment, and the most there have ever been.
static RENDERS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// See [`RENDERS_IN_FLIGHT`].
static MAX_RENDERS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// How many renders must be in flight before any of them may finish. Nought
/// turns the rendezvous off; see [`LumitTestPlugSetRenderRendezvous`].
static RENDER_RENDEZVOUS: AtomicUsize = AtomicUsize::new(0);

/// Non-zero makes every render answer `kOfxStatFailed`.
static RENDER_FAILS: AtomicUsize = AtomicUsize::new(0);

/// What the plugin found when it asked the host how big its Source clip was,
/// during `getRegionOfDefinition`: nought if it never asked, one if the host
/// answered, two if the host could not say.
///
/// A real plugin asks that question there — most of openfx-misc does — and a
/// host that has not bound its clips until the render action answers "there is
/// no image", which is why this is worth a probe of its own.
static ROD_SAW_SOURCE: AtomicU32 = AtomicU32::new(0);

/// How long a render waits at the rendezvous before giving up. A host that
/// serialises the render will never reach the count, and the wait must end
/// rather than hang the test suite.
const RENDEZVOUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Render this frame and then die, without warning and without unwinding —
/// which is what a plugin with a bad pointer does. Set in the **broker's**
/// environment, because that is the only way to reach a plugin that is not in
/// the test's own process any more.
pub const CRASH_ON_FRAME_ENV: &str = "LUMIT_TESTPLUG_CRASH_ON_FRAME";

/// Never come back from a render. The host's deadline is what ends it.
pub const HANG_ENV: &str = "LUMIT_TESTPLUG_HANG";

/// Say this many things through the message suite during a render. A plugin in
/// a loop is a plugin the host must not let fill its memory.
pub const MESSAGE_SPAM_ENV: &str = "LUMIT_TESTPLUG_MESSAGE_SPAM";

/// Declare, and fetch, the source frames from `t − R` to `t + R`: a retimer,
/// in the smallest form that still is one. The output is the mean of all
/// `2R + 1` of them, so a frame that never arrived shows up in the pixels.
pub const TEMPORAL_ENV: &str = "LUMIT_TESTPLUG_TEMPORAL";

/// One of the environment flags, as a number. Absent, empty and unparseable all
/// read as "off", because a flag nobody set must never change behaviour.
fn flag(name: &str) -> Option<f64> {
    std::env::var(name).ok()?.trim().parse::<f64>().ok()
}

/// What each of the eight plugins is for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// `com.lumitlab.testplug` v1: one parameter of every standard kind, a
    /// group, a page, and both drivable contexts.
    Full,
    /// `com.lumitlab.testplug` v2: the same identifier at another version, with
    /// a different parameter list, so two versions cannot come out the same.
    Slim,
    /// Declares only the generator context, which this package does not drive.
    GeneratorOnly,
    /// Answers `kOfxStatFailed` to describe.
    Broken,
    /// Defines `centre` as a 2-D double **and** `centre_x` as a double, which
    /// collide once the 2-D one is spread across two rows.
    Duplicate,
    /// Hands its input back untouched, at fp32, and declares
    /// `kOfxImageEffectRenderFullySafe`. The plugin a host's pixel path is
    /// proved against: anything but the picture it was given is the host's
    /// fault.
    Passthrough,
    /// Answers `isIdentity` with its Source clip, so the host never renders it
    /// at all.
    Identity,
    /// Renders like the passthrough but declares
    /// `kOfxImageEffectRenderUnsafe`, so two of its renders must never overlap.
    ThreadUnsafe,
}

// ------------------------------------------------------------ the plugins --

/// A wrapper that lets an `OfxPlugin` full of raw pointers be a `static`.
struct PluginStatic(OfxPlugin);

// SAFETY: the struct is built at compile time from string literals and
// function pointers and is never written to. Sharing it is sharing constants.
unsafe impl Sync for PluginStatic {}

/// Declare one plugin: its own `mainEntry`, and the static that names it.
macro_rules! plugin {
    ($static_name:ident, $entry:ident, $id:expr, $major:expr, $variant:expr) => {
        unsafe extern "C" fn $entry(
            action: *const c_char,
            handle: *const c_void,
            in_args: *mut c_void,
            out_args: *mut c_void,
        ) -> OfxStatus {
            // SAFETY: the arguments are OFX's own, passed straight through to
            // the one dispatcher.
            unsafe { dispatch($variant, action, handle, in_args, out_args) }
        }

        static $static_name: PluginStatic = PluginStatic(OfxPlugin {
            plugin_api: c"OfxImageEffectPluginAPI".as_ptr(),
            api_version: K_OFX_IMAGE_EFFECT_PLUGIN_API_VERSION,
            plugin_identifier: $id.as_ptr(),
            plugin_version_major: $major,
            plugin_version_minor: 0,
            set_host: Some(set_host),
            main_entry: Some($entry),
        });
    };
}

plugin!(FULL, entry_full, c"com.lumitlab.testplug", 1, Variant::Full);
plugin!(SLIM, entry_slim, c"com.lumitlab.testplug", 2, Variant::Slim);
plugin!(
    GENERATOR,
    entry_generator,
    c"com.lumitlab.testplug.generator",
    1,
    Variant::GeneratorOnly
);
plugin!(
    BROKEN,
    entry_broken,
    c"com.lumitlab.testplug.broken",
    1,
    Variant::Broken
);
plugin!(
    DUPLICATE,
    entry_duplicate,
    c"com.lumitlab.testplug.duplicate",
    1,
    Variant::Duplicate
);
plugin!(
    PASSTHROUGH,
    entry_passthrough,
    c"com.lumitlab.testplug.passthrough",
    1,
    Variant::Passthrough
);
plugin!(
    IDENTITY,
    entry_identity,
    c"com.lumitlab.testplug.identity",
    1,
    Variant::Identity
);
plugin!(
    THREAD_UNSAFE,
    entry_thread_unsafe,
    c"com.lumitlab.testplug.unsafe",
    1,
    Variant::ThreadUnsafe
);

/// `OfxGetNumberOfPlugins`.
#[no_mangle]
pub extern "C" fn OfxGetNumberOfPlugins() -> c_int {
    PLUGIN_COUNT
}

/// `OfxGetPlugin`.
#[no_mangle]
pub extern "C" fn OfxGetPlugin(index: c_int) -> *const OfxPlugin {
    match index {
        0 => std::ptr::from_ref(&FULL.0),
        1 => std::ptr::from_ref(&SLIM.0),
        2 => std::ptr::from_ref(&GENERATOR.0),
        3 => std::ptr::from_ref(&BROKEN.0),
        4 => std::ptr::from_ref(&DUPLICATE.0),
        5 => std::ptr::from_ref(&PASSTHROUGH.0),
        6 => std::ptr::from_ref(&IDENTITY.0),
        7 => std::ptr::from_ref(&THREAD_UNSAFE.0),
        _ => std::ptr::null(),
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

/// Fetch one suite and hand it over as a reference, or `None`.
///
/// # Safety
///
/// `T` must be the struct the host declares for `name` at version 1.
unsafe fn suite<'a, T>(name: &CStr) -> Option<&'a T> {
    let pointer = with_host(|host| fetch(host, name))?;
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the caller's contract, plus the null check above; the host's
    // suite tables are statics that outlive the plugin.
    Some(unsafe { &*pointer.cast::<T>() })
}

/// The one dispatcher every plugin's `mainEntry` funnels into.
///
/// # Safety
///
/// The arguments are OFX's own: a NUL-terminated action name, and handles the
/// host minted.
unsafe fn dispatch(
    variant: Variant,
    action: *const c_char,
    handle: *const c_void,
    in_args: *mut c_void,
    out_args: *mut c_void,
) -> OfxStatus {
    if action.is_null() {
        return Status::ErrValue.code();
    }
    // SAFETY: OFX passes a NUL-terminated action name that outlives the call.
    let Ok(action) = (unsafe { CStr::from_ptr(action) }).to_str() else {
        return Status::ErrValue.code();
    };
    record_action(action);

    match action {
        actions::LOAD => on_load(),
        actions::UNLOAD => Status::Ok.code(),
        actions::DESCRIBE => describe(variant, handle.cast_mut()),
        actions::DESCRIBE_IN_CONTEXT => describe_in_context(variant, handle.cast_mut()),
        actions::CREATE_INSTANCE | actions::DESTROY_INSTANCE => Status::Ok.code(),
        actions::BEGIN_SEQUENCE_RENDER | actions::END_SEQUENCE_RENDER => Status::Ok.code(),
        actions::GET_REGION_OF_DEFINITION => region_of_definition(handle.cast_mut()),
        actions::IS_IDENTITY => is_identity(variant, out_args),
        actions::GET_FRAMES_NEEDED => frames_needed(in_args, out_args),
        actions::RENDER => render(variant, handle.cast_mut(), in_args),
        // An action a plugin does not implement is not an error.
        _ => Status::ReplyDefault.code(),
    }
}

/// Note an action in the log the test reads back.
fn record_action(action: &str) {
    let mut log = ACTION_LOG.lock().unwrap_or_else(PoisonError::into_inner);
    if !log.is_empty() {
        log.push(',');
    }
    log.push_str(action);
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
        if !fetch(host, c"OfxImageEffectSuite").is_null() {
            mask |= SUITE_IMAGE_EFFECT;
        }
        if !fetch(host, c"OfxParameterSuite").is_null() {
            mask |= SUITE_PARAMETER;
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

// ------------------------------------------------ writing into a prop set --

/// The property suite, or a failure the caller passes on.
fn properties<'a>() -> Option<&'a OfxPropertySuiteV1> {
    // SAFETY: `OfxPropertySuiteV1` is what the host declares for this name.
    unsafe { suite(c"OfxPropertySuite") }
}

fn set_string(
    props_suite: &OfxPropertySuiteV1,
    props: OfxPropertySetHandle,
    key: &CStr,
    index: c_int,
    value: &CStr,
) {
    // SAFETY: the host's own function, given strings that outlive the call.
    unsafe { (props_suite.prop_set_string)(props, key.as_ptr(), index, value.as_ptr()) };
}

fn set_double(
    props_suite: &OfxPropertySuiteV1,
    props: OfxPropertySetHandle,
    key: &CStr,
    index: c_int,
    value: f64,
) {
    // SAFETY: as above.
    unsafe { (props_suite.prop_set_double)(props, key.as_ptr(), index, value) };
}

fn set_int(
    props_suite: &OfxPropertySuiteV1,
    props: OfxPropertySetHandle,
    key: &CStr,
    index: c_int,
    value: c_int,
) {
    // SAFETY: as above.
    unsafe { (props_suite.prop_set_int)(props, key.as_ptr(), index, value) };
}

// ---------------------------------------------------------------- describe --

fn describe(variant: Variant, handle: *mut c_void) -> OfxStatus {
    if variant == Variant::Broken {
        // The plugin that says no. A host must come away with no schema, no
        // panic, and the rest of the bundle intact.
        return Status::Failed.code();
    }

    let (Some(props_suite), Some(effect_suite)) = (properties(), image_effect_suite()) else {
        return Status::ErrMissingHostFeature.code();
    };

    let mut props: OfxPropertySetHandle = std::ptr::null_mut();
    // SAFETY: the host's own function, given a valid out-parameter.
    let status = unsafe { (effect_suite.get_property_set)(handle, &raw mut props) };
    if status != Status::Ok.code() || props.is_null() {
        return Status::Failed.code();
    }

    set_string(props_suite, props, c"OfxPropLabel", 0, label_of(variant));
    set_string(
        props_suite,
        props,
        c"OfxImageEffectPluginPropGrouping",
        0,
        c"Lumit/Test",
    );
    // The declaration the host schedules from. Saying it here, at describe
    // time, is where a real plugin says it too.
    set_string(
        props_suite,
        props,
        c"OfxImageEffectPluginRenderThreadSafety",
        0,
        thread_safety_of(variant),
    );
    set_string(
        props_suite,
        props,
        c"OfxImageEffectPropSupportedPixelDepths",
        0,
        c"OfxBitDepthFloat",
    );

    match variant {
        Variant::GeneratorOnly => {
            set_string(
                props_suite,
                props,
                c"OfxImageEffectPropSupportedContexts",
                0,
                c"OfxImageEffectContextGenerator",
            );
        }
        Variant::Full => {
            set_string(
                props_suite,
                props,
                c"OfxImageEffectPropSupportedContexts",
                0,
                c"OfxImageEffectContextFilter",
            );
            set_string(
                props_suite,
                props,
                c"OfxImageEffectPropSupportedContexts",
                1,
                c"OfxImageEffectContextGeneral",
            );
            // Only the full plugin claims to read other frames, so a test can
            // see the trait follow the declaration rather than a constant.
            set_int(
                props_suite,
                props,
                c"OfxImageEffectPropTemporalClipAccess",
                0,
                1,
            );
        }
        _ => {
            set_string(
                props_suite,
                props,
                c"OfxImageEffectPropSupportedContexts",
                0,
                c"OfxImageEffectContextFilter",
            );
        }
    }
    Status::Ok.code()
}

fn label_of(variant: Variant) -> &'static CStr {
    match variant {
        Variant::Full => c"Test plug",
        Variant::Slim => c"Test plug mark two",
        Variant::GeneratorOnly => c"Test generator",
        Variant::Broken => c"Test broken",
        Variant::Duplicate => c"Test duplicate",
        Variant::Passthrough => c"Test passthrough",
        Variant::Identity => c"Test identity",
        Variant::ThreadUnsafe => c"Test unsafe",
    }
}

/// What each variant declares about running two renders at once.
fn thread_safety_of(variant: Variant) -> &'static CStr {
    match variant {
        Variant::ThreadUnsafe => c"OfxImageEffectRenderUnsafe",
        _ => c"OfxImageEffectRenderFullySafe",
    }
}

fn image_effect_suite<'a>() -> Option<&'a OfxImageEffectSuiteV1> {
    // SAFETY: `OfxImageEffectSuiteV1` is what the host declares for this name.
    unsafe { suite(c"OfxImageEffectSuite") }
}

fn parameter_suite<'a>() -> Option<&'a OfxParameterSuiteV1> {
    // SAFETY: `OfxParameterSuiteV1` is what the host declares for this name.
    unsafe { suite(c"OfxParameterSuite") }
}

/// Define one parameter and hand back its property set, or null. A null is a
/// definition the host refused; every writer below tolerates one, because
/// `propSetString` on a bad handle is an error code and not a crash — which is
/// the host behaviour this plugin exists to lean on.
fn define(
    param_suite: &OfxParameterSuiteV1,
    params: OfxParamSetHandle,
    param_type: &CStr,
    name: &CStr,
) -> OfxPropertySetHandle {
    let mut props: OfxPropertySetHandle = std::ptr::null_mut();
    // SAFETY: the host's own function, given strings that outlive the call and
    // a valid out-parameter.
    let status = unsafe {
        (param_suite.param_define)(params, param_type.as_ptr(), name.as_ptr(), &raw mut props)
    };
    if status == Status::Ok.code() {
        props
    } else {
        std::ptr::null_mut()
    }
}

fn describe_in_context(variant: Variant, handle: *mut c_void) -> OfxStatus {
    let (Some(props_suite), Some(effect_suite), Some(param_suite)) =
        (properties(), image_effect_suite(), parameter_suite())
    else {
        return Status::ErrMissingHostFeature.code();
    };

    // Every effect has a source and an output; a filter is exactly those two.
    for name in [c"Source", c"Output"] {
        let mut clip: OfxPropertySetHandle = std::ptr::null_mut();
        // SAFETY: the host's own function, given a valid out-parameter.
        let status = unsafe { (effect_suite.clip_define)(handle, name.as_ptr(), &raw mut clip) };
        if status != Status::Ok.code() || clip.is_null() {
            return Status::Failed.code();
        }
        set_string(
            props_suite,
            clip,
            c"OfxImageEffectPropSupportedComponents",
            0,
            c"OfxImageComponentRGBA",
        );
    }

    let mut params: OfxParamSetHandle = std::ptr::null_mut();
    // SAFETY: as above.
    let status = unsafe { (effect_suite.get_param_set)(handle, &raw mut params) };
    if status != Status::Ok.code() || params.is_null() {
        return Status::Failed.code();
    }

    match variant {
        Variant::Slim => {
            let gain = define(param_suite, params, c"OfxParamTypeDouble", c"gain");
            if gain.is_null() {
                return Status::Failed.code();
            }
            set_string(props_suite, gain, c"OfxPropLabel", 0, c"Gain");
            set_double(props_suite, gain, c"OfxParamPropDefault", 0, 1.0);
        }
        Variant::Duplicate => {
            // `centre` spreads into `centre_x` and `centre_y`, and then the
            // second definition lands on `centre_x` too. Both definitions are
            // legal OFX; the collision only exists once Lumit spells a point as
            // two rows, which is why the host has to catch it.
            let centre = define(param_suite, params, c"OfxParamTypeDouble2D", c"centre");
            let clash = define(param_suite, params, c"OfxParamTypeDouble", c"centre_x");
            if centre.is_null() || clash.is_null() {
                return Status::Failed.code();
            }
        }
        // The three render plugins have no controls at all: what they are for
        // is the pixels, and a control would only be something else to explain
        // when one of those tests goes red.
        Variant::Passthrough | Variant::Identity | Variant::ThreadUnsafe => {}
        _ => describe_full(props_suite, param_suite, params),
    }
    Status::Ok.code()
}

/// The full plugin's controls: one of every standard kind, a group with two
/// members, and a page with two more.
fn describe_full(
    props_suite: &OfxPropertySuiteV1,
    param_suite: &OfxParameterSuiteV1,
    params: OfxParamSetHandle,
) {
    let label = c"OfxPropLabel";
    let default = c"OfxParamPropDefault";
    let min = c"OfxParamPropMin";
    let max = c"OfxParamPropMax";
    let display_min = c"OfxParamPropDisplayMin";
    let display_max = c"OfxParamPropDisplayMax";
    let double_type = c"OfxParamPropDoubleType";
    let parent = c"OfxParamPropParent";

    // A plain number with both a slider range and a hard range.
    let gain = define(param_suite, params, c"OfxParamTypeDouble", c"gain");
    set_string(props_suite, gain, label, 0, c"Gain");
    set_double(props_suite, gain, default, 0, 0.5);
    set_double(props_suite, gain, min, 0, 0.0);
    set_double(props_suite, gain, max, 0, 4.0);
    set_double(props_suite, gain, display_min, 0, 0.0);
    set_double(props_suite, gain, display_max, 0, 2.0);

    // An angle, which is degrees by definition.
    let rotation = define(param_suite, params, c"OfxParamTypeDouble", c"rotation");
    set_string(props_suite, rotation, label, 0, c"Rotation");
    set_string(
        props_suite,
        rotation,
        double_type,
        0,
        c"OfxParamDoubleTypeAngle",
    );
    set_double(props_suite, rotation, default, 0, 45.0);

    // A point, in absolute coordinates: a distance, and therefore px@comp.
    let centre = define(param_suite, params, c"OfxParamTypeDouble2D", c"centre");
    set_string(props_suite, centre, label, 0, c"Centre");
    set_string(
        props_suite,
        centre,
        double_type,
        0,
        c"OfxParamDoubleTypeXYAbsolute",
    );
    for axis in 0..2 {
        set_double(props_suite, centre, default, axis, 0.0);
        set_double(props_suite, centre, display_min, axis, -100.0);
        set_double(props_suite, centre, display_max, axis, 100.0);
    }

    // A group, and the two rows that live inside it.
    let advanced = define(param_suite, params, c"OfxParamTypeGroup", c"advanced");
    set_string(props_suite, advanced, label, 0, c"Advanced");
    set_int(props_suite, advanced, c"OfxParamPropGroupOpen", 0, 0);

    let offset = define(param_suite, params, c"OfxParamTypeDouble3D", c"offset");
    set_string(props_suite, offset, label, 0, c"Offset");
    set_string(props_suite, offset, parent, 0, c"advanced");
    for (axis, value) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        set_double(props_suite, offset, default, axis as c_int, value);
    }

    let count = define(param_suite, params, c"OfxParamTypeInteger", c"count");
    set_string(props_suite, count, label, 0, c"Count");
    set_string(props_suite, count, parent, 0, c"advanced");
    set_int(props_suite, count, default, 0, 3);
    set_int(props_suite, count, min, 0, 1);
    set_int(props_suite, count, max, 0, 10);
    set_int(props_suite, count, display_min, 0, 1);
    set_int(props_suite, count, display_max, 0, 10);

    let size = define(param_suite, params, c"OfxParamTypeInteger2D", c"size");
    set_string(props_suite, size, label, 0, c"Size");
    set_int(props_suite, size, default, 0, 640);
    set_int(props_suite, size, default, 1, 480);

    let enabled = define(param_suite, params, c"OfxParamTypeBoolean", c"enabled");
    set_string(props_suite, enabled, label, 0, c"Enabled");
    set_int(props_suite, enabled, default, 0, 1);

    let mode = define(param_suite, params, c"OfxParamTypeChoice", c"mode");
    set_string(props_suite, mode, label, 0, c"Mode");
    for (index, option) in [c"Soft", c"Hard", c"Wild"].into_iter().enumerate() {
        set_string(
            props_suite,
            mode,
            c"OfxParamPropChoiceOption",
            index as c_int,
            option,
        );
    }
    set_int(props_suite, mode, default, 0, 1);

    let tint = define(param_suite, params, c"OfxParamTypeRGBA", c"tint");
    set_string(props_suite, tint, label, 0, c"Tint");
    for (channel, value) in [0.25, 0.5, 0.75, 1.0].into_iter().enumerate() {
        set_double(props_suite, tint, default, channel as c_int, value);
    }

    let wash = define(param_suite, params, c"OfxParamTypeRGB", c"wash");
    set_string(props_suite, wash, label, 0, c"Wash");
    for (channel, value) in [1.0, 0.0, 0.0].into_iter().enumerate() {
        set_double(props_suite, wash, default, channel as c_int, value);
    }

    // Text, which Lumit has no row for; it is reported, not silently dropped.
    let caption = define(param_suite, params, c"OfxParamTypeString", c"caption");
    set_string(props_suite, caption, label, 0, c"Caption");

    // A path, which Lumit does have a row for.
    let lut = define(param_suite, params, c"OfxParamTypeString", c"lutPath");
    set_string(props_suite, lut, label, 0, c"LUT file");
    set_string(
        props_suite,
        lut,
        c"OfxParamPropStringMode",
        0,
        c"OfxParamStringIsFilePath",
    );

    // An opaque vendor blob, round-tripped and never interpreted.
    let blob = define(param_suite, params, c"OfxParamTypeCustom", c"vendorBlob");
    set_string(props_suite, blob, label, 0, c"Vendor blob");

    let trigger = define(param_suite, params, c"OfxParamTypePushButton", c"trigger");
    set_string(props_suite, trigger, label, 0, c"Trigger");

    // A page listing the two rows that are not in the group.
    let page = define(param_suite, params, c"OfxParamTypePage", c"filesPage");
    set_string(props_suite, page, label, 0, c"Files");
    for (index, child) in [c"lutPath", c"trigger"].into_iter().enumerate() {
        set_string(
            props_suite,
            page,
            c"OfxParamPropPageChild",
            index as c_int,
            child,
        );
    }
}

// ----------------------------------------------------------------- render --

/// `kOfxImageEffectActionGetRegionOfDefinition`: ask the host how big the input
/// is, note what it said, and leave the answer to the host.
///
/// The plugin wants nothing from this action — its output is its input's size,
/// which is what the host assumes anyway — but asking is the point. It is the
/// first moment a plugin touches its clips, long before the render action, and
/// the host has to have bound them by then.
fn region_of_definition(handle: *mut c_void) -> OfxStatus {
    let Some(effect_suite) = image_effect_suite() else {
        return Status::ErrMissingHostFeature.code();
    };
    let mut clip: OfxImageClipHandle = std::ptr::null_mut();
    let mut props: OfxPropertySetHandle = std::ptr::null_mut();
    // SAFETY: the host's own functions, given the handle it passed us and
    // valid out-parameters.
    let seen = unsafe {
        let got = (effect_suite.clip_get_handle)(
            handle,
            c"Source".as_ptr(),
            &raw mut clip,
            &raw mut props,
        );
        if got != Status::Ok.code() || clip.is_null() {
            2
        } else {
            let mut bounds = OfxRectD::default();
            if (effect_suite.clip_get_region_of_definition)(clip, 0.0, &raw mut bounds)
                == Status::Ok.code()
                && bounds.x2 > bounds.x1
            {
                1
            } else {
                2
            }
        }
    };
    ROD_SAW_SOURCE.store(seen, Ordering::SeqCst);
    // The host's own answer is the right one; this action exists here only to
    // ask a question on the way past.
    Status::ReplyDefault.code()
}

/// `kOfxImageEffectActionIsIdentity`. Only one variant ever says yes, and it
/// says it by naming the clip its output would have been.
fn is_identity(variant: Variant, out_args: *mut c_void) -> OfxStatus {
    if variant != Variant::Identity {
        return Status::ReplyDefault.code();
    }
    let Some(props_suite) = properties() else {
        return Status::ErrMissingHostFeature.code();
    };
    set_string(props_suite, out_args, c"OfxPropName", 0, c"Source");
    set_double(props_suite, out_args, c"OfxPropTime", 0, 0.0);
    Status::Ok.code()
}

/// `kOfxImageEffectActionGetFramesNeeded`. With [`TEMPORAL_ENV`] unset this is
/// an action the plugin does not implement, which is not an error; with it set
/// the plugin declares `t ± R` on its Source, which is what a retimer does and
/// what the host's prefetch is driven by.
fn frames_needed(in_args: *mut c_void, out_args: *mut c_void) -> OfxStatus {
    let Some(radius) = flag(TEMPORAL_ENV).filter(|radius| *radius >= 1.0) else {
        return Status::ReplyDefault.code();
    };
    let Some(props_suite) = properties() else {
        return Status::ErrMissingHostFeature.code();
    };
    let mut time = 0.0;
    // SAFETY: the host's own function, given the `inArgs` it passed us.
    unsafe {
        (props_suite.prop_get_double)(in_args, c"OfxPropTime".as_ptr(), 0, &raw mut time);
    }
    set_double(
        props_suite,
        out_args,
        c"OfxImageClipPropFrameRange_Source",
        0,
        time - radius,
    );
    set_double(
        props_suite,
        out_args,
        c"OfxImageClipPropFrameRange_Source",
        1,
        time + radius,
    );
    Status::Ok.code()
}

/// One image, as the four numbers that describe it.
struct ImageView {
    /// The pixel at the bottom-left, which for a top-down image is inside the
    /// block rather than at its start.
    data: *mut c_void,
    /// `x1, y1, x2, y2`.
    bounds: [c_int; 4],
    /// **Signed.** Negative means the rows run backwards through memory, and
    /// this plugin honours it rather than assuming.
    row_bytes: c_int,
    /// The property set to release when the render is done.
    handle: OfxPropertySetHandle,
}

impl ImageView {
    /// The floats of one OFX row, `y` counted up from `y1`.
    ///
    /// # Safety
    ///
    /// The image must still be pinned — fetched and not yet released.
    unsafe fn row(&self, y: usize) -> *mut f32 {
        let offset = (y as isize) * (self.row_bytes as isize);
        // SAFETY: the caller's contract, plus `y` being inside the bounds the
        // host itself gave us; the sign of `row_bytes` is what makes this land
        // in the block either way.
        unsafe { self.data.cast::<u8>().offset(offset).cast::<f32>() }
    }

    fn width(&self) -> usize {
        (self.bounds[2] - self.bounds[0]).max(0) as usize
    }

    fn height(&self) -> usize {
        (self.bounds[3] - self.bounds[1]).max(0) as usize
    }
}

/// Fetch one clip's image and read its description out of the property set.
fn fetch_image(
    effect_suite: &OfxImageEffectSuiteV1,
    props_suite: &OfxPropertySuiteV1,
    clip: OfxImageClipHandle,
    time: f64,
) -> Option<ImageView> {
    if clip.is_null() {
        return None;
    }
    let mut handle: OfxPropertySetHandle = std::ptr::null_mut();
    // SAFETY: the host's own function, given a valid out-parameter.
    let status =
        unsafe { (effect_suite.clip_get_image)(clip, time, std::ptr::null(), &raw mut handle) };
    if status != Status::Ok.code() || handle.is_null() {
        return None;
    }

    let mut data: *mut c_void = std::ptr::null_mut();
    // SAFETY: the host's own function; `OfxImagePropData` is a pointer
    // property, which is what `propGetPointer` reads.
    let got = unsafe {
        (props_suite.prop_get_pointer)(handle, c"OfxImagePropData".as_ptr(), 0, &raw mut data)
    };
    let mut bounds = [0; 4];
    for (index, slot) in bounds.iter_mut().enumerate() {
        // SAFETY: as above; the bounds are an int property of four elements.
        unsafe {
            (props_suite.prop_get_int)(
                handle,
                c"OfxImagePropBounds".as_ptr(),
                index as c_int,
                slot,
            );
        }
    }
    let mut row_bytes: c_int = 0;
    // SAFETY: as above.
    unsafe {
        (props_suite.prop_get_int)(
            handle,
            c"OfxImagePropRowBytes".as_ptr(),
            0,
            &raw mut row_bytes,
        );
    }
    if got != Status::Ok.code() || data.is_null() {
        // SAFETY: the host's own function, given the handle it just minted.
        unsafe { (effect_suite.clip_release_image)(handle) };
        return None;
    }
    Some(ImageView {
        data,
        bounds,
        row_bytes,
        handle,
    })
}

/// One clip handle, or null.
fn clip_handle(
    effect_suite: &OfxImageEffectSuiteV1,
    effect: *mut c_void,
    name: &CStr,
) -> OfxImageClipHandle {
    let mut clip: OfxImageClipHandle = std::ptr::null_mut();
    let mut props: OfxPropertySetHandle = std::ptr::null_mut();
    // SAFETY: the host's own function, given valid out-parameters and a string
    // that outlives the call.
    let status = unsafe {
        (effect_suite.clip_get_handle)(effect, name.as_ptr(), &raw mut clip, &raw mut props)
    };
    if status == Status::Ok.code() {
        clip
    } else {
        std::ptr::null_mut()
    }
}

/// One double control's value at a time, or `fallback` if the plugin has no
/// such control.
fn param_double(
    param_suite: &OfxParameterSuiteV1,
    props_suite: &OfxPropertySuiteV1,
    effect_suite: &OfxImageEffectSuiteV1,
    effect: *mut c_void,
    name: &CStr,
    time: f64,
    fallback: f64,
) -> f64 {
    let _ = props_suite;
    let mut params: OfxParamSetHandle = std::ptr::null_mut();
    // SAFETY: the host's own function, given a valid out-parameter.
    if unsafe { (effect_suite.get_param_set)(effect, &raw mut params) } != Status::Ok.code() {
        return fallback;
    }
    let mut param: OfxParamHandle = std::ptr::null_mut();
    let mut param_props: OfxPropertySetHandle = std::ptr::null_mut();
    // SAFETY: as above.
    let status = unsafe {
        (param_suite.param_get_handle)(params, name.as_ptr(), &raw mut param, &raw mut param_props)
    };
    if status != Status::Ok.code() || param.is_null() {
        return fallback;
    }
    let mut value = fallback;
    // A real C-variadic call, as a plugin compiled against the header makes
    // it: one trailing pointer, because a double is one dimension. This is
    // what proves the host's shim on every platform the suite runs on.
    // SAFETY: the host's own entry point, given a live handle and a pointer to
    // a double, which is what it declares the parameter to be.
    let read = unsafe {
        (param_suite.param_get_value_at_time)(
            param,
            time,
            std::ptr::from_mut(&mut value).cast::<c_void>(),
        )
    };
    if read == Status::Ok.code() {
        value
    } else {
        fallback
    }
}

/// `kOfxImageEffectActionRender` — the whole point.
fn render(variant: Variant, effect: *mut c_void, in_args: *mut c_void) -> OfxStatus {
    let in_flight = RENDERS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
    MAX_RENDERS_IN_FLIGHT.fetch_max(in_flight, Ordering::SeqCst);
    // If a test asked for a rendezvous, hold here until enough renders have
    // arrived — or until the deadline, which is what a host that serialises
    // them will hit. Either way the maximum above is the answer.
    let wanted = RENDER_RENDEZVOUS.load(Ordering::SeqCst);
    if wanted > 1 {
        let deadline = Instant::now() + RENDEZVOUS_TIMEOUT;
        while RENDERS_IN_FLIGHT.load(Ordering::SeqCst) < wanted && Instant::now() < deadline {
            std::thread::yield_now();
        }
    }
    let status = render_body(variant, effect, in_args);
    RENDERS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    status
}

/// Crash, hang, or shout — whichever a test asked this process to do.
///
/// The crash is a real one: `abort` does not unwind, does not run destructors,
/// and leaves the process dead the way a bad pointer does. That is the point —
/// a host that only survives a tidy failure has not been tested.
fn misbehave(props_suite: &OfxPropertySuiteV1, time: f64) {
    let _ = props_suite;
    if let Some(frame) = flag(CRASH_ON_FRAME_ENV) {
        if (time - frame).abs() < 0.5 {
            std::process::abort();
        }
    }
    if flag(HANG_ENV).is_some_and(|value| value != 0.0) {
        loop {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    if let Some(count) = flag(MESSAGE_SPAM_ENV) {
        // SAFETY: `OfxMessageSuiteV1` is what the host declares for this name.
        let Some(suite) = (unsafe { suite::<OfxMessageSuiteV1>(c"OfxMessageSuite") }) else {
            return;
        };
        let mut sent = 0.0;
        while sent < count {
            // SAFETY: the host's own function, given strings that outlive it.
            unsafe {
                (suite.message)(
                    std::ptr::null_mut(),
                    c"OfxMessageLog".as_ptr(),
                    c"".as_ptr(),
                    c"lumit-ofx-testplug says this rather a lot".as_ptr(),
                );
            }
            sent += 1.0;
        }
    }
}

/// Fill and scale: every pixel of the output is the matching pixel of the
/// Source multiplied by the `gain` control, and where there is no Source at all
/// the output is filled with a flat colour instead.
fn render_body(variant: Variant, effect: *mut c_void, in_args: *mut c_void) -> OfxStatus {
    let (Some(props_suite), Some(effect_suite), Some(param_suite)) =
        (properties(), image_effect_suite(), parameter_suite())
    else {
        return Status::ErrMissingHostFeature.code();
    };

    // The plugin that says no, on demand. It refuses **after** the host has
    // set the render up, which is the case that matters: the output buffer
    // exists and is half-written, and the host must not hand it on.
    if RENDER_FAILS.load(Ordering::SeqCst) != 0 {
        return Status::Failed.code();
    }

    let mut time = 0.0;
    // SAFETY: the host's own function, given the `inArgs` it passed us.
    unsafe {
        (props_suite.prop_get_double)(in_args, c"OfxPropTime".as_ptr(), 0, &raw mut time);
    }

    // The three ways a plugin goes wrong, on purpose. None of them can happen
    // unless a test asked for it in this process's environment.
    misbehave(props_suite, time);

    let output = clip_handle(effect_suite, effect, c"Output");
    let Some(destination) = fetch_image(effect_suite, props_suite, output, time) else {
        return Status::Failed.code();
    };
    let source = fetch_image(
        effect_suite,
        props_suite,
        clip_handle(effect_suite, effect, c"Source"),
        time,
    );

    // The retimer's fetch: every other frame it declared in `getFramesNeeded`,
    // asked for one at a time exactly as a real one asks. The host is expected
    // to have them all in hand already.
    let mut others = Vec::new();
    let radius = flag(TEMPORAL_ENV).filter(|radius| *radius >= 1.0);
    let divisor = match radius {
        Some(radius) => {
            let steps = radius as i32;
            for step in -steps..=steps {
                if step == 0 {
                    continue;
                }
                if let Some(view) = fetch_image(
                    effect_suite,
                    props_suite,
                    clip_handle(effect_suite, effect, c"Source"),
                    time + f64::from(step),
                ) {
                    others.push(view);
                }
            }
            (steps * 2 + 1) as f32
        }
        None => 1.0,
    };

    // A passthrough scales by one, whatever anybody's control says.
    let gain = if variant == Variant::Full || variant == Variant::Slim {
        param_double(
            param_suite,
            props_suite,
            effect_suite,
            effect,
            c"gain",
            time,
            1.0,
        )
    } else {
        1.0
    };
    // The flat colour for the no-input case: opaque mid grey, which is
    // recognisably "the plugin filled this" and not "the host forgot to".
    let fill = [0.5_f32, 0.5, 0.5, 1.0];

    let (width, height) = (destination.width(), destination.height());
    for y in 0..height {
        // SAFETY: the image is pinned until it is released below, and `y` is
        // inside the bounds the host gave us.
        let row = unsafe { destination.row(y) };
        for x in 0..width {
            for (channel, flat) in fill.iter().enumerate() {
                let index = x * 4 + channel;
                let value = match &source {
                    Some(source) if source.width() == width && source.height() == height => {
                        // SAFETY: as above, for the source.
                        let from = unsafe { source.row(y) };
                        // SAFETY: `index` is inside one row of four-channel
                        // pixels, which is what both rows hold.
                        let mut sum = unsafe { *from.add(index) };
                        for other in &others {
                            if other.width() != width || other.height() != height {
                                continue;
                            }
                            // SAFETY: as above, for a frame at another time.
                            let row = unsafe { other.row(y) };
                            // SAFETY: as above.
                            sum += unsafe { *row.add(index) };
                        }
                        // Divided by how many frames were *asked* for, not by
                        // how many arrived: a frame the host failed to prefetch
                        // has to show in the picture, or the test that counts
                        // them is testing nothing.
                        (sum / divisor) * (gain as f32)
                    }
                    _ => *flat,
                };
                // SAFETY: as above.
                unsafe { *row.add(index) = value };
            }
        }
    }

    // Releasing is not optional and releasing twice is not allowed; both are
    // things the host is tested on, so this plugin does the correct one.
    if let Some(source) = source {
        // SAFETY: the host's own function, given a handle it minted and this
        // plugin has not released.
        unsafe { (effect_suite.clip_release_image)(source.handle) };
    }
    for other in others {
        // SAFETY: as above.
        unsafe { (effect_suite.clip_release_image)(other.handle) };
    }
    // SAFETY: as above.
    unsafe { (effect_suite.clip_release_image)(destination.handle) };
    Status::Ok.code()
}

// ------------------------------------------------------------- the probes --

/// How many times a plugin here was given a host.
#[no_mangle]
pub extern "C" fn LumitTestPlugSetHostCalls() -> c_int {
    SET_HOST_CALLS.load(Ordering::SeqCst) as c_int
}

/// Whether a host was already in hand at the first load.
#[no_mangle]
pub extern "C" fn LumitTestPlugHostSeenBeforeLoad() -> c_int {
    HOST_SEEN_BEFORE_LOAD.load(Ordering::SeqCst) as c_int
}

/// Which suites the host handed over, as [`SUITE_PROPERTY`] and friends.
#[no_mangle]
pub extern "C" fn LumitTestPlugSuiteMask() -> c_int {
    SUITE_MASK.load(Ordering::SeqCst) as c_int
}

/// Forget the action log and the concurrency high-water mark, and turn the
/// rendezvous off. A test calls this immediately before the stretch it means to
/// observe.
#[no_mangle]
pub extern "C" fn LumitTestPlugResetProbes() {
    ACTION_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    RENDERS_IN_FLIGHT.store(0, Ordering::SeqCst);
    MAX_RENDERS_IN_FLIGHT.store(0, Ordering::SeqCst);
    RENDER_RENDEZVOUS.store(0, Ordering::SeqCst);
    RENDER_FAILS.store(0, Ordering::SeqCst);
    ROD_SAW_SOURCE.store(0, Ordering::SeqCst);
}

/// What the host said when the plugin asked its Source clip's size during
/// `getRegionOfDefinition`: nought never asked, one answered, two could not.
#[no_mangle]
pub extern "C" fn LumitTestPlugRodSawSource() -> c_int {
    ROD_SAW_SOURCE.load(Ordering::SeqCst) as c_int
}

/// Make every render answer `kOfxStatFailed`, or stop doing so.
#[no_mangle]
pub extern "C" fn LumitTestPlugSetRenderFails(fails: c_int) {
    RENDER_FAILS.store(fails.max(0) as usize, Ordering::SeqCst);
}

/// Copy the action log into `buffer` as a NUL-terminated string, and answer
/// with how long the log actually is. A buffer too small is truncated, not
/// overrun.
///
/// # Safety
///
/// `buffer` must be null or point at `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn LumitTestPlugActionLog(buffer: *mut c_char, capacity: c_int) -> c_int {
    let log = ACTION_LOG.lock().unwrap_or_else(PoisonError::into_inner);
    let bytes = log.as_bytes();
    let length = c_int::try_from(bytes.len()).unwrap_or(c_int::MAX);
    if buffer.is_null() || capacity <= 0 {
        return length;
    }
    let room = (capacity as usize).saturating_sub(1).min(bytes.len());
    for (index, byte) in bytes.iter().take(room).enumerate() {
        // SAFETY: the caller's contract; `index` is below `capacity - 1`.
        unsafe { *buffer.add(index) = *byte as c_char };
    }
    // SAFETY: as above; `room` is at most `capacity - 1`.
    unsafe { *buffer.add(room) = 0 };
    length
}

/// The most renders that were ever in flight at one moment since the last
/// reset. One means the host serialised them.
#[no_mangle]
pub extern "C" fn LumitTestPlugMaxConcurrentRenders() -> c_int {
    c_int::try_from(MAX_RENDERS_IN_FLIGHT.load(Ordering::SeqCst)).unwrap_or(c_int::MAX)
}

/// Make every render wait until `count` of them are in flight before any
/// finishes, so "did these two overlap?" is a question with a definite answer
/// rather than a race with a sleep in it. Nought turns it off.
#[no_mangle]
pub extern "C" fn LumitTestPlugSetRenderRendezvous(count: c_int) {
    RENDER_RENDEZVOUS.store(count.max(0) as usize, Ordering::SeqCst);
}

/// What the multi-thread suite told this plugin, so a test can read the host's
/// answers from the plugin's side rather than from the host's.
///
/// Answers `numCPUs`, and then runs a fan-out of that many threads which
/// records the indices it was given: the return is the number of *distinct*
/// indices seen, which equals the thread count exactly when
/// `multiThreadIndex` is right.
///
/// # Safety
///
/// `num_cpus_out` must be null or point at writable storage for one `int`.
#[no_mangle]
pub unsafe extern "C" fn LumitTestPlugFanOut(num_cpus_out: *mut c_int) -> c_int {
    // SAFETY: `OfxMultiThreadSuiteV1` is what the host declares for this name.
    let Some(suite) = (unsafe { suite::<OfxMultiThreadSuiteV1>(c"OfxMultiThreadSuite") }) else {
        return -1;
    };
    let mut cpus: c_uint = 0;
    // SAFETY: the host's own function, given a valid out-parameter.
    if unsafe { (suite.multi_thread_num_cpus)(&raw mut cpus) } != Status::Ok.code() {
        return -1;
    }
    if !num_cpus_out.is_null() {
        // SAFETY: the caller's out-parameter, checked non-null.
        unsafe { *num_cpus_out = cpus as c_int };
    }

    FAN_OUT_SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    // SAFETY: the host's own function, given this plugin's own thread body and
    // a null custom argument, which the body does not follow.
    let status = unsafe { (suite.multi_thread)(fan_out_body, cpus, std::ptr::null_mut()) };
    if status != Status::Ok.code() {
        return -1;
    }
    let mut seen = FAN_OUT_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    seen.sort_unstable();
    seen.dedup();
    c_int::try_from(seen.len()).unwrap_or(c_int::MAX)
}

/// The indices the last fan-out handed out.
static FAN_OUT_SEEN: Mutex<Vec<c_uint>> = Mutex::new(Vec::new());

/// One thread of [`LumitTestPlugFanOut`]: note the index the host says this is.
unsafe extern "C" fn fan_out_body(thread_index: c_uint, _thread_max: c_uint, _arg: *mut c_void) {
    FAN_OUT_SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(thread_index);
}
