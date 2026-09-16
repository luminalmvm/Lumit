//! The frame plane: one shared-memory ring per bundle, sized by the depth it
//! carries and charged to the governor's ledger (docs/impl/lfx.md §3.4).
//!
//! # In plain terms
//!
//! A 4K frame is a hundred megabytes. Pushing that down a pipe, twice per
//! render, would cost more than the effect. So the two processes agree on one
//! block of memory that is *the same memory* in both of them - write it here,
//! read it there, no copy in between - and the pipe carries only the slot
//! number.
//!
//! The block is divided into equal **slots**, used in turn. Three is the floor,
//! so that the slot being written is never the slot being read, with one spare
//! between them. Every slot begins with a sixty-four byte header, and the
//! header is the whole contract for the bytes after it: what rectangle they
//! are, how far apart the rows are, **which depth they were written at**,
//! whether the alpha is premultiplied, how many bytes there are, and a hash of
//! them. Frames are tightly packed and top-down: row nought is the top.
//!
//! The reader checks the hash. Shared memory is the one place where a wrong
//! answer arrives silently - no error, no status, just the previous frame's
//! pixels - and the hash is what turns that into a noticed fault. It also
//! checks the header against **itself**, because the other end of the mapping
//! is a stranger's compiled code and a hash says only that the bytes are the
//! ones the writer meant: a header claiming a 4K rectangle over sixty-four
//! honestly hashed bytes is a header nothing may walk by.
//!
//! This is the OFX host's `ipc::shm` design with two changes, and they are the
//! two the note names.
//!
//! **A slot is sized by the frame's depth.** `slot_bytes = w × h × 4 ×
//! depth_bytes + 64`, so an fp16 ring is half an fp32 ring and buys twice the
//! slots inside the same budget. The rule is [`slot_bytes_for`] and
//! [`slots_for`] rather than a table, because a number written in prose drifts
//! and this one already has: `ring-slots.txt` beside this crate is generated
//! from the arithmetic itself, and docs/impl/lfx.md §3.4 prints it only as an
//! illustration.
//!
//! **And the ring pays the governor.** docs/13 §3 says an unaccounted frame
//! allocation fails code review, and neither older host depends on
//! `lumit-budget` - so the OFX ring is up to half a gigabyte of RAM the ledger
//! has never heard of. [`Ring::create`] takes an `&Arc<Ledger>` (an `&Ledger`
//! cannot reserve: a [`Reservation`] owns an `Arc` so that its `Drop` can give
//! the bytes back, which is the very property being relied on), holds the
//! reservation **in the same struct as the mapping**, and walks the slot count
//! down when the ledger will not grant the whole ring.
//! [`RING_MIN_SLOTS`] is a floor the ledger may not push through: three slots
//! are taken whatever it says, because a ring of two cannot hold one input, one
//! output and one in flight at once. The denial is recorded against the ledger
//! rather than silently forgotten, and the bundle is skipped only when the
//! mapping itself fails, which is a different sentence.
//!
//! **The ceiling that is left is counted in slots, not bytes.** A `t ± 5`
//! prefetch is eleven frames plus an output, and halving the slot size does not
//! change that. So the count is asked for from the plugin's **declared temporal
//! window** as well as from the budget, and the ledger is what says no - the
//! honest form of the promise, declared before the first frame instead of
//! discovered at one.
//!
//! # Thread role
//!
//! One ring per broker, owned by the broker that made it. Slots are handed out
//! round-robin by the caller and a slot is never read while it is being
//! written; nothing here takes a lock. Regrowing - making a wider ring when a
//! frame bigger than a slot arrives - is the caller's, is
//! [`crate::ipc::broker`]'s `fit`, and is only ever run between renders. A
//! replacement ring reserves before the old one is dropped, so the ledger would
//! briefly see both: the caller drops the ring it is replacing first, and that
//! is what `fit` does.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use half::f16;
use lumit_budget::{Ledger, Reservation, Tier};
use memmap2::MmapMut;
use thiserror::Error;

use crate::ipc::proto::{PixelDepth, RectI, RingSpec, Slot, CHANNELS};

/// The bytes at the head of every slot.
pub const HEADER_BYTES: usize = 64;

/// The four bytes that say a slot has been written by this protocol at all -
/// `LFXR`, and not the OFX ring's magic: a slot read by the wrong host is a
/// slot that holds no frame rather than one that holds a plausible wrong one.
const HEADER_MAGIC: u32 = 0x4c46_5852;

/// The slot header's own version, which is not the protocol's: the header is
/// written and read by the same pair of processes, and it changes when the
/// sixty-four bytes do.
const HEADER_VERSION: u32 = 1;

/// How much memory one bundle's ring may ask the ledger for before the declared
/// window has anything to say. A budget, chosen rather than measured, and the
/// file really is this big on disk - so it is deliberately not enormous.
///
/// What it buys is [`slots_for`]'s arithmetic and `ring-slots.txt`'s table, not
/// a number written here: at 1080p it is sixteen fp32 slots or thirty-two fp16
/// ones, and at UHD 4K four and eight.
pub const RING_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

/// Triple buffering, as the floor it is - and a floor the ledger may not push
/// through (docs/impl/lfx.md §3.4).
pub const RING_MIN_SLOTS: u32 = 3;

/// The ceiling, so that a tiny frame does not mint a hundred thousand slots
/// nobody will ever use, and so that the widest window a plugin may declare
/// cannot ask for a ring nobody could map.
pub const RING_MAX_SLOTS: u32 = 64;

/// What can go wrong with the ring.
#[derive(Debug, Error)]
pub enum RingError {
    /// The backing file.
    #[error("the frame ring could not be opened: {0}")]
    Io(#[from] std::io::Error),
    /// A slot number that is not in the ring.
    #[error("slot {0} is not in the frame ring")]
    NoSuchSlot(Slot),
    /// A frame bigger than a slot. The caller regrows its ring before a render,
    /// so this is a frame that arrived by another road.
    #[error("a {needed}-byte frame does not fit a {slot_bytes}-byte ring slot")]
    TooBig {
        /// What the frame needs, header included.
        needed: u64,
        /// What a slot holds.
        slot_bytes: u64,
    },
    /// A picture whose sample count is not the rectangle it says it is.
    #[error("a {given}-sample frame is not a {expected}-sample rectangle")]
    WrongSampleCount {
        /// How many samples were handed over.
        given: u64,
        /// How many the bounds call for.
        expected: u64,
    },
    /// A slot header that claims a payload its own bounds do not call for.
    ///
    /// The read-path half of [`RingError::WrongSampleCount`], and it is here
    /// because the far end of this mapping is a stranger's compiled code: a
    /// header is only worth trusting to the extent it has been checked against
    /// itself. A slot claiming a 4K rectangle over sixty-four bytes of payload
    /// would otherwise come back as a frame whose bounds walk the caller off
    /// the end of the buffer it was handed.
    #[error("the slot header claims {given} bytes of pixels where its bounds call for {expected}")]
    WrongPayloadBytes {
        /// What the header says follows it.
        given: u64,
        /// What its own bounds and depth call for.
        expected: u64,
    },
    /// A slot header whose rows are not the tight rows the ring writes.
    ///
    /// The ring's own block is always tightly packed - [`FrameHeader::row_bytes`]
    /// describes the ring rather than the plugin - so a stride that is not the
    /// width times the samples times the depth is a header disagreeing with
    /// itself.
    #[error(
        "the slot header puts its rows {given} bytes apart where its bounds call for {expected}"
    )]
    WrongRowBytes {
        /// What the header says the stride is.
        given: u32,
        /// What its own bounds and depth call for.
        expected: u32,
    },
    /// A slot that was never written, or was written by something else.
    #[error("the frame ring slot holds no frame")]
    Empty,
    /// A slot written at one depth and read at the other.
    ///
    /// Refused rather than converted: the host never converts a depth to
    /// accommodate anybody (docs/12 §3.3), and reading fp16 bytes as fp32 would
    /// not fail - it would hand back a picture of noise.
    #[error("the slot holds a {written} frame and was read as {wanted}")]
    DepthMismatch {
        /// What the writer wrote.
        written: &'static str,
        /// What the reader asked for.
        wanted: &'static str,
    },
    /// The hash in the header does not match the bytes after it.
    #[error("a frame crossed the ring and arrived changed")]
    Corrupt,
}

/// One slot's header, as values rather than bytes.
///
/// **Row nought is the top.** Frames are tightly packed and top-down, as
/// `Frame16::from_pixels` and docs/impl/lfx.md §2.5 have them; the ring does
/// not carry an upside-down-on-purpose path the way the OFX host does for
/// plugins that want one. It is written here because the doc on
/// [`FrameHeader::premultiplied`] makes the argument for writing assumptions
/// down, and row order is exactly such an assumption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    /// The rectangle the pixels cover.
    pub bounds: RectI,
    /// How far apart the rows are in the ring's own block: always positive and
    /// always tight, because this describes the ring rather than the plugin.
    pub row_bytes: u32,
    /// Which depth the pixels were written at. The one field the OFX ring's
    /// header has not got, and the reason it is here is that a ring sized for
    /// fp32 holds an fp16 frame perfectly well - so the slot has to say which
    /// it is rather than the ring saying it for every slot at once.
    pub depth: PixelDepth,
    /// Whether the alpha is premultiplied. Lumit's always is
    /// (docs/06-RENDER-PIPELINE.md); it is written down anyway, because a frame
    /// that crosses a process boundary carrying an assumption is a frame that
    /// will one day carry the wrong one.
    pub premultiplied: bool,
    /// How many bytes of pixels follow the header.
    pub payload_bytes: u64,
    /// FNV-1a over exactly those bytes.
    pub hash: u64,
}

impl FrameHeader {
    /// The header as the sixty-four bytes that go at the head of a slot.
    fn to_bytes(self) -> [u8; HEADER_BYTES] {
        let mut out = [0_u8; HEADER_BYTES];
        let mut put = |offset: usize, bytes: &[u8]| {
            if let Some(slot) = out.get_mut(offset..offset + bytes.len()) {
                slot.copy_from_slice(bytes);
            }
        };
        put(0, &HEADER_MAGIC.to_le_bytes());
        put(4, &HEADER_VERSION.to_le_bytes());
        put(8, &self.bounds.x0.to_le_bytes());
        put(12, &self.bounds.y0.to_le_bytes());
        put(16, &self.bounds.x1.to_le_bytes());
        put(20, &self.bounds.y1.to_le_bytes());
        put(24, &self.row_bytes.to_le_bytes());
        put(28, &self.depth.as_pixel_format().to_le_bytes());
        put(32, &u32::from(self.premultiplied).to_le_bytes());
        put(40, &self.payload_bytes.to_le_bytes());
        put(48, &self.hash.to_le_bytes());
        out
    }

    /// The header a slot begins with, or [`RingError::Empty`] if it begins with
    /// anything else.
    fn from_bytes(bytes: &[u8]) -> Result<Self, RingError> {
        let u32_at = |offset: usize| -> Option<u32> {
            let slice: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
            Some(u32::from_le_bytes(slice))
        };
        let u64_at = |offset: usize| -> Option<u64> {
            let slice: [u8; 8] = bytes.get(offset..offset + 8)?.try_into().ok()?;
            Some(u64::from_le_bytes(slice))
        };
        #[allow(clippy::cast_possible_wrap)]
        let i32_at = |offset: usize| -> Option<i32> { u32_at(offset).map(|value| value as i32) };

        if u32_at(0) != Some(HEADER_MAGIC) || u32_at(4) != Some(HEADER_VERSION) {
            return Err(RingError::Empty);
        }
        let (Some(x0), Some(y0), Some(x1), Some(y1)) =
            (i32_at(8), i32_at(12), i32_at(16), i32_at(20))
        else {
            return Err(RingError::Empty);
        };
        let (Some(row_bytes), Some(format), Some(premultiplied)) =
            (u32_at(24), u32_at(28), u32_at(32))
        else {
            return Err(RingError::Empty);
        };
        let (Some(payload_bytes), Some(hash)) = (u64_at(40), u64_at(48)) else {
            return Err(RingError::Empty);
        };
        // A depth the header does not name is not a frame this build can read.
        // It is `Empty` rather than `DepthMismatch`, because the reader asked
        // for one of two depths and this is neither.
        let Some(depth) = PixelDepth::from_pixel_format(format) else {
            return Err(RingError::Empty);
        };
        Ok(Self {
            bounds: RectI { x0, y0, x1, y1 },
            row_bytes,
            depth,
            premultiplied: premultiplied != 0,
            payload_bytes,
            hash,
        })
    }
}

/// FNV-1a, 64 bit. Not a cryptographic hash and not asked to be one: this
/// catches a slot that was overwritten or never written, which is the failure
/// shared memory has.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// How many bytes one frame of this size and depth needs in a slot, header
/// included: `w × h × 4 × depth_bytes + 64`.
#[must_use]
pub fn slot_bytes_for(width: u32, height: u32, depth: PixelDepth) -> u64 {
    u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(u64::from(CHANNELS))
        .saturating_mul(depth.bytes_per_sample())
        .saturating_add(HEADER_BYTES as u64)
        .max(HEADER_BYTES as u64)
}

/// How many slots a ring of this frame asks for, before the ledger has its say.
///
/// Two askings, and the larger wins. The **budget** buys
/// `floor(RING_BUDGET_BYTES / slot_bytes)` slots, which is what makes a smaller
/// frame or a shallower depth worth more slots rather than the same number of
/// bigger ones. The **declared window** asks for `hi − lo + 2` - every frame
/// the plugin says it reads, plus the one being written - which is the answer
/// only LFX can give, because only LFX knows the window before the first frame
/// rather than at one (docs/impl/lfx.md §3.4).
///
/// Both are then held between [`RING_MIN_SLOTS`] and [`RING_MAX_SLOTS`]. A
/// plugin that declares nothing asks for the budget's own answer and never for
/// less than the floor.
///
/// **The ceiling answers before the ledger does.** `LFX_MAX_TEMPORAL_WINDOW` is
/// sixty-four each way and [`RING_MAX_SLOTS`] is sixty-four slots, so every
/// declared window wider than ±31 is held to the ring rather than to the
/// machine's memory - which is a different sentence from "the budget would not
/// buy it" and reads identically from here. The two answers are told apart by
/// the broker's report line rather than by this function, which returns a
/// number: `ring_report` holds all three numbers and prints
/// [`LfxRejection::WindowHeldToTheRing`](crate::LfxRejection::WindowHeldToTheRing)
/// or
/// [`LfxRejection::RingNarrowedByTheLedger`](crate::LfxRejection::RingNarrowedByTheLedger).
#[must_use]
pub fn slots_for(width: u32, height: u32, depth: PixelDepth, window: (i32, i32)) -> u32 {
    let slot_bytes = slot_bytes_for(width, height, depth).max(1);
    let by_budget = u32::try_from(RING_BUDGET_BYTES / slot_bytes).unwrap_or(RING_MAX_SLOTS);
    let (lo, hi) = window;
    let by_window = u32::try_from(i64::from(hi) - i64::from(lo) + 2).unwrap_or(RING_MIN_SLOTS);
    by_budget
        .max(by_window)
        .clamp(RING_MIN_SLOTS, RING_MAX_SLOTS)
}

/// How many slots the bundle's ring holds **right now**, published for whoever
/// must read it without taking the broker's lock.
///
/// The count is not a constant of the session: [`crate::ipc::broker`]'s `fit`
/// drops the ring and makes a wider one the first time a frame arrives that a
/// slot will not hold, and `slots_for` answers that larger frame with *fewer*
/// slots. So a reader that copied the number once - §4.4's pool did - would go
/// on believing a sixty-four slot ring after the first 4K fp32 frame had
/// narrowed it to [`RING_MIN_SLOTS`], and would believe it in the permissive
/// direction (docs/impl/lfx.md §4.4).
///
/// A relaxed atomic rather than a lock: the broker's own mutex is held across a
/// render, and the pool reads this between frames of its own. A read that
/// crosses a regrow answers the count either side of it, and the next frame
/// reads the new one.
#[derive(Clone, Debug)]
pub struct RingSlots(Arc<std::sync::atomic::AtomicU32>);

impl RingSlots {
    /// A count nobody has published yet, or the one a test chose.
    #[must_use]
    pub fn of(slots: u32) -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU32::new(slots)))
    }

    /// What the ring holds now.
    #[must_use]
    pub fn get(&self) -> u32 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Say what the ring holds, which only the broker that made it may.
    pub fn set(&self, slots: u32) {
        self.0.store(slots, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Default for RingSlots {
    /// No ring at all, which is what a pool reading it treats as one frame in
    /// flight and no more.
    fn default() -> Self {
        Self::of(0)
    }
}

/// What a ring is being made for: one comp frame, at one depth, for a plugin
/// that declared one temporal window.
///
/// A struct rather than four arguments because all four decide the same one
/// number, and a caller that passed the depth where the height goes would
/// otherwise get a ring that worked and was wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingPlan {
    /// The frame's width in pixels.
    pub width: u32,
    /// The frame's height in pixels.
    pub height: u32,
    /// The depth the slots are sized for.
    pub depth: PixelDepth,
    /// The plugin's declared window, as
    /// [`DeclaredTraits::temporal_window`](crate::ipc::proto::DeclaredTraits::temporal_window)
    /// answers it. `(0, 0)` for a plugin that declared none.
    pub window: (i32, i32),
}

impl RingPlan {
    /// A plan for a frame of this size at this depth, for a plugin that reads
    /// no neighbours.
    #[must_use]
    pub fn frame(width: u32, height: u32, depth: PixelDepth) -> Self {
        Self {
            width,
            height,
            depth,
            window: (0, 0),
        }
    }

    /// The same plan with the plugin's declared window on it.
    #[must_use]
    pub fn reading(self, window: (i32, i32)) -> Self {
        Self { window, ..self }
    }

    /// The plan that satisfies both: the wider frame, the deeper depth, and the
    /// wider window either asks for.
    ///
    /// A ring is **raised and never lowered**. The caller that regrows for a
    /// bigger frame is not the caller that sized it from a declared window, and
    /// a regrow that took the newer plan whole would quietly give back the
    /// slots a temporal plugin was promised - which is a prefetch refused later
    /// for a reason nobody can see. The join is here rather than in the caller
    /// because it is the same decision every caller has to make
    /// (docs/impl/lfx.md §3.4).
    #[must_use]
    pub fn max_of(self, other: Self) -> Self {
        Self {
            width: self.width.max(other.width),
            height: self.height.max(other.height),
            depth: self.depth.max(other.depth),
            window: (
                self.window.0.min(other.window.0),
                self.window.1.max(other.window.1),
            ),
        }
    }

    /// How many bytes one slot of this plan takes.
    #[must_use]
    pub fn slot_bytes(self) -> u64 {
        slot_bytes_for(self.width, self.height, self.depth)
    }

    /// How many slots it asks for, before the ledger has its say.
    #[must_use]
    pub fn slots_wanted(self) -> u32 {
        slots_for(self.width, self.height, self.depth, self.window)
    }
}

/// The ring, mapped into this process.
pub struct Ring {
    spec: RingSpec,
    map: MmapMut,
    /// Set on the side that made the file, so that side deletes it.
    owned: Option<PathBuf>,
    /// The maker's own handle, kept open for the life of the ring. On Windows
    /// it carries delete-on-close, so the file goes when this process ends
    /// whether or not anything dropped the ring.
    file: Option<File>,
    /// What the ledger granted for the mapping, **in the same struct as the
    /// mapping** - lumit-budget's own rule, keep it beside the thing it paid
    /// for. `None` on the broker's side, which maps a ring somebody else made
    /// and pays for nothing, and `None` for the floor the ledger refused and
    /// the ring took anyway.
    reservation: Option<Reservation>,
}

impl std::fmt::Debug for Ring {
    /// The layout and what the ledger holds for it - never the mapping, which
    /// is up to half a gigabyte of somebody's frames and is what a derived
    /// `Debug` would reach for.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ring")
            .field("slots", &self.spec.slots)
            .field("slot_bytes", &self.spec.slot_bytes)
            .field("depth", &self.spec.depth)
            .field("reserved_bytes", &self.reserved_bytes())
            .finish_non_exhaustive()
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        // The reservation goes back to the ledger through its own destructor,
        // which is the whole reason it is held here rather than counted
        // somewhere. Nothing to do for it.
        let Some(path) = self.owned.take() else {
            return;
        };
        // Windows will not delete a file that is still mapped, and the file is
        // as big as the ring. So the mapping goes first, swapped for a
        // one-byte anonymous one, then the handle, which on Windows is the
        // delete. The remove is for the other platforms.
        if let Ok(empty) = MmapMut::map_anon(1) {
            drop(std::mem::replace(&mut self.map, empty));
        }
        drop(self.file.take());
        let _ = std::fs::remove_file(path);
    }
}

impl Ring {
    /// Make a ring for this plan and map it, charging it to the ledger. Called
    /// once, when a broker is spawned, and again when a frame bigger than a
    /// slot arrives.
    ///
    /// The slot count starts at [`RingPlan::slots_wanted`] and is halved until
    /// the ledger grants it or it reaches [`RING_MIN_SLOTS`], which is taken
    /// whatever the ledger says - a ring of two cannot hold one input, one
    /// output and one in flight at once, and a plugin with no ring at all is a
    /// bundle that is skipped rather than one that is slow. The ledger's own
    /// denial count records the floor being taken over its head.
    ///
    /// # Errors
    ///
    /// [`RingError::Io`] - the file could not be made or mapped, which is the
    /// one failure that does skip the bundle.
    pub fn create(path: &Path, plan: RingPlan, ledger: &Arc<Ledger>) -> Result<Self, RingError> {
        let slot_bytes = plan.slot_bytes();
        let (slots, reservation) = reserve_slots(plan.slots_wanted(), slot_bytes, ledger);
        let spec = RingSpec {
            path: path.to_string_lossy().into_owned(),
            slots,
            slot_bytes,
            depth: plan.depth,
        };
        let mut options = OpenOptions::new();
        // `create_new`, not `create` + `truncate`.
        //
        // In plain terms: the ring is a real file in the temporary directory,
        // and opening whatever is already at that path and truncating it would
        // follow a symbolic link somebody else planted, wherever it pointed.
        // Refusing a path that already exists costs nothing - the name is 128
        // random bits, so it never does - and turns that into an error instead.
        options.read(true).write(true).create_new(true);
        // FILE_FLAG_DELETE_ON_CLOSE. The broker still opens the file by name
        // while this handle is open; std's default share mode allows that.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0400_0000);
        }
        // Readable and writable by this user alone. The temporary directory is
        // usually world-writable, and a ring holds the frames of whatever the
        // user is editing.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        file.set_len(slot_bytes.saturating_mul(u64::from(slots)))?;
        let map = map_file(&file)?;
        Ok(Self {
            spec,
            map,
            owned: Some(path.to_path_buf()),
            file: Some(file),
            reservation,
        })
    }

    /// Map a ring somebody else made. Called once, in the broker, which pays
    /// for nothing: the ledger is the host's.
    ///
    /// # Errors
    ///
    /// [`RingError::Io`].
    pub fn open(spec: &RingSpec) -> Result<Self, RingError> {
        let file = OpenOptions::new().read(true).write(true).open(&spec.path)?;
        let map = map_file(&file)?;
        Ok(Self {
            spec: spec.clone(),
            map,
            owned: None,
            file: None,
            reservation: None,
        })
    }

    /// Take the ring's file out of the directory, now that the broker has it
    /// mapped. Unix only, and a no-op everywhere else.
    ///
    /// # In plain terms
    ///
    /// A Unix mapping is of the *file*, not of its name: once both processes
    /// have called `mmap`, the name in the temporary directory is doing nothing
    /// but letting other programs find it. Removing it at that point leaves the
    /// ring working perfectly in both processes, makes it impossible for a
    /// third to open it by name, and - the part that matters most in practice -
    /// has the kernel reclaim the space the moment the last of the two exits, a
    /// crash or a `kill -9` included. That is also why a restarted broker
    /// always gets a new ring, which is consistent with a restart being a
    /// replay rather than a recovery.
    ///
    /// Windows is the other way round: a mapped file cannot be unlinked at all,
    /// which is why the handle carries `FILE_FLAG_DELETE_ON_CLOSE` instead and
    /// the operating system does the same job on process exit.
    ///
    /// Called by the host when the broker says
    /// [`RingOpened`](crate::ipc::proto::BrokerMessage::RingOpened) - never
    /// before, or the broker would be opening a name that has gone.
    pub fn unlink_now_it_is_shared(&mut self) {
        if cfg!(windows) {
            return;
        }
        let Some(path) = self.owned.take() else {
            return;
        };
        // The handle stays open and the mapping stays live; only the name goes.
        let _ = std::fs::remove_file(path);
    }

    /// The layout, to send to the other side.
    #[must_use]
    pub const fn spec(&self) -> &RingSpec {
        &self.spec
    }

    /// How many slots there are.
    #[must_use]
    pub const fn slots(&self) -> u32 {
        self.spec.slots
    }

    /// The depth the slots were sized for.
    #[must_use]
    pub const fn depth(&self) -> PixelDepth {
        self.spec.depth
    }

    /// How many bytes the ledger is holding for this ring - nought for a ring
    /// the ledger refused and the floor took anyway, and nought in the broker,
    /// which maps what the host paid for.
    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.reservation.as_ref().map_or(0, Reservation::bytes)
    }

    /// Where one slot starts and ends.
    fn range(&self, slot: Slot) -> Result<(usize, usize), RingError> {
        if slot >= self.spec.slots {
            return Err(RingError::NoSuchSlot(slot));
        }
        let start: usize = self
            .spec
            .slot_bytes
            .saturating_mul(u64::from(slot))
            .try_into()
            .map_err(|_| RingError::NoSuchSlot(slot))?;
        let length: usize = self
            .spec
            .slot_bytes
            .try_into()
            .map_err(|_| RingError::NoSuchSlot(slot))?;
        Ok((start, start.saturating_add(length)))
    }

    /// Put an fp32 picture in a slot and answer with the header that was
    /// written.
    ///
    /// # Errors
    ///
    /// [`RingError::WrongSampleCount`] for pixels that are not the rectangle
    /// they claim, [`RingError::TooBig`] for a frame bigger than the ring was
    /// sized for, [`RingError::NoSuchSlot`] for a slot that is not there.
    pub fn write_f32(
        &mut self,
        slot: Slot,
        pixels: &[f32],
        bounds: RectI,
        premultiplied: bool,
    ) -> Result<FrameHeader, RingError> {
        self.write_samples(
            slot,
            PixelDepth::F32,
            pixels.len(),
            bounds,
            premultiplied,
            |body| {
                for (cell, value) in body.chunks_exact_mut(4).zip(pixels) {
                    cell.copy_from_slice(&value.to_le_bytes());
                }
            },
        )
    }

    /// Put an fp16 picture in a slot - the working depth, crossing without
    /// being converted at either end.
    ///
    /// # Errors
    ///
    /// As [`Ring::write_f32`].
    pub fn write_f16(
        &mut self,
        slot: Slot,
        pixels: &[f16],
        bounds: RectI,
        premultiplied: bool,
    ) -> Result<FrameHeader, RingError> {
        self.write_samples(
            slot,
            PixelDepth::F16,
            pixels.len(),
            bounds,
            premultiplied,
            |body| {
                for (cell, value) in body.chunks_exact_mut(2).zip(pixels) {
                    cell.copy_from_slice(&value.to_le_bytes());
                }
            },
        )
    }

    /// The half both writers share: the bounds check, the fit, the fill, the
    /// hash and the header.
    fn write_samples(
        &mut self,
        slot: Slot,
        depth: PixelDepth,
        samples: usize,
        bounds: RectI,
        premultiplied: bool,
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<FrameHeader, RingError> {
        let (start, end) = self.range(slot)?;
        let expected = bounds.samples();
        if samples as u64 != expected {
            return Err(RingError::WrongSampleCount {
                given: samples as u64,
                expected,
            });
        }
        let payload_bytes = (samples as u64).saturating_mul(depth.bytes_per_sample());
        let needed = payload_bytes.saturating_add(HEADER_BYTES as u64);
        if needed > self.spec.slot_bytes {
            return Err(RingError::TooBig {
                needed,
                slot_bytes: self.spec.slot_bytes,
            });
        }

        let body_start = start.saturating_add(HEADER_BYTES);
        let body_end = body_start
            .saturating_add(usize::try_from(payload_bytes).unwrap_or(0))
            .min(end);
        {
            let body = self
                .map
                .get_mut(body_start..body_end)
                .ok_or(RingError::NoSuchSlot(slot))?;
            fill(body);
        }

        let hash = self
            .map
            .get(body_start..body_end)
            .map(hash_bytes)
            .unwrap_or_default();
        let row_bytes = u64::from(bounds.width())
            .saturating_mul(u64::from(CHANNELS))
            .saturating_mul(depth.bytes_per_sample());
        let header = FrameHeader {
            bounds,
            row_bytes: u32::try_from(row_bytes).unwrap_or(0),
            depth,
            premultiplied,
            payload_bytes,
            hash,
        };
        let bytes = header.to_bytes();
        if let Some(head) = self.map.get_mut(start..start.saturating_add(HEADER_BYTES)) {
            head.copy_from_slice(&bytes);
        }
        Ok(header)
    }

    /// Read a slot back as fp32.
    ///
    /// # Errors
    ///
    /// [`RingError::Empty`] for a slot nobody wrote, [`RingError::DepthMismatch`]
    /// for one written at the other depth, [`RingError::WrongPayloadBytes`] or
    /// [`RingError::WrongRowBytes`] for a header that disagrees with its own
    /// bounds, [`RingError::Corrupt`] if the hash does not match the bytes.
    pub fn read_f32(&self, slot: Slot) -> Result<(FrameHeader, Vec<f32>), RingError> {
        let (header, body) = self.body_of(slot, PixelDepth::F32)?;
        let mut out = Vec::with_capacity(body.len() / 4);
        for chunk in body.chunks_exact(4) {
            let bytes: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            out.push(f32::from_le_bytes(bytes));
        }
        Ok((header, out))
    }

    /// Read a slot back as fp16.
    ///
    /// # Errors
    ///
    /// As [`Ring::read_f32`].
    pub fn read_f16(&self, slot: Slot) -> Result<(FrameHeader, Vec<f16>), RingError> {
        let (header, body) = self.body_of(slot, PixelDepth::F16)?;
        let mut out = Vec::with_capacity(body.len() / 2);
        for chunk in body.chunks_exact(2) {
            let bytes: [u8; 2] = chunk.try_into().unwrap_or([0; 2]);
            out.push(f16::from_le_bytes(bytes));
        }
        Ok((header, out))
    }

    /// The header and the checked bytes of a slot, at the depth the reader
    /// asked for.
    ///
    /// **The header is checked against itself before it is believed.** The
    /// other end of this mapping is a stranger's compiled code, which is why
    /// the hash is here at all - and a hash only says that the bytes are the
    /// bytes the writer meant. It says nothing about a header whose bounds
    /// claim a 4K rectangle over sixty-four bytes of payload it hashed
    /// honestly. Since the header is the whole contract for the bytes after it,
    /// a caller walking the buffer by `bounds` or by `row_bytes` would run off
    /// the end of it, so the two numbers that describe the payload are held to
    /// what the bounds and the depth call for before a frame comes back at all.
    fn body_of(&self, slot: Slot, wanted: PixelDepth) -> Result<(FrameHeader, &[u8]), RingError> {
        let (start, end) = self.range(slot)?;
        let head = self
            .map
            .get(start..start.saturating_add(HEADER_BYTES))
            .ok_or(RingError::NoSuchSlot(slot))?;
        let header = FrameHeader::from_bytes(head)?;
        if header.depth != wanted {
            return Err(RingError::DepthMismatch {
                written: header.depth.name(),
                wanted: wanted.name(),
            });
        }
        let expected_payload = header
            .bounds
            .samples()
            .saturating_mul(wanted.bytes_per_sample());
        if header.payload_bytes != expected_payload {
            return Err(RingError::WrongPayloadBytes {
                given: header.payload_bytes,
                expected: expected_payload,
            });
        }
        let expected_row_bytes = u32::try_from(
            u64::from(header.bounds.width())
                .saturating_mul(u64::from(CHANNELS))
                .saturating_mul(wanted.bytes_per_sample()),
        )
        .unwrap_or(0);
        if header.row_bytes != expected_row_bytes {
            return Err(RingError::WrongRowBytes {
                given: header.row_bytes,
                expected: expected_row_bytes,
            });
        }
        // And the payload the header calls for is a payload this slot holds,
        // so the slice below is the whole of it rather than a clipped end the
        // hash was taken over.
        let needed = header.payload_bytes.saturating_add(HEADER_BYTES as u64);
        if needed > self.spec.slot_bytes {
            return Err(RingError::TooBig {
                needed,
                slot_bytes: self.spec.slot_bytes,
            });
        }

        let body_start = start.saturating_add(HEADER_BYTES);
        let body_end = body_start
            .saturating_add(usize::try_from(header.payload_bytes).unwrap_or(0))
            .min(end);
        let body = self
            .map
            .get(body_start..body_end)
            .ok_or(RingError::NoSuchSlot(slot))?;
        if hash_bytes(body) != header.hash {
            return Err(RingError::Corrupt);
        }
        Ok((header, body))
    }
}

/// Ask the ledger for the ring, and walk the slot count down until it says yes.
///
/// Halving rather than decrementing: the answer wanted is "how much of this can
/// the machine afford", and a ring of sixty-four slots refused one slot at a
/// time would ask sixty times. The floor is [`RING_MIN_SLOTS`], and it is taken
/// whether or not the ledger grants it - the denial stands on the ledger's own
/// count, which is what makes taking it visible rather than silent.
fn reserve_slots(wanted: u32, slot_bytes: u64, ledger: &Arc<Ledger>) -> (u32, Option<Reservation>) {
    let mut slots = wanted.max(RING_MIN_SLOTS);
    loop {
        let bytes = slot_bytes.saturating_mul(u64::from(slots));
        if let Some(reservation) = ledger.try_reserve(Tier::Ram, bytes) {
            return (slots, Some(reservation));
        }
        if slots <= RING_MIN_SLOTS {
            return (RING_MIN_SLOTS, None);
        }
        slots = (slots / 2).max(RING_MIN_SLOTS);
    }
}

/// Map a file into this process, shared with everyone else who maps it.
#[allow(unsafe_code)]
fn map_file(file: &File) -> Result<MmapMut, RingError> {
    // SAFETY: the file is one this process just made or was told the name of by
    // the process that made it; nothing else writes it except the broker at the
    // other end of the pipe, which is exactly the sharing that is wanted. The
    // mapping's length is the file's, so every read through it is in bounds.
    // This is the crate's one `unsafe`, and it is the one the sibling hosts
    // spell the same way.
    let map = unsafe { MmapMut::map_mut(file) }?;
    Ok(map)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A ledger with room for anything, which is what a test that is not about
    /// the ledger wants.
    fn open_ledger() -> Arc<Ledger> {
        Ledger::with_budgets(1 << 30, 1 << 30)
    }

    /// A path in the temporary directory nobody else is using.
    fn ring_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lumit-lfx-test-{}-{}-{name}.ring",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ))
    }

    /// A ramp, so that a frame that came back wrong comes back *visibly* wrong
    /// rather than plausibly.
    fn ramp(samples: usize) -> Vec<f32> {
        (0..samples).map(|n| n as f32 * 0.25).collect()
    }

    /// **Both depths, end to end** (§14 item 6), the fp16 half: the working
    /// depth crosses the ring and comes back the same bits it went in as. Not
    /// "close": fp16 in and fp16 out is the promise docs/12 §3.3 makes, and a
    /// conversion at either end would show up here as a rounding.
    #[test]
    fn an_fp16_frame_crosses_the_ring_and_comes_back_unchanged() {
        let path = ring_path("fp16");
        let ledger = open_ledger();
        let plan = RingPlan::frame(8, 4, PixelDepth::F16);
        let mut ring = Ring::create(&path, plan, &ledger).expect("a ring");
        let bounds = RectI::of(8, 4);
        let pixels: Vec<f16> = ramp(bounds.samples() as usize)
            .into_iter()
            .map(f16::from_f32)
            .collect();

        let written = ring
            .write_f16(0, &pixels, bounds, true)
            .expect("a frame goes in");
        let (header, back) = ring.read_f16(0).expect("and comes out");
        assert_eq!(header, written, "the reader read the writer's own header");
        assert_eq!(back, pixels, "the picture changed crossing the ring");
        assert_eq!(header.depth, PixelDepth::F16);
        assert!(header.premultiplied);
        assert_eq!(header.bounds, bounds);
        assert_eq!(header.row_bytes, 8 * 4 * 2);
    }

    /// The fp32 half of the same item.
    #[test]
    fn an_fp32_frame_crosses_the_ring_and_comes_back_unchanged() {
        let path = ring_path("fp32");
        let ledger = open_ledger();
        let plan = RingPlan::frame(8, 4, PixelDepth::F32);
        let mut ring = Ring::create(&path, plan, &ledger).expect("a ring");
        let bounds = RectI::of(8, 4);
        let pixels = ramp(bounds.samples() as usize);

        let written = ring
            .write_f32(1, &pixels, bounds, true)
            .expect("a frame goes in");
        let (header, back) = ring.read_f32(1).expect("and comes out");
        assert_eq!(header, written);
        assert_eq!(back, pixels);
        assert_eq!(header.depth, PixelDepth::F32);
        assert_eq!(header.row_bytes, 8 * 4 * 4);
    }

    /// A slot written at one depth and read at the other is **refused**, both
    /// ways round (§14 item 6). It is the one mistake shared memory would not
    /// otherwise announce: the bytes are there, the hash matches them, and
    /// reading fp16 pairs as fp32 quadruples hands back a picture of noise with
    /// nothing wrong with it.
    #[test]
    fn a_slot_written_at_one_depth_and_read_at_the_other_is_refused() {
        let path = ring_path("depths");
        let ledger = open_ledger();
        // An fp32-sized ring holds a frame of either depth, which is exactly
        // why the slot has to say which one it holds.
        let mut ring =
            Ring::create(&path, RingPlan::frame(4, 4, PixelDepth::F32), &ledger).expect("a ring");
        let bounds = RectI::of(4, 4);
        let wide = ramp(bounds.samples() as usize);
        let narrow: Vec<f16> = wide.iter().copied().map(f16::from_f32).collect();

        ring.write_f32(0, &wide, bounds, true).expect("fp32 in");
        ring.write_f16(1, &narrow, bounds, true).expect("fp16 in");

        let wrong = ring.read_f16(0).expect_err("fp32 read as fp16");
        assert!(
            matches!(
                wrong,
                RingError::DepthMismatch {
                    written: "fp32",
                    wanted: "fp16"
                }
            ),
            "{wrong}"
        );
        let other_way = ring.read_f32(1).expect_err("fp16 read as fp32");
        assert!(
            matches!(
                other_way,
                RingError::DepthMismatch {
                    written: "fp16",
                    wanted: "fp32"
                }
            ),
            "{other_way}"
        );
        // And each still reads correctly at its own depth.
        assert_eq!(ring.read_f32(0).expect("fp32 out").1, wide);
        assert_eq!(ring.read_f16(1).expect("fp16 out").1, narrow);
    }

    /// **The ring is charged to the ledger and given back when it is dropped**
    /// (§14 item 6). docs/13 §3's rule, which neither older host keeps: the
    /// reservation rides in the same struct as the mapping, so the bytes go
    /// back through its destructor rather than through somebody remembering.
    ///
    /// The name says dropped rather than *when the broker dies* because there
    /// is no broker in this module; the broker's death is what drops the ring,
    /// and that half is `ipc::broker`'s own `Drop`.
    #[test]
    fn the_ring_is_charged_to_the_ledger_and_given_back_when_it_is_dropped() {
        let path = ring_path("ledger");
        let ledger = Ledger::with_budgets(1 << 30, 1 << 30);
        assert_eq!(ledger.used(Tier::Ram), 0);

        let plan = RingPlan::frame(64, 64, PixelDepth::F16);
        let ring = Ring::create(&path, plan, &ledger).expect("a ring");
        let held = u64::from(ring.slots()).saturating_mul(plan.slot_bytes());
        assert_eq!(
            ledger.used(Tier::Ram),
            held,
            "the ledger holds every byte of the mapping"
        );
        assert_eq!(ring.reserved_bytes(), held);

        drop(ring);
        assert_eq!(
            ledger.used(Tier::Ram),
            0,
            "the bytes came back when the ring went"
        );
        assert!(!path.exists(), "and so did the file");
    }

    /// **A ledger with no room buys three slots rather than none** (§14 item 6,
    /// §3.4). The floor is the ledger's one veto it does not get: a ring of two
    /// cannot hold an input, an output and one in flight at once, so the bundle
    /// is either hosted at three slots or not hosted at all - and taking them
    /// over the ledger's head is recorded as a denial rather than passed over.
    #[test]
    fn a_ledger_with_no_room_buys_three_slots_rather_than_none() {
        let path = ring_path("squeezed");
        let ledger = Ledger::with_budgets(1 << 20, 0);
        let plan = RingPlan::frame(32, 32, PixelDepth::F32);

        let ring = Ring::create(&path, plan, &ledger).expect("a ring, even with no room");
        assert_eq!(ring.slots(), RING_MIN_SLOTS);
        assert_eq!(
            ring.reserved_bytes(),
            0,
            "the floor was taken, so nothing was granted for it"
        );
        assert!(
            ledger.denials(Tier::Ram) > 0,
            "the ledger records the floor being taken over its head"
        );

        // And the ring works: a frame crosses it as it would any other.
        let mut ring = ring;
        let bounds = RectI::of(32, 32);
        let pixels = ramp(bounds.samples() as usize);
        ring.write_f32(2, &pixels, bounds, true).expect("a frame");
        assert_eq!(ring.read_f32(2).expect("back").1, pixels);
    }

    /// Under pressure the count is walked down rather than refused: a ledger
    /// with room for part of the ring buys the part it has room for, and the
    /// bytes it granted are the bytes the ring is.
    #[test]
    fn a_ledger_with_some_room_buys_the_slots_it_can_afford() {
        let path = ring_path("walked-down");
        let plan = RingPlan::frame(64, 64, PixelDepth::F32);
        let wanted = plan.slots_wanted();
        assert!(
            wanted > RING_MIN_SLOTS * 2,
            "the walk needs somewhere to go"
        );
        // Room for a quarter of what is asked for, and not a byte more.
        let ledger = Ledger::with_budgets(1 << 20, plan.slot_bytes() * u64::from(wanted / 4));

        let ring = Ring::create(&path, plan, &ledger).expect("a ring");
        assert!(ring.slots() < wanted, "the whole ring was not affordable");
        assert!(ring.slots() >= RING_MIN_SLOTS);
        assert_eq!(
            ring.reserved_bytes(),
            u64::from(ring.slots()) * plan.slot_bytes(),
            "what the ledger holds is what the ring is"
        );
    }

    /// A slot is sized by the frame's depth, which is the first of §3.4's two
    /// changes: an fp16 slot is exactly half an fp32 one, so the same budget
    /// buys twice as many of them.
    #[test]
    fn an_fp16_slot_is_half_an_fp32_slot() {
        for (w, h) in [(1920, 1080), (3840, 2160), (7, 3)] {
            let wide = slot_bytes_for(w, h, PixelDepth::F32) - HEADER_BYTES as u64;
            let narrow = slot_bytes_for(w, h, PixelDepth::F16) - HEADER_BYTES as u64;
            assert_eq!(wide, narrow * 2, "{w}x{h}");
        }
        assert_eq!(
            slot_bytes_for(1920, 1080, PixelDepth::F32),
            1920 * 1080 * 4 * 4 + 64,
            "w x h x 4 x depth_bytes + 64, exactly"
        );
    }

    /// The table in docs/impl/lfx.md §3.4, generated from the arithmetic rather
    /// than copied into prose - which is how the OFX host's own comment came to
    /// be off by one in both of its cells (docs/impl/lfx.md §13).
    #[test]
    fn the_ring_slot_table_matches_the_rule() {
        let want = rendered_table();
        let got = std::fs::read_to_string(table_path()).unwrap_or_default();
        assert_eq!(
            got, want,
            "ring-slots.txt is stale. Regenerate it:\n  cargo test -p lumit-lfx \
             regenerate_ring_slot_table -- --ignored"
        );
    }

    /// The numbers the note prints, spelled out here as well, so that a change
    /// to the budget or to the header size is a failure with the old numbers in
    /// it rather than a fixture that quietly agrees with itself.
    #[test]
    fn the_budget_buys_sixteen_slots_at_1080p_and_four_at_4k() {
        for (w, h, depth, slots) in [
            (1920, 1080, PixelDepth::F32, 16),
            (1920, 1080, PixelDepth::F16, 32),
            (3840, 2160, PixelDepth::F32, 4),
            (3840, 2160, PixelDepth::F16, 8),
        ] {
            assert_eq!(
                slots_for(w, h, depth, (0, 0)),
                slots,
                "{w}x{h} at {}",
                depth.name()
            );
        }
    }

    /// The second of §3.4's answers, and the one only LFX can give: the ring is
    /// sized from the **declared** window as well as from the budget, so a
    /// retimer's `t ± 5` asks for its eleven frames plus an output before the
    /// first frame rather than discovering the refusal at one. A plugin that
    /// declares nothing asks for nothing extra.
    #[test]
    fn the_declared_window_asks_for_the_slots_the_budget_would_not_buy() {
        let uhd = RingPlan::frame(3840, 2160, PixelDepth::F16);
        assert_eq!(uhd.slots_wanted(), 8, "the budget's own answer");
        assert_eq!(
            uhd.reading((-5, 5)).slots_wanted(),
            12,
            "eleven frames the plugin reads, and the one being written"
        );
        // A window inside what the budget already bought changes nothing.
        assert_eq!(uhd.reading((-1, 1)).slots_wanted(), 8);
        // And nobody may ask for more than the ceiling.
        assert_eq!(
            uhd.reading((-64, 64)).slots_wanted(),
            RING_MAX_SLOTS,
            "a window wider than the ring may be is held to the ring"
        );
        // A frame so large that even one slot is over budget still gets the
        // floor: three slots, and a plugin that declared nothing knows why.
        let enormous = RingPlan::frame(30_000, 30_000, PixelDepth::F32);
        assert_eq!(enormous.slots_wanted(), RING_MIN_SLOTS);
    }

    /// The failure shared memory has, made loud: a slot whose bytes changed
    /// under the header is a fault rather than a picture.
    #[test]
    fn a_frame_that_arrived_changed_is_noticed() {
        let path = ring_path("corrupt");
        let ledger = open_ledger();
        let mut ring =
            Ring::create(&path, RingPlan::frame(4, 4, PixelDepth::F32), &ledger).expect("a ring");
        let bounds = RectI::of(4, 4);
        let pixels = ramp(bounds.samples() as usize);
        ring.write_f32(0, &pixels, bounds, true).expect("a frame");

        // One byte of the payload, changed behind the header's back.
        if let Some(byte) = ring.map.get_mut(HEADER_BYTES) {
            *byte ^= 0xff;
        }
        assert!(matches!(
            ring.read_f32(0).expect_err("a changed frame"),
            RingError::Corrupt
        ));
    }

    /// The half of that failure the hash cannot see: a header is checked
    /// against **itself** before it is believed. The far end of the mapping is
    /// a stranger's compiled code, which can write the block directly and can
    /// hash whatever it wrote - FNV is nobody's secret. A slot whose bounds
    /// claim a rectangle its payload does not hold would otherwise come back as
    /// a frame, and a caller walking those bounds would walk off the end of the
    /// buffer it was handed.
    #[test]
    fn a_header_that_lies_about_its_size_is_not_a_frame() {
        let path = ring_path("lying-header");
        let ledger = open_ledger();
        let mut ring =
            Ring::create(&path, RingPlan::frame(4, 4, PixelDepth::F32), &ledger).expect("a ring");
        let bounds = RectI::of(4, 4);
        let pixels = ramp(bounds.samples() as usize);
        ring.write_f32(0, &pixels, bounds, true)
            .expect("a good frame");
        assert!(ring.read_f32(0).is_ok(), "which reads back as one");

        // Only the header is rewritten. The payload and the hash over it stay
        // the honest ones the writer left, so the corruption check has nothing
        // to say and these are the checks that do.
        let poke32 = |ring: &mut Ring, offset: usize, value: u32| {
            if let Some(cell) = ring.map.get_mut(offset..offset + 4) {
                cell.copy_from_slice(&value.to_le_bytes());
            }
        };
        let poke64 = |ring: &mut Ring, offset: usize, value: u64| {
            if let Some(cell) = ring.map.get_mut(offset..offset + 8) {
                cell.copy_from_slice(&value.to_le_bytes());
            }
        };

        // A 4K rectangle over sixteen pixels of payload.
        poke32(&mut ring, 16, 3840);
        poke32(&mut ring, 20, 2160);
        let lie = ring.read_f32(0).expect_err("bounds nothing holds");
        assert!(
            matches!(
                lie,
                RingError::WrongPayloadBytes {
                    given: 256,
                    expected: 132_710_400
                }
            ),
            "{lie}"
        );

        // Honest bounds again, and a stride four times the row it describes.
        poke32(&mut ring, 16, 4);
        poke32(&mut ring, 20, 4);
        poke32(&mut ring, 24, 4 * 4 * 4 * 4);
        let stride = ring
            .read_f32(0)
            .expect_err("rows nothing is that far apart");
        assert!(
            matches!(
                stride,
                RingError::WrongRowBytes {
                    given: 256,
                    expected: 64
                }
            ),
            "{stride}"
        );

        // And a header that agrees with itself about a frame twice the size of
        // the slot it is sitting in: the payload the bounds call for is not a
        // payload this slot holds, so the hash is never taken over a clipped
        // end of it.
        poke32(&mut ring, 16, 8);
        poke32(&mut ring, 20, 8);
        poke32(&mut ring, 24, 8 * 4 * 4);
        poke64(&mut ring, 40, 8 * 8 * 4 * 4);
        let over = ring.read_f32(0).expect_err("a frame bigger than its slot");
        assert!(matches!(over, RingError::TooBig { .. }), "{over}");
    }

    /// The claim the module exists to make: the two processes agree on one
    /// block of memory that is **the same memory** in both of them. Proved
    /// across two mappings in one process, which is exactly what the broker's
    /// side is - it opens the spec the host sends it and pays for nothing. The
    /// second *process* is `lumit-lfx-broker`'s own suite, in
    /// `both_depths_cross_the_ring_between_two_processes_unchanged`.
    #[test]
    fn a_second_mapping_of_the_same_ring_reads_what_the_first_wrote() {
        let path = ring_path("two-mappings");
        let ledger = open_ledger();
        let mut ring =
            Ring::create(&path, RingPlan::frame(8, 4, PixelDepth::F16), &ledger).expect("a ring");
        let bounds = RectI::of(8, 4);
        let pixels: Vec<f16> = ramp(bounds.samples() as usize)
            .into_iter()
            .map(f16::from_f32)
            .collect();
        let written = ring
            .write_f16(2, &pixels, bounds, true)
            .expect("a frame in");

        let opened = Ring::open(ring.spec()).expect("the same block, mapped again");
        let (header, back) = opened.read_f16(2).expect("read through the other mapping");
        assert_eq!(header, written, "the header crossed whole");
        assert_eq!(back, pixels, "and so did the pixels");
        assert_eq!(
            opened.reserved_bytes(),
            0,
            "the broker's side pays for nothing: the ledger is the host's"
        );
        assert_eq!(
            ledger.used(Tier::Ram),
            ring.reserved_bytes(),
            "and one block is charged once, however many mappings of it there are"
        );
        assert!(
            matches!(
                opened.read_f16(1).expect_err("nobody wrote that one"),
                RingError::Empty
            ),
            "a slot nobody wrote is empty through either mapping"
        );
    }

    /// The sixty-four bytes, by number, from the side that writes them - the
    /// discipline `lumit-lfx-abi` keeps for the header it shares with a
    /// stranger, kept here for the one it shares with the broker.
    /// [`FrameHeader::to_bytes`] and [`FrameHeader::from_bytes`] otherwise
    /// agree only because one author wrote both.
    #[test]
    fn the_slot_header_is_sixty_four_bytes_in_this_order() {
        let header = FrameHeader {
            bounds: RectI {
                x0: -3,
                y0: -5,
                x1: 13,
                y1: 9,
            },
            row_bytes: 16 * 4 * 2,
            depth: PixelDepth::F16,
            premultiplied: true,
            payload_bytes: 16 * 14 * 4 * 2,
            hash: 0x0123_4567_89ab_cdef,
        };
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), HEADER_BYTES, "sixty-four, and no more");

        let u32_at = |offset: usize| -> u32 {
            let cell: [u8; 4] = bytes
                .get(offset..offset + 4)
                .and_then(|slice| slice.try_into().ok())
                .expect("four bytes");
            u32::from_le_bytes(cell)
        };
        let u64_at = |offset: usize| -> u64 {
            let cell: [u8; 8] = bytes
                .get(offset..offset + 8)
                .and_then(|slice| slice.try_into().ok())
                .expect("eight bytes");
            u64::from_le_bytes(cell)
        };
        #[allow(clippy::cast_possible_wrap)]
        let i32_at = |offset: usize| -> i32 { u32_at(offset) as i32 };

        assert_eq!(u32_at(0), HEADER_MAGIC, "0: LFXR, and not the OFX ring's");
        assert_eq!(u32_at(4), HEADER_VERSION, "4: the header's own version");
        assert_eq!(i32_at(8), -3, "8: x0");
        assert_eq!(i32_at(12), -5, "12: y0");
        assert_eq!(i32_at(16), 13, "16: x1");
        assert_eq!(i32_at(20), 9, "20: y1");
        assert_eq!(u32_at(24), 16 * 4 * 2, "24: row bytes");
        assert_eq!(
            u32_at(28),
            lumit_lfx_abi::LFX_RGBA_F16,
            "28: the header's own pixel format, not this crate's discriminant"
        );
        assert_eq!(u32_at(32), 1, "32: premultiplied");
        assert_eq!(u32_at(36), 0, "36: padding, so the two 64-bit fields align");
        assert_eq!(u64_at(40), 16 * 14 * 4 * 2, "40: payload bytes");
        assert_eq!(u64_at(48), 0x0123_4567_89ab_cdef, "48: the hash");
        assert_eq!(u64_at(56), 0, "56: the tail a later field grows into");

        assert_eq!(
            FrameHeader::from_bytes(&bytes).expect("and back"),
            header,
            "the two halves read the same bytes the same way"
        );
    }

    /// A slot nobody wrote holds no frame, and neither does one written by
    /// something that is not this protocol. The magic is the ring's own, so a
    /// slot from another host's ring is empty rather than plausible.
    #[test]
    fn a_slot_nobody_wrote_holds_no_frame() {
        let path = ring_path("empty");
        let ledger = open_ledger();
        let mut ring =
            Ring::create(&path, RingPlan::frame(4, 4, PixelDepth::F32), &ledger).expect("a ring");
        assert!(matches!(
            ring.read_f32(0).expect_err("nothing"),
            RingError::Empty
        ));
        assert!(matches!(
            ring.read_f32(ring.slots()).expect_err("off the end"),
            RingError::NoSuchSlot(_)
        ));

        // Somebody else's magic.
        if let Some(head) = ring.map.get_mut(0..4) {
            head.copy_from_slice(&0x4c4f_4658_u32.to_le_bytes());
        }
        assert!(matches!(
            ring.read_f32(0).expect_err("not ours"),
            RingError::Empty
        ));
    }

    /// A frame bigger than a slot is refused rather than written over the slot
    /// after it, and a picture that is not the rectangle it claims is refused
    /// before anything is written at all.
    #[test]
    fn a_frame_bigger_than_a_slot_is_refused() {
        let path = ring_path("toobig");
        let ledger = open_ledger();
        let mut ring =
            Ring::create(&path, RingPlan::frame(4, 4, PixelDepth::F32), &ledger).expect("a ring");

        let big = RectI::of(64, 64);
        let pixels = ramp(big.samples() as usize);
        let refused = ring.write_f32(0, &pixels, big, true).expect_err("too big");
        assert!(matches!(refused, RingError::TooBig { .. }), "{refused}");

        let ragged = ring
            .write_f32(0, &[0.0; 3], RectI::of(4, 4), true)
            .expect_err("three samples are not a 4x4 picture");
        assert!(
            matches!(
                ragged,
                RingError::WrongSampleCount {
                    given: 3,
                    expected: 64
                }
            ),
            "{ragged}"
        );
    }

    /// The name is claimed, never cleared: a ring refuses a path that already
    /// exists, so a planted file - or a planted symbolic link - is an error
    /// rather than something to truncate.
    #[test]
    fn a_ring_never_opens_a_path_that_already_exists() {
        let path = ring_path("taken");
        std::fs::write(&path, b"somebody else's").expect("a file in the way");
        let ledger = open_ledger();
        let refused = Ring::create(&path, RingPlan::frame(4, 4, PixelDepth::F16), &ledger)
            .expect_err("the path is taken");
        assert!(matches!(refused, RingError::Io(_)), "{refused}");
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            b"somebody else's",
            "and what was there was not truncated"
        );
        assert_eq!(
            ledger.used(Tier::Ram),
            0,
            "a ring that was never made holds nothing: the reservation is taken \
             before the file is opened, so every road out of `create` gives it back"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Once the mapping is held the name is taken out of the directory, which
    /// is what has the kernel reclaim the ring on a crash or a `kill -9` rather
    /// than on a destructor nobody reached. A Unix mapping is of the file and
    /// not of its name, so the ring goes on working after it - and the ordering
    /// rule [`Ring::unlink_now_it_is_shared`]'s own doc gives is pinned here as
    /// well as described: after the unlink there is no name left for a second
    /// end to open, which is why the host waits for `RingOpened` first.
    #[cfg(unix)]
    #[test]
    fn a_ring_goes_on_working_after_its_name_is_removed() {
        let path = ring_path("unlinked");
        let ledger = open_ledger();
        let mut ring =
            Ring::create(&path, RingPlan::frame(4, 4, PixelDepth::F16), &ledger).expect("a ring");
        assert!(path.exists(), "the broker has a name to open");
        let spec = ring.spec().clone();
        drop(Ring::open(&spec).expect("and can open it while it is there"));

        ring.unlink_now_it_is_shared();
        assert!(!path.exists(), "and then it has not");
        assert!(
            Ring::open(&spec).is_err(),
            "a name that has gone is a ring nobody else can join - hence never before RingOpened"
        );

        let bounds = RectI::of(4, 4);
        let pixels: Vec<f16> = ramp(bounds.samples() as usize)
            .into_iter()
            .map(f16::from_f32)
            .collect();
        ring.write_f16(0, &pixels, bounds, true)
            .expect("the mapping outlives the name");
        assert_eq!(ring.read_f16(0).expect("back").1, pixels);
    }

    // ------------------------------------------------- the generated table --

    /// Where the fixture lives: the crate root, beside `Cargo.toml`, as
    /// `fx-labels.txt` sits beside `lumit-core`'s.
    fn table_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("ring-slots.txt")
    }

    /// The sizes the table is worked out over: the two docs/impl/lfx.md §3.4
    /// prints, and the two either side of them that say which way the rule
    /// runs.
    const TABLE_SIZES: [(&str, u32, u32); 4] = [
        ("720p", 1280, 720),
        ("1080p", 1920, 1080),
        ("UHD 4K", 3840, 2160),
        ("8K", 7680, 4320),
    ];

    fn rendered_table() -> String {
        let mut out = String::new();
        out.push_str(
            "# How many slots a ring buys, by frame size and depth.\n\
             # Generated - do not edit. To refresh after a change to the rule:\n\
             #   cargo test -p lumit-lfx regenerate_ring_slot_table -- --ignored\n\
             #\n\
             # slots = clamp(floor(RING_BUDGET_BYTES / slot_bytes), RING_MIN_SLOTS, RING_MAX_SLOTS)\n\
             # slot_bytes = w x h x 4 x depth_bytes + HEADER_BYTES\n\
             # A plugin's declared window asks for hi - lo + 2 and may raise, never lower.\n",
        );
        out.push_str(&format!(
            "#\n# RING_BUDGET_BYTES = {RING_BUDGET_BYTES}, HEADER_BYTES = {HEADER_BYTES}, \
             slots in [{RING_MIN_SLOTS}, {RING_MAX_SLOTS}]\n\n"
        ));
        out.push_str("| | fp32 slots | fp16 slots |\n|---|---|---|\n");
        for (name, w, h) in TABLE_SIZES {
            out.push_str(&format!(
                "| {name} | {} | {} |\n",
                slots_for(w, h, PixelDepth::F32, (0, 0)),
                slots_for(w, h, PixelDepth::F16, (0, 0)),
            ));
        }
        out.push_str("\nAnd what one declared window costs, at UHD 4K:\n\n");
        out.push_str("| window | fp32 slots | fp16 slots |\n|---|---|---|\n");
        for window in [(0, 0), (-1, 1), (-5, 5), (-32, 32)] {
            out.push_str(&format!(
                "| {} .. {} | {} | {} |\n",
                window.0,
                window.1,
                slots_for(3840, 2160, PixelDepth::F32, window),
                slots_for(3840, 2160, PixelDepth::F16, window),
            ));
        }
        out
    }

    #[test]
    #[ignore = "writes the fixture; run after a change to the rule"]
    fn regenerate_ring_slot_table() {
        std::fs::write(table_path(), rendered_table()).expect("write ring-slots.txt");
    }
}
