//! The host's own tests.
//!
//! The handle tests are the ones that matter most: they are the seed corpus
//! for the sanitiser run that comes with the out-of-process broker
//! (docs/impl/ofx-host.md §5 item 2). Every one of them hands a suite entry
//! point a handle it was never given and checks that the answer is a status
//! code and that nothing moved.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};

use lumit_core::fx::EffectSchema;

use half::f16;
use lumit_eval::epoch::Epoch;

use crate::bundle::{scan_dir, Bundle, BUNDLE_ARCH_DIR};
use crate::describe::{describe_bundle, Context, DescribedPlugin, Rejection, ScanReport};
use crate::ffi::{
    actions, prop_keys as keys, prop_values as values, OfxHost, OfxImageEffectSuiteV1,
    OfxMemorySuiteV1, OfxMessageSuiteV1, OfxMultiThreadSuiteV1, OfxParameterSuiteV1, OfxPlugin,
    OfxPropertySuiteV1,
};
use crate::handles::{Handle, HandleKind, HandleRegistry};
use crate::host::{dump, host, host_props_handle, state};
use crate::image::{Frame16, Image, RowOrder};
use crate::instance::{Instance, ParamSnapshot, ThreadSafety};
use crate::props::{Element, PropValue, PropertySet};
use crate::quirks::QuirksTable;
use crate::render::{RenderError, RenderRequest, Rendered};
use crate::status::Status;
use crate::suites::{memory, message, property};

// ---------------------------------------------------------------- handles --

#[test]
fn a_handle_carries_its_kind_and_index_back() {
    let handle = Handle::encode(HandleKind::Param, 7);
    let handle = handle.expect("an index of seven fits");
    assert_eq!(handle.kind(), Some(HandleKind::Param));
    assert_eq!(Handle::from_ptr(handle.as_ptr()), handle);
}

#[test]
fn a_number_that_is_not_a_handle_has_no_kind() {
    assert_eq!(Handle::from_ptr(std::ptr::null()).kind(), None);
    assert_eq!(Handle::from_ptr(0xdead_beef as *const c_void).kind(), None);
}

#[test]
fn a_stale_handle_never_names_a_later_object() {
    let mut registry = HandleRegistry::new(HandleKind::PropertySet);
    let first = registry.insert(PropertySet::new()).expect("room for one");
    registry.remove(first).expect("it was just inserted");
    let second = registry.insert(PropertySet::new()).expect("room for two");

    assert_ne!(first, second, "an index must never be issued twice");
    assert_eq!(registry.get(first).err(), Some(Status::ErrBadHandle));
    assert!(registry.get(second).is_ok());
}

#[test]
fn a_handle_of_the_wrong_kind_is_refused() {
    let mut registry = HandleRegistry::new(HandleKind::PropertySet);
    let handle = registry.insert(PropertySet::new()).expect("room for one");
    let wrong = Handle::encode(HandleKind::Clip, 0).expect("index nought fits");

    assert!(registry.get(handle).is_ok());
    assert_eq!(registry.get(wrong).err(), Some(Status::ErrBadHandle));
}

// ------------------------------------------------------------- properties --

#[test]
fn a_double_property_read_as_an_int_is_a_value_error() {
    let mut set = PropertySet::new();
    set.seed("OfxTestDouble", PropValue::double(2.5));

    assert_eq!(set.get_double("OfxTestDouble", 0), Ok(2.5));
    assert_eq!(set.get_int("OfxTestDouble", 0), Err(Status::ErrValue));
    assert_eq!(set.get_pointer("OfxTestDouble", 0), Err(Status::ErrValue));
    assert!(set.get_string("OfxTestDouble", 0).is_err());
}

#[test]
fn a_property_does_not_change_type_when_it_is_written() {
    let mut set = PropertySet::new();
    set.seed("OfxTestInt", PropValue::int(4));

    assert_eq!(
        set.set_element("OfxTestInt", 0, Element::Double(1.0)),
        Err(Status::ErrValue)
    );
    assert_eq!(set.get_int("OfxTestInt", 0), Ok(4));
}

#[test]
fn writing_at_the_end_extends_and_writing_past_it_does_not() {
    let mut set = PropertySet::new();
    set.seed("OfxTestInt", PropValue::int(1));

    assert_eq!(set.set_element("OfxTestInt", 1, Element::Int(2)), Ok(()));
    assert_eq!(set.dimension("OfxTestInt"), Ok(2));
    assert_eq!(
        set.set_element("OfxTestInt", 5, Element::Int(3)),
        Err(Status::ErrBadIndex)
    );
    assert_eq!(set.dimension("OfxTestInt"), Ok(2));
}

#[test]
fn reset_puts_back_what_the_host_seeded() {
    let mut set = PropertySet::new();
    set.seed("OfxTestInt", PropValue::int(1));
    assert_eq!(set.set_element("OfxTestInt", 0, Element::Int(9)), Ok(()));
    assert_eq!(set.reset("OfxTestInt"), Ok(()));
    assert_eq!(set.get_int("OfxTestInt", 0), Ok(1));
    assert_eq!(set.reset("OfxTestMissing"), Err(Status::ErrUnknown));
}

// ------------------------------------------------- the property suite, C --

/// Make a property set the suite can be called against, and return its handle
/// as the C API carries it.
fn a_live_property_set() -> *mut c_void {
    let mut set = PropertySet::new();
    set.seed("OfxTestInt", PropValue::int(11));
    set.seed("OfxTestDouble", PropValue::double(0.5));
    if let Ok(text) = PropValue::string("hello") {
        set.seed("OfxTestString", text);
    }
    set.seed("OfxTestPointer", PropValue::Pointer(vec![0]));
    let mut state = state();
    match state.props.insert(set) {
        Ok(handle) => handle.as_ptr(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Call every entry point of the property suite once, with arguments that are
/// valid in themselves, and report what each answered.
fn call_every_property_entry(handle: *mut c_void) -> Vec<(&'static str, c_int)> {
    let suite = &property::SUITE;
    let key = c"OfxTestInt".as_ptr();

    let mut int_out: c_int = 0;
    let mut double_out: f64 = 0.0;
    let mut string_out: *mut c_char = std::ptr::null_mut();
    let mut pointer_out: *mut c_void = std::ptr::null_mut();

    let ints = [1_i32];
    let doubles = [1.0_f64];
    let strings = [c"x".as_ptr()];
    let pointers = [std::ptr::null_mut::<c_void>()];

    // SAFETY: every argument here is a valid pointer to storage that outlives
    // the call; the handle is the only thing under test, and the suite is
    // required to reject a bad one without following it.
    unsafe {
        vec![
            (
                "propSetPointer",
                (suite.prop_set_pointer)(handle, key, 0, std::ptr::null_mut()),
            ),
            (
                "propSetString",
                (suite.prop_set_string)(handle, key, 0, c"x".as_ptr()),
            ),
            (
                "propSetDouble",
                (suite.prop_set_double)(handle, key, 0, 1.0),
            ),
            ("propSetInt", (suite.prop_set_int)(handle, key, 0, 1)),
            (
                "propSetPointerN",
                (suite.prop_set_pointer_n)(handle, key, 1, pointers.as_ptr()),
            ),
            (
                "propSetStringN",
                (suite.prop_set_string_n)(handle, key, 1, strings.as_ptr()),
            ),
            (
                "propSetDoubleN",
                (suite.prop_set_double_n)(handle, key, 1, doubles.as_ptr()),
            ),
            (
                "propSetIntN",
                (suite.prop_set_int_n)(handle, key, 1, ints.as_ptr()),
            ),
            (
                "propGetPointer",
                (suite.prop_get_pointer)(handle, key, 0, &raw mut pointer_out),
            ),
            (
                "propGetString",
                (suite.prop_get_string)(handle, key, 0, &raw mut string_out),
            ),
            (
                "propGetDouble",
                (suite.prop_get_double)(handle, key, 0, &raw mut double_out),
            ),
            (
                "propGetInt",
                (suite.prop_get_int)(handle, key, 0, &raw mut int_out),
            ),
            (
                "propGetPointerN",
                (suite.prop_get_pointer_n)(handle, key, 1, &raw mut pointer_out),
            ),
            (
                "propGetStringN",
                (suite.prop_get_string_n)(handle, key, 1, &raw mut string_out),
            ),
            (
                "propGetDoubleN",
                (suite.prop_get_double_n)(handle, key, 1, &raw mut double_out),
            ),
            (
                "propGetIntN",
                (suite.prop_get_int_n)(handle, key, 1, &raw mut int_out),
            ),
            ("propReset", (suite.prop_reset)(handle, key)),
            (
                "propGetDimension",
                (suite.prop_get_dimension)(handle, key, &raw mut int_out),
            ),
        ]
    }
}

#[test]
fn every_property_entry_point_refuses_a_forged_handle() {
    // The set that must be untouched afterwards.
    let live = a_live_property_set();
    let live_handle = Handle::from_ptr(live);

    let stale = {
        let mut state = state();
        let handle = state
            .props
            .insert(PropertySet::new())
            .expect("room for one more");
        state.props.remove(handle).expect("it was just inserted");
        handle
    };

    let forged = [
        ("null", std::ptr::null_mut::<c_void>()),
        ("garbage", 0xdead_beef_usize as *mut c_void),
        ("stale", stale.as_ptr()),
        (
            "wrong kind",
            Handle::encode(HandleKind::Clip, 0)
                .expect("index nought fits")
                .as_ptr(),
        ),
        (
            "past the end",
            Handle::encode(HandleKind::PropertySet, 1 << 30)
                .expect("the index fits in the field")
                .as_ptr(),
        ),
    ];

    for (name, handle) in forged {
        for (entry, status) in call_every_property_entry(handle) {
            assert_eq!(
                status,
                Status::ErrBadHandle.code(),
                "{entry} accepted a {name} handle"
            );
        }
    }

    // Nothing the forged calls did may have reached a real property set.
    let state = state();
    let set = state.props.get(live_handle).expect("the live set is live");
    assert_eq!(set.get_int("OfxTestInt", 0), Ok(11));
    assert_eq!(set.dimension("OfxTestInt"), Ok(1));
}

#[test]
fn the_property_suite_reads_and_writes_through_c() {
    let handle = a_live_property_set();
    let suite = &property::SUITE;
    let mut int_out: c_int = 0;
    let mut string_out: *mut c_char = std::ptr::null_mut();

    // SAFETY: a live handle and valid out-parameters.
    unsafe {
        assert_eq!(
            (suite.prop_set_int)(handle, c"OfxTestInt".as_ptr(), 0, 42),
            Status::Ok.code()
        );
        assert_eq!(
            (suite.prop_get_int)(handle, c"OfxTestInt".as_ptr(), 0, &raw mut int_out),
            Status::Ok.code()
        );
        assert_eq!(int_out, 42);

        // A double read as an int is a value error, through C as in Rust.
        assert_eq!(
            (suite.prop_get_int)(handle, c"OfxTestDouble".as_ptr(), 0, &raw mut int_out),
            Status::ErrValue.code()
        );

        // A name the set does not have.
        assert_eq!(
            (suite.prop_get_int)(handle, c"OfxTestAbsent".as_ptr(), 0, &raw mut int_out),
            Status::ErrUnknown.code()
        );

        // A null property name, and a null out-parameter.
        assert_eq!(
            (suite.prop_get_int)(handle, std::ptr::null(), 0, &raw mut int_out),
            Status::ErrValue.code()
        );
        assert_eq!(
            (suite.prop_get_int)(handle, c"OfxTestInt".as_ptr(), 0, std::ptr::null_mut()),
            Status::ErrValue.code()
        );

        // A negative index is not an index.
        assert_eq!(
            (suite.prop_get_int)(handle, c"OfxTestInt".as_ptr(), -1, &raw mut int_out),
            Status::ErrBadIndex.code()
        );

        assert_eq!(
            (suite.prop_get_string)(handle, c"OfxTestString".as_ptr(), 0, &raw mut string_out),
            Status::Ok.code()
        );
        assert!(!string_out.is_null());
        assert_eq!(CStr::from_ptr(string_out).to_bytes(), b"hello");
    }
}

// ------------------------------------------------------------ the host --

#[test]
fn the_host_property_table_says_only_true_things() {
    let _name = host_name_lock();
    let handle = host_props_handle().expect("the host has its own property set");
    let state = state();
    let set = state.props.get(handle).expect("the host set is live");

    let expected = format!(
        "\
OfxImageEffectHostPropIsBackground = 0
OfxImageEffectHostPropNativeOrigin = \"OfxImageEffectHostPropNativeOriginBottomLeft\"
OfxImageEffectInstancePropSequentialRender = 0
OfxImageEffectPropCudaRenderSupported = \"false\"
OfxImageEffectPropCudaStreamSupported = \"false\"
OfxImageEffectPropMetalRenderSupported = \"false\"
OfxImageEffectPropMultipleClipDepths = 0
OfxImageEffectPropOpenCLRenderSupported = \"false\"
OfxImageEffectPropOpenGLRenderSupported = \"false\"
OfxImageEffectPropRenderQualityDraft = 0
OfxImageEffectPropSetableFielding = 0
OfxImageEffectPropSetableFrameRate = 0
OfxImageEffectPropSupportedComponents = \"OfxImageComponentRGBA\"
OfxImageEffectPropSupportedContexts = \"OfxImageEffectContextFilter\", \
\"OfxImageEffectContextGeneral\", \"OfxImageEffectContextGenerator\", \
\"OfxImageEffectContextTransition\"
OfxImageEffectPropSupportedPixelDepths = \"OfxBitDepthFloat\"
OfxImageEffectPropSupportsMultiResolution = 0
OfxImageEffectPropSupportsMultipleClipPARs = 0
OfxImageEffectPropSupportsOverlays = 0
OfxImageEffectPropSupportsTiles = 0
OfxImageEffectPropTemporalClipAccess = 1
OfxParamHostPropMaxPages = 0
OfxParamHostPropMaxParameters = -1
OfxParamHostPropPageRowColumnCount = 0, 0
OfxParamHostPropSupportsBooleanAnimation = 1
OfxParamHostPropSupportsChoiceAnimation = 0
OfxParamHostPropSupportsCustomAnimation = 0
OfxParamHostPropSupportsCustomInteract = 0
OfxParamHostPropSupportsParametricAnimation = 0
OfxParamHostPropSupportsStrChoiceAnimation = 0
OfxParamHostPropSupportsStringAnimation = 0
OfxPropAPIVersion = 1, 4
OfxPropHostOSHandle = 0x0
OfxPropLabel = \"Lumit\"
OfxPropName = \"com.lumitlab.Lumit\"
OfxPropType = \"OfxTypeImageEffectHost\"
OfxPropVersion = {version_numbers}
OfxPropVersionLabel = \"{version}\"
",
        version = env!("CARGO_PKG_VERSION"),
        version_numbers = env!("CARGO_PKG_VERSION").replace('.', ", "),
    );

    assert_eq!(dump(set), expected);

    // The two that would break plugins if they were ever quietly flipped.
    assert_eq!(set.get_int(keys::SUPPORTS_TILES, 0), Ok(0));
    assert_eq!(set.get_int(keys::TEMPORAL_CLIP_ACCESS, 0), Ok(1));
}

/// **Six OFX properties are not named after their macros**, and a host that
/// seeds the macro's own name puts the property where no plugin will ever look
/// for it — silently, because a property nobody finds is a property nobody
/// complains about. ntsc-rs refuses to load without the first of them; the
/// conformance bench is what found that, and this is the list it came back
/// with. Each line is `ofxImageEffect.h`'s `#define`, verbatim.
#[test]
fn the_properties_whose_names_are_not_their_macros_are_spelled_the_headers_way() {
    for (macro_name, string, ours) in [
        (
            "kOfxImageEffectPropSupportsMultipleClipDepths",
            "OfxImageEffectPropMultipleClipDepths",
            keys::SUPPORTS_MULTIPLE_CLIP_DEPTHS,
        ),
        (
            "kOfxImageEffectPropProjectPixelAspectRatio",
            "OfxImageEffectPropPixelAspectRatio",
            keys::PROJECT_PIXEL_ASPECT_RATIO,
        ),
        (
            "kOfxImageEffectPropUnmappedFrameRange",
            "OfxImageEffectPropUnmappedFrameRange",
            keys::CLIP_UNMAPPED_FRAME_RANGE,
        ),
        (
            "kOfxImagePreMultiplied",
            "OfxImageAlphaPremultiplied",
            values::IMAGE_PRE_MULTIPLIED,
        ),
        (
            "kOfxImageFieldNone",
            "OfxFieldNone",
            values::IMAGE_FIELD_NONE,
        ),
        (
            "kOfxImageFieldBoth",
            "OfxFieldBoth",
            values::IMAGE_FIELD_BOTH,
        ),
    ] {
        assert_eq!(
            ours, string,
            "{macro_name} is a macro name, not a property name"
        );
    }

    // And the host really answers to the corrected name, which is the half a
    // constant on its own cannot promise.
    // The handle is fetched **before** the state is locked: `host_props_handle`
    // takes the same lock, and this mutex is not reentrant.
    let handle = host_props_handle().expect("the host has properties");
    let state = state();
    let set = state.props.get(handle).expect("the host's set is live");
    assert_eq!(set.get_int(keys::SUPPORTS_MULTIPLE_CLIP_DEPTHS, 0), Ok(0));
}

#[test]
fn fetch_suite_answers_for_what_exists_and_null_for_what_does_not() {
    // SAFETY: the host struct is built once and lives for the process.
    let host = unsafe { &*host() };
    let fetch = host.fetch_suite.expect("the host has a fetchSuite");

    let ask = |name: &CStr, version: c_int| {
        // SAFETY: the host's own function, given a string that outlives it.
        unsafe { fetch(host.host, name.as_ptr(), version) }
    };

    assert!(!ask(c"OfxPropertySuite", 1).is_null());
    assert!(!ask(c"OfxMemorySuite", 1).is_null());
    assert!(!ask(c"OfxMessageSuite", 1).is_null());

    assert!(!ask(c"OfxImageEffectSuite", 1).is_null());
    assert!(!ask(c"OfxParameterSuite", 1).is_null());
    assert!(!ask(c"OfxMultiThreadSuite", 1).is_null());

    // Served so the stock support library will describe at all; there is
    // never an interact for it to act on. And the message suite's
    // second version, which HitFilm will not load without.
    assert!(!ask(c"OfxInteractSuite", 1).is_null());
    assert!(!ask(c"OfxMessageSuite", 2).is_null());

    // A version we do not have is a null, not the version we do have.
    assert!(ask(c"OfxPropertySuite", 2).is_null());
    assert!(ask(c"OfxPropertySuite", 0).is_null());

    // And a plugin that asks for nothing at all gets nothing at all.
    // SAFETY: as above; a null name is a case the host must survive.
    assert!(unsafe { fetch(host.host, std::ptr::null(), 1) }.is_null());
}

// ----------------------------------------------------- memory and message --

#[test]
fn the_memory_suite_gives_back_only_what_it_gave_out() {
    let suite = &memory::SUITE;
    let mut block: *mut c_void = std::ptr::null_mut();

    // SAFETY: valid out-parameter; the block is freed below.
    unsafe {
        assert_eq!(
            (suite.memory_alloc)(std::ptr::null_mut(), 1024, &raw mut block),
            Status::Ok.code()
        );
        assert!(!block.is_null());

        // A pointer nobody was given is refused without being followed.
        assert_eq!(
            (suite.memory_free)(0xdead_beef_usize as *mut c_void),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (suite.memory_free)(std::ptr::null_mut()),
            Status::ErrBadHandle.code()
        );

        assert_eq!((suite.memory_free)(block), Status::Ok.code());
        // Freeing it twice is the same forged pointer as any other.
        assert_eq!((suite.memory_free)(block), Status::ErrBadHandle.code());

        // A zero-byte request still answers with something freeable.
        assert_eq!(
            (suite.memory_alloc)(std::ptr::null_mut(), 0, &raw mut block),
            Status::Ok.code()
        );
        assert!(!block.is_null());
        assert_eq!((suite.memory_free)(block), Status::Ok.code());

        // A null out-parameter is a value error, not a leak.
        assert_eq!(
            (suite.memory_alloc)(std::ptr::null_mut(), 16, std::ptr::null_mut()),
            Status::ErrValue.code()
        );
    }
}

#[test]
fn a_question_the_host_cannot_ask_is_answered_by_default() {
    let suite = &message::SUITE;
    // SAFETY: NUL-terminated literals that outlive the call.
    unsafe {
        assert_eq!(
            (suite.message)(
                std::ptr::null_mut(),
                c"OfxMessageQuestion".as_ptr(),
                c"test".as_ptr(),
                c"is this a question?".as_ptr(),
            ),
            Status::ReplyDefault.code()
        );
        assert_eq!(
            (suite.message)(
                std::ptr::null_mut(),
                c"OfxMessageLog".as_ptr(),
                c"test".as_ptr(),
                c"a line for the log".as_ptr(),
            ),
            Status::Ok.code()
        );
        // A message with nothing in it must not take the host down.
        assert_eq!(
            (suite.message)(
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            ),
            Status::Ok.code()
        );
    }

    assert!(state()
        .messages
        .iter()
        .any(|message| message.text == "a line for the log"));
}

// ----------------------------------------------------------------- quirks --

#[test]
fn the_shipped_quirks_file_parses_and_promises_nothing() {
    let table = QuirksTable::parse(include_str!("../quirks.json"))
        .map_err(|error| error.to_string())
        .expect("the shipped quirks file must parse");
    let quirks = table.for_plugin("com.example.anything", 1);

    assert_eq!(quirks, crate::quirks::Quirks::default());
    assert_eq!(quirks.render_timeout.as_secs(), 10);
    assert_eq!(quirks.control_timeout.as_secs(), 2);
    assert!(quirks.suite_versions.is_empty());
}

/// A family of plugins under one prefix is one entry, and the host name it is
/// shown is that entry's `present_as`.
#[test]
fn a_quirks_entry_may_name_a_family_and_who_the_host_says_it_is() {
    let table = QuirksTable::parse(
        r#"{ "plugins": [ {
            "identifier": "com.example.suite.*",
            "present_as": "SomeOtherHost",
            "note": "refuses every host it has not heard of"
        } ] }"#,
    )
    .map_err(|error| error.to_string())
    .expect("a well-formed table parses");

    let matched = table.for_plugin("com.example.suite.Glow", 3);
    assert_eq!(matched.present_as.as_deref(), Some("SomeOtherHost"));
    assert_eq!(matched.render_timeout.as_secs(), 10, "nothing else changed");

    assert_eq!(
        table.for_plugin("com.example.suitecase", 3).present_as,
        None,
        "a prefix is a prefix, not a substring"
    );
    assert_eq!(
        table.for_plugin("com.example.suite", 3).present_as,
        None,
        "the family, not the name the star hangs off"
    );
}

/// The shipped table presents Lumit to Red Giant Universe as DaVinci Resolve,
/// because Universe reads `kOfxPropName` in `describeInContext` and answers
/// `kOfxStatErrMissingHostFeature` to every host but a handful.
#[test]
fn the_shipped_quirks_file_presents_the_host_to_universe_as_resolve() {
    let table = QuirksTable::shipped();
    assert_eq!(
        table
            .for_plugin("com.redgiantsoftware.Universe_Glow_Glow_OFX", 3)
            .present_as
            .as_deref(),
        Some("DaVinciResolve")
    );
    assert_eq!(
        table.for_plugin("com.redgiant.MBFilm.ofx", 5).present_as,
        None,
        "Magic Bullet takes Lumit's own name"
    );
}

#[test]
fn a_quirks_entry_overrides_only_what_it_names() {
    let table = QuirksTable::parse(
        r#"{ "plugins": [ {
            "identifier": "com.example.slow",
            "version_major": 3,
            "render_timeout_ms": 45000,
            "suite_versions": { "OfxImageEffectSuite": 1 },
            "note": "asks for suite 2 and then calls it as 1"
        } ] }"#,
    )
    .map_err(|error| error.to_string())
    .expect("a well-formed table parses");

    let matched = table.for_plugin("com.example.slow", 3);
    assert_eq!(matched.render_timeout.as_secs(), 45);
    assert_eq!(matched.control_timeout.as_secs(), 2);
    assert_eq!(matched.suite_versions.get("OfxImageEffectSuite"), Some(&1));

    // Another version of the same plugin is another plugin.
    assert_eq!(
        table.for_plugin("com.example.slow", 4),
        crate::quirks::Quirks::default()
    );
}

// --------------------------------------------------------- struct layouts --

#[test]
fn the_c_structs_are_laid_out_as_c_lays_them_out() {
    let pointer = size_of::<*const c_void>();

    assert_eq!(size_of::<OfxHost>(), 2 * pointer);
    assert_eq!(offset_of!(OfxHost, host), 0);
    assert_eq!(offset_of!(OfxHost, fetch_suite), pointer);

    // Two pointers, an int, two unsigned ints and two function pointers, with
    // the padding a C compiler would insert after the first int.
    assert_eq!(size_of::<OfxPlugin>(), 6 * pointer);
    assert_eq!(offset_of!(OfxPlugin, plugin_api), 0);
    assert_eq!(offset_of!(OfxPlugin, api_version), pointer);
    assert_eq!(offset_of!(OfxPlugin, plugin_identifier), 2 * pointer);
    assert_eq!(offset_of!(OfxPlugin, plugin_version_major), 3 * pointer);
    assert_eq!(
        offset_of!(OfxPlugin, plugin_version_minor),
        3 * pointer + size_of::<u32>()
    );
    assert_eq!(offset_of!(OfxPlugin, set_host), 4 * pointer);
    assert_eq!(offset_of!(OfxPlugin, main_entry), 5 * pointer);

    // Eighteen function pointers, in the order the header declares them: the
    // order is the ABI, so the last one's offset is the whole assertion.
    assert_eq!(size_of::<OfxPropertySuiteV1>(), 18 * pointer);
    assert_eq!(offset_of!(OfxPropertySuiteV1, prop_set_pointer), 0);
    assert_eq!(offset_of!(OfxPropertySuiteV1, prop_set_int), 3 * pointer);
    assert_eq!(
        offset_of!(OfxPropertySuiteV1, prop_get_pointer),
        8 * pointer
    );
    assert_eq!(offset_of!(OfxPropertySuiteV1, prop_reset), 16 * pointer);
    assert_eq!(
        offset_of!(OfxPropertySuiteV1, prop_get_dimension),
        17 * pointer
    );

    assert_eq!(size_of::<OfxMemorySuiteV1>(), 2 * pointer);
    assert_eq!(offset_of!(OfxMemorySuiteV1, memory_free), pointer);
    assert_eq!(size_of::<OfxMessageSuiteV1>(), pointer);
}

// ---------------------------------------------------------------- bundles --

/// The test plugin's file name on this platform.
fn test_plugin_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_ofx_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_ofx_testplug.dylib"
    } else {
        "liblumit_ofx_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
fn test_plugin() -> Option<PathBuf> {
    let name = test_plugin_file_name();
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..3 {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        let in_deps = dir.join("deps").join(name);
        if in_deps.is_file() {
            return Some(in_deps);
        }
        dir = dir.parent()?;
    }
    None
}

/// Lay the test plugin out as a real bundle inside `root`, and answer with the
/// path of the binary. `None` means the plugin was not built, and the caller
/// skips.
fn a_bundle_in(root: &Path) -> Option<PathBuf> {
    let source = test_plugin()?;
    let contents = root.join("Test.ofx.bundle").join("Contents");
    let dir = contents.join(BUNDLE_ARCH_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    let binary = dir.join("test.ofx");
    std::fs::copy(&source, &binary).ok()?;
    Some(binary)
}

/// Say why a test did nothing, by name, so a skip is never silent.
fn skipped(test: &str) {
    eprintln!(
        "{test}: skipped — {} was not found in the target directory. \
         Build it first: cargo build -p lumit-ofx-testplug",
        test_plugin_file_name()
    );
}

#[test]
fn a_bundle_tree_is_scanned_for_its_binaries() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(binary) = a_bundle_in(root.path()) else {
        skipped("a_bundle_tree_is_scanned_for_its_binaries");
        return;
    };

    assert_eq!(scan_dir(root.path()), vec![binary]);

    // A folder that is not a bundle, and a bundle with nothing in it, are both
    // simply not plugins.
    let ignored = root.path().join("not-a-bundle");
    assert!(std::fs::create_dir_all(ignored.join("Contents").join(BUNDLE_ARCH_DIR)).is_ok());
    assert_eq!(scan_dir(root.path()).len(), 1);
    assert!(scan_dir(&root.path().join("nowhere")).is_empty());

    // **And a bundle in a folder of its own is found too.** Every suite that
    // ships more than one plugin installs that way — `OFX/Plugins/Red Giant
    // Universe/`, `OFX/Plugins/Magic Bullet Suite/` — and a scan that read only
    // the top of the search path offered a machine full of plugins nothing at
    // all.
    let vendor = root.path().join("A Vendor Suite").join("deeper");
    assert!(std::fs::create_dir_all(&vendor).is_ok());
    let Some(nested) = a_bundle_in(&vendor) else {
        return;
    };
    let found = scan_dir(root.path());
    assert_eq!(found.len(), 2, "the nested bundle was missed: {found:?}");
    assert!(found.contains(&nested));
}

#[test]
fn the_host_is_given_to_a_plugin_once_before_it_is_loaded() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(binary) = a_bundle_in(root.path()) else {
        skipped("the_host_is_given_to_a_plugin_once_before_it_is_loaded");
        return;
    };

    // The test's own handle on the same binary. The loader hands back the same
    // module for the same path, so this reads the counters the bundle's copy
    // is writing — and keeps the library alive after the bundle drops it.
    // SAFETY: loading a library runs its initialisers; this one is ours.
    let Ok(probe) = (unsafe { libloading::Library::new(&binary) }) else {
        skipped("the_host_is_given_to_a_plugin_once_before_it_is_loaded");
        return;
    };
    // SAFETY: the three probe exports are declared in the test plugin with
    // exactly this signature.
    let read = |name: &[u8]| -> c_int {
        let symbol: Result<libloading::Symbol<unsafe extern "C" fn() -> c_int>, _> =
            unsafe { probe.get(name) };
        match symbol {
            Ok(symbol) => unsafe { symbol() },
            Err(_) => -1,
        }
    };

    assert_eq!(read(b"LumitTestPlugSetHostCalls\0"), 0);

    let mut bundle = Bundle::open(&binary)
        .map_err(|error| error.to_string())
        .expect("the test plugin is an OFX plugin");
    assert_eq!(
        bundle.plugins().len(),
        lumit_ofx_testplug::PLUGIN_COUNT as usize
    );
    let plugin = &bundle.plugins()[0];
    assert_eq!(plugin.identifier, "com.lumitlab.testplug");
    assert_eq!(plugin.version, (1, 0));
    assert!(plugin.is_supported_image_effect());
    // Opening is not loading: nothing has been said to the plugin yet.
    assert_eq!(read(b"LumitTestPlugSetHostCalls\0"), 0);

    // Loading sets the process-wide host name, so it takes the name lock.
    let _name = host_name_lock();
    bundle.load();
    assert_eq!(bundle.plugins()[0].load_status, Some(Status::Ok));
    assert_eq!(
        read(b"LumitTestPlugSetHostCalls\0"),
        lumit_ofx_testplug::PLUGIN_COUNT
    );
    assert_eq!(read(b"LumitTestPlugHostSeenBeforeLoad\0"), 1);

    // Loading twice does not hand the plugins a second host.
    bundle.load();
    assert_eq!(
        read(b"LumitTestPlugSetHostCalls\0"),
        lumit_ofx_testplug::PLUGIN_COUNT
    );

    // The plugin got the six suites this host has and nothing it has not,
    // which is the whole of `fetchSuite` proved from the plugin's side.
    let mask = read(b"LumitTestPlugSuiteMask\0");
    let expected = lumit_ofx_testplug::SUITE_PROPERTY
        | lumit_ofx_testplug::SUITE_MEMORY
        | lumit_ofx_testplug::SUITE_MESSAGE
        | lumit_ofx_testplug::SUITE_IMAGE_EFFECT
        | lumit_ofx_testplug::SUITE_PARAMETER
        | lumit_ofx_testplug::SUITE_INTERACT;
    assert_eq!(u32::try_from(mask), Ok(expected));

    // And its load message came back through the message suite.
    let message = lumit_ofx_testplug::LOAD_MESSAGE
        .to_string_lossy()
        .to_string();
    assert!(state().messages.iter().any(|logged| logged.text == message));

    bundle.unload();
    bundle.unload();
    drop(probe);
}

#[test]
fn a_binary_that_is_not_a_plugin_is_refused_rather_than_called() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let path = root.path().join("nothing.ofx");
    assert!(std::fs::write(&path, b"not a library").is_ok());

    let opened = Bundle::open(&path);
    assert!(opened.is_err(), "a text file is not a plugin");
    assert!(Bundle::open(root.path().join("absent.ofx")).is_err());
}

#[test]
fn the_standard_plugin_location_is_always_searched() {
    let paths = crate::bundle::search_paths();
    let first = paths.first().map(|path| path.to_string_lossy().to_string());
    // docs/12 §2.6: the platform's own location, always, plus whatever
    // OFX_PLUGIN_PATH adds for people who install plugins elsewhere.
    assert_eq!(
        first
            .as_deref()
            .map(|path| path.ends_with("OFX") || path.ends_with("Plugins")),
        Some(true),
        "the standard location is missing from {paths:?}"
    );
}

// ---------------------------------------------------------------- describe --

/// A loaded bundle of the five test plugins, or `None` if the plugin was not
/// built. The temporary directory is handed back with it: dropping it would
/// take the binary out from under the loaded library.
fn a_loaded_bundle(test: &str) -> Option<(tempfile::TempDir, Bundle)> {
    let root = tempfile::tempdir().ok()?;
    let Some(binary) = a_bundle_in(root.path()) else {
        skipped(test);
        return None;
    };
    let mut bundle = Bundle::open(&binary).ok()?;
    // Loading sets the process-wide host name from the quirks table, so it
    // takes the same lock the tests that read that name hold.
    {
        let _name = host_name_lock();
        bundle.load();
    }
    Some((root, bundle))
}

/// One plugin from a scan, by identifier and major version.
fn found<'a>(report: &'a ScanReport, identifier: &str, major: u32) -> Option<&'a DescribedPlugin> {
    report.effects.iter().find(|effect| {
        effect.descriptor.identifier == identifier && effect.descriptor.version.0 == major
    })
}

/// Why one plugin was turned away.
fn refused<'a>(report: &'a ScanReport, identifier: &str) -> Option<&'a Rejection> {
    report
        .rejected
        .iter()
        .find(|entry| entry.identifier == identifier)
        .map(|entry| &entry.reason)
}

/// A schema as one stable block of text: the assertion is the whole shape at
/// once, so a change to any of it is a change somebody has to type.
fn render(schema: &EffectSchema) -> String {
    let mut out = format!(
        "match_name = {}\nlabel = {}\nversion = {}\ncategory = {:?}\ncost = {:?}\n\
         roi = {:?}\ntemporal = {:?}\npremultiplied = {}\nmatte = {:?}\n",
        schema.match_name,
        schema.label,
        schema.version,
        schema.category,
        schema.traits.cost,
        schema.traits.roi,
        schema.traits.temporal,
        schema.traits.premultiplied,
        schema.matte,
    );
    for param in schema.params {
        out.push_str(&format!(
            "param {} | {} | {:?} | {:?}\n",
            param.id, param.label, param.kind, param.unit
        ));
    }
    for group in schema.groups {
        out.push_str(&format!(
            "group {} | {:?} | collapsed {}\n",
            group.label, group.params, group.collapsed
        ));
    }
    out
}

#[test]
fn a_plugin_describes_itself_into_the_schema_a_built_in_has() {
    let Some((_root, bundle)) = a_loaded_bundle("a_plugin_describes_itself_into_the_schema") else {
        return;
    };
    let report = describe_bundle(&bundle);
    let full = found(&report, "com.lumitlab.testplug", 1).expect("the full plugin described");

    // What it said about itself.
    assert_eq!(full.descriptor.label, "Test plug");
    assert_eq!(full.descriptor.grouping, "Lumit/Test");
    assert_eq!(
        full.descriptor.contexts,
        vec![Context::Filter, Context::General],
        "a plugin offering both is driven as the simpler one"
    );
    assert!(full.descriptor.temporal, "it declared temporal clip access");
    assert_eq!(
        full.descriptor
            .clips
            .iter()
            .map(|clip| clip.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Source", "Output"]
    );
    // The two controls Lumit has no row for are reported, never silently lost.
    assert_eq!(
        full.descriptor.unrepresented(),
        vec![
            ("caption", "OfxParamTypeString"),
            ("vendorBlob", "OfxParamTypeCustom"),
        ]
    );

    let expected = "\
match_name = ofx:com.lumitlab.testplug
label = Test plug
version = 1
category = Utility
cost = Heavy
roi = FullFrame
temporal = [-1, 0, 1]
premultiplied = true
matte = None
param gain | Gain | Float { default: 0.5, slider: (0.0, 2.0), hard: (Some(0.0), Some(4.0)) } | Raw
param rotation | Rotation | Angle { default: 45.0, dial_step: 1.0 } | Degrees
param centre_x | Centre X | Float { default: 0.0, slider: (-100.0, 100.0), hard: (None, None) } | Px
param centre_y | Centre Y | Float { default: 0.0, slider: (-100.0, 100.0), hard: (None, None) } | Px
param offset_x | Offset X | Float { default: 1.0, slider: (0.0, 1.0), hard: (None, None) } | Raw
param offset_y | Offset Y | Float { default: 2.0, slider: (0.0, 1.0), hard: (None, None) } | Raw
param offset_z | Offset Z | Float { default: 3.0, slider: (0.0, 1.0), hard: (None, None) } | Raw
param count | Count | Int { default: 3, slider: (1, 10), hard: (Some(1), Some(10)) } | Raw
param size_x | Size X | Int { default: 640, slider: (0, 100), hard: (None, None) } | Raw
param size_y | Size Y | Int { default: 480, slider: (0, 100), hard: (None, None) } | Raw
param enabled | Enabled | Bool { default: true } | Raw
param mode | Mode | Choice { options: [\"Soft\", \"Hard\", \"Wild\"], default: 1, dividers_after: [] } | Raw
param tint | Tint | Colour { default: [0.25, 0.5, 0.75, 1.0], range: (0.0, 1.0) } | Raw
param wash | Wash | Colour { default: [1.0, 0.0, 0.0, 1.0], range: (0.0, 1.0) } | Raw
param lutPath | LUT file | File { filter: [], filter_name: \"All files\" } | Raw
param trigger | Trigger | Action | Raw
group Advanced | [\"offset_x\", \"offset_y\", \"offset_z\", \"count\"] | collapsed true
group Files | [\"lutPath\", \"trigger\"] | collapsed false
";
    assert_eq!(render(&full.schema), expected);

    // A point is two adjacent number rows, which is what makes the panel fold
    // it into one, and the convention applies to a plugin's rows
    // unchanged. The 3-D Offset folds its x and y and leaves z beside them,
    // which is what the rule says and what a built-in with those three ids
    // would get.
    assert_eq!(
        full.schema
            .pairs()
            .map(|pair| pair.stem)
            .collect::<Vec<_>>(),
        vec!["centre", "offset"]
    );
}

#[test]
fn a_context_this_host_cannot_drive_is_a_reason_and_not_the_end_of_the_scan() {
    let Some((_root, bundle)) = a_loaded_bundle("a_context_this_host_cannot_drive") else {
        return;
    };
    let report = describe_bundle(&bundle);

    let reason = refused(&report, "com.lumitlab.testplug.generator")
        .expect("a generator-only plugin is turned away");
    assert_eq!(
        reason,
        &Rejection::NoDrivenContext {
            declared: vec!["OfxImageEffectContextGenerator".to_owned()],
        }
    );
    // The reason is a sentence somebody can read, not a code.
    assert!(reason
        .to_string()
        .contains("OfxImageEffectContextGenerator"));

    // And the rest of the bundle came through it untouched, which is the whole
    // point: one plugin Lumit cannot drive must not cost the others.
    assert!(found(&report, "com.lumitlab.testplug", 1).is_some());
    assert!(found(&report, "com.lumitlab.testplug", 2).is_some());
}

#[test]
fn a_plugin_that_fails_to_describe_yields_no_schema_and_no_panic() {
    let Some((_root, bundle)) = a_loaded_bundle("a_plugin_that_fails_to_describe") else {
        return;
    };
    let report = describe_bundle(&bundle);

    assert_eq!(
        refused(&report, "com.lumitlab.testplug.broken"),
        Some(&Rejection::DescribeFailed(Status::Failed))
    );
    assert!(found(&report, "com.lumitlab.testplug.broken", 1).is_none());
}

#[test]
fn two_versions_of_one_identifier_are_two_schemas() {
    let Some((_root, bundle)) = a_loaded_bundle("two_versions_of_one_identifier") else {
        return;
    };
    let report = describe_bundle(&bundle);

    let first = found(&report, "com.lumitlab.testplug", 1).expect("version one described");
    let second = found(&report, "com.lumitlab.testplug", 2).expect("version two described");

    // Same match name, different version: the pair is the cache key, so two
    // versions of one plugin are two effects that can sit in one project.
    assert_eq!(first.schema.match_name, second.schema.match_name);
    assert_ne!(first.schema.version, second.schema.version);
    assert_ne!(first.schema, second.schema);
    assert_eq!(second.schema.label, "Test plug mark two");
    assert_eq!(second.schema.params.len(), 1);
    // The one that never claimed temporal access does not get it advertised.
    assert_eq!(second.schema.traits.temporal, &[0]);
}

#[test]
fn two_parameters_that_would_share_an_id_are_refused() {
    let Some((_root, bundle)) = a_loaded_bundle("two_parameters_that_would_share_an_id") else {
        return;
    };
    let report = describe_bundle(&bundle);

    // `centre` spreads into `centre_x`, and the plugin also defined `centre_x`.
    // A `ParamId` collision is silent (docs/impl/effect-registry.md §5), so it
    // is caught here rather than shipped.
    assert_eq!(
        refused(&report, "com.lumitlab.testplug.duplicate"),
        Some(&Rejection::DuplicateParamId {
            first: "centre_x".to_owned(),
            second: "centre_x".to_owned(),
        })
    );
    assert!(found(&report, "com.lumitlab.testplug.duplicate", 1).is_none());
}

// ------------------------------------------- the two new suites, through C --

/// A live descriptor, and the handle the plugin would hold. Released by the
/// caller.
fn a_live_descriptor() -> Handle {
    crate::describe::new_descriptor(crate::describe::base_property_set("com.example.test"))
        .expect("room for one descriptor")
}

#[test]
fn a_parameter_defined_twice_under_one_name_is_refused() {
    let effect = a_live_descriptor();
    let mut param_set: *mut c_void = std::ptr::null_mut();
    let suite = &crate::suites::image_effect::SUITE;
    let params = &crate::suites::parameter::SUITE;

    // SAFETY: a live handle and valid out-parameters throughout.
    unsafe {
        assert_eq!(
            (suite.get_param_set)(effect.as_ptr(), &raw mut param_set),
            Status::Ok.code()
        );
        let mut props: *mut c_void = std::ptr::null_mut();
        assert_eq!(
            (params.param_define)(
                param_set,
                c"OfxParamTypeDouble".as_ptr(),
                c"amount".as_ptr(),
                &raw mut props,
            ),
            Status::Ok.code()
        );
        assert!(!props.is_null());

        // The same name again is `kOfxStatErrExists`, not a second row.
        assert_eq!(
            (params.param_define)(
                param_set,
                c"OfxParamTypeDouble".as_ptr(),
                c"amount".as_ptr(),
                &raw mut props,
            ),
            Status::ErrExists.code()
        );

        // A type this host has never heard of is refused rather than stored.
        assert_eq!(
            (params.param_define)(
                param_set,
                c"OfxParamTypeQuaternion".as_ptr(),
                c"spin".as_ptr(),
                &raw mut props,
            ),
            Status::ErrUnsupported.code()
        );

        // A defined parameter can be found again, and its properties read.
        let mut handle: *mut c_void = std::ptr::null_mut();
        assert_eq!(
            (params.param_get_handle)(
                param_set,
                c"amount".as_ptr(),
                &raw mut handle,
                &raw mut props,
            ),
            Status::Ok.code()
        );
        let mut back: *mut c_void = std::ptr::null_mut();
        assert_eq!(
            (params.param_get_property_set)(handle, &raw mut back),
            Status::Ok.code()
        );
        assert_eq!(back, props);

        // A *descriptor's* parameters have no values: nothing has been
        // evaluated, so there is nothing to read, and the suite says so rather
        // than handing back the default as though it were a value.
        let mut value = 0.0_f64;
        assert_eq!(
            (params.param_get_value)(handle, std::ptr::from_mut(&mut value).cast::<c_void>()),
            Status::ErrUnsupported.code()
        );
    }

    crate::describe::release_descriptor(effect);
}

/// The animation half of the parameter suite is not built, and says so — but it
/// must say **which** no. A forged handle is `kOfxStatErrBadHandle` at every
/// entry point of every suite; `kOfxStatErrUnsupported` there would tell a
/// plugin the feature is missing when the truth is that its handle is rubbish,
/// and it is the one answer that would have it try the same handle elsewhere.
///
/// The conformance fuzz target (`tests/handle_fuzz.rs`) is what found this;
/// this is the seed it grew from, kept because it is the smaller thing to read
/// (docs/impl/ofx-host.md §5 item 2).
#[test]
fn an_unbuilt_parameter_entry_point_still_tells_a_bad_handle_apart() {
    let effect = a_live_descriptor();
    let params = &crate::suites::parameter::SUITE;
    let suite = &crate::suites::image_effect::SUITE;
    let mut param_set: *mut c_void = std::ptr::null_mut();
    let mut props: *mut c_void = std::ptr::null_mut();
    let mut handle: *mut c_void = std::ptr::null_mut();
    let mut keys: c_uint = 0;

    // SAFETY: a live handle and valid out-parameters throughout.
    unsafe {
        assert_eq!(
            (suite.get_param_set)(effect.as_ptr(), &raw mut param_set),
            Status::Ok.code()
        );
        assert_eq!(
            (params.param_define)(
                param_set,
                c"OfxParamTypeDouble".as_ptr(),
                c"amount".as_ptr(),
                &raw mut props,
            ),
            Status::Ok.code()
        );
        assert_eq!(
            (params.param_get_handle)(
                param_set,
                c"amount".as_ptr(),
                &raw mut handle,
                &raw mut props,
            ),
            Status::Ok.code()
        );

        // A real parameter: the feature is genuinely missing.
        assert_eq!(
            (params.param_get_num_keys)(handle, &raw mut keys),
            Status::ErrUnsupported.code()
        );
        // A *descriptor's* parameter has no value to write, exactly as it has
        // none to read. On an instance this is a real write; see
        // `a_plugin_writes_its_own_control_while_it_is_being_built`.
        assert_eq!(
            (params.param_set_value)(handle, 0.0_f64),
            Status::ErrUnsupported.code()
        );
        assert_eq!(
            (params.param_edit_begin)(param_set, c"edit".as_ptr()),
            Status::Ok.code()
        );
        assert_eq!((params.param_edit_end)(param_set), Status::Ok.code());

        // The same calls with a handle nobody minted: a different no.
        let forged = 0xdead_beef_usize as *mut c_void;
        assert_eq!(
            (params.param_get_num_keys)(forged, &raw mut keys),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_get_key_time)(forged, 0, std::ptr::null_mut()),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_get_key_index)(forged, 0.0, 0, std::ptr::null_mut()),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_set_value)(forged, 0.0_f64),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_set_value_at_time)(forged, 0.0, 0.0_f64),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_get_derivative)(forged, 0.0),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_get_integral)(forged, 0.0, 1.0),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_delete_key)(forged, 0.0),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_delete_all_keys)(forged),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_copy)(forged, forged, 0.0, std::ptr::null()),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (params.param_edit_begin)(forged, c"edit".as_ptr()),
            Status::ErrBadHandle.code()
        );
        assert_eq!((params.param_edit_end)(forged), Status::ErrBadHandle.code());
    }

    crate::describe::release_descriptor(effect);
}

#[test]
fn the_definition_suites_refuse_a_forged_handle() {
    let effect = a_live_descriptor();
    let forged = [
        ("null", std::ptr::null_mut::<c_void>()),
        ("garbage", 0xdead_beef_usize as *mut c_void),
        (
            "wrong kind",
            Handle::encode(HandleKind::Clip, 0)
                .expect("index nought fits")
                .as_ptr(),
        ),
        (
            "past the end",
            Handle::encode(HandleKind::ImageEffect, 1 << 30)
                .expect("the index fits in the field")
                .as_ptr(),
        ),
    ];

    let suite = &crate::suites::image_effect::SUITE;
    let params = &crate::suites::parameter::SUITE;
    let mut out: *mut c_void = std::ptr::null_mut();

    for (name, handle) in forged {
        // SAFETY: valid out-parameters; the handle is the thing under test, and
        // each entry point must reject it without following it.
        unsafe {
            assert_eq!(
                (suite.get_property_set)(handle, &raw mut out),
                Status::ErrBadHandle.code(),
                "getPropertySet accepted a {name} handle"
            );
            assert_eq!(
                (suite.get_param_set)(handle, &raw mut out),
                Status::ErrBadHandle.code(),
                "getParamSet accepted a {name} handle"
            );
            assert_eq!(
                (suite.clip_define)(handle, c"Source".as_ptr(), &raw mut out),
                Status::ErrBadHandle.code(),
                "clipDefine accepted a {name} handle"
            );
            assert_eq!(
                (params.param_define)(
                    handle,
                    c"OfxParamTypeDouble".as_ptr(),
                    c"amount".as_ptr(),
                    &raw mut out,
                ),
                Status::ErrBadHandle.code(),
                "paramDefine accepted a {name} param set"
            );
            assert_eq!(
                (params.param_get_property_set)(handle, &raw mut out),
                Status::ErrBadHandle.code(),
                "paramGetPropertySet accepted a {name} handle"
            );
        }
    }

    // An effect handle is not a param set, and a param set is not an effect:
    // the kinds are the whole of the check.
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (params.param_define)(
                effect.as_ptr(),
                c"OfxParamTypeDouble".as_ptr(),
                c"amount".as_ptr(),
                &raw mut out,
            ),
            Status::ErrBadHandle.code()
        );
        let param_set = effect
            .recast(HandleKind::ParamSet)
            .expect("an effect handle recasts");
        assert_eq!(
            (suite.get_property_set)(param_set.as_ptr(), &raw mut out),
            Status::ErrBadHandle.code()
        );
    }

    crate::describe::release_descriptor(effect);
}

/// The clip half of the suite is live now, but only for a clip handle the host
/// actually minted: a forged one is a status, never a followed pointer.
#[test]
fn the_clip_entry_points_refuse_a_handle_that_is_not_a_clip() {
    let suite = &crate::suites::image_effect::SUITE;
    let mut out: *mut c_void = std::ptr::null_mut();
    let mut bounds = crate::ffi::OfxRectD::default();

    // SAFETY: valid out-parameters; each of these is handed a handle it was
    // never given and must answer a status a plugin expects.
    unsafe {
        assert_eq!(
            (suite.clip_get_property_set)(std::ptr::null_mut(), &raw mut out),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (suite.clip_get_image)(std::ptr::null_mut(), 0.0, std::ptr::null(), &raw mut out),
            Status::ErrBadHandle.code()
        );
        assert_eq!(
            (suite.clip_get_region_of_definition)(std::ptr::null_mut(), 0.0, &raw mut bounds),
            Status::ErrBadHandle.code()
        );
        // Releasing something that is not an image is a status too.
        assert_eq!(
            (suite.clip_release_image)(std::ptr::null_mut()),
            Status::ErrBadHandle.code()
        );
        // `abort` is the one that is not a status, and nought is what lets a
        // render carry on. No render is in flight on this thread, so nothing is
        // cancelled.
        assert_eq!((suite.abort)(std::ptr::null_mut()), 0);
    }
}

#[test]
fn the_two_new_suite_tables_are_laid_out_as_c_lays_them_out() {
    let pointer = size_of::<*const c_void>();

    // Thirteen entry points, in the order the header declares them.
    assert_eq!(size_of::<OfxImageEffectSuiteV1>(), 13 * pointer);
    assert_eq!(offset_of!(OfxImageEffectSuiteV1, get_property_set), 0);
    assert_eq!(offset_of!(OfxImageEffectSuiteV1, clip_define), 2 * pointer);
    assert_eq!(offset_of!(OfxImageEffectSuiteV1, abort), 8 * pointer);
    assert_eq!(
        offset_of!(OfxImageEffectSuiteV1, image_memory_unlock),
        12 * pointer
    );

    // Eighteen, likewise.
    assert_eq!(size_of::<OfxParameterSuiteV1>(), 18 * pointer);
    assert_eq!(offset_of!(OfxParameterSuiteV1, param_define), 0);
    assert_eq!(
        offset_of!(OfxParameterSuiteV1, param_get_property_set),
        3 * pointer
    );
    assert_eq!(
        offset_of!(OfxParameterSuiteV1, param_get_num_keys),
        10 * pointer
    );
    assert_eq!(
        offset_of!(OfxParameterSuiteV1, param_edit_end),
        17 * pointer
    );
}

// ------------------------------------------------- instances and rendering --

/// A picture whose every pixel says where it is and which channel it is, so an
/// upside-down or mirrored frame is obvious rather than plausible.
fn a_test_frame(width: usize, height: usize) -> Frame16 {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.push(f16::from_f32(x as f32 / 16.0));
            pixels.push(f16::from_f32(y as f32 / 16.0));
            // A value above one, because the working space is scene-linear and
            // highlights above one are legal and meaningful (docs/08 §2.1).
            pixels.push(f16::from_f32(2.5));
            pixels.push(f16::ONE);
        }
    }
    Frame16::from_pixels(width, height, pixels).expect("the count matches the size")
}

/// A loaded bundle and its scan, ready for instances to be made from.
fn a_described_bundle(test: &str) -> Option<(tempfile::TempDir, Bundle, ScanReport)> {
    let (root, bundle) = a_loaded_bundle(test)?;
    let report = describe_bundle(&bundle);
    Some((root, bundle, report))
}

/// The `PluginRef` in `bundle` with this identifier.
fn plugin_of<'a>(bundle: &'a Bundle, identifier: &str) -> &'a crate::bundle::PluginRef {
    bundle
        .plugins()
        .iter()
        .find(|plugin| plugin.identifier == identifier)
        .expect("the test bundle declares it")
}

/// One described plugin out of a scan, by identifier.
fn described<'a>(report: &'a ScanReport, identifier: &str) -> &'a DescribedPlugin {
    report
        .effects
        .iter()
        .find(|effect| effect.descriptor.identifier == identifier)
        .expect("the plugin described itself")
}

/// The action log the test plugin recorded, as the host's own action names.
fn action_log(probe: &libloading::Library) -> Vec<String> {
    // SAFETY: the export is declared in the test plugin with this signature.
    let symbol: Result<libloading::Symbol<unsafe extern "C" fn(*mut c_char, c_int) -> c_int>, _> =
        unsafe { probe.get(b"LumitTestPlugActionLog\0") };
    let Ok(symbol) = symbol else {
        return Vec::new();
    };
    let mut buffer = vec![0 as c_char; 4096];
    // SAFETY: the buffer is 4096 writable bytes, which is what is promised.
    let _ = unsafe { symbol(buffer.as_mut_ptr(), 4096) };
    // SAFETY: the plugin wrote a NUL-terminated string into the buffer.
    let text = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    if text.is_empty() {
        return Vec::new();
    }
    text.split(',').map(str::to_owned).collect()
}

/// Call one of the plugin's no-argument probe exports.
fn probe_call(probe: &libloading::Library, name: &[u8]) -> c_int {
    // SAFETY: every probe named this way is declared with this signature.
    let symbol: Result<libloading::Symbol<unsafe extern "C" fn() -> c_int>, _> =
        unsafe { probe.get(name) };
    match symbol {
        // SAFETY: as above.
        Ok(symbol) => unsafe { symbol() },
        Err(_) => -1,
    }
}

/// Call one of the plugin's one-argument probe setters.
fn probe_set(probe: &libloading::Library, name: &[u8], value: c_int) {
    // SAFETY: every setter named this way is declared with this signature.
    let symbol: Result<libloading::Symbol<unsafe extern "C" fn(c_int)>, _> =
        unsafe { probe.get(name) };
    if let Ok(symbol) = symbol {
        // SAFETY: as above.
        unsafe { symbol(value) };
    }
}

/// A handle on the loaded plugin binary of our own, so the probes can be read.
/// The loader hands back the same module for the same path, so this reads the
/// counters the bundle's copy is writing.
fn a_probe(bundle: &Bundle) -> Option<libloading::Library> {
    // SAFETY: loading a library runs its initialisers; this one is ours, and it
    // is already loaded, so this only takes a second reference to it.
    unsafe { libloading::Library::new(bundle.path()) }.ok()
}

/// The image ledger is process-wide, and so is the assertion two tests below
/// make about it: *every picture the host handed a plugin was let go of*. That
/// question has no answer while another test's render is in flight — the count
/// it reads would be somebody else's — so every test that hands images to a
/// plugin takes this first. It is the only lock in the suite, it is held for
/// the length of one test, and a test that fails while holding it poisons
/// nothing worth keeping.
/// The host's name is one process-wide property: a test that changes
/// it, and the golden that reads it, take this so neither sees the other's.
fn host_name_lock() -> std::sync::MutexGuard<'static, ()> {
    static NAME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    NAME.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn image_ledger() -> std::sync::MutexGuard<'static, ()> {
    static LEDGER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LEDGER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The whole of an instance's life in one arrangement: create, render, destroy.
fn render_once(
    bundle: &Bundle,
    report: &ScanReport,
    identifier: &str,
    request: &RenderRequest,
    values: &ParamSnapshot,
) -> Result<Rendered, RenderError> {
    let plugin = plugin_of(bundle, identifier);
    let descriptor = &described(report, identifier).descriptor;
    let instance =
        Instance::create(plugin, descriptor, Context::Filter, values).map_err(RenderError::Host)?;
    let token = Epoch::new().token();
    let rendered = crate::render::render(plugin, &instance, request, &token);
    instance.destroy(plugin).expect("it was destroyed");
    rendered
}

/// The plugin reads its `gain` control out of the snapshot the host handed in,
/// and multiplies the picture by it. Real pixels, through a real plugin, in
/// process.
#[test]
fn a_frame_goes_through_a_plugin_and_comes_out_scaled() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_frame_goes_through_a_plugin") else {
        return;
    };
    let source = a_test_frame(8, 5);

    // The default in the descriptor is 0.5, and the instance is created with its
    // parameters already holding their defaults — which is what the plugin reads
    // back through `paramGetValueAtTime`.
    let rendered = render_once(
        &bundle,
        &report,
        "com.lumitlab.testplug",
        &RenderRequest::filter(0.0, source.clone()),
        &ParamSnapshot::new(),
    )
    .expect("it rendered");
    assert_eq!(rendered.identity_of, None, "it is not a no-op");
    for y in 0..5 {
        for x in 0..8 {
            let (was, now) = (source.pixel(x, y), rendered.frame.pixel(x, y));
            for channel in 0..4 {
                assert!(
                    (now[channel] - was[channel] * 0.5).abs() <= 0.01,
                    "pixel {x},{y} channel {channel}: {} is not half of {}",
                    now[channel],
                    was[channel]
                );
            }
        }
    }

    // And a value the *host* supplies overrides the default, which is the whole
    // point of the snapshot: the plugin has no store of its own.
    let mut values = ParamSnapshot::new();
    values.set("gain", PropValue::double(2.0));
    let rendered = render_once(
        &bundle,
        &report,
        "com.lumitlab.testplug",
        &RenderRequest::filter(0.0, source.clone()),
        &values,
    )
    .expect("it rendered");
    let (was, now) = (source.pixel(3, 2), rendered.frame.pixel(3, 2));
    assert!((now[1] - was[1] * 2.0).abs() <= 0.01, "{now:?} vs {was:?}");
}

/// **The clips are bound before the first question, not before the render.**
///
/// A plugin asks its input how big it is inside
/// `getRegionOfDefinition` — most of openfx-misc does — and a host that binds
/// its clips in time for `kOfxImageEffectActionRender` and no earlier answers
/// "there is no image" to all of that. The plugin then fails the action, and the
/// whole render fails for a reason that has nothing to do with pixels. The
/// conformance bench is what found it; this is the small version.
#[test]
fn a_clip_can_be_measured_before_the_render_action() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_clip_can_be_measured") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("a_clip_can_be_measured_before_the_render_action");
        return;
    };
    probe_call(&probe, b"LumitTestPlugResetProbes ");
    assert_eq!(
        probe_call(&probe, b"LumitTestPlugRodSawSource "),
        0,
        "nothing has asked yet"
    );

    let rendered = render_once(
        &bundle,
        &report,
        "com.lumitlab.testplug",
        &RenderRequest::filter(0.0, a_test_frame(6, 4)),
        &ParamSnapshot::new(),
    );
    assert!(
        rendered.is_ok(),
        "the render itself worked ({:?})",
        rendered.err()
    );
    assert_eq!(
        probe_call(&probe, b"LumitTestPlugRodSawSource "),
        1,
        "the plugin asked its Source clip how big it was during          getRegionOfDefinition, and the host could say"
    );
    // And the pictures did not stay behind afterwards, which is the other half
    // of binding them earlier.
    assert_eq!(crate::suites::memory::image_bytes_live(), 0);
}

/// **An instance carries the project it is part of**, and a plugin reads all of
/// it while it is being constructed: the project's size and extent, how long the
/// effect runs, whether tiles are on. A plugin that cannot find one of them
/// throws before it exists — six of the conformance bench's plugins died on
/// `ProjectExtent` alone, and none of them had done anything wrong.
///
/// The size is the frame being rendered, not a standing default, because a
/// generator places itself by it.
#[test]
fn an_instance_knows_how_big_the_project_is_and_the_render_keeps_it_true() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("an_instance_knows_how_big") else {
        return;
    };
    let plugin = plugin_of(&bundle, "com.lumitlab.testplug.passthrough");
    let descriptor = &described(&report, "com.lumitlab.testplug.passthrough").descriptor;
    let instance = Instance::create(plugin, descriptor, Context::Filter, &ParamSnapshot::new())
        .expect("an instance");

    let props_of = |handle: Handle| -> PropertySet {
        let state = state();
        let props = state.effects.get(handle).expect("the instance").props;
        state.props.get(props).expect("its property set").clone()
    };

    let born = props_of(instance.handle());
    assert_eq!(born.get_double(keys::PROJECT_SIZE, 0), Ok(1920.0));
    assert_eq!(born.get_double(keys::PROJECT_EXTENT, 1), Ok(1080.0));
    assert_eq!(born.get_double(keys::EFFECT_DURATION, 0), Ok(1.0));
    assert_eq!(born.get_int(keys::SUPPORTS_TILES, 0), Ok(0));

    // A render of a 9x4 frame says so, rather than leaving 1080p standing.
    let token = Epoch::new().token();
    let request = RenderRequest::filter(0.0, a_test_frame(9, 4));
    crate::render::render(plugin, &instance, &request, &token).expect("it rendered");
    let after = props_of(instance.handle());
    assert_eq!(after.get_double(keys::PROJECT_SIZE, 0), Ok(9.0));
    assert_eq!(after.get_double(keys::PROJECT_SIZE, 1), Ok(4.0));
    assert_eq!(after.get_double(keys::PROJECT_EXTENT, 0), Ok(9.0));

    instance.destroy(plugin).expect("destroyed");
}

/// **The clip is as long as the frames in hand, and the plugin is told so.**
/// A plugin that reads the frames either side clamps what it asks for to its
/// clip's frame range, so a range that stops at the frame in hand is a plugin
/// that never asks for a second frame. The neighbours a request carries widen
/// the range before the plugin's first question, and come off with the
/// pictures afterwards. A second input the request has no picture for is
/// unconnected, so a plugin with an optional one does not fetch from it.
#[test]
fn a_render_with_neighbours_tells_the_plugin_how_long_its_clip_is() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_render_with_neighbours") else {
        return;
    };
    let plugin = plugin_of(&bundle, "com.lumitlab.testplug.passthrough");
    let mut descriptor = described(&report, "com.lumitlab.testplug.passthrough")
        .descriptor
        .clone();
    // A second, optional input the plugin never fetches from, as RSMB Pro's.
    descriptor.clips.push(crate::describe::ClipDescription {
        name: "Vectors".to_owned(),
        props: PropertySet::new(),
    });
    let instance = Instance::create(plugin, &descriptor, Context::Filter, &ParamSnapshot::new())
        .expect("an instance");

    let clip_props = |handle: Handle, name: &str| -> PropertySet {
        let state = state();
        let clip = state
            .effects
            .get(handle)
            .expect("the instance")
            .clips
            .iter()
            .find(|clip| clip.name == name)
            .expect("the clip")
            .props;
        state.props.get(clip).expect("its property set").clone()
    };
    let source_clip = |handle: Handle| clip_props(handle, crate::render::SOURCE_CLIP);
    assert_eq!(
        clip_props(instance.handle(), "Vectors").get_int(keys::CLIP_CONNECTED, 0),
        Ok(0),
        "an input this host never feeds is born unconnected"
    );
    let duration = |handle: Handle| -> f64 {
        let state = state();
        let props = state.effects.get(handle).expect("the instance").props;
        state
            .props
            .get(props)
            .expect("its property set")
            .get_double(keys::EFFECT_DURATION, 0)
            .expect("a duration")
    };

    let token = Epoch::new().token();
    let mut request = RenderRequest::filter(20.0, a_test_frame(9, 4));
    request.neighbours = vec![(-2, a_test_frame(9, 4)), (1, a_test_frame(9, 4))];
    assert_eq!(request.frame_span(), (18.0, 21.0));
    crate::render::render(plugin, &instance, &request, &token).expect("it rendered");

    let after = source_clip(instance.handle());
    assert_eq!(after.get_double(keys::FRAME_RANGE, 0), Ok(18.0));
    assert_eq!(after.get_double(keys::FRAME_RANGE, 1), Ok(21.0));
    assert_eq!(
        after.get_double(keys::CLIP_UNMAPPED_FRAME_RANGE, 0),
        Ok(18.0)
    );
    assert_eq!(
        after.get_double(keys::CLIP_UNMAPPED_FRAME_RANGE, 1),
        Ok(21.0)
    );
    assert_eq!(after.get_int(keys::CLIP_CONNECTED, 0), Ok(1));
    assert_eq!(duration(instance.handle()), 4.0);
    let vectors = clip_props(instance.handle(), "Vectors");
    assert_eq!(vectors.get_int(keys::CLIP_CONNECTED, 0), Ok(0));
    assert_eq!(vectors.get_double(keys::FRAME_RANGE, 1), Ok(21.0));

    // A plain frame afterwards is a clip of that one frame again.
    let plain = RenderRequest::filter(30.0, a_test_frame(9, 4));
    crate::render::render(plugin, &instance, &plain, &token).expect("it rendered");
    let after = source_clip(instance.handle());
    assert_eq!(after.get_double(keys::FRAME_RANGE, 0), Ok(30.0));
    assert_eq!(after.get_double(keys::FRAME_RANGE, 1), Ok(30.0));
    assert_eq!(duration(instance.handle()), 1.0);

    instance.destroy(plugin).expect("destroyed");
}

/// **A plugin writes its own controls, and the host must let it.** The OFX
/// support library every commercial vendor is built on settles a plugin's
/// parameters with `paramSetValue` while `kOfxActionCreateInstance` is still
/// running; a host that answers `kOfxStatErrUnsupported` there has the library
/// throw, the action fail, and the instance never exist. From a layer that is
/// every plugin refusing to apply, each with a different status — whichever one
/// the vendor's own handler turned the exception into (docs/impl/ofx-host.md
/// §5).
///
/// So the write is accepted, into the instance's snapshot, and reads back.
/// **Every call here is a real C-variadic call** through the suite's own
/// function pointers (an `int`, a `double`, two doubles, a `const char *`),
/// so this one test proves the C shim (`suites/variadic.c`) pulls the right
/// number of the right kind on whichever platform the suite is running on,
/// including the one whose ABI puts variadic arguments somewhere else.
#[test]
fn a_plugin_writes_its_own_control_while_it_is_being_built() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_plugin_writes_its_own") else {
        return;
    };
    let plugin = plugin_of(&bundle, "com.lumitlab.testplug");
    let descriptor = &described(&report, "com.lumitlab.testplug").descriptor;
    let instance = Instance::create(plugin, descriptor, Context::Filter, &ParamSnapshot::new())
        .expect("an instance");

    let params = &crate::suites::parameter::SUITE;
    let suite = &crate::suites::image_effect::SUITE;
    let mut param_set: *mut c_void = std::ptr::null_mut();

    let handle_of = |param_set: *mut c_void, name: &CStr| -> *mut c_void {
        let mut handle: *mut c_void = std::ptr::null_mut();
        let mut props: *mut c_void = std::ptr::null_mut();
        // SAFETY: a live param set and valid out-parameters.
        assert_eq!(
            unsafe {
                (params.param_get_handle)(param_set, name.as_ptr(), &raw mut handle, &raw mut props)
            },
            Status::Ok.code()
        );
        handle
    };

    // SAFETY: a live instance handle and valid out-parameters throughout.
    unsafe {
        assert_eq!(
            (suite.get_param_set)(instance.handle().as_ptr(), &raw mut param_set),
            Status::Ok.code()
        );

        // A double.
        let amount = handle_of(param_set, c"gain");
        assert_eq!(
            (params.param_set_value)(amount, 0.75_f64),
            Status::Ok.code()
        );
        let mut read = 0.0_f64;
        assert_eq!(
            (params.param_get_value)(amount, std::ptr::from_mut(&mut read).cast::<c_void>()),
            Status::Ok.code()
        );
        assert!(
            (read - 0.75).abs() < f64::EPSILON,
            "the double came back {read}"
        );

        // An integer.
        let count = handle_of(param_set, c"count");
        assert_eq!(
            (params.param_set_value)(count, c_int::from(7_i16)),
            Status::Ok.code()
        );
        let mut read: c_int = 0;
        assert_eq!(
            (params.param_get_value)(count, std::ptr::from_mut(&mut read).cast::<c_void>()),
            Status::Ok.code()
        );
        assert_eq!(read, 7);

        // Two doubles, which proves the second argument is pulled and not
        // assumed, and at a time, which proves the shim skips the named
        // argument before the list.
        let centre = handle_of(param_set, c"centre");
        assert_eq!(
            (params.param_set_value_at_time)(centre, 12.0, 3.0_f64, 4.0_f64),
            Status::Ok.code()
        );
        let (mut x, mut y) = (0.0_f64, 0.0_f64);
        assert_eq!(
            (params.param_get_value_at_time)(
                centre,
                12.0,
                std::ptr::from_mut(&mut x).cast::<c_void>(),
                std::ptr::from_mut(&mut y).cast::<c_void>(),
            ),
            Status::Ok.code()
        );
        assert_eq!((x, y), (3.0, 4.0));

        // A string, as the pointer OFX passes.
        let label = handle_of(param_set, c"caption");
        let text = c"written";
        assert_eq!(
            (params.param_set_value)(label, text.as_ptr()),
            Status::Ok.code()
        );
        let mut read: *const c_char = std::ptr::null();
        assert_eq!(
            (params.param_get_value)(label, std::ptr::from_mut(&mut read).cast::<c_void>()),
            Status::Ok.code()
        );
        assert_eq!(CStr::from_ptr(read), c"written");

        // A push button has no value, so writing one is still the honest no —
        // and it is `kOfxStatErrUnsupported`, not a bad handle.
        let button = handle_of(param_set, c"trigger");
        assert_eq!(
            (params.param_set_value)(button),
            Status::ErrUnsupported.code()
        );
    }

    instance.destroy(plugin).expect("destroyed");
}

/// `clipGetHandle`'s property set is **optional** — the header says "if not
/// null" — and answering `kOfxStatErrValue` to a plugin that passed null for it
/// failed an action the plugin had done nothing wrong in. ntsc-rs passes null.
#[test]
fn a_clip_handle_can_be_asked_for_without_its_property_set() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_clip_handle_without_props") else {
        return;
    };
    let plugin = plugin_of(&bundle, "com.lumitlab.testplug.passthrough");
    let descriptor = &described(&report, "com.lumitlab.testplug.passthrough").descriptor;
    let instance = Instance::create(plugin, descriptor, Context::Filter, &ParamSnapshot::new())
        .expect("an instance");

    let suite = &crate::suites::image_effect::SUITE;
    let mut clip: *mut c_void = std::ptr::null_mut();
    // SAFETY: a live handle, a valid clip out-parameter, and a null property
    // set — which is the thing under test.
    unsafe {
        assert_eq!(
            (suite.clip_get_handle)(
                instance.handle().as_ptr(),
                c"Source".as_ptr(),
                &raw mut clip,
                std::ptr::null_mut(),
            ),
            Status::Ok.code(),
            "a null property set is a plugin not wanting one, not an error"
        );
        assert!(!clip.is_null(), "the clip handle still came back");

        // Nowhere to put the clip handle *is* an error: that argument is not
        // optional, and the call would have done nothing at all.
        assert_eq!(
            (suite.clip_get_handle)(
                instance.handle().as_ptr(),
                c"Source".as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ),
            Status::ErrValue.code()
        );
    }

    instance.destroy(plugin).expect("destroyed");
}

/// The pixel path itself: a plugin that changes nothing must give back exactly
/// what it was given. The comparison is bit-for-bit at fp16 because the
/// fp16 → fp32 → fp16 round trip is lossless — every half float is exactly a
/// float — which is well inside the docs/08 §1.6 tolerance of two fp16 ULP.
#[test]
fn a_passthrough_plugin_returns_the_input_unchanged() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_passthrough_plugin_returns") else {
        return;
    };
    let source = a_test_frame(9, 4);
    let rendered = render_once(
        &bundle,
        &report,
        "com.lumitlab.testplug.passthrough",
        &RenderRequest::filter(0.0, source.clone()),
        &ParamSnapshot::new(),
    )
    .expect("it rendered");
    assert_eq!(rendered.frame, source, "the round trip is not lossless");
}

/// The same picture whichever way the block runs. A top-down image is handed
/// over with **negative** row bytes, the plugin steps backwards through memory
/// because the host told it to, and the picture comes back the right way up.
#[test]
fn a_negative_row_bytes_image_comes_back_the_right_way_up() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_negative_row_bytes_image") else {
        return;
    };
    let source = a_test_frame(6, 7);

    let mut both = Vec::new();
    for order in [RowOrder::BottomUp, RowOrder::TopDown] {
        let mut request = RenderRequest::filter(0.0, source.clone());
        request.order = order;
        let rendered = render_once(
            &bundle,
            &report,
            "com.lumitlab.testplug.passthrough",
            &request,
            &ParamSnapshot::new(),
        )
        .expect("it rendered");
        assert_eq!(rendered.frame, source, "{order:?} came back wrong");
        both.push(rendered.frame);
    }
    assert_eq!(both[0], both[1], "the two layouts disagree");

    // And the sign really was negative for the top-down one, so the test above
    // is testing what it says it is.
    let image = Image::from_frame(&source, RowOrder::TopDown).expect("an image");
    assert!(image.row_bytes() < 0, "top-down means negative row bytes");
}

/// What the host actually hands a plugin is the bottom-up layout, with
/// **positive** row bytes. Both layouts are legal and the test above proves
/// both work through a plugin that honours the sign; this one pins that the
/// negative sign is never sent, because a shipped plugin gets it wrong.
/// ntsc-rs 0.9.4 computes its first-row offset for a negative stride in pixel
/// units and applies it in bytes, so a top-down fp32 frame has it write three
/// quarters of a frame past the end of the block: a heap corruption in the
/// broker, and a mostly transparent picture in the viewer with no error to
/// show for it.
#[test]
fn a_filter_request_hands_the_plugin_positive_row_bytes() {
    let source = a_test_frame(6, 7);
    let request = RenderRequest::filter(0.0, source.clone());
    assert_eq!(request.order, RowOrder::BottomUp);
    let image = Image::from_frame(&source, request.order).expect("an image");
    assert!(
        image.row_bytes() > 0,
        "the layout a plugin is handed has positive row bytes"
    );
}

/// The action order, verbatim: the sequence the plugin observed against the
/// listing in docs/impl/ofx-host.md §3, which `render::RENDER_ACTIONS`
/// transcribes.
#[test]
fn the_action_order_is_the_one_the_note_lists() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("the_action_order") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("the_action_order_is_the_one_the_note_lists");
        return;
    };
    probe_call(&probe, b"LumitTestPlugResetProbes\0");

    let _ = render_once(
        &bundle,
        &report,
        "com.lumitlab.testplug.passthrough",
        &RenderRequest::filter(0.0, a_test_frame(4, 4)),
        &ParamSnapshot::new(),
    )
    .expect("it rendered");

    let seen = action_log(&probe);
    let mut expected = vec![actions::CREATE_INSTANCE.to_owned()];
    expected.extend(
        crate::render::RENDER_ACTIONS
            .iter()
            .map(|action| (*action).to_owned()),
    );
    expected.push(actions::DESTROY_INSTANCE.to_owned());
    assert_eq!(seen, expected);

    // And the constant really is the note's listing, spelled out here so that a
    // change to either has to be a change to both.
    assert_eq!(
        expected,
        vec![
            "OfxActionCreateInstance",
            "OfxImageEffectActionGetRegionOfDefinition",
            "OfxImageEffectActionGetRegionsOfInterest",
            "OfxImageEffectActionGetClipPreferences",
            "OfxImageEffectActionGetFramesNeeded",
            "OfxImageEffectActionIsIdentity",
            "OfxImageEffectActionBeginSequenceRender",
            "OfxImageEffectActionRender",
            "OfxImageEffectActionEndSequenceRender",
            "OfxActionDestroyInstance",
        ]
    );
}

/// A change to a control fires wrapped in begin and end, and lands **between**
/// renders — never inside one, which is the thing Sapphire relies on.
#[test]
fn a_changed_control_fires_wrapped_and_between_renders() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_changed_control_fires") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("a_changed_control_fires_wrapped_and_between_renders");
        return;
    };

    let identifier = "com.lumitlab.testplug";
    let plugin = plugin_of(&bundle, identifier);
    let instance = Instance::create(
        plugin,
        &described(&report, identifier).descriptor,
        Context::Filter,
        &ParamSnapshot::new(),
    )
    .expect("an instance");

    probe_call(&probe, b"LumitTestPlugResetProbes\0");
    let token = Epoch::new().token();
    let source = a_test_frame(4, 4);
    let request = RenderRequest::filter(0.0, source.clone());
    let _ = crate::render::render(plugin, &instance, &request, &token).expect("first render");
    instance
        .changed(
            plugin,
            "gain",
            PropValue::double(3.0),
            crate::ffi::prop_values::CHANGE_USER_EDITED,
            0.0,
        )
        .expect("the change went through");
    let after = crate::render::render(plugin, &instance, &request, &token).expect("second render");
    instance.destroy(plugin).expect("destroyed");

    let seen = action_log(&probe);
    let position = |wanted: &str| {
        seen.iter()
            .position(|action| action == wanted)
            .unwrap_or_else(|| panic!("{wanted} never fired: {seen:?}"))
    };
    let begin = position(actions::BEGIN_INSTANCE_CHANGED);
    let changed = position(actions::INSTANCE_CHANGED);
    let end = position(actions::END_INSTANCE_CHANGED);
    assert!(begin < changed && changed < end, "{seen:?}");

    // Between renders: every render action is either wholly before the begin or
    // wholly after the end.
    for (index, action) in seen.iter().enumerate() {
        if crate::render::RENDER_ACTIONS.contains(&action.as_str()) {
            assert!(
                index < begin || index > end,
                "{action} landed inside the change: {seen:?}"
            );
        }
    }

    // And the new value is what the second render used.
    let (was, now) = (source.pixel(2, 2), after.frame.pixel(2, 2));
    assert!((now[1] - was[1] * 3.0).abs() <= 0.02, "{now:?} vs {was:?}");
}

/// Two instances of a fully safe plugin overlap; two of an unsafe one cannot.
///
/// The plugin holds each render at a rendezvous until two are in flight, or
/// until its own deadline — so "did they overlap?" has a definite answer rather
/// than a race with a sleep in it.
#[test]
fn concurrent_renders_follow_the_plugins_own_declaration() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("concurrent_renders") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("concurrent_renders_follow_the_plugins_own_declaration");
        return;
    };

    /// A `PluginRef` shared with the threads of one scope.
    struct Shared(*const crate::bundle::PluginRef);
    // SAFETY: the pointer is into the bundle's plugin list, which outlives the
    // scope that reads it; nothing mutates the `PluginRef`; and calling the
    // plugin's own entry from two threads is exactly what its declared thread
    // safety is a statement about — which the host obeys, so the unsafe plugin
    // is never actually re-entered.
    unsafe impl Sync for Shared {}

    for (identifier, expected, safety) in [
        (
            "com.lumitlab.testplug.passthrough",
            2,
            ThreadSafety::FullySafe,
        ),
        ("com.lumitlab.testplug.unsafe", 1, ThreadSafety::Unsafe),
    ] {
        let plugin = plugin_of(&bundle, identifier);
        let descriptor = &described(&report, identifier).descriptor;
        let instances: Vec<Instance> = (0..2)
            .map(|_| {
                Instance::create(plugin, descriptor, Context::Filter, &ParamSnapshot::new())
                    .expect("an instance")
            })
            .collect();
        assert_eq!(instances[0].thread_safety(), safety, "{identifier}");

        probe_call(&probe, b"LumitTestPlugResetProbes\0");
        probe_set(&probe, b"LumitTestPlugSetRenderRendezvous\0", 2);

        let shared = Shared(std::ptr::from_ref(plugin));
        std::thread::scope(|scope| {
            for instance in &instances {
                let shared = &shared;
                scope.spawn(move || {
                    // SAFETY: as the `unsafe impl` above.
                    let plugin = unsafe { &*shared.0 };
                    let token = Epoch::new().token();
                    let request = RenderRequest::filter(0.0, a_test_frame(4, 4));
                    let _ = crate::render::render(plugin, instance, &request, &token);
                });
            }
        });

        let seen = probe_call(&probe, b"LumitTestPlugMaxConcurrentRenders\0");
        assert_eq!(seen, expected, "{identifier} ran {seen} renders at once");
        probe_set(&probe, b"LumitTestPlugSetRenderRendezvous\0", 0);

        for instance in instances {
            instance.destroy(plugin).expect("destroyed");
        }
    }
}

/// A plugin that says it is a no-op is not rendered at all: the input is the
/// output, and no begin, render or end is dispatched.
#[test]
fn an_identity_plugin_short_circuits_to_its_input() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("an_identity_plugin") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("an_identity_plugin_short_circuits_to_its_input");
        return;
    };
    probe_call(&probe, b"LumitTestPlugResetProbes\0");

    let source = a_test_frame(5, 3);
    let rendered = render_once(
        &bundle,
        &report,
        "com.lumitlab.testplug.identity",
        &RenderRequest::filter(0.0, source.clone()),
        &ParamSnapshot::new(),
    )
    .expect("it answered");

    assert_eq!(rendered.identity_of.as_deref(), Some("Source"));
    assert_eq!(rendered.frame, source);
    let seen = action_log(&probe);
    for action in [
        actions::BEGIN_SEQUENCE_RENDER,
        actions::RENDER,
        actions::END_SEQUENCE_RENDER,
    ] {
        assert!(
            !seen.iter().any(|had| had == action),
            "{action} in {seen:?}"
        );
    }
}

/// A plugin that fails its render yields a typed error, and the half-written
/// output buffer goes with it rather than to the caller.
#[test]
fn a_failed_render_is_an_error_and_not_a_half_written_frame() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_failed_render") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("a_failed_render_is_an_error_and_not_a_half_written_frame");
        return;
    };
    probe_call(&probe, b"LumitTestPlugResetProbes\0");
    probe_set(&probe, b"LumitTestPlugSetRenderFails\0", 1);

    let outcome = render_once(
        &bundle,
        &report,
        "com.lumitlab.testplug.passthrough",
        &RenderRequest::filter(0.0, a_test_frame(4, 4)),
        &ParamSnapshot::new(),
    );
    probe_set(&probe, b"LumitTestPlugSetRenderFails\0", 0);

    assert_eq!(
        outcome.err(),
        Some(RenderError::Plugin {
            action: actions::RENDER,
            status: Status::Failed,
        })
    );
    // The end action still ran, so a plugin that allocated in the begin got its
    // chance to free.
    let seen = action_log(&probe);
    assert!(seen.iter().any(|had| had == actions::END_SEQUENCE_RENDER));
    // And nothing is left of the pictures.
    assert_eq!(crate::suites::memory::image_bytes_live(), 0);
}

/// Cancellation: an epoch that has already turned over stops the render before
/// the plugin is asked anything at all.
#[test]
fn a_stale_epoch_stops_the_render_before_the_plugin_is_asked() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("a_stale_epoch") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("a_stale_epoch_stops_the_render_before_the_plugin_is_asked");
        return;
    };

    let identifier = "com.lumitlab.testplug.passthrough";
    let plugin = plugin_of(&bundle, identifier);
    let instance = Instance::create(
        plugin,
        &described(&report, identifier).descriptor,
        Context::Filter,
        &ParamSnapshot::new(),
    )
    .expect("an instance");

    probe_call(&probe, b"LumitTestPlugResetProbes\0");
    let epoch = Epoch::new();
    let token = epoch.token();
    epoch.bump();

    let request = RenderRequest::filter(0.0, a_test_frame(4, 4));
    let outcome = crate::render::render(plugin, &instance, &request, &token);
    instance.destroy(plugin).expect("destroyed");

    assert_eq!(outcome.err(), Some(RenderError::Cancelled));
    let seen = action_log(&probe);
    assert!(
        seen.iter()
            .all(|action| action == actions::DESTROY_INSTANCE),
        "the plugin was asked something: {seen:?}"
    );
}

/// Releasing an image twice is a status, not a second free. The first release
/// strikes the handle off; the second finds nothing, which is exactly the
/// forged-handle path every suite call already walks.
#[test]
fn releasing_an_image_twice_is_a_status() {
    let _ledger = image_ledger();
    let Some((_root, bundle, report)) = a_described_bundle("releasing_an_image_twice") else {
        return;
    };
    let identifier = "com.lumitlab.testplug.passthrough";
    let plugin = plugin_of(&bundle, identifier);
    let instance = Instance::create(
        plugin,
        &described(&report, identifier).descriptor,
        Context::Filter,
        &ParamSnapshot::new(),
    )
    .expect("an instance");

    // Put a picture on the instance by hand, so the fetch below has something
    // to find without a render being in flight.
    let mut images = std::collections::BTreeMap::new();
    images.insert(
        "Source".to_owned(),
        Image::from_frame(&a_test_frame(4, 4), RowOrder::TopDown).expect("an image"),
    );
    drop(
        crate::instance::set_images(instance.handle(), images, std::collections::BTreeMap::new())
            .expect("set"),
    );

    let suite = &crate::suites::image_effect::SUITE;
    let mut clip: *mut c_void = std::ptr::null_mut();
    let mut props: *mut c_void = std::ptr::null_mut();
    let mut image: *mut c_void = std::ptr::null_mut();
    // SAFETY: valid out-parameters, and handles the host itself minted.
    unsafe {
        assert_eq!(
            (suite.clip_get_handle)(
                instance.handle().as_ptr(),
                c"Source".as_ptr(),
                &raw mut clip,
                &raw mut props,
            ),
            Status::Ok.code()
        );
        assert!(!clip.is_null(), "an instance's clip has a handle");
        assert_eq!(
            (suite.clip_get_image)(clip, 0.0, std::ptr::null(), &raw mut image),
            Status::Ok.code()
        );
        assert_eq!((suite.clip_release_image)(image), Status::Ok.code());
        assert_eq!(
            (suite.clip_release_image)(image),
            Status::ErrBadHandle.code(),
            "the second release must be a status, not a second free"
        );
    }

    drop(crate::instance::take_images(instance.handle()).expect("taken"));
    instance.destroy(plugin).expect("destroyed");
    assert_eq!(crate::suites::memory::image_bytes_live(), 0);
}

// -------------------------------------------------------- the thread suite --

/// `multiThreadNumCPUs` says what the host really spends, and every thread of a
/// fan-out is told a different index — which is what plugins key their
/// per-thread scratch by.
#[test]
fn the_thread_suite_counts_honestly_and_indexes_correctly() {
    let Some((_root, bundle)) = a_loaded_bundle("the_thread_suite_counts_honestly") else {
        return;
    };
    let Some(probe) = a_probe(&bundle) else {
        skipped("the_thread_suite_counts_honestly_and_indexes_correctly");
        return;
    };
    // SAFETY: the export is declared in the test plugin with this signature.
    let symbol: Result<libloading::Symbol<unsafe extern "C" fn(*mut c_int) -> c_int>, _> =
        unsafe { probe.get(b"LumitTestPlugFanOut\0") };
    let Ok(fan_out) = symbol else {
        skipped("the_thread_suite_counts_honestly_and_indexes_correctly");
        return;
    };

    let mut cpus: c_int = 0;
    // SAFETY: a valid out-parameter.
    let distinct = unsafe { fan_out(&raw mut cpus) };
    let expected = crate::suites::multi_thread::host_thread_count();
    assert_eq!(
        usize::try_from(cpus),
        Ok(expected),
        "the count must be what the host really spends"
    );
    assert_eq!(
        usize::try_from(distinct),
        Ok(expected),
        "every thread must be told a different index"
    );
}

#[test]
fn a_host_mutex_locks_unlocks_and_refuses_a_forged_handle() {
    let suite = &crate::suites::multi_thread::SUITE;
    let mut mutex: *mut c_void = std::ptr::null_mut();
    // SAFETY: valid out-parameters, and handles this test minted through the
    // suite itself.
    unsafe {
        assert_eq!((suite.mutex_create)(&raw mut mutex, 0), Status::Ok.code());
        assert_eq!((suite.mutex_lock)(mutex), Status::Ok.code());
        // Somebody already holds it, so a try is a plain failure.
        assert_eq!((suite.mutex_try_lock)(mutex), Status::Failed.code());
        assert_eq!((suite.mutex_un_lock)(mutex), Status::Ok.code());
        assert_eq!((suite.mutex_try_lock)(mutex), Status::Ok.code());
        assert_eq!((suite.mutex_un_lock)(mutex), Status::Ok.code());
        // Unlocking one nobody holds is the plugin's bug and a status, never a
        // second unlock.
        assert_eq!((suite.mutex_un_lock)(mutex), Status::ErrValue.code());
        assert_eq!((suite.mutex_destroy)(mutex), Status::Ok.code());
        // And every one of them refuses a handle it never gave out — including
        // the one just destroyed.
        for forged in [
            std::ptr::null_mut::<c_void>(),
            0xdead_beef_usize as *mut c_void,
            mutex,
        ] {
            assert_eq!((suite.mutex_lock)(forged), Status::ErrBadHandle.code());
            assert_eq!((suite.mutex_destroy)(forged), Status::ErrBadHandle.code());
        }
    }
}

#[test]
fn the_thread_suite_table_is_laid_out_as_c_lays_it_out() {
    let pointer = size_of::<*const c_void>();
    // Nine entry points, in the order the header declares them.
    assert_eq!(size_of::<OfxMultiThreadSuiteV1>(), 9 * pointer);
    assert_eq!(offset_of!(OfxMultiThreadSuiteV1, multi_thread), 0);
    assert_eq!(offset_of!(OfxMultiThreadSuiteV1, mutex_create), 4 * pointer);
    assert_eq!(
        offset_of!(OfxMultiThreadSuiteV1, mutex_try_lock),
        8 * pointer
    );
}

// -------------------------------------------------- a plugin as an effect --

use crate::def::{LocalHost, OfxEffectDef, PluginHost, Rendering};
use crate::describe::PluginDescriptor;
use lumit_core::fx::{EffectDef, Params, Value};
use lumit_core::model::EffectValue;

/// A host that never renders — what a disabled or crashed plugin is, from the
/// definition's side (docs/12 §2.3).
struct DeadHost;

impl PluginHost for DeadHost {
    fn render(
        &self,
        _instance: uuid::Uuid,
        _time: f64,
        _params: &ParamSnapshot,
        source: Frame16,
        _neighbours: &[(i32, Frame16)],
    ) -> Rendering {
        Rendering {
            frame: source,
            error: Some("the plugin is disabled for this session".to_owned()),
        }
    }

    fn frames_needed(
        &self,
        _instance: uuid::Uuid,
        _time: f64,
        _params: &ParamSnapshot,
    ) -> Option<Vec<i32>> {
        None
    }

    fn press(
        &self,
        _instance: uuid::Uuid,
        _time: f64,
        _params: &ParamSnapshot,
        _name: &str,
        _source: Frame16,
    ) -> Result<ParamSnapshot, String> {
        Err("the plugin is disabled".to_owned())
    }
}

/// The declaration one described plugin becomes, leaked as the catalogue holds
/// them.
fn a_leaked_schema(report: &ScanReport, identifier: &str, major: u32) -> &'static EffectSchema {
    let found = found(report, identifier, major).expect("the plugin described itself");
    Box::leak(Box::new(found.schema))
}

/// A described plugin, hosted in process, as an entry in the effect catalogue.
///
/// The bundle is handed to the host, and the definition is leaked into the
/// catalogue, so both live as long as the process — which is what registration
/// means.
fn a_registered_plugin(test: &str, identifier: &str, major: u32) -> Option<&'static EffectSchema> {
    let (root, bundle, report) = a_described_bundle(test)?;
    let schema = a_leaked_schema(&report, identifier, major);
    let descriptor = found(&report, identifier, major)?.descriptor.clone();
    let host = std::sync::Arc::new(LocalHost::new(bundle, descriptor.clone()));
    let def = OfxEffectDef::new(&descriptor, schema, host).leak();
    let registered = lumit_core::fx::BUILTIN_DEFS.register(def);
    // The temporary directory outlives nothing on purpose: the library is open
    // and stays open, and Windows will not delete an open binary. Leaking the
    // handle is how the test says so rather than failing on the tidy-up.
    std::mem::forget(root);
    registered.then_some(schema)
}

/// The seam, end to end: a real plugin describes itself, becomes an
/// `EffectDef`, registers, and is found by the same lookup that finds Blur.
#[test]
fn a_plugin_registers_and_is_found_by_the_catalogue() {
    let Some(schema) = a_registered_plugin(
        "a_plugin_registers_and_is_found",
        "com.lumitlab.testplug",
        2,
    ) else {
        return;
    };
    assert_eq!(schema.match_name, "ofx:com.lumitlab.testplug");

    let found = lumit_core::fx::BUILTIN_DEFS
        .get("ofx:com.lumitlab.testplug")
        .expect("the catalogue answers to the plugin");
    assert!(std::ptr::eq(found.schema(), schema));
    assert_eq!(
        lumit_core::fx::schema("ofx:com.lumitlab.testplug").map(|s| s.label),
        Some(schema.label)
    );

    // And it instantiates in the plugin namespace, with the plugin's own
    // defaults, so a project that saves it round-trips as a plugin instance.
    let inst = lumit_core::fx::instantiate("ofx:com.lumitlab.testplug")
        .expect("the catalogue instantiates it");
    assert_eq!(
        inst.effect.namespace,
        lumit_core::model::EffectNamespace::Ofx
    );
    assert_eq!(inst.effect.match_name, "ofx:com.lumitlab.testplug");
    assert!(inst.params.iter().any(|p| p.id == "gain"));

    // The built-in menu order is untouched by its arrival.
    let order: Vec<&str> = lumit_core::fx::BUILTIN_DEFS
        .iter()
        .map(|d| d.schema().match_name)
        .collect();
    let builtins: Vec<&str> = lumit_core::fx::BUILTINS
        .iter()
        .map(|s| s.match_name)
        .collect();
    assert_eq!(&order[..builtins.len()], &builtins[..]);
    assert!(order.contains(&"ofx:com.lumitlab.testplug"));
}

/// Real pixels through the definition: the bag goes out as the plugin's values,
/// the picture goes out as fp32 and comes back multiplied by the control the
/// host owns.
#[test]
fn a_plugin_definition_renders_from_the_resolved_bag() {
    let Some(_) = a_registered_plugin("a_plugin_definition_renders", "com.lumitlab.testplug", 2)
    else {
        return;
    };
    let def = lumit_core::fx::BUILTIN_DEFS
        .get("ofx:com.lumitlab.testplug")
        .expect("registered");

    // Gain of two, set on the instance exactly as the panel would set it.
    let mut inst = lumit_core::fx::instantiate("ofx:com.lumitlab.testplug").expect("instantiated");
    for param in &mut inst.params {
        if param.id == "gain" {
            param.value = EffectValue::Float(lumit_core::anim::Property::fixed(2.0));
        }
    }
    let stack = lumit_core::fx::resolve_stack(
        std::slice::from_ref(&inst),
        0.0,
        1000.0,
        1.0,
        &lumit_core::fx::MarkerContext::NONE,
        std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
    );
    let op = stack.get(0).expect("the plugin resolved to an op");
    assert_eq!(
        op.params.get(lumit_core::fx::ParamId::new("gain")),
        Some(Value::Float(2.0))
    );

    let mut rgba: Vec<f32> = (0..4 * 4 * 4).map(|i| (i as f32) / 64.0).collect();
    let before = rgba.clone();
    def.apply_cpu_at(inst.id, 0.0, &mut rgba, 4, 4, op.params);
    assert_eq!(def.last_error(), None, "the plugin rendered");
    for (out, was) in rgba.iter().zip(&before) {
        assert!(
            (*out - was * 2.0).abs() < 0.01,
            "expected {} doubled, got {out}",
            was
        );
    }
}

/// **A disabled plugin renders identity, byte for byte** (docs/12 §2.3): the
/// layer keeps compositing and wears a badge, and the picture is not so much
/// as rounded on its way past.
#[test]
fn a_disabled_plugin_renders_identity_byte_for_byte() {
    // A declaration of its own, so this test needs no bundle at all: what is
    // under test is what the definition does when its host will not render.
    let schema: &'static EffectSchema = Box::leak(Box::new(EffectSchema {
        match_name: "ofx:test.disabled",
        label: "Disabled test plugin",
        version: 1,
        category: lumit_core::fx::FxCategory::Utility,
        traits: lumit_core::fx::EffectTraits {
            cost: lumit_core::fx::CostClass::Heavy,
            roi: lumit_core::fx::Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[],
        groups: &[],
        enabled_when: &[],
        matte: lumit_core::fx::MatteRole::None,
    }));
    let descriptor = PluginDescriptor {
        identifier: "test.disabled".to_owned(),
        version: (1, 0),
        grouping: String::new(),
        label: "Disabled test plugin".to_owned(),
        contexts: vec![Context::Filter],
        params: Vec::new(),
        clips: Vec::new(),
        temporal: false,
        render_thread_safety: None,
    };
    let def = OfxEffectDef::new(&descriptor, schema, std::sync::Arc::new(DeadHost));

    // Values chosen so that a trip through the fp16 boundary would change them:
    // an identity that rounds is not an identity.
    let before: Vec<f32> = vec![
        0.100_000_1,
        1.000_000_1,
        2.500_003,
        0.999_999_9,
        -0.25,
        3.7,
        0.0,
        1.0,
        0.333_333_34,
        0.666_666_7,
        12.345_678,
        0.5,
    ];
    let mut rgba = before.clone();
    def.apply_cpu(&mut rgba, 3, 1, Params::EMPTY);
    assert_eq!(rgba, before, "a disabled plugin changed the picture");
    assert_eq!(
        def.last_error().as_deref(),
        Some("the plugin is disabled for this session"),
        "the failure is reported for the badge"
    );
    // Read once, then gone: a stale reason must never badge a later frame.
    assert_eq!(def.last_error(), None);
}

// ------------------------------------------------------------------ presses --

/// A press goes to the plugin with the frame in place, and what the plugin
/// wrote comes back: a row it changed as a row, and the blob no row carries as
/// memory. The test plugin's Trigger writes both, the way Looks stores a look.
#[test]
fn a_pressed_button_brings_back_what_the_plugin_wrote() {
    let _ledger = image_ledger();
    let Some(schema) =
        a_registered_plugin("a_pressed_button_brings_back", "com.lumitlab.testplug", 2)
    else {
        return;
    };
    let def = lumit_core::fx::BUILTIN_DEFS
        .get(schema.match_name)
        .expect("registered");
    let inst = lumit_core::fx::instantiate(schema.match_name).expect("instantiated");
    let rgba = vec![255u8; 4 * 4 * 4];
    let source = lumit_core::fx::PressFrame {
        rgba: &rgba,
        width: 4,
        height: 4,
    };

    let pressed = def
        .press(&inst, 0.0, "trigger", &source)
        .expect("the press went through");
    assert_eq!(
        pressed.rows,
        vec![(
            "gain",
            Value::Float(lumit_ofx_testplug::TRIGGERED_GAIN as f32)
        )],
        "the one row the plugin changed, and only that one"
    );
    let memory = pressed
        .memory
        .expect("the blob has no row, so it is memory");
    let memory: ParamSnapshot = bincode::deserialize(&memory).expect("a snapshot");
    assert_eq!(
        memory.get("vendorBlob"),
        Some(&PropValue::String(vec![
            lumit_ofx_testplug::TRIGGERED_BLOB.to_owned()
        ])),
        "the blob came back as memory"
    );
    assert_eq!(memory.get("gain"), None, "a row is never also memory");

    // A button the plugin has not got is a sentence, not a press.
    assert!(def.press(&inst, 0.0, "nothing", &source).is_err());
}

/// What a plugin keeps beyond its rows reaches its render: the memory on the
/// document's instance is laid over the values the plugin is rendered with,
/// once the resolve walk has seen the instance, and comes off again when the
/// document forgets it.
#[test]
fn a_plugins_memory_reaches_its_render() {
    struct Recording(std::sync::Mutex<Option<ParamSnapshot>>);

    impl PluginHost for Recording {
        fn render(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            params: &ParamSnapshot,
            source: Frame16,
            _neighbours: &[(i32, Frame16)],
        ) -> Rendering {
            *self.0.lock().expect("the record") = Some(params.clone());
            Rendering {
                frame: source,
                error: None,
            }
        }

        fn frames_needed(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            _params: &ParamSnapshot,
        ) -> Option<Vec<i32>> {
            None
        }

        fn press(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            _params: &ParamSnapshot,
            _name: &str,
            _source: Frame16,
        ) -> Result<ParamSnapshot, String> {
            Err("not this test".to_owned())
        }
    }

    // One parameter with no row, so there is something only memory can carry.
    let descriptor = PluginDescriptor {
        identifier: "test.memory".to_owned(),
        version: (1, 0),
        grouping: String::new(),
        label: "Memory test plugin".to_owned(),
        contexts: vec![Context::Filter],
        params: vec![crate::describe::ParamDescription {
            name: "vendorBlob".to_owned(),
            param_type: crate::ffi::param_types::CUSTOM.to_owned(),
            props: PropertySet::new(),
        }],
        clips: Vec::new(),
        temporal: false,
        render_thread_safety: None,
    };
    let schema: &'static EffectSchema = Box::leak(Box::new(
        crate::schema::schema_of(&descriptor).expect("a schema"),
    ));
    let host = std::sync::Arc::new(Recording(std::sync::Mutex::new(None)));
    let def = OfxEffectDef::new(&descriptor, schema, host.clone());
    let recorded = || {
        host.0
            .lock()
            .expect("the record")
            .clone()
            .expect("a render happened")
    };

    let mut inst = lumit_core::model::EffectInstance {
        id: uuid::Uuid::now_v7(),
        effect: lumit_core::model::EffectKey {
            namespace: lumit_core::model::EffectNamespace::Ofx,
            match_name: schema.match_name.to_owned(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: Vec::new(),
        sample_temporally: true,
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
        roto: None,
        extra: serde_json::Map::new(),
    };
    let mut kept = ParamSnapshot::new();
    kept.set("vendorBlob", PropValue::string("kept").expect("a string"));
    inst.set_plugin_state(&bincode::serialize(&kept).expect("bytes"));

    let mut rgba = vec![0.5f32; 2 * 2 * 4];
    let resolve = |inst: &lumit_core::model::EffectInstance| {
        let cx = lumit_core::fx::ResolveCx {
            inst,
            lt: 0.0,
            diag_px: 100.0,
            px_scale: 1.0,
            markers: &lumit_core::fx::MarkerContext::NONE,
            context: std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        };
        let mut pushed = Vec::new();
        def.resolve_derived(&cx, &mut |id, value| pushed.push((id, value)));
        pushed
    };

    // Nothing has resolved this instance yet, so the render sees no memory.
    def.apply_cpu_at(inst.id, 0.0, &mut rgba, 2, 2, Params::EMPTY);
    assert_eq!(recorded().get("vendorBlob"), None);

    // The resolve files the memory and puts its hash in the bag.
    let pushed = resolve(&inst);
    assert_eq!(
        pushed.len(),
        1,
        "the memory's hash rides in the bag: {pushed:?}"
    );
    assert_ne!(pushed[0].1, Value::Int(0));
    def.apply_cpu_at(inst.id, 0.0, &mut rgba, 2, 2, Params::EMPTY);
    assert_eq!(
        recorded().get("vendorBlob"),
        Some(&PropValue::string("kept").expect("a string")),
        "the render was handed the memory"
    );

    // Forgotten in the document, forgotten in the render, and the bag says so.
    inst.plugin_state = None;
    assert_eq!(resolve(&inst)[0].1, Value::Int(0));
    def.apply_cpu_at(inst.id, 0.0, &mut rgba, 2, 2, Params::EMPTY);
    assert_eq!(recorded().get("vendorBlob"), None);
}

/// **The plugin is told its frame, and handed the frames either side.** The
/// resolve walk speaks in seconds and OFX counts in frames, so the comp's rate
/// turns the layer time into the frame the bag carries. The neighbours the
/// pass read back go to the host by offset, beside the picture, with one of
/// another size left out.
#[test]
fn a_plugin_is_told_its_frame_and_handed_its_neighbours() {
    use lumit_core::expression::ExpressionContext;
    use lumit_core::fx::{MarkerContext, ResolveCx};
    use lumit_core::model::{Composition, Document, LinearColour, ProjectItem};
    use lumit_core::time::{Duration, FrameRate, Rational};

    type Seen = (f64, Vec<(i32, f32)>);
    struct Seeing(std::sync::Mutex<Option<Seen>>);

    impl PluginHost for Seeing {
        fn render(
            &self,
            _instance: uuid::Uuid,
            time: f64,
            _params: &ParamSnapshot,
            source: Frame16,
            neighbours: &[(i32, Frame16)],
        ) -> Rendering {
            let seen = neighbours
                .iter()
                .map(|(offset, frame)| (*offset, frame.pixel(0, 0)[0]))
                .collect();
            *self.0.lock().expect("the record") = Some((time, seen));
            Rendering {
                frame: source,
                error: None,
            }
        }

        fn frames_needed(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            _params: &ParamSnapshot,
        ) -> Option<Vec<i32>> {
            None
        }

        fn press(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            _params: &ParamSnapshot,
            _name: &str,
            _source: Frame16,
        ) -> Result<ParamSnapshot, String> {
            Err("not this test".to_owned())
        }
    }

    let descriptor = PluginDescriptor {
        identifier: "test.frame".to_owned(),
        version: (1, 0),
        grouping: String::new(),
        label: "Frame test plugin".to_owned(),
        contexts: vec![Context::Filter],
        params: Vec::new(),
        clips: Vec::new(),
        temporal: true,
        render_thread_safety: None,
    };
    let schema: &'static EffectSchema = Box::leak(Box::new(
        crate::schema::schema_of(&descriptor).expect("a schema"),
    ));
    let host = std::sync::Arc::new(Seeing(std::sync::Mutex::new(None)));
    let def = OfxEffectDef::new(&descriptor, schema, host.clone());
    let seen = || {
        host.0
            .lock()
            .expect("the record")
            .clone()
            .expect("a render happened")
    };
    let inst = lumit_core::model::EffectInstance {
        id: uuid::Uuid::now_v7(),
        effect: lumit_core::model::EffectKey {
            namespace: lumit_core::model::EffectNamespace::Ofx,
            match_name: schema.match_name.to_owned(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: Vec::new(),
        sample_temporally: true,
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
        roto: None,
        extra: serde_json::Map::new(),
    };

    // A comp at sixty frames a second, resolved half a second in: frame 30.
    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: uuid::Uuid::now_v7(),
        name: "c".into(),
        width: 16,
        height: 16,
        frame_rate: FrameRate::new(60, 1).expect("a rate"),
        duration: Duration(Rational::new(10, 1).expect("a length")),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    let mut document = Document::new();
    document.items.push(ProjectItem::Composition(comp));
    let cx = ResolveCx {
        inst: &inst,
        lt: 0.5,
        diag_px: 100.0,
        px_scale: 1.0,
        markers: &MarkerContext::NONE,
        context: std::sync::Arc::new(ExpressionContext {
            document: std::sync::Arc::new(document),
            comp: Some(comp_id),
            layer: None,
            comp_time: 0.5,
            current_depth: 0,
        }),
    };
    let mut bag = Vec::new();
    def.resolve_derived(&cx, &mut |id, value| bag.push((id, value)));

    let mut rgba = vec![0.5_f32; 2 * 2 * 4];
    let previous = vec![0.25_f32; 2 * 2 * 4];
    let next = vec![0.75_f32; 2 * 2 * 4];
    let short = vec![1.0_f32; 4];
    def.apply_cpu_temporal(
        inst.id,
        0.5,
        &mut rgba,
        2,
        2,
        Params::new(&bag),
        &[(-1, &previous), (1, &next), (2, &short)],
    );
    let (time, neighbours) = seen();
    assert_eq!(time, 30.0, "half a second at sixty a second is frame 30");
    assert_eq!(
        neighbours.len(),
        2,
        "a neighbour of another size is left out: {neighbours:?}"
    );
    assert_eq!(neighbours[0].0, -1);
    assert!((neighbours[0].1 - 0.25).abs() < 1e-3);
    assert_eq!(neighbours[1].0, 1);
    assert!((neighbours[1].1 - 0.75).abs() < 1e-3);

    // A stack built by hand names no comp, and the time it gives stands.
    def.apply_cpu_at(inst.id, 7.0, &mut rgba, 2, 2, Params::EMPTY);
    let (time, neighbours) = seen();
    assert_eq!(time, 7.0);
    assert!(neighbours.is_empty());
}

// --------------------------------------------------------- console windows --

/// A broker is a console program and Lumit is a windowed one, so on Windows a
/// spawn without `CREATE_NO_WINDOW` opens a console window per plugin file
/// during the start-up scan — reported against 0.3.0. Nothing in this process
/// can observe whether a child was given a console, so the guard is that the
/// spawn still asks for none.
#[test]
fn the_broker_is_spawned_without_a_console_window() {
    let source = include_str!("ipc/broker.rs");
    assert!(
        source.contains("no_console(&mut command);"),
        "the broker spawn must ask for no console window"
    );
    assert!(
        source.contains("command.creation_flags(CREATE_NO_WINDOW);"),
        "no_console must be CREATE_NO_WINDOW and nothing else"
    );
}

/// The host keeps its brokers in a static, and a static is never dropped, so
/// a ring file only ever went away by luck. The maker's handle now carries
/// delete-on-close: the file must be gone once the process that made it has
/// ended, dropped or not. The probe is a child copy of this test binary that
/// makes a ring, forgets it, and exits.
#[cfg(windows)]
#[test]
fn a_forgotten_ring_goes_with_its_process() {
    const PROBE: &str = "LUMIT_OFX_RING_PROBE";
    if let Ok(path) = std::env::var(PROBE) {
        let ring = crate::ipc::shm::Ring::create(Path::new(&path), 8, 8).expect("a ring");
        std::mem::forget(ring);
        std::process::exit(0);
    }
    let path = std::env::temp_dir().join(format!("lumit-ofx-probe-{}.ring", std::process::id()));
    let status = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "a_forgotten_ring_goes_with_its_process",
            "--exact",
            "--test-threads=1",
        ])
        .env(PROBE, &path)
        .status()
        .expect("the probe runs");
    assert!(status.success(), "the probe made its ring and exited");
    let leaked = path.exists();
    let _ = std::fs::remove_file(&path);
    assert!(
        !leaked,
        "the ring file outlived the process that made it: {}",
        path.display()
    );
}
