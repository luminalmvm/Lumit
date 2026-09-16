//! `lfx` - a safe Rust wrapper over the frozen LFX C ABI.
//!
//! # In plain terms
//!
//! `lfx-sys` is the agreement; this crate is the part of it a Rust author
//! would otherwise write again in every plugin. Implement one trait, list your
//! effects in one macro, and the entry table, the instance lifetime, the
//! strided value array and both pixel depths are somebody else's problem:
//!
//! ```ignore
//! #[derive(Default)]
//! struct Gain;
//!
//! impl lfx::Effect for Gain {
//!     const SPEC: lfx::Spec = lfx::Spec {
//!         id: c"com.example.lfx.gain",
//!         name: c"Gain",
//!         vendor: c"Example",
//!         version: (1, 0, 0),
//!         categories: &[lfx::Category::Colour],
//!         traits: lfx::Traits::per_pixel(lfx::Cost::Cheap),
//!         required_extensions: &[],
//!     };
//!
//!     fn describe(&mut self, sink: &mut lfx::Describe<'_>) -> bool {
//!         sink.slider(c"gain", c"Gain", lfx::Unit::Raw, 1.0, 0.0, 4.0);
//!         true
//!     }
//!
//!     fn process(&mut self, call: &lfx::Request<'_>) -> lfx::Status {
//!         let gain = call.values().number(0, 1.0) as f32;
//!         call.for_each_pixel(|_, _, [r, g, b, a]| [r * gain, g * gain, b * gain, a])
//!     }
//! }
//!
//! lfx::bundle!(Gain);
//! ```
//!
//! # What it does not hide
//!
//! Four of the header's rules are the author's whatever wrapper they use, and
//! this crate states them rather than pretending otherwise.
//!
//! **Nothing is kept between frames.** An effect is an ordinary Rust value and
//! may hold scratch, but everything in it must be derivable from the values
//! [`Request`] carries. Anything else is a stale-frame bug: the host's frame
//! key is minted from the values, so it will serve a cached picture from
//! before the change, and be right to.
//!
//! **`process` may run on any worker thread**, and on two instances of one
//! effect at once. One instance is never re-entered, which is why `process`
//! takes `&mut self` and why nothing here needs a lock. An effect that cannot
//! bear even that declares [`Traits::thread_unsafe`] and is serialised for the
//! whole bundle.
//!
//! **Both depths are mandatory.** fp16 and fp32 are the project's choice and
//! never the plugin's; [`Request::for_each_pixel`] converts, so the maths is
//! written once in `f32`.
//!
//! **A panic must not cross the boundary.** Unwinding into C is undefined, so
//! every callback this crate hands the host catches one and answers
//! [`Status::Failed`] instead. That is a net, not a licence: a plugin that
//! panics is a plugin whose layer wears a badge.
//!
//! # Licence
//!
//! MIT, as `lfx-sys` and the header are.

#![deny(missing_docs)]

mod describe;
mod entry;
mod halves;
mod request;

pub use describe::Describe;
pub use entry::{registration, Registration, Table};
pub use halves::{from_half, to_half};
pub use request::{Rect, Request, Values};

/// The raw declarations, re-exported so a plugin needs one dependency rather
/// than two and cannot end up compiled against two different ABIs.
pub use lfx_sys as sys;

use std::ffi::CStr;

/// What `process` answers.
///
/// A typed refusal, never a message: the host turns each of these into a
/// sentence of its own in the user's language.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum Status {
    /// The frame was rendered.
    Ok = sys::LFX_STATUS_OK,
    /// This frame could not be produced. The host renders the input unchanged
    /// and badges the layer.
    Failed = sys::LFX_STATUS_FAILED,
    /// The host asked for the work to stop and the effect obliged.
    Cancelled = sys::LFX_STATUS_CANCELLED,
    /// The effect ran out of memory.
    OutOfMemory = sys::LFX_STATUS_OUT_OF_MEMORY,
    /// The request carried something this build does not do - a format, a
    /// region. Both depths are mandatory, so this is never the answer to fp16.
    Unsupported = sys::LFX_STATUS_UNSUPPORTED,
}

/// What a number *means*. Mandatory on every declaration a kind does not fix:
/// "dimensionless" and "nobody decided" must not look alike.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Unit {
    /// A plain number: a gamma, a count, a threshold.
    Raw = sys::LFX_UNIT_RAW,
    /// 100 is the whole of whatever it is a share of.
    Percent = sys::LFX_UNIT_PERCENT,
    /// **Pixels at composition size**, never pixels of whatever buffer the
    /// effect was handed. The host converts to the raster in play.
    Px = sys::LFX_UNIT_PX,
    /// Degrees. An angle declares this and nothing else.
    Degrees = sys::LFX_UNIT_DEGREES,
    /// Seconds of layer time.
    Seconds = sys::LFX_UNIT_SECONDS,
    /// Comp-rate frames.
    Frames = sys::LFX_UNIT_FRAMES,
}

/// The picture families an effect may claim, closed and frozen.
///
/// The first declared is the heading the effect is browsed under; the rest are
/// search keywords. An author may not invent one, which is what lets an LFX
/// effect sit under Lumit's own headings rather than in a plugin ghetto.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Category {
    /// Blur and sharpen.
    BlurSharpen = sys::LFX_CATEGORY_BLUR_SHARPEN,
    /// Colour correction and grading.
    Colour = sys::LFX_CATEGORY_COLOUR,
    /// Distortion and warping.
    Distortion = sys::LFX_CATEGORY_DISTORTION,
    /// Generators.
    Generate = sys::LFX_CATEGORY_GENERATE,
    /// Stylise.
    Stylise = sys::LFX_CATEGORY_STYLISE,
    /// Time-based effects.
    Temporal = sys::LFX_CATEGORY_TEMPORAL,
    /// Transitions.
    Transition = sys::LFX_CATEGORY_TRANSITION,
    /// Everything else.
    Utility = sys::LFX_CATEGORY_UTILITY,
}

/// What one frame of this effect costs, read by the scheduler and the
/// degradation ladder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Cost {
    /// A copy, near enough.
    Trivial = sys::LFX_COST_TRIVIAL,
    /// A per-pixel pass.
    Cheap = sys::LFX_COST_CHEAP,
    /// Several passes, or a small kernel.
    Moderate = sys::LFX_COST_MODERATE,
    /// Expensive enough that the host should schedule around it.
    Heavy = sys::LFX_COST_HEAVY,
}

/// How far past an output pixel the effect reads.
///
/// Claiming less reach than the kernel uses produces tile seams, which is a
/// correctness bug rather than a slow render - so the unstated answer is the
/// expensive one, and [`Roi::Exact`] is a promise rather than a hint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Roi {
    /// The output pixel and nothing around it.
    Exact,
    /// The output pixel and this many pixels around it, in px@comp, sized from
    /// the effect's own hard maximum.
    Padded(f32),
    /// The whole frame, whatever region was asked for.
    FullFrame,
}

/// Which alpha the effect's maths expects. Lumit's own working form is
/// premultiplied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Alpha {
    /// The working form.
    Premultiplied,
    /// Straight, and the host undoes and redoes the multiply around the call.
    Straight,
}

/// What the host schedules from, declared once and never again.
///
/// Every field has a deliberate value here because every zero in the frozen
/// trait block is a *declaration* - of the pessimistic case. A block left
/// blank schedules as heavy, full-frame work.
#[derive(Clone, Copy, Debug)]
pub struct Traits {
    /// What one frame costs.
    pub cost: Cost,
    /// How far past an output pixel the effect reads.
    pub roi: Roi,
    /// The frame window the effect reads, in comp frames, relative to the one
    /// being rendered. **This is the gate**: an effect that declares no window
    /// never sees a neighbour. Each end reaches at most
    /// [`sys::LFX_MAX_TEMPORAL_WINDOW`], and the window must contain nought.
    pub window: (i32, i32),
    /// Which alpha the maths expects.
    pub alpha: Alpha,
    /// The effect reads its Seed row and must stay bit-identical between two
    /// exports of the same project.
    pub seeded: bool,
    /// The sole, discouraged opt-out from instance-level concurrency: the host
    /// serialises the whole bundle.
    pub thread_unsafe: bool,
    /// [`Request::cancelled`] is worth calling: the effect polls it and
    /// answers [`Status::Cancelled`] promptly.
    pub cancellable: bool,
    /// The working memory one megapixel of output costs. LFX has no host
    /// allocator, so this declaration is the ceiling the resource ledger is
    /// asked for.
    pub scratch_bytes_per_megapixel: u32,
}

impl Traits {
    /// The traits of an effect that reads the output pixel and nothing else:
    /// exact reach, this frame only, premultiplied, cancellable, no scratch.
    ///
    /// The commonest honest declaration there is, and the one a first plugin
    /// should start from.
    #[must_use]
    pub const fn per_pixel(cost: Cost) -> Self {
        Self {
            cost,
            roi: Roi::Exact,
            window: (0, 0),
            alpha: Alpha::Premultiplied,
            seeded: false,
            thread_unsafe: false,
            cancellable: true,
            scratch_bytes_per_megapixel: 0,
        }
    }

    /// This declaration as the frozen block the host reads.
    fn lowered(&self) -> sys::LfxTraits {
        let mut flags = sys::LFX_TRAIT_NONE;
        if self.seeded {
            flags |= sys::LFX_TRAIT_SEEDED;
        }
        if self.thread_unsafe {
            flags |= sys::LFX_TRAIT_THREAD_UNSAFE;
        }
        if self.cancellable {
            flags |= sys::LFX_TRAIT_CANCELLABLE;
        }
        sys::LfxTraits {
            struct_size: size_of::<sys::LfxTraits>() as u32,
            cost: self.cost as u32,
            roi_kind: match self.roi {
                Roi::Exact => sys::LFX_ROI_EXACT,
                Roi::Padded(_) => sys::LFX_ROI_PADDED,
                Roi::FullFrame => sys::LFX_ROI_FULL_FRAME,
            },
            roi_padding_px: match self.roi {
                Roi::Padded(px) => px,
                Roi::Exact | Roi::FullFrame => 0.0,
            },
            temporal_lo: self.window.0,
            temporal_hi: self.window.1,
            alpha: match self.alpha {
                Alpha::Premultiplied => sys::LFX_ALPHA_PREMULTIPLIED,
                Alpha::Straight => sys::LFX_ALPHA_STRAIGHT,
            },
            flags,
            scratch_bytes_per_megapixel: self.scratch_bytes_per_megapixel,
        }
    }
}

/// What one effect in the bundle *is*, answered without creating it.
///
/// The bundle's `Contents/lfx.toml` states all of this as well - that is the
/// cheap listing the host reads with the module shut - and a disagreement
/// between the two once the module is open refuses the effect. The code is the
/// truth; the listing is the thing to keep in step with it.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// Reverse-DNS, stable for the effect's life: it is half of the identity
    /// the host's frame keys are minted from.
    pub id: &'static CStr,
    /// What a person reads in the Add-effect menu.
    pub name: &'static CStr,
    /// Who to blame, shown in the row's context menu.
    pub vendor: &'static CStr,
    /// Major, minor and patch. **All three re-key the host's cached frames**,
    /// so a release whose maths moved must move one of them. Minor and patch
    /// are below 1000.
    pub version: (u32, u32, u32),
    /// The first is the heading; the rest are search keywords.
    pub categories: &'static [Category],
    /// What the host schedules from.
    pub traits: Traits,
    /// The extensions without which this effect cannot work. An effect that
    /// asks for one the host has not got is refused before it is instantiated,
    /// with the extension named - so an empty list is the one that always
    /// loads. Version 1 of the host offers no extension table at all.
    pub required_extensions: &'static [&'static CStr],
}

/// One effect somebody wrote.
///
/// The host creates one of these per in-flight frame, from a pool of its own,
/// and destroys them when the row goes away. `Default` is how one is made:
/// there is nothing to configure at birth, because the values arrive with
/// every frame.
pub trait Effect: Default + 'static {
    /// What this effect is.
    const SPEC: Spec;

    /// Declare every control, in the order they should be drawn.
    ///
    /// Answer `false` to refuse the effect outright, which the host reports by
    /// name. A single declaration the host would not take is not that: it
    /// answers `false` from the [`Describe`] method itself, the row keeps the
    /// default declared here, and the effect still loads.
    ///
    /// **Control thread.**
    fn describe(&mut self, sink: &mut Describe<'_>) -> bool;

    /// Render one frame.
    ///
    /// **Any worker thread**, and two instances of one effect may be in here
    /// at once. This instance is never re-entered.
    fn process(&mut self, call: &Request<'_>) -> Status;
}

/// Declare the bundle's entry table from a list of [`Effect`] types.
///
/// It goes at the top level of the crate root, once per bundle, and it is the
/// whole of what a plugin owes the loader:
///
/// ```ignore
/// lfx::bundle!(Exposure, Vignette);
/// ```
#[macro_export]
macro_rules! bundle {
    ($($effect:ty),+ $(,)?) => {
        /// The bundle's table, built once on the first ask and never changed
        /// after: the header says a descriptor stays valid and unchanged until
        /// `deinit`.
        #[doc(hidden)]
        fn __lfx_table() -> &'static $crate::Table {
            static TABLE: ::std::sync::OnceLock<$crate::Table> = ::std::sync::OnceLock::new();
            TABLE.get_or_init(|| {
                $crate::Table::of(::std::vec![$($crate::registration::<$effect>()),+])
            })
        }

        /// The one exported object, under the name the header gives the
        /// loader.
        #[no_mangle]
        #[allow(non_upper_case_globals)]
        pub static lfx_entry_point: $crate::sys::LfxEntry = $crate::sys::LfxEntry {
            struct_size: ::core::mem::size_of::<$crate::sys::LfxEntry>() as u32,
            abi_version: $crate::sys::LFX_ABI_VERSION,
            init: Some(__lfx_entry_init),
            deinit: Some(__lfx_entry_deinit),
            count: Some(__lfx_entry_count),
            descriptor: Some(__lfx_entry_descriptor),
            create: Some(__lfx_entry_create),
        };

        #[doc(hidden)]
        unsafe extern "C" fn __lfx_entry_init(_bundle_path: *const ::core::ffi::c_char) -> u32 {
            1
        }

        #[doc(hidden)]
        unsafe extern "C" fn __lfx_entry_deinit() {}

        #[doc(hidden)]
        unsafe extern "C" fn __lfx_entry_count() -> u32 {
            __lfx_table().count()
        }

        #[doc(hidden)]
        unsafe extern "C" fn __lfx_entry_descriptor(
            index: u32,
        ) -> *const $crate::sys::LfxDescriptor {
            __lfx_table().descriptor(index)
        }

        #[doc(hidden)]
        unsafe extern "C" fn __lfx_entry_create(
            host: *const $crate::sys::LfxHost,
            id: *const ::core::ffi::c_char,
        ) -> *mut $crate::sys::LfxPlugin {
            // SAFETY: the host's contract - `id` is null or a NUL-terminated
            // string valid for the call, and `host` is null or a host table
            // valid until `deinit`.
            unsafe { __lfx_table().create(host, id) }
        }
    };
}
