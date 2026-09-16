//! The instance pool, and the first written-down concurrency policy
//! (docs/impl/lfx.md §4.4).
//!
//! # In plain terms
//!
//! A plugin's `process` may be called from any worker thread, and on different
//! instances of one plugin at once - but **one instance is never re-entered**
//! (§2.6). So a row whose frames are being rendered two at a time needs two
//! live plugin instances, and a row rendering one frame at a time needs one.
//! This is what keeps that count: a frame asks for an instance, renders with
//! it, and hands it back.
//!
//! The instances are interchangeable because an LFX plugin holds no opaque
//! state (D8) - everything it reads arrives in the dense value array - so a
//! frame that leases one tells it what its controls hold and is then rendering
//! with the numbers it meant. Telling it and asking it are one turn by
//! construction rather than by a lock: a leased instance is not visible to any
//! other frame, so nothing can land between the two.
//!
//! # The policy, and why its numbers are provisional
//!
//! docs/12 §3.4 defers to an adaptive-concurrency policy stated nowhere and
//! docs/13 names no instance-pool rule, so §4.4 writes the first one down and
//! says outright that **the numbers exist to be measured, not defended**: a
//! limit nobody can justify is a limit somebody will raise the first time it
//! fires. What is here is that policy, entire:
//!
//! * One live instance per in-flight frame, keyed by the effect instance and
//!   leased for the length of one frame.
//! * The pool grows by one when it is **saturated**, [`Ledger::pressure`] on
//!   [`Tier::Ram`] reads [`Pressure::Easy`], and measured per-frame throughput
//!   **rose** on the last growth - [`Throughput`] is that measurement.
//! * Capped at the render worker count, at [`MAX_POOL_INSTANCES`], and at what
//!   the ring the ledger actually granted will hold in flight at once - where
//!   *in flight* is counted in the slots **this** plugin's frames ship, its
//!   declared window and the one being written.
//! * [`Pressure::should_trim`] - Severe or Full - collapses it to one.
//! * `lfx.thread-unsafe` pins it to one, **bundle-wide**, behind a single
//!   [`Serial`] lock: the bundle's lock is armed from the whole described list
//!   ([`Serial::for_bundle`]) rather than from each plugin's own trait block,
//!   because a bundle where one plugin declares it and the next does not is
//!   exactly the shape the header's promise is about.
//! * A frame whose declared `scratch_bytes_per_megapixel` the ledger will not
//!   grant is **not dispatched** (§8), which is the ceiling LFX has in place of
//!   OFX's `memoryAlloc`.
//!
//! # What the ledger refuses, and what it merely pays for
//!
//! Two of those read the ledger and they read it differently. The scratch is a
//! real [`Reservation`], held for the length of the frame and given back by its
//! own destructor, because it stands for memory the plugin is about to
//! allocate in another process. Growth is not a reservation at all: the bytes
//! an extra in-flight frame costs were bought once, by the ring
//! ([`Ring::create`](crate::ipc::ring::Ring::create)), and charging for them a
//! second time here would be double counting the same slots. So growth reads
//! the ring's **granted** slot count - what the ledger was willing to pay for,
//! read as it stands rather than as it stood when the definition was built,
//! since the broker replaces the ring under a frame bigger than a slot - and
//! never asks for a slot the ring has not got. That is §4.4's "refused by the
//! ledger rather than discovered by an allocator", with the refusal where the
//! money was actually spent.
//!
//! **And the frame's own reservation is taken after the queue, not before
//! it.** A frame that waits - behind the bundle's lock, or at its row's
//! ceiling - is a frame that is not allocating anything yet, and one holding
//! its scratch while it waited would raise the very pressure the pool reads to
//! decide whether it may grow: the queue would make itself longer and then
//! badge a later frame for the congestion. What *is* answered before the wait
//! is the question the queue cannot change - whether the tier's whole budget
//! would cover this frame at all - so a frame the machine has not got is still
//! refused without waiting on anybody.
//!
//! # No lock of this module's is held across somebody else's code
//!
//! Opening an instance, telling it its values and closing it all reach another
//! process, and [`LfxDef::frames_needed`](crate::LfxDef::frames_needed) reads
//! this table once per live effect per frame on the frame-key walk - a walk
//! that may not wait behind a render. So the rows are locked for the
//! bookkeeping and unlocked before any of it runs. The one lock that *is* held
//! across a plugin's code is [`Serial`], and that is its whole purpose.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use lumit_budget::{Ledger, Pressure, Reservation, Tier};
use lumit_lfx_abi::LFX_TRAIT_THREAD_UNSAFE;
use thiserror::Error;
use uuid::Uuid;

use crate::def::LfxInstances;
use crate::describe::Traits;
use crate::ipc::broker::BrokerError;
use crate::ipc::proto::{InstanceId, ParamValue};
use crate::ipc::ring::RingSlots;

/// The most live instances one row may ever hold, whatever the machine has.
///
/// **Provisional** (§4.4). Sixteen frames of one row in flight at once is more
/// than any render this project schedules today, and the number is here so that
/// a machine with ninety-six cores does not answer "ninety-six" to a question
/// nobody has measured.
pub const MAX_POOL_INSTANCES: usize = 16;

/// The most rows one definition keeps instances for before the least recently
/// used of them are closed.
///
/// **Provisional**, and the answer to a ceiling `LfxDef` left standing: nothing
/// evicted a row that had left the layer, the shipping definition is leaked so
/// [`Drop`] never ran either, and a session of add-and-delete churn therefore
/// walked towards
/// [`MAX_LIVE_INSTANCES`](crate::ipc::broker::MAX_LIVE_INSTANCES) - the 1,024
/// live instances a bundle's broker refuses past - after which every new row of
/// that plugin badged rather than rendered until Lumit was restarted.
/// Sixty-four rows of up to [`MAX_POOL_INSTANCES`] each is that ceiling **for a
/// bundle of one plugin**, which is what this narrows: the 1,024 is the
/// broker's, so it belongs to the bundle, and a [`Pool`] belongs to one
/// [`LfxDef`](crate::LfxDef) - that is, to one plugin of it.
///
/// *ponytail:* a bundle of eight much-used effects can therefore still reach
/// eight times this and be refused at the broker. Closing it wants one pool per
/// **broker**, keyed by plugin id and row, which is also where the bundle's
/// [`Serial`] belongs; scaling this number by the bundle's plugin count only
/// moves the arithmetic, since a bundle may describe
/// `LFX_MAX_EFFECTS_PER_BUNDLE` of them and the first instance of a row can
/// never be refused. What is true today is that one plugin's churn is bounded
/// and a bundle's is bounded by the number of plugins in it.
///
/// A row's instances are evicted only while **nothing of it is leased**, so
/// eviction never races a frame; a frame that comes back to an evicted row
/// opens an instance again, which costs one round trip and no picture. What it
/// does not cost is the picture: the row keeps its remembered `frames_needed`,
/// which the frame key is computed over - see [`MAX_POOL_MEMOS`].
pub const MAX_POOL_ROWS: usize = 64;

/// The most rows one definition remembers a temporal window for.
///
/// Eviction closes instances; it deliberately does **not** forget what a row's
/// last render said it reads. `frames_needed` is read by the frame-key walk
/// ([`LfxDef::frames_needed`](crate::LfxDef::frames_needed)), and a row that
/// answered `None` because its instances had been closed would be keyed,
/// prefetched and rendered against the *declared* window instead - which is a
/// different picture rather than a round trip, and which of a project's rows
/// lost their window would depend on the order threads happened to lease in.
///
/// So the memo outlives the instance and is bounded on its own, larger count.
/// A memo is at most `2 × LFX_MAX_TEMPORAL_WINDOW + 1` offsets - 129 `i32`s -
/// so the whole table's worst case is well under a megabyte, and a thousand
/// rows of one plugin is already more than any project this host has seen.
pub const MAX_POOL_MEMOS: usize = 1_024;

/// How many frames a grown pool must complete before it may grow again.
///
/// **Provisional.** Growth is allowed on the strength of a measurement, and a
/// measurement over one frame is a measurement of that frame's picture rather
/// than of the pool. Eight is enough for the mean to mean something and short
/// enough that a pool which really should be wider gets there inside a second.
const SAMPLE_FRAMES: u64 = 8;

/// One megapixel, as the declared scratch counts them.
const MEGAPIXEL: u64 = 1_048_576;

// ------------------------------------------------------------- the refusals --

/// Why a frame has no live instance to render with.
///
/// Typed rather than a sentence, as docs/14 §4 requires: the two are different
/// refusals with different fixes - one is the plugin or its broker saying no,
/// and the other is this host declining to dispatch a frame whose declared
/// working memory the machine has not got.
#[derive(Debug, Error)]
pub enum PoolError {
    /// The driver refused: switched off, put away after three strikes, out of
    /// handles, or the broker itself.
    #[error(transparent)]
    Broker(#[from] BrokerError),
    /// The frame's declared scratch is more than the ledger will grant, so it
    /// is not dispatched at all (§8). The alternative is dispatching it and
    /// having the plugin discover the refusal with an allocator, in a process
    /// with no way to say so.
    #[error("the plugin declared {wanted} bytes of working memory for this frame, which the memory budget would not grant")]
    Scratch {
        /// What this frame's area and the declared rate came to.
        wanted: u64,
    },
}

// --------------------------------------------------------------- the policy --

/// The one lock a **thread-unsafe** bundle's plugins share, and whether the
/// bundle armed it.
///
/// `lfx.thread-unsafe` serialises the bundle rather than the plugin (§2.6) -
/// `lfx.h` says so beside the constant and §4.4 says "pins it to one,
/// bundle-wide" - so **the arming is a property of the bundle too**. It is read
/// off the whole described list once, by [`Serial::for_bundle`], and every
/// definition built over that bundle's broker is handed the same value. A
/// bundle where one plugin declares the bit and the next does not is the only
/// shape that tells bundle-wide from per-plugin, and it is the shape the
/// promise is about: the plugin that declared nothing shares one process, one
/// broker and one ring with the plugin that did, and letting its frames run
/// beside them is exactly the re-entrancy the declaration was made to prevent.
///
/// A bundle that never declares it never takes the lock, which is the common
/// case and costs it nothing.
#[derive(Clone, Debug, Default)]
pub struct Serial {
    lock: Arc<Mutex<()>>,
    /// Whether any plugin of the bundle declared `lfx.thread-unsafe`.
    armed: bool,
}

impl Serial {
    /// A lock no plugin has armed - a bundle that declared nothing of the sort.
    ///
    /// [`Serial::for_bundle`] is what a bundle's own lock is built with; this
    /// is for a caller that has no described list to read, and it declares, on
    /// the caller's behalf, that nothing in the bundle asked to be serialised.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The lock one bundle's definitions share, armed where **any** plugin of
    /// the bundle declared `lfx.thread-unsafe`.
    ///
    /// `flags` is every described plugin's trait flags, in any order: the raw
    /// `lfx_traits.flags` the broker read, or a host-side [`Traits`]'s own. A
    /// plugin with no trait block at all declares nothing, including this, so
    /// it contributes nought (§4.4's "a zero in the trait block does not pin the
    /// pool").
    #[must_use]
    pub fn for_bundle(flags: impl IntoIterator<Item = lumit_lfx_abi::LfxTraitFlags>) -> Self {
        let armed = flags
            .into_iter()
            .any(|declared| declared & LFX_TRAIT_THREAD_UNSAFE != 0);
        Self {
            lock: Arc::new(Mutex::new(())),
            armed,
        }
    }

    /// Whether the bundle this lock belongs to declared `lfx.thread-unsafe`.
    #[must_use]
    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    /// Hold the bundle, whatever a poisoned lock says - the same reasoning the
    /// rest of this crate gives for its own tables. A thread that panicked
    /// while holding it left nothing behind but the lock, and refusing to
    /// render the bundle ever again is the worse answer.
    fn hold(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Every ceiling §4.4 names, and the ledger the growth is read against.
///
/// One value rather than six fields on the pool, because it is the thing most
/// likely to change: the numbers are provisional, and this is where they are.
#[derive(Clone, Debug)]
pub struct Policy {
    /// The least of the render worker count and [`MAX_POOL_INSTANCES`] - the
    /// two ceilings that are the machine's rather than the bundle's, and the
    /// only two that do not move under a running session.
    workers: usize,
    /// What the bundle's ring holds **now**, read rather than copied: the
    /// broker replaces the ring under the first frame bigger than a slot, and a
    /// bigger frame buys fewer slots.
    slots: RingSlots,
    /// How many of those slots one frame of this plugin ships: every frame its
    /// declared window says it reads, plus the one being written. Two for a
    /// plugin that declared no window, which is the common case.
    slots_per_frame: u32,
    /// `Some` when the bundle declared `lfx.thread-unsafe`, in which case the
    /// ceiling is one as well and every lease takes this lock.
    serial: Option<Serial>,
    /// The governor's, for the pressure read at every lease and the scratch
    /// reservation held across every frame.
    ledger: Arc<Ledger>,
    /// What one megapixel of output costs the plugin in working memory, as it
    /// declared it. Nought is a plugin that declared none, and asks the ledger
    /// for nothing.
    scratch_bytes_per_megapixel: u64,
}

impl Policy {
    /// The policy for one described plugin of one bundle.
    ///
    /// `slots` is what the bundle's broker actually mapped -
    /// [`Broker::granted_slots`](crate::ipc::broker::Broker::granted_slots) -
    /// rather than what it asked for, which is the half of §4.4's ledger clause
    /// that is a number rather than a reservation. It arrives as a handle
    /// rather than a number because the count changes under a running session:
    /// a definition that copied sixty-four at the scan would keep a ceiling of
    /// sixteen after the first 4K fp32 frame narrowed the ring to its floor,
    /// and would keep it in the permissive direction.
    ///
    /// **What one frame costs is the plugin's own**, not two slots for
    /// everybody. The broker charges a shipment `neighbours + 2` slots and
    /// sizes the whole ring as `hi − lo + 2` - every frame the declared window
    /// says it reads, plus the one being written - so a plugin declaring ±5
    /// gets a twelve-slot ring that carries exactly one frame. Dividing by two
    /// there would read it as room for six, and the ring would refuse the sixth
    /// shipment with the plugin's name on it.
    ///
    /// `serial` is the **bundle's** lock and carries the bundle's own answer to
    /// `lfx.thread-unsafe` ([`Serial::for_bundle`]); this plugin's own bit is
    /// read beside it, so a lock somebody armed from a shorter list than the
    /// bundle still pins the plugin that declared. A `None` trait block
    /// declares nothing, including this: the pessimistic lowering
    /// [`crate::schema::traits_of`] makes is about cost and reach, where a
    /// wrong guess is a slow render or a tile seam. Reading an absent
    /// declaration as thread-unsafe would make the pessimistic case the
    /// *serial* case - a performance answer to a correctness question, and one
    /// that would collapse the pool for the commonest declaration there is.
    #[must_use]
    pub fn declared(
        traits: Option<&Traits>,
        slots: &RingSlots,
        ledger: &Arc<Ledger>,
        serial: &Serial,
    ) -> Self {
        let pinned = serial.is_armed()
            || traits.is_some_and(|declared| declared.flags & LFX_TRAIT_THREAD_UNSAFE != 0);
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        // The same sum the evaluation pool spends, rather than a second opinion
        // about how many threads this machine renders with - which is the
        // reasoning `lumit-ofx` gives for answering `multiThreadNumCPUs` with
        // it.
        let workers = lumit_eval::pool::worker_threads(cores);
        Self {
            workers: workers.min(MAX_POOL_INSTANCES),
            slots: slots.clone(),
            slots_per_frame: slots_per_frame(traits),
            serial: pinned.then(|| serial.clone()),
            ledger: Arc::clone(ledger),
            scratch_bytes_per_megapixel: traits.map_or(0, |declared| {
                u64::from(declared.scratch_bytes_per_megapixel)
            }),
        }
    }

    /// The most instances this policy would allow one row as things stand,
    /// pressure aside: the least of the three ceilings §4.4 names, and never
    /// nought - a pool of no instances renders nothing.
    #[must_use]
    pub fn ceiling(&self) -> usize {
        if self.serial.is_some() {
            return 1;
        }
        let in_flight = (self.slots.get() / self.slots_per_frame) as usize;
        self.workers.min(in_flight).max(1)
    }

    /// Whether this pool is pinned behind the bundle's lock.
    #[must_use]
    pub const fn is_serial(&self) -> bool {
        self.serial.is_some()
    }

    /// The ceiling as it stands, which is one while the tier is asking
    /// everybody to trim.
    fn ceiling_now(&self) -> usize {
        if self.ledger.pressure(Tier::Ram).should_trim() {
            1
        } else {
            self.ceiling()
        }
    }

    /// Whether the tier is quiet enough to grow into.
    ///
    /// `Easy` and not merely "not trimming": §4.4 names the one rung, and a
    /// pool that grew at Tight would be adding in-flight frames to a tier
    /// already three quarters spent.
    fn room_to_grow(&self) -> bool {
        self.ledger.pressure(Tier::Ram) == Pressure::Easy
    }
}

/// How many ring slots one frame of this plugin ships: the frames its declared
/// window reads, and the one being written.
///
/// The declaration is held the way
/// [`DeclaredTraits::temporal_window`](crate::ipc::proto::DeclaredTraits::temporal_window)
/// holds it, so a block whose numbers never went through a describe cannot ask
/// for a divisor nobody checked. Never fewer than the two a windowless plugin
/// ships.
fn slots_per_frame(traits: Option<&Traits>) -> u32 {
    let limit = lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW;
    let (lo, hi) = traits.map_or((0, 0), |declared| {
        (
            declared.temporal_lo.clamp(-limit, 0),
            declared.temporal_hi.clamp(0, limit),
        )
    });
    u32::try_from(i64::from(hi) - i64::from(lo) + 2)
        .unwrap_or(2)
        .max(2)
}

// ---------------------------------------------------------- the measurement --

/// Per-frame throughput, measured in epochs either side of a growth.
///
/// §4.4 says the pool grows when "measured per-frame throughput rose on the
/// last growth", which is a comparison and therefore needs two measurements. An
/// **epoch** is the run of frames since the last growth; its rate is the frames
/// it completed over the time the pool spent **busy**, which is throughput
/// rather than latency and so is the number that goes *up* when a wider pool
/// helps. A growth closes the epoch, banks its rate, and opens the next one.
///
/// **Busy, and not the wall clock.** A rate over wall time is a rate a pause
/// dilutes: a person scrubs, stops for a minute, and an epoch measured across
/// that minute cannot beat the one before it - after which the pool may never
/// grow again for the life of a definition that is leaked for the session. So
/// the clock here runs only while something is in flight: [`Self::arrived`]
/// starts it when the pool goes from idle to busy and [`Self::completed`] stops
/// it when the last frame in flight ends, and the time in between, in which the
/// pool rendered nothing because it was asked for nothing, is nobody's
/// evidence either way.
///
/// The clock arrives as an argument rather than being read in here, so the
/// policy can be driven over instants a test chooses.
#[derive(Debug)]
pub struct Throughput {
    /// When the run of busy time the pool is in now began. Meaningless while
    /// nothing is in flight, which is what `in_flight` says.
    began: Instant,
    /// The busy time this epoch has banked, the run in progress apart.
    busy: Duration,
    /// How many frames hold an instance right now, so an idle gap can be told
    /// from a slow frame.
    in_flight: usize,
    /// Frames completed in this epoch.
    frames: u64,
    /// What the epoch before the last growth achieved, or `None` while the pool
    /// has never grown - which is what makes the **first** growth free.
    before: Option<f64>,
}

impl Throughput {
    /// A pool that has rendered nothing and grown never.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            began: now,
            busy: Duration::ZERO,
            in_flight: 0,
            frames: 0,
            before: None,
        }
    }

    /// One more frame has an instance, so the pool is busy from here.
    pub fn arrived(&mut self, now: Instant) {
        if self.in_flight == 0 {
            self.began = now;
        }
        self.in_flight = self.in_flight.saturating_add(1);
    }

    /// One more frame is done, and the pool is idle again if it was the last.
    pub fn completed(&mut self, now: Instant) {
        self.frames = self.frames.saturating_add(1);
        self.in_flight = self.in_flight.saturating_sub(1);
        if self.in_flight == 0 {
            self.busy = self
                .busy
                .saturating_add(now.saturating_duration_since(self.began));
        }
    }

    /// Whether the last growth is one this pool may repeat.
    ///
    /// The first growth is free: there is no previous epoch to have beaten, and
    /// a pool that could never take its first step would never measure
    /// anything at all. After that a growth wants [`SAMPLE_FRAMES`] frames of
    /// evidence and a rate that beat the epoch before it - so **a growth that
    /// did not help is the last one**, which is the whole of what "adaptive"
    /// means here.
    #[must_use]
    pub fn rose(&self, now: Instant) -> bool {
        match self.before {
            None => true,
            Some(before) => self.frames >= SAMPLE_FRAMES && self.rate(now) > before,
        }
    }

    /// Bank this epoch's rate and open the next one.
    pub fn grew(&mut self, now: Instant) {
        self.before = Some(self.rate(now));
        self.began = now;
        self.busy = Duration::ZERO;
        self.frames = 0;
    }

    /// Frames per second of **busy** time so far this epoch, the run in
    /// progress included.
    fn rate(&self, now: Instant) -> f64 {
        let mut busy = self.busy;
        if self.in_flight > 0 {
            busy = busy.saturating_add(now.saturating_duration_since(self.began));
        }
        let seconds = busy.as_secs_f64();
        if seconds <= 0.0 {
            return 0.0;
        }
        self.frames as f64 / seconds
    }
}

// ----------------------------------------------------------------- the pool --

/// One live plugin instance, idle between frames.
///
/// [`Default`] for one reason only: [`Lease`] holds this **by value**, so that
/// there is no empty state for a caller to be answered out of, and `Drop` needs
/// something to leave in its place while it hands the instance back. The value
/// it leaves names no instance and is read by nobody - the lease it is inside
/// is being dropped.
#[derive(Debug, Default)]
struct Pooled {
    /// The driver's own name for it.
    id: InstanceId,
    /// What it was last told, so a frame whose bag has not moved costs no round
    /// trip.
    told: Vec<ParamValue>,
}

/// One effect instance's share of the pool.
#[derive(Debug, Default)]
struct Row {
    /// The instances nobody is rendering with.
    idle: Vec<Pooled>,
    /// How many exist at all: idle plus leased.
    live: usize,
    /// The offsets this row's last render said it reads, already clamped. It
    /// belongs to the row rather than to an instance: the key walk asks about
    /// the effect, and which of the row's instances happened to answer last is
    /// not a distinction that walk has.
    frames_needed: Vec<i32>,
    /// When this row was last **handed** an instance, on the pool's own
    /// counter - what the eviction reads, and nothing else. A turn that ended
    /// in a wait or a trim does not stamp it: a row whose frames are all queued
    /// behind a collapsed ceiling would otherwise keep refreshing its place in
    /// the queue without ever being leased, and eviction would prefer a
    /// genuinely quieter row over it.
    used: u64,
}

/// Everything the pool locks at once: the rows, the measurement over all of
/// them, and the counter eviction orders them by.
#[derive(Debug)]
struct Rows {
    rows: BTreeMap<Uuid, Row>,
    throughput: Throughput,
    /// Monotonic, so "least recently leased" is a comparison rather than a
    /// clock read.
    tick: u64,
}

/// What one turn around the lock decided, to be acted on outside it.
///
/// There is no `Wait` arm: waiting happens **inside** the critical section the
/// decision was taken in, on the pool's own condvar, because a decision to wait
/// that let the lock go first would be a decision taken against a table that
/// could have freed an instance in the meantime - and the wake for it would
/// already have been sent.
enum Step {
    /// An instance to render this frame with.
    Have(Pooled),
    /// A place has been kept in the row; open an instance to fill it.
    Open,
    /// The pool has collapsed under this row; close these and look again.
    Trim(Vec<InstanceId>),
}

/// The live instances one definition holds, and the policy they are grown
/// under.
#[derive(Debug)]
pub struct Pool {
    policy: Policy,
    rows: Mutex<Rows>,
    /// Woken when an instance goes back on a row's idle list, which is what a
    /// frame waiting at the ceiling is waiting for.
    freed: Condvar,
}

impl Pool {
    /// A pool under this policy, holding nothing.
    #[must_use]
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            rows: Mutex::new(Rows {
                rows: BTreeMap::new(),
                throughput: Throughput::new(Instant::now()),
                tick: 0,
            }),
            freed: Condvar::new(),
        }
    }

    /// The policy this pool grows under.
    #[must_use]
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    /// A live instance for one in-flight frame of `row`, with `values` in its
    /// controls.
    ///
    /// `samples` is the output's own sample count, which is what the declared
    /// scratch is charged against; a press has no picture and passes nought.
    ///
    /// The order is the policy read from the outside in: what the machine could
    /// never afford, refused before the frame waits on anybody; then the
    /// bundle's lock, where a thread-unsafe declaration made one; then the row,
    /// where an idle instance is taken, a new one opened, or the frame waits
    /// for one to come back; and only then the reservation the frame actually
    /// holds.
    ///
    /// **The reservation is last on purpose.** A frame in the queue is a frame
    /// allocating nothing, and one that held its declared scratch while it
    /// queued would raise [`Ledger::pressure`] on the tier the pool reads to
    /// decide whether it may grow - so the queue would collapse its own ceiling
    /// and then badge a later frame with the plugin's name for what was
    /// congestion. What survives before the queue is the question waiting
    /// cannot change: whether the tier's whole budget covers this frame at all.
    ///
    /// # Errors
    ///
    /// [`PoolError`] - the declared scratch the ledger would not grant, or the
    /// driver refusing to open an instance.
    pub fn lease<'p>(
        &'p self,
        row: Uuid,
        values: &[ParamValue],
        samples: usize,
        instances: &'p dyn LfxInstances,
    ) -> Result<Lease<'p>, PoolError> {
        let wanted = self.scratch_wanted(samples);
        if wanted > self.policy.ledger.budget(Tier::Ram) {
            // Not "more than is free", which the queue itself moves, but more
            // than the tier holds when it is empty: a frame no amount of
            // waiting could pay for.
            return Err(PoolError::Scratch { wanted });
        }
        // Taken **before** the rows, always, and never the other way round: a
        // lease holds this for the whole frame and the rows for none of it.
        let serial = self.policy.serial.as_ref().map(Serial::hold);
        let pooled = self.take(row, values, instances)?;
        self.rows().throughput.arrived(Instant::now());
        let mut lease = Lease {
            pool: self,
            instances,
            row,
            pooled,
            _scratch: None,
            _serial: serial,
        };
        // Built first, so that a refusal here hands the instance back through
        // the lease's own destructor rather than leaking it out of the row.
        lease._scratch = self.reserve(wanted)?;
        Ok(lease)
    }

    /// What this row's last render said it reads.
    #[must_use]
    pub fn frames_needed(&self, row: Uuid) -> Option<Vec<i32>> {
        let rows = self.rows();
        let offsets = &rows.rows.get(&row)?.frames_needed;
        // "Nothing more specific to say than the declaration" is `None`, which
        // is the frame in hand and nothing else.
        (offsets.len() > 1).then(|| offsets.clone())
    }

    /// Keep what a row's last render said it reads.
    pub fn remember(&self, row: Uuid, offsets: Vec<i32>) {
        let mut rows = self.rows();
        if let Some(entry) = rows.rows.get_mut(&row) {
            entry.frames_needed = offsets;
        }
    }

    /// How many live instances this row holds, idle and leased together.
    #[must_use]
    pub fn live(&self, row: Uuid) -> usize {
        self.rows().rows.get(&row).map_or(0, |entry| entry.live)
    }

    /// Every idle instance the pool holds, and the pool emptied.
    ///
    /// What is leased is not here: a frame in flight still holds its instance,
    /// and its lease closes it on the way out because the row it belonged to
    /// has gone.
    #[must_use]
    pub fn drain(&self) -> Vec<InstanceId> {
        let mut rows = self.rows();
        let held = std::mem::take(&mut rows.rows);
        held.into_values()
            .flat_map(|entry| entry.idle.into_iter().map(|pooled| pooled.id))
            .collect()
    }

    /// The rows, whatever a poisoned lock says.
    fn rows(&self) -> MutexGuard<'_, Rows> {
        self.rows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What this frame's declared working memory comes to, or nought for a
    /// plugin that declared none and for a call that carries no picture.
    fn scratch_wanted(&self, samples: usize) -> u64 {
        let rate = self.policy.scratch_bytes_per_megapixel;
        if rate == 0 || samples == 0 {
            return 0;
        }
        // Four samples to the pixel, and a part-megapixel frame is charged as
        // one: the declaration is a rate, and a frame smaller than the unit it
        // is quoted in still asks for working memory.
        let pixels = (samples as u64) / 4;
        let megapixels = pixels.div_ceil(MEGAPIXEL).max(1);
        rate.saturating_mul(megapixels)
    }

    /// Those bytes, held until the frame is over - or the refusal that keeps it
    /// from being dispatched at all.
    fn reserve(&self, wanted: u64) -> Result<Option<Reservation>, PoolError> {
        if wanted == 0 {
            return Ok(None);
        }
        self.policy
            .ledger
            .try_reserve(Tier::Ram, wanted)
            .map(Some)
            .ok_or(PoolError::Scratch { wanted })
    }

    /// Take an instance for this row: an idle one, a new one, or the next one
    /// handed back.
    fn take(
        &self,
        row: Uuid,
        values: &[ParamValue],
        instances: &dyn LfxInstances,
    ) -> Result<Pooled, PoolError> {
        loop {
            let step = {
                let mut rows = self.rows();
                loop {
                    if let Some(step) = self.decide(&mut rows, row, Instant::now()) {
                        break step;
                    }
                    // At the ceiling with nothing idle: wait for one of this
                    // row's own frames to end, and look again rather than
                    // trusting the wake - the condvar is the pool's.
                    rows = self
                        .freed
                        .wait(rows)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            match step {
                Step::Have(pooled) => return Ok(pooled),
                Step::Trim(closing) => {
                    // Outside the lock: closing reaches another process.
                    for id in closing {
                        instances.close(id);
                    }
                }
                Step::Open => {
                    return match instances.open(values.to_vec()) {
                        Ok(id) => {
                            self.evict(instances);
                            Ok(Pooled {
                                id,
                                told: values.to_vec(),
                            })
                        }
                        Err(error) => {
                            // The place kept for it goes back, or the row would
                            // count an instance that never opened against its
                            // ceiling for ever. And a row whose first open
                            // failed goes with it: a broker refusing every open -
                            // disabled after three strikes, or past
                            // `MAX_LIVE_INSTANCES` - would otherwise leave one
                            // empty row per effect id behind it for the life of
                            // a definition that is leaked for the session.
                            let mut rows = self.rows();
                            if let Some(entry) = rows.rows.get_mut(&row) {
                                entry.live = entry.live.saturating_sub(1);
                                if entry.live == 0
                                    && entry.idle.is_empty()
                                    && entry.frames_needed.is_empty()
                                {
                                    rows.rows.remove(&row);
                                }
                            }
                            drop(rows);
                            self.freed.notify_all();
                            Err(PoolError::Broker(error))
                        }
                    };
                }
            }
        }
    }

    /// One turn around the rows lock, which decides and never acts. `None` is
    /// "wait", and its caller waits without letting the lock go.
    ///
    /// `now` is read once by the caller and handed in, so that one turn's
    /// measurement and its growth are stamped with the same instant.
    fn decide(&self, rows: &mut Rows, row: Uuid, now: Instant) -> Option<Step> {
        let ceiling = self.policy.ceiling_now();
        let room = self.policy.room_to_grow();
        rows.tick = rows.tick.saturating_add(1);
        let tick = rows.tick;
        let grew = rows.throughput.rose(now);
        let entry = rows.rows.entry(row).or_default();
        // **The collapse.** A pool that grew while the tier was quiet gives the
        // instances back when it stops being quiet, and does it here rather
        // than waiting for every frame in flight to end.
        if entry.live > ceiling && !entry.idle.is_empty() {
            let over = entry.live - ceiling;
            let closing: Vec<InstanceId> = (0..over)
                .filter_map(|_| entry.idle.pop())
                .map(|pooled| pooled.id)
                .collect();
            entry.live = entry.live.saturating_sub(closing.len());
            return Some(Step::Trim(closing));
        }
        if let Some(pooled) = entry.idle.pop() {
            // An idle instance of this row, and the commonest road by far: one
            // row rendering one frame at a time never opens a second.
            entry.used = tick;
            return Some(Step::Have(pooled));
        }
        // The first instance of a row is not a growth. A pool collapsed to one
        // still has to render, and a row holding nothing has nothing to have
        // grown from.
        let first = entry.live == 0;
        if !first && (entry.live >= ceiling || !room || !grew) {
            return None;
        }
        entry.live = entry.live.saturating_add(1);
        entry.used = tick;
        if !first {
            rows.throughput.grew(now);
        }
        Some(Step::Open)
    }

    /// Hand an instance back, or close it where the pool collapsed under it.
    fn give_back(&self, row: Uuid, pooled: Pooled, instances: &dyn LfxInstances) {
        let ceiling = self.policy.ceiling_now();
        let closing = {
            let mut rows = self.rows();
            rows.throughput.completed(Instant::now());
            match rows.rows.get_mut(&row) {
                Some(entry) if entry.live <= ceiling => {
                    entry.idle.push(pooled);
                    None
                }
                Some(entry) => {
                    entry.live = entry.live.saturating_sub(1);
                    Some(pooled.id)
                }
                // The row went away under the frame, which only a drain does.
                None => Some(pooled.id),
            }
        };
        if let Some(id) = closing {
            instances.close(id);
        }
    }

    /// Close the idle instances of the least recently leased rows where more
    /// than [`MAX_POOL_ROWS`] of them hold one, and forget the rows past
    /// [`MAX_POOL_MEMOS`] that hold nothing at all.
    fn evict(&self, instances: &dyn LfxInstances) {
        let closing = {
            let mut rows = self.rows();
            let closing = rows.evict();
            rows.forget();
            closing
        };
        for id in closing {
            instances.close(id);
        }
    }
}

impl Rows {
    /// Close the idle instances of the least recently leased rows until no more
    /// than [`MAX_POOL_ROWS`] of them hold an instance at all.
    ///
    /// Only rows with nothing leased are candidates - `live == idle` is that
    /// test - so eviction never takes an instance out from under a frame.
    ///
    /// **The row itself stays**, emptied. What it keeps is its remembered
    /// window: the frame key is computed over `frames_needed`, and a row that
    /// lost it on the way out of the pool would come back keyed, prefetched and
    /// rendered against the *declared* window instead - a different picture
    /// rather than a round trip, and a different one depending on which rows
    /// the eviction happened to reach, which is what
    /// `two_exports_of_the_same_project_are_bit_identical_whatever_the_pool_did`
    /// says may not happen. [`Rows::forget`] is what bounds the rows
    /// themselves.
    fn evict(&mut self) -> Vec<InstanceId> {
        let holding = self.rows.values().filter(|entry| entry.live > 0).count();
        let over = holding.saturating_sub(MAX_POOL_ROWS);
        if over == 0 {
            return Vec::new();
        }
        let mut order: Vec<(u64, Uuid)> = self
            .rows
            .iter()
            .filter(|(_, entry)| entry.live > 0 && entry.live == entry.idle.len())
            .map(|(id, entry)| (entry.used, *id))
            .collect();
        order.sort_unstable();
        let mut closing = Vec::new();
        for (_, id) in order.into_iter().take(over) {
            let Some(entry) = self.rows.get_mut(&id) else {
                continue;
            };
            closing.extend(
                std::mem::take(&mut entry.idle)
                    .into_iter()
                    .map(|pooled| pooled.id),
            );
            entry.live = 0;
        }
        closing
    }

    /// Drop the least recently used rows that hold no instance at all, where
    /// the table has gone past [`MAX_POOL_MEMOS`].
    ///
    /// This is the memo table's own bound and the one place a row's remembered
    /// window is thrown away. A row holding an instance is never a candidate:
    /// it is in use, and [`Rows::evict`] is what answers those.
    fn forget(&mut self) {
        let over = self.rows.len().saturating_sub(MAX_POOL_MEMOS);
        if over == 0 {
            return;
        }
        let mut order: Vec<(u64, Uuid)> = self
            .rows
            .iter()
            .filter(|(_, entry)| entry.live == 0)
            .map(|(id, entry)| (entry.used, *id))
            .collect();
        order.sort_unstable();
        for (_, id) in order.into_iter().take(over) {
            self.rows.remove(&id);
        }
    }
}

// ---------------------------------------------------------------- the lease --

/// One in-flight frame's hold on a live instance.
///
/// While this is alive the instance is nobody else's, which is what makes
/// telling it its values and asking it for a picture one indivisible turn with
/// no lock of their own: there is no other frame to interleave with.
pub struct Lease<'p> {
    pool: &'p Pool,
    instances: &'p dyn LfxInstances,
    row: Uuid,
    /// The instance itself, **by value**: there is no empty state for a caller
    /// to be answered out of, so [`Lease::instance`] cannot invent a handle
    /// nobody minted and [`Lease::tell`] cannot quietly skip an update and
    /// answer `Ok` - which is the one failure §4.2 says this seam may not have.
    /// [`Drop`] takes it and leaves a defaulted one nothing reads behind.
    pooled: Pooled,
    /// The declared working memory this frame was granted, given back by its
    /// own destructor when the frame is over. Filled in by [`Pool::lease`]
    /// after the instance is in hand, so that a frame waiting in the queue
    /// holds no bytes.
    _scratch: Option<Reservation>,
    /// The bundle's lock, where a thread-unsafe declaration made one.
    _serial: Option<MutexGuard<'p, ()>>,
}

impl Lease<'_> {
    /// The driver's own name for the instance this frame is rendering with.
    #[must_use]
    pub const fn instance(&self) -> InstanceId {
        self.pooled.id
    }

    /// Tell the instance what its controls hold, where they have moved since it
    /// was last told - so a frame whose bag has not moved costs no round trip.
    ///
    /// # Errors
    ///
    /// [`BrokerError`], from the driver.
    pub fn tell(&mut self, values: &[ParamValue]) -> Result<(), BrokerError> {
        if self.pooled.told.as_slice() == values {
            return Ok(());
        }
        self.instances.update(self.pooled.id, values.to_vec())?;
        self.pooled.told = values.to_vec();
        Ok(())
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        // The instance is handed back and a `Pooled` naming nothing is left in
        // its place. The lease it is inside is being dropped, so the only code
        // that could read it is this function.
        let pooled = std::mem::take(&mut self.pooled);
        self.pool.give_back(self.row, pooled, self.instances);
        self.pool.freed.notify_all();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use lumit_lfx_abi::LFX_COST_HEAVY;

    use super::*;

    /// A ring of this many slots, as the broker publishes it.
    fn ring(slots: u32) -> RingSlots {
        RingSlots::of(slots)
    }

    /// The three ceilings §4.4 names, each read on its own.
    ///
    /// The worker count is the machine's, so the case asserts the **rule**
    /// rather than a number: whatever this machine answers, the ceiling is the
    /// least of the three and never nought.
    #[test]
    fn the_pool_is_capped_at_the_worker_count_and_at_the_ring_the_ledger_granted() {
        let ledger = Ledger::new();
        let serial = Serial::new();
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        let workers = lumit_eval::pool::worker_threads(cores);

        let wide = Policy::declared(None, &ring(u32::MAX), &ledger, &serial);
        assert_eq!(
            wide.ceiling(),
            workers.min(MAX_POOL_INSTANCES),
            "a ring that is no ceiling leaves the worker count and the sixteen"
        );

        let eight = Policy::declared(None, &ring(8), &ledger, &serial);
        assert_eq!(
            eight.ceiling(),
            workers.min(MAX_POOL_INSTANCES).min(4),
            "a ring of eight slots carries four frames in flight, input and output apiece"
        );

        let floor = Policy::declared(
            None,
            &ring(crate::ipc::ring::RING_MIN_SLOTS),
            &ledger,
            &serial,
        );
        assert_eq!(
            floor.ceiling(),
            1,
            "a ring the ledger narrowed to its floor carries one frame, and pins the pool with it"
        );

        let none = Policy::declared(None, &ring(0), &ledger, &serial);
        assert_eq!(
            none.ceiling(),
            1,
            "and a pool of no instances renders nothing"
        );
    }

    /// A frame of a temporal plugin ships its whole window, so the ring holds
    /// fewer of them as the declaration widens (§3.4, §4.4).
    ///
    /// The broker charges a shipment `neighbours + 2` slots and sizes the ring
    /// as `hi − lo + 2` for one frame, so reading a ring as "two slots a frame"
    /// would let a ±5 plugin's pool grow to six and have the sixth shipment
    /// refused with that plugin's name on it.
    #[test]
    fn a_temporal_plugins_ceiling_falls_as_its_declared_window_widens() {
        let ledger = Ledger::new();
        let serial = Serial::new();
        let reading = |lo: i32, hi: i32| Traits {
            temporal_lo: lo,
            temporal_hi: hi,
            ..Traits::default()
        };

        let windowless = Policy::declared(Some(&reading(0, 0)), &ring(24), &ledger, &serial);
        assert_eq!(
            windowless.ceiling(),
            windowless.workers.min(12),
            "a frame that reads no neighbour ships an input slot and an output slot"
        );

        let narrow = Policy::declared(Some(&reading(-1, 1)), &ring(24), &ledger, &serial);
        assert_eq!(
            narrow.ceiling(),
            narrow.workers.min(6),
            "t ± 1 is three frames read and one written, so a frame ships four slots"
        );

        let wide = Policy::declared(Some(&reading(-5, 5)), &ring(12), &ledger, &serial);
        assert_eq!(
            wide.ceiling(),
            1,
            "a twelve-slot ring is exactly one frame of a t ± 5 plugin"
        );
    }

    /// The ceiling is the ring the broker has **now**, not the one the
    /// definition was built over.
    ///
    /// `Broker::fit` drops the ring and makes another the first time a frame
    /// arrives that a slot will not hold, and a bigger frame buys fewer slots -
    /// so a policy that copied the count once would keep a ceiling of sixteen
    /// after the first 4K fp32 frame narrowed the ring to its floor, and would
    /// keep it in the permissive direction.
    #[test]
    fn the_ceiling_follows_the_ring_the_broker_has_now() {
        let slots = ring(64);
        let policy = Policy::declared(None, &slots, &Ledger::new(), &Serial::new());
        assert!(
            policy.ceiling() > 1,
            "a sixty-four slot ring is more than one frame in flight"
        );

        slots.set(crate::ipc::ring::RING_MIN_SLOTS);
        assert_eq!(
            policy.ceiling(),
            1,
            "and the ring the ledger narrowed under it pins the pool to one"
        );
    }

    /// A growth that did not raise throughput is the last one, and the first
    /// growth is free (§4.4).
    ///
    /// The clock is fed rather than read, so what the case measures is the rule
    /// and not the machine it ran on.
    #[test]
    fn a_growth_that_did_not_raise_throughput_is_the_last_one() {
        let start = Instant::now();
        let mut measured = Throughput::new(start);
        assert!(
            measured.rose(start + Duration::from_secs(1)),
            "the first growth is free: there is no epoch for it to have beaten"
        );

        // Eight frames in a second, and then a growth that banks that rate.
        render(&mut measured, start, 8, Duration::from_millis(125));
        measured.grew(start + Duration::from_secs(1));

        // The new epoch does better: eight frames in half a second.
        render(
            &mut measured,
            start + Duration::from_secs(1),
            8,
            Duration::from_millis(62),
        );
        assert!(
            measured.rose(start + Duration::from_millis(1_500)),
            "sixteen a second beat eight a second, so the pool may grow again"
        );
        measured.grew(start + Duration::from_millis(1_500));

        // And this one does worse: eight frames in two seconds.
        render(
            &mut measured,
            start + Duration::from_millis(1_500),
            8,
            Duration::from_millis(250),
        );
        assert!(
            !measured.rose(start + Duration::from_millis(3_500)),
            "four a second did not beat sixteen a second, so that growth was the last"
        );
    }

    /// A rate is not evidence until there is enough of it.
    ///
    /// Without this a pool would grow on the strength of its first frame after
    /// each growth - which is a measurement of that frame's picture rather than
    /// of the pool, and would walk to the ceiling whatever the widening did.
    #[test]
    fn a_growth_wants_a_run_of_frames_behind_it_rather_than_one() {
        let start = Instant::now();
        let mut measured = Throughput::new(start);
        measured.grew(start);
        render(
            &mut measured,
            start,
            SAMPLE_FRAMES - 1,
            Duration::from_millis(100),
        );
        assert!(
            !measured.rose(start + Duration::from_secs(1)),
            "seven frames is not yet a measurement"
        );
        render(
            &mut measured,
            start + Duration::from_millis(700),
            1,
            Duration::from_millis(100),
        );
        assert!(
            measured.rose(start + Duration::from_secs(1)),
            "and eight is"
        );
    }

    /// An idle gap is not a slow epoch (§4.4).
    ///
    /// The measurement is frames over the time the pool spent **busy**, not
    /// over the wall clock: a person scrubs, stops for a minute and comes back,
    /// and the epoch that minute fell in must not be a rate the next epoch
    /// cannot beat. A definition is leaked for the session, so "cannot beat"
    /// would have meant "never grows again" for as long as Lumit was running.
    #[test]
    fn an_idle_gap_is_not_a_slow_epoch() {
        let start = Instant::now();
        let mut measured = Throughput::new(start);

        // Two growths, the second banking a brisk epoch: eight frames in half a
        // second is sixteen a second.
        measured.grew(start);
        render(&mut measured, start, 8, Duration::from_millis(62));
        assert!(measured.rose(start + Duration::from_millis(500)));
        measured.grew(start + Duration::from_millis(500));

        // Nobody renders anything for a minute. Then eight frames arrive a
        // little quicker than the epoch that was banked.
        let back = start + Duration::from_secs(60);
        render(&mut measured, back, 8, Duration::from_millis(40));
        assert!(
            measured.rose(back + Duration::from_millis(320)),
            "the minute nothing rendered in is nobody's evidence, so the pool may still grow"
        );
    }

    /// `count` frames, one after another, each taking `each`.
    fn render(measured: &mut Throughput, from: Instant, count: u64, each: Duration) {
        for frame in 0..count {
            let at = from + each * u32::try_from(frame).unwrap_or(u32::MAX);
            measured.arrived(at);
            measured.completed(at + each);
        }
    }

    /// `lfx.thread-unsafe` pins the pool to one instance and puts it behind the
    /// bundle's own lock (§4.4, §2.6).
    #[test]
    fn a_thread_unsafe_declaration_pins_the_pool_to_one_behind_the_bundles_lock() {
        let declared = Traits {
            flags: LFX_TRAIT_THREAD_UNSAFE,
            ..Traits::default()
        };
        let serial = Serial::for_bundle([declared.flags]);
        assert!(serial.is_armed(), "the bundle declared it");
        let policy = Policy::declared(Some(&declared), &ring(u32::MAX), &Ledger::new(), &serial);
        assert_eq!(policy.ceiling(), 1);
        assert!(policy.is_serial());
    }

    /// And so does a plugin of that bundle which declared nothing of the sort.
    ///
    /// The header says the declaration "is serialised bundle-wide" and §4.4
    /// says it "pins it to one, bundle-wide": the plugin that declared nothing
    /// shares one process, one broker and one ring with the plugin that did, so
    /// arming the lock from each plugin's own block would leave the bundle
    /// running frames beside the very code the declaration was made about.
    #[test]
    fn a_bundle_one_plugin_declared_thread_unsafe_is_pinned_whole() {
        let declaring = Traits {
            flags: LFX_TRAIT_THREAD_UNSAFE,
            ..Traits::default()
        };
        let quiet = Traits::default();
        let serial = Serial::for_bundle([quiet.flags, declaring.flags]);
        assert!(serial.is_armed(), "one of the bundle's plugins declared it");

        let policy = Policy::declared(Some(&quiet), &ring(u32::MAX), &Ledger::new(), &serial);
        assert_eq!(
            policy.ceiling(),
            1,
            "a plugin of a thread-unsafe bundle is pinned whether it declared it or not"
        );
        assert!(policy.is_serial());
    }

    /// A declaration the plugin never made does **not** pin the pool.
    ///
    /// Every other zero in a trait block is the pessimistic answer (§2.4), and
    /// this one deliberately is not: pinning every plugin that declared no
    /// traits at all would make the pessimistic case the serial case, which is
    /// a performance answer to a correctness question and would collapse the
    /// pool for the commonest declaration there is.
    #[test]
    fn a_declaration_the_plugin_never_made_does_not_pin_the_pool() {
        let ledger = Ledger::new();
        let elsewhere = Traits {
            cost: LFX_COST_HEAVY,
            ..Traits::default()
        };
        let serial = Serial::for_bundle([0, elsewhere.flags]);
        assert!(
            !serial.is_armed(),
            "a bundle where nobody declared it arms nothing"
        );
        assert!(!Policy::declared(None, &ring(u32::MAX), &ledger, &serial).is_serial());

        let policy = Policy::declared(Some(&elsewhere), &ring(u32::MAX), &ledger, &serial);
        assert!(
            !policy.is_serial(),
            "a block that declared other things and not this one declared not this one"
        );
        assert!(policy.ceiling() >= 1);
    }
}
