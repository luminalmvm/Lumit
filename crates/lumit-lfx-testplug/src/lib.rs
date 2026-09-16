//! `lumit-lfx-testplug` - minimal LFX plugins, for testing the host
//! (docs/impl/lfx.md §10).
//!
//! # In plain terms
//!
//! A host cannot be tested against nothing, and it should not be tested only
//! against somebody else's plugin: a commercial plugin cannot be shipped in a
//! repository, and a free one changes underneath the tests. So this is a plugin
//! of our own - the smallest thing that is genuinely an LFX bundle. It exports
//! the one symbol the header names, answers how many effects it holds, hands
//! back a descriptor for each, makes instances, declares controls into the
//! sink, and processes frames at both depths.
//!
//! There are **twelve personalities in one table** ([`Personality`]), because
//! the host has twelve kinds of answer to give and each needs something to give
//! it to: an effect with one control of every admitted kind, one with a single
//! control and no trait block at all, one that reads its neighbours, one that
//! refuses to describe, one whose controls collide, one that hands its input
//! back untouched, one that writes nothing at all, one that says two of its
//! processes may never run at once, one that requires an extension the host has
//! not got, and the three dangerous ones - crash, hang, and say too much.
//!
//! **The dangerous three are disarmed unless an environment variable is set**
//! ([`CRASH_ON_FRAME_ENV`], [`HANG_ENV`], [`NOTE_SPAM_ENV`]), and the variable
//! has to be set in the environment of the process that loads the bundle - the
//! broker's, in the shipping path. A scan describes every plugin in a bundle,
//! so a personality that crashed on sight would take the whole suite with it.
//!
//! It also carries a few exports of its own - names beginning `LumitLfxProbe` -
//! which no real plugin has. They are how a test asks what was seen: the
//! exact sequence of calls the host made, the depths it handed over, where each
//! frame said its buffer sat and which region of it was wanted, how many lines
//! the plugin itself sent, the most processes ever in flight at one moment, and
//! the most ever in flight **on one instance**, which is the number the header's
//! "one instance is never re-entered" is a claim about. A stress test that
//! asserts an absence proves nothing unless the fixture records the overlap
//! ([`LumitLfxProbeRendezvous`] is the barrier that makes the overlap
//! deliberate rather than hoped for, docs/impl/lfx.md §11 item 12). Four of
//! them make the bundle misbehave rather than watch it:
//! [`LumitLfxProbeRendezvous`]; [`LumitLfxProbeDeclaredCount`], which makes the
//! entry lie about how many effects it holds; [`LumitLfxProbeTableBytes`], which
//! makes an instance lie about how long its own table is; and
//! [`LumitLfxProbeShortTraits`], which hands over a trait block that really is
//! shorter than this header's - three things no honest personality can be, and
//! all of them faults the host must meet before it reads the bytes behind them.
//!
//! The processing is real. It reads the input frame's data pointer, bounds and
//! row bytes, honours the depth it was handed rather than assuming one, walks
//! the value array **by `lfx_process.value_stride` and never by its own
//! `size_of`** (docs/impl/lfx.md §11 item 15), and writes the output. That is
//! the whole of what a plugin does, and a host that survives it is a host.
//!
//! # Thread role
//!
//! The plugin's, as the header pins it: `init`, `describe` and the instance
//! lifecycle on the host's control thread, `process` on any worker thread and
//! on different instances at once. The probes are statics behind atomics and a
//! mutex, so every test in one process shares them - which is why the host's
//! suite takes a lock around anything it means to observe.

use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use lumit_lfx_abi::{
    LfxActionParam, LfxAngleParam, LfxBoolParam, LfxCategory, LfxChoiceParam, LfxColourParam,
    LfxCurveParam, LfxDescribeSink, LfxDescriptor, LfxEntry, LfxFileParam, LfxFloatParam,
    LfxGroupParam, LfxHost, LfxIntParam, LfxPlugin, LfxPoint2Param, LfxPoint3Param, LfxProcess,
    LfxSeedParam, LfxSliderParam, LfxTraits, LfxValue, LFX_ABI_VERSION, LFX_ALPHA_PREMULTIPLIED,
    LFX_BOUND_MAX, LFX_BOUND_MIN, LFX_CATEGORY_BLUR_SHARPEN, LFX_CATEGORY_COLOUR,
    LFX_CATEGORY_STYLISE, LFX_CATEGORY_TEMPORAL, LFX_CATEGORY_UTILITY, LFX_COST_CHEAP,
    LFX_COST_MODERATE, LFX_EXT_GPU_FRAMES, LFX_EXT_TEMPORAL, LFX_LOG_WARN, LFX_PARAM_FLAG_NONE,
    LFX_PARAM_FLOAT, LFX_RGBA_F16, LFX_RGBA_F32, LFX_ROI_EXACT, LFX_ROI_PADDED,
    LFX_STATUS_CANCELLED, LFX_STATUS_FAILED, LFX_STATUS_OK, LFX_TRAIT_CANCELLABLE,
    LFX_TRAIT_SEEDED, LFX_TRAIT_THREAD_UNSAFE, LFX_UNIT_DEGREES, LFX_UNIT_PERCENT, LFX_UNIT_PX,
    LFX_UNIT_RAW,
};
// ------------------------------------------------------------- the probes --

/// Every call the host has made since the log was last reset, comma separated.
/// Read through [`LumitLfxProbeLog`].
static CALL_LOG: Mutex<String> = Mutex::new(String::new());

/// Every depth the host has handed over, comma separated, in order. Read
/// through [`LumitLfxProbeDepths`].
static DEPTH_LOG: Mutex<String> = Mutex::new(String::new());

/// What `lfx_entry.init` was handed. Read through [`LumitLfxProbeBundlePath`].
static BUNDLE_PATH: Mutex<String> = Mutex::new(String::new());

/// Where each process call said its input buffer sat and which region of it was
/// wanted, comma separated, in order. Read through [`LumitLfxProbeRegions`].
static REGION_LOG: Mutex<String> = Mutex::new(String::new());

/// Processes in flight at this moment, across every instance.
static IN_FLIGHT: AtomicU32 = AtomicU32::new(0);
/// The most there have ever been; two means the host really overlapped them.
static MAX_IN_FLIGHT: AtomicU32 = AtomicU32::new(0);
/// The most there have ever been **on one instance**. One means no instance was
/// re-entered, which is the header's own promise.
static MAX_PER_INSTANCE: AtomicU32 = AtomicU32::new(0);
/// How many processes must be in flight before any of them may finish. Nought
/// turns the barrier off; see [`LumitLfxProbeRendezvous`].
static RENDEZVOUS: AtomicU32 = AtomicU32::new(0);
/// How many lines the plugin has put through `lfx_host.log`.
static NOTES_SENT: AtomicU32 = AtomicU32::new(0);
/// What `lfx_entry.count` answers instead of the honest number, or nought for
/// the honest one. See [`LumitLfxProbeDeclaredCount`].
static DECLARED_COUNT: AtomicU32 = AtomicU32::new(0);
/// What an instance's `struct_size` claims instead of the honest number, or
/// nought for the honest one. See [`LumitLfxProbeTableBytes`].
static TABLE_BYTES: AtomicU32 = AtomicU32::new(0);
/// Whether every descriptor points at the short trait block instead of its own.
/// See [`LumitLfxProbeShortTraits`].
static SHORT_TRAIT_BLOCK: AtomicU32 = AtomicU32::new(0);

/// How long a process waits at the barrier before giving up. A host that
/// serialises its calls never reaches the count, and the wait must end rather
/// than hang the suite.
const RENDEZVOUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Crash on this comp frame, without warning and without unwinding - which is
/// what a plugin with a bad pointer does. **Set in the environment of the
/// process that loads the bundle**, because a scan describes every plugin in a
/// bundle and this one must be inert until a test asks for it.
pub const CRASH_ON_FRAME_ENV: &str = "LUMIT_LFX_TESTPLUG_CRASH_ON_FRAME";

/// Never come back from a process call. The host's deadline is what ends it.
pub const HANG_ENV: &str = "LUMIT_LFX_TESTPLUG_HANG";

/// Say this many things through `lfx_host.log` during one process call. A
/// plugin in a loop is a plugin the host must not let fill its memory.
pub const NOTE_SPAM_ENV: &str = "LUMIT_LFX_TESTPLUG_NOTE_SPAM";

/// Write one entry into the call log.
fn record(log: &Mutex<String>, entry: &str) {
    let mut held = log.lock().unwrap_or_else(PoisonError::into_inner);
    if !held.is_empty() {
        held.push(',');
    }
    held.push_str(entry);
}

/// Note one call the host made.
fn note_call(entry: &str) {
    record(&CALL_LOG, entry);
}

/// Copy `text` into `buffer` as a NUL-terminated string and answer how long the
/// text actually is. A buffer too small is truncated, never overrun.
///
/// # Safety
///
/// `buffer` must be null or point at `capacity` writable bytes.
unsafe fn spill(text: &str, buffer: *mut c_char, capacity: u32) -> u32 {
    let bytes = text.as_bytes();
    let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    if buffer.is_null() || capacity == 0 {
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

/// Every call the host made since the last reset, comma separated.
///
/// # Safety
///
/// `buffer` must be null or point at `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn LumitLfxProbeLog(buffer: *mut c_char, capacity: u32) -> u32 {
    let log = CALL_LOG.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: the caller's contract, passed straight on.
    unsafe { spill(&log, buffer, capacity) }
}

/// Every depth the host handed over since the last reset, comma separated:
/// `f16` or `f32`, one per process call.
///
/// # Safety
///
/// `buffer` must be null or point at `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn LumitLfxProbeDepths(buffer: *mut c_char, capacity: u32) -> u32 {
    let log = DEPTH_LOG.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: the caller's contract, passed straight on.
    unsafe { spill(&log, buffer, capacity) }
}

/// The directory `lfx_entry.init` was handed, so a test can check the host gave
/// the bundle's own rather than the module's file name.
///
/// # Safety
///
/// `buffer` must be null or point at `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn LumitLfxProbeBundlePath(buffer: *mut c_char, capacity: u32) -> u32 {
    let path = BUNDLE_PATH.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: the caller's contract, passed straight on.
    unsafe { spill(&path, buffer, capacity) }
}

/// Where each process call said the input buffer's top-left pixel sat and which
/// region of it was asked for, one entry per call, comma separated:
/// `origin_x:origin_y:roi_x0:roi_y0:roi_x1:roi_y1`.
///
/// The header says `origin` is the buffer's own corner and the ROI is the
/// region wanted out of it, so a plugin finds the first requested pixel at
/// `roi_x0 - origin_x`. That subtraction is only right if the host wrote the
/// two from one rectangle, and this is how a test sees which two it wrote.
///
/// # Safety
///
/// `buffer` must be null or point at `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn LumitLfxProbeRegions(buffer: *mut c_char, capacity: u32) -> u32 {
    let log = REGION_LOG.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: the caller's contract, passed straight on.
    unsafe { spill(&log, buffer, capacity) }
}

/// The most process calls that were ever in flight at one moment since the last
/// reset. One means the host serialised them.
#[no_mangle]
pub extern "C" fn LumitLfxProbeMaxConcurrent() -> u32 {
    MAX_IN_FLIGHT.load(Ordering::SeqCst)
}

/// The most process calls ever in flight **on one instance**. One means no
/// instance was re-entered; anything above it is the host breaking the header's
/// own promise.
#[no_mangle]
pub extern "C" fn LumitLfxProbeMaxPerInstance() -> u32 {
    MAX_PER_INSTANCE.load(Ordering::SeqCst)
}

/// How many lines the plugin has put through `lfx_host.log` since the last
/// reset.
#[no_mangle]
pub extern "C" fn LumitLfxProbeNotesSent() -> u32 {
    NOTES_SENT.load(Ordering::SeqCst)
}

/// Make `lfx_entry.count` claim this many effects instead of the honest
/// [`PLUGIN_COUNT`]. Nought is the honest number.
///
/// A bundle that lies about how many effects it holds is the one thing no
/// honest personality can be, and the host's answer to a count past
/// `LFX_MAX_EFFECTS_PER_BUNDLE` has nowhere else to be proved: no bundle that
/// fits in a repository holds a thousand effects. The descriptors past the end
/// of the real table are null, which is what a bundle whose count runs ahead
/// of its list actually offers.
///
/// **Set it back to nought**: it is a static of the loaded module, so it
/// outlives the test that set it.
#[no_mangle]
pub extern "C" fn LumitLfxProbeDeclaredCount(count: u32) {
    DECLARED_COUNT.store(count, Ordering::SeqCst);
}

/// Make every instance's `lfx_plugin.struct_size` claim this many bytes instead
/// of the honest `sizeof`. Nought is the honest number.
///
/// The table itself is whole - what is short is the prefix, which is the only
/// thing a host may read before it trusts the rest. A host that formed a
/// reference over the table without reading the prefix would find every
/// function pointer where it expects one and never notice, which is exactly why
/// the lie has to be told from here rather than caught by inspection.
///
/// **Set it back to nought**: it is a static of the loaded module, so it
/// outlives the test that set it.
#[no_mangle]
pub extern "C" fn LumitLfxProbeTableBytes(bytes: u32) {
    TABLE_BYTES.store(bytes, Ordering::SeqCst);
}

/// Point every descriptor that declares traits at a trait block from an
/// earlier header - genuinely two words long, rather than a whole block lying
/// about its own length. Nought is each personality's own honest block.
///
/// It is a switch rather than a byte count because the lie is the allocation:
/// [`LumitLfxProbeTableBytes`] can say any number because the table behind it
/// is whole, and this one cannot, because what a host that formed a reference
/// over this block without reading the prefix first would read past is the end
/// of the object. A number would make the fault tellable and leave it
/// uncommitted.
///
/// **Set it back to nought**: it is a static of the loaded module, so it
/// outlives the test that set it.
#[no_mangle]
pub extern "C" fn LumitLfxProbeShortTraits(on: u32) {
    SHORT_TRAIT_BLOCK.store(on, Ordering::SeqCst);
}

/// Make every process call wait until `count` of them are in flight before any
/// finishes, so "did these two overlap?" is a question with a definite answer
/// rather than a race with a sleep in it. Nought turns it off.
#[no_mangle]
pub extern "C" fn LumitLfxProbeRendezvous(count: u32) {
    RENDEZVOUS.store(count, Ordering::SeqCst);
}

/// Forget the logs and the high-water marks, and turn the barrier off. A test
/// calls this immediately before the stretch it means to observe.
#[no_mangle]
pub extern "C" fn LumitLfxProbeReset() {
    CALL_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    DEPTH_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    REGION_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    IN_FLIGHT.store(0, Ordering::SeqCst);
    MAX_IN_FLIGHT.store(0, Ordering::SeqCst);
    MAX_PER_INSTANCE.store(0, Ordering::SeqCst);
    RENDEZVOUS.store(0, Ordering::SeqCst);
    NOTES_SENT.store(0, Ordering::SeqCst);
    DECLARED_COUNT.store(0, Ordering::SeqCst);
    TABLE_BYTES.store(0, Ordering::SeqCst);
    SHORT_TRAIT_BLOCK.store(0, Ordering::SeqCst);
}

// ------------------------------------------------------ the personalities --

/// What one effect in this bundle is for.
///
/// One table, in this order, because the descriptor list a host reads is this
/// list and the host's own suite names each of them by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Personality {
    /// One control of every kind version 1 admits, under one heading, with a
    /// full trait block. Its process multiplies the picture by its first
    /// control, so a test can prove a declared value reached `process`.
    Full,
    /// One control and **no trait block at all**: the `NULL` pointer that is
    /// the pessimistic case, and means it.
    Slim,
    /// Declares the window `[-1, 1]`, which is the gate a neighbour arrives
    /// through.
    Temporal,
    /// `describe` answers `false`. The effect is not catalogued and the bundle
    /// carries on.
    BrokenDescribe,
    /// Two controls on one id, which is one of them silently driving the other
    /// and refuses the whole effect.
    DuplicateIds,
    /// Hands its input back untouched.
    Passthrough,
    /// Says two of its processes may never run at once.
    ThreadUnsafe,
    /// Writes nothing at all and answers `LFX_STATUS_OK`: the identity an
    /// effect with nothing to do renders, byte for byte.
    Identity,
    /// Requires `lfx.gpu-frames`, which no version 1 host offers, so it is
    /// refused before it is instantiated.
    MissingExtension,
    /// Dies on the frame [`CRASH_ON_FRAME_ENV`] names, and is an ordinary
    /// passthrough otherwise.
    Crash,
    /// Never returns, when [`HANG_ENV`] is set.
    Hang,
    /// Says [`NOTE_SPAM_ENV`] things per process call, and one otherwise.
    NoteSpam,
}

/// Every personality, in the order the descriptor list reports them.
pub const PERSONALITIES: [Personality; 12] = [
    Personality::Full,
    Personality::Slim,
    Personality::Temporal,
    Personality::BrokenDescribe,
    Personality::DuplicateIds,
    Personality::Passthrough,
    Personality::ThreadUnsafe,
    Personality::Identity,
    Personality::MissingExtension,
    Personality::Crash,
    Personality::Hang,
    Personality::NoteSpam,
];

/// How many effects this bundle holds.
pub const PLUGIN_COUNT: u32 = PERSONALITIES.len() as u32;

/// Who this bundle says it is, on every descriptor.
pub const VENDOR: &CStr = c"The Lumit authors";

impl Personality {
    /// The reverse-DNS id, stable for the plugin's life: what
    /// `CreateInstance` names and what the host's match name is built from.
    #[must_use]
    pub const fn id(self) -> &'static CStr {
        match self {
            Personality::Full => c"org.lumit.testplug.full",
            Personality::Slim => c"org.lumit.testplug.slim",
            Personality::Temporal => c"org.lumit.testplug.temporal",
            Personality::BrokenDescribe => c"org.lumit.testplug.broken-describe",
            Personality::DuplicateIds => c"org.lumit.testplug.duplicate-ids",
            Personality::Passthrough => c"org.lumit.testplug.passthrough",
            Personality::ThreadUnsafe => c"org.lumit.testplug.thread-unsafe",
            Personality::Identity => c"org.lumit.testplug.identity",
            Personality::MissingExtension => c"org.lumit.testplug.missing-extension",
            Personality::Crash => c"org.lumit.testplug.crash",
            Personality::Hang => c"org.lumit.testplug.hang",
            Personality::NoteSpam => c"org.lumit.testplug.note-spam",
        }
    }

    /// What a person would read in the Add-effect menu.
    #[must_use]
    pub const fn name(self) -> &'static CStr {
        match self {
            Personality::Full => c"Test full",
            Personality::Slim => c"Test slim",
            Personality::Temporal => c"Test temporal",
            Personality::BrokenDescribe => c"Test broken describe",
            Personality::DuplicateIds => c"Test duplicate ids",
            Personality::Passthrough => c"Test passthrough",
            Personality::ThreadUnsafe => c"Test thread unsafe",
            Personality::Identity => c"Test identity",
            Personality::MissingExtension => c"Test missing extension",
            Personality::Crash => c"Test crash",
            Personality::Hang => c"Test hang",
            Personality::NoteSpam => c"Test note spam",
        }
    }

    /// Major, minor and patch. `Full` carries all three so the host's version
    /// arithmetic has something to mint from that is not nought.
    #[must_use]
    pub const fn version(self) -> (u32, u32, u32) {
        match self {
            Personality::Full => (2, 3, 4),
            _ => (1, 0, 0),
        }
    }

    /// The picture families claimed: the **first is the heading**, the rest are
    /// search keywords.
    #[must_use]
    pub const fn categories(self) -> &'static [LfxCategory] {
        match self {
            Personality::Full => &[LFX_CATEGORY_COLOUR, LFX_CATEGORY_STYLISE],
            Personality::Temporal => &[LFX_CATEGORY_TEMPORAL],
            Personality::Passthrough | Personality::Identity => &[LFX_CATEGORY_BLUR_SHARPEN],
            _ => &[LFX_CATEGORY_UTILITY],
        }
    }

    /// The extensions without which this effect cannot work.
    #[must_use]
    pub const fn required_extensions(self) -> &'static [&'static [u8]] {
        match self {
            // `lfx.gpu-frames` is a reserved id with no version 1 header, so
            // nothing offers it and this effect is refused before create.
            Personality::MissingExtension => &[LFX_EXT_GPU_FRAMES],
            _ => &[],
        }
    }

    /// The instance's trait block, or `None` for the `NULL` that is the
    /// pessimistic case.
    #[must_use]
    pub fn traits(self) -> Option<&'static LfxTraits> {
        match self {
            Personality::Full => Some(&FULL_TRAITS),
            Personality::Temporal => Some(&TEMPORAL_TRAITS),
            Personality::ThreadUnsafe => Some(&THREAD_UNSAFE_TRAITS),
            // Every other personality declares nothing, which is the `NULL`
            // pointer the header says means the pessimistic case.
            _ => None,
        }
    }
}

/// `Full`'s traits: everything stated, so the lowering has real numbers rather
/// than defaults to read.
static FULL_TRAITS: LfxTraits = LfxTraits {
    struct_size: size_of::<LfxTraits>() as u32,
    cost: LFX_COST_CHEAP,
    roi_kind: LFX_ROI_PADDED,
    roi_padding_px: 8.0,
    temporal_lo: 0,
    temporal_hi: 0,
    alpha: LFX_ALPHA_PREMULTIPLIED,
    flags: LFX_TRAIT_SEEDED | LFX_TRAIT_CANCELLABLE,
    scratch_bytes_per_megapixel: 4 * 1024 * 1024,
};

/// `Temporal`'s traits: the window that is the gate.
static TEMPORAL_TRAITS: LfxTraits = LfxTraits {
    struct_size: size_of::<LfxTraits>() as u32,
    cost: LFX_COST_MODERATE,
    roi_kind: LFX_ROI_EXACT,
    roi_padding_px: 0.0,
    temporal_lo: -1,
    temporal_hi: 1,
    alpha: LFX_ALPHA_PREMULTIPLIED,
    flags: 0,
    scratch_bytes_per_megapixel: 0,
};

/// `ThreadUnsafe`'s traits: the sole, discouraged opt-out.
static THREAD_UNSAFE_TRAITS: LfxTraits = LfxTraits {
    struct_size: size_of::<LfxTraits>() as u32,
    cost: LFX_COST_CHEAP,
    roi_kind: LFX_ROI_EXACT,
    roi_padding_px: 0.0,
    temporal_lo: 0,
    temporal_hi: 0,
    alpha: LFX_ALPHA_PREMULTIPLIED,
    flags: LFX_TRAIT_THREAD_UNSAFE,
    scratch_bytes_per_megapixel: 0,
};

/// A trait block from a header older than this one: two words, where
/// `lfx_traits` now carries nine.
///
/// It is **genuinely** that long, rather than a whole block whose prefix lies,
/// because the rule it exists to hold the host to is about the allocation: a
/// host that forms a `&lfx_traits` over this static before reading the prefix
/// reads sixty-odd bytes out of an eight-byte object, which is a fault a
/// sanitiser can see and a lying prefix is not.
#[repr(C)]
struct ShortTraits {
    struct_size: u32,
    cost: u32,
}

/// The one short block, which every descriptor points at while
/// [`LumitLfxProbeShortTraits`] is on.
static SHORT_TRAITS: ShortTraits = ShortTraits {
    struct_size: size_of::<ShortTraits>() as u32,
    cost: LFX_COST_CHEAP,
};

// -------------------------------------------------------- the descriptors --

/// The descriptor list, built once and never moved: the header says a
/// descriptor must stay valid and unchanged until `deinit`.
struct Table {
    /// The `const char *const *` each descriptor's `required_extensions`
    /// points at. Written once and never read again - it is here because the
    /// descriptors point into it, and the header says a descriptor stays valid
    /// until `deinit`.
    #[allow(dead_code)]
    required: Vec<Vec<*const c_char>>,
    descriptors: Vec<LfxDescriptor>,
}

// SAFETY: every pointer in the table is into this library's own read-only
// statics or into the heap the table itself owns and never frees, so sharing
// the table across threads shares immutable bytes that outlive every reader.
unsafe impl Sync for Table {}
// SAFETY: as above.
unsafe impl Send for Table {}

/// The honest table, built on the first ask.
fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| built_table(false))
}

/// The same list again, with every trait block the short one.
///
/// A table of its own rather than a field written into the honest one: the
/// header says a descriptor stays valid **and unchanged** until `deinit`, so
/// the probe chooses between two permanent lists rather than editing a live
/// one under a host that may already have read it.
fn short_trait_table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| built_table(true))
}

/// One descriptor list, with every trait block either each personality's own or
/// the short one.
fn built_table(short_traits: bool) -> Table {
    let required: Vec<Vec<*const c_char>> = PERSONALITIES
        .iter()
        .map(|personality| {
            personality
                .required_extensions()
                .iter()
                .map(|id| id.as_ptr().cast::<c_char>())
                .collect()
        })
        .collect();
    let descriptors = PERSONALITIES
        .iter()
        .enumerate()
        .map(|(index, personality)| {
            let (major, minor, patch) = personality.version();
            let list = required.get(index).map_or(&[][..], Vec::as_slice);
            LfxDescriptor {
                struct_size: size_of::<LfxDescriptor>() as u32,
                id: personality.id().as_ptr(),
                name: personality.name().as_ptr(),
                vendor: VENDOR.as_ptr(),
                major,
                minor,
                patch,
                categories: personality.categories().as_ptr(),
                category_count: personality.categories().len() as u32,
                traits: match (personality.traits(), short_traits) {
                    (None, _) => std::ptr::null(),
                    (Some(_), true) => std::ptr::from_ref(&SHORT_TRAITS).cast::<LfxTraits>(),
                    (Some(traits), false) => traits as *const LfxTraits,
                },
                required_extensions: list.as_ptr(),
                required_extension_count: list.len() as u32,
            }
        })
        .collect();
    Table {
        required,
        descriptors,
    }
}

// -------------------------------------------------------------- the entry --

/// The one exported object, under the name the header gives the loader.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static lfx_entry_point: LfxEntry = LfxEntry {
    struct_size: size_of::<LfxEntry>() as u32,
    abi_version: LFX_ABI_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    count: Some(entry_count),
    descriptor: Some(entry_descriptor),
    create: Some(entry_create),
};

/// Start the bundle, and remember the directory the host handed over.
///
/// # Safety
///
/// `bundle_path` must be null or a NUL-terminated string valid for the call.
unsafe extern "C" fn entry_init(bundle_path: *const c_char) -> u32 {
    note_call("entry.init");
    let path = if bundle_path.is_null() {
        String::new()
    } else {
        // SAFETY: the host's contract: a NUL-terminated string valid for the
        // call.
        unsafe { CStr::from_ptr(bundle_path) }
            .to_string_lossy()
            .into_owned()
    };
    let mut held = BUNDLE_PATH.lock().unwrap_or_else(PoisonError::into_inner);
    *held = path;
    // Non-zero is true. The answers a plugin gives cross as `u32` so the host
    // never has to trust a stranger's compiler to have normalised a C `bool`.
    1
}

/// Stop the bundle. Once, last.
unsafe extern "C" fn entry_deinit() {
    note_call("entry.deinit");
}

/// How many effects this bundle holds.
unsafe extern "C" fn entry_count() -> u32 {
    note_call("count");
    match DECLARED_COUNT.load(Ordering::SeqCst) {
        0 => PLUGIN_COUNT,
        lied => lied,
    }
}

/// The descriptor at `index`, or null past the end.
unsafe extern "C" fn entry_descriptor(index: u32) -> *const LfxDescriptor {
    note_call(&format!("descriptor:{index}"));
    let table = match SHORT_TRAIT_BLOCK.load(Ordering::SeqCst) {
        0 => table(),
        _ => short_trait_table(),
    };
    table
        .descriptors
        .get(index as usize)
        .map_or(std::ptr::null(), |descriptor| {
            descriptor as *const LfxDescriptor
        })
}

/// One instance of the effect named `id`, or null.
///
/// # Safety
///
/// `host` must be null or point at an `lfx_host` that outlives the instance,
/// and `id` must be a NUL-terminated string valid for the call.
unsafe extern "C" fn entry_create(host: *const LfxHost, id: *const c_char) -> *mut LfxPlugin {
    if id.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the host's contract: a NUL-terminated string valid for the call.
    let wanted = unsafe { CStr::from_ptr(id) };
    let Some(personality) = PERSONALITIES
        .iter()
        .copied()
        .find(|personality| personality.id() == wanted)
    else {
        note_call(&format!("create-refused:{}", wanted.to_string_lossy()));
        return std::ptr::null_mut();
    };
    note_call(&format!("create:{}", wanted.to_string_lossy()));

    let instance = Box::new(Instance {
        plugin: LfxPlugin {
            struct_size: match TABLE_BYTES.load(Ordering::SeqCst) {
                0 => size_of::<LfxPlugin>() as u32,
                lied => lied,
            },
            plugin_data: std::ptr::null_mut(),
            init: Some(plugin_init),
            destroy: Some(plugin_destroy),
            describe: Some(plugin_describe),
            process: Some(plugin_process),
            get_extension: Some(plugin_get_extension),
        },
        personality,
        host,
        in_flight: AtomicU32::new(0),
    });
    let raw = Box::into_raw(instance);
    // SAFETY: `raw` is a live, uniquely owned allocation this call just made.
    unsafe { (*raw).plugin.plugin_data = raw.cast::<c_void>() };
    raw.cast::<LfxPlugin>()
}

// ----------------------------------------------------------- the instance --

/// One live effect.
///
/// `#[repr(C)]` with the `lfx_plugin` first, so the pointer the host holds and
/// the allocation this crate owns are the same address - which is how
/// [`plugin_destroy`] gets its box back.
#[repr(C)]
struct Instance {
    plugin: LfxPlugin,
    personality: Personality,
    /// The host, kept because the header says this pointer may be: it is what
    /// a plugin logs and fetches extensions through from inside `process`.
    host: *const LfxHost,
    /// Process calls in flight on **this** instance.
    in_flight: AtomicU32,
}

/// The instance behind a plugin pointer, or `None` for a null one.
///
/// # Safety
///
/// `plugin` must be null or a pointer this crate's `create` handed out and
/// `destroy` has not taken back.
unsafe fn instance<'a>(plugin: *mut LfxPlugin) -> Option<&'a Instance> {
    if plugin.is_null() {
        return None;
    }
    // SAFETY: the caller's contract, and `Instance` is `#[repr(C)]` with the
    // plugin as its first field, so the two addresses are one.
    Some(unsafe { &*plugin.cast::<Instance>() })
}

/// Prepare this instance.
///
/// # Safety
///
/// As [`instance`].
unsafe extern "C" fn plugin_init(plugin: *mut LfxPlugin) -> u32 {
    // SAFETY: the caller's contract, passed straight on.
    let Some(instance) = (unsafe { instance(plugin) }) else {
        return 0;
    };
    note_call(&format!(
        "init:{}",
        instance.personality.id().to_string_lossy()
    ));
    if instance.personality == Personality::Temporal {
        // The gate is the declared window; this is the plugin asking whether
        // the host also offers the table that carries a neighbour. A missing
        // extension is a null, never a status.
        // SAFETY: the host pointer is the one `create` was handed, which the
        // header says is valid until `deinit`.
        let offered = unsafe { ask_host(instance.host, LFX_EXT_TEMPORAL, 1) };
        note_call(if offered.is_null() {
            "host-extension:lfx.temporal=none"
        } else {
            "host-extension:lfx.temporal=offered"
        });
    }
    // Non-zero is true, as `entry_init` says.
    1
}

/// Take this instance down.
///
/// # Safety
///
/// As [`instance`], and never while a process call is running.
unsafe extern "C" fn plugin_destroy(plugin: *mut LfxPlugin) {
    if plugin.is_null() {
        return;
    }
    // SAFETY: the caller's contract: a pointer `create` handed out and nobody
    // has taken back, so this reclaims the one box that owns it.
    let owned = unsafe { Box::from_raw(plugin.cast::<Instance>()) };
    note_call(&format!(
        "destroy:{}",
        owned.personality.id().to_string_lossy()
    ));
}

/// This plugin's own extension table for `id`, or null. It offers none.
///
/// # Safety
///
/// As [`instance`], and `id` must be a NUL-terminated string valid for the
/// call.
unsafe extern "C" fn plugin_get_extension(
    _plugin: *mut LfxPlugin,
    id: *const c_char,
    version: u32,
) -> *const c_void {
    if !id.is_null() {
        // SAFETY: the host's contract: a NUL-terminated string valid for the
        // call.
        let name = unsafe { CStr::from_ptr(id) };
        note_call(&format!(
            "plugin-extension:{}@{version}",
            name.to_string_lossy()
        ));
    }
    std::ptr::null()
}

/// Ask the host for an extension table.
///
/// # Safety
///
/// `host` must be null or point at an `lfx_host` valid for the call.
unsafe fn ask_host(host: *const LfxHost, id: &'static [u8], version: u32) -> *const c_void {
    if host.is_null() {
        return std::ptr::null();
    }
    // SAFETY: the caller's contract.
    let table = unsafe { &*host };
    let Some(get) = table.get_extension else {
        return std::ptr::null();
    };
    // SAFETY: the host's own function, with the host it belongs to and a
    // NUL-terminated id from this crate's own statics.
    unsafe { get(host, id.as_ptr().cast::<c_char>(), version) }
}

/// Say one thing through the host's log.
///
/// # Safety
///
/// `host` must be null or point at an `lfx_host` valid for the call.
unsafe fn say(host: *const LfxHost, level: u32, message: &CStr) {
    if host.is_null() {
        return;
    }
    // SAFETY: the caller's contract.
    let table = unsafe { &*host };
    let Some(log) = table.log else {
        return;
    };
    NOTES_SENT.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the host's own function, with the host it belongs to and a
    // NUL-terminated message that lives for the call.
    unsafe { log(host, level, message.as_ptr()) };
}

// ----------------------------------------------------------- the describe --

/// The four words every declaration opens with, filled in the same way each
/// time so the bodies below read as the one field they differ in.
macro_rules! head {
    ($struct:ty, $unit:expr) => {
        (size_of::<$struct>() as u32, $unit, LFX_PARAM_FLAG_NONE)
    };
}

/// Declare every control, in the order they should be drawn.
///
/// # Safety
///
/// As [`instance`], and `sink` must point at an `lfx_describe_sink` valid for
/// the call.
unsafe extern "C" fn plugin_describe(plugin: *mut LfxPlugin, sink: *mut LfxDescribeSink) -> u32 {
    // SAFETY: the caller's contract, passed straight on.
    let Some(instance) = (unsafe { instance(plugin) }) else {
        return 0;
    };
    note_call(&format!(
        "describe:{}",
        instance.personality.id().to_string_lossy()
    ));
    if sink.is_null() {
        return 0;
    }
    // Non-zero is true, as `entry_init` says.
    u32::from(match instance.personality {
        // SAFETY: the caller's contract on `sink`, carried into each of these.
        Personality::Full => unsafe { describe_full(sink) },
        Personality::DuplicateIds => unsafe { describe_duplicate_ids(sink) },
        Personality::BrokenDescribe => false,
        // SAFETY: as above.
        _ => unsafe { declare_amount(sink) },
    })
}

/// One plain control, which is what every personality that is about something
/// else declares.
///
/// # Safety
///
/// `sink` must point at an `lfx_describe_sink` valid for the call.
unsafe fn declare_amount(sink: *mut LfxDescribeSink) -> bool {
    let (struct_size, unit, flags) = head!(LfxFloatParam, LFX_UNIT_RAW);
    let amount = LfxFloatParam {
        struct_size,
        unit,
        flags,
        bounds: LFX_BOUND_MIN,
        id: c"amount".as_ptr(),
        label: c"Amount".as_ptr(),
        default_value: 1.0,
        slider_min: 0.0,
        slider_max: 1.0,
        hard_min: 0.0,
        hard_max: 0.0,
    };
    // SAFETY: the caller's contract, and the declaration lives for the call.
    unsafe { push_float(sink, &amount) };
    true
}

/// Two controls on one id: one of them silently driving the other, which is
/// what the sink refuses the whole effect for.
///
/// # Safety
///
/// As [`declare_amount`].
unsafe fn describe_duplicate_ids(sink: *mut LfxDescribeSink) -> bool {
    let (struct_size, unit, flags) = head!(LfxFloatParam, LFX_UNIT_RAW);
    let first = LfxFloatParam {
        struct_size,
        unit,
        flags,
        bounds: 0,
        id: c"gain".as_ptr(),
        label: c"Gain".as_ptr(),
        default_value: 1.0,
        slider_min: 0.0,
        slider_max: 2.0,
        hard_min: 0.0,
        hard_max: 0.0,
    };
    let second = LfxFloatParam {
        label: c"Gain again".as_ptr(),
        ..first
    };
    // SAFETY: the caller's contract, and both declarations live for the call.
    unsafe {
        push_float(sink, &first);
        push_float(sink, &second);
    }
    true
}

/// One control of every kind version 1 admits, under one heading, plus one the
/// host cannot draw - a dropdown with nothing in it, which is a line in the
/// scan report and costs the effect nothing else.
///
/// # Safety
///
/// As [`declare_amount`].
#[allow(clippy::too_many_lines)]
unsafe fn describe_full(sink: *mut LfxDescribeSink) -> bool {
    let group = LfxGroupParam {
        struct_size: size_of::<LfxGroupParam>() as u32,
        flags: LFX_PARAM_FLAG_NONE,
        id: c"basics".as_ptr(),
        label: c"Basics".as_ptr(),
    };
    let (float_size, _, flags) = head!(LfxFloatParam, LFX_UNIT_RAW);
    let gain = LfxFloatParam {
        struct_size: float_size,
        unit: LFX_UNIT_RAW,
        flags,
        bounds: LFX_BOUND_MIN,
        id: c"gain".as_ptr(),
        label: c"Gain".as_ptr(),
        default_value: 1.0,
        slider_min: 0.0,
        slider_max: 2.0,
        hard_min: 0.0,
        hard_max: 0.0,
    };
    let mix = LfxSliderParam {
        struct_size: size_of::<LfxSliderParam>() as u32,
        unit: LFX_UNIT_PERCENT,
        flags,
        log: 0,
        id: c"mix".as_ptr(),
        label: c"Mix".as_ptr(),
        default_value: 100.0,
        range_min: 0.0,
        range_max: 100.0,
    };
    let steps = LfxIntParam {
        struct_size: size_of::<LfxIntParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        bounds: LFX_BOUND_MIN | LFX_BOUND_MAX,
        id: c"steps".as_ptr(),
        label: c"Steps".as_ptr(),
        default_value: 4,
        slider_min: 1,
        slider_max: 16,
        hard_min: 1,
        hard_max: 64,
    };
    let tilt = LfxAngleParam {
        struct_size: size_of::<LfxAngleParam>() as u32,
        unit: LFX_UNIT_DEGREES,
        flags,
        reserved_0: 0,
        id: c"tilt".as_ptr(),
        label: c"Tilt".as_ptr(),
        default_value: 0.0,
        dial_step: 15.0,
    };
    let invert = LfxBoolParam {
        struct_size: size_of::<LfxBoolParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        default_value: 0,
        id: c"invert".as_ptr(),
        label: c"Invert".as_ptr(),
    };
    let options: [*const c_char; 3] = [c"Linear".as_ptr(), c"Smooth".as_ptr(), c"Hard".as_ptr()];
    let dividers: [u32; 1] = [1];
    let mode = LfxChoiceParam {
        struct_size: size_of::<LfxChoiceParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        default_index: 1,
        option_count: options.len() as u32,
        divider_count: dividers.len() as u32,
        id: c"mode".as_ptr(),
        label: c"Mode".as_ptr(),
        options: options.as_ptr(),
        dividers_after: dividers.as_ptr(),
    };
    let tint = LfxColourParam {
        struct_size: size_of::<LfxColourParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        reserved_0: 0,
        id: c"tint".as_ptr(),
        label: c"Tint".as_ptr(),
        default_rgba: [1.0, 1.0, 1.0, 1.0],
        range_min: -1.0,
        range_max: 4.0,
    };
    let seed = LfxSeedParam {
        struct_size: size_of::<LfxSeedParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        reserved_0: 0,
        id: c"seed".as_ptr(),
        label: c"Seed".as_ptr(),
    };
    let centre = LfxPoint2Param {
        struct_size: size_of::<LfxPoint2Param>() as u32,
        unit: LFX_UNIT_PX,
        flags,
        reserved_0: 0,
        id: c"centre".as_ptr(),
        label: c"Centre".as_ptr(),
        default_x: 0.0,
        default_y: 0.0,
        slider_min: -1000.0,
        slider_max: 1000.0,
    };
    let pivot = LfxPoint3Param {
        struct_size: size_of::<LfxPoint3Param>() as u32,
        unit: LFX_UNIT_PX,
        flags,
        reserved_0: 0,
        id: c"pivot".as_ptr(),
        label: c"Pivot".as_ptr(),
        default_x: 0.0,
        default_y: 0.0,
        default_z: 0.0,
        slider_min: -1000.0,
        slider_max: 1000.0,
    };
    let points: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    let shape = LfxCurveParam {
        struct_size: size_of::<LfxCurveParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        point_count: 2,
        id: c"shape".as_ptr(),
        label: c"Shape".as_ptr(),
        points: points.as_ptr(),
    };
    let filters: [*const c_char; 1] = [c"cube".as_ptr()];
    let table = LfxFileParam {
        struct_size: size_of::<LfxFileParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        filter_count: filters.len() as u32,
        id: c"table".as_ptr(),
        label: c"Table".as_ptr(),
        filter: filters.as_ptr(),
        filter_name: c"Lookup tables".as_ptr(),
    };
    // A dropdown with nothing in it: a row this build cannot draw, which is a
    // line in the scan report and leaves the effect catalogued.
    let empty = LfxChoiceParam {
        default_index: 0,
        option_count: 0,
        divider_count: 0,
        id: c"empty".as_ptr(),
        label: c"Empty".as_ptr(),
        options: std::ptr::null(),
        dividers_after: std::ptr::null(),
        ..mode
    };
    let reset = LfxActionParam {
        struct_size: size_of::<LfxActionParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags,
        reserved_0: 0,
        id: c"reset".as_ptr(),
        label: c"Reset".as_ptr(),
    };

    // SAFETY: the caller's contract on `sink`, and every declaration above
    // lives until this function returns.
    unsafe {
        push(sink, (*sink).group_begin, &group);
        push_float(sink, &gain);
        push(sink, (*sink).declare_slider, &mix);
        push(sink, (*sink).declare_int, &steps);
        push(sink, (*sink).declare_angle, &tilt);
        push(sink, (*sink).declare_bool, &invert);
        push(sink, (*sink).declare_choice, &mode);
        push(sink, (*sink).declare_colour, &tint);
        push(sink, (*sink).declare_seed, &seed);
        push(sink, (*sink).declare_point2, &centre);
        push(sink, (*sink).declare_point3, &pivot);
        push(sink, (*sink).declare_curve, &shape);
        push(sink, (*sink).declare_file, &table);
        push(sink, (*sink).declare_choice, &empty);
        push(sink, (*sink).declare_action, &reset);
        if let Some(end) = (*sink).group_end {
            end(sink);
        }
    }
    true
}

/// Push one declaration through the sink entry point that takes it.
///
/// # Safety
///
/// `sink` must point at an `lfx_describe_sink` valid for the call and
/// `declaration` at a live declaration of the type `entry` takes.
unsafe fn push<T>(
    sink: *mut LfxDescribeSink,
    entry: Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const T) -> bool>,
    declaration: &T,
) -> bool {
    match entry {
        // SAFETY: the caller's contract.
        Some(entry) => unsafe { entry(sink, declaration) },
        None => false,
    }
}

/// [`push`] for the one kind three personalities share.
///
/// # Safety
///
/// As [`push`].
unsafe fn push_float(sink: *mut LfxDescribeSink, declaration: &LfxFloatParam) -> bool {
    // SAFETY: the caller's contract.
    unsafe { push(sink, (*sink).declare_float, declaration) }
}

// ------------------------------------------------------------ the process --

/// Render one frame.
///
/// # Safety
///
/// As [`instance`], and `request` must point at an `lfx_process` whose values
/// array and both frames are valid for the call.
unsafe extern "C" fn plugin_process(plugin: *mut LfxPlugin, request: *const LfxProcess) -> i32 {
    // SAFETY: the caller's contract, passed straight on.
    let Some(instance) = (unsafe { instance(plugin) }) else {
        return LFX_STATUS_FAILED;
    };
    if request.is_null() {
        return LFX_STATUS_FAILED;
    }
    // SAFETY: the caller's contract: a request valid for the call.
    let request = unsafe { &*request };

    note_call(&format!(
        "process:{}@{}",
        instance.personality.id().to_string_lossy(),
        request.time
    ));
    record(
        &DEPTH_LOG,
        match request.pixel_format {
            LFX_RGBA_F16 => "f16",
            LFX_RGBA_F32 => "f32",
            _ => "unset",
        },
    );
    let origin = if request.input.is_null() {
        (0, 0)
    } else {
        // SAFETY: the caller's contract: a non-null input frame is valid for
        // the call.
        let input = unsafe { &*request.input };
        (input.origin_x, input.origin_y)
    };
    record(
        &REGION_LOG,
        &format!(
            "{}:{}:{}:{}:{}:{}",
            origin.0, origin.1, request.roi_x0, request.roi_y0, request.roi_x1, request.roi_y1
        ),
    );

    let across = IN_FLIGHT.fetch_add(1, Ordering::SeqCst).saturating_add(1);
    MAX_IN_FLIGHT.fetch_max(across, Ordering::SeqCst);
    let mine = instance
        .in_flight
        .fetch_add(1, Ordering::SeqCst)
        .saturating_add(1);
    MAX_PER_INSTANCE.fetch_max(mine, Ordering::SeqCst);
    wait_for_company();

    // SAFETY: the caller's contract on the request, carried into the body.
    let status = unsafe { run(instance, request) };

    instance.in_flight.fetch_sub(1, Ordering::SeqCst);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    status
}

/// Hold here until as many process calls are in flight as
/// [`LumitLfxProbeRendezvous`] asked for, or until the wait has gone on long
/// enough that the host plainly serialises them.
fn wait_for_company() {
    let target = RENDEZVOUS.load(Ordering::SeqCst);
    if target == 0 {
        return;
    }
    let deadline = Instant::now() + RENDEZVOUS_TIMEOUT;
    while IN_FLIGHT.load(Ordering::SeqCst) < target && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// What each personality actually does with the frame.
///
/// # Safety
///
/// As [`plugin_process`].
unsafe fn run(instance: &Instance, request: &LfxProcess) -> i32 {
    match instance.personality {
        // Nothing written at all, which is identity byte for byte: not the
        // input copied back, which would put the picture through the depth
        // boundary and change it very slightly.
        Personality::Identity => LFX_STATUS_OK,
        Personality::Full => {
            // SAFETY: the caller's contract on the request.
            if unsafe { asked_to_stop(request) } {
                return LFX_STATUS_CANCELLED;
            }
            // Walk the whole array first, by the host's own stride, and
            // refuse if any element is not where its own `param` says it is.
            // That is the stride agreement made observable: a plugin striding
            // by its own `size_of` after the struct grew would read
            // correct-looking kind tags over wrong values, and this is the
            // check that would catch it (docs/impl/lfx.md §11 item 15).
            for index in 0..request.value_count {
                // SAFETY: the caller's contract on the request.
                if unsafe { value_at(request, index) }.is_none() {
                    return LFX_STATUS_FAILED;
                }
            }
            // SAFETY: as above; the first declaration that carries a value is
            // the Float this effect multiplies by.
            let gain = unsafe { value_at(request, 0) }
                .filter(|value| value.kind == LFX_PARAM_FLOAT)
                // SAFETY: the tag says which arm of the union to read, which
                // is what makes a kind mismatch impossible here.
                .map_or(1.0, |value| unsafe { value.v.f });
            // SAFETY: the caller's contract on both frames.
            unsafe { shade(request, gain) }
        }
        Personality::Crash => {
            if let Some(frame) = number_from(CRASH_ON_FRAME_ENV) {
                if (request.time - frame).abs() < 0.5 {
                    // What a plugin with a bad pointer does: no warning, no
                    // unwinding, and the host's answer to it is a new broker.
                    std::process::abort();
                }
            }
            // SAFETY: the caller's contract on both frames.
            unsafe { shade(request, 1.0) }
        }
        Personality::Hang => {
            if std::env::var_os(HANG_ENV).is_some() {
                loop {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            // SAFETY: the caller's contract on both frames.
            unsafe { shade(request, 1.0) }
        }
        Personality::NoteSpam => {
            let lines = number_from(NOTE_SPAM_ENV).unwrap_or(1.0).max(1.0) as u32;
            for _ in 0..lines {
                // SAFETY: the host pointer is the one `create` was handed,
                // which the header says is valid until `deinit`.
                unsafe {
                    say(
                        instance.host,
                        LFX_LOG_WARN,
                        c"the plugin is saying too much",
                    )
                };
            }
            // SAFETY: the caller's contract on both frames.
            unsafe { shade(request, 1.0) }
        }
        _ => {
            // SAFETY: the caller's contract on both frames.
            unsafe { shade(request, 1.0) }
        }
    }
}

/// Whether the host has stopped wanting this frame.
///
/// # Safety
///
/// As [`plugin_process`].
unsafe fn asked_to_stop(request: &LfxProcess) -> bool {
    match request.cancelled {
        // SAFETY: the host's own function, handed the request it belongs to,
        // called while the host is still waiting for this frame.
        Some(ask) => unsafe { ask(request as *const LfxProcess) },
        None => false,
    }
}

/// The number an environment variable holds, or `None` for one that is unset or
/// is not a number. This is what disarms the three dangerous personalities.
fn number_from(variable: &str) -> Option<f64> {
    std::env::var(variable).ok()?.trim().parse::<f64>().ok()
}

/// One element of the dense value array, **read by the stride the host wrote**
/// and never by this crate's own `size_of` - which is the whole reason
/// `lfx_value` is the one struct with no size prefix.
///
/// # Safety
///
/// As [`plugin_process`].
unsafe fn value_at(request: &LfxProcess, index: u32) -> Option<LfxValue> {
    if index >= request.value_count || request.values.is_null() || request.value_stride == 0 {
        return None;
    }
    let offset = (index as usize).checked_mul(request.value_stride as usize)?;
    // SAFETY: `index` is below the count the host declared and the stride is
    // the host's own, so this element is inside the array the host wrote.
    let element = unsafe { request.values.cast::<u8>().add(offset) };
    // The header says the array's base and its stride are both aligned for
    // `lfx_value`, so an element that is not is the host breaking its own
    // contract rather than something to read around: it is refused, and the
    // personality turns that into `LFX_STATUS_FAILED`. A C plugin reading
    // `((const lfx_value *)element)->v.f` would fault here on a strict
    // alignment target, and this is the fixture standing where it would.
    if element.align_offset(align_of::<LfxValue>()) != 0 {
        return None;
    }
    // SAFETY: as above. Read unaligned even so: a fixture whose answer to a
    // host's regression is undefined behaviour is a worse witness than one
    // that fails the frame.
    let value = unsafe { element.cast::<LfxValue>().read_unaligned() };
    (value.param == index).then_some(value)
}

/// Multiply the input into the output, at whichever depth the host handed over.
///
/// # Safety
///
/// As [`plugin_process`].
unsafe fn shade(request: &LfxProcess, gain: f64) -> i32 {
    if request.input.is_null() || request.output.is_null() {
        return LFX_STATUS_FAILED;
    }
    // SAFETY: the caller's contract: both frames are valid for the call.
    let (input, output) = unsafe { (&*request.input, &mut *request.output) };
    if input.data.is_null() || output.data.is_null() {
        return LFX_STATUS_FAILED;
    }
    if input.format != output.format || input.format != request.pixel_format {
        return LFX_STATUS_FAILED;
    }
    let rows = input.height.min(output.height) as usize;
    let across = (input.width.min(output.width) as usize).saturating_mul(4);
    for row in 0..rows {
        let from = row.saturating_mul(input.row_bytes as usize);
        let to = row.saturating_mul(output.row_bytes as usize);
        // SAFETY: `row` is below both heights and the row offsets are the
        // frames' own declared row strides, so both runs are inside the
        // buffers the host handed over.
        unsafe {
            let source = input.data.cast::<u8>().add(from);
            let destination = output.data.cast::<u8>().add(to);
            match request.pixel_format {
                LFX_RGBA_F32 => scale_f32(
                    source.cast::<f32>(),
                    destination.cast::<f32>(),
                    across,
                    gain as f32,
                ),
                LFX_RGBA_F16 => scale_f16(
                    source.cast::<half::f16>(),
                    destination.cast::<half::f16>(),
                    across,
                    gain as f32,
                ),
                _ => return LFX_STATUS_FAILED,
            }
        }
    }
    LFX_STATUS_OK
}

/// One row of fp32.
///
/// # Safety
///
/// `source` and `destination` must each be valid for `count` floats.
unsafe fn scale_f32(source: *const f32, destination: *mut f32, count: usize, gain: f32) {
    for index in 0..count {
        // SAFETY: the caller's contract; `index` is below `count`.
        unsafe { *destination.add(index) = *source.add(index) * gain };
    }
}

/// One row of fp16, in the halves themselves: the depth crosses the boundary
/// unconverted, which is the whole of what docs/12 §3.3 promises.
///
/// # Safety
///
/// `source` and `destination` must each be valid for `count` halves.
unsafe fn scale_f16(
    source: *const half::f16,
    destination: *mut half::f16,
    count: usize,
    gain: f32,
) {
    for index in 0..count {
        // SAFETY: the caller's contract; `index` is below `count`.
        unsafe {
            let value = (*source.add(index)).to_f32() * gain;
            *destination.add(index) = half::f16::from_f32(value);
        }
    }
}
