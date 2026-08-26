//! The OFX C declarations, transcribed by hand.
//!
//! # In plain terms
//!
//! OpenFX is a C interface: a handful of structs full of function pointers
//! that the host and the plugin agree on, byte for byte. These are those
//! structs, written out in Rust with `#[repr(C)]` so the compiler lays them
//! out exactly as a C compiler would. There are few enough of them that
//! generating them from the headers would cost more than typing them, and a
//! generated file could not carry the notes that matter; the layout tests in
//! [`crate::tests`] are what keeps the transcription honest (docs/14 §7).
//!
//! Nothing here has behaviour. It is a description of a shape.

use std::ffi::{c_char, c_int, c_uint, c_void};

use crate::status::OfxStatus;

/// Every OFX object — a property set, an effect, a parameter, a clip — is an
/// opaque pointer the host mints. Ours are never real pointers; see
/// [`crate::handles`].
pub type OfxPropertySetHandle = *mut c_void;

/// `OfxImageEffectHandle` — a described effect, or an instance of one.
pub type OfxImageEffectHandle = *mut c_void;
/// `OfxParamSetHandle` — the bag of parameters hanging off an effect.
pub type OfxParamSetHandle = *mut c_void;
/// `OfxParamHandle` — one parameter.
pub type OfxParamHandle = *mut c_void;
/// `OfxImageClipHandle` — one image input or output.
pub type OfxImageClipHandle = *mut c_void;
/// `OfxImageMemoryHandle` — a block of image memory the host owns.
pub type OfxImageMemoryHandle = *mut c_void;

/// `OfxTime` — a frame number, as a decimal so a plugin can ask between two.
pub type OfxTime = f64;

/// `OfxRectD`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OfxRectD {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// `OfxRangeD`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OfxRangeD {
    pub min: f64,
    pub max: f64,
}

/// The API name a plugin must declare to be an image effect.
pub const K_OFX_IMAGE_EFFECT_PLUGIN_API: &str = "OfxImageEffectPluginAPI";
/// The version of that API this host implements (spec 1.4 semantics).
pub const K_OFX_IMAGE_EFFECT_PLUGIN_API_VERSION: c_int = 1;

/// The two exports every bundle binary must provide.
pub const K_OFX_GET_NUMBER_OF_PLUGINS: &[u8] = b"OfxGetNumberOfPlugins\0";
/// See [`K_OFX_GET_NUMBER_OF_PLUGINS`].
pub const K_OFX_GET_PLUGIN: &[u8] = b"OfxGetPlugin\0";

/// `OfxGetNumberOfPlugins`.
pub type OfxGetNumberOfPluginsFn = unsafe extern "C" fn() -> c_int;
/// `OfxGetPlugin`.
pub type OfxGetPluginFn = unsafe extern "C" fn(c_int) -> *const OfxPlugin;

/// `OfxHost` — what we hand the plugin: a property set describing us, and the
/// one function it uses to ask for everything else.
#[repr(C)]
pub struct OfxHost {
    /// The host's own property set (see [`crate::host`]).
    pub host: OfxPropertySetHandle,
    /// Returns a pointer to a suite struct, or null if this host does not have
    /// that suite at that version. Null is a legitimate answer and plugins are
    /// required to cope with it.
    pub fetch_suite: Option<
        unsafe extern "C" fn(
            host: OfxPropertySetHandle,
            suite_name: *const c_char,
            suite_version: c_int,
        ) -> *const c_void,
    >,
}

/// `OfxPlugin` — what the plugin hands us.
#[repr(C)]
pub struct OfxPlugin {
    /// `kOfxImageEffectPluginApi` for the plugins this host cares about.
    pub plugin_api: *const c_char,
    /// The version of that API the plugin was written against.
    pub api_version: c_int,
    /// The reverse-domain identifier, e.g. `net.sf.openfx.invertPlugin`.
    pub plugin_identifier: *const c_char,
    /// Plugin version, used by the quirks table and by project reload.
    pub plugin_version_major: c_uint,
    /// See [`Self::plugin_version_major`].
    pub plugin_version_minor: c_uint,
    /// Called once, **before anything else** (docs/impl/ofx-host.md §1).
    pub set_host: Option<unsafe extern "C" fn(host: *const OfxHost)>,
    /// Every action goes through here.
    pub main_entry: Option<
        unsafe extern "C" fn(
            action: *const c_char,
            handle: *const c_void,
            in_args: OfxPropertySetHandle,
            out_args: OfxPropertySetHandle,
        ) -> OfxStatus,
    >,
}

/// `OfxPropertySuiteV1` — the load-bearing suite: eighteen entry points, in
/// the order the header declares them. The order **is** the ABI.
#[repr(C)]
pub struct OfxPropertySuiteV1 {
    pub prop_set_pointer: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: *mut c_void,
    ) -> OfxStatus,
    pub prop_set_string: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: *const c_char,
    ) -> OfxStatus,
    pub prop_set_double: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: f64,
    ) -> OfxStatus,
    pub prop_set_int: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: c_int,
    ) -> OfxStatus,
    pub prop_set_pointer_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *const *mut c_void,
    ) -> OfxStatus,
    pub prop_set_string_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *const *const c_char,
    ) -> OfxStatus,
    pub prop_set_double_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *const f64,
    ) -> OfxStatus,
    pub prop_set_int_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *const c_int,
    ) -> OfxStatus,
    pub prop_get_pointer: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: *mut *mut c_void,
    ) -> OfxStatus,
    pub prop_get_string: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: *mut *mut c_char,
    ) -> OfxStatus,
    pub prop_get_double: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: *mut f64,
    ) -> OfxStatus,
    pub prop_get_int: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        index: c_int,
        value: *mut c_int,
    ) -> OfxStatus,
    pub prop_get_pointer_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *mut *mut c_void,
    ) -> OfxStatus,
    pub prop_get_string_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *mut *mut c_char,
    ) -> OfxStatus,
    pub prop_get_double_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *mut f64,
    ) -> OfxStatus,
    pub prop_get_int_n: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: c_int,
        value: *mut c_int,
    ) -> OfxStatus,
    pub prop_reset: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
    ) -> OfxStatus,
    pub prop_get_dimension: unsafe extern "C" fn(
        properties: OfxPropertySetHandle,
        property: *const c_char,
        count: *mut c_int,
    ) -> OfxStatus,
}

/// `OfxMemorySuiteV1`.
#[repr(C)]
pub struct OfxMemorySuiteV1 {
    pub memory_alloc: unsafe extern "C" fn(
        handle: *mut c_void,
        n_bytes: usize,
        allocated_data: *mut *mut c_void,
    ) -> OfxStatus,
    pub memory_free: unsafe extern "C" fn(allocated_data: *mut c_void) -> OfxStatus,
}

/// `OfxMessageSuiteV1`.
///
/// The C declaration is variadic (`printf`-style trailing arguments after the
/// format string). Rust cannot *define* a C-variadic function on stable, and
/// the fixed part of the call is ABI-identical on every platform this host
/// targets, so the pointer is declared with the four fixed arguments only and
/// the format string is taken verbatim. Substituting the trailing arguments
/// needs a `printf` parser and lands with the out-of-process broker, which is
/// where messages become something a person sees.
#[repr(C)]
pub struct OfxMessageSuiteV1 {
    pub message: unsafe extern "C" fn(
        handle: *mut c_void,
        message_type: *const c_char,
        message_id: *const c_char,
        format: *const c_char,
    ) -> OfxStatus,
}

/// `OfxImageEffectSuiteV1` — thirteen entry points, in the order the header
/// declares them. The order **is** the ABI.
///
/// This package builds the *definition* half only: the three a plugin uses
/// while it is describing itself. Everything from `clip_get_handle` onwards
/// belongs to an instance, and no instance exists yet, so each of those answers
/// `kOfxStatErrUnsupported` — a code the spec already requires plugins to cope
/// with, and an honest one, because the feature genuinely is not here.
#[repr(C)]
pub struct OfxImageEffectSuiteV1 {
    pub get_property_set: unsafe extern "C" fn(
        image_effect: OfxImageEffectHandle,
        prop_handle: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub get_param_set: unsafe extern "C" fn(
        image_effect: OfxImageEffectHandle,
        param_set: *mut OfxParamSetHandle,
    ) -> OfxStatus,
    pub clip_define: unsafe extern "C" fn(
        image_effect: OfxImageEffectHandle,
        name: *const c_char,
        property_set: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub clip_get_handle: unsafe extern "C" fn(
        image_effect: OfxImageEffectHandle,
        name: *const c_char,
        clip: *mut OfxImageClipHandle,
        property_set: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub clip_get_property_set: unsafe extern "C" fn(
        clip: OfxImageClipHandle,
        prop_handle: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub clip_get_image: unsafe extern "C" fn(
        clip: OfxImageClipHandle,
        time: OfxTime,
        region: *const OfxRectD,
        image_handle: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub clip_release_image: unsafe extern "C" fn(image_handle: OfxPropertySetHandle) -> OfxStatus,
    pub clip_get_region_of_definition: unsafe extern "C" fn(
        clip: OfxImageClipHandle,
        time: OfxTime,
        bounds: *mut OfxRectD,
    ) -> OfxStatus,
    /// The one entry point that is not a status: nought means carry on.
    pub abort: unsafe extern "C" fn(image_effect: OfxImageEffectHandle) -> c_int,
    pub image_memory_alloc: unsafe extern "C" fn(
        instance_handle: OfxImageEffectHandle,
        n_bytes: usize,
        memory_handle: *mut OfxImageMemoryHandle,
    ) -> OfxStatus,
    pub image_memory_free: unsafe extern "C" fn(memory_handle: OfxImageMemoryHandle) -> OfxStatus,
    pub image_memory_lock: unsafe extern "C" fn(
        memory_handle: OfxImageMemoryHandle,
        returned_ptr: *mut *mut c_void,
    ) -> OfxStatus,
    pub image_memory_unlock: unsafe extern "C" fn(memory_handle: OfxImageMemoryHandle) -> OfxStatus,
}

/// `OfxParameterSuiteV1` — eighteen entry points, in header order.
///
/// Eight of them are **C-variadic** in the header (`paramGetValue` and its
/// relatives take the out-parameters as trailing arguments, one per dimension).
/// Rust cannot define a C-variadic function on stable, and the same argument
/// [`OfxMessageSuiteV1`] records applies: the fixed prefix is ABI-identical on
/// every platform this host targets, and none of the eight reads a trailing
/// argument in this package — they answer `kOfxStatErrUnsupported`, because
/// nothing has a value until an instance exists (P3).
#[repr(C)]
pub struct OfxParameterSuiteV1 {
    pub param_define: unsafe extern "C" fn(
        param_set: OfxParamSetHandle,
        param_type: *const c_char,
        name: *const c_char,
        property_set: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub param_get_handle: unsafe extern "C" fn(
        param_set: OfxParamSetHandle,
        name: *const c_char,
        param: *mut OfxParamHandle,
        property_set: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub param_set_get_property_set: unsafe extern "C" fn(
        param_set: OfxParamSetHandle,
        prop_handle: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub param_get_property_set: unsafe extern "C" fn(
        param: OfxParamHandle,
        prop_handle: *mut OfxPropertySetHandle,
    ) -> OfxStatus,
    pub param_get_value: unsafe extern "C" fn(param: OfxParamHandle) -> OfxStatus,
    pub param_get_value_at_time:
        unsafe extern "C" fn(param: OfxParamHandle, time: OfxTime) -> OfxStatus,
    pub param_get_derivative:
        unsafe extern "C" fn(param: OfxParamHandle, time: OfxTime) -> OfxStatus,
    pub param_get_integral:
        unsafe extern "C" fn(param: OfxParamHandle, time1: OfxTime, time2: OfxTime) -> OfxStatus,
    pub param_set_value: unsafe extern "C" fn(param: OfxParamHandle) -> OfxStatus,
    pub param_set_value_at_time:
        unsafe extern "C" fn(param: OfxParamHandle, time: OfxTime) -> OfxStatus,
    pub param_get_num_keys:
        unsafe extern "C" fn(param: OfxParamHandle, number_of_keys: *mut c_uint) -> OfxStatus,
    pub param_get_key_time: unsafe extern "C" fn(
        param: OfxParamHandle,
        nth_key: c_uint,
        time: *mut OfxTime,
    ) -> OfxStatus,
    pub param_get_key_index: unsafe extern "C" fn(
        param: OfxParamHandle,
        time: OfxTime,
        direction: c_int,
        index: *mut c_int,
    ) -> OfxStatus,
    pub param_delete_key: unsafe extern "C" fn(param: OfxParamHandle, time: OfxTime) -> OfxStatus,
    pub param_delete_all_keys: unsafe extern "C" fn(param: OfxParamHandle) -> OfxStatus,
    pub param_copy: unsafe extern "C" fn(
        param_to: OfxParamHandle,
        param_from: OfxParamHandle,
        dst_offset: OfxTime,
        frame_range: *const OfxRangeD,
    ) -> OfxStatus,
    pub param_edit_begin:
        unsafe extern "C" fn(param_set: OfxParamSetHandle, name: *const c_char) -> OfxStatus,
    pub param_edit_end: unsafe extern "C" fn(param_set: OfxParamSetHandle) -> OfxStatus,
}

/// Suite names, as `fetchSuite` spells them.
pub mod suite_names {
    /// `kOfxPropertySuite`
    pub const PROPERTY: &str = "OfxPropertySuite";
    /// `kOfxMemorySuite`
    pub const MEMORY: &str = "OfxMemorySuite";
    /// `kOfxMessageSuite`
    pub const MESSAGE: &str = "OfxMessageSuite";
    /// `kOfxImageEffectSuite` — the definition half only (see
    /// [`OfxImageEffectSuiteV1`]).
    pub const IMAGE_EFFECT: &str = "OfxImageEffectSuite";
    /// `kOfxParameterSuite` — the definition half only (see
    /// [`OfxParameterSuiteV1`]).
    pub const PARAMETER: &str = "OfxParameterSuite";
    /// `kOfxMultiThreadSuite` — not implemented in this package.
    pub const MULTI_THREAD: &str = "OfxMultiThreadSuite";
    /// `kOfxInteractSuite` — deliberately never fetched successfully in v1;
    /// overlays degrade to no overlay (docs/impl/ofx-host.md §2).
    pub const INTERACT: &str = "OfxInteractSuite";
}

/// The action names this package dispatches.
pub mod actions {
    /// `kOfxActionLoad`
    pub const LOAD: &str = "OfxActionLoad";
    /// `kOfxActionUnload`
    pub const UNLOAD: &str = "OfxActionUnload";
    /// `kOfxActionDescribe`
    pub const DESCRIBE: &str = "OfxActionDescribe";
    /// `kOfxImageEffectActionDescribeInContext`
    pub const DESCRIBE_IN_CONTEXT: &str = "OfxImageEffectActionDescribeInContext";
    /// `kOfxImageEffectActionRender`
    pub const RENDER: &str = "OfxImageEffectActionRender";
    /// `kOfxActionCreateInstance`
    pub const CREATE_INSTANCE: &str = "OfxActionCreateInstance";
    /// `kOfxActionDestroyInstance`
    pub const DESTROY_INSTANCE: &str = "OfxActionDestroyInstance";
}

/// The property keys this package uses, spelled exactly as the C headers do.
/// A typo here is a property the plugin never finds, so they are written once.
pub mod prop_keys {
    pub const TYPE: &str = "OfxPropType";
    pub const NAME: &str = "OfxPropName";
    pub const LABEL: &str = "OfxPropLabel";
    pub const VERSION: &str = "OfxPropVersion";
    pub const VERSION_LABEL: &str = "OfxPropVersionLabel";
    pub const API_VERSION: &str = "OfxPropAPIVersion";

    pub const HOST_IS_BACKGROUND: &str = "OfxImageEffectHostPropIsBackground";
    pub const SUPPORTS_OVERLAYS: &str = "OfxImageEffectPropSupportsOverlays";
    pub const SUPPORTS_MULTI_RESOLUTION: &str = "OfxImageEffectPropSupportsMultiResolution";
    pub const SUPPORTS_TILES: &str = "OfxImageEffectPropSupportsTiles";
    pub const TEMPORAL_CLIP_ACCESS: &str = "OfxImageEffectPropTemporalClipAccess";
    pub const SUPPORTED_COMPONENTS: &str = "OfxImageEffectPropSupportedComponents";
    pub const SUPPORTED_CONTEXTS: &str = "OfxImageEffectPropSupportedContexts";
    pub const SUPPORTED_PIXEL_DEPTHS: &str = "OfxImageEffectPropSupportedPixelDepths";
    pub const SUPPORTS_MULTIPLE_CLIP_DEPTHS: &str = "OfxImageEffectPropSupportsMultipleClipDepths";
    pub const SUPPORTS_MULTIPLE_CLIP_PARS: &str = "OfxImageEffectPropSupportsMultipleClipPARs";
    pub const SETABLE_FRAME_RATE: &str = "OfxImageEffectPropSetableFrameRate";
    pub const SETABLE_FIELDING: &str = "OfxImageEffectPropSetableFielding";
    pub const SEQUENTIAL_RENDER: &str = "OfxImageEffectInstancePropSequentialRender";
    pub const CONTEXT: &str = "OfxImageEffectPropContext";
    pub const PLUGIN_RENDER_THREAD_SAFETY: &str = "OfxImageEffectPluginRenderThreadSafety";
    pub const GROUPING: &str = "OfxImageEffectPluginPropGrouping";

    pub const PARAM_SUPPORTS_STRING_ANIMATION: &str = "OfxParamHostPropSupportsStringAnimation";
    pub const PARAM_SUPPORTS_CUSTOM_INTERACT: &str = "OfxParamHostPropSupportsCustomInteract";
    pub const PARAM_SUPPORTS_CHOICE_ANIMATION: &str = "OfxParamHostPropSupportsChoiceAnimation";
    pub const PARAM_SUPPORTS_BOOLEAN_ANIMATION: &str = "OfxParamHostPropSupportsBooleanAnimation";
    pub const PARAM_SUPPORTS_CUSTOM_ANIMATION: &str = "OfxParamHostPropSupportsCustomAnimation";
    pub const PARAM_SUPPORTS_PARAMETRIC_ANIMATION: &str =
        "OfxParamHostPropSupportsParametricAnimation";
    pub const PARAM_MAX_PARAMETERS: &str = "OfxParamHostPropMaxParameters";
    pub const PARAM_MAX_PAGES: &str = "OfxParamHostPropMaxPages";
    pub const PARAM_PAGE_ROW_COLUMN_COUNT: &str = "OfxParamHostPropPageRowColumnCount";

    pub const SHORT_LABEL: &str = "OfxPropShortLabel";
    pub const LONG_LABEL: &str = "OfxPropLongLabel";

    pub const PARAM_TYPE: &str = "OfxParamPropType";
    pub const PARAM_DEFAULT: &str = "OfxParamPropDefault";
    pub const PARAM_MIN: &str = "OfxParamPropMin";
    pub const PARAM_MAX: &str = "OfxParamPropMax";
    pub const PARAM_DISPLAY_MIN: &str = "OfxParamPropDisplayMin";
    pub const PARAM_DISPLAY_MAX: &str = "OfxParamPropDisplayMax";
    pub const PARAM_DOUBLE_TYPE: &str = "OfxParamPropDoubleType";
    pub const PARAM_STRING_MODE: &str = "OfxParamPropStringMode";
    pub const PARAM_CHOICE_OPTION: &str = "OfxParamPropChoiceOption";
    pub const PARAM_PARENT: &str = "OfxParamPropParent";
    pub const PARAM_PAGE_CHILD: &str = "OfxParamPropPageChild";
    pub const PARAM_HINT: &str = "OfxParamPropHint";
    pub const PARAM_ANIMATES: &str = "OfxParamPropAnimates";
    pub const PARAM_GROUP_OPEN: &str = "OfxParamPropGroupOpen";

    pub const CLIP_OPTIONAL: &str = "OfxImageClipPropOptional";
    pub const CLIP_IS_MASK: &str = "OfxImageClipPropIsMask";
}

/// The parameter types `paramDefine` accepts — every standard type, spelled as
/// the C headers spell them.
pub mod param_types {
    pub const INTEGER: &str = "OfxParamTypeInteger";
    pub const DOUBLE: &str = "OfxParamTypeDouble";
    pub const BOOLEAN: &str = "OfxParamTypeBoolean";
    pub const CHOICE: &str = "OfxParamTypeChoice";
    pub const RGBA: &str = "OfxParamTypeRGBA";
    pub const RGB: &str = "OfxParamTypeRGB";
    pub const DOUBLE_2D: &str = "OfxParamTypeDouble2D";
    pub const INTEGER_2D: &str = "OfxParamTypeInteger2D";
    pub const DOUBLE_3D: &str = "OfxParamTypeDouble3D";
    pub const INTEGER_3D: &str = "OfxParamTypeInteger3D";
    pub const STRING: &str = "OfxParamTypeString";
    pub const CUSTOM: &str = "OfxParamTypeCustom";
    pub const GROUP: &str = "OfxParamTypeGroup";
    pub const PAGE: &str = "OfxParamTypePage";
    pub const PUSH_BUTTON: &str = "OfxParamTypePushButton";
    /// A curve a plugin evaluates itself. Accepted by `paramDefine` — refusing
    /// a type the spec defines is how a plugin fails to describe at all — but
    /// it has no schema row, because Lumit's own curve is its *control points*
    /// (K-412) and a parametric parameter is a function, not points.
    pub const PARAMETRIC: &str = "OfxParamTypeParametric";

    /// Every type above, for the "is this a type at all" check.
    pub const ALL: &[&str] = &[
        INTEGER,
        DOUBLE,
        BOOLEAN,
        CHOICE,
        RGBA,
        RGB,
        DOUBLE_2D,
        INTEGER_2D,
        DOUBLE_3D,
        INTEGER_3D,
        STRING,
        CUSTOM,
        GROUP,
        PAGE,
        PUSH_BUTTON,
        PARAMETRIC,
    ];
}

/// `kOfxParamPropDoubleType` — what a double *means*, which is the only thing
/// that says whether it is a distance (docs/impl/effect-registry.md §2.2).
pub mod double_types {
    pub const PLAIN: &str = "OfxParamDoubleTypePlain";
    pub const ANGLE: &str = "OfxParamDoubleTypeAngle";
    pub const SCALE: &str = "OfxParamDoubleTypeScale";
    pub const TIME: &str = "OfxParamDoubleTypeTime";
    pub const ABSOLUTE_TIME: &str = "OfxParamDoubleTypeAbsoluteTime";
    pub const X: &str = "OfxParamDoubleTypeX";
    pub const X_ABSOLUTE: &str = "OfxParamDoubleTypeXAbsolute";
    pub const Y: &str = "OfxParamDoubleTypeY";
    pub const Y_ABSOLUTE: &str = "OfxParamDoubleTypeYAbsolute";
    pub const XY: &str = "OfxParamDoubleTypeXY";
    pub const XY_ABSOLUTE: &str = "OfxParamDoubleTypeXYAbsolute";
}

/// `kOfxParamPropStringMode`.
pub mod string_modes {
    pub const SINGLE_LINE: &str = "OfxParamStringIsSingleLine";
    pub const MULTI_LINE: &str = "OfxParamStringIsMultiLine";
    pub const FILE_PATH: &str = "OfxParamStringIsFilePath";
    pub const DIRECTORY_PATH: &str = "OfxParamStringIsDirectoryPath";
    pub const LABEL: &str = "OfxParamStringIsLabel";
}

/// The property *values* that are themselves fixed strings.
pub mod prop_values {
    pub const TYPE_IMAGE_EFFECT_HOST: &str = "OfxTypeImageEffectHost";
    pub const COMPONENT_RGBA: &str = "OfxImageComponentRGBA";
    pub const BIT_DEPTH_FLOAT: &str = "OfxBitDepthFloat";
    pub const CONTEXT_FILTER: &str = "OfxImageEffectContextFilter";
    pub const CONTEXT_GENERAL: &str = "OfxImageEffectContextGeneral";
    pub const CONTEXT_GENERATOR: &str = "OfxImageEffectContextGenerator";
    pub const CONTEXT_TRANSITION: &str = "OfxImageEffectContextTransition";
    /// The retimer context, which docs/12 §2.1 defers: real retimers ship as
    /// filter or general effects.
    pub const CONTEXT_RETIMER: &str = "OfxImageEffectContextRetimer";
    pub const CONTEXT_PAINT: &str = "OfxImageEffectContextPaint";
    pub const RENDER_THREAD_SAFETY_FULLY_SAFE: &str = "OfxImageEffectRenderFullySafe";
    pub const TYPE_IMAGE_EFFECT: &str = "OfxTypeImageEffect";
    pub const TYPE_PARAMETER: &str = "OfxTypeParameter";
    pub const TYPE_CLIP: &str = "OfxTypeClip";
}

/// The message types of `OfxMessageSuiteV1`.
pub mod message_types {
    pub const FATAL: &str = "OfxMessageFatal";
    pub const ERROR: &str = "OfxMessageError";
    pub const WARNING: &str = "OfxMessageWarning";
    pub const MESSAGE: &str = "OfxMessageMessage";
    pub const LOG: &str = "OfxMessageLog";
    pub const QUESTION: &str = "OfxMessageQuestion";
}
