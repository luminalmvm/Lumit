//! The block plane: one shared-memory ring per module.
//!
//! # In plain terms
//!
//! Two processes agree on one piece of memory that is *the same memory* in both
//! of them — write it here, read it there, no copy in between — and the pipe
//! carries only the slot number.
//!
//! The block is divided into equal **slots**, used in turn. A slot holds one
//! block of sound: 512 frames of interleaved stereo float, four kilobytes, plus
//! a header. That is small, which is why the whole ring is a hundred and
//! thirty kilobytes and its size is a constant rather than a budget — the video
//! host's ring has to be sized from the frame size because a 4K frame is a
//! hundred megabytes; a block of sound is a block of sound.
//!
//! Every slot begins with a header, and the header is the whole contract for the
//! bytes after it: how many samples there are and a hash of them. The reader
//! checks the hash. Shared memory is the one place where a wrong answer arrives
//! silently — no error, no status, just the previous block's samples — and the
//! hash is what turns that into a noticed fault.
//!
//! **Interleaved, as Lumit carries sound.** The de-interleaving into the planes
//! both plugin standards want happens inside the broker, where
//! [`Block`](crate::process::Block) already knows how.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use memmap2::MmapMut;
use thiserror::Error;

use crate::ipc::proto::RingSpec;
use crate::process::INTERLEAVED_LEN;

/// The bytes at the head of every slot.
pub const HEADER_BYTES: usize = 32;

/// The four bytes that say a slot has been written by this protocol at all.
const HEADER_MAGIC: u32 = 0x4c41_5544;

/// The header layout's own version, so a slot written by another build of the
/// protocol reads as empty rather than as plausible.
const HEADER_VERSION: u32 = 1;

/// How many bytes one slot is: a whole block, header included.
pub const SLOT_BYTES: u64 =
    HEADER_BYTES as u64 + (INTERLEAVED_LEN * std::mem::size_of::<f32>()) as u64;

/// How many slots the ring holds.
///
/// A batch pre-renders eight blocks of lookahead (docs/impl/audio-plugins.md
/// §3) and each block wants an input slot and an output slot, so sixteen would
/// do; thirty-two is two batches in flight and still a hundred and thirty
/// kilobytes.
pub const RING_SLOTS: u32 = 32;

/// What can go wrong with the ring.
#[derive(Debug, Error)]
pub enum RingError {
    /// The backing file.
    #[error("the block ring could not be opened: {0}")]
    Io(#[from] std::io::Error),
    /// A slot number that is not in the ring.
    #[error("slot {0} is not in the block ring")]
    NoSuchSlot(u32),
    /// More samples than a slot holds.
    #[error("a {0}-sample block does not fit a ring slot")]
    TooBig(usize),
    /// A slot that was never written, or was written by something else.
    #[error("the block ring slot holds no block")]
    Empty,
    /// The hash in the header does not match the bytes after it.
    #[error("a block crossed the ring and arrived changed")]
    Corrupt,
}

/// One slot's header, as values rather than bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockHeader {
    /// How many interleaved samples follow the header. A short block is the
    /// last block of a layer, and it is short rather than padded so the reader
    /// knows where the sound ended.
    pub samples: u32,
    /// FNV-1a over exactly those bytes.
    pub hash: u64,
}

impl BlockHeader {
    /// The header as the bytes that go at the head of a slot.
    fn to_bytes(self) -> [u8; HEADER_BYTES] {
        let mut out = [0_u8; HEADER_BYTES];
        let mut put = |offset: usize, bytes: &[u8]| {
            if let Some(slot) = out.get_mut(offset..offset + bytes.len()) {
                slot.copy_from_slice(bytes);
            }
        };
        put(0, &HEADER_MAGIC.to_le_bytes());
        put(4, &HEADER_VERSION.to_le_bytes());
        put(8, &self.samples.to_le_bytes());
        put(16, &self.hash.to_le_bytes());
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
        if u32_at(0) != Some(HEADER_MAGIC) || u32_at(4) != Some(HEADER_VERSION) {
            return Err(RingError::Empty);
        }
        let (Some(samples), Some(hash)) = (u32_at(8), u64_at(16)) else {
            return Err(RingError::Empty);
        };
        Ok(Self { samples, hash })
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

/// The ring, mapped into this process.
pub struct Ring {
    spec: RingSpec,
    map: MmapMut,
    /// Set on the side that made the file, so that side deletes it.
    owned: Option<PathBuf>,
}

impl Drop for Ring {
    fn drop(&mut self) {
        let Some(path) = self.owned.take() else {
            return;
        };
        // Windows will not delete a file that is still mapped, so the mapping
        // goes first, swapped for a one-byte anonymous one.
        if let Ok(empty) = MmapMut::map_anon(1) {
            drop(std::mem::replace(&mut self.map, empty));
        }
        let _ = std::fs::remove_file(path);
    }
}

impl Ring {
    /// Make a ring and map it. Called once, when a broker is spawned.
    ///
    /// # Errors
    ///
    /// [`RingError::Io`].
    pub fn create(path: &Path) -> Result<Self, RingError> {
        let spec = RingSpec {
            path: path.to_string_lossy().into_owned(),
            slots: RING_SLOTS,
            slot_bytes: SLOT_BYTES,
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(SLOT_BYTES.saturating_mul(u64::from(RING_SLOTS)))?;
        let map = map_file(&file)?;
        Ok(Self {
            spec,
            map,
            owned: Some(path.to_path_buf()),
        })
    }

    /// Map a ring somebody else made. Called once, in the broker.
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
        })
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

    /// Where one slot starts and ends.
    fn range(&self, slot: u32) -> Result<(usize, usize), RingError> {
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

    /// Put one block of interleaved stereo in a slot.
    ///
    /// # Errors
    ///
    /// [`RingError::TooBig`] for more samples than a block, and
    /// [`RingError::NoSuchSlot`] for a slot that is not there.
    pub fn write_block(&mut self, slot: u32, samples: &[f32]) -> Result<BlockHeader, RingError> {
        if samples.len() > INTERLEAVED_LEN {
            return Err(RingError::TooBig(samples.len()));
        }
        let (start, end) = self.range(slot)?;
        let body_start = start.saturating_add(HEADER_BYTES);
        let payload = samples.len().saturating_mul(std::mem::size_of::<f32>());
        let body_end = body_start.saturating_add(payload).min(end);
        {
            let body = self
                .map
                .get_mut(body_start..body_end)
                .ok_or(RingError::NoSuchSlot(slot))?;
            for (index, value) in samples.iter().enumerate() {
                let offset = index.saturating_mul(std::mem::size_of::<f32>());
                if let Some(cell) = body.get_mut(offset..offset + std::mem::size_of::<f32>()) {
                    cell.copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        let hash = self
            .map
            .get(body_start..body_end)
            .map(hash_bytes)
            .unwrap_or_default();
        let header = BlockHeader {
            samples: u32::try_from(samples.len()).unwrap_or(0),
            hash,
        };
        let bytes = header.to_bytes();
        if let Some(head) = self.map.get_mut(start..start.saturating_add(HEADER_BYTES)) {
            head.copy_from_slice(&bytes);
        }
        Ok(header)
    }

    /// Read a slot back into `samples`, and say how many were there.
    ///
    /// `samples` is filled from the start and the rest of it is left silent, so
    /// a short block leaves silence where the sound ran out rather than the
    /// previous block's tail.
    ///
    /// # Errors
    ///
    /// [`RingError::Empty`] for a slot nobody wrote, [`RingError::Corrupt`] if
    /// the hash does not match.
    pub fn read_block(&self, slot: u32, samples: &mut [f32]) -> Result<usize, RingError> {
        let (start, end) = self.range(slot)?;
        let head = self
            .map
            .get(start..start.saturating_add(HEADER_BYTES))
            .ok_or(RingError::NoSuchSlot(slot))?;
        let header = BlockHeader::from_bytes(head)?;

        let count = (header.samples as usize).min(INTERLEAVED_LEN);
        let body_start = start.saturating_add(HEADER_BYTES);
        let body_end = body_start
            .saturating_add(count.saturating_mul(std::mem::size_of::<f32>()))
            .min(end);
        let body = self
            .map
            .get(body_start..body_end)
            .ok_or(RingError::NoSuchSlot(slot))?;
        if hash_bytes(body) != header.hash {
            return Err(RingError::Corrupt);
        }

        samples.fill(0.0);
        for (index, chunk) in body.chunks_exact(std::mem::size_of::<f32>()).enumerate() {
            let Some(cell) = samples.get_mut(index) else {
                break;
            };
            let bytes: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            *cell = f32::from_le_bytes(bytes);
        }
        Ok(count)
    }
}

/// Map a file into this process, shared with everyone else who maps it.
fn map_file(file: &File) -> Result<MmapMut, RingError> {
    // SAFETY: the file is one this process just made or was told the name of by
    // the process that made it; nothing else writes it except the broker at the
    // other end of the pipe, which is exactly the sharing that is wanted. The
    // mapping's length is the file's, so every read through it is in bounds.
    let map = unsafe { MmapMut::map_mut(file) }?;
    Ok(map)
}
