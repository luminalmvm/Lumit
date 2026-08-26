//! Handles: the opaque pointers a plugin holds, and how a forged one is
//! caught rather than followed.
//!
//! # In plain terms
//!
//! OFX plugins refer to the host's objects by "handles", which the C API
//! declares as pointers. A host that mints those handles by handing out the
//! address of a real Rust object is one buggy plugin away from a crash: the
//! plugin keeps a handle after the object is gone, or invents one, and the
//! host follows a pointer into nothing.
//!
//! So none of our handles is a pointer. Each is a number with three parts
//! packed into it — a fixed *magic* pattern, a *kind* saying what sort of
//! object it names, and an *index* into a list the host keeps. Every lookup
//! checks all three. A number a plugin invented almost certainly fails the
//! magic; one it kept too long fails the occupancy check, because indices are
//! never handed out twice. The answer in every failing case is
//! `kOfxStatErrBadHandle` — a code the plugin is required to expect — and not
//! a crash.

use std::ffi::c_void;

use crate::status::Status;

/// The magic pattern in the top sixteen bits. It exists to make a number a
/// plugin made up fail the first check, not to withstand an attacker: a
/// hostile plugin already runs our code (docs/12 §5 puts the real boundary at
/// the process edge).
const HANDLE_MAGIC: usize = 0x1F0C;

/// Bits reserved for the index.
const INDEX_BITS: u32 = 40;
const INDEX_MASK: usize = (1 << INDEX_BITS) - 1;
const KIND_SHIFT: u32 = INDEX_BITS;
const MAGIC_SHIFT: u32 = INDEX_BITS + 8;

/// What sort of object a handle names. [`HandleKind::Clip`] has no registry
/// yet; it is named so that a lookup of the wrong kind is a thing that can be
/// written, tested, and rejected before the object it names exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HandleKind {
    /// An `OfxPropertySetHandle`.
    PropertySet = 1,
    /// An `OfxImageEffectHandle`.
    ImageEffect = 2,
    /// An `OfxParamHandle`.
    Param = 3,
    /// An `OfxImageClipHandle`.
    Clip = 4,
    /// An `OfxParamSetHandle` — the bag of parameters hanging off one effect.
    ///
    /// It has **no registry of its own**: a param set is not a separate object,
    /// it is one face of an effect, so its handle carries the same index as the
    /// effect's and differs only in this kind. That is what
    /// [`Handle::recast`] is for. Keeping the kind distinct rather than handing
    /// the effect handle straight back is what stops an effect handle being
    /// accepted where a param set was meant, and the other way round.
    ParamSet = 5,
    /// An `OfxMutexHandle` — a lock the host holds on a plugin's behalf
    /// ([`crate::suites::multi_thread`]).
    Mutex = 6,
}

impl HandleKind {
    const fn bits(self) -> usize {
        self as usize
    }
}

/// A handle, as the host understands it. The plugin sees the same bits as a
/// pointer and must never dereference them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(usize);

impl Handle {
    /// Build the handle for `kind` at `index`, or `None` if the index space
    /// for that kind is exhausted (2^40 of them; see [`HandleRegistry`]).
    #[must_use]
    pub const fn encode(kind: HandleKind, index: usize) -> Option<Self> {
        if index > INDEX_MASK {
            return None;
        }
        Some(Self(
            (HANDLE_MAGIC << MAGIC_SHIFT) | (kind.bits() << KIND_SHIFT) | index,
        ))
    }

    /// The bits as the C API carries them.
    #[must_use]
    pub const fn as_ptr(self) -> *mut c_void {
        self.0 as *mut c_void
    }

    /// Take the bits back off the C API. This never dereferences anything.
    #[must_use]
    pub fn from_ptr(ptr: *const c_void) -> Self {
        Self(ptr as usize)
    }

    /// The raw bits, for tests that need to forge one.
    #[must_use]
    pub const fn bits(self) -> usize {
        self.0
    }

    /// The kind this handle claims to be, if it is one of ours at all.
    #[must_use]
    pub const fn kind(self) -> Option<HandleKind> {
        if (self.0 >> MAGIC_SHIFT) != HANDLE_MAGIC {
            return None;
        }
        match (self.0 >> KIND_SHIFT) & 0xff {
            1 => Some(HandleKind::PropertySet),
            2 => Some(HandleKind::ImageEffect),
            3 => Some(HandleKind::Param),
            4 => Some(HandleKind::Clip),
            5 => Some(HandleKind::ParamSet),
            6 => Some(HandleKind::Mutex),
            _ => None,
        }
    }

    /// The same index under another kind, or `None` if this is not one of ours.
    ///
    /// The one use is the effect/param-set pair (see [`HandleKind::ParamSet`]):
    /// two names for two faces of one object, so the second name is the first
    /// with the kind swapped rather than a second registry to keep in step.
    #[must_use]
    pub const fn recast(self, kind: HandleKind) -> Option<Self> {
        if self.kind().is_none() {
            return None;
        }
        Self::encode(kind, self.index())
    }

    const fn index(self) -> usize {
        self.0 & INDEX_MASK
    }
}

/// The host's list of one kind of object.
///
/// Slots are **never reused**. That is what makes a stale handle detectable:
/// if index 7 could come back as a different property set, a plugin holding
/// the old handle would silently read the new object. The cost is that the
/// list only grows within a session, which is bounded by the 2^40 indices a
/// handle can carry — an editing session that creates a thousand million
/// property sets a second would need a month to reach it.
pub struct HandleRegistry<T> {
    kind: HandleKind,
    slots: Vec<Option<T>>,
}

impl<T> HandleRegistry<T> {
    /// An empty registry for one kind of object.
    #[must_use]
    pub const fn new(kind: HandleKind) -> Self {
        Self {
            kind,
            slots: Vec::new(),
        }
    }

    /// How many live objects it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Take ownership of `value` and mint the handle that names it.
    ///
    /// # Errors
    ///
    /// [`Status::ErrMemory`] if the index space for this kind is exhausted.
    pub fn insert(&mut self, value: T) -> Result<Handle, Status> {
        let index = self.slots.len();
        let handle = Handle::encode(self.kind, index).ok_or(Status::ErrMemory)?;
        self.slots.push(Some(value));
        Ok(handle)
    }

    /// Resolve a handle to an index, checking magic, kind, range and
    /// occupancy in that order.
    fn resolve(&self, handle: Handle) -> Result<usize, Status> {
        if handle.kind() != Some(self.kind) {
            return Err(Status::ErrBadHandle);
        }
        let index = handle.index();
        match self.slots.get(index) {
            Some(Some(_)) => Ok(index),
            _ => Err(Status::ErrBadHandle),
        }
    }

    /// The object a handle names.
    ///
    /// # Errors
    ///
    /// [`Status::ErrBadHandle`] for a forged, stale or wrong-kind handle.
    pub fn get(&self, handle: Handle) -> Result<&T, Status> {
        let index = self.resolve(handle)?;
        self.slots
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Status::ErrBadHandle)
    }

    /// The object a handle names, for writing.
    ///
    /// # Errors
    ///
    /// As [`HandleRegistry::get`].
    pub fn get_mut(&mut self, handle: Handle) -> Result<&mut T, Status> {
        let index = self.resolve(handle)?;
        self.slots
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or(Status::ErrBadHandle)
    }

    /// Destroy the object a handle names. The handle is dead from here on and
    /// its index is never issued again.
    ///
    /// # Errors
    ///
    /// As [`HandleRegistry::get`].
    pub fn remove(&mut self, handle: Handle) -> Result<T, Status> {
        let index = self.resolve(handle)?;
        self.slots
            .get_mut(index)
            .and_then(Option::take)
            .ok_or(Status::ErrBadHandle)
    }
}
