//! The host: what Lumit tells a plugin about itself, and the one piece of
//! state every suite call reaches through.
//!
//! # In plain terms
//!
//! A plugin's first question is "what sort of host am I in?", and it asks by
//! reading a property set we fill in before it ever runs. The answers must be
//! **true**. Saying we support tiles when we render full frames, or several
//! pixel depths when we hand out float, is the classic host bug: the plugin
//! believes us, asks for something we cannot do, and the crash lands on the
//! host's side of the line. So the table below says no to everything this
//! version does not do, including its own overlays.
//!
//! The plugin holds one pointer to that host struct for as long as it is
//! loaded, and the C API gives us nowhere to hang a context, so the state
//! behind it is process-wide: one mutex, taken and released inside each suite
//! call. It is never held while calling *into* a plugin — a plugin re-enters
//! the suites from inside an action, and a lock held across that call would
//! deadlock the first plugin that read a property (docs/14 §7).

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use crate::describe::{ClipBinding, EffectDescriptor, ParamRecord};
use crate::ffi::{prop_keys as keys, prop_values as values, suite_names, OfxHost};
use crate::handles::{Handle, HandleKind, HandleRegistry};
use crate::props::{PropValue, PropertySet};
use crate::status::Status;
use crate::suites;

/// How many messages from plugins are kept. The message suite has nowhere to
/// put them until the broker forwards them to the UI, and an unbounded log is
/// a memory leak a chatty plugin controls (docs/14 §5).
const MESSAGE_LOG_CAPACITY: usize = 64;

/// One message a plugin asked us to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMessage {
    /// One of [`crate::ffi::message_types`].
    pub message_type: String,
    /// The plugin's own identifier for the message, often empty.
    pub message_id: String,
    /// The format string, verbatim (see [`crate::ffi::OfxMessageSuiteV1`]).
    pub text: String,
}

/// Everything the suites read and write.
pub struct HostState {
    /// Every property set the host owns, including its own.
    pub props: HandleRegistry<PropertySet>,
    /// Every effect descriptor a describe pass has minted, and every instance:
    /// they are one kind of handle, so they are one registry
    /// ([`EffectDescriptor`]).
    pub effects: HandleRegistry<EffectDescriptor>,
    /// Every parameter: its property set, and the effect and name that let a
    /// value be looked up in that effect's snapshot.
    pub params: HandleRegistry<ParamRecord>,
    /// Every clip handle an instance has minted. A descriptor's clips are not
    /// in here: a clip handle names a live image input, and a description has
    /// none.
    pub clips: HandleRegistry<ClipBinding>,
    /// The host's own property set, once it exists.
    pub host_props: Option<Handle>,
    /// Every lock a plugin asked the host to keep for it
    /// ([`crate::suites::multi_thread`]).
    pub mutexes: HandleRegistry<std::sync::Arc<suites::multi_thread::HostMutex>>,
    /// Live plugin allocations: address to size. Keeping the addresses means
    /// `memoryFree` can reject one we never handed out **without following
    /// it**, which is the only safe answer to a forged pointer.
    pub allocations: std::collections::BTreeMap<usize, usize>,
    /// The most recent messages, oldest first.
    pub messages: Vec<HostMessage>,
}

impl HostState {
    fn new() -> Self {
        let mut props = HandleRegistry::new(HandleKind::PropertySet);
        // The host's own set is the first thing in the registry; if that
        // insert could fail the registry would have to be full, which it
        // cannot be one line after it was created.
        let host_props = props.insert(host_property_set()).ok();
        Self {
            props,
            effects: HandleRegistry::new(HandleKind::ImageEffect),
            params: HandleRegistry::new(HandleKind::Param),
            clips: HandleRegistry::new(HandleKind::Clip),
            mutexes: HandleRegistry::new(HandleKind::Mutex),
            host_props,
            allocations: std::collections::BTreeMap::new(),
            messages: Vec::new(),
        }
    }

    /// Record a message, dropping the oldest once the log is full.
    pub fn push_message(&mut self, message: HostMessage) {
        if self.messages.len() >= MESSAGE_LOG_CAPACITY {
            self.messages.remove(0);
        }
        self.messages.push(message);
    }

    /// Take the log, leaving it empty — what the frontend does when it has
    /// drawn them, so a plugin's message becomes one calm toast rather than one
    /// per poll (docs/12 §2.2).
    pub fn take_messages(&mut self) -> Vec<HostMessage> {
        std::mem::take(&mut self.messages)
    }
}

static STATE: OnceLock<Mutex<HostState>> = OnceLock::new();

/// The host state, locked. A poisoned mutex is recovered rather than
/// propagated: the state is a set of plain values, a panic cannot have left it
/// half-written, and a plugin waiting on a suite call has no way to be told
/// about a poisoning anyway.
pub fn state() -> MutexGuard<'static, HostState> {
    STATE
        .get_or_init(|| Mutex::new(HostState::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The handle of the host's own property set.
///
/// # Errors
///
/// [`Status::ErrFatal`] if the host state could not be built, which cannot
/// happen in practice and is reported rather than asserted.
pub fn host_props_handle() -> Result<Handle, Status> {
    state().host_props.ok_or(Status::ErrFatal)
}

/// The `OfxHost` a plugin is given. Deliberately leaked: the plugin keeps this
/// pointer for as long as it is loaded and the host must outlive everything
/// (docs/impl/ofx-host.md §1).
struct HostPtr(*const OfxHost);

// SAFETY: the pointed-to `OfxHost` is written once, before any plugin can see
// it, and never mutated again. Its `host` field is a handle (an integer in
// pointer's clothing), not a reference to anything, so sharing the pointer
// across threads shares no mutable state. The upholder is this module: nothing
// outside it can obtain a `&mut OfxHost`.
unsafe impl Send for HostPtr {}
// SAFETY: as above.
unsafe impl Sync for HostPtr {}

static HOST: OnceLock<HostPtr> = OnceLock::new();

/// The host struct, built once.
#[must_use]
pub fn host() -> *const OfxHost {
    HOST.get_or_init(|| {
        let handle = host_props_handle().map_or(std::ptr::null_mut(), Handle::as_ptr);
        HostPtr(Box::into_raw(Box::new(OfxHost {
            host: handle,
            fetch_suite: Some(fetch_suite),
        })))
    })
    .0
}

/// `OfxHost::fetchSuite`.
///
/// Returning null is a legitimate answer, and this host gives it for every
/// suite it has not built yet — including the interact suite, which is how
/// overlays degrade to no overlay instead of to a crash.
unsafe extern "C" fn fetch_suite(
    _host: *mut c_void,
    suite_name: *const c_char,
    suite_version: c_int,
) -> *const c_void {
    if suite_name.is_null() {
        return std::ptr::null();
    }
    // SAFETY: the plugin passes a NUL-terminated C string literal from its own
    // image; we checked for null and read it without keeping the borrow.
    let name = match unsafe { CStr::from_ptr(suite_name) }.to_str() {
        Ok(name) => name,
        Err(_) => return std::ptr::null(),
    };
    match (name, suite_version) {
        (suite_names::PROPERTY, 1) => std::ptr::from_ref(&suites::property::SUITE).cast(),
        (suite_names::MEMORY, 1) => std::ptr::from_ref(&suites::memory::SUITE).cast(),
        (suite_names::MESSAGE, 1) => std::ptr::from_ref(&suites::message::SUITE).cast(),
        (suite_names::IMAGE_EFFECT, 1) => std::ptr::from_ref(&suites::image_effect::SUITE).cast(),
        (suite_names::PARAMETER, 1) => std::ptr::from_ref(&suites::parameter::SUITE).cast(),
        (suite_names::MULTI_THREAD, 1) => std::ptr::from_ref(&suites::multi_thread::SUITE).cast(),
        _ => std::ptr::null(),
    }
}

/// The host's property set: the honest answers.
///
/// Every `0` here is a promise this version keeps by not doing the thing.
/// Tiles are off because the render pipeline is full-frame
/// (docs/06 and docs/impl/ofx-host.md §2); temporal clip access is on because
/// retimer-class plugins cannot work without it; the depth is float and only
/// float, and the components are RGBA and only RGBA, which is what the
/// working space converts to at the boundary (docs/12 §2.1).
fn host_property_set() -> PropertySet {
    /// A key or value with a NUL in it would be a literal in this file with a
    /// NUL in it; there is nothing to report to and nothing to do, so the
    /// property is simply not seeded and the golden test catches it.
    fn seed_string(set: &mut PropertySet, key: &str, value: &str) {
        if let Ok(value) = PropValue::string(value) {
            set.seed(key, value);
        }
    }

    let mut set = PropertySet::new();
    seed_string(&mut set, keys::TYPE, values::TYPE_IMAGE_EFFECT_HOST);
    seed_string(&mut set, keys::NAME, "com.lumitlab.Lumit");
    seed_string(&mut set, keys::LABEL, "Lumit");
    seed_string(&mut set, keys::VERSION_LABEL, env!("CARGO_PKG_VERSION"));

    set.seed(keys::VERSION, PropValue::Int(package_version()));
    // Spec 1.4 semantics (docs/12 §2.1).
    set.seed(keys::API_VERSION, PropValue::Int(vec![1, 4]));

    if let Ok(components) = PropValue::strings(&[values::COMPONENT_RGBA]) {
        set.seed(keys::SUPPORTED_COMPONENTS, components);
    }
    if let Ok(depths) = PropValue::strings(&[values::BIT_DEPTH_FLOAT]) {
        set.seed(keys::SUPPORTED_PIXEL_DEPTHS, depths);
    }
    if let Ok(contexts) = PropValue::strings(&[
        values::CONTEXT_FILTER,
        values::CONTEXT_GENERAL,
        values::CONTEXT_GENERATOR,
        values::CONTEXT_TRANSITION,
    ]) {
        set.seed(keys::SUPPORTED_CONTEXTS, contexts);
    }

    // The GPU render extensions, answered as the false they are. Saying nothing
    // is worse than saying no: a plugin whose framework cannot read the answer
    // does not fall back, it simply stops drawing (see `render_args`).
    for key in [
        keys::CUDA_RENDER_SUPPORTED,
        keys::CUDA_STREAM_SUPPORTED,
        keys::OPENCL_RENDER_SUPPORTED,
        keys::OPENCL_SUPPORTED,
        keys::METAL_RENDER_SUPPORTED,
    ] {
        seed_string(&mut set, key, "false");
    }

    // Lumit is an interactive application, not a render farm node.
    set.seed(keys::HOST_IS_BACKGROUND, PropValue::int(0));
    // No interact suite yet, so no overlays. Saying otherwise would have
    // plugins draw into a viewer that never asks them to.
    set.seed(keys::SUPPORTS_OVERLAYS, PropValue::int(0));
    set.seed(keys::SUPPORTS_MULTI_RESOLUTION, PropValue::int(0));
    set.seed(keys::SUPPORTS_TILES, PropValue::int(0));
    set.seed(keys::TEMPORAL_CLIP_ACCESS, PropValue::int(1));
    set.seed(keys::SUPPORTS_MULTIPLE_CLIP_DEPTHS, PropValue::int(0));
    set.seed(keys::SUPPORTS_MULTIPLE_CLIP_PARS, PropValue::int(0));
    set.seed(keys::SETABLE_FRAME_RATE, PropValue::int(0));
    set.seed(keys::SETABLE_FIELDING, PropValue::int(0));
    set.seed(keys::SEQUENTIAL_RENDER, PropValue::int(0));

    // The parameter suite is a later package; every answer here is "not yet",
    // and each becomes a considered yes when the thing it names is built.
    set.seed(keys::PARAM_SUPPORTS_STRING_ANIMATION, PropValue::int(0));
    set.seed(keys::PARAM_SUPPORTS_CUSTOM_INTERACT, PropValue::int(0));
    set.seed(keys::PARAM_SUPPORTS_CHOICE_ANIMATION, PropValue::int(0));
    set.seed(keys::PARAM_SUPPORTS_BOOLEAN_ANIMATION, PropValue::int(1));
    set.seed(keys::PARAM_SUPPORTS_CUSTOM_ANIMATION, PropValue::int(0));
    set.seed(keys::PARAM_SUPPORTS_PARAMETRIC_ANIMATION, PropValue::int(0));
    // -1 is the OFX spelling of "no limit".
    set.seed(keys::PARAM_MAX_PARAMETERS, PropValue::int(-1));
    set.seed(keys::PARAM_MAX_PAGES, PropValue::int(0));
    set.seed(
        keys::PARAM_PAGE_ROW_COLUMN_COUNT,
        PropValue::Int(vec![0, 0]),
    );
    set
}

/// The crate version as OFX wants it: major, minor, patch as whole numbers.
fn package_version() -> Vec<i32> {
    env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<i32>().unwrap_or(0))
        .collect()
}

/// A stable, ordered dump of a property set, for the golden test and for
/// diagnostics. One property per line, `key = value`.
#[must_use]
pub fn dump(set: &PropertySet) -> String {
    let mut out = String::new();
    for key in set.keys() {
        let Ok(value) = set.get(key) else { continue };
        let rendered = match value {
            PropValue::Int(v) => v.iter().map(i32::to_string).collect::<Vec<_>>().join(", "),
            PropValue::Double(v) => v
                .iter()
                .map(|d| format!("{d:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            PropValue::String(v) => v
                .iter()
                .map(|s| format!("{:?}", s.to_string_lossy()))
                .collect::<Vec<_>>()
                .join(", "),
            PropValue::Pointer(v) => v
                .iter()
                .map(|p| format!("{p:#x}"))
                .collect::<Vec<_>>()
                .join(", "),
        };
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(&rendered);
        out.push('\n');
    }
    out
}
