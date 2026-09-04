//! Handles: the numbers that name an instance across the pipe, and how a wrong
//! one is caught rather than followed.
//!
//! # In plain terms
//!
//! The host names each live plugin by a number, and every message about that
//! plugin quotes the number. A number is a thing that can be wrong: a message
//! for an instance that was destroyed, a message replayed after a restart, a
//! number that never named anything. So the number is not a plain counter —
//! it carries a fixed **magic** pattern and a **kind** alongside its index, and
//! every lookup checks all three before it looks anything up.
//!
//! Nothing here can be undefined behaviour the way a forged OFX handle could
//! be: these are `u32`s into a map, not pointers. The check is here because the
//! *answer* matters — a bad handle must come back as one calm
//! [`BrokerMessage::Failed`](crate::ipc::proto::BrokerMessage::Failed) sentence
//! rather than as a block silently processed by the wrong plugin, which is the
//! failure that would be found six months later in an export.

use std::collections::BTreeMap;

/// The magic pattern in the top eight bits. It exists so that a number nobody
/// minted fails the first check, not to withstand an attacker: the broker
/// process is the real boundary.
const HANDLE_MAGIC: u32 = 0xA7;

/// Bits reserved for the index — a million instances in one session, which is
/// four hundred years of adding one effect a second.
const INDEX_BITS: u32 = 20;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
const KIND_SHIFT: u32 = INDEX_BITS;
const KIND_MASK: u32 = 0xf;
const MAGIC_SHIFT: u32 = INDEX_BITS + 4;

/// A live plugin instance.
///
/// The kind field has one value today and is not folded away, because the
/// second one is already named: the plugin's own editor window, which the
/// broker will own and the host will refer to by a handle of its own
/// (docs/impl/audio-plugins.md §6). A kind checked from the first day is a kind
/// that cannot be confused with an instance on the day it arrives.
pub const KIND_INSTANCE: u32 = 1;

/// One handle, as it crosses the pipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(u32);

impl Handle {
    /// Build the handle for `kind` at `index`, or `None` when either is past
    /// the bits it has.
    #[must_use]
    pub const fn encode(kind: u32, index: u32) -> Option<Self> {
        if index > INDEX_MASK || kind > KIND_MASK || kind == 0 {
            return None;
        }
        Some(Self(
            (HANDLE_MAGIC << MAGIC_SHIFT) | (kind << KIND_SHIFT) | index,
        ))
    }

    /// The bits, which is what the protocol carries.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Take the bits back off the wire. This validates nothing on its own;
    /// [`Handle::kind`] is the check.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The kind this handle claims to be, or `None` when it is not one of ours
    /// at all.
    #[must_use]
    pub const fn kind(self) -> Option<u32> {
        if (self.0 >> MAGIC_SHIFT) != HANDLE_MAGIC {
            return None;
        }
        let kind = (self.0 >> KIND_SHIFT) & KIND_MASK;
        if kind == 0 {
            return None;
        }
        Some(kind)
    }
}

/// The broker's list of one kind of object, keyed by the handle the host
/// minted.
///
/// The host mints, the broker validates: after a restart the host replays the
/// same handles, so the broker cannot keep its own counter and still answer the
/// messages that were in flight.
pub struct Registry<T> {
    kind: u32,
    live: BTreeMap<u32, T>,
}

impl<T> Registry<T> {
    /// An empty registry for one kind.
    #[must_use]
    pub const fn new(kind: u32) -> Self {
        Self {
            kind,
            live: BTreeMap::new(),
        }
    }

    /// How many live objects it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// Whether a handle passes magic and kind. A handle that does not is never
    /// looked up at all.
    fn valid(&self, handle: Handle) -> bool {
        handle.kind() == Some(self.kind)
    }

    /// Take ownership of `value` under the handle that names it. Answers
    /// `false` for a handle that is not one of this registry's kind, in which
    /// case nothing is stored.
    pub fn insert(&mut self, handle: Handle, value: T) -> bool {
        if !self.valid(handle) {
            return false;
        }
        self.live.insert(handle.bits(), value);
        true
    }

    /// The object a handle names, or `None` for a forged, stale or wrong-kind
    /// handle.
    pub fn get(&self, handle: Handle) -> Option<&T> {
        if !self.valid(handle) {
            return None;
        }
        self.live.get(&handle.bits())
    }

    /// The object a handle names, for writing.
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        if !self.valid(handle) {
            return None;
        }
        self.live.get_mut(&handle.bits())
    }

    /// Destroy the object a handle names.
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        if !self.valid(handle) {
            return None;
        }
        self.live.remove(&handle.bits())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_of_the_wrong_kind_or_no_magic_names_nothing() {
        let mut registry: Registry<u8> = Registry::new(KIND_INSTANCE);
        let good = Handle::encode(KIND_INSTANCE, 7).unwrap();
        assert!(registry.insert(good, 42));
        assert_eq!(registry.get(good), Some(&42));

        // A plain counter, which is what a host that did not do this would
        // send, fails the magic.
        assert_eq!(Handle::from_bits(7).kind(), None);
        assert_eq!(registry.get(Handle::from_bits(7)), None);

        // The right magic and the wrong kind is the one a second kind of
        // object would collide on.
        let other = Handle::encode(2, 7).unwrap();
        assert_eq!(other.kind(), Some(2));
        assert_eq!(registry.get(other), None);
        assert!(!registry.insert(other, 9), "and it stores nothing either");
        assert_eq!(registry.len(), 1);

        // A destroyed instance's handle names nothing from then on.
        assert_eq!(registry.remove(good), Some(42));
        assert_eq!(registry.get(good), None);
        assert!(registry.is_empty());
    }

    #[test]
    fn an_index_past_the_bits_mints_no_handle() {
        assert!(Handle::encode(KIND_INSTANCE, INDEX_MASK).is_some());
        assert!(Handle::encode(KIND_INSTANCE, INDEX_MASK + 1).is_none());
        assert!(Handle::encode(0, 1).is_none(), "nought is not a kind");
    }
}
