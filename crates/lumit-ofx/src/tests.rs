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

use crate::bundle::{scan_dir, Bundle, BUNDLE_ARCH_DIR};
use crate::ffi::{
    prop_keys as keys, OfxHost, OfxMemorySuiteV1, OfxMessageSuiteV1, OfxPlugin, OfxPropertySuiteV1,
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

    // Not built yet, and an honest null is the whole point: overlays degrade
    // to no overlay rather than to a crash.
    assert!(ask(c"OfxInteractSuite", 1).is_null());
    assert!(ask(c"OfxImageEffectSuite", 1).is_null());
    assert!(ask(c"OfxParameterSuite", 1).is_null());
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
    assert_eq!(bundle.plugins().len(), 1);
    let plugin = &bundle.plugins()[0];
    assert_eq!(plugin.identifier, "com.lumitlab.testplug");
    assert_eq!(plugin.version, (1, 0));
    assert!(plugin.is_supported_image_effect());
    // Opening is not loading: nothing has been said to the plugin yet.
    assert_eq!(read(b"LumitTestPlugSetHostCalls\0"), 0);

    bundle.load();
    assert_eq!(bundle.plugins()[0].load_status, Some(Status::Ok));
    assert_eq!(read(b"LumitTestPlugSetHostCalls\0"), 1);
    assert_eq!(read(b"LumitTestPlugHostSeenBeforeLoad\0"), 1);

    // Loading twice does not hand the plugin a second host.
    bundle.load();
    assert_eq!(read(b"LumitTestPlugSetHostCalls\0"), 1);

    // The plugin got the three suites this host has and not the one it has
    // not, which is the whole of `fetchSuite` proved from the plugin's side.
    let mask = read(b"LumitTestPlugSuiteMask\0");
    let expected = lumit_ofx_testplug::SUITE_PROPERTY
        | lumit_ofx_testplug::SUITE_MEMORY
        | lumit_ofx_testplug::SUITE_MESSAGE;
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
