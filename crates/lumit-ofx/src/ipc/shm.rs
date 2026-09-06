//! The frame plane: one shared-memory ring per bundle.
//!
//! # In plain terms
//!
//! A 4K frame is a hundred megabytes. Pushing that down a pipe, twice per
//! render, would cost more than the effect. So the two processes agree on one
//! block of memory that is *the same memory* in both of them — write it here,
//! read it there, no copy in between — and the pipe carries only the slot
//! number.
//!
//! The block is divided into equal **slots**, and the slots are used in turn.
//! The note asks for triple buffering, and that is the floor: at least three
//! slots, so that the slot being written is never the slot being read, with
//! one spare between them. A small frame gets many more, because the ring is
//! sized by a byte budget rather than by a slot count, and it is sized
//! **once per bundle** — at the moment the broker is spawned, from the comp's
//! frame size — never per frame.
//!
//! Every slot begins with a header, and the header is the whole contract for
//! the bytes after it: what rectangle they are, how far apart the rows are,
//! whether the alpha is premultiplied, how many bytes there are, and a hash of
//! them. The reader checks the hash. Shared memory is the one place where a
//! wrong answer arrives silently — no error, no status, just the previous
//! frame's pixels — and the hash is what turns that into a noticed fault.
//!
//! **Row bytes here describe the ring, not the plugin.** The block is tightly
//! packed, top-down, four floats per pixel. The flip to OFX's bottom-up
//! convention happens at the plugin boundary, inside the broker, where
//! [`crate::image::Image`] already knows how (docs/impl/ofx-host.md §2).

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use memmap2::MmapMut;
use thiserror::Error;

use crate::image::{Frame16, RectI, CHANNELS};
use crate::ipc::proto::RingSpec;

/// The bytes at the head of every slot.
pub const HEADER_BYTES: usize = 64;

/// The four bytes that say a slot has been written by this protocol at all.
const HEADER_MAGIC: u32 = 0x4c4f_4658;

/// How much memory one bundle's ring may take. A budget, chosen rather than
/// measured, and the file really is this big on disk — so it is deliberately
/// not enormous.
///
/// At 1080p it buys fifteen slots, which covers a retimer's `t ± 5` with room
/// over. **Ceiling:** at 4K a frame is 132 MB, so this buys three — the floor —
/// and a `t ± 5` prefetch at that size will not fit and is refused (the plugin
/// gets the frame it was handed, which is a legal OFX answer). Lifting that is a
/// preference and a bigger number here, not a change of design.
pub const RING_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

/// The note's triple buffering, as the floor it is.
pub const RING_MIN_SLOTS: u32 = 3;

/// The ceiling, so that a tiny frame does not mint a hundred thousand slots
/// nobody will ever use.
pub const RING_MAX_SLOTS: u32 = 64;

/// What can go wrong with the ring.
#[derive(Debug, Error)]
pub enum ShmError {
    /// The backing file.
    #[error("the frame ring could not be opened: {0}")]
    Io(#[from] std::io::Error),
    /// A slot number that is not in the ring.
    #[error("slot {0} is not in the frame ring")]
    NoSuchSlot(u32),
    /// A frame bigger than a slot. The ring is sized once, so this is a comp
    /// whose frame size changed under a live broker.
    #[error("a {needed}-byte frame does not fit a {slot_bytes}-byte ring slot")]
    TooBig {
        /// What the frame needs, header included.
        needed: u64,
        /// What a slot holds.
        slot_bytes: u64,
    },
    /// A slot that was never written, or was written by something else.
    #[error("the frame ring slot holds no frame")]
    Empty,
    /// The hash in the header does not match the bytes after it.
    #[error("a frame crossed the ring and arrived changed")]
    Corrupt,
}

/// One slot's header, as values rather than bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    /// The rectangle the pixels cover.
    pub bounds: RectI,
    /// How far apart the rows are in the ring's own block: always positive and
    /// always tight, because this describes the ring.
    pub row_bytes: i32,
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
        put(4, &1_u32.to_le_bytes());
        put(8, &self.bounds.x1.to_le_bytes());
        put(12, &self.bounds.y1.to_le_bytes());
        put(16, &self.bounds.x2.to_le_bytes());
        put(20, &self.bounds.y2.to_le_bytes());
        put(24, &self.row_bytes.to_le_bytes());
        put(28, &u32::from(self.premultiplied).to_le_bytes());
        put(32, &self.payload_bytes.to_le_bytes());
        put(40, &self.hash.to_le_bytes());
        out
    }

    /// The header a slot begins with, or [`ShmError::Empty`] if it begins with
    /// anything else.
    fn from_bytes(bytes: &[u8]) -> Result<Self, ShmError> {
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

        if u32_at(0) != Some(HEADER_MAGIC) || u32_at(4) != Some(1) {
            return Err(ShmError::Empty);
        }
        let (Some(x1), Some(y1), Some(x2), Some(y2)) =
            (i32_at(8), i32_at(12), i32_at(16), i32_at(20))
        else {
            return Err(ShmError::Empty);
        };
        let (Some(row_bytes), Some(premultiplied), Some(payload_bytes), Some(hash)) =
            (i32_at(24), u32_at(28), u64_at(32), u64_at(40))
        else {
            return Err(ShmError::Empty);
        };
        Ok(Self {
            bounds: RectI { x1, y1, x2, y2 },
            row_bytes,
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

/// How many bytes one frame of this size needs in a slot.
#[must_use]
pub fn slot_bytes_for(width: usize, height: usize) -> u64 {
    let pixels = (width as u64).saturating_mul(height as u64);
    let payload = pixels
        .saturating_mul(CHANNELS as u64)
        .saturating_mul(size_of::<f32>() as u64);
    payload.saturating_add(HEADER_BYTES as u64)
}

/// The ring, mapped into this process.
pub struct Ring {
    spec: RingSpec,
    map: MmapMut,
    /// Set on the side that made the file, so that side deletes it.
    owned: Option<PathBuf>,
    /// The maker's own handle, kept open for the life of the ring. On Windows
    /// it carries delete-on-close, so the file goes when this process ends
    /// whether or not anything dropped the ring. The host keeps its brokers
    /// in a static, and a static is never dropped, which is how half a
    /// gigabyte a broker used to pile up in the temp directory.
    file: Option<File>,
}

impl Drop for Ring {
    fn drop(&mut self) {
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
    /// Make a ring for a comp of this frame size and map it. Called once, when
    /// a broker is spawned.
    ///
    /// # Errors
    ///
    /// [`ShmError::Io`].
    pub fn create(path: &Path, width: usize, height: usize) -> Result<Self, ShmError> {
        let slot_bytes = slot_bytes_for(width, height).max(HEADER_BYTES as u64);
        let slots = (RING_BUDGET_BYTES / slot_bytes.max(1))
            .clamp(u64::from(RING_MIN_SLOTS), u64::from(RING_MAX_SLOTS));
        let slots = u32::try_from(slots).unwrap_or(RING_MIN_SLOTS);
        let spec = RingSpec {
            path: path.to_string_lossy().into_owned(),
            slots,
            slot_bytes,
        };
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(true);
        // FILE_FLAG_DELETE_ON_CLOSE. The broker still opens the file by name
        // while this handle is open; std's default share mode allows that.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0400_0000);
        }
        let file = options.open(path)?;
        file.set_len(slot_bytes.saturating_mul(u64::from(slots)))?;
        let map = map_file(&file)?;
        Ok(Self {
            spec,
            map,
            owned: Some(path.to_path_buf()),
            file: Some(file),
        })
    }

    /// Map a ring somebody else made. Called once, in the broker.
    ///
    /// # Errors
    ///
    /// [`ShmError::Io`].
    pub fn open(spec: &RingSpec) -> Result<Self, ShmError> {
        let file = OpenOptions::new().read(true).write(true).open(&spec.path)?;
        let map = map_file(&file)?;
        Ok(Self {
            spec: spec.clone(),
            map,
            owned: None,
            file: None,
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
    fn range(&self, slot: u32) -> Result<(usize, usize), ShmError> {
        if slot >= self.spec.slots {
            return Err(ShmError::NoSuchSlot(slot));
        }
        let start = self
            .spec
            .slot_bytes
            .saturating_mul(u64::from(slot))
            .try_into()
            .map_err(|_| ShmError::NoSuchSlot(slot))?;
        let length: usize = self
            .spec
            .slot_bytes
            .try_into()
            .map_err(|_| ShmError::NoSuchSlot(slot))?;
        Ok((start, start.saturating_add(length)))
    }

    /// Put a frame in a slot, widened to the float the plugin boundary speaks
    /// (docs/12 §2.1), and answer with the header that was written.
    ///
    /// # Errors
    ///
    /// [`ShmError::TooBig`] if the frame is bigger than the ring was sized for,
    /// [`ShmError::NoSuchSlot`] for a slot that is not there.
    pub fn write_frame(
        &mut self,
        slot: u32,
        frame: &Frame16,
        bounds: RectI,
        premultiplied: bool,
    ) -> Result<FrameHeader, ShmError> {
        let (start, end) = self.range(slot)?;
        let pixels = frame.pixels();
        let payload_bytes = (pixels.len() as u64).saturating_mul(size_of::<f32>() as u64);
        let needed = payload_bytes.saturating_add(HEADER_BYTES as u64);
        if needed > self.spec.slot_bytes {
            return Err(ShmError::TooBig {
                needed,
                slot_bytes: self.spec.slot_bytes,
            });
        }

        let body_start = start.saturating_add(HEADER_BYTES);
        let body_end = body_start.saturating_add(payload_bytes as usize).min(end);
        {
            let body = self
                .map
                .get_mut(body_start..body_end)
                .ok_or(ShmError::NoSuchSlot(slot))?;
            for (index, value) in pixels.iter().enumerate() {
                let offset = index.saturating_mul(size_of::<f32>());
                if let Some(cell) = body.get_mut(offset..offset + size_of::<f32>()) {
                    cell.copy_from_slice(&f32::from(*value).to_le_bytes());
                }
            }
        }

        let hash = self
            .map
            .get(body_start..body_end)
            .map(hash_bytes)
            .unwrap_or_default();
        let row_bytes = i32::try_from(
            frame
                .width()
                .saturating_mul(CHANNELS)
                .saturating_mul(size_of::<f32>()),
        )
        .unwrap_or(0);
        let header = FrameHeader {
            bounds,
            row_bytes,
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

    /// Read a slot back: the header, and the frame narrowed to the working
    /// depth.
    ///
    /// # Errors
    ///
    /// [`ShmError::Empty`] for a slot nobody wrote, [`ShmError::Corrupt`] if
    /// the hash does not match.
    pub fn read_frame(&self, slot: u32) -> Result<(FrameHeader, Frame16), ShmError> {
        let (start, end) = self.range(slot)?;
        let head = self
            .map
            .get(start..start.saturating_add(HEADER_BYTES))
            .ok_or(ShmError::NoSuchSlot(slot))?;
        let header = FrameHeader::from_bytes(head)?;

        let body_start = start.saturating_add(HEADER_BYTES);
        let body_end = body_start
            .saturating_add(header.payload_bytes as usize)
            .min(end);
        let body = self
            .map
            .get(body_start..body_end)
            .ok_or(ShmError::NoSuchSlot(slot))?;
        if hash_bytes(body) != header.hash {
            return Err(ShmError::Corrupt);
        }

        let mut floats = Vec::with_capacity(body.len() / size_of::<f32>());
        for chunk in body.chunks_exact(size_of::<f32>()) {
            let bytes: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            floats.push(f32::from_le_bytes(bytes));
        }
        let width = header.bounds.width();
        let height = header.bounds.height();
        let frame = Frame16::from_f32(width, height, &floats).map_err(|_| ShmError::Corrupt)?;
        Ok((header, frame))
    }
}

/// Map a file into this process, shared with everyone else who maps it.
fn map_file(file: &File) -> Result<MmapMut, ShmError> {
    // SAFETY: the file is one this process just made or was told the name of by
    // the process that made it; nothing else writes it except the broker at the
    // other end of the pipe, which is exactly the sharing that is wanted. The
    // mapping's length is the file's, so every read through it is in bounds.
    let map = unsafe { MmapMut::map_mut(file) }?;
    Ok(map)
}
