//! `lumit-lfx-abi` - the LFX C ABI: the canonical header and its Rust mirror
//! (docs/12-PLUGINS.md §3, docs/impl/lfx.md §2 says how).
//!
//! # In plain terms
//!
//! LFX is Lumit's own plugin interface: somebody writes an effect in C or Rust,
//! builds it against the header in `crates/lumit-lfx-abi/include/lfx.h`, and
//! Lumit runs it in a process of its own. This crate is that agreement
//! written down twice - once in C, which is what a vendor compiles against, and
//! once here in `#[repr(C)]` Rust, which is what the host and the broker read
//! the bundle's memory with - plus the tests that hold the two copies to the
//! same numbers.
//!
//! Nothing here has behaviour, and nothing else may go in this crate. It is
//! **MIT**, deliberately, so a proprietary vendor adopts the header without
//! licence anxiety, and `cargo deny` checks what comes *in* to a crate rather
//! than what goes out of one: a helper that drifted in here would become MIT by
//! accident.
//!
//! # The freeze, and how it is a mechanism
//!
//! [`LFX_ABI_VERSION`] is the integer that intends to reach 2 never. Every
//! struct opens with `struct_size`, growth is additive and at the end, and
//! fields are never re-ordered or removed - so version 1's fields are always
//! the first bytes of a later version's struct, and `struct_size` is read in
//! the direction growth happens: a side handed a *longer* struct than it was
//! built against reads the fields it knows and ignores the tail.
//!
//! The other direction is answered per struct rather than in general, because a
//! struct *shorter* than the reader's own is missing fields that reader
//! requires and version 1 invents none of them. [`LfxEntry`] and [`LfxPlugin`]
//! are refused - a function pointer that is not there cannot be called;
//! [`LfxTraits`] reads as the pessimistic case, exactly as a null block does;
//! [`LfxDescriptor`] and the declaration records are declined with a line in
//! the scan report. The cost is that those two tables do not grow: a field
//! appended to either would leave every bundle built against version 1 short,
//! so they gain capability through `get_extension` instead.
//!
//! For a neighbouring reason no answer the *plugin* gives crosses as a C
//! `bool`. `bool` has only two valid representations and the host cannot
//! inspect the byte a stranger's compiler left in the return register before it
//! reads it, so [`LfxEntry::init`], [`LfxPlugin::init`] and
//! [`LfxPlugin::describe`] answer a `u32` in which any non-zero value is true -
//! the choice [`LfxBoolParam::default_value`] already makes for a declared
//! switch. The `bool`s that remain are the host's own, read by the plugin
//! rather than written by it: [`LfxDescribeSink`]'s answers and
//! [`LfxProcess::cancelled`].
//!
//! [`LfxValue`] is the one named exemption, because it crosses as a dense array
//! addressed by index: what the two sides must agree on is the element
//! **stride**, not where a field starts, so [`LfxProcess::value_stride`] is what
//! both sides walk by. A plugin striding by its own `size_of` after the struct
//! has grown would read correct-looking kind tags over silently wrong values,
//! which no kind check can see. The host guarantees that the array's base and
//! the stride are both aligned for [`LfxValue`], so every element is too and a
//! plugin reads one with an ordinary aligned read.
//!
//! Every enumeration crosses as a `u32`. A C `enum`'s width is the compiler's
//! to choose, so the header declares its constants in anonymous enums and its
//! *fields* in fixed-width typedefs, and this mirror does the same.
//!
//! # Thread role and contract
//!
//! None: these are declarations. The contract they carry is the plugin's -
//! `describe` and the instance lifecycle run on one host-designated control
//! thread, `process` runs on any worker thread and on different instances
//! concurrently, and one instance is never re-entered. Each declaration below
//! restates the context it is called in, as the header does.
// The crate as a whole allows `unsafe` - the layout suite walks a strided array
// the way a plugin does, and a raw read is what that is. The library half needs
// none of it: a description of a shape has nothing to do unsafely, so the deny
// is put back here, where it costs nothing and says so.
#![deny(unsafe_code)]

use std::ffi::{c_char, c_void};

/// The core ABI version: one integer, and it intends to reach 2 never.
///
/// The C half of the layout suite returns the header's own spelling of this,
/// so a mirror that drifted from the header would fail the test rather than the
/// review.
pub const LFX_ABI_VERSION: u32 = 1;

/// The one symbol a bundle exports, NUL-terminated for the dynamic loader.
pub const LFX_ENTRY_SYMBOL: &[u8] = b"lfx_entry_point\0";

// -------------------------------------------------------------- ceilings --

/// The most bytes any `*const c_char` in this ABI may occupy, the NUL counted,
/// and every one of them is UTF-8.
///
/// Every count and every string a plugin hands the host is bounded, and the
/// bounds are part of the frozen agreement rather than a number each reader
/// invents: a limit that turned out too small cannot be raised once a vendor
/// has shipped inside it, and one that turned out too wide cannot be narrowed.
/// A string with no NUL inside the ceiling is a refusal rather than a longer
/// read - the host never walks past the limit looking for the end - and the
/// host enforces the same numbers where a stranger's bytes arrive, so the two
/// sides agree by construction rather than by coincidence.
pub const LFX_MAX_STRING_BYTES: u32 = 1024;
/// What [`LfxHost::log`] may say, which is longer because it is diagnostics
/// rather than identity.
pub const LFX_MAX_LOG_BYTES: u32 = 4096;
/// One per member of [`LfxCategory`], which is the most an effect could claim.
pub const LFX_MAX_CATEGORIES: u32 = 8;
/// The most options one [`LfxChoiceParam`] may offer.
pub const LFX_MAX_OPTIONS: u32 = 256;
/// The most rules one [`LfxChoiceParam`] may draw between them, which is the
/// option ceiling itself.
///
/// A dropdown draws at most one rule per option, so [`LFX_MAX_OPTIONS`] is the
/// only number here that cannot turn out too small. A smaller one would admit a
/// dropdown and refuse the divider list that groups it, and a ceiling declines
/// the whole declaration rather than trimming it: the plugin would lose the
/// control, not the rules.
pub const LFX_MAX_DIVIDERS: u32 = LFX_MAX_OPTIONS;
/// The most extensions one [`LfxFileParam`] may filter on.
pub const LFX_MAX_FILTERS: u32 = 32;
/// The most extensions one [`LfxDescriptor`] may require.
pub const LFX_MAX_REQUIRED_EXTENSIONS: u32 = 16;
/// The most effects one bundle may hold.
pub const LFX_MAX_EFFECTS_PER_BUNDLE: u32 = 1024;
/// The most controls one effect may declare through [`LfxDescribeSink`], and
/// the most headings it may open.
///
/// Describe runs on the control thread and every row it mints lives as long as
/// the session, so the count is bounded here rather than left to whatever a
/// plugin's loop happens to produce. Exceeding it refuses the **effect** rather
/// than costing one row: a panel the host cannot draw is not a control lost.
pub const LFX_MAX_PARAMS: u32 = 512;
/// The fewest control points a tone curve may carry: one point is not a curve.
pub const LFX_MIN_CURVE_POINTS: u32 = 2;
/// The most control points a tone curve may carry.
pub const LFX_MAX_CURVE_POINTS: u32 = 16;
/// How far either end of [`LfxTraits`]'s temporal window may reach, in comp
/// frames. Signed, because the window is: `[-64, 64]` is the whole of what a
/// plugin may declare, and it must contain the frame being rendered.
pub const LFX_MAX_TEMPORAL_WINDOW: i32 = 64;

// ------------------------------------------------------------------ kinds --

/// What a declared control is ([`LfxDescribeSink`]), as it crosses: a `u32`.
pub type LfxParamKind = u32;

/// Nobody declared a kind. Never valid on the wire.
pub const LFX_PARAM_UNSET: LfxParamKind = 0;
/// An unbounded number, with optional hard bounds.
pub const LFX_PARAM_FLOAT: LfxParamKind = 1;
/// A closed range, optionally logarithmic.
pub const LFX_PARAM_SLIDER: LfxParamKind = 2;
/// A whole number.
pub const LFX_PARAM_INT: LfxParamKind = 3;
/// A switch.
pub const LFX_PARAM_BOOL: LfxParamKind = 4;
/// A dropdown, with its dividers declared rather than guessed.
pub const LFX_PARAM_CHOICE: LfxParamKind = 5;
/// Scene-linear RGBA.
pub const LFX_PARAM_COLOUR: LfxParamKind = 6;
/// Degrees, drawn as a dial.
pub const LFX_PARAM_ANGLE: LfxParamKind = 7;
/// The randomness a seeded effect follows.
pub const LFX_PARAM_SEED: LfxParamKind = 8;
/// Two rows, `<id>_x` and `<id>_y`, folded back into one crosshair by the panel.
pub const LFX_PARAM_POINT2: LfxParamKind = 9;
/// Three rows, `<id>_x`, `<id>_y` and `<id>_z`.
pub const LFX_PARAM_POINT3: LfxParamKind = 10;
/// The *tone* curve: points in the unit square.
pub const LFX_PARAM_CURVE: LfxParamKind = 11;
/// A file chosen from a dialog, whose payload rides beside the op rather than
/// in the value bag.
///
/// *ponytail:* the kind is admitted and the payload is not settled. The path a
/// File row resolves to arrives as an auxiliary slot, and the generic file aux
/// that would carry an arbitrary plugin's file does not exist yet - a LUT and a
/// lens file are the only two the host loads today. Until it lands,
/// [`LfxFileValue::path`] is null, and a plugin that cannot work without the
/// path is better off not declaring the row.
pub const LFX_PARAM_FILE: LfxParamKind = 12;
/// A button: no value, no keyframe, nothing in the bag.
pub const LFX_PARAM_ACTION: LfxParamKind = 13;
/// A heading over a run of rows; no row of its own.
pub const LFX_PARAM_GROUP: LfxParamKind = 14;
/// A bezier path. **Reserved, and refused in version 1**: a path with no
/// on-Viewer handles is a control nobody can edit, so it waits for
/// [`LFX_EXT_OVERLAY`].
pub const LFX_PARAM_PATH: LfxParamKind = 15;
/// Text. **Reserved, and refused in version 1**: the resolved value bag carries
/// no text at all, so the value would never reach `process`.
///
/// Both refused discriminants exist from day one, so admitting them later adds
/// no variant and breaks no compiled plugin.
pub const LFX_PARAM_STRING: LfxParamKind = 16;

/// What a number means, as it crosses.
pub type LfxUnit = u32;

/// Nobody decided, which is a describe refusal rather than a unit: a
/// dimensionless control declares [`LFX_UNIT_RAW`] deliberately.
pub const LFX_UNIT_UNSET: LfxUnit = 0;
/// A plain number: a gamma, a count, a threshold, a rate in Hz.
pub const LFX_UNIT_RAW: LfxUnit = 1;
/// A percentage, where 100 is the whole of whatever the control is a share of.
pub const LFX_UNIT_PERCENT: LfxUnit = 2;
/// A per cent of the composition diagonal - the ROI padding's unit, and a
/// describe refusal on a parameter.
pub const LFX_UNIT_PCT_DIAG: LfxUnit = 3;
/// Pixels at composition size, never pixels of the buffer handed over.
pub const LFX_UNIT_PX: LfxUnit = 4;
/// Degrees.
pub const LFX_UNIT_DEGREES: LfxUnit = 5;
/// Seconds of layer time.
pub const LFX_UNIT_SECONDS: LfxUnit = 6;
/// Comp-rate frames.
pub const LFX_UNIT_FRAMES: LfxUnit = 7;

/// A picture family an effect may claim, as it crosses.
pub type LfxCategory = u32;

/// Nobody declared a family.
pub const LFX_CATEGORY_UNSET: LfxCategory = 0;
/// Blur & sharpen.
pub const LFX_CATEGORY_BLUR_SHARPEN: LfxCategory = 1;
/// Colour.
pub const LFX_CATEGORY_COLOUR: LfxCategory = 2;
/// Distortion.
pub const LFX_CATEGORY_DISTORTION: LfxCategory = 3;
/// Generate.
pub const LFX_CATEGORY_GENERATE: LfxCategory = 4;
/// Stylise.
pub const LFX_CATEGORY_STYLISE: LfxCategory = 5;
/// Temporal.
pub const LFX_CATEGORY_TEMPORAL: LfxCategory = 6;
/// Transition.
pub const LFX_CATEGORY_TRANSITION: LfxCategory = 7;
/// Utility - and where an unrecognised value lands, plus a report line.
pub const LFX_CATEGORY_UTILITY: LfxCategory = 8;

/// A working depth, as it crosses.
pub type LfxPixelFormat = u32;

/// Nobody declared a depth.
pub const LFX_PIXEL_UNSET: LfxPixelFormat = 0;
/// Half-float RGBA - the working depth, and what an 8 bpc project sends.
pub const LFX_RGBA_F16: LfxPixelFormat = 1;
/// Float RGBA.
pub const LFX_RGBA_F32: LfxPixelFormat = 2;

/// What one frame costs, as it crosses.
pub type LfxCost = u32;

/// Unstated, which the host lowers to [`LFX_COST_HEAVY`].
pub const LFX_COST_UNSET: LfxCost = 0;
/// Trivial.
pub const LFX_COST_TRIVIAL: LfxCost = 1;
/// Cheap.
pub const LFX_COST_CHEAP: LfxCost = 2;
/// Moderate.
pub const LFX_COST_MODERATE: LfxCost = 3;
/// Heavy.
pub const LFX_COST_HEAVY: LfxCost = 4;

/// How far past an output pixel the effect reads, as it crosses.
pub type LfxRoiKind = u32;

/// Unstated, which the host lowers to [`LFX_ROI_FULL_FRAME`]: claiming less
/// reach than the kernel uses produces tile seams, so the unstated answer is
/// the expensive one.
pub const LFX_ROI_UNSET: LfxRoiKind = 0;
/// One output pixel needs one input pixel.
pub const LFX_ROI_EXACT: LfxRoiKind = 1;
/// Dilated by [`LfxTraits::roi_padding_px`], in px@comp.
pub const LFX_ROI_PADDED: LfxRoiKind = 2;
/// The whole input.
pub const LFX_ROI_FULL_FRAME: LfxRoiKind = 3;

/// Which alpha the effect's maths expects, as it crosses.
pub type LfxAlpha = u32;

/// Unstated, which the host lowers to [`LFX_ALPHA_PREMULTIPLIED`].
pub const LFX_ALPHA_UNSET: LfxAlpha = 0;
/// Premultiplied - the working form.
pub const LFX_ALPHA_PREMULTIPLIED: LfxAlpha = 1;
/// Straight, so the host unpremultiplies before the effect and puts it back
/// after.
pub const LFX_ALPHA_STRAIGHT: LfxAlpha = 2;

/// [`LfxTraits::flags`], as it crosses.
pub type LfxTraitFlags = u32;

/// Nothing declared, which is the pessimistic case here too.
pub const LFX_TRAIT_NONE: LfxTraitFlags = 0;
/// The effect reads its Seed row, and must stay bit-identical between two
/// exports of the same project.
pub const LFX_TRAIT_SEEDED: LfxTraitFlags = 1 << 0;
/// The sole, discouraged opt-out from instance-level concurrency: the host
/// serialises the whole bundle.
///
/// This flag **is** the opt-out docs/12 §3.4 names as a capability. There is no
/// `lfx.thread-unsafe` extension id: asking `get_extension` for one answers the
/// null that means "not offered", which would leave the plugin scheduled
/// concurrently.
pub const LFX_TRAIT_THREAD_UNSAFE: LfxTraitFlags = 1 << 1;
/// [`LfxProcess::cancelled`] is worth calling.
pub const LFX_TRAIT_CANCELLABLE: LfxTraitFlags = 1 << 2;

/// Per-parameter flags, as they cross.
pub type LfxParamFlags = u32;

/// The ordinary row: visible, and animatable.
pub const LFX_PARAM_FLAG_NONE: LfxParamFlags = 0;
/// The row never keyframes: one value for the whole of the effect's life.
pub const LFX_PARAM_FLAG_STATIC: LfxParamFlags = 1 << 0;
/// Declared, kept, serialised - and not drawn.
pub const LFX_PARAM_FLAG_HIDDEN: LfxParamFlags = 1 << 1;

/// Which hard bounds a Float or an Int declaration means, as it crosses.
pub type LfxBounds = u32;

/// Unbounded.
pub const LFX_BOUND_NONE: LfxBounds = 0;
/// [`LfxFloatParam::hard_min`] (or [`LfxIntParam::hard_min`]) is meant.
pub const LFX_BOUND_MIN: LfxBounds = 1 << 0;
/// [`LfxFloatParam::hard_max`] (or [`LfxIntParam::hard_max`]) is meant.
pub const LFX_BOUND_MAX: LfxBounds = 1 << 1;

/// The level a [`LfxHost::log`] line is filed at, as it crosses.
pub type LfxLogLevel = u32;

/// Unstated.
pub const LFX_LOG_UNSET: LfxLogLevel = 0;
/// Something a user would want reported.
pub const LFX_LOG_ERROR: LfxLogLevel = 1;
/// A degradation, or a recovery from one.
pub const LFX_LOG_WARN: LfxLogLevel = 2;
/// Lifecycle.
pub const LFX_LOG_INFO: LfxLogLevel = 3;
/// Detail.
pub const LFX_LOG_DEBUG: LfxLogLevel = 4;
/// Per-frame detail.
pub const LFX_LOG_TRACE: LfxLogLevel = 5;

/// What [`LfxPlugin::process`] returns: a typed refusal, never a message.
pub type LfxStatus = i32;

/// The frame was rendered.
pub const LFX_STATUS_OK: LfxStatus = 0;
/// The effect could not produce this frame; the host renders the input
/// unchanged and badges the layer.
pub const LFX_STATUS_FAILED: LfxStatus = 1;
/// The host asked for the work to stop and the plugin obliged.
pub const LFX_STATUS_CANCELLED: LfxStatus = 2;
/// The plugin ran out of working memory.
pub const LFX_STATUS_OUT_OF_MEMORY: LfxStatus = 3;
/// The request carried something this build does not do. Both depths are
/// mandatory, so this is never the answer to fp16.
pub const LFX_STATUS_UNSUPPORTED: LfxStatus = 4;

// --------------------------------------------------------- extension ids --

/// Frames at other times: the declaration of what is wanted, and the fetch.
/// The only extension version 1 offers.
pub const LFX_EXT_TEMPORAL: &[u8] = b"lfx.temporal\0";
/// Shared-texture input and output. A reserved id and a version 1 header,
/// nothing more.
pub const LFX_EXT_GPU_FRAMES: &[u8] = b"lfx.gpu-frames\0";
/// Viewer interaction and drawing - what unblocks [`LFX_PARAM_PATH`]. A
/// reserved id.
pub const LFX_EXT_OVERLAY: &[u8] = b"lfx.overlay\0";
/// The host's own optical flow, offered to a plugin. A reserved id.
pub const LFX_EXT_MOTION_VECTORS: &[u8] = b"lfx.motion-vectors\0";
/// Audio effects, later. A reserved id.
pub const LFX_EXT_AUDIO: &[u8] = b"lfx.audio\0";

// ------------------------------------------------------------- the traits --

/// What the host schedules from, declared once in the descriptor.
///
/// Reached by pointer from [`LfxDescriptor`] rather than embedded by value: two
/// size-prefixed structs cannot both grow when one is nested inside the other,
/// and an offset that depended on the `struct_size` a plugin happened to be
/// built with is not a number the layout suite could assert.
///
/// **Every field's zero means "unstated"**, and the host lowers each to the
/// pessimistic answer - heavy, full-frame, premultiplied - never to
/// discriminant zero. A `NULL` pointer, a zeroed block and a short one are the
/// same declaration.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LfxTraits {
    /// `size_of::<LfxTraits>()` as the plugin was built.
    pub struct_size: u32,
    /// What one frame costs; unset lowers to heavy.
    pub cost: LfxCost,
    /// How far past an output pixel the effect reads; unset lowers to full
    /// frame.
    pub roi_kind: LfxRoiKind,
    /// The dilation [`LFX_ROI_PADDED`] asks for, in px@comp, sized from the
    /// effect's own hard maximum.
    pub roi_padding_px: f32,
    /// The first frame the effect reads, relative to the one being rendered.
    ///
    /// This pair **is the gate**: a plugin that declares no window never sees a
    /// neighbour, however loudly [`LFX_EXT_TEMPORAL`] asks at render time.
    ///
    /// Each end reaches at most [`LFX_MAX_TEMPORAL_WINDOW`] frames, and the
    /// window must contain the frame being rendered: `temporal_lo <= 0 <=
    /// temporal_hi`. A window that does not is a refusal rather than a clamp -
    /// it is a declaration nobody can honour, not an ambitious one.
    pub temporal_lo: i32,
    /// The last frame the effect reads, relative to the one being rendered.
    pub temporal_hi: i32,
    /// Which alpha the effect's maths expects; unset lowers to premultiplied.
    pub alpha: LfxAlpha,
    /// Seeded, thread-unsafe, cancellable.
    pub flags: LfxTraitFlags,
    /// The working memory one megapixel of output costs. LFX has no host
    /// allocator, so this declaration is the ceiling the ledger is asked for.
    pub scratch_bytes_per_megapixel: u32,
}

// ------------------------------------------------------ the describe sink --

/// An unbounded number, with optional hard bounds.
///
/// Every parameter declaration opens with the same four words - size, unit,
/// flags, and one kind-specific `u32` - and spells them out rather than nesting
/// a shared header by value, for the reason [`LfxTraits`] is reached by
/// pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxFloatParam {
    /// `size_of::<LfxFloatParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Which of `hard_min` and `hard_max` are meant.
    pub bounds: LfxBounds,
    /// snake_case, stable, and hashed to the host's own parameter id.
    pub id: *const c_char,
    /// Sentence case, and what a person reads.
    pub label: *const c_char,
    /// The value a fresh instance starts at.
    pub default_value: f64,
    /// The slider's travel, which typing may exceed.
    pub slider_min: f64,
    /// The other end of the travel.
    pub slider_max: f64,
    /// Read only if `bounds` says so.
    pub hard_min: f64,
    /// Read only if `bounds` says so.
    pub hard_max: f64,
}

/// A **bounded** number: the range is the parameter's whole nature, so there is
/// no soft slider and hard bound to keep apart.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxSliderParam {
    /// `size_of::<LfxSliderParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Non-zero: the thumb moves through the range logarithmically. Honest only
    /// above zero - a range starting at nought has no ratio to raise.
    pub log: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// The value a fresh instance starts at.
    pub default_value: f64,
    /// The closed range's start: the travel and the hard bound at once.
    pub range_min: f64,
    /// The closed range's end.
    pub range_max: f64,
}

/// A whole number. It animates and serialises exactly as a Float does.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxIntParam {
    /// `size_of::<LfxIntParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Which of `hard_min` and `hard_max` are meant.
    pub bounds: LfxBounds,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// The value a fresh instance starts at.
    pub default_value: i64,
    /// The slider's travel, which typing may exceed.
    pub slider_min: i64,
    /// The other end of the travel.
    pub slider_max: i64,
    /// Read only if `bounds` says so.
    pub hard_min: i64,
    /// Read only if `bounds` says so.
    pub hard_max: i64,
}

/// An angle in degrees, drawn as a dial and deliberately unbounded: an angle
/// animates through full turns rather than stopping at 360.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxAngleParam {
    /// `size_of::<LfxAngleParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means, and it must be [`LFX_UNIT_DEGREES`].
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Padding to the pointer's alignment, spelled out rather than left to the
    /// compiler. Must be zero.
    pub reserved_0: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// The value a fresh instance starts at, in degrees.
    pub default_value: f64,
    /// The snapping increment while a modifier is held, in degrees.
    pub dial_step: f64,
}

/// A switch.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxBoolParam {
    /// `size_of::<LfxBoolParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// `0` or `1`; nothing else is a boolean.
    pub default_value: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
}

/// A dropdown, with its dividers **declared** rather than guessed from the
/// labels.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxChoiceParam {
    /// `size_of::<LfxChoiceParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// The option a fresh instance starts on.
    pub default_index: u32,
    /// How many options `options` holds, at most [`LFX_MAX_OPTIONS`].
    pub option_count: u32,
    /// How many indices `dividers_after` holds, at most [`LFX_MAX_DIVIDERS`].
    pub divider_count: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// `option_count` labels, in menu order.
    pub options: *const *const c_char,
    /// The indices after which the list draws a rule.
    pub dividers_after: *const u32,
}

/// Scene-linear RGBA, with the edit range declared per colour because a linear
/// value may exceed one or dip below nought.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxColourParam {
    /// `size_of::<LfxColourParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Padding to the pointer's alignment. Must be zero.
    pub reserved_0: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// The colour a fresh instance starts at, scene-linear.
    pub default_rgba: [f64; 4],
    /// The low end of each channel's edit range.
    pub range_min: f64,
    /// The high end of each channel's edit range.
    pub range_max: f64,
}

/// An integer seed. There is deliberately **no declared default**: the host
/// draws one from the fresh instance's own id, so two copies of a seeded effect
/// never wobble in sync.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxSeedParam {
    /// `size_of::<LfxSeedParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Padding to the pointer's alignment. Must be zero.
    pub reserved_0: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
}

/// A point, which becomes two rows: Lumit has deliberately no Point kind, and
/// the panel folds `<id>_x` and `<id>_y` back into one crosshair row.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxPoint2Param {
    /// `size_of::<LfxPoint2Param>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Padding to the pointer's alignment. Must be zero.
    pub reserved_0: u32,
    /// snake_case, stable; the rows are `<id>_x` and `<id>_y`.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// Where a fresh instance's point sits, across.
    pub default_x: f64,
    /// Where a fresh instance's point sits, down.
    pub default_y: f64,
    /// The slider travel both axes share.
    pub slider_min: f64,
    /// The other end of that travel.
    pub slider_max: f64,
}

/// Three rows: `<id>_x`, `<id>_y` and `<id>_z`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxPoint3Param {
    /// `size_of::<LfxPoint3Param>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Padding to the pointer's alignment. Must be zero.
    pub reserved_0: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// Where a fresh instance's point sits, across.
    pub default_x: f64,
    /// Where a fresh instance's point sits, down.
    pub default_y: f64,
    /// Where a fresh instance's point sits, in depth.
    pub default_z: f64,
    /// The slider travel all three axes share.
    pub slider_min: f64,
    /// The other end of that travel.
    pub slider_max: f64,
}

/// A **tone** curve: [`LFX_MIN_CURVE_POINTS`] to [`LFX_MAX_CURVE_POINTS`]
/// points in the unit square, static in version 1. A bezier *path* is
/// [`LFX_PARAM_PATH`] and is a different control.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxCurveParam {
    /// `size_of::<LfxCurveParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// How many points `points` holds: [`LFX_MIN_CURVE_POINTS`] to
    /// [`LFX_MAX_CURVE_POINTS`].
    pub point_count: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// `2 * point_count` floats, `x` then `y`, sorted by `x`.
    pub points: *const f32,
}

/// A file chosen from a dialog. The **payload rides beside the op**: at
/// `process` the host fills [`LfxValue`]'s `file.path` from the auxiliary slot
/// it loaded, because only the host knows which file actually opened.
///
/// *ponytail:* that slot is not built. See [`LFX_PARAM_FILE`] for what a vendor
/// gets until it is.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxFileParam {
    /// `size_of::<LfxFileParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// How many extensions `filter` holds, at most [`LFX_MAX_FILTERS`].
    pub filter_count: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
    /// Lower case, no dot: `cube`, `exr`.
    pub filter: *const *const c_char,
    /// What the dialog calls that set of extensions.
    pub filter_name: *const c_char,
}

/// A button: a row that asks the host to *do* something. No value, no keyframe,
/// and nothing in the value bag.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxActionParam {
    /// `size_of::<LfxActionParam>()` as the plugin was built.
    pub struct_size: u32,
    /// What the number means; [`LFX_UNIT_UNSET`] is a refusal.
    pub unit: LfxUnit,
    /// The departures from the ordinary row: static, hidden.
    pub flags: LfxParamFlags,
    /// Padding to the pointer's alignment. Must be zero.
    pub reserved_0: u32,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
}

/// A heading over the rows declared until the matching `group_end`. It is a
/// run, not a row: nothing is stored for it and nothing animates.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxGroupParam {
    /// `size_of::<LfxGroupParam>()` as the plugin was built.
    pub struct_size: u32,
    /// The departures from the ordinary run; [`LFX_PARAM_FLAG_HIDDEN`] hides
    /// the whole of it.
    pub flags: LfxParamFlags,
    /// snake_case, stable.
    pub id: *const c_char,
    /// Sentence case.
    pub label: *const c_char,
}

/// What `describe` pushes its declarations into: typed records, in the order
/// they should be drawn. There is no key, no string-valued answer, and no
/// question the host can ask that a plugin can answer in the wrong type.
///
/// Every call answers `true` when the declaration was accepted. `false` is the
/// graceful refusal of **that one row**: a kind this version does not admit
/// ([`LFX_PARAM_STRING`], [`LFX_PARAM_PATH`]), or a declaration the host cannot
/// represent. The plugin carries on declaring, each refusal is a line in the
/// scan report, the effect still loads, and the row keeps the default it was
/// declared with for the effect's life.
///
/// **Two faults are not that, and the return value does not distinguish them.**
/// A duplicate `id` and an unset `unit` are structural, and they refuse the
/// whole effect: two rows hashing to one parameter id would ship one control
/// silently driving another, and a unit cannot be guessed at all. A `false` for
/// either means the effect will not be catalogued whatever the plugin declares
/// next. Carrying on after a refusal is allowed and costs nothing; it is simply
/// not always enough to save the effect.
///
/// **Control thread only**, and only from inside [`LfxPlugin::describe`].
#[repr(C)]
pub struct LfxDescribeSink {
    /// `size_of::<LfxDescribeSink>()` as the host built it.
    pub struct_size: u32,
    /// The host's own, opaque. A plugin passes the sink back unchanged.
    pub sink_data: *mut c_void,
    /// Declare an unbounded number.
    pub declare_float:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxFloatParam) -> bool>,
    /// Declare a bounded number.
    pub declare_slider:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxSliderParam) -> bool>,
    /// Declare a whole number.
    pub declare_int: Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxIntParam) -> bool>,
    /// Declare an angle.
    pub declare_angle:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxAngleParam) -> bool>,
    /// Declare a switch.
    pub declare_bool:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxBoolParam) -> bool>,
    /// Declare a dropdown.
    pub declare_choice:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxChoiceParam) -> bool>,
    /// Declare a colour.
    pub declare_colour:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxColourParam) -> bool>,
    /// Declare a seed.
    pub declare_seed:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxSeedParam) -> bool>,
    /// Declare a point.
    pub declare_point2:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxPoint2Param) -> bool>,
    /// Declare a point in three dimensions.
    pub declare_point3:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxPoint3Param) -> bool>,
    /// Declare a tone curve.
    pub declare_curve:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxCurveParam) -> bool>,
    /// Declare a file.
    pub declare_file:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxFileParam) -> bool>,
    /// Declare a button.
    pub declare_action:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxActionParam) -> bool>,
    /// Open a run of rows under a heading.
    pub group_begin:
        Option<unsafe extern "C" fn(*mut LfxDescribeSink, *const LfxGroupParam) -> bool>,
    /// Close the run the last `group_begin` opened.
    pub group_end: Option<unsafe extern "C" fn(*mut LfxDescribeSink) -> bool>,
}

// ------------------------------------------------------------- the values --

/// The curve arm of [`LfxValuePayload`]: points in the unit square.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxCurveValue {
    /// `2 * n` floats, `x` then `y`.
    pub pt: *const f32,
    /// How many points.
    pub n: u32,
}

/// The file arm of [`LfxValuePayload`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxFileValue {
    /// The file the host actually loaded, filled from the auxiliary slot beside
    /// the op. Null when nothing loaded - which, until the generic file aux
    /// lands, is every File row (*ponytail:* see [`LFX_PARAM_FILE`]).
    pub path: *const c_char,
}

/// One resolved value, tagged by the kind its declaration minted.
#[repr(C)]
#[derive(Clone, Copy)]
pub union LfxValuePayload {
    /// [`LFX_PARAM_FLOAT`], [`LFX_PARAM_SLIDER`] and [`LFX_PARAM_ANGLE`].
    pub f: f64,
    /// [`LFX_PARAM_INT`] and [`LFX_PARAM_SEED`].
    pub i: i64,
    /// [`LFX_PARAM_BOOL`].
    pub b: bool,
    /// [`LFX_PARAM_CHOICE`]: the chosen index.
    pub choice: u32,
    /// [`LFX_PARAM_COLOUR`], scene-linear.
    pub rgba: [f32; 4],
    /// **Both** axes of a [`LFX_PARAM_POINT2`], carried in one element; the
    /// host's two rows are folded back here.
    pub xy: [f32; 2],
    /// All three axes of a [`LFX_PARAM_POINT3`], carried in one element.
    pub xyz: [f32; 3],
    /// [`LFX_PARAM_CURVE`].
    pub curve: LfxCurveValue,
    /// [`LFX_PARAM_FILE`].
    pub file: LfxFileValue,
}

/// One resolved control value at the frame being rendered.
///
/// **This is the one struct with no `struct_size`, and the stride is why.** The
/// values cross as a dense array in declaration order, addressed by index;
/// [`LfxProcess::value_stride`] is the number of bytes between two elements and
/// is what both sides walk by. A plugin that strides by its own `size_of` after
/// this struct has grown reads correct-looking kind tags over silently wrong
/// values, which no kind check can see.
///
/// A declaration the host carries no value for - an Action, a Group - is not in
/// the array at all.
///
/// **One declaration is one element**, including the ones the host spreads over
/// rows of its own: a [`LFX_PARAM_POINT2`] contributes a single element whose
/// `v.xy` carries both axes, and a [`LFX_PARAM_POINT3`] a single `v.xyz`. The
/// host's `<id>_x` / `<id>_y` rows are folded back before the array is written,
/// so the count here is the count of declarations and never the count of rows.
///
/// **A colour and a point are declared wide and arrive narrow.**
/// [`LfxColourParam`], [`LfxPoint2Param`] and [`LfxPoint3Param`] state their
/// defaults as `f64`, a declaration being read once; the host's resolved value
/// bag is single precision, so the narrowing happens on the way in and `rgba`,
/// `xy` and `xyz` are `f32`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LfxValue {
    /// Which declaration this is: its index among the declarations that carry a
    /// value, in declaration order - counting an Action or a Group not at all,
    /// and a point once. It is the element's own index too, so
    /// `values[i].param == i` always; it is carried so a plugin walking a
    /// strided array can check that it walked it correctly.
    pub param: u32,
    /// The tag, checked by the wrapper, which is what makes a kind mismatch
    /// impossible rather than a runtime status.
    pub kind: LfxParamKind,
    /// The value itself, read through the arm `kind` names.
    pub v: LfxValuePayload,
}

// -------------------------------------------------------------- the frame --

/// One picture crossing the boundary: scene-linear, premultiplied, tightly
/// packed, top-down. The host never converts a depth to accommodate a plugin.
///
/// **Both frames are the host's**, and in the shipping host each is a slot of a
/// shared-memory ring. The frame reached through [`LfxProcess::input`] is read
/// only for the duration of the call: writing through its `data` is undefined
/// however permissive the mapping happens to be, and corrupts whatever the host
/// does with that slot next. Only `output`'s `data` may be written, and only
/// from inside `process`.
///
/// Neither frame outlives the `process` call that carried it. A pointer kept
/// past the return names a slot the host has since given to something else.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LfxFrame {
    /// `size_of::<LfxFrame>()` as the host built it.
    pub struct_size: u32,
    /// The depth, which the host never converts to accommodate a plugin.
    pub format: LfxPixelFormat,
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// Bytes from the start of one row to the start of the next. Read it rather
    /// than computing it.
    pub row_bytes: u32,
    /// Where this buffer's top-left pixel sits, across, in the space the
    /// request's region is given in.
    pub origin_x: i32,
    /// Where this buffer's top-left pixel sits, down.
    pub origin_y: i32,
    /// Padding to the pointer's alignment. Must be zero.
    pub reserved_0: u32,
    /// The pixels.
    pub data: *mut c_void,
    /// The comp frame this picture is. Equal to [`LfxProcess::time`] for the
    /// effect's own input; a neighbour carries its own.
    pub time: f64,
}

/// One request to render one frame.
///
/// **Any worker thread**, and two instances of one plugin may be inside
/// `process` at once. One instance is never re-entered.
///
/// The request, its `values` array and both frames are valid **only for the
/// duration of the call**. A plugin may not keep the pointer and may not read
/// it from a thread of its own once `process` has returned - including to poll
/// `cancelled`, which answers only while the host is still waiting for this
/// frame.
#[repr(C)]
pub struct LfxProcess {
    /// `size_of::<LfxProcess>()` as the host built it.
    pub struct_size: u32,
    /// The depth, and it matches both frames.
    pub pixel_format: LfxPixelFormat,
    /// Bytes between two elements of `values`. **Index by this, never by
    /// `size_of::<LfxValue>()`.**
    ///
    /// At least `size_of::<LfxValue>()` as the host built it, and a multiple of
    /// that struct's alignment; `values` itself is aligned for it. So an
    /// element reached at `i * value_stride` is aligned, and a plugin reads it
    /// without `read_unaligned` and without faulting on a strict-alignment
    /// target.
    pub value_stride: u32,
    /// How many elements `values` holds.
    pub value_count: u32,
    /// The output region asked for, `x0` inclusive.
    pub roi_x0: i32,
    /// The output region asked for, `y0` inclusive.
    pub roi_y0: i32,
    /// The output region asked for, `x1` exclusive.
    pub roi_x1: i32,
    /// The output region asked for, `y1` exclusive.
    pub roi_y1: i32,
    /// Where the input has pixels at all, `x0` inclusive.
    pub dod_x0: i32,
    /// Where the input has pixels at all, `y0` inclusive.
    pub dod_y0: i32,
    /// Where the input has pixels at all, `x1` exclusive.
    pub dod_x1: i32,
    /// Where the input has pixels at all, `y1` exclusive.
    pub dod_y1: i32,
    /// The comp frame being rendered, as a decimal.
    pub time: f64,
    /// `value_count` elements, in declaration order, strided.
    pub values: *const LfxValue,
    /// The picture to read.
    pub input: *const LfxFrame,
    /// The picture to write.
    pub output: *mut LfxFrame,
    /// True once the host has stopped wanting this frame. Worth calling only if
    /// the effect declared [`LFX_TRAIT_CANCELLABLE`]; the honest answer to a
    /// true is [`LFX_STATUS_CANCELLED`], promptly. Never null.
    pub cancelled: Option<unsafe extern "C" fn(*const LfxProcess) -> bool>,
    /// The host's own, opaque, and the reason `cancelled` takes the request back
    /// rather than nothing.
    pub host_context: *mut c_void,
}

// --------------------------------------------------------------- the host --

/// What the plugin is handed: the mirror image of its own `get_extension`, and
/// somewhere to say something. That is the whole of it.
///
/// The pointer handed to [`LfxEntry::create`] is valid until `deinit` and **may
/// be kept**: it is what a plugin logs and fetches extensions through from
/// inside `process`. It is the only pointer in this ABI with that lifetime -
/// every other one a plugin is handed lives for the length of one call.
#[repr(C)]
pub struct LfxHost {
    /// `size_of::<LfxHost>()` as the host built it.
    pub struct_size: u32,
    /// The host's ABI version, which may be newer than the plugin's.
    pub abi_version: u32,
    /// The host's own, opaque; passed back on every call.
    pub host_data: *mut c_void,
    /// The typed table for `id` at `version`, or null. A missing extension is a
    /// null, never a status. **Any thread.**
    pub get_extension: Option<
        unsafe extern "C" fn(*const LfxHost, id: *const c_char, version: u32) -> *const c_void,
    >,
    /// One line of diagnostics, at most [`LFX_MAX_LOG_BYTES`] long. Not a
    /// user-facing message: the host's own sentences are translated and this is
    /// not. **Any thread**, and never from a signal handler.
    pub log:
        Option<unsafe extern "C" fn(*const LfxHost, level: LfxLogLevel, message: *const c_char)>,
}

// -------------------------------------------------------- the descriptor --

/// What one effect in the bundle *is*, answered without creating it.
///
/// The bundle states all of this in its manifest too, which the host reads
/// before it opens the module at all. The manifest is the cheap listing and
/// this struct is the truth; a disagreement once the module is open refuses the
/// plugin.
#[repr(C)]
pub struct LfxDescriptor {
    /// `size_of::<LfxDescriptor>()` as the plugin was built.
    pub struct_size: u32,
    /// Reverse-DNS, and stable for the plugin's life: it is half of the
    /// identity the host's frame keys are minted from.
    pub id: *const c_char,
    /// What a person reads in the Add-effect menu.
    pub name: *const c_char,
    /// Who to blame, shown in the row's context menu.
    pub vendor: *const c_char,
    /// The major version. All three numbers re-key the host's cached frames, so
    /// a release whose maths moved must move one of them.
    pub major: u32,
    /// The minor version; below 1000.
    pub minor: u32,
    /// The patch version; below 1000.
    pub patch: u32,
    /// The **first is the heading** the effect is browsed under; the rest are
    /// search keywords.
    pub categories: *const LfxCategory,
    /// How many categories `categories` holds: one to [`LFX_MAX_CATEGORIES`].
    pub category_count: u32,
    /// Null is the pessimistic case, and means it.
    pub traits: *const LfxTraits,
    /// The extensions without which this effect cannot work - the code's own
    /// answer, checked against what the manifest declared.
    pub required_extensions: *const *const c_char,
    /// How many extensions `required_extensions` holds, at most
    /// [`LFX_MAX_REQUIRED_EXTENSIONS`].
    pub required_extension_count: u32,
}

// ------------------------------------------------------------ the plugin --

/// One live effect.
///
/// The host creates one per in-flight frame from its instance pool, and a
/// plugin holds **no opaque state of its own**: the host's values are the only
/// truth, which is what makes a frame key complete and a restart an exact
/// replay.
///
/// `plugin_data` is not a contradiction of that. It is the instance's own
/// `this`: scratch, a working buffer, a table decoded once - anything
/// **derivable from the values in [`LfxProcess`] alone**. Nothing in it reaches
/// the host's frame key, so anything in it that can change the output is a
/// stale-frame bug: the host will serve a cached frame from before it changed,
/// and be right to. What "no opaque state" refuses is the host-persisted blob
/// OFX calls plugin state - there is nothing here the host saves for a plugin
/// and nothing it hands back, which is what makes a restart an exact replay.
#[repr(C)]
pub struct LfxPlugin {
    /// `size_of::<LfxPlugin>()` as the plugin was built.
    pub struct_size: u32,
    /// The instance's own, untouched by the host and invisible to it. Its
    /// contents must be derivable from the values `process` is handed.
    pub plugin_data: *mut c_void,
    /// Prepare this instance. Non-zero accepts it; zero refuses it, and the
    /// host badges the layer rather than failing the frame. A `u32` rather than
    /// a `bool` because it is the plugin's answer and not the host's - see the
    /// freeze note at the top of this file. **Control thread.**
    pub init: Option<unsafe extern "C" fn(*mut LfxPlugin) -> u32>,
    /// **Control thread**, and never while a `process` is running.
    pub destroy: Option<unsafe extern "C" fn(*mut LfxPlugin)>,
    /// Declare every control, in the order they should be drawn. Non-zero when
    /// the plugin declared what it meant to; zero refuses the effect, which the
    /// host reports by name. A `u32` for the reason `init` is.
    /// **Control thread.**
    pub describe: Option<unsafe extern "C" fn(*mut LfxPlugin, *mut LfxDescribeSink) -> u32>,
    /// Render one frame, answering an [`LfxStatus`]. **Any worker thread**, one
    /// call per instance at a time.
    pub process: Option<unsafe extern "C" fn(*mut LfxPlugin, *const LfxProcess) -> i32>,
    /// The plugin's side of the extension handshake: its typed table for `id` at
    /// `version`, or null. **Any thread.**
    pub get_extension: Option<
        unsafe extern "C" fn(*mut LfxPlugin, id: *const c_char, version: u32) -> *const c_void,
    >,
}

// ------------------------------------------------------------- the entry --

/// The one exported object, found under [`LFX_ENTRY_SYMBOL`]. Everything else
/// is reached from here.
#[repr(C)]
pub struct LfxEntry {
    /// `size_of::<LfxEntry>()` as the plugin was built.
    pub struct_size: u32,
    /// [`LFX_ABI_VERSION`] as this bundle was built against.
    pub abi_version: u32,
    /// Called once per process, **after** the manifest has been read and before
    /// anything else, with the bundle's own directory so a plugin can find its
    /// resources without guessing. Non-zero loads; zero declines, and the host
    /// says so without opening anything else. A `u32` for the reason
    /// [`LfxPlugin::init`] is. **Control thread.**
    pub init: Option<unsafe extern "C" fn(bundle_path: *const c_char) -> u32>,
    /// **Control thread**, once, last.
    pub deinit: Option<unsafe extern "C" fn()>,
    /// How many effects this bundle holds, at most
    /// [`LFX_MAX_EFFECTS_PER_BUNDLE`]. **Control thread.**
    pub count: Option<unsafe extern "C" fn() -> u32>,
    /// The descriptor at `index`, or null past the end. It stays valid and
    /// unchanged until `deinit`. **Control thread.**
    pub descriptor: Option<unsafe extern "C" fn(index: u32) -> *const LfxDescriptor>,
    /// One instance of the effect named `id`, or null. **Control thread.**
    pub create: Option<unsafe extern "C" fn(*const LfxHost, id: *const c_char) -> *mut LfxPlugin>,
}
