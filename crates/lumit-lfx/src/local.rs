//! The in-process host: a bundle opened here, driven here
//! (docs/impl/lfx.md §4.2, §10).
//!
//! # In plain terms
//!
//! This is the whole of LFX with the second process taken out. It opens a
//! module, asks the entry point what effects it holds, fills in the frozen
//! describe sink so a plugin's typed declarations become [`Declaration`]s,
//! creates an instance, and hands it one frame at a time at whichever depth it
//! is given. Everything above it - the lowering onto the same
//! [`EffectSchema`](lumit_core::fx::EffectSchema) a built-in carries, the
//! refusals, the report lines - is the same code the shipping path runs, which
//! is the point: what the broker adds is a process boundary and a watchdog, not
//! a second way of reading a plugin.
//!
//! **Nothing in the editor process reaches this module.** docs/12:354-356
//! forbids an in-process path in version 1 - "one fewer code path, and the
//! crash-isolation promise stays unconditional" - so what the editor holds is a
//! `BrokerHost` and a pipe. That is not the same as this module being test-only:
//! the broker binary is what sits at the other end of that pipe, and it opens a
//! third party's module through [`LocalHost::open`] and renders through
//! [`LocalInstance::process`], so every raw-pointer read below runs against
//! untrusted bytes in production. **The isolation is the process boundary rather
//! than this module's audience.** A crash here takes the broker, which is a
//! report line and a badged layer rather than the editor's session; in this
//! crate's own suite it takes the test binary, which is exactly the blast radius
//! a test wants. What the in-process spelling buys that suite is that the ABI
//! edge, which is the part no amount of care makes obviously correct, can be
//! exercised by a plain `cargo test` with no second process, no pipe and no
//! ring.
//!
//! # Where the raw pointers stop
//!
//! Here. [`crate::describe`] and [`crate::schema`] see none of them: every
//! `declare_*` entry point below turns one `*const lfx_*_param` into a
//! [`Declaration`] and calls [`Describe::declare`], and the three conversions
//! that are a decision rather than a copy are that module's, not this one's, so
//! that this edge and the broker's cannot read one frozen struct two different
//! ways.
//!
//! Three rules govern everything a stranger's bytes are read under, and all
//! three are the header's own. A **count is checked against the header's
//! ceiling before its array is read**, never after, and a count past it means
//! the array is not read *at all* rather than read to the ceiling: a dropdown
//! declaring four billion options is a report line filed from the count alone
//! ([`Describe::decline`]), because reading the array first is not a mistake a
//! later check can undo. A **string is read to the first NUL inside
//! `LFX_MAX_STRING_BYTES` or not at all** - the host never walks past the
//! limit looking for the end. And a **size prefix is read before the struct it
//! prefixes**, as a bare `uint32_t` rather than through a reference to the
//! whole: forming `&T` over an allocation shorter than `T` is already reading
//! somebody else's memory, before a field is touched. Every size-prefixed
//! struct the ABI carries is read that way - the entry, the descriptor, the
//! instance table, the trait block and every declaration.
//!
//! # Thread role
//!
//! [`LocalHost::open`], [`LocalHost::describe`], [`LocalHost::create`] and
//! dropping a [`LocalInstance`] are the control thread's, as the header pins
//! them, and each of the four takes a lock of the host's own so that two
//! callers sharing a `&LocalHost` serialise rather than racing inside somebody
//! else's `create`. [`LocalInstance::process`] is any worker thread's, and takes `&mut
//! self`, so one instance cannot be re-entered by construction while two
//! instances may be inside `process` at once.

// The one place in this crate where the workspace's `unsafe_code = "deny"` is
// given up, and it is given up **here** rather than at the crate root so that
// the deny still holds over the lowering, the protocol and the ring - none of
// which touch a raw pointer at all. The sibling hosts say `unsafe_code =
// "allow"` for a whole crate because FFI is most of what they are; this crate
// gives it up one module wide. Driving a frozen C entry point is unsafe by
// construction, and docs/14 §7 permits it in a plugin host and asks for the
// `// SAFETY:` comment and the layout tests that go with it - every block below
// carries the first, and `lumit-lfx-abi` is the second.
#![allow(unsafe_code)]

use std::ffi::{c_char, c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use half::f16;
use lumit_lfx_abi::{
    LfxActionParam, LfxAngleParam, LfxBoolParam, LfxChoiceParam, LfxColourParam, LfxCurveParam,
    LfxCurveValue, LfxDescribeSink, LfxDescriptor, LfxEntry, LfxFileParam, LfxFileValue,
    LfxFloatParam, LfxGroupParam, LfxHost, LfxIntParam, LfxLogLevel, LfxParamKind, LfxPixelFormat,
    LfxPlugin, LfxPoint2Param, LfxPoint3Param, LfxProcess, LfxSeedParam, LfxSliderParam, LfxStatus,
    LfxValue, LfxValuePayload, LFX_ABI_VERSION, LFX_ENTRY_SYMBOL, LFX_MAX_CATEGORIES,
    LFX_MAX_CURVE_POINTS, LFX_MAX_DIVIDERS, LFX_MAX_EFFECTS_PER_BUNDLE, LFX_MAX_FILTERS,
    LFX_MAX_LOG_BYTES, LFX_MAX_OPTIONS, LFX_MAX_REQUIRED_EXTENSIONS, LFX_MAX_STRING_BYTES,
    LFX_PARAM_ACTION, LFX_PARAM_ANGLE, LFX_PARAM_BOOL, LFX_PARAM_CHOICE, LFX_PARAM_COLOUR,
    LFX_PARAM_CURVE, LFX_PARAM_FILE, LFX_PARAM_FLOAT, LFX_PARAM_INT, LFX_PARAM_POINT2,
    LFX_PARAM_POINT3, LFX_PARAM_SEED, LFX_PARAM_SLIDER, LFX_RGBA_F16, LFX_RGBA_F32, LFX_STATUS_OK,
};
use thiserror::Error;

use crate::describe::{Declaration, Declared, Describe, Identity, PluginDescriptor, Traits};
use crate::rejection::Ceiling;
use crate::{schema, LfxRejection};

/// How many lines one bundle may say before the host stops listening, carried
/// over from the older host's `MAX_NOTES` (docs/impl/lfx.md §3.5).
///
/// A plugin in a loop is a plugin that would otherwise fill the process it is
/// talking to, and the shipping path's answer is the same number read at the
/// other end of a pipe.
pub const MAX_NOTES: usize = 64;

/// The widest dense value array this host will write, in bytes per element.
///
/// A stride is the one number in a [`Request`] the header puts no ceiling on,
/// and it has no ceiling there for a good reason: it is the *host's* own
/// number, written so a plugin built against a narrower `lfx_value` still
/// walks the array correctly (docs/impl/lfx.md §2.1). The header's ceilings
/// bound what a stranger declares; this one bounds what this host's own caller
/// may ask for, because `stride × count` bytes are allocated from it and a
/// `u32` asks for fifty gigabytes as readily as for forty-eight.
///
/// Four elements' worth is room for three growths of a frozen struct, which is
/// more than the ABI intends to have.
pub const MAX_VALUE_STRIDE: u32 = 4 * size_of::<LfxValue>() as u32;

/// The extensions this host offers, re-exported from [`crate::extensions`]
/// where the negotiation itself lives.
///
/// The name stays here because this is where a reader of the in-process host
/// looks for it, and the list stays there because discovery answers the same
/// question from the listing, before `create` and before any of that plugin's
/// own code - two readers of one list, so that a host cannot catalogue an
/// effect it then cannot make (docs/impl/lfx.md §4.3).
pub use crate::extensions::OFFERED_EXTENSIONS;

// ------------------------------------------------------------- the errors --

/// Why a bundle could not be opened, or an instance driven.
///
/// Typed, never a string: docs/14 §3 asks for it, and the Addons page turns
/// each of these into a sentence of its own.
#[derive(Debug, Error)]
pub enum LocalError {
    /// The operating system would not load the module at all.
    #[error("the module did not load ({0})")]
    NotLoaded(String),
    /// The module loaded and exports no `lfx_entry_point`, so it is not an LFX
    /// bundle however it is named.
    #[error("the module exports no {}", String::from_utf8_lossy(LFX_ENTRY_SYMBOL).trim_end_matches('\0'))]
    NoEntry,
    /// The module exports an `lfx_entry_point` whose `init` hook is null, so
    /// there is nothing to start the bundle with.
    ///
    /// Its own name rather than [`LocalError::NoEntry`]'s: the symbol is right
    /// there, and a sentence saying the bundle exports no entry point would
    /// send a vendor looking for the one thing they did do.
    #[error("the bundle's entry declares no init")]
    NoInit,
    /// The entry's size prefix is smaller than the one this header declares, so
    /// the fields the host would read are not there to read.
    #[error("the entry carries {bytes} bytes, and this host reads {}", size_of::<LfxEntry>())]
    ShortEntry {
        /// The size prefix it carried.
        bytes: u32,
    },
    /// An instance's own table is shorter than the one this header declares,
    /// so the function pointers the host would call are not there to read.
    ///
    /// [`LocalError::ShortEntry`] one struct further in, and the same rule:
    /// the growth mechanism reads upwards and cannot read downwards. The
    /// instance is taken down first where the prefix reaches its own `destroy`
    /// and leaked where it does not, there being nothing else that may be
    /// called on bytes the plugin itself says are not there.
    #[error("the instance {id:?} carries {bytes} bytes, and this host reads {}", size_of::<LfxPlugin>())]
    ShortPlugin {
        /// The effect whose table it was.
        id: String,
        /// The size prefix it carried.
        bytes: u32,
    },
    /// The module declares an ABI version this host does not speak.
    ///
    /// One integer, and it intends to reach 2 never, so version 1 admits
    /// exactly one number.
    #[error("the module declares LFX ABI {declared}, and this host speaks {LFX_ABI_VERSION}")]
    AbiUnsupported {
        /// What it declared.
        declared: u32,
    },
    /// `lfx_entry.init` answered false: the bundle declines to load, and per
    /// the header nothing else in it may be called.
    ///
    /// It means that and only that. The host's own failure to spell the bundle
    /// directory is [`LocalError::UnspellablePath`], because a report line
    /// blaming a plugin for this host's string handling is a line a vendor
    /// cannot act on.
    #[error("the bundle declined to load")]
    InitRefused,
    /// The bundle's own directory cannot cross as a C string, something in the
    /// path carrying an interior NUL.
    ///
    /// The host's fault rather than the plugin's, and named apart from
    /// [`LocalError::InitRefused`] so that that one means "`init` answered
    /// nought" and nothing else. *ponytail:* no test reaches it, and none can
    /// through [`LocalHost::open`] - a module path with a NUL in it is refused
    /// by the loader first, as [`LocalError::NotLoaded`] - so what this variant
    /// buys is the name rather than a behaviour.
    #[error("the bundle's own path {path:?} cannot cross as a C string")]
    UnspellablePath {
        /// The directory that could not be spelled.
        path: PathBuf,
    },
    /// The bundle holds no effect of that name.
    #[error("the bundle holds no effect called {id:?}")]
    NoSuchPlugin {
        /// The id that was asked for.
        id: String,
    },
    /// The effect cannot work without an extension this host does not offer, so
    /// it is refused before it is instantiated rather than left to fail
    /// somewhere later (docs/impl/lfx.md §4.3).
    #[error(
        "the effect {id:?} requires the extension {extension:?}, which this host does not offer"
    )]
    RequiresExtension {
        /// The effect that asked.
        id: String,
        /// What it asked for.
        extension: String,
    },
    /// `lfx_entry.create` answered null.
    #[error("the effect {id:?} refused to be created")]
    CreateRefused {
        /// The effect that refused.
        id: String,
    },
    /// The instance's own `init` answered false. The host badges the layer
    /// rather than failing the frame.
    #[error("the effect {id:?} refused to initialise")]
    InstanceRefused {
        /// The effect that refused.
        id: String,
    },
    /// `lfx_plugin.describe` answered false and the sink met no fault of its
    /// own, so the plugin is simply declining to say what it is.
    #[error("the effect {id:?} refused to describe itself")]
    DescribeRefused {
        /// The effect that refused.
        id: String,
    },
    /// The sink or the lowering met a fault that ends the effect.
    #[error(transparent)]
    Refused(#[from] LfxRejection),
    /// The caller offered a different number of values from the number of
    /// declarations that carry one.
    #[error("the request carries {given} values and the effect declared {declared}")]
    ValueCount {
        /// How many were offered.
        given: usize,
        /// How many the effect declared.
        declared: usize,
    },
    /// A value whose arm is not the one its declaration's kind names. The dense
    /// array is tagged by the declaration, so this is refused here rather than
    /// handed over as a kind the plugin would read through the wrong arm.
    #[error("value {element} is of kind {given} and its declaration is of kind {declared}")]
    ValueKindMismatch {
        /// Which element of the dense array.
        element: usize,
        /// The kind the declaration minted.
        declared: LfxParamKind,
        /// The kind the value offered.
        given: LfxParamKind,
    },
    /// The two frames are at different depths. The host never converts one to
    /// accommodate a plugin (docs/12 §3.3), so this is a caller's fault rather
    /// than a conversion.
    #[error("the input is {input} and the output is {output}, and no depth is converted here")]
    DepthMismatch {
        /// The input's depth, as `lfx_pixel_format` spells it.
        input: LfxPixelFormat,
        /// The output's depth.
        output: LfxPixelFormat,
    },
    /// A stride past the most this host will write the dense value array at.
    ///
    /// The number is the host's own rather than the header's: what the header
    /// bounds is what a *stranger* declares, and this one is what this host's
    /// caller asks for. It is bounded all the same, because the array is
    /// `stride × count` bytes and nothing else between a caller's `u32` and
    /// the allocator would notice.
    #[error(
        "the request asks for a value stride of {given} bytes, and at most {most} are written"
    )]
    ValueStride {
        /// What the caller asked for, before it was rounded up to the
        /// element's alignment.
        given: u32,
        /// [`MAX_VALUE_STRIDE`].
        most: u32,
    },
    /// A request whose region and whose buffer do not describe the same
    /// picture.
    ///
    /// The frozen `lfx_frame` says where the buffer's own top-left pixel sits
    /// and the request says which region is wanted out of it, so the two have
    /// to agree by construction or a plugin locating the first requested pixel
    /// as `roi_x0 - origin_x` writes it in the wrong place. The definition is
    /// the buffer, exactly, and the region asked for is inside it.
    #[error("the request's definition is {dod:?}, its buffer is {width}×{height}, and the region asked for is {roi:?}")]
    Region {
        /// The buffer's width.
        width: u32,
        /// The buffer's height.
        height: u32,
        /// The region asked for.
        roi: (i32, i32, i32, i32),
        /// Where the input has pixels at all.
        dod: (i32, i32, i32, i32),
    },
    /// A frame whose buffer is not four channels of `width × height`.
    ///
    /// It names which of the two halves it is about: the check runs over both,
    /// and a refusal that said only "given 8" would leave a caller - and a
    /// broker's report line - to guess which buffer to go and look at.
    #[error("a {width}×{height} frame needs {wanted} samples and the {side} was given {given}")]
    FrameSize {
        /// Which half of the call the buffer was.
        side: FrameSide,
        /// The width asked for.
        width: u32,
        /// The height asked for.
        height: u32,
        /// How many samples that comes to.
        wanted: usize,
        /// How many the caller offered.
        given: usize,
    },
    /// The plugin answered something other than `LFX_STATUS_OK`. The caller
    /// renders the input unchanged and badges the layer.
    #[error("the effect answered status {0}")]
    Process(LfxStatus),
}

/// Which half of a process call a refusal is about.
///
/// One word, so that [`LocalError::FrameSize`] names a buffer rather than a
/// number that could have come from either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameSide {
    /// The picture the effect reads.
    Input,
    /// The picture it writes.
    Output,
}

impl std::fmt::Display for FrameSide {
    fn fmt(&self, into: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        into.write_str(match self {
            FrameSide::Input => "input",
            FrameSide::Output => "output",
        })
    }
}

impl LocalError {
    /// The same fault as the refusal that crosses the pipe.
    ///
    /// **A describe that failed has to reach the host as a sentence rather than
    /// as an absence.** The describe runs in the broker, so a plugin dropped
    /// there for a structural reason would otherwise be indistinguishable from
    /// one the user switched off: not catalogued, not refused, and nowhere in
    /// the report - which leaves §5.3's `REFUSED` table with nothing to print.
    /// So the broker files one of these against the plugin's own id.
    ///
    /// Two faults keep their own names, because §4.3 and §14 item 4 name them:
    /// a plugin that declines to describe itself, and one that needs an
    /// extension this host has not got. A fault the sink itself raised is
    /// already an [`LfxRejection`] and crosses as itself. Everything else is
    /// [`LfxRejection::DescribeFailed`] carrying this host's own sentence, cut
    /// to the same number a note is cut to so that a module name nobody
    /// bounded cannot arrive as a paragraph.
    #[must_use]
    pub fn as_rejection(self, id: &str) -> LfxRejection {
        match self {
            LocalError::Refused(rejection) => rejection,
            LocalError::DescribeRefused { id } => LfxRejection::DescribeRefused { id },
            LocalError::RequiresExtension { id, extension } => {
                LfxRejection::RequiresExtension { id, extension }
            }
            other => LfxRejection::DescribeFailed {
                id: id.to_owned(),
                why: cut_to_the_log_ceiling(other.to_string()),
            },
        }
    }
}

/// The front of a sentence, held to [`LFX_MAX_LOG_BYTES`] and cut on a
/// character boundary.
///
/// The same number a log line is held to, and for the same reason: some of
/// these sentences quote a stranger's own bytes - a module path, an effect id -
/// and a refusal that crosses a pipe may not be as long as whatever the
/// stranger wrote.
fn cut_to_the_log_ceiling(text: String) -> String {
    let limit = LFX_MAX_LOG_BYTES as usize;
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default().to_owned()
}

/// One line a plugin put through `lfx_host.log`.
///
/// Diagnostics rather than identity: the host's own sentences are translated
/// and this is not, which is why it is kept as the plugin wrote it and shown
/// only in the scan report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The level the plugin filed it at.
    pub level: LfxLogLevel,
    /// What it said, cut to [`LFX_MAX_LOG_BYTES`] on a character boundary.
    pub text: String,
}

// -------------------------------------------------------- what the host is --

/// What the host remembers about one bundle's behaviour, reached from the C
/// side through `lfx_host.host_data`.
///
/// Behind a `Box` that is never moved again, because the header says the host
/// pointer handed to `create` is valid until `deinit` and **may be kept**: it
/// is the one pointer in the whole ABI with that lifetime.
#[derive(Debug, Default)]
struct HostState {
    /// What the bundle said, to the ceiling and no further.
    notes: Mutex<Vec<Note>>,
    /// Every extension id the bundle asked for, in order, so a test can see
    /// the negotiation from the plugin's side.
    asked: Mutex<Vec<String>>,
}

/// One bundle, opened in this process and driven from it.
pub struct LocalHost {
    /// Kept so the library outlives every pointer taken out of it.
    ///
    /// What holds it up is [`Drop::drop`], which runs **before** any field is
    /// dropped: `deinit` is called there, so the module is still loaded when
    /// the bundle is told to stop, and `host` and `state` - whose addresses
    /// the header says the plugin may have kept until `deinit` returns - are
    /// still where the plugin left them. Nothing here may be reordered ahead
    /// of that `Drop` impl, and the field order is not what is doing the work:
    /// fields are dropped in declaration order, so this one goes first among
    /// them.
    ///
    /// Nothing reads it - holding it *is* the effect, which is what the allow
    /// says out loud rather than papering over.
    #[allow(dead_code)]
    library: libloading::Library,
    entry: *const LfxEntry,
    /// The host the plugin is handed. Boxed so its address outlives every move
    /// of this struct.
    host: Box<LfxHost>,
    /// What `host.host_data` points at. Boxed for the same reason.
    state: Box<HostState>,
    /// Held for the whole of [`LocalHost::describe`], [`LocalHost::create`]
    /// and dropping a [`LocalInstance`], so that the header's one control
    /// thread is a fact about this type rather than an obligation on its
    /// callers.
    ///
    /// It guards nothing of this struct's own - it is a rule made mechanical.
    /// `&LocalHost` is [`Sync`], because an instance borrows the host and the
    /// suite lends instances across threads; without this lock that same
    /// `Sync` would let two threads be inside one bundle's `create` at once,
    /// which the header forbids and the bundle has no way to notice.
    control: Mutex<()>,
    path: PathBuf,
    plugins: Vec<Identity>,
    /// Per-plugin lines the descriptor read could not take at face value, in
    /// step with `plugins`.
    reports: Vec<Vec<LfxRejection>>,
    /// Lines about the bundle rather than about one effect in it.
    report: Vec<LfxRejection>,
}

// SAFETY: the entry pointer is into the loaded library's own read-only data and
// the library outlives this struct; the host and its state are behind boxes
// whose addresses never move and whose only interior mutability is a mutex. The
// one method that calls into somebody else's code from a shared reference,
// `LocalHost::create`, hands out an instance the header says may be processed
// on any worker thread.
unsafe impl Send for LocalHost {}
// SAFETY: as above, and the header's control-thread rule is kept by the
// `control` mutex rather than asked of the caller: every call into the bundle
// from a shared reference - `describe`, `create`, the `instantiate` they share,
// and an instance's own `Drop` - holds it for the whole of the call, so two
// threads sharing a `&LocalHost` serialise there instead of racing inside
// somebody else's `create`.
unsafe impl Sync for LocalHost {}

impl LocalHost {
    /// Open a module, start it, and read its descriptor list.
    ///
    /// The bundle path handed to `lfx_entry.init` is the module's own
    /// directory, so a plugin can find its resources without guessing.
    ///
    /// # Errors
    ///
    /// Every way a third party's file can disappoint: it will not load, it is
    /// not an LFX bundle, its size prefix is shorter than this header's, it
    /// speaks an ABI this host does not, its entry declares no `init`, it
    /// declines to load, or it says it holds more effects than the header
    /// carries. All of them are report lines in the end, never dialogues
    /// (docs/12 §2.6).
    pub fn open(path: &Path) -> Result<Self, LocalError> {
        // SAFETY: loading a library runs its initialisers, which is inherently
        // third-party code. There is no safe spelling of this; what makes it
        // survivable is the broker process rather than a Rust keyword, which is
        // why the editor never reaches this call and the broker always does.
        let library = unsafe { libloading::Library::new(path) }
            .map_err(|error| LocalError::NotLoaded(error.to_string()))?;

        let entry: *const LfxEntry = {
            // SAFETY: `lfx_entry_point` is a data symbol; `Symbol<*const T>`
            // reads the symbol's address as that pointer, which is what a data
            // symbol is.
            let symbol = unsafe { library.get::<*const LfxEntry>(LFX_ENTRY_SYMBOL) }
                .map_err(|_| LocalError::NoEntry)?;
            *symbol
        };
        if entry.is_null() {
            return Err(LocalError::NoEntry);
        }
        // The size prefix before the struct it prefixes, as `instantiate`
        // reads the instance table's and `readable` every declaration's. The
        // growth mechanism reads upwards and cannot read downwards, and a
        // bundle built against an earlier, shorter `lfx_entry` has a static of
        // fewer bytes than this one: forming a `&LfxEntry` over it is already
        // reading somebody else's memory, before `abi_version` is touched.
        //
        // SAFETY: a non-null data symbol from a loaded library points at that
        // library's own static, whose first word the frozen header declares to
        // be its own `struct_size`.
        let bytes = unsafe { entry.cast::<u32>().read() };
        if (bytes as usize) < size_of::<LfxEntry>() {
            return Err(LocalError::ShortEntry { bytes });
        }
        // SAFETY: the size prefix says the whole of the table is there, and the
        // library's own static lives as long as the library.
        let table = unsafe { &*entry };
        if table.abi_version != LFX_ABI_VERSION {
            return Err(LocalError::AbiUnsupported {
                declared: table.abi_version,
            });
        }

        let mut state = Box::new(HostState::default());
        let host = Box::new(LfxHost {
            struct_size: size_of::<LfxHost>() as u32,
            abi_version: LFX_ABI_VERSION,
            host_data: std::ptr::from_mut::<HostState>(&mut *state).cast::<c_void>(),
            get_extension: Some(host_get_extension),
            log: Some(host_log),
        });

        let bundle = path.parent().unwrap_or(path);
        let directory = CString::new(bundle.to_string_lossy().into_owned()).map_err(|_| {
            LocalError::UnspellablePath {
                path: bundle.to_path_buf(),
            }
        })?;
        let init = table.init.ok_or(LocalError::NoInit)?;
        // SAFETY: the entry's own function, called once, before anything else,
        // exactly as the header requires.
        //
        // The answer is a `u32` in which non-zero is true, and it is read as
        // one here: the byte a stranger's compiler left in the return register
        // is not a Rust `bool` until something says which values count.
        if (unsafe { init(directory.as_ptr()) }) == 0 {
            return Err(LocalError::InitRefused);
        }

        let mut opened = Self {
            library,
            entry,
            host,
            state,
            control: Mutex::new(()),
            path: path.to_path_buf(),
            plugins: Vec::new(),
            reports: Vec::new(),
            report: Vec::new(),
        };
        // A bundle whose own list is past the header's ceiling is refused
        // here, not truncated: `opened` is dropped on the way out, so `deinit`
        // is called and the module unloaded exactly as it would have been.
        opened.read_descriptors()?;
        Ok(opened)
    }

    /// The module this bundle came out of.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the descriptor list said, in the entry's own order.
    #[must_use]
    pub fn plugins(&self) -> &[Identity] {
        &self.plugins
    }

    /// Lines about the bundle rather than about one effect in it - a
    /// descriptor with no readable id, a descriptor shorter than this header,
    /// an id declared twice. A bundle past the header's own effect ceiling is
    /// not one of these: it never opens at all.
    #[must_use]
    pub fn report(&self) -> &[LfxRejection] {
        &self.report
    }

    /// Everything the bundle has said through `lfx_host.log`, to
    /// [`MAX_NOTES`] and no further.
    #[must_use]
    pub fn notes(&self) -> Vec<Note> {
        self.notes_from(0)
    }

    /// The lines said since the caller last looked, and only those.
    ///
    /// The list is append-only until it reaches [`MAX_NOTES`], so a caller that
    /// remembers how many it has forwarded can ask for the rest. The broker
    /// forwards on **every** answered message, a rendered frame included, and
    /// cloning the whole list there would make a plugin that logged sixty-four
    /// lines during its first describe cost sixty-four string clones a frame
    /// for the life of the process - the per-frame allocation docs/13 §3 says
    /// fails review.
    #[must_use]
    pub fn notes_from(&self, first: usize) -> Vec<Note> {
        let held = self
            .state
            .notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        held.get(first..).map(<[Note]>::to_vec).unwrap_or_default()
    }

    /// Every extension id the bundle has asked for, in order.
    #[must_use]
    pub fn extensions_asked(&self) -> Vec<String> {
        self.state
            .asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ask one effect to declare its controls.
    ///
    /// `describe` is a method on the instance, so this creates one, describes
    /// it, and takes it down again - which is what the broker does too, once
    /// per plugin rather than once per instance.
    ///
    /// # Errors
    ///
    /// The effect could not be created or initialised, its own table is
    /// shorter than this header's, it refused to describe itself, or the sink
    /// met a fault that ends the effect: a unit nobody stated, two rows on one
    /// [`ParamId`](lumit_core::fx::ParamId), more declarations than the ABI
    /// carries.
    pub fn describe(&self, id: &str) -> Result<PluginDescriptor, LocalError> {
        // The control thread's, and held to it: see `control`.
        let _control = self.control.lock().unwrap_or_else(PoisonError::into_inner);
        let index = self.index_of(id)?;
        let plugin = self.instantiate(id)?;
        let mut into = Describe::new();
        let mut sink = sink_for(&mut into);
        let describe = {
            // SAFETY: `instantiate` answered a live plugin this call owns.
            let table = unsafe { &*plugin };
            table.describe
        };
        let answered = match describe {
            // SAFETY: the plugin's own function, on the control thread, with a
            // sink whose `sink_data` is the `Describe` above and which lives
            // until this call returns. Non-zero is true, as it is at the entry.
            Some(describe) => (unsafe { describe(plugin, &mut sink) }) != 0,
            None => false,
        };
        // SAFETY: the same live plugin, on the control thread, with no process
        // running: this call has not started one.
        unsafe { destroy(plugin) };

        let described = into.finish()?;
        if !answered {
            return Err(LocalError::DescribeRefused { id: id.to_owned() });
        }
        let mut plugin = PluginDescriptor::new(
            self.plugins.get(index).cloned().unwrap_or_default(),
            described,
        );
        // The descriptor's own lines go in front of the sink's, because they
        // were met first: a count past the header's ceiling was read before
        // anything was declared.
        if let Some(lines) = self.reports.get(index) {
            let mut report = lines.clone();
            report.append(&mut plugin.report);
            plugin.report = report;
        }
        Ok(plugin)
    }

    /// Make one live instance of an effect that has already been described.
    ///
    /// The descriptor is what the dense value array is tagged from, so the
    /// instance keeps it: a value offered through the wrong arm is refused at
    /// [`LocalInstance::process`] rather than handed over.
    ///
    /// # Errors
    ///
    /// The bundle holds no effect of that name, the effect requires an
    /// extension this host does not offer, its own table is shorter than this
    /// header's, or it refused to be created or initialised.
    pub fn create(&self, plugin: &PluginDescriptor) -> Result<LocalInstance<'_>, LocalError> {
        // The control thread's, and held to it: see `control`.
        let _control = self.control.lock().unwrap_or_else(PoisonError::into_inner);
        let id = plugin.identity.id.clone();
        let instance = self.instantiate(&id)?;
        Ok(LocalInstance {
            host: self,
            plugin: instance,
            id,
            elements: elements_of(plugin),
            cancelled: Box::new(AtomicBool::new(false)),
        })
    }

    /// Where an effect sits in the descriptor list.
    fn index_of(&self, id: &str) -> Result<usize, LocalError> {
        self.plugins
            .iter()
            .position(|plugin| plugin.id == id)
            .ok_or_else(|| LocalError::NoSuchPlugin { id: id.to_owned() })
    }

    /// Negotiate, create and initialise one instance. The caller owns what
    /// comes back and must `destroy` it.
    ///
    /// The caller holds [`LocalHost::control`]: this reaches into the bundle's
    /// `create` and `init`, both of which the header puts on the one control
    /// thread.
    fn instantiate(&self, id: &str) -> Result<*mut LfxPlugin, LocalError> {
        let index = self.index_of(id)?;
        let identity = self
            .plugins
            .get(index)
            .ok_or_else(|| LocalError::NoSuchPlugin { id: id.to_owned() })?;
        // Before `create`, never after: a plugin whose required list this host
        // cannot satisfy never reaches instantiation.
        if let Some(wanted) = crate::extensions::missing_from(&identity.required_extensions) {
            return Err(LocalError::RequiresExtension {
                id: id.to_owned(),
                extension: wanted.to_owned(),
            });
        }

        // SAFETY: the entry pointer is the library's own static and the library
        // is still loaded.
        let create = unsafe { &*self.entry }
            .create
            .ok_or_else(|| LocalError::CreateRefused { id: id.to_owned() })?;
        let name = CString::new(id.as_bytes())
            .map_err(|_| LocalError::NoSuchPlugin { id: id.to_owned() })?;
        // SAFETY: the entry's own function, on the control thread, with a host
        // whose box outlives every instance this struct hands out and a
        // NUL-terminated id that lives for the call.
        let plugin = unsafe { create(&*self.host, name.as_ptr()) };
        if plugin.is_null() {
            return Err(LocalError::CreateRefused { id: id.to_owned() });
        }
        // The size prefix before the struct, as `open` reads the entry's and
        // `readable` reads every declaration's: a reference to an
        // `lfx_plugin` shorter than this header's is already reading somebody
        // else's memory, before a field of it is touched.
        //
        // SAFETY: a non-null plugin the entry just handed over, whose first
        // word the frozen header declares to be its own `struct_size`.
        let bytes = unsafe { plugin.cast::<u32>().read() };
        if (bytes as usize) < size_of::<LfxPlugin>() {
            if (bytes as usize) >= DESTROYABLE_BYTES {
                // SAFETY: the prefix says the table reaches its own `destroy`,
                // which is all `destroy` reads, and no process has started.
                unsafe { destroy(plugin) };
            }
            return Err(LocalError::ShortPlugin {
                id: id.to_owned(),
                bytes,
            });
        }
        // SAFETY: the size prefix says the whole of the table is there.
        let table = unsafe { &*plugin };
        let refused = match table.init {
            // SAFETY: the plugin's own function, on the control thread, once.
            // Non-zero is true, as it is at the entry.
            Some(init) => (unsafe { init(plugin) }) == 0,
            None => false,
        };
        if refused {
            // SAFETY: the same live plugin, with no process running.
            unsafe { destroy(plugin) };
            return Err(LocalError::InstanceRefused { id: id.to_owned() });
        }
        Ok(plugin)
    }

    /// Walk the entry's descriptor list once.
    ///
    /// # Errors
    ///
    /// The bundle declares more effects than
    /// [`LFX_MAX_EFFECTS_PER_BUNDLE`]. That is the wire's own answer to the
    /// same number - `BrokerMessage::checked` refuses a module longer than the
    /// ceiling rather than reading part of it - and it is this one too,
    /// because the alternative is a bundle that loaded 1024 of its 2000
    /// effects and reads as refused to a caller asking the report and as
    /// whole to a caller that does not.
    fn read_descriptors(&mut self) -> Result<(), LocalError> {
        // SAFETY: the entry pointer is the library's own static and the library
        // is still loaded.
        let table = unsafe { &*self.entry };
        let (Some(count), Some(descriptor)) = (table.count, table.descriptor) else {
            return Ok(());
        };
        // SAFETY: the entry's own function, on the control thread.
        let total = unsafe { count() };
        if total > LFX_MAX_EFFECTS_PER_BUNDLE {
            return Err(LfxRejection::PastCeiling {
                ceiling: Ceiling::EffectsPerBundle,
                subject: "the bundle",
                given: u64::from(total),
            }
            .into());
        }
        let list: Vec<*const LfxDescriptor> = (0..total)
            // SAFETY: `index` is below the count the entry itself reported.
            .map(|index| unsafe { descriptor(index) })
            .collect();
        // SAFETY: each entry is null or the bundle's own static, which the
        // header says stays valid and unchanged until `deinit`.
        let catalogue = unsafe { catalogue_of(&list) };
        self.plugins = catalogue.plugins;
        self.reports = catalogue.reports;
        self.report = catalogue.report;
        Ok(())
    }
}

/// What one bundle's descriptor list came to: the effects it holds, the lines
/// each of their own numbers could not be taken at face value for, and the
/// lines about the bundle rather than about any one of them.
#[derive(Debug, Default)]
struct Catalogue {
    /// The effects, in the entry's own order.
    plugins: Vec<Identity>,
    /// Per-plugin lines, in step with `plugins`.
    reports: Vec<Vec<LfxRejection>>,
    /// Lines about the bundle.
    report: Vec<LfxRejection>,
}

/// Read a bundle's descriptor list into a catalogue.
///
/// Taken out of [`LocalHost`] so that a list of descriptors written by hand is
/// walked by exactly the code a bundle's own list is: what this answers to a
/// descriptor shorter than the header, to one with no readable id and to two
/// declaring the same id are the three things no fixture can be made to
/// declare.
///
/// **An id declared twice is not a second effect.** `index_of` is a search by
/// id, so a repeated one would give two rows in the Add-effect menu, two match
/// names and one frame key, with `describe` and `create` on either silently
/// driving the first - the collision
/// [`LfxRejection::DuplicateParamId`] refuses one level down, arriving where it
/// decides which effect a saved project resolves to.
///
/// # Safety
///
/// Each entry must be null or a live `lfx_descriptor` the bundle owns, whose
/// strings and arrays are as the header declares them.
unsafe fn catalogue_of(list: &[*const LfxDescriptor]) -> Catalogue {
    let mut catalogue = Catalogue::default();
    for raw in list {
        if raw.is_null() {
            continue;
        }
        // SAFETY: the caller's contract, passed straight on.
        let (identity, lines) = match unsafe { identity_of(*raw) } {
            Ok(read) => read,
            // An effect nobody can name is one nothing else can stand in for:
            // there is no row to file the line against, so it is filed against
            // the bundle.
            Err(line) => {
                catalogue.report.push(line);
                continue;
            }
        };
        if catalogue
            .plugins
            .iter()
            .any(|already| already.id == identity.id)
        {
            catalogue
                .report
                .push(LfxRejection::DuplicateEffectId { id: identity.id });
            continue;
        }
        catalogue.plugins.push(identity);
        catalogue.reports.push(lines);
    }
    catalogue
}

impl Drop for LocalHost {
    fn drop(&mut self) {
        // This body runs before any field is dropped, which is what makes the
        // order right: the library is still loaded, and `host` and `state` -
        // whose addresses the header says the plugin may have kept - are still
        // where the plugin left them until `deinit` has returned.
        //
        // SAFETY: the entry pointer is the library's own static and the
        // library is still loaded.
        let table = unsafe { &*self.entry };
        if let Some(deinit) = table.deinit {
            // SAFETY: the entry's own function, paired with the `init` that
            // succeeded in `open`, and after every instance is gone: an
            // instance borrows this struct, so none can outlive it.
            unsafe { deinit() };
        }
    }
}

/// How many bytes of an `lfx_plugin` reach the end of its `destroy` pointer,
/// which is the least a table must carry for the host to be able to take it
/// down at all.
const DESTROYABLE_BYTES: usize = std::mem::offset_of!(LfxPlugin, destroy)
    + size_of::<Option<unsafe extern "C" fn(*mut LfxPlugin)>>();

/// Take one instance down.
///
/// # Safety
///
/// `plugin` must be a live plugin this crate created and has not destroyed,
/// whose size prefix is at least [`DESTROYABLE_BYTES`], and no process call may
/// be running on it.
unsafe fn destroy(plugin: *mut LfxPlugin) {
    // The one field is reached as a raw place rather than through a
    // `&LfxPlugin`, so that a table long enough to carry `destroy` and no
    // longer can still be taken down.
    //
    // SAFETY: the caller's contract.
    if let Some(destroy) = unsafe { (&raw const (*plugin).destroy).read() } {
        // SAFETY: the plugin's own function, on the control thread.
        unsafe { destroy(plugin) };
    }
}

// --------------------------------------------------- reading a descriptor --

/// What a descriptor says, and the lines its own numbers could not be taken at
/// face value for.
///
/// The `Err` is the line filed against the bundle: a descriptor whose size
/// prefix is shorter than this header's, or one with no readable id - which is
/// the one field nothing else can stand in for.
///
/// **The size prefix is read before the struct is**, as
/// [`readable`] reads every declaration's and [`LocalHost::open`] reads the
/// entry's: a `&LfxDescriptor` over four bytes claiming to be a descriptor is
/// already reading somebody else's memory, before `major`, `category_count` or
/// `traits` is touched. The same rule again one struct further in, at the trait
/// block `traits` points at, which is the last of the ABI's five.
///
/// # Safety
///
/// `descriptor` must point at at least four readable bytes whose first word is
/// the descriptor's own `struct_size`, and at a live `lfx_descriptor` the
/// bundle owns - whose strings and arrays are as the header declares them -
/// when that word is at least `size_of::<LfxDescriptor>()`. Its `traits` must
/// be null or point at at least four readable bytes whose first word is the
/// block's own `struct_size`, and at a live `lfx_traits` when that word is at
/// least `size_of::<LfxTraits>()`.
unsafe fn identity_of(
    descriptor: *const LfxDescriptor,
) -> Result<(Identity, Vec<LfxRejection>), LfxRejection> {
    // SAFETY: the caller's contract: the first word of every struct in the
    // frozen header is its `uint32_t struct_size`.
    let bytes = unsafe { descriptor.cast::<u32>().read() };
    if (bytes as usize) < size_of::<LfxDescriptor>() {
        return Err(LfxRejection::UnreadableDeclaration {
            kind: lumit_lfx_abi::LFX_PARAM_UNSET,
            bytes,
        });
    }
    // The line a descriptor nobody can name is filed under, which carries no
    // size prefix because the prefix was not the fault.
    let nameless = || LfxRejection::UnreadableDeclaration {
        kind: lumit_lfx_abi::LFX_PARAM_UNSET,
        bytes: 0,
    };
    // SAFETY: the size prefix says the whole of the struct is there.
    let descriptor = unsafe { &*descriptor };
    // SAFETY: the caller's contract: the header says every `const char *` here
    // is NUL-terminated within `LFX_MAX_STRING_BYTES`.
    let (id, name, vendor) = unsafe {
        (
            text_within(descriptor.id).ok_or_else(nameless)?,
            text_within(descriptor.name).unwrap_or_default(),
            text_within(descriptor.vendor).unwrap_or_default(),
        )
    };
    if id.is_empty() {
        return Err(nameless());
    }
    let mut lines = Vec::new();

    // Counted, then read - and a count past the ceiling means the array is not
    // read **at all**, which is the same answer `within` gives a declaration's
    // counted lists. Clamping the count to the ceiling and reading to it would
    // be reading up to eight values, or sixteen `const char *`, out of an
    // array the plugin's own declaration need not have made that long: the
    // read the check exists to prevent, performed because the check failed.
    let mut categories = Vec::new();
    if descriptor.category_count > LFX_MAX_CATEGORIES {
        lines.push(LfxRejection::TooManyCategories {
            declared: descriptor.category_count,
        });
    } else if !descriptor.categories.is_null() {
        // SAFETY: the caller's contract, and the count is inside the header's
        // own ceiling: `categories` is an array of `category_count` values.
        categories.extend_from_slice(unsafe {
            std::slice::from_raw_parts(descriptor.categories, descriptor.category_count as usize)
        });
    }

    let mut required = Vec::new();
    if descriptor.required_extension_count > LFX_MAX_REQUIRED_EXTENSIONS {
        lines.push(LfxRejection::TooManyRequiredExtensions {
            declared: descriptor.required_extension_count,
        });
    } else if !descriptor.required_extensions.is_null() {
        for index in 0..descriptor.required_extension_count as usize {
            // SAFETY: as above; `index` is below the count, which is inside
            // the ceiling.
            let entry = unsafe { *descriptor.required_extensions.add(index) };
            // SAFETY: an entry in that array is a NUL-terminated string.
            if let Some(text) = unsafe { text_within(entry) } {
                required.push(text);
            }
        }
    }

    let traits = if descriptor.traits.is_null() {
        None
    } else {
        // The prefix before the block, as this function read the descriptor's
        // own a few lines up: a bundle built against an earlier, shorter
        // `lfx_traits` hands over an object smaller than this header's, and a
        // `&LfxTraits` formed over it is already reading somebody else's
        // memory. A short block reads as the pessimistic case with no help
        // from here - every enumeration starts at `UNSET = 0` - so it is read
        // as the `NULL` it is indistinguishable from.
        //
        // SAFETY: the caller's contract: a non-null trait block's first word is
        // its own `struct_size`.
        let bytes = unsafe { descriptor.traits.cast::<u32>().read() };
        if (bytes as usize) < size_of::<lumit_lfx_abi::LfxTraits>() {
            None
        } else {
            // SAFETY: the size prefix says the whole of the block is there.
            Some(Traits::from_abi(unsafe { &*descriptor.traits }))
        }
    };

    Ok((
        Identity {
            id,
            name,
            vendor,
            major: descriptor.major,
            minor: descriptor.minor,
            patch: descriptor.patch,
            categories,
            traits,
            required_extensions: required,
        },
        lines,
    ))
}

/// A string the plugin owns, read to the first NUL inside
/// [`LFX_MAX_STRING_BYTES`] - or `None`, which is the whole of the answer to
/// one that has no end inside it. **The host never walks past the limit looking
/// for the end.**
///
/// Read one byte at a time rather than as a slice of the ceiling's length,
/// which would read past the allocation of every honest short string.
/// *ponytail:* a plugin whose string has no NUL at all is still over-read by
/// whatever lies between its end and the ceiling, which is the read `strlen`
/// itself makes; there is no better answer available at this boundary.
///
/// # Safety
///
/// `pointer` must be null or a string the plugin owns, NUL-terminated within
/// [`LFX_MAX_STRING_BYTES`] as the header declares every string here to be.
unsafe fn text_within(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let ceiling = LFX_MAX_STRING_BYTES as usize;
    let mut bytes: Vec<u8> = Vec::new();
    for index in 0..ceiling {
        // SAFETY: the caller's contract: the NUL is inside the ceiling, so
        // every byte up to and including it is the plugin's own.
        let byte = unsafe { *pointer.add(index) } as u8;
        if byte == 0 {
            return Some(String::from_utf8_lossy(&bytes).into_owned());
        }
        bytes.push(byte);
    }
    None
}

// ------------------------------------------------- what the plugin is given --

/// The host's `get_extension`: a typed table for `id` at `version`, or null.
///
/// Version 1 offers none ([`OFFERED_EXTENSIONS`]), and a missing extension is a
/// null rather than a status - which is the header's own rule, not a gap.
///
/// # Safety
///
/// `host` must be one of this module's own, and `id` a NUL-terminated string
/// valid for the call.
unsafe extern "C" fn host_get_extension(
    host: *const LfxHost,
    id: *const c_char,
    _version: u32,
) -> *const c_void {
    // SAFETY: the caller's contract, passed straight on.
    if let (Some(state), Some(name)) = (unsafe { state_of(host) }, unsafe { text_within(id) }) {
        let mut asked = state.asked.lock().unwrap_or_else(PoisonError::into_inner);
        if asked.len() < MAX_NOTES {
            asked.push(name);
        }
    }
    std::ptr::null()
}

/// The host's `log`: one line of diagnostics, cut to [`LFX_MAX_LOG_BYTES`] on a
/// character boundary and kept to [`MAX_NOTES`] lines.
///
/// Cut rather than refused, for the reason the wire uses the same way: a plugin
/// that says too much should still be heard.
///
/// # Safety
///
/// As [`host_get_extension`], with `message` in place of `id`.
unsafe extern "C" fn host_log(host: *const LfxHost, level: LfxLogLevel, message: *const c_char) {
    // SAFETY: the caller's contract.
    let Some(state) = (unsafe { state_of(host) }) else {
        return;
    };
    let mut notes = state.notes.lock().unwrap_or_else(PoisonError::into_inner);
    if notes.len() >= MAX_NOTES {
        return;
    }
    // SAFETY: the caller's contract. A log line is allowed its own, longer
    // ceiling, being diagnostics rather than identity.
    let Some(text) = (unsafe { log_text(message) }) else {
        return;
    };
    notes.push(Note { level, text });
}

/// The state behind a host pointer, or `None` for one that is not this
/// module's.
///
/// # Safety
///
/// `host` must be null or one of this module's own hosts, whose `host_data`
/// points at the [`HostState`] boxed beside it.
unsafe fn state_of<'a>(host: *const LfxHost) -> Option<&'a HostState> {
    if host.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    let data = unsafe { &*host }.host_data.cast::<HostState>();
    if data.is_null() {
        return None;
    }
    // SAFETY: as above: the box lives as long as the `LocalHost`, which the
    // header says outlives every instance.
    Some(unsafe { &*data })
}

/// A log line, read to the first NUL inside [`LFX_MAX_LOG_BYTES`] and cut on a
/// character boundary.
///
/// # Safety
///
/// As [`text_within`], with the log's own ceiling.
unsafe fn log_text(message: *const c_char) -> Option<String> {
    if message.is_null() {
        return None;
    }
    let ceiling = LFX_MAX_LOG_BYTES as usize;
    let mut bytes: Vec<u8> = Vec::new();
    for index in 0..ceiling {
        // SAFETY: the caller's contract.
        let byte = unsafe { *message.add(index) } as u8;
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    // `from_utf8_lossy` over a run cut at the ceiling can split a character;
    // the replacement it puts there is the cut said out loud rather than a
    // half-character kept.
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

// -------------------------------------------------------------- the sink --

/// The frozen sink, filled in, with `sink_data` pointing at the [`Describe`]
/// every entry point below pushes into.
fn sink_for(into: &mut Describe) -> LfxDescribeSink {
    LfxDescribeSink {
        struct_size: size_of::<LfxDescribeSink>() as u32,
        sink_data: std::ptr::from_mut::<Describe>(into).cast::<c_void>(),
        declare_float: Some(declare_float),
        declare_slider: Some(declare_slider),
        declare_int: Some(declare_int),
        declare_angle: Some(declare_angle),
        declare_bool: Some(declare_bool),
        declare_choice: Some(declare_choice),
        declare_colour: Some(declare_colour),
        declare_seed: Some(declare_seed),
        declare_point2: Some(declare_point2),
        declare_point3: Some(declare_point3),
        declare_curve: Some(declare_curve),
        declare_file: Some(declare_file),
        declare_action: Some(declare_action),
        group_begin: Some(group_begin),
        group_end: Some(group_end),
    }
}

/// Run `body` against the sink's own [`Describe`], or answer `false` for a sink
/// that is not one of ours.
///
/// # Safety
///
/// `sink` must be null or one of [`sink_for`]'s, whose `sink_data` points at a
/// [`Describe`] that lives for the call.
unsafe fn with_sink(sink: *mut LfxDescribeSink, body: impl FnOnce(&mut Describe) -> bool) -> bool {
    if sink.is_null() {
        return false;
    }
    // SAFETY: the caller's contract.
    let data = unsafe { &*sink }.sink_data.cast::<Describe>();
    if data.is_null() {
        return false;
    }
    // SAFETY: as above. One sink is used by one `describe` on one thread, so
    // this exclusive borrow is the only one.
    body(unsafe { &mut *data })
}

/// The declaration behind a pointer, once its size prefix says the fields this
/// header reads are there to read. `None` is the row declined and the line
/// already filed.
///
/// **The size prefix is read before the struct is**, which is why it is read as
/// the `uint32_t` it is rather than through a reference to the whole
/// declaration: a four-byte allocation claiming to be four bytes is a lie this
/// host must be able to catch without having already trusted it. Every struct
/// in the frozen header opens with that word, which is the guarantee this rests
/// on.
///
/// # Safety
///
/// `declared` must be null or point at at least four readable bytes whose first
/// word is the declaration's own `struct_size`, and at a live `T` when that
/// word is at least `size_of::<T>()`.
unsafe fn readable<'a, T>(
    declared: *const T,
    kind: LfxParamKind,
    into: &mut Describe,
) -> Option<&'a T> {
    if declared.is_null() {
        into.decline(LfxRejection::UnreadableDeclaration { kind, bytes: 0 });
        return None;
    }
    // SAFETY: the caller's contract: the first word of every declaration in the
    // frozen header is its `uint32_t struct_size`.
    let bytes = unsafe { declared.cast::<u32>().read() };
    if (bytes as usize) < size_of::<T>() {
        into.decline(LfxRejection::UnreadableDeclaration { kind, bytes });
        return None;
    }
    // SAFETY: the size prefix says the whole of `T` is there.
    Some(unsafe { &*declared })
}

/// The three words every declaration opens with after its size: the id and the
/// label read to the header's ceiling, and the unit lowered onto Lumit's own.
///
/// `None` is a row declined and the line already filed.
///
/// # Safety
///
/// `id` and `label` must be null or strings the plugin owns, NUL-terminated
/// within [`LFX_MAX_STRING_BYTES`].
unsafe fn opening(
    kind: LfxParamKind,
    into: &mut Describe,
    id: *const c_char,
    label: *const c_char,
) -> Option<(String, String)> {
    // SAFETY: the caller's contract.
    let Some(id) = (unsafe { text_within(id) }) else {
        // Nought, as the null declaration is filed under, and for the same
        // reason: the field the line's number names is the size prefix, and
        // here the size prefix was not the fault. A declaration whose
        // identifier has no end inside the ceiling has no readable prefix
        // *of the kind this number is about* to quote.
        into.decline(LfxRejection::UnreadableDeclaration { kind, bytes: 0 });
        return None;
    };
    // SAFETY: as above.
    let Some(label) = (unsafe { text_within(label) }) else {
        into.decline(LfxRejection::StringTooLong {
            id,
            bytes: LFX_MAX_STRING_BYTES,
        });
        return None;
    };
    Some((id, label))
}

/// A count the header puts a ceiling on, checked **before** its array is read.
///
/// `false` means the array is not to be read at all, and the line is filed from
/// the count the plugin declared rather than from the number that was read.
fn within(
    into: &mut Describe,
    given: u32,
    ceiling: u32,
    line: impl FnOnce() -> LfxRejection,
) -> bool {
    if given > ceiling {
        into.decline(line());
        return false;
    }
    true
}

/// An array of `count` strings the plugin owns, read to the header's ceiling.
///
/// # Safety
///
/// `list` must be null or an array of at least `count` NUL-terminated strings.
unsafe fn texts(list: *const *const c_char, count: u32) -> Vec<String> {
    if list.is_null() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        // SAFETY: the caller's contract; `index` is below `count`.
        let entry = unsafe { *list.add(index) };
        // SAFETY: an entry in that array is a NUL-terminated string.
        out.push(unsafe { text_within(entry) }.unwrap_or_default());
    }
    out
}

/// One declaration, from the four words every kind opens with to the push.
///
/// Every entry point below is this shape and differs in one expression - what
/// the kind's own numbers make - so the shape is written once.
macro_rules! declaration {
    ($name:ident, $param:ty, $tag:expr, |$declared:ident, $into:ident| $kind:expr) => {
        /// Declare one control. The plugin's side of this is the frozen sink.
        ///
        /// # Safety
        ///
        /// `sink` must be one of [`sink_for`]'s, and `declared` null or a live
        /// declaration of this kind.
        unsafe extern "C" fn $name(sink: *mut LfxDescribeSink, declared: *const $param) -> bool {
            let body = |$into: &mut Describe| -> bool {
                // SAFETY: the caller's contract: the declaration's first word
                // is its own `struct_size`, which is what `readable` reads
                // before it trusts the rest.
                let Some($declared) = (unsafe { readable(declared, $tag, $into) }) else {
                    return false;
                };
                // SAFETY: the header declares both of these NUL-terminated
                // within `LFX_MAX_STRING_BYTES`.
                let Some((id, label)) =
                    (unsafe { opening($tag, $into, $declared.id, $declared.label) })
                else {
                    return false;
                };
                let Some(kind) = ($kind) else { return false };
                $into.declare(Declaration {
                    id,
                    label,
                    unit: schema::unit_of($declared.unit),
                    flags: $declared.flags,
                    kind,
                })
            };
            // SAFETY: the caller's contract: a sink this module built, whose
            // `sink_data` is the `Describe` the body above pushes into.
            unsafe { with_sink(sink, body) }
        }
    };
}

declaration!(
    declare_float,
    LfxFloatParam,
    LFX_PARAM_FLOAT,
    |declared, _into| { Some(Declared::float_from_abi(declared)) }
);
declaration!(
    declare_slider,
    LfxSliderParam,
    LFX_PARAM_SLIDER,
    |declared, _into| { Some(Declared::slider_from_abi(declared)) }
);
declaration!(
    declare_int,
    LfxIntParam,
    LFX_PARAM_INT,
    |declared, _into| { Some(Declared::int_from_abi(declared)) }
);
declaration!(
    declare_bool,
    LfxBoolParam,
    LFX_PARAM_BOOL,
    |declared, _into| { Some(Declared::bool_from_abi(declared)) }
);
declaration!(
    declare_angle,
    LfxAngleParam,
    LFX_PARAM_ANGLE,
    |declared, _into| {
        Some(Declared::Angle {
            default: declared.default_value,
            dial_step: declared.dial_step,
        })
    }
);
declaration!(
    declare_colour,
    LfxColourParam,
    LFX_PARAM_COLOUR,
    |declared, _into| {
        Some(Declared::Colour {
            default: declared.default_rgba,
            range: (declared.range_min, declared.range_max),
        })
    }
);
declaration!(
    declare_seed,
    LfxSeedParam,
    LFX_PARAM_SEED,
    |_declared, _into| { Some(Declared::Seed) }
);
declaration!(
    declare_action,
    LfxActionParam,
    LFX_PARAM_ACTION,
    |_declared, _into| { Some(Declared::Action) }
);
declaration!(
    declare_point2,
    LfxPoint2Param,
    LFX_PARAM_POINT2,
    |declared, _into| {
        Some(Declared::Point2 {
            default: (declared.default_x, declared.default_y),
            slider: (declared.slider_min, declared.slider_max),
        })
    }
);
declaration!(
    declare_point3,
    LfxPoint3Param,
    LFX_PARAM_POINT3,
    |declared, _into| {
        Some(Declared::Point3 {
            default: (declared.default_x, declared.default_y, declared.default_z),
            slider: (declared.slider_min, declared.slider_max),
        })
    }
);
declaration!(
    declare_choice,
    LfxChoiceParam,
    LFX_PARAM_CHOICE,
    |declared, into| {
        // Counted, then read. The id is already in hand, so the line names the
        // control as well as the count it declared.
        let id = || {
            // SAFETY: the declaration's own id, read to the ceiling above.
            unsafe { text_within(declared.id) }.unwrap_or_default()
        };
        if !within(into, declared.option_count, LFX_MAX_OPTIONS, || {
            LfxRejection::TooManyOptions {
                id: id(),
                declared: declared.option_count,
            }
        }) || !within(into, declared.divider_count, LFX_MAX_DIVIDERS, || {
            LfxRejection::TooManyDividers {
                id: id(),
                declared: declared.divider_count,
            }
        }) {
            None
        } else {
            let dividers_after = if declared.dividers_after.is_null() {
                Vec::new()
            } else {
                // SAFETY: the header declares an array of `divider_count` values,
                // and the count has been held to the ceiling above.
                unsafe {
                    std::slice::from_raw_parts(
                        declared.dividers_after,
                        declared.divider_count as usize,
                    )
                }
                .to_vec()
            };
            Some(Declared::Choice {
                // SAFETY: as above, for `option_count` strings.
                options: unsafe { texts(declared.options, declared.option_count) },
                default: declared.default_index,
                dividers_after,
            })
        }
    }
);
declaration!(
    declare_file,
    LfxFileParam,
    LFX_PARAM_FILE,
    |declared, into| {
        if within(into, declared.filter_count, LFX_MAX_FILTERS, || {
            LfxRejection::TooManyFilters {
                // SAFETY: the declaration's own id, read to the ceiling above.
                id: unsafe { text_within(declared.id) }.unwrap_or_default(),
                declared: declared.filter_count,
            }
        }) {
            Some(Declared::File {
                // SAFETY: the header declares an array of `filter_count` strings,
                // and the count has been held to the ceiling above.
                filter: unsafe { texts(declared.filter, declared.filter_count) },
                // SAFETY: a NUL-terminated string the plugin owns.
                filter_name: unsafe { text_within(declared.filter_name) }.unwrap_or_default(),
            })
        } else {
            None
        }
    }
);
declaration!(
    declare_curve,
    LfxCurveParam,
    LFX_PARAM_CURVE,
    |declared, into| {
        if !within(into, declared.point_count, LFX_MAX_CURVE_POINTS, || {
            LfxRejection::CurvePointsOutOfRange {
                // SAFETY: the declaration's own id, read to the ceiling above.
                id: unsafe { text_within(declared.id) }.unwrap_or_default(),
                declared: declared.point_count,
            }
        }) {
            // The line is already filed, and it names the count the plugin
            // declared. Declaring the row anyway would file a second one -
            // `undrawable` naming the nought points that were read - so the
            // report would carry two sentences for one control and the honest
            // number would be the one nobody reads first.
            None
        } else if declared.points.is_null() {
            // A curve with no points at all is a curve too short, which the
            // lowering names for itself once the declaration is built.
            Some(Declared::Curve {
                default: Vec::new(),
            })
        } else {
            // SAFETY: the header declares `2 * point_count` floats, x then y, and
            // the count has been held to the ceiling above.
            let flat = unsafe {
                std::slice::from_raw_parts(declared.points, (declared.point_count as usize) * 2)
            };
            Some(Declared::Curve {
                default: flat
                    .chunks_exact(2)
                    .map(|pair| [pair[0], pair[1]])
                    .collect(),
            })
        }
    }
);

/// Open a heading over the rows declared until the matching `group_end`.
///
/// # Safety
///
/// `sink` must be one of [`sink_for`]'s, and `declared` null or a live
/// `lfx_group_param`.
unsafe extern "C" fn group_begin(
    sink: *mut LfxDescribeSink,
    declared: *const LfxGroupParam,
) -> bool {
    let body = |into: &mut Describe| -> bool {
        // SAFETY: the caller's contract, as in every other entry point.
        let Some(declared) = (unsafe { readable(declared, lumit_lfx_abi::LFX_PARAM_GROUP, into) })
        else {
            return false;
        };
        // SAFETY: as above.
        let Some((id, label)) = (unsafe {
            opening(
                lumit_lfx_abi::LFX_PARAM_GROUP,
                into,
                declared.id,
                declared.label,
            )
        }) else {
            return false;
        };
        into.group_begin(&id, &label, declared.flags)
    };
    // SAFETY: the caller's contract: a sink this module built.
    unsafe { with_sink(sink, body) }
}

/// Close the heading the last `group_begin` opened.
///
/// # Safety
///
/// `sink` must be one of [`sink_for`]'s.
unsafe extern "C" fn group_end(sink: *mut LfxDescribeSink) -> bool {
    // SAFETY: the caller's contract.
    unsafe { with_sink(sink, Describe::group_end) }
}

// ------------------------------------------------------------ the values --

/// One element of the dense value array, in the arm its declaration's kind
/// names.
///
/// One variant per kind that crosses, so an element is self-describing: the
/// `kind` tag the host writes beside it is read off the value rather than
/// invented, and the tag is then checked against the declaration it belongs to.
/// That is what makes a kind mismatch a refusal here rather than a plugin
/// reading the wrong arm of a union.
///
/// Resolving a bag of the project's own values into this list is the discovery
/// package's, through [`schema::value_routes`]; what is here is the carriage.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An unbounded number.
    Float(f64),
    /// A bounded number.
    Slider(f64),
    /// An angle, in degrees.
    Angle(f64),
    /// A whole number.
    Int(i64),
    /// A seed.
    Seed(i64),
    /// A switch.
    Bool(bool),
    /// The chosen index of a dropdown.
    Choice(u32),
    /// Scene-linear RGBA.
    Colour([f32; 4]),
    /// **Both** axes of a point, in one element.
    Point2([f32; 2]),
    /// All three axes of a point, in one element.
    Point3([f32; 3]),
    /// A tone curve's control points, x then y, in the unit square.
    Curve(Vec<[f32; 2]>),
    /// The file the host actually loaded, or `None` - which, until the generic
    /// file aux lands, is every File row.
    File(Option<CString>),
}

impl Value {
    /// The frozen ABI's own tag for this value, which is the tag its
    /// declaration minted and the arm of the union it is read through.
    #[must_use]
    pub const fn tag(&self) -> LfxParamKind {
        match self {
            Value::Float(_) => LFX_PARAM_FLOAT,
            Value::Slider(_) => LFX_PARAM_SLIDER,
            Value::Angle(_) => LFX_PARAM_ANGLE,
            Value::Int(_) => LFX_PARAM_INT,
            Value::Seed(_) => LFX_PARAM_SEED,
            Value::Bool(_) => LFX_PARAM_BOOL,
            Value::Choice(_) => LFX_PARAM_CHOICE,
            Value::Colour(_) => LFX_PARAM_COLOUR,
            Value::Point2(_) => LFX_PARAM_POINT2,
            Value::Point3(_) => LFX_PARAM_POINT3,
            Value::Curve(_) => LFX_PARAM_CURVE,
            Value::File(_) => LFX_PARAM_FILE,
        }
    }

    /// The same value, as it arrived on the control plane.
    ///
    /// [`crate::ipc::proto::ParamValue`] is the wire's twelve arms and this is
    /// the ABI's, one for one, so the conversion is a match with no `_` arm and
    /// nothing to decide. The one thing it does do is turn the wire's `String`
    /// into a `CString`: `lfx_value.v.file.path` is a `const char *`, and a
    /// path with a NUL in the middle of it is not a path this host will hand
    /// over at all - it becomes the `None` every File row carries today anyway,
    /// until the render pass's generic file aux lands.
    #[must_use]
    pub fn from_wire(value: &crate::ipc::proto::ParamValue) -> Self {
        use crate::ipc::proto::ParamValue;
        match value {
            ParamValue::Float(number) => Value::Float(*number),
            ParamValue::Slider(number) => Value::Slider(*number),
            ParamValue::Angle(number) => Value::Angle(*number),
            ParamValue::Int(whole) => Value::Int(*whole),
            ParamValue::Seed(whole) => Value::Seed(*whole),
            ParamValue::Bool(switch) => Value::Bool(*switch),
            ParamValue::Choice(chosen) => Value::Choice(*chosen),
            ParamValue::Colour(rgba) => Value::Colour(*rgba),
            ParamValue::Point2(xy) => Value::Point2(*xy),
            ParamValue::Point3(xyz) => Value::Point3(*xyz),
            ParamValue::Curve(points) => Value::Curve(points.clone()),
            ParamValue::File(path) => Value::File(
                path.as_ref()
                    .and_then(|path| CString::new(path.as_bytes()).ok()),
            ),
        }
    }
}

/// The kind tag of every declaration that carries a value, in declaration
/// order - which is the order and the length of the dense array.
///
/// An Action carries none, and a heading is not a declaration at all, so
/// neither is here: the same answer [`schema::Carriage::crosses`] gives one
/// level down, and `the_elements_the_host_writes_are_the_ones_that_cross` is
/// where the two are held together.
fn elements_of(plugin: &PluginDescriptor) -> Vec<LfxParamKind> {
    plugin
        .params
        .iter()
        .map(|declaration| declaration.kind.tag())
        .filter(|tag| *tag != LFX_PARAM_ACTION)
        .collect()
}

// ------------------------------------------------------------ the frames --

/// One request to render one frame: the numbers `lfx_process` carries that are
/// not the values or the pictures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    /// The comp frame being rendered, as a decimal.
    pub time: f64,
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// The output region asked for: `x0`, `y0` inclusive, `x1`, `y1` exclusive.
    pub roi: (i32, i32, i32, i32),
    /// Where the input has pixels at all, in the same spelling.
    pub dod: (i32, i32, i32, i32),
    /// Bytes between two elements of the dense value array, or `None` for the
    /// struct's own size.
    ///
    /// It is a field rather than a constant because the stride is the whole of
    /// what the two sides agree on for `lfx_value` - the one struct with no
    /// size prefix - so a host that can write a wider one is a host whose
    /// plugins can be held to walking by it (docs/impl/lfx.md §11 item 15).
    pub value_stride: Option<u32>,
}

impl Request {
    /// The whole frame, which is the degenerate region rather than the
    /// assumption.
    #[must_use]
    pub fn full_frame(time: f64, width: u32, height: u32) -> Self {
        let (across, down) = (
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
        );
        Self {
            time,
            width,
            height,
            roi: (0, 0, across, down),
            dod: (0, 0, across, down),
            value_stride: None,
        }
    }

    /// The stride the dense value array is actually written at: the caller's
    /// own, raised to one element's size and rounded up to its alignment.
    ///
    /// **Rounded up rather than taken as given.** A plugin reaches an element
    /// as `(const lfx_value *)((const char *)values + i * value_stride)` and
    /// reads it with an ordinary aligned read, which is what the header asks
    /// of it; a stride that is not a multiple of the struct's alignment would
    /// put every other element four bytes off and make that read undefined in
    /// C and a fault on a strict-alignment target.
    ///
    /// # Errors
    ///
    /// The caller asked for more than [`MAX_VALUE_STRIDE`].
    fn stride(&self) -> Result<u32, LocalError> {
        let least = size_of::<LfxValue>() as u32;
        let given = self.value_stride.unwrap_or(least).max(least);
        if given > MAX_VALUE_STRIDE {
            return Err(LocalError::ValueStride {
                given,
                most: MAX_VALUE_STRIDE,
            });
        }
        Ok(given.next_multiple_of(align_of::<LfxValue>() as u32))
    }

    /// Whether the region this request names and the buffer it is about
    /// describe one picture.
    ///
    /// The definition **is** the buffer, exactly, and the region asked for is
    /// inside it. The frozen `lfx_frame` carries the buffer's own top-left
    /// corner and the request carries the region wanted out of it, so a plugin
    /// locating the first requested pixel as `roi_x0 - origin_x` is right only
    /// if the two were written from the same rectangle. Refusing the pair the
    /// host cannot write honestly is cheaper than a frame drawn in the wrong
    /// corner.
    ///
    /// # Errors
    ///
    /// The definition is not the buffer's own size, or the region asked for
    /// reaches outside it, or its ends are the wrong way round.
    fn regions_agree(&self) -> Result<(), LocalError> {
        let (x0, y0, x1, y1) = self.dod;
        let (rx0, ry0, rx1, ry1) = self.roi;
        let agree = i64::from(x1) - i64::from(x0) == i64::from(self.width)
            && i64::from(y1) - i64::from(y0) == i64::from(self.height)
            && rx0 >= x0
            && ry0 >= y0
            && rx1 <= x1
            && ry1 <= y1
            && rx0 <= rx1
            && ry0 <= ry1;
        if agree {
            Ok(())
        } else {
            Err(LocalError::Region {
                width: self.width,
                height: self.height,
                roi: self.roi,
                dod: self.dod,
            })
        }
    }
}

/// The pixels of one picture to read, at whichever depth the project is in.
#[derive(Clone, Copy, Debug)]
pub enum Pixels<'a> {
    /// Half floats, which is what Lumit's working texture holds.
    F16(&'a [f16]),
    /// Single floats.
    F32(&'a [f32]),
}

/// The pixels of one picture to write.
#[derive(Debug)]
pub enum PixelsMut<'a> {
    /// Half floats.
    F16(&'a mut [f16]),
    /// Single floats.
    F32(&'a mut [f32]),
}

impl Pixels<'_> {
    /// The depth, as `lfx_pixel_format` spells it.
    #[must_use]
    pub const fn format(&self) -> LfxPixelFormat {
        match self {
            Pixels::F16(_) => LFX_RGBA_F16,
            Pixels::F32(_) => LFX_RGBA_F32,
        }
    }

    /// How many samples there are: four per pixel.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Pixels::F16(half) => half.len(),
            Pixels::F32(whole) => whole.len(),
        }
    }

    /// Whether there are no samples at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl PixelsMut<'_> {
    /// The depth, as `lfx_pixel_format` spells it.
    #[must_use]
    pub const fn format(&self) -> LfxPixelFormat {
        match self {
            PixelsMut::F16(_) => LFX_RGBA_F16,
            PixelsMut::F32(_) => LFX_RGBA_F32,
        }
    }

    /// How many samples there are: four per pixel.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            PixelsMut::F16(half) => half.len(),
            PixelsMut::F32(whole) => whole.len(),
        }
    }

    /// Whether there are no samples at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// How many bytes one sample of this depth takes.
const fn sample_bytes(format: LfxPixelFormat) -> usize {
    if format == LFX_RGBA_F16 {
        size_of::<f16>()
    } else {
        size_of::<f32>()
    }
}

// ---------------------------------------------------------- the instance --

/// One live effect, in this process.
///
/// Dropping it destroys the instance, so it is dropped on the control thread -
/// the header's rule, and the one thing this type cannot enforce for the
/// caller.
pub struct LocalInstance<'host> {
    host: &'host LocalHost,
    plugin: *mut LfxPlugin,
    id: String,
    /// The kind tag of each element of the dense array, in order.
    elements: Vec<LfxParamKind>,
    /// What `lfx_process.cancelled` answers, reached from the C side through
    /// `host_context`. Boxed so its address outlives every move of this struct.
    cancelled: Box<AtomicBool>,
}

// SAFETY: `process` is the one thing this type does with the plugin pointer and
// the header says it may be called from any worker thread, on different
// instances of one plugin at once. It takes `&mut self`, so one instance cannot
// be re-entered. What crossing a thread boundary does **not** license is the
// lifecycle: `Drop` calls `destroy`, which the header puts on the control
// thread, so an instance moved to a worker and dropped there breaks that rule.
// This crate's own suite lends `&mut` across a scope and drops on the thread
// that created it.
unsafe impl Send for LocalInstance<'_> {}

impl LocalInstance<'_> {
    /// Which effect this is.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// How many elements the dense value array takes.
    #[must_use]
    pub fn value_count(&self) -> usize {
        self.elements.len()
    }

    /// Stop wanting this frame, or start again. What
    /// `lfx_process.cancelled` answers, which is worth a plugin's while only if
    /// it declared `LFX_TRAIT_CANCELLABLE`.
    pub fn set_cancelled(&self, stop: bool) {
        self.cancelled.store(stop, Ordering::SeqCst);
    }

    /// Render one frame.
    ///
    /// **Any worker thread.** `&mut self` is what makes "one instance is never
    /// re-entered" a fact about this type rather than a promise about its
    /// callers.
    ///
    /// # Errors
    ///
    /// The values are not the ones the effect declared, the two frames are at
    /// different depths or the wrong size, the region and the buffer do not
    /// describe one picture, the value stride is past
    /// [`MAX_VALUE_STRIDE`], or the plugin answered something other than
    /// `LFX_STATUS_OK` - in which case the caller renders the input unchanged
    /// and badges the layer.
    pub fn process(
        &mut self,
        request: &Request,
        values: &[Value],
        input: Pixels<'_>,
        output: PixelsMut<'_>,
    ) -> Result<(), LocalError> {
        if values.len() != self.elements.len() {
            return Err(LocalError::ValueCount {
                given: values.len(),
                declared: self.elements.len(),
            });
        }
        for (element, (value, declared)) in values.iter().zip(&self.elements).enumerate() {
            if value.tag() != *declared {
                return Err(LocalError::ValueKindMismatch {
                    element,
                    declared: *declared,
                    given: value.tag(),
                });
            }
        }
        let format = input.format();
        if format != output.format() {
            return Err(LocalError::DepthMismatch {
                input: format,
                output: output.format(),
            });
        }
        let wanted = (request.width as usize)
            .saturating_mul(request.height as usize)
            .saturating_mul(4);
        for (side, given) in [
            (FrameSide::Input, input.len()),
            (FrameSide::Output, output.len()),
        ] {
            if given != wanted {
                return Err(LocalError::FrameSize {
                    side,
                    width: request.width,
                    height: request.height,
                    wanted,
                    given,
                });
            }
        }
        request.regions_agree()?;
        let stride = request.stride()?;

        // The curves' points and the files' paths have to outlive the call, so
        // they are held here rather than inside the array they are pointed at
        // from.
        let curves: Vec<Vec<f32>> = values
            .iter()
            .map(|value| match value {
                Value::Curve(points) => points.iter().flat_map(|pair| [pair[0], pair[1]]).collect(),
                _ => Vec::new(),
            })
            .collect();
        // Backed by `LfxValue`s rather than by bytes, so the array's base
        // carries the element's own alignment: the stride is a multiple of it
        // (`Request::stride`), so every element lands aligned and the natural
        // C spelling - `(const lfx_value *)((const char *)values + i * stride)` -
        // is a read a strict-alignment target can make.
        //
        // The room between two elements at a widened stride is filled with an
        // element that is nobody's: a plugin striding by its own `size_of`
        // lands on it and sees a `param` that is not the index it asked for,
        // which is the stride agreement made observable rather than a silence.
        let unset = LfxValue {
            param: u32::MAX,
            kind: lumit_lfx_abi::LFX_PARAM_UNSET,
            v: LfxValuePayload { i: 0 },
        };
        let per_element = (stride as usize).div_ceil(size_of::<LfxValue>());
        let mut array: Vec<LfxValue> = vec![unset; per_element.saturating_mul(values.len())];
        let bytes = array.as_mut_ptr().cast::<u8>();
        for (index, value) in values.iter().enumerate() {
            let element = LfxValue {
                param: u32::try_from(index).unwrap_or(u32::MAX),
                kind: value.tag(),
                v: payload_of(value, curves.get(index).map_or(&[][..], Vec::as_slice)),
            };
            let offset = (stride as usize).saturating_mul(index);
            // SAFETY: the array is `stride × count` bytes and `stride` is at
            // least this struct's own size and a multiple of its alignment, so
            // one element fits at `offset` and lands aligned there.
            unsafe { bytes.add(offset).cast::<LfxValue>().write(element) };
        }

        let row_bytes = u32::try_from(
            (request.width as usize)
                .saturating_mul(4)
                .saturating_mul(sample_bytes(format)),
        )
        .unwrap_or(u32::MAX);
        let incoming = frame_for(request, format, row_bytes, pixels_ptr(&input));
        let mut outgoing = frame_for(request, format, row_bytes, pixels_mut_ptr(output));
        let call = LfxProcess {
            struct_size: size_of::<LfxProcess>() as u32,
            pixel_format: format,
            value_stride: stride,
            value_count: u32::try_from(values.len()).unwrap_or(u32::MAX),
            roi_x0: request.roi.0,
            roi_y0: request.roi.1,
            roi_x1: request.roi.2,
            roi_y1: request.roi.3,
            dod_x0: request.dod.0,
            dod_y0: request.dod.1,
            dod_x1: request.dod.2,
            dod_y1: request.dod.3,
            time: request.time,
            values: array.as_ptr(),
            input: &raw const incoming,
            output: &raw mut outgoing,
            cancelled: Some(host_cancelled),
            host_context: std::ptr::from_ref::<AtomicBool>(&*self.cancelled)
                .cast_mut()
                .cast::<c_void>(),
        };

        // SAFETY: a live plugin this struct owns, called once, with a request
        // whose values array and both frames live until it returns.
        let table = unsafe { &*self.plugin };
        let Some(process) = table.process else {
            return Err(LocalError::Process(lumit_lfx_abi::LFX_STATUS_UNSUPPORTED));
        };
        // SAFETY: as above.
        let status = unsafe { process(self.plugin, &raw const call) };
        // Both frames and the array are named here so that nothing they point
        // at is dropped before the call has returned: the header says neither
        // frame outlives the `process` call that carried it, and the other
        // half of that sentence is that both must live until it ends.
        let _ = (&incoming, &outgoing, &array, &curves);
        if status == LFX_STATUS_OK {
            Ok(())
        } else {
            Err(LocalError::Process(status))
        }
    }
}

impl Drop for LocalInstance<'_> {
    fn drop(&mut self) {
        // The last of the four lifecycle calls, and held to the same one
        // thread as the other three: `create` has long since given the lock
        // back, and nothing this crate does drops an instance from inside a
        // call that holds it.
        let _control = self
            .host
            .control
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // SAFETY: a live plugin this struct owns and nothing else has taken
        // back, with no process running: `process` takes `&mut self`.
        unsafe { destroy(self.plugin) };
    }
}

/// Whether the host has stopped wanting the frame this request is for.
///
/// # Safety
///
/// `request` must be one this module built, whose `host_context` points at the
/// instance's own flag.
unsafe extern "C" fn host_cancelled(request: *const LfxProcess) -> bool {
    if request.is_null() {
        return false;
    }
    // SAFETY: the caller's contract.
    let flag = unsafe { &*request }.host_context.cast::<AtomicBool>();
    if flag.is_null() {
        return false;
    }
    // SAFETY: as above: the box lives as long as the instance, which the header
    // says outlives the call.
    unsafe { &*flag }.load(Ordering::SeqCst)
}

/// One value's payload, in the arm its kind names.
fn payload_of(value: &Value, curve: &[f32]) -> LfxValuePayload {
    match value {
        Value::Float(number) | Value::Slider(number) | Value::Angle(number) => {
            LfxValuePayload { f: *number }
        }
        Value::Int(whole) | Value::Seed(whole) => LfxValuePayload { i: *whole },
        Value::Bool(switch) => LfxValuePayload { b: *switch },
        Value::Choice(chosen) => LfxValuePayload { choice: *chosen },
        Value::Colour(rgba) => LfxValuePayload { rgba: *rgba },
        Value::Point2(xy) => LfxValuePayload { xy: *xy },
        Value::Point3(xyz) => LfxValuePayload { xyz: *xyz },
        Value::Curve(points) => LfxValuePayload {
            curve: LfxCurveValue {
                pt: curve.as_ptr(),
                n: u32::try_from(points.len()).unwrap_or(u32::MAX),
            },
        },
        Value::File(path) => LfxValuePayload {
            file: LfxFileValue {
                path: path.as_ref().map_or(std::ptr::null(), |path| path.as_ptr()),
            },
        },
    }
}

/// One frame of the request's own size, pointing at `data`.
///
/// The origin is **the buffer's own top-left corner**, which the header says
/// it is - the definition's, not the region's. Writing the region asked for
/// there would tell a plugin its first pixel sits where the request's ROI
/// begins while handing it a buffer that starts somewhere else, and a plugin
/// locating that pixel as `roi_x0 - origin_x` would find nought and draw the
/// region in the buffer's corner. `Request::regions_agree` is what holds the
/// two to one rectangle.
fn frame_for(
    request: &Request,
    format: LfxPixelFormat,
    row_bytes: u32,
    data: *mut c_void,
) -> lumit_lfx_abi::LfxFrame {
    lumit_lfx_abi::LfxFrame {
        struct_size: size_of::<lumit_lfx_abi::LfxFrame>() as u32,
        format,
        width: request.width,
        height: request.height,
        row_bytes,
        origin_x: request.dod.0,
        origin_y: request.dod.1,
        reserved_0: 0,
        data,
        time: request.time,
    }
}

/// The input's bytes.
///
/// The frozen `lfx_frame` carries one `void *` for a picture that may be read
/// or written, and the header is what says which: "writing through the input's
/// `data` is undefined however permissive the mapping happens to be". So the
/// cast is the ABI's shape rather than a licence, and a plugin that writes
/// through it has broken the contract it compiled against.
fn pixels_ptr(pixels: &Pixels<'_>) -> *mut c_void {
    match pixels {
        Pixels::F16(half) => half.as_ptr().cast_mut().cast::<c_void>(),
        Pixels::F32(whole) => whole.as_ptr().cast_mut().cast::<c_void>(),
    }
}

/// The output's bytes, which are the only ones a plugin may write.
fn pixels_mut_ptr(pixels: PixelsMut<'_>) -> *mut c_void {
    match pixels {
        PixelsMut::F16(half) => half.as_mut_ptr().cast::<c_void>(),
        PixelsMut::F32(whole) => whole.as_mut_ptr().cast::<c_void>(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{MutexGuard, OnceLock};

    use lumit_core::fx::{CostClass, FxCategory, MatteRole, ParamKind, Roi, Unit};
    use lumit_lfx_abi::{
        LFX_LOG_INFO, LFX_MAX_TEMPORAL_WINDOW, LFX_STATUS_CANCELLED, LFX_TRAIT_THREAD_UNSAFE,
        LFX_UNIT_RAW,
    };
    use lumit_lfx_testplug::{Personality, PERSONALITIES};

    use super::*;

    // ------------------------------------------------------------ fixture --

    /// Serialises every test that opens the fixture, because the probes it
    /// records into are one set of statics shared by the whole process - so two
    /// tests running at once would read each other's calls.
    fn fixture_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The test bundle's file name on this platform.
    fn cdylib_name() -> &'static str {
        if cfg!(target_os = "windows") {
            "lumit_lfx_testplug.dll"
        } else if cfg!(target_os = "macos") {
            "liblumit_lfx_testplug.dylib"
        } else {
            "liblumit_lfx_testplug.so"
        }
    }

    /// Where Cargo put a library it built, if it built it.
    fn built(name: &str) -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let mut dir = exe.parent()?;
        for _ in 0..3 {
            for candidate in [
                dir.join(name),
                dir.join("deps").join(name),
                dir.join("examples").join(name),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            dir = dir.parent()?;
        }
        None
    }

    /// What the suite keeps hold of for the whole of its life: the temporary
    /// folder, the path inside it, and one open handle on the module.
    struct Fixture {
        _root: tempfile::TempDir,
        path: PathBuf,
        /// Held so the module stays loaded between tests. The loader unloads a
        /// library when its last handle goes, and the probes are statics
        /// **inside** that library - so a test that dropped the host and then
        /// asked what the host had done would be asking a fresh, empty copy.
        _keep: libloading::Library,
    }

    /// The one `.lfx` this process loads, copied out once.
    ///
    /// **One path, for the whole process**: the operating system's loader hands
    /// back the same module for the same file, so the host's copy and the
    /// probes' own handle share the statics the log lives in. Two copies in two
    /// temporary folders would be two modules and two empty logs.
    fn fixture() -> Option<&'static Path> {
        static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
        FIXTURE
            .get_or_init(|| {
                let source = built(cdylib_name())?;
                let root = tempfile::tempdir().ok()?;
                let path = root.path().join("Lumit test.lfx");
                std::fs::copy(&source, &path).ok()?;
                // SAFETY: the file this suite has just written, loaded for the
                // one reason given above.
                let keep = unsafe { libloading::Library::new(&path) }.ok()?;
                Some(Fixture {
                    _root: root,
                    path,
                    _keep: keep,
                })
            })
            .as_ref()
            .map(|fixture| fixture.path.as_path())
    }

    /// Say why a test did nothing, by name, so a skip is never silent.
    fn skipped(test: &str) {
        eprintln!("{test}: the test bundle was not built, so nothing was checked");
    }

    /// The bundle, open, with its probes cleared.
    fn opened() -> Option<LocalHost> {
        let host = LocalHost::open(fixture()?).ok()?;
        probe_reset();
        Some(host)
    }

    /// One of the bundles that is wrong at its own front door, and an open
    /// handle on it.
    ///
    /// Held for the reason [`Fixture`]'s handle is: the counters are statics
    /// **inside** that module, and the loader unloads a library when its last
    /// handle goes - so a test that watched the host refuse the bundle and then
    /// asked what the host had called would be asking a fresh, empty copy.
    struct Held {
        path: PathBuf,
        library: libloading::Library,
    }

    /// The library at `name`, loaded once and kept.
    fn held(name: &str) -> Option<Held> {
        let path = built(name)?;
        // SAFETY: a library this workspace built, loaded for the one reason
        // given above.
        let library = unsafe { libloading::Library::new(&path) }.ok()?;
        Some(Held { path, library })
    }

    /// The one lying bundle this process loads, opened once.
    fn liar() -> Option<&'static Held> {
        static LIAR: OnceLock<Option<Held>> = OnceLock::new();
        LIAR.get_or_init(|| {
            held(if cfg!(target_os = "windows") {
                "an_lfx_entry_that_lies.dll"
            } else if cfg!(target_os = "macos") {
                "liban_lfx_entry_that_lies.dylib"
            } else {
                "liban_lfx_entry_that_lies.so"
            })
        })
        .as_ref()
    }

    /// The bundle whose entry table really is short, opened once.
    ///
    /// The liar's static is the whole of an `lfx_entry` and only says
    /// otherwise, so a host that read it through a reference would be reading
    /// its own fixture's bytes; this one's static stops where its prefix says
    /// it does, which is what makes the rule a thing a sanitiser can see.
    fn cut_short() -> Option<&'static Held> {
        static CUT_SHORT: OnceLock<Option<Held>> = OnceLock::new();
        CUT_SHORT
            .get_or_init(|| {
                held(if cfg!(target_os = "windows") {
                    "an_lfx_entry_cut_short.dll"
                } else if cfg!(target_os = "macos") {
                    "liban_lfx_entry_cut_short.dylib"
                } else {
                    "liban_lfx_entry_cut_short.so"
                })
            })
            .as_ref()
    }

    /// Tell the liar what to say about itself the next time a host opens it,
    /// and forget what it has been asked so far.
    ///
    /// Every test below calls this before it opens anything, so the shape one
    /// leaves behind is never what the next one reads.
    fn liar_shape(struct_size: u32, abi_version: u32, init_answers: u32, init_present: u32) {
        let Some(liar) = liar() else {
            return;
        };
        // SAFETY: the export's signature is the one the fixture declares.
        let Ok(reset) = (unsafe {
            liar.library
                .get::<unsafe extern "C" fn()>(b"LumitLfxLiarReset\0")
        }) else {
            return;
        };
        // SAFETY: the fixture's own export, which takes nothing.
        unsafe { reset() };
        // SAFETY: as above.
        let Ok(shape) = (unsafe {
            liar.library
                .get::<unsafe extern "C" fn(u32, u32, u32, u32)>(b"LumitLfxLiarShape\0")
        }) else {
            return;
        };
        // SAFETY: the fixture's own export, whose four arguments are plain
        // integers.
        unsafe { shape(struct_size, abi_version, init_answers, init_present) };
    }

    /// Read one of a held bundle's counting probes.
    fn held_count(bundle: &Held, symbol: &[u8]) -> u32 {
        // SAFETY: the export's signature is the one the fixture declares.
        let Ok(read) = (unsafe { bundle.library.get::<unsafe extern "C" fn() -> u32>(symbol) })
        else {
            return 0;
        };
        // SAFETY: the fixture's own export, which takes nothing.
        unsafe { read() }
    }

    /// Read one of the liar's counting probes.
    fn liar_count(symbol: &[u8]) -> u32 {
        let Some(liar) = liar() else {
            return 0;
        };
        held_count(liar, symbol)
    }

    /// The honest number of bytes an entry table carries.
    fn honest_entry_bytes() -> u32 {
        u32::try_from(size_of::<LfxEntry>()).unwrap_or(u32::MAX)
    }

    /// One of the personalities, by name.
    fn id(personality: Personality) -> String {
        personality.id().to_string_lossy().into_owned()
    }

    /// Read one of the fixture's own exports, out of the copy the host loaded.
    fn read_probe(symbol: &[u8]) -> Vec<String> {
        let Some(path) = fixture() else {
            return Vec::new();
        };
        // SAFETY: the same file the host loaded, so the loader answers with the
        // same module, and the symbol is one this crate's own fixture exports.
        let Ok(library) = (unsafe { libloading::Library::new(path) }) else {
            return Vec::new();
        };
        // SAFETY: the export's signature is the one the fixture declares.
        let Ok(read) =
            (unsafe { library.get::<unsafe extern "C" fn(*mut c_char, u32) -> u32>(symbol) })
        else {
            return Vec::new();
        };
        let mut buffer = vec![0 as c_char; 64 * 1024];
        let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: the buffer is writable for its own length.
        let written = unsafe { read(buffer.as_mut_ptr(), capacity) } as usize;
        if written == 0 || written >= buffer.len() {
            return Vec::new();
        }
        let bytes: Vec<u8> = buffer[..written].iter().map(|byte| *byte as u8).collect();
        String::from_utf8_lossy(&bytes)
            .split(',')
            .map(str::to_owned)
            .collect()
    }

    /// Read one of the fixture's counting probes.
    fn read_count(symbol: &[u8]) -> u32 {
        let Some(path) = fixture() else {
            return 0;
        };
        // SAFETY: as `read_probe`.
        let Ok(library) = (unsafe { libloading::Library::new(path) }) else {
            return 0;
        };
        // SAFETY: as `read_probe`.
        let Ok(read) = (unsafe { library.get::<unsafe extern "C" fn() -> u32>(symbol) }) else {
            return 0;
        };
        // SAFETY: the fixture's own export, which takes nothing.
        unsafe { read() }
    }

    /// Call one of the fixture's setting probes.
    fn set_count(symbol: &[u8], count: u32) {
        let Some(path) = fixture() else {
            return;
        };
        // SAFETY: as `read_probe`.
        let Ok(library) = (unsafe { libloading::Library::new(path) }) else {
            return;
        };
        // SAFETY: as `read_probe`.
        let Ok(write) = (unsafe { library.get::<unsafe extern "C" fn(u32)>(symbol) }) else {
            return;
        };
        // SAFETY: the fixture's own export.
        unsafe { write(count) };
    }

    /// Every call the host has made since the last reset.
    fn probe_log() -> Vec<String> {
        read_probe(b"LumitLfxProbeLog\0")
    }

    /// Every depth the host has handed over since the last reset.
    fn probe_depths() -> Vec<String> {
        read_probe(b"LumitLfxProbeDepths\0")
    }

    /// Forget the logs and the high-water marks.
    fn probe_reset() {
        let Some(path) = fixture() else {
            return;
        };
        // SAFETY: as `read_probe`.
        let Ok(library) = (unsafe { libloading::Library::new(path) }) else {
            return;
        };
        // SAFETY: as `read_probe`.
        let Ok(reset) = (unsafe { library.get::<unsafe extern "C" fn()>(b"LumitLfxProbeReset\0") })
        else {
            return;
        };
        // SAFETY: the fixture's own export, which takes nothing.
        unsafe { reset() };
    }

    /// A picture of `width × height`, filled with one value.
    fn picture(width: u32, height: u32, value: f32) -> Vec<f32> {
        vec![value; (width as usize) * (height as usize) * 4]
    }

    // -------------------------------------------------------- the module --

    /// The descriptor list is the fixture's twelve personalities, in the
    /// entry's own order, with the identity each of them declares.
    #[test]
    fn a_module_declares_the_effects_it_holds() {
        let test = "a_module_declares_the_effects_it_holds";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let declared: Vec<&str> = host
            .plugins()
            .iter()
            .map(|plugin| plugin.id.as_str())
            .collect();
        let wanted: Vec<String> = PERSONALITIES.iter().copied().map(id).collect();
        assert_eq!(
            declared,
            wanted.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(host.report().is_empty(), "{:?}", host.report());

        let full = host.plugins().first().expect("the first personality");
        assert_eq!(full.name, "Test full");
        assert_eq!(full.vendor, "The Lumit authors");
        assert_eq!((full.major, full.minor, full.patch), (2, 3, 4));
        assert_eq!(full.categories.len(), 2);
    }

    /// A text file is not a plugin, and neither is a path with nothing at it.
    /// Both are refused rather than called.
    #[test]
    fn a_file_that_is_not_a_bundle_is_refused_rather_than_called() {
        let Ok(root) = tempfile::tempdir() else {
            return;
        };
        let path = root.path().join("nothing.lfx");
        assert!(std::fs::write(&path, b"not a library").is_ok());
        assert!(matches!(
            LocalHost::open(&path),
            Err(LocalError::NotLoaded(_))
        ));
        assert!(matches!(
            LocalHost::open(&root.path().join("absent.lfx")),
            Err(LocalError::NotLoaded(_))
        ));
    }

    /// A library that loads perfectly well and exports no `lfx_entry_point` is
    /// not an LFX bundle however it is named, and the refusal says which of the
    /// two it is.
    #[test]
    fn a_library_with_no_entry_point_is_refused() {
        let test = "a_library_with_no_entry_point_is_refused";
        let name = if cfg!(target_os = "windows") {
            "not_an_lfx_bundle.dll"
        } else if cfg!(target_os = "macos") {
            "libnot_an_lfx_bundle.dylib"
        } else {
            "libnot_an_lfx_bundle.so"
        };
        let Some(library) = built(name) else {
            skipped(test);
            return;
        };
        assert!(matches!(
            LocalHost::open(&library),
            Err(LocalError::NoEntry)
        ));
    }

    /// An entry whose size prefix is shorter than this header's names fields
    /// that are not there, so it is refused - and refused **before it is
    /// called**. The growth mechanism reads upwards and cannot read downwards,
    /// and a function pointer that is not there is uncallable, so `init` is
    /// never reached (docs/impl/lfx.md §2.1, §10).
    ///
    /// Two bundles, because the rule has two halves. The liar's static is the
    /// whole of an `lfx_entry` and lies in its prefix, which is what pins the
    /// refusal and the number it carries; `an_lfx_entry_cut_short`'s static
    /// really stops after two words and a hook, which is what pins **the prefix
    /// being read before a reference is formed** - under Miri or a sanitiser a
    /// host that read it the other way round is caught here and nowhere else.
    #[test]
    fn an_entry_shorter_than_the_header_is_refused_before_it_is_called() {
        let test = "an_entry_shorter_than_the_header_is_refused_before_it_is_called";
        let _guard = fixture_lock();
        let Some(liar) = liar() else {
            skipped(test);
            return;
        };
        // One function pointer short, which is what a bundle built against a
        // version of this table with one fewer hook would carry.
        let short =
            honest_entry_bytes().saturating_sub(u32::try_from(size_of::<usize>()).unwrap_or(8));
        liar_shape(short, LFX_ABI_VERSION, 1, 1);
        assert!(matches!(
            LocalHost::open(&liar.path),
            Err(LocalError::ShortEntry { bytes }) if bytes == short
        ));
        assert_eq!(
            liar_count(b"LumitLfxLiarInits\0"),
            0,
            "the prefix is read before the table it prefixes is"
        );
        assert_eq!(
            liar_count(b"LumitLfxLiarCounts\0"),
            0,
            "and nothing behind it is called either"
        );

        let Some(cut) = cut_short() else {
            skipped(test);
            return;
        };
        let bytes = held_count(cut, b"LumitLfxCutShortBytes\0");
        assert!(
            (bytes as usize) < size_of::<LfxEntry>(),
            "the cut-short bundle carries a whole table, so it pins nothing"
        );
        assert!(matches!(
            LocalHost::open(&cut.path),
            Err(LocalError::ShortEntry { bytes: said }) if said == bytes
        ));
        assert_eq!(
            held_count(cut, b"LumitLfxCutShortInits\0"),
            0,
            "and the hook past the end of that object was never read, let alone called"
        );
    }

    /// A bundle that exports an entry point whose `init` is null is refused by
    /// a name of its own, because the symbol is right there and a sentence
    /// saying the bundle has no entry point would send a vendor looking for the
    /// one thing they did do.
    #[test]
    fn an_entry_whose_init_is_null_is_refused_by_that_name() {
        let test = "an_entry_whose_init_is_null_is_refused_by_that_name";
        let _guard = fixture_lock();
        let Some(liar) = liar() else {
            skipped(test);
            return;
        };
        liar_shape(honest_entry_bytes(), LFX_ABI_VERSION, 1, 0);
        assert!(matches!(
            LocalHost::open(&liar.path),
            Err(LocalError::NoInit)
        ));
        assert_eq!(
            liar_count(b"LumitLfxLiarCounts\0"),
            0,
            "and nothing else in the bundle is called: there was no start to it"
        );
        assert_eq!(liar_count(b"LumitLfxLiarDeinits\0"), 0);

        // And it is not the answer to a bundle that exports nothing, which is a
        // different file and a different sentence.
        liar_shape(honest_entry_bytes(), LFX_ABI_VERSION, 1, 1);
        LocalHost::open(&liar.path).expect("the honest hook");
    }

    /// One integer, and it intends to reach 2 never, so version 1 admits
    /// exactly one number - refused before anything in the bundle is called.
    #[test]
    fn a_module_declaring_another_abi_is_refused_before_it_is_called() {
        let test = "a_module_declaring_another_abi_is_refused_before_it_is_called";
        let _guard = fixture_lock();
        let Some(liar) = liar() else {
            skipped(test);
            return;
        };
        let declared = LFX_ABI_VERSION.saturating_add(1);
        liar_shape(honest_entry_bytes(), declared, 1, 1);
        assert!(matches!(
            LocalHost::open(&liar.path),
            Err(LocalError::AbiUnsupported { declared: said }) if said == declared
        ));
        assert_eq!(
            liar_count(b"LumitLfxLiarInits\0"),
            0,
            "the version is read before the bundle is started"
        );
        assert_eq!(
            liar_count(b"LumitLfxLiarCounts\0"),
            0,
            "and nothing behind it is called either"
        );
        assert_eq!(liar_count(b"LumitLfxLiarDeinits\0"), 0);
    }

    /// A bundle whose `init` declines is refused by that name, and **nothing
    /// else in it is called**: the header says the host says so without opening
    /// anything else, and a `deinit` pairs with an `init` that succeeded.
    #[test]
    fn a_bundle_that_declines_to_load_is_not_then_asked_what_it_holds() {
        let test = "a_bundle_that_declines_to_load_is_not_then_asked_what_it_holds";
        let _guard = fixture_lock();
        let Some(liar) = liar() else {
            skipped(test);
            return;
        };
        liar_shape(honest_entry_bytes(), LFX_ABI_VERSION, 0, 1);
        assert!(matches!(
            LocalHost::open(&liar.path),
            Err(LocalError::InitRefused)
        ));
        assert_eq!(liar_count(b"LumitLfxLiarInits\0"), 1, "asked once");
        assert_eq!(
            liar_count(b"LumitLfxLiarCounts\0"),
            0,
            "and never asked what it holds"
        );
        assert_eq!(
            liar_count(b"LumitLfxLiarDeinits\0"),
            0,
            "nor told to stop, having never started"
        );
    }

    /// The answers a bundle gives cross as `uint32_t` in which **non-zero is
    /// true**, never as a C `bool`: an `init` answering two has loaded. The
    /// host reads the byte a stranger's compiler left in the return register
    /// rather than trusting it to be one of two (docs/impl/lfx.md §2.1).
    #[test]
    fn an_init_that_answers_anything_but_nought_has_loaded() {
        let test = "an_init_that_answers_anything_but_nought_has_loaded";
        let _guard = fixture_lock();
        let Some(liar) = liar() else {
            skipped(test);
            return;
        };
        liar_shape(honest_entry_bytes(), LFX_ABI_VERSION, 2, 1);
        let Ok(host) = LocalHost::open(&liar.path) else {
            panic!("two is not nought, so the bundle loaded");
        };
        assert!(
            host.plugins().is_empty(),
            "a bundle that holds nothing is a bundle, not a refusal"
        );
        assert_eq!(liar_count(b"LumitLfxLiarCounts\0"), 1, "and it was asked");
        drop(host);
        assert_eq!(
            liar_count(b"LumitLfxLiarDeinits\0"),
            1,
            "and told to stop when the host was done with it"
        );
    }

    /// `init` is handed the bundle's own directory, so a plugin can find its
    /// resources without guessing - not the module's file name.
    #[test]
    fn the_bundles_own_directory_is_what_init_is_handed() {
        let test = "the_bundles_own_directory_is_what_init_is_handed";
        let _guard = fixture_lock();
        let Some(path) = fixture() else {
            skipped(test);
            return;
        };
        let Ok(host) = LocalHost::open(path) else {
            skipped(test);
            return;
        };
        let handed = read_probe(b"LumitLfxProbeBundlePath\0").join(",");
        assert_eq!(
            handed,
            path.parent().expect("a parent").to_string_lossy(),
            "the bundle's directory, never its file"
        );
        drop(host);
    }

    /// The effect a bundle has not got is refused by name, and nothing is
    /// created.
    #[test]
    fn an_effect_the_bundle_has_not_got_is_refused_by_name() {
        let test = "an_effect_the_bundle_has_not_got_is_refused_by_name";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        assert!(matches!(
            host.describe("org.lumit.testplug.absent"),
            Err(LocalError::NoSuchPlugin { .. })
        ));
        assert!(!probe_log().iter().any(|call| call.starts_with("create:")));
    }

    // ------------------------------------------------------ the describe --

    /// A described plugin becomes the same `EffectSchema` a built-in carries:
    /// the rows in declaration order, the point spread into axes the panel
    /// folds back, the heading over the run, and the identity the descriptor
    /// declared.
    #[test]
    fn a_described_plugin_becomes_the_schema_a_builtin_carries() {
        let test = "a_described_plugin_becomes_the_schema_a_builtin_carries";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Full))
            .expect("the full personality describes itself");
        let declared: Vec<&str> = described
            .params
            .iter()
            .map(|param| param.id.as_str())
            .collect();
        assert_eq!(
            declared,
            [
                "gain", "mix", "steps", "tilt", "invert", "mode", "tint", "seed", "centre",
                "pivot", "shape", "table", "reset",
            ]
        );

        let schema = schema::schema_of(&described).expect("the lowering");
        let rows: Vec<&str> = schema.params.iter().map(|row| row.id).collect();
        assert_eq!(
            rows,
            [
                "gain", "mix", "steps", "tilt", "invert", "mode", "tint", "seed", "centre_x",
                "centre_y", "pivot_x", "pivot_y", "pivot_z", "shape", "table", "reset",
            ]
        );
        assert_eq!(schema.match_name, "lfx:org.lumit.testplug.full");
        assert_eq!(schema.label, "Test full");
        // Every one of the three declared numbers moves the key.
        assert_eq!(schema.version, 2 * 1_000_000 + 3 * 1_000 + 4);
        // The **first** declared family is the heading; the rest are keywords.
        assert_eq!(schema.category, FxCategory::Colour);
        assert_eq!(
            schema::families(&described),
            [FxCategory::Colour, FxCategory::Stylise]
        );
        assert_eq!(schema.traits.cost, CostClass::Cheap);
        assert_eq!(schema.traits.roi, Roi::PaddedPx(8.0));
        assert!(schema.traits.seeded);
        assert!(schema.traits.premultiplied);
        assert_eq!(schema.matte, MatteRole::None);
        assert_eq!(schema.groups.len(), 1);
        assert_eq!(schema.groups[0].label, "Basics");
        assert_eq!(schema.groups[0].params.len(), rows.len());
        // The panel's own question, asked of the schema rather than of the ids.
        assert!(schema
            .pairs()
            .any(|pair| pair.x == "centre_x" && pair.y == "centre_y"));
        // A unit is what the plugin declared, and a kind that carries none is
        // normalised.
        let unit_of = |wanted: &str| {
            schema
                .params
                .iter()
                .find(|row| row.id == wanted)
                .map(|row| row.unit)
        };
        assert_eq!(unit_of("mix"), Some(Unit::Percent));
        assert_eq!(unit_of("centre_x"), Some(Unit::Px));
        assert_eq!(unit_of("tilt"), Some(Unit::Degrees));
    }

    /// A row this build cannot draw is a line in the scan report and the plugin
    /// still loads, keeping that row's declared default for the effect's life.
    #[test]
    fn a_row_this_build_cannot_draw_is_a_report_line_and_the_plugin_still_loads() {
        let test = "a_row_this_build_cannot_draw_is_a_report_line_and_the_plugin_still_loads";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Full))
            .expect("the effect still loads");
        assert!(
            !described.params.iter().any(|param| param.id == "empty"),
            "a dropdown with nothing in it is not drawn"
        );
        let lines = schema::notes(&described);
        assert!(
            lines.iter().any(
                |line| matches!(line, LfxRejection::ChoiceWithNoOptions { id } if id == "empty")
            ),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|line| !line.refuses_the_effect()),
            "a report line is never the end of the effect"
        );
    }

    /// A plugin that simply declines to say what it is is refused by name, and
    /// the refusal is not confused with a fault in what it declared.
    #[test]
    fn a_plugin_that_refuses_to_describe_is_refused_by_name() {
        let test = "a_plugin_that_refuses_to_describe_is_refused_by_name";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        assert!(matches!(
            host.describe(&id(Personality::BrokenDescribe)),
            Err(LocalError::DescribeRefused { .. })
        ));
        // It was created and taken down again all the same: a refusal to
        // describe is not a licence to leak an instance.
        let log = probe_log();
        assert!(log
            .iter()
            .any(|call| call.ends_with("broken-describe") && call.starts_with("destroy:")));
    }

    /// Two controls on one id is one of them silently driving the other, so it
    /// refuses the whole effect - answered on the call that made the duplicate.
    #[test]
    fn two_controls_on_one_id_refuse_the_effect_at_the_sink() {
        let test = "two_controls_on_one_id_refuse_the_effect_at_the_sink";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let refused = host.describe(&id(Personality::DuplicateIds));
        assert!(
            matches!(
                refused,
                Err(LocalError::Refused(LfxRejection::DuplicateParamId { .. }))
            ),
            "{refused:?}"
        );
    }

    /// The pessimistic case, from the other end: a `NULL` trait pointer is the
    /// same declaration as a zeroed block, and both schedule as heavy with a
    /// full-frame region.
    #[test]
    fn a_null_trait_block_schedules_as_heavy_and_full_frame() {
        let test = "a_null_trait_block_schedules_as_heavy_and_full_frame";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Slim))
            .expect("the slim personality describes itself");
        assert!(
            described.identity.traits.is_none(),
            "a null pointer is the pessimistic case and means it"
        );
        let schema = schema::schema_of(&described).expect("the lowering");
        assert_eq!(schema.traits.cost, CostClass::Heavy);
        assert_eq!(schema.traits.roi, Roi::FullFrame);
        assert_eq!(schema.traits.temporal, &[0]);
        assert!(schema.traits.premultiplied);
        assert!(!schema.traits.seeded);
    }

    /// The declared window is the gate: it reaches the schema as the
    /// neighbours the effect asks for, and it needs no extension to do it -
    /// the one this version names is not offered, and the plugin is told so
    /// with a null rather than a status.
    #[test]
    fn a_declared_window_reaches_the_schema_as_the_neighbours_it_asks_for() {
        let test = "a_declared_window_reaches_the_schema_as_the_neighbours_it_asks_for";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Temporal))
            .expect("the temporal personality describes itself");
        let schema = schema::schema_of(&described).expect("the lowering");
        assert_eq!(schema.traits.temporal, &[-1, 0, 1]);
        assert!(OFFERED_EXTENSIONS.is_empty());
        assert_eq!(host.extensions_asked(), ["lfx.temporal"]);
        assert!(probe_log()
            .iter()
            .any(|call| call == "host-extension:lfx.temporal=none"));
        // And the window a plugin may declare is held to the header's own
        // ceiling rather than to whatever it wrote.
        assert!(i32::abs(schema.traits.temporal[0]) <= LFX_MAX_TEMPORAL_WINDOW);
    }

    /// The sole, discouraged opt-out survives into the descriptor as the flag
    /// it is. `EffectTraits` carries no field for it - the pool it pins is a
    /// later package's - so the declaration has to be readable where it landed.
    #[test]
    fn a_thread_unsafe_declaration_survives_into_the_descriptor() {
        let test = "a_thread_unsafe_declaration_survives_into_the_descriptor";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::ThreadUnsafe))
            .expect("the thread-unsafe personality describes itself");
        let traits = described.identity.traits.expect("a declared trait block");
        assert!(traits.flags & LFX_TRAIT_THREAD_UNSAFE != 0);
        assert!(schema::schema_of(&described).is_ok());
    }

    // ------------------------------------------------------- the process --

    /// A value the host wrote reaches `process` and changes the picture.
    #[test]
    fn a_plugin_reads_the_values_the_host_wrote() {
        let test = "a_plugin_reads_the_values_the_host_wrote";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Full)).expect("describe");
        let mut instance = host.create(&described).expect("create");
        assert_eq!(instance.value_count(), full_values().len());

        let input = picture(4, 3, 1.0);
        let mut output = picture(4, 3, 0.0);
        instance
            .process(
                &Request::full_frame(0.0, 4, 3),
                &full_values(),
                Pixels::F32(&input),
                PixelsMut::F32(&mut output),
            )
            .expect("the frame");
        assert!(
            output
                .iter()
                .all(|sample| (*sample - 0.5).abs() < f32::EPSILON),
            "the gain the host wrote is the gain the plugin applied"
        );
    }

    /// Both depths cross the boundary and neither is converted to accommodate
    /// the plugin: an fp16 frame is handed over as halves.
    #[test]
    fn both_depths_cross_the_boundary_unconverted() {
        let test = "both_depths_cross_the_boundary_unconverted";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Full)).expect("describe");
        let mut instance = host.create(&described).expect("create");
        probe_reset();

        let whole = picture(2, 2, 1.0);
        let mut whole_out = picture(2, 2, 0.0);
        instance
            .process(
                &Request::full_frame(0.0, 2, 2),
                &full_values(),
                Pixels::F32(&whole),
                PixelsMut::F32(&mut whole_out),
            )
            .expect("the fp32 frame");

        let halves = vec![f16::from_f32(1.0); 2 * 2 * 4];
        let mut halves_out = vec![f16::ZERO; 2 * 2 * 4];
        instance
            .process(
                &Request::full_frame(1.0, 2, 2),
                &full_values(),
                Pixels::F16(&halves),
                PixelsMut::F16(&mut halves_out),
            )
            .expect("the fp16 frame");

        assert_eq!(probe_depths(), ["f32", "f16"]);
        assert!(halves_out
            .iter()
            .all(|sample| sample.to_bits() == f16::from_f32(0.5).to_bits()));

        // And the host converts neither to accommodate the other.
        let mismatched = instance.process(
            &Request::full_frame(2.0, 2, 2),
            &full_values(),
            Pixels::F32(&whole),
            PixelsMut::F16(&mut halves_out),
        );
        assert!(matches!(mismatched, Err(LocalError::DepthMismatch { .. })));
    }

    /// The dense array is walked by the stride the host wrote, never by the
    /// plugin's own `size_of` - which is the whole reason `lfx_value` is the
    /// one struct with no size prefix.
    #[test]
    fn a_plugin_walks_the_value_array_by_the_stride_the_host_wrote() {
        let test = "a_plugin_walks_the_value_array_by_the_stride_the_host_wrote";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Full)).expect("describe");
        let mut instance = host.create(&described).expect("create");

        let mut request = Request::full_frame(0.0, 2, 2);
        request.value_stride = Some((size_of::<LfxValue>() * 2) as u32);
        let input = picture(2, 2, 1.0);
        let mut output = picture(2, 2, 0.0);
        instance
            .process(
                &request,
                &full_values(),
                Pixels::F32(&input),
                PixelsMut::F32(&mut output),
            )
            .expect("a padded stride is still an array");
        assert!(output
            .iter()
            .all(|sample| (*sample - 0.5).abs() < f32::EPSILON));
    }

    /// A value offered through an arm its declaration does not name is refused
    /// before the plugin sees it, rather than handed over as a kind it would
    /// read through the wrong half of a union.
    #[test]
    fn a_value_of_the_wrong_kind_never_reaches_the_plugin() {
        let test = "a_value_of_the_wrong_kind_never_reaches_the_plugin";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Full)).expect("describe");
        let mut instance = host.create(&described).expect("create");
        probe_reset();

        let mut values = full_values();
        values[0] = Value::Int(3);
        let input = picture(2, 2, 1.0);
        let mut output = picture(2, 2, 0.25);
        let refused = instance.process(
            &Request::full_frame(0.0, 2, 2),
            &values,
            Pixels::F32(&input),
            PixelsMut::F32(&mut output),
        );
        assert!(
            matches!(
                refused,
                Err(LocalError::ValueKindMismatch { element: 0, .. })
            ),
            "{refused:?}"
        );
        assert!(!probe_log().iter().any(|call| call.starts_with("process:")));
        assert!(output
            .iter()
            .all(|sample| (*sample - 0.25).abs() < f32::EPSILON));

        // A short list is the other half of the same question.
        assert!(matches!(
            instance.process(
                &Request::full_frame(0.0, 2, 2),
                &values[..3],
                Pixels::F32(&input),
                PixelsMut::F32(&mut output),
            ),
            Err(LocalError::ValueCount { .. })
        ));
    }

    /// A buffer that is not the size the request names never reaches the
    /// plugin, in either direction.
    ///
    /// The frame the plugin is handed carries the request's own `width`,
    /// `height` and `row_bytes`, so a buffer shorter than they describe is an
    /// out-of-bounds write by a plugin that did exactly what it was told - the
    /// one fault at this edge the plugin cannot be blamed for. It is the
    /// host's own caller being held to its word, which is why it is checked
    /// before a pointer is taken out of either half.
    #[test]
    fn a_buffer_that_is_not_the_size_the_request_names_never_reaches_the_plugin() {
        let test = "a_buffer_that_is_not_the_size_the_request_names_never_reaches_the_plugin";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Passthrough))
            .expect("describe");
        let mut instance = host.create(&described).expect("create");
        probe_reset();

        // The one control a simple personality declares; a short list would be
        // refused before the buffers were looked at, which is a different
        // question with a test of its own.
        let values = [Value::Float(1.0)];
        let whole = picture(2, 2, 1.0);
        let short = picture(2, 1, 1.0);
        let request = Request::full_frame(0.0, 2, 2);

        // The input short, the output whole. The refusal names the side, so
        // that a host checking the input twice and the output never is a
        // failure here rather than a pass that the call never reached the
        // plugin.
        let mut output = picture(2, 2, 0.25);
        assert!(matches!(
            instance.process(
                &request,
                &values,
                Pixels::F32(&short),
                PixelsMut::F32(&mut output),
            ),
            Err(LocalError::FrameSize { side: FrameSide::Input, given, wanted, .. })
                if given == short.len() && wanted == whole.len()
        ));

        // The output short, the input whole.
        let mut output = picture(2, 1, 0.25);
        assert!(matches!(
            instance.process(
                &request,
                &values,
                Pixels::F32(&whole),
                PixelsMut::F32(&mut output),
            ),
            Err(LocalError::FrameSize { side: FrameSide::Output, given, .. })
                if given == short.len()
        ));

        assert!(
            !probe_log().iter().any(|call| call.starts_with("process:")),
            "neither reached the plugin"
        );
    }

    /// An effect with nothing to do writes nothing at all, and the output is
    /// what it was byte for byte - not the input copied back, which would put
    /// the picture through the depth boundary and change it very slightly.
    #[test]
    fn a_plugin_that_writes_nothing_leaves_the_output_as_it_found_it() {
        let test = "a_plugin_that_writes_nothing_leaves_the_output_as_it_found_it";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Identity)).expect("describe");
        let mut instance = host.create(&described).expect("create");

        let input = vec![f16::from_f32(1.0); 2 * 2 * 4];
        let mut output = vec![f16::from_f32(0.25); 2 * 2 * 4];
        let before: Vec<u16> = output.iter().map(|half| half.to_bits()).collect();
        instance
            .process(
                &Request::full_frame(0.0, 2, 2),
                &[Value::Float(1.0)],
                Pixels::F16(&input),
                PixelsMut::F16(&mut output),
            )
            .expect("identity is a success, not a failure");
        let after: Vec<u16> = output.iter().map(|half| half.to_bits()).collect();
        assert_eq!(before, after, "byte for byte");
    }

    /// A plugin that says the host has stopped wanting the frame answers
    /// promptly, and the answer is a status rather than a picture.
    #[test]
    fn a_cancelled_frame_is_answered_rather_than_rendered() {
        let test = "a_cancelled_frame_is_answered_rather_than_rendered";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Full)).expect("describe");
        let mut instance = host.create(&described).expect("create");
        instance.set_cancelled(true);

        let input = picture(2, 2, 1.0);
        let mut output = picture(2, 2, 0.25);
        let answered = instance.process(
            &Request::full_frame(0.0, 2, 2),
            &full_values(),
            Pixels::F32(&input),
            PixelsMut::F32(&mut output),
        );
        assert!(
            matches!(answered, Err(LocalError::Process(LFX_STATUS_CANCELLED))),
            "{answered:?}"
        );
        assert!(output
            .iter()
            .all(|sample| (*sample - 0.25).abs() < f32::EPSILON));
    }

    /// An effect that cannot work without an extension this host has not got is
    /// refused **before** it is instantiated, with the extension named, rather
    /// than left to fail somewhere later.
    #[test]
    fn an_effect_requiring_an_extension_the_host_has_not_got_is_refused_before_create() {
        let test = "an_effect_requiring_an_extension_the_host_has_not_got_is_refused_before_create";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let refused = host.describe(&id(Personality::MissingExtension));
        match refused {
            Err(LocalError::RequiresExtension { extension, .. }) => {
                assert_eq!(extension, "lfx.gpu-frames");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            !probe_log().iter().any(|call| call.starts_with("create:")),
            "nothing of the plugin's ran"
        );
    }

    /// The order the header pins, read back from the plugin's own side: the
    /// bundle starts, the list is read, an instance is made, initialised,
    /// described and taken down, and the bundle stops last.
    #[test]
    fn the_call_order_is_the_one_the_header_pins() {
        let test = "the_call_order_is_the_one_the_header_pins";
        let _guard = fixture_lock();
        let Some(path) = fixture() else {
            skipped(test);
            return;
        };
        probe_reset();
        let host = LocalHost::open(path).expect("the bundle opens");
        let described = host.describe(&id(Personality::Slim)).expect("describe");
        let instance = host.create(&described).expect("create");
        drop(instance);
        drop(host);

        let log = probe_log();
        let shape: Vec<&str> = log
            .iter()
            .map(|call| call.split(':').next().unwrap_or_default())
            .collect();
        let position = |wanted: &str| shape.iter().position(|call| *call == wanted);
        assert_eq!(position("entry.init"), Some(0), "{log:?}");
        assert!(position("count") < position("descriptor"), "{log:?}");
        assert!(position("descriptor") < position("create"), "{log:?}");
        assert!(position("create") < position("init"), "{log:?}");
        assert!(position("init") < position("describe"), "{log:?}");
        assert!(position("describe") < position("destroy"), "{log:?}");
        assert_eq!(
            shape.last().copied(),
            Some("entry.deinit"),
            "the bundle stops last, after every instance is gone: {log:?}"
        );
    }

    /// Frames are dispatched out of order and on more than one thread at once,
    /// and **no instance is re-entered**. The fixture's barrier is what makes
    /// the overlap deliberate: a stress test that asserts an absence proves
    /// nothing unless the overlap it is about really happened.
    #[test]
    fn two_instances_render_at_once_and_neither_is_re_entered() {
        let test = "two_instances_render_at_once_and_neither_is_re_entered";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Passthrough))
            .expect("describe");
        let mut first = host.create(&described).expect("the first instance");
        let mut second = host.create(&described).expect("the second instance");
        probe_reset();
        set_count(b"LumitLfxProbeRendezvous\0", 2);

        std::thread::scope(|threads| {
            for (offset, instance) in [&mut first, &mut second].into_iter().enumerate() {
                threads.spawn(move || {
                    let input = picture(2, 2, 1.0);
                    let mut output = picture(2, 2, 0.0);
                    // Out of order by design: the later frame goes first.
                    let time = if offset == 0 { 7.0 } else { 3.0 };
                    instance
                        .process(
                            &Request::full_frame(time, 2, 2),
                            &[Value::Float(1.0)],
                            Pixels::F32(&input),
                            PixelsMut::F32(&mut output),
                        )
                        .expect("the frame");
                    assert!(output
                        .iter()
                        .all(|sample| (*sample - 1.0).abs() < f32::EPSILON));
                });
            }
        });
        set_count(b"LumitLfxProbeRendezvous\0", 0);

        assert_eq!(
            read_count(b"LumitLfxProbeMaxConcurrent\0"),
            2,
            "the two really were inside `process` at the same moment"
        );
        assert_eq!(
            read_count(b"LumitLfxProbeMaxPerInstance\0"),
            1,
            "one instance is never re-entered"
        );
        // Dropped here, on the thread that made them: the lifecycle is the
        // control thread's however the frames were dispatched.
        drop((first, second));
    }

    /// The three dangerous personalities do nothing dangerous until their own
    /// environment variable is set in the process that loaded the bundle - a
    /// scan describes every plugin in a bundle, so a personality that crashed
    /// on sight would take the whole suite with it.
    #[test]
    fn the_dangerous_personalities_are_disarmed_unless_their_variable_is_set() {
        let test = "the_dangerous_personalities_are_disarmed_unless_their_variable_is_set";
        let _guard = fixture_lock();
        for variable in [
            lumit_lfx_testplug::CRASH_ON_FRAME_ENV,
            lumit_lfx_testplug::HANG_ENV,
            lumit_lfx_testplug::NOTE_SPAM_ENV,
        ] {
            assert!(
                std::env::var_os(variable).is_none(),
                "{variable} is set, and this suite is not the place to arm it"
            );
        }
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        for personality in [Personality::Crash, Personality::Hang, Personality::NoteSpam] {
            let described = host.describe(&id(personality)).expect("describe");
            let mut instance = host.create(&described).expect("create");
            let input = picture(2, 2, 1.0);
            let mut output = picture(2, 2, 0.0);
            instance
                .process(
                    &Request::full_frame(0.0, 2, 2),
                    &[Value::Float(1.0)],
                    Pixels::F32(&input),
                    PixelsMut::F32(&mut output),
                )
                .expect("a disarmed personality is an ordinary effect");
            assert!(output
                .iter()
                .all(|sample| (*sample - 1.0).abs() < f32::EPSILON));
        }
        assert!(
            host.notes().len() <= 1,
            "disarmed, the note-spam personality says one thing"
        );
    }

    /// A plugin that says too much is heard to the ceiling and no further, and
    /// one line longer than the log's own ceiling is cut rather than dropped -
    /// a plugin that says too much should still be heard.
    ///
    /// The plugin's own count is read back beside the host's, because "the
    /// host kept sixty-four" and "the plugin only sent sixty-four" are the same
    /// number seen from the two sides and only one of them is what this is
    /// about.
    #[test]
    fn a_plugin_that_says_too_much_is_heard_to_the_ceiling_and_no_further() {
        let test = "a_plugin_that_says_too_much_is_heard_to_the_ceiling_and_no_further";
        let _guard = fixture_lock();
        {
            let Some(host) = opened() else {
                skipped(test);
                return;
            };
            probe_reset();
            // Disarmed, `NoteSpam` says one line per frame, so twice the
            // ceiling's worth of frames is twice the ceiling's worth of lines.
            let described = host.describe(&id(Personality::NoteSpam)).expect("describe");
            let mut instance = host.create(&described).expect("create");
            let input = picture(1, 1, 1.0);
            let mut output = picture(1, 1, 0.0);
            for frame in 0..(MAX_NOTES * 2) {
                instance
                    .process(
                        &Request::full_frame(frame as f64, 1, 1),
                        &[Value::Float(1.0)],
                        Pixels::F32(&input),
                        PixelsMut::F32(&mut output),
                    )
                    .expect("process");
            }
            let sent = read_count(b"LumitLfxProbeNotesSent\0") as usize;
            assert_eq!(sent, MAX_NOTES * 2, "the plugin said one line a frame");
            assert_eq!(
                host.notes().len(),
                MAX_NOTES,
                "the host heard it to the ceiling"
            );
            assert!(
                sent > host.notes().len(),
                "the host capped rather than the plugin under-sending"
            );
        }
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let long =
            CString::new(vec![b'a'; (LFX_MAX_LOG_BYTES as usize) * 2]).expect("no interior nul");
        for _ in 0..(MAX_NOTES * 2) {
            // SAFETY: this module's own host, and a message that lives for the
            // call - which is what a plugin's own `log` call hands over.
            unsafe { host_log(&*host.host, LFX_LOG_INFO, long.as_ptr()) };
        }
        let notes = host.notes();
        assert_eq!(notes.len(), MAX_NOTES);
        let first = notes.first().expect("a note");
        assert_eq!(first.level, LFX_LOG_INFO);
        assert_eq!(first.text.len(), LFX_MAX_LOG_BYTES as usize);
    }

    // ------------------------------------------ the stranger's own numbers --

    /// A declaration whose size prefix is shorter than this header's names
    /// fields that are not there to read, so the row is lost and the line says
    /// so - without the fields ever being read.
    #[test]
    fn a_declaration_shorter_than_the_header_is_a_report_line() {
        let mut into = Describe::new();
        let mut sink = sink_for(&mut into);
        let declared = LfxFloatParam {
            struct_size: 4,
            unit: LFX_UNIT_RAW,
            flags: 0,
            bounds: 0,
            id: c"gain".as_ptr(),
            label: c"Gain".as_ptr(),
            default_value: 1.0,
            slider_min: 0.0,
            slider_max: 1.0,
            hard_min: 0.0,
            hard_max: 0.0,
        };
        // SAFETY: a sink this module built, and a live declaration whose first
        // word is its own - untrue - size prefix.
        assert!(!unsafe { declare_float(&raw mut sink, &raw const declared) });
        // And a null declaration is the same answer.
        // SAFETY: as above.
        assert!(!unsafe { declare_float(&raw mut sink, std::ptr::null()) });

        let described = into.finish().expect("a lost row is not a fault");
        assert!(described.params.is_empty());
        assert_eq!(
            described
                .report
                .iter()
                .filter(|line| matches!(
                    line,
                    LfxRejection::UnreadableDeclaration {
                        kind: LFX_PARAM_FLOAT,
                        ..
                    }
                ))
                .count(),
            2,
            "{:?}",
            described.report
        );
    }

    /// A string with no end inside the header's ceiling loses its row: the host
    /// never walks past the limit looking for the end.
    #[test]
    fn a_string_with_no_end_inside_the_ceiling_loses_its_row() {
        let endless = vec![b'a'; (LFX_MAX_STRING_BYTES as usize) * 2];
        let mut into = Describe::new();
        let mut sink = sink_for(&mut into);
        let template = LfxFloatParam {
            struct_size: size_of::<LfxFloatParam>() as u32,
            unit: LFX_UNIT_RAW,
            flags: 0,
            bounds: 0,
            id: endless.as_ptr().cast::<c_char>(),
            label: c"Gain".as_ptr(),
            default_value: 1.0,
            slider_min: 0.0,
            slider_max: 1.0,
            hard_min: 0.0,
            hard_max: 0.0,
        };
        // SAFETY: a sink this module built, and a declaration whose id runs on
        // for twice the ceiling inside an allocation of its own - so the read
        // that stops at the ceiling stops inside it.
        assert!(!unsafe { declare_float(&raw mut sink, &raw const template) });

        let labelled = LfxFloatParam {
            id: c"gain".as_ptr(),
            label: endless.as_ptr().cast::<c_char>(),
            ..template
        };
        // SAFETY: as above, for the label.
        assert!(!unsafe { declare_float(&raw mut sink, &raw const labelled) });

        let described = into.finish().expect("a lost row is not a fault");
        assert!(described.params.is_empty());
        assert!(
            described
                .report
                .iter()
                .any(|line| matches!(line, LfxRejection::UnreadableDeclaration { bytes: 0, .. })),
            "the line names a size prefix the declaration's was not: {:?}",
            described.report
        );
        assert!(described
            .report
            .iter()
            .any(|line| matches!(line, LfxRejection::StringTooLong { id, .. } if id == "gain")));
    }

    /// A count past the header's own ceiling is declined **before** its array is
    /// read, and the line names the count the plugin declared rather than the
    /// number that was read - because nothing was read.
    #[test]
    fn a_count_past_the_headers_ceiling_is_declined_before_its_array_is_read() {
        let mut into = Describe::new();
        let mut sink = sink_for(&mut into);
        let declared = LfxChoiceParam {
            struct_size: size_of::<LfxChoiceParam>() as u32,
            unit: LFX_UNIT_RAW,
            flags: 0,
            default_index: 0,
            option_count: u32::MAX,
            divider_count: 0,
            id: c"mode".as_ptr(),
            label: c"Mode".as_ptr(),
            // Null, and never dereferenced: the count is refused first.
            options: std::ptr::null(),
            dividers_after: std::ptr::null(),
        };
        // SAFETY: a sink this module built, and a live declaration whose
        // options array is null because the count is checked before it.
        assert!(!unsafe { declare_choice(&raw mut sink, &raw const declared) });

        let described = into.finish().expect("a lost row is not a fault");
        assert!(described.params.is_empty());
        assert!(
            described.report.iter().any(|line| matches!(
                line,
                LfxRejection::TooManyOptions { id, declared } if id == "mode" && *declared == u32::MAX
            )),
            "{:?}",
            described.report
        );
    }

    /// A descriptor's counted list is declined **before** its array is read:
    /// the count is a stranger's `uint32_t` and the array is whatever is behind
    /// it, so a count past the header's ceiling leaves the list empty rather
    /// than clamped to the ceiling and read to it.
    #[test]
    fn a_descriptor_count_past_the_headers_ceiling_is_declined_before_its_array_is_read() {
        // The arrays hold two entries each and the counts claim four billion.
        // Reading to the ceiling would take eight values and sixteen
        // `const char *` out of them, dereference fourteen words of whatever
        // followed, and lift the bytes into the scan report - the read the
        // check exists to prevent, performed because the check failed.
        let categories = [lumit_lfx_abi::LFX_CATEGORY_COLOUR; 2];
        let extension = c"lfx.temporal";
        let required = [extension.as_ptr(); 2];
        let descriptor = LfxDescriptor {
            struct_size: size_of::<LfxDescriptor>() as u32,
            id: c"org.lumit.overreaching".as_ptr(),
            name: c"Overreaching".as_ptr(),
            vendor: c"Somebody".as_ptr(),
            major: 1,
            minor: 0,
            patch: 0,
            categories: categories.as_ptr(),
            category_count: u32::MAX,
            traits: std::ptr::null(),
            required_extensions: required.as_ptr(),
            required_extension_count: u32::MAX,
        };
        // SAFETY: a live descriptor whose counted arrays are shorter than the
        // counts claim, which is the case the counts are checked first for.
        let (identity, lines) =
            unsafe { identity_of(&raw const descriptor) }.expect("a readable id");
        assert!(
            identity.categories.is_empty(),
            "a count past the ceiling had its array read anyway"
        );
        assert!(
            identity.required_extensions.is_empty(),
            "a count past the ceiling had its array read anyway"
        );
        assert!(lines.iter().any(|line| matches!(
            line,
            LfxRejection::TooManyCategories { declared } if *declared == u32::MAX
        )));
        assert!(lines.iter().any(|line| matches!(
            line,
            LfxRejection::TooManyRequiredExtensions { declared } if *declared == u32::MAX
        )));

        // An honest count below the ceiling is read in full, which is the other
        // half of the rule: the check is on the count, not on the list.
        let honest = LfxDescriptor {
            category_count: 2,
            required_extension_count: 2,
            ..descriptor
        };
        // SAFETY: as above, with counts its arrays really do cover.
        let (identity, lines) = unsafe { identity_of(&raw const honest) }.expect("a readable id");
        assert_eq!(identity.categories.len(), 2);
        assert_eq!(identity.required_extensions.len(), 2);
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// A descriptor whose size prefix is shorter than this header's names
    /// fields that are not there, and a descriptor with no readable id names
    /// nothing at all. Both are lines against the bundle, because there is no
    /// row to file them against.
    #[test]
    fn a_descriptor_shorter_than_the_header_is_a_line_against_the_bundle() {
        let short: [u32; 1] = [4];
        // SAFETY: four readable bytes whose first word is the descriptor's own -
        // untrue - size prefix, which is all this reads before refusing.
        let refused = unsafe { identity_of(short.as_ptr().cast::<LfxDescriptor>()) };
        assert!(matches!(
            refused,
            Err(LfxRejection::UnreadableDeclaration {
                kind: lumit_lfx_abi::LFX_PARAM_UNSET,
                bytes: 4
            })
        ));

        let nameless = LfxDescriptor {
            struct_size: size_of::<LfxDescriptor>() as u32,
            id: std::ptr::null(),
            name: c"Nameless".as_ptr(),
            vendor: c"Somebody".as_ptr(),
            major: 1,
            minor: 0,
            patch: 0,
            categories: std::ptr::null(),
            category_count: 0,
            traits: std::ptr::null(),
            required_extensions: std::ptr::null(),
            required_extension_count: 0,
        };
        // SAFETY: a live descriptor whose id is the null it declares.
        let refused = unsafe { identity_of(&raw const nameless) };
        assert!(
            matches!(
                refused,
                Err(LfxRejection::UnreadableDeclaration {
                    kind: lumit_lfx_abi::LFX_PARAM_UNSET,
                    bytes: 0
                })
            ),
            "a descriptor nobody can name carries no size prefix to quote"
        );
    }

    /// Two descriptors declaring one id are one effect and a line: a repeated
    /// id would be two rows in the menu, two match names and one frame key,
    /// with either of them silently driving the first.
    #[test]
    fn two_descriptors_declaring_one_id_are_one_effect_and_a_line() {
        let template = LfxDescriptor {
            struct_size: size_of::<LfxDescriptor>() as u32,
            id: c"org.example.blur".as_ptr(),
            name: c"Blur".as_ptr(),
            vendor: c"Somebody".as_ptr(),
            major: 1,
            minor: 0,
            patch: 0,
            categories: std::ptr::null(),
            category_count: 0,
            traits: std::ptr::null(),
            required_extensions: std::ptr::null(),
            required_extension_count: 0,
        };
        let again = LfxDescriptor {
            name: c"Blur, again".as_ptr(),
            ..template
        };
        let other = LfxDescriptor {
            id: c"org.example.sharpen".as_ptr(),
            name: c"Sharpen".as_ptr(),
            ..template
        };
        let list = [
            &raw const template,
            &raw const again,
            &raw const other,
            std::ptr::null(),
        ];
        // SAFETY: three live descriptors and a null, which is the shape a
        // bundle's own list has.
        let catalogue = unsafe { catalogue_of(&list) };
        let ids: Vec<&str> = catalogue
            .plugins
            .iter()
            .map(|plugin| plugin.id.as_str())
            .collect();
        assert_eq!(ids, ["org.example.blur", "org.example.sharpen"]);
        assert_eq!(catalogue.reports.len(), catalogue.plugins.len());
        assert!(
            catalogue.report.iter().any(|line| matches!(
                line,
                LfxRejection::DuplicateEffectId { id } if id == "org.example.blur"
            )),
            "{:?}",
            catalogue.report
        );
    }

    /// A bundle declaring more effects than the header carries is refused
    /// outright rather than truncated to the ceiling and carried on with.
    ///
    /// The two answers are not interchangeable: a caller that asks
    /// `report().iter().any(LfxRejection::refuses_the_effect)` and one that
    /// does not would otherwise see the same bundle as refused and as whole,
    /// with 976 of its effects gone from the second reading.
    #[test]
    fn a_bundle_past_the_headers_effect_ceiling_is_refused_rather_than_truncated() {
        let test = "a_bundle_past_the_headers_effect_ceiling_is_refused_rather_than_truncated";
        let _guard = fixture_lock();
        let Some(path) = fixture() else {
            skipped(test);
            return;
        };
        probe_reset();
        set_count(
            b"LumitLfxProbeDeclaredCount\0",
            LFX_MAX_EFFECTS_PER_BUNDLE + 1,
        );
        let opened = LocalHost::open(path);
        // Put the bundle back before anything can fail: it is a static of the
        // loaded module, so it would outlive this test.
        set_count(b"LumitLfxProbeDeclaredCount\0", 0);
        assert!(
            matches!(
                opened,
                Err(LocalError::Refused(LfxRejection::PastCeiling {
                    ceiling: Ceiling::EffectsPerBundle,
                    ..
                }))
            ),
            "a bundle past the ceiling loaded"
        );

        // And the honest count opens exactly as it did before.
        let host = LocalHost::open(path).expect("the honest count");
        assert_eq!(host.plugins().len(), PERSONALITIES.len());
    }

    /// An instance whose own table is shorter than this header's is refused
    /// before a reference is formed over it - the same rule the entry, the
    /// trait block and every declaration are read under, at the one struct
    /// that had been exempt.
    #[test]
    fn an_instance_table_shorter_than_the_header_is_refused_before_it_is_read() {
        let test = "an_instance_table_shorter_than_the_header_is_refused_before_it_is_read";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Passthrough))
            .expect("describe");

        // Long enough to carry its own `destroy`, so the instance is taken
        // down rather than leaked: the refusal costs the bundle nothing it
        // could have kept.
        probe_reset();
        set_count(
            b"LumitLfxProbeTableBytes\0",
            u32::try_from(DESTROYABLE_BYTES).unwrap_or(u32::MAX),
        );
        let refused = host.create(&described);
        set_count(b"LumitLfxProbeTableBytes\0", 0);
        assert!(
            matches!(
                refused,
                Err(LocalError::ShortPlugin { bytes, .. })
                    if bytes as usize == DESTROYABLE_BYTES
            ),
            "a short table was read as a whole one"
        );
        assert!(
            probe_log().iter().any(|call| call.starts_with("destroy:")),
            "a table that reaches its own destroy is taken down: {:?}",
            probe_log()
        );

        // And one too short even for that is refused all the same, with
        // nothing called on it at all.
        probe_reset();
        set_count(b"LumitLfxProbeTableBytes\0", 4);
        let refused = host.create(&described);
        set_count(b"LumitLfxProbeTableBytes\0", 0);
        assert!(matches!(
            refused,
            Err(LocalError::ShortPlugin { bytes: 4, .. })
        ));
        assert!(
            !probe_log().iter().any(|call| call.starts_with("destroy:")),
            "a table that does not reach its own destroy had it called anyway"
        );

        // The honest table is unaffected.
        host.create(&described).expect("the honest table");
    }

    /// A trait block shorter than this header's names fields that are not
    /// there, so it is read as the `NULL` it is indistinguishable from: the
    /// pessimistic case, with every enumeration at its own `UNSET`.
    ///
    /// The last of the ABI's five size-prefixed structs, and the one whose
    /// prefix a host has the least excuse to read late - it is one dereference
    /// past a descriptor it has already checked, so a reference formed over it
    /// looks exactly as safe as the one before it. The fixture's block really
    /// is two words rather than a whole block claiming to be, so a host that
    /// read it the other way round reads past the end of a static.
    #[test]
    fn a_trait_block_shorter_than_the_header_reads_as_the_null_it_is_indistinguishable_from() {
        let test =
            "a_trait_block_shorter_than_the_header_reads_as_the_null_it_is_indistinguishable_from";
        let _guard = fixture_lock();
        let Some(path) = fixture() else {
            skipped(test);
            return;
        };
        probe_reset();
        set_count(b"LumitLfxProbeShortTraits\0", 1);
        let opened = LocalHost::open(path);
        // Put the bundle back before anything can fail: it is a static of the
        // loaded module, so it would outlive this test.
        set_count(b"LumitLfxProbeShortTraits\0", 0);
        let host = opened.expect("a short trait block is not a refusal of the bundle");
        let full = host
            .plugins()
            .iter()
            .find(|plugin| plugin.id == id(Personality::Full))
            .expect("the personality that declares traits");
        assert!(
            full.traits.is_none(),
            "a short block was read as a whole one: {:?}",
            full.traits
        );
        assert!(
            host.report().is_empty(),
            "and it is not a line against the bundle either: {:?}",
            host.report()
        );

        // The honest block is unaffected, and says what it always said.
        let host = LocalHost::open(path).expect("the honest block");
        let full = host
            .plugins()
            .iter()
            .find(|plugin| plugin.id == id(Personality::Full))
            .expect("the personality that declares traits");
        let declared = full.traits.expect("the honest block is read");
        assert_eq!(declared.cost, lumit_lfx_abi::LFX_COST_CHEAP);
    }

    /// A curve declaring more points than the ABI carries files **one** line,
    /// and it names the count the plugin declared. Declaring the row anyway
    /// would file a second line naming the nought points that were read, and
    /// the report would carry two sentences for one control.
    #[test]
    fn a_curve_past_the_point_ceiling_is_one_line_naming_the_count_declared() {
        let mut into = Describe::new();
        let mut sink = sink_for(&mut into);
        let declared = LfxCurveParam {
            struct_size: size_of::<LfxCurveParam>() as u32,
            unit: LFX_UNIT_RAW,
            flags: 0,
            point_count: u32::MAX,
            id: c"tone".as_ptr(),
            label: c"Tone".as_ptr(),
            // Null, and never dereferenced: the count is refused first.
            points: std::ptr::null(),
        };
        // SAFETY: a sink this module built, and a live declaration whose
        // points array is null because the count is checked before it.
        assert!(!unsafe { declare_curve(&raw mut sink, &raw const declared) });

        let described = into.finish().expect("a lost row is not a fault");
        assert!(described.params.is_empty());
        let lines: Vec<&LfxRejection> = described
            .report
            .iter()
            .filter(|line| matches!(line, LfxRejection::CurvePointsOutOfRange { .. }))
            .collect();
        assert_eq!(lines.len(), 1, "{:?}", described.report);
        assert!(matches!(
            lines.first(),
            Some(LfxRejection::CurvePointsOutOfRange { id, declared })
                if id == "tone" && *declared == u32::MAX
        ));
    }

    /// The dense value array is aligned for its own elements however wide the
    /// stride the caller asked for: the header says a plugin reaches an element
    /// with an ordinary aligned read, and a stride of the struct's size plus
    /// four would put every other one four bytes off.
    #[test]
    fn the_value_array_is_aligned_for_its_elements_at_any_stride() {
        let test = "the_value_array_is_aligned_for_its_elements_at_any_stride";
        let _guard = fixture_lock();

        // The rounding itself, which is the whole of the guarantee.
        let mut request = Request::full_frame(0.0, 2, 2);
        let element = size_of::<LfxValue>() as u32;
        request.value_stride = Some(element + 4);
        let stride = request.stride().expect("inside the ceiling");
        assert_eq!(stride % align_of::<LfxValue>() as u32, 0);
        assert!(stride >= element + 4);

        // And a stride nobody could mean is refused rather than allocated: the
        // array is `stride × count` bytes and the allocator's answer to fifty
        // gigabytes is to end the process.
        request.value_stride = Some(u32::MAX);
        assert!(matches!(
            request.stride(),
            Err(LocalError::ValueStride {
                given: u32::MAX,
                most: MAX_VALUE_STRIDE
            })
        ));

        // The plugin's own side: the fixture refuses an element that is not
        // aligned for `lfx_value`, which is what a C plugin's natural read
        // would fault on, so a misaligned array is a failed frame here.
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Full)).expect("describe");
        let mut instance = host.create(&described).expect("create");
        let input = picture(2, 2, 1.0);
        let mut output = picture(2, 2, 0.0);
        let mut request = Request::full_frame(0.0, 2, 2);
        request.value_stride = Some(element + 4);
        instance
            .process(
                &request,
                &full_values(),
                Pixels::F32(&input),
                PixelsMut::F32(&mut output),
            )
            .expect("an odd stride is still an aligned array");
        assert!(output
            .iter()
            .all(|sample| (*sample - 0.5).abs() < f32::EPSILON));
    }

    /// A frame's origin is the buffer's own top-left corner rather than the
    /// region asked for, so a plugin finding its first requested pixel at
    /// `roi_x0 - origin_x` finds it where it is.
    #[test]
    fn a_frames_origin_is_the_buffers_own_corner_rather_than_the_region_asked_for() {
        let test = "a_frames_origin_is_the_buffers_own_corner_rather_than_the_region_asked_for";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host
            .describe(&id(Personality::Passthrough))
            .expect("describe");
        let mut instance = host.create(&described).expect("create");

        // A twenty-square buffer whose own corner is at (10, 10), with four
        // pixels of it asked for.
        let request = Request {
            time: 0.0,
            width: 20,
            height: 20,
            roi: (12, 12, 16, 16),
            dod: (10, 10, 30, 30),
            value_stride: None,
        };
        let input = picture(20, 20, 1.0);
        let mut output = picture(20, 20, 0.0);
        probe_reset();
        instance
            .process(
                &request,
                &[Value::Float(1.0)],
                Pixels::F32(&input),
                PixelsMut::F32(&mut output),
            )
            .expect("process");
        assert_eq!(
            read_probe(b"LumitLfxProbeRegions\0"),
            ["10:10:12:12:16:16"],
            "the plugin was told its buffer starts where the region does"
        );

        // A definition that is not the buffer, or a region outside it, is
        // refused rather than written down two different ways.
        for wrong in [
            Request {
                dod: (10, 10, 20, 20),
                ..request
            },
            Request {
                roi: (0, 0, 20, 20),
                ..request
            },
        ] {
            assert!(
                matches!(
                    instance.process(
                        &wrong,
                        &[Value::Float(1.0)],
                        Pixels::F32(&input),
                        PixelsMut::F32(&mut output),
                    ),
                    Err(LocalError::Region { .. })
                ),
                "{wrong:?} was accepted"
            );
        }
    }

    /// The elements the host writes are exactly the declarations that cross -
    /// the same answer `Carriage::crosses` gives one level down, read off a
    /// built schema rather than spelled a second time.
    #[test]
    fn the_elements_the_host_writes_are_the_ones_that_cross() {
        let test = "the_elements_the_host_writes_are_the_ones_that_cross";
        let _guard = fixture_lock();
        let Some(host) = opened() else {
            skipped(test);
            return;
        };
        let described = host.describe(&id(Personality::Full)).expect("describe");
        let schema = schema::schema_of(&described).expect("the lowering");
        let routes = schema::value_routes(&described, &schema);
        let crossing: Vec<u32> = {
            let mut seen: Vec<u32> = routes.iter().map(|route| route.element).collect();
            seen.dedup();
            seen
        };
        let elements = elements_of(&described);
        assert_eq!(elements.len(), crossing.len());
        for route in &routes {
            assert_eq!(
                elements.get(route.element as usize).copied(),
                Some(route.kind),
                "element {} is the kind its route says",
                route.element
            );
        }
        // An Action takes no element, and is the only kind that takes none -
        // which is the answer `Carriage` gives one level down.
        assert!(described
            .params
            .iter()
            .any(|param| param.kind == Declared::Action));
        assert!(!elements.contains(&LFX_PARAM_ACTION));
        assert!(!schema::carriage(&ParamKind::Action).crosses());
    }

    /// The twelve values `Full` declares, in declaration order - the dense
    /// array's own vocabulary, one element per declaration that carries a
    /// value.
    fn full_values() -> Vec<Value> {
        vec![
            Value::Float(0.5),
            Value::Slider(100.0),
            Value::Int(4),
            Value::Angle(0.0),
            Value::Bool(false),
            Value::Choice(1),
            Value::Colour([1.0, 1.0, 1.0, 1.0]),
            Value::Seed(7),
            Value::Point2([0.0, 0.0]),
            Value::Point3([0.0, 0.0, 0.0]),
            Value::Curve(vec![[0.0, 0.0], [1.0, 1.0]]),
            Value::File(None),
        ]
    }
}
