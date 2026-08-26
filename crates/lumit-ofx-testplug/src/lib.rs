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
//! There are five, because the host has five kinds of answer to give and each
//! needs something to give it to (see [`Variant`]): a plugin with one parameter
//! of every standard kind, a second version of the same plugin, a plugin that
//! works only in a context this host does not drive, a plugin that fails to
//! describe, and a plugin whose parameters collide.
//!
//! They also carry a few extra exports of their own — names beginning
//! `LumitTestPlug` — which no real plugin has. They are how a test asks what
//! was seen: how many times a host was handed over, whether one was in hand
//! before the load, and which suites the host actually gave.
//!
//! Render still answers `kOfxStatErrMissingHostFeature`: there is nothing to
//! render into until an instance exists.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use lumit_ofx::ffi::{
    actions, OfxHost, OfxImageEffectSuiteV1, OfxMessageSuiteV1, OfxParamSetHandle,
    OfxParameterSuiteV1, OfxPlugin, OfxPropertySetHandle, OfxPropertySuiteV1,
    K_OFX_IMAGE_EFFECT_PLUGIN_API_VERSION,
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
pub const PLUGIN_COUNT: c_int = 5;

/// What each of the five plugins is for.
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
        actions::DESCRIBE => describe(variant, handle.cast_mut()),
        actions::DESCRIBE_IN_CONTEXT => describe_in_context(variant, handle.cast_mut()),
        // The copy-the-input render lands with the instance package.
        actions::RENDER => Status::ErrMissingHostFeature.code(),
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
