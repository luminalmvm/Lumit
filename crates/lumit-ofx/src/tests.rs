//! The host's own tests.
//!
//! The handle tests are the ones that matter most: they are the seed corpus
//! for the sanitiser run that comes with the out-of-process broker
//! (docs/impl/ofx-host.md §5 item 2). Every one of them hands a suite entry
//! point a handle it was never given and checks that the answer is a status
//! code and that nothing moved.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};

use lumit_core::fx::EffectSchema;

use crate::bundle::{scan_dir, Bundle, BUNDLE_ARCH_DIR};
use crate::describe::{describe_bundle, Context, DescribedPlugin, Rejection, ScanReport};
use crate::ffi::{
    prop_keys as keys, OfxHost, OfxImageEffectSuiteV1, OfxMemorySuiteV1, OfxMessageSuiteV1,
    OfxParameterSuiteV1, OfxPlugin, OfxPropertySuiteV1,
};
use crate::handles::{Handle, HandleKind, HandleRegistry};
use crate::host::{dump, host, host_props_handle, state};
use crate::props::{Element, PropValue, PropertySet};
use crate::quirks::QuirksTable;
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
    let handle = host_props_handle().expect("the host has its own property set");
    let state = state();
    let set = state.props.get(handle).expect("the host set is live");

    let expected = format!(
        "\
OfxImageEffectHostPropIsBackground = 0
OfxImageEffectInstancePropSequentialRender = 0
OfxImageEffectPropSetableFielding = 0
OfxImageEffectPropSetableFrameRate = 0
OfxImageEffectPropSupportedComponents = \"OfxImageComponentRGBA\"
OfxImageEffectPropSupportedContexts = \"OfxImageEffectContextFilter\", \
\"OfxImageEffectContextGeneral\", \"OfxImageEffectContextGenerator\", \
\"OfxImageEffectContextTransition\"
OfxImageEffectPropSupportedPixelDepths = \"OfxBitDepthFloat\"
OfxImageEffectPropSupportsMultiResolution = 0
OfxImageEffectPropSupportsMultipleClipDepths = 0
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
OfxParamHostPropSupportsStringAnimation = 0
OfxPropAPIVersion = 1, 4
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

    // Not built yet, and an honest null is the whole point: overlays degrade
    // to no overlay rather than to a crash.
    assert!(ask(c"OfxInteractSuite", 1).is_null());
    assert!(ask(c"OfxMultiThreadSuite", 1).is_null());

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

    // The plugin got the five suites this host has and not the one it has not,
    // which is the whole of `fetchSuite` proved from the plugin's side.
    let mask = read(b"LumitTestPlugSuiteMask\0");
    let expected = lumit_ofx_testplug::SUITE_PROPERTY
        | lumit_ofx_testplug::SUITE_MEMORY
        | lumit_ofx_testplug::SUITE_MESSAGE
        | lumit_ofx_testplug::SUITE_IMAGE_EFFECT
        | lumit_ofx_testplug::SUITE_PARAMETER;
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
    bundle.load();
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
    // it into one (K-443), and the convention applies to a plugin's rows
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
    // A `ParamId` collision is silent (docs/impl/effect-registry.md Â§5), so it
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

        // The value half is not here yet, and says so rather than lying.
        assert_eq!(
            (params.param_get_value)(handle),
            Status::ErrUnsupported.code()
        );
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

#[test]
fn the_instance_half_of_the_image_effect_suite_says_it_is_not_here() {
    let suite = &crate::suites::image_effect::SUITE;
    let mut out: *mut c_void = std::ptr::null_mut();
    let mut bounds = crate::ffi::OfxRectD::default();

    // SAFETY: valid out-parameters; each of these is a stub that must answer a
    // status a plugin expects rather than pretend.
    unsafe {
        assert_eq!(
            (suite.clip_get_property_set)(std::ptr::null_mut(), &raw mut out),
            Status::ErrUnsupported.code()
        );
        assert_eq!(
            (suite.clip_get_image)(std::ptr::null_mut(), 0.0, std::ptr::null(), &raw mut out),
            Status::ErrUnsupported.code()
        );
        assert_eq!(
            (suite.clip_get_region_of_definition)(std::ptr::null_mut(), 0.0, &raw mut bounds),
            Status::ErrUnsupported.code()
        );
        // `abort` is the one that is not a status, and nought is what lets a
        // render carry on. Nothing is cancellable yet, so nothing is cancelled.
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
