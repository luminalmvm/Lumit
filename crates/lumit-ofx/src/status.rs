//! The OFX status codes, as a typed enum.
//!
//! # In plain terms
//!
//! Every function in the OFX C API answers with a small whole number: nought
//! means it worked, and each other number is a different kind of "no". Rust
//! would rather work in named results than in numbers, so this module names
//! them once, converts a `Result` into a number at the very edge where the
//! plugin is waiting for one, and never lets a raw integer wander further in.

use std::ffi::c_int;
use std::sync::atomic::{AtomicU64, Ordering};

/// A status code as the OFX C API spells it: a plain `int`.
pub type OfxStatus = c_int;

/// The status codes from `ofxCore.h`, in their defined order (the numbers are
/// the API, so the discriminants are written out rather than left to chance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum Status {
    /// `kOfxStatOK`
    Ok = 0,
    /// `kOfxStatFailed` — the action was understood and did not work.
    Failed = 1,
    /// `kOfxStatErrFatal` — the plugin or host is unusable from here on.
    ErrFatal = 2,
    /// `kOfxStatErrUnknown`
    ErrUnknown = 3,
    /// `kOfxStatErrMissingHostFeature`
    ErrMissingHostFeature = 4,
    /// `kOfxStatErrUnsupported`
    ErrUnsupported = 5,
    /// `kOfxStatErrExists`
    ErrExists = 6,
    /// `kOfxStatErrFormat`
    ErrFormat = 7,
    /// `kOfxStatErrMemory`
    ErrMemory = 8,
    /// `kOfxStatErrBadHandle` — the handle we were given is not one of ours,
    /// is of the wrong kind, or names something that has been destroyed.
    ErrBadHandle = 9,
    /// `kOfxStatErrBadIndex`
    ErrBadIndex = 10,
    /// `kOfxStatErrValue` — including a property read at the wrong type.
    ErrValue = 11,
    /// `kOfxStatReplyYes`
    ReplyYes = 12,
    /// `kOfxStatReplyNo`
    ReplyNo = 13,
    /// `kOfxStatReplyDefault` — "you decide", which is what a host with no
    /// dialogue to show says to a question.
    ReplyDefault = 14,
}

impl Status {
    /// The code as the C API carries it.
    #[must_use]
    pub const fn code(self) -> OfxStatus {
        self as OfxStatus
    }

    /// Read a code that came back from a plugin. Anything outside the defined
    /// set is [`Status::ErrUnknown`] — plugins do return numbers nobody
    /// documented, and guessing at their meaning is worse than not knowing.
    #[must_use]
    pub const fn from_code(code: OfxStatus) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::Failed,
            2 => Self::ErrFatal,
            4 => Self::ErrMissingHostFeature,
            5 => Self::ErrUnsupported,
            6 => Self::ErrExists,
            7 => Self::ErrFormat,
            8 => Self::ErrMemory,
            9 => Self::ErrBadHandle,
            10 => Self::ErrBadIndex,
            11 => Self::ErrValue,
            12 => Self::ReplyYes,
            13 => Self::ReplyNo,
            14 => Self::ReplyDefault,
            _ => Self::ErrUnknown,
        }
    }
}

/// What every suite entry point works in internally: the unit answer or a
/// status. [`finish`] turns it into the number the plugin gets.
pub type StatusResult = Result<(), Status>;

/// Collapse an internal result into the C answer.
#[must_use]
pub fn finish(result: StatusResult) -> OfxStatus {
    match result {
        Ok(()) => Status::Ok.code(),
        Err(status) => status.code(),
    }
}

/// How many statuses there are, which is how many counters the tally keeps.
const STATUS_COUNT: usize = 15;

/// One counter per status, bumped as the host answers
/// (docs/impl/ofx-host.md §5).
///
/// # In plain terms
///
/// The conformance bench asks a hard question — *did any of the eighty plugins
/// make a suite call the host had to refuse?* — and the only place that can
/// answer it is the host itself, at the moment it hands the number back. A
/// plugin never reports its own bad handle; it swallows the code and carries
/// on, which is exactly how a host bug hides behind a picture that came out
/// looking right.
///
/// So every suite entry point tallies what it returned. Relaxed atomics and no
/// allocation: this is on the path of every property read a plugin makes, and
/// the tally must cost nothing worth measuring.
static ANSWERED: [AtomicU64; STATUS_COUNT] = [const { AtomicU64::new(0) }; STATUS_COUNT];
/// Answers that were the plugin's doing rather than the host's: see [`uncount`].
static FORGIVEN: [AtomicU64; STATUS_COUNT] = [const { AtomicU64::new(0) }; STATUS_COUNT];

/// Tally one answer. Called by the suites' guards and by nothing else.
pub(crate) fn record(code: OfxStatus) {
    let index = usize::try_from(code).unwrap_or(usize::MAX);
    if let Some(slot) = ANSWERED.get(index) {
        slot.fetch_add(1, Ordering::Relaxed);
    }
}

/// Take one answer of `code` back out of the tally.
///
/// For the one case where the code is the spec's answer and yet not the host's
/// refusal: a plugin handing the property suite a **null** handle (Red Giant
/// Universe reads `kOfxPropTime` off `createInstance`'s `inArgs`, which the
/// spec says is null). The plugin is told `kOfxStatErrBadHandle`, as it must
/// be, and carries on; counting that against the host would have the
/// conformance bench call a plugin's slip a host bug (K-757).
pub(crate) fn uncount(code: OfxStatus) {
    // Its own counter, subtracted on read: this runs inside the call, before
    // the guard records the answer, so taking one off the tally here would
    // find nothing to take.
    let index = usize::try_from(code).unwrap_or(usize::MAX);
    if let Some(slot) = FORGIVEN.get(index) {
        slot.fetch_add(1, Ordering::Relaxed);
    }
}

/// How many times the host has answered `status` since the last [`forget`].
#[must_use]
pub fn answered(status: Status) -> u64 {
    let index = usize::try_from(status.code()).unwrap_or(usize::MAX);
    let count = |table: &[AtomicU64; STATUS_COUNT]| {
        table
            .get(index)
            .map_or(0, |slot| slot.load(Ordering::Relaxed))
    };
    count(&ANSWERED).saturating_sub(count(&FORGIVEN))
}

/// Start the tally again from nought — what a harness does before the pass it
/// wants the count for.
pub fn forget() {
    for slot in ANSWERED.iter().chain(&FORGIVEN) {
        slot.store(0, Ordering::Relaxed);
    }
}
