//! Bounded ingress: one budget every reader of untrusted input shares.
//!
//! # In plain terms
//!
//! A project file, an imported EXR, a colour config, a roto sidecar — every one
//! of them is a stranger's bytes. None of them is *trusted*, and none of them
//! is *hostile* either; they are simply data, and the only honest stance is
//! that the numbers inside decide how much work Lumit does. A header that says
//! "this picture is 4 billion pixels wide" will be believed by any reader that
//! does not check, and the check has to happen before the allocation, not after.
//!
//! Fixing that one parser at a time does not scale and does not stay fixed: the
//! next parser arrives without the lesson. So instead of a limit per reader,
//! this crate holds **one budget type** that readers carry through their work
//! and spend as they go. When the budget runs out the read fails with a typed
//! error naming which ceiling it hit, the caller shows a calm sentence, and the
//! application is still running — which is the whole point (docs/14 §4, §5).
//!
//! A budget spends four separate things, because untrusted input has four
//! separate ways to be expensive:
//!
//! | Meter | What it stops |
//! |---|---|
//! | **bytes** | allocations — decoded pixels, decompressed payloads, strings |
//! | **items** | counts — array entries, records, nodes, channels |
//! | **depth** | recursion — nested YAML/JSON that would overflow the stack |
//! | **work** | time — loop iterations that are cheap each and ruinous in bulk |
//!
//! Bytes and items are the two that catch most things. Depth is what keeps a
//! recursive-descent parser from ending the process: a stack overflow is not a
//! catchable error in Rust, so the only defence is never to recurse that far.
//! Work is for the readers whose cost is not an allocation at all — a run-length
//! payload that decodes to nothing but takes a year doing it.
//!
//! # Using it
//!
//! ```
//! use lumit_ingress::{Budget, Limits};
//!
//! # fn demo() -> Result<(), lumit_ingress::IngressError> {
//! let mut budget = Budget::new(Limits::PROJECT_FILE);
//!
//! // Before allocating, say how big it is going to be.
//! let pixels = lumit_ingress::checked_area(1920, 1080)?;
//! budget.take_bytes(pixels.saturating_mul(4))?;
//! let frame = vec![0u8; 1920 * 1080 * 4];
//!
//! // Before recursing, go through `nested` so the depth is counted.
//! budget.nested(|budget| budget.take_items(1))?;
//! # let _ = frame;
//! # Ok(())
//! # }
//! # demo().unwrap();
//! ```
//!
//! # Thread role
//!
//! Plain data, `Send` and `Sync`, holding no locks and doing no IO except the
//! two capped file readers below. A [`Budget`] is threaded by `&mut` through
//! one reader's work; sharing one across threads is deliberately not offered,
//! because two readers racing on one meter would make a refusal depend on
//! scheduling, and refusals that depend on scheduling are not reproducible
//! (docs/14 §3).

#![forbid(unsafe_code)]

use std::io::Read;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// Why a bounded read stopped. Never a panic (docs/14 §4).
///
/// Every variant names the ceiling it hit and the number that hit it, because
/// "the file is too big" is not an actionable sentence and "this project's
/// media index claims 9,000,000 entries, and the limit is 1,000,000" is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IngressError {
    /// The aggregate allocation ceiling.
    #[error("this file would need {needed} bytes of memory, and the limit is {limit}")]
    Bytes { needed: u64, limit: u64 },

    /// The aggregate count ceiling — entries, records, nodes, channels.
    #[error("this file holds more items than Lumit reads ({needed}, limit {limit})")]
    Items { needed: u64, limit: u64 },

    /// The nesting ceiling. Hit before the stack is, which is the point.
    #[error("this file nests deeper than Lumit reads (limit {limit})")]
    Depth { limit: u32 },

    /// The work ceiling — iterations, decode steps, expansions.
    #[error("reading this file would take more work than Lumit allows (limit {limit})")]
    Work { limit: u64 },

    /// Arithmetic on sizes taken from the file did not fit. A header claiming
    /// 65535 × 65535 × 32 channels overflows before it allocates, and this is
    /// what that overflow becomes instead of a wrap or a panic.
    #[error("the sizes in this file do not make sense together")]
    Overflow,

    /// A file longer than its reader's ceiling, refused without reading it all.
    #[error("{path} is larger than Lumit reads ({size} bytes, limit {limit})")]
    FileTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },

    /// The file could not be read at all. Kept as a string because the caller
    /// wraps this in its own crate's error and prints it, and `std::io::Error`
    /// is neither `Clone` nor `PartialEq`.
    #[error("{path} could not be read: {reason}")]
    Io { path: PathBuf, reason: String },
}

impl IngressError {
    /// The stable id this refusal crosses the bridge under
    /// (docs/17 "Display text crosses the bridge in English").
    ///
    /// The wrapping crate's own error carries this through, so a frontend
    /// sentence can be written once for "too big" rather than once per parser.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            IngressError::Bytes { .. } => "ingress_bytes",
            IngressError::Items { .. } => "ingress_items",
            IngressError::Depth { .. } => "ingress_depth",
            IngressError::Work { .. } => "ingress_work",
            IngressError::Overflow => "ingress_overflow",
            IngressError::FileTooLarge { .. } => "ingress_file_too_large",
            IngressError::Io { .. } => "ingress_io",
        }
    }
}

/// The shorthand this crate returns.
pub type Result<T> = std::result::Result<T, IngressError>;

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// The four ceilings a [`Budget`] enforces.
///
/// The named constants below are the ones Lumit's own readers use. They are
/// deliberately generous — an honest file should never meet one — and the
/// reasoning for each sits beside it, because a limit whose number nobody can
/// justify is a limit somebody will raise the first time it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Aggregate bytes the read may allocate.
    pub bytes: u64,
    /// Aggregate items — entries, records, nodes — the read may materialise.
    pub items: u64,
    /// How deep the read may nest.
    pub depth: u32,
    /// Aggregate abstract work units the read may spend.
    pub work: u64,
}

impl Limits {
    /// A project file's structured contents: the `.lum` container's JSON, its
    /// journal, its presets and collections.
    ///
    /// 512 MiB is past any real project — the pixels live in the media files,
    /// not in here — and a million items is more layers, keyframes and
    /// properties than a timeline can hold and still be edited.
    pub const PROJECT_FILE: Limits = Limits {
        bytes: 512 << 20,
        items: 1_000_000,
        depth: 64,
        work: 64_000_000,
    };

    /// A colour config and the tables it points at.
    ///
    /// The largest configs in circulation (ACES Studio) are a few megabytes of
    /// YAML; 64 MiB leaves room for one that ships its tables inline. The depth
    /// is low because OCIO's grammar is flat — a transform group inside a
    /// colour space inside the config is three, and sixteen is already absurd.
    pub const COLOUR_CONFIG: Limits = Limits {
        bytes: 64 << 20,
        items: 2_000_000,
        depth: 32,
        work: 16_000_000,
    };

    /// One imported picture: an EXR, a still, a sidecar's decoded matte.
    ///
    /// 4 GiB is one 16K RGBA float frame with room to spare, which is the
    /// largest single raster this application has any business decoding in one
    /// piece.
    pub const IMAGE: Limits = Limits {
        bytes: 4 << 30,
        items: 4096,
        depth: 8,
        work: 1 << 32,
    };

    /// A sidecar or cache file written beside a project — roto mattes, media
    /// indexes, thumbnails.
    pub const SIDECAR: Limits = Limits {
        bytes: 1 << 30,
        items: 1_000_000,
        depth: 16,
        work: 1 << 30,
    };

    /// A budget that refuses nothing, for tests and for readers whose input is
    /// Lumit's own freshly written bytes rather than a stranger's.
    pub const UNLIMITED: Limits = Limits {
        bytes: u64::MAX,
        items: u64::MAX,
        depth: u32::MAX,
        work: u64::MAX,
    };
}

// ---------------------------------------------------------------------------
// The budget
// ---------------------------------------------------------------------------

/// A running tally of what one read has spent, and what it has left.
///
/// Threaded by `&mut` through a reader's call tree. Spending is monotonic:
/// nothing is ever given back except the depth a [`Budget::nested`] scope
/// borrowed, which is returned when the scope ends.
#[derive(Debug, Clone)]
pub struct Budget {
    limits: Limits,
    bytes: u64,
    items: u64,
    work: u64,
    depth: u32,
}

impl Budget {
    /// A fresh budget with nothing spent.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Budget {
            limits,
            bytes: 0,
            items: 0,
            work: 0,
            depth: 0,
        }
    }

    /// A budget that refuses nothing. For tests, and for reading back bytes
    /// this application itself just wrote.
    #[must_use]
    pub fn unlimited() -> Self {
        Budget::new(Limits::UNLIMITED)
    }

    /// The ceilings this budget was built with.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Bytes spent so far.
    #[must_use]
    pub fn spent_bytes(&self) -> u64 {
        self.bytes
    }

    /// Items spent so far.
    #[must_use]
    pub fn spent_items(&self) -> u64 {
        self.items
    }

    /// Work spent so far.
    #[must_use]
    pub fn spent_work(&self) -> u64 {
        self.work
    }

    /// How many [`Budget::nested`] scopes are open right now.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Whether `extra` further levels would fit, without entering them.
    ///
    /// For the reader that is about to graft a subtree it already holds — a
    /// YAML alias expanding somewhere deeper than it was declared — where the
    /// depth arrives all at once rather than one recursive call at a time.
    pub fn check_depth(&self, extra: u32) -> Result<()> {
        let total = self
            .depth
            .checked_add(extra)
            .ok_or(IngressError::Overflow)?;
        if total > self.limits.depth {
            return Err(IngressError::Depth {
                limit: self.limits.depth,
            });
        }
        Ok(())
    }

    /// Bytes still available, for a reader that wants to size a buffer to
    /// whatever is left rather than ask for a number and be refused.
    #[must_use]
    pub fn bytes_remaining(&self) -> u64 {
        self.limits.bytes.saturating_sub(self.bytes)
    }

    /// Charge `n` bytes of allocation. Call this **before** the allocation,
    /// with the number the file asked for — charging afterwards is charging
    /// after the machine has already swapped itself to death.
    pub fn take_bytes(&mut self, n: u64) -> Result<()> {
        let next = self.bytes.checked_add(n).ok_or(IngressError::Overflow)?;
        if next > self.limits.bytes {
            return Err(IngressError::Bytes {
                needed: next,
                limit: self.limits.bytes,
            });
        }
        self.bytes = next;
        Ok(())
    }

    /// Charge `n` items — entries, records, nodes, channels.
    pub fn take_items(&mut self, n: u64) -> Result<()> {
        let next = self.items.checked_add(n).ok_or(IngressError::Overflow)?;
        if next > self.limits.items {
            return Err(IngressError::Items {
                needed: next,
                limit: self.limits.items,
            });
        }
        self.items = next;
        Ok(())
    }

    /// Charge `n` units of abstract work.
    pub fn take_work(&mut self, n: u64) -> Result<()> {
        let next = self.work.checked_add(n).ok_or(IngressError::Overflow)?;
        if next > self.limits.work {
            return Err(IngressError::Work {
                limit: self.limits.work,
            });
        }
        self.work = next;
        Ok(())
    }

    /// Charge one item and one level of nesting for the duration of `f`.
    ///
    /// This is how a recursive reader stays off the stack's limit: every
    /// recursive call goes through here, so the depth is counted whether the
    /// author remembered to count it or not, and it is given back when the
    /// scope ends whether `f` succeeded or not.
    ///
    /// Generic over the closure's error so a parser keeps returning its own
    /// crate's error type rather than having to translate at every call.
    ///
    /// ```
    /// # use lumit_ingress::{Budget, Limits, IngressError};
    /// let mut budget = Budget::new(Limits { depth: 2, ..Limits::UNLIMITED });
    /// let deep = budget.nested(|b| b.nested(|b| b.nested(|_| Ok::<_, IngressError>(()))));
    /// assert!(matches!(deep, Err(IngressError::Depth { .. })));
    /// // The scopes that unwound gave their depth back.
    /// assert!(budget.nested(|_| Ok::<_, IngressError>(())).is_ok());
    /// ```
    pub fn nested<T, E>(
        &mut self,
        f: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<IngressError>,
    {
        if self.depth >= self.limits.depth {
            return Err(IngressError::Depth {
                limit: self.limits.depth,
            }
            .into());
        }
        self.take_items(1)?;
        self.depth = self.depth.saturating_add(1);
        let out = f(self);
        self.depth = self.depth.saturating_sub(1);
        out
    }

    /// Charge the bytes a `Vec<T>` of `count` elements will occupy, then build
    /// it with that capacity reserved.
    ///
    /// The pairing is the point: a reader that says `Vec::with_capacity(n)` with
    /// an `n` out of a file has already lost, and a reader that calls this has
    /// the check and the allocation in one place where neither can be forgotten.
    pub fn vec_with_capacity<T>(&mut self, count: usize) -> Result<Vec<T>> {
        let bytes = u64::try_from(count)
            .map_err(|_| IngressError::Overflow)?
            .checked_mul(
                u64::try_from(std::mem::size_of::<T>()).map_err(|_| IngressError::Overflow)?,
            )
            .ok_or(IngressError::Overflow)?;
        self.take_bytes(bytes)?;
        Ok(Vec::with_capacity(count))
    }
}

// ---------------------------------------------------------------------------
// Checked arithmetic on sizes that came out of a file
// ---------------------------------------------------------------------------

/// `width × height`, refusing rather than wrapping.
///
/// The one multiplication every image reader does, and the one every image
/// reader gets wrong: `65_536 * 65_536` fits in a `u64` but not in a 32-bit
/// `usize`, and `u32::MAX * u32::MAX` fits in neither.
pub fn checked_area(width: u64, height: u64) -> Result<u64> {
    width.checked_mul(height).ok_or(IngressError::Overflow)
}

/// `width × height × channels × bytes_per_sample`, refusing rather than wrapping.
///
/// What a raster actually costs, worked out in `u64` so the answer is the true
/// one even on a machine where it could not be allocated.
pub fn checked_raster_bytes(
    width: u64,
    height: u64,
    channels: u64,
    bytes_per_sample: u64,
) -> Result<u64> {
    checked_area(width, height)?
        .checked_mul(channels)
        .and_then(|n| n.checked_mul(bytes_per_sample))
        .ok_or(IngressError::Overflow)
}

/// A `u64` count out of a file, as a `usize` this machine can index with.
///
/// Refuses instead of truncating: on a 32-bit target a count of 5 billion is
/// not 705 million, it is a file this machine cannot read.
pub fn checked_usize(n: u64) -> Result<usize> {
    usize::try_from(n).map_err(|_| IngressError::Overflow)
}

// ---------------------------------------------------------------------------
// Capped file reading
// ---------------------------------------------------------------------------

/// Read a whole file, refusing one longer than `limit` without reading it all.
///
/// `std::fs::read` sizes its buffer from the file's own metadata and then reads
/// until end of file, which is two separate ways to be told how much memory to
/// use by whoever wrote the path. This checks the metadata first *and* stops
/// reading at the ceiling, so a file that lies about its length — a pipe, a
/// `/proc` entry, a file being appended to while it is read — is refused rather
/// than believed.
pub fn read_capped(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let io = |e: std::io::Error| IngressError::Io {
        path: path.to_path_buf(),
        reason: e.to_string(),
    };
    let file = std::fs::File::open(path).map_err(io)?;
    // The declared length is a hint for sizing, never a promise about content.
    let declared = file.metadata().map_err(io)?.len();
    if declared > limit {
        return Err(IngressError::FileTooLarge {
            path: path.to_path_buf(),
            size: declared,
            limit,
        });
    }
    // One byte past the ceiling, so a file that grew between the metadata read
    // and here is caught by having produced it rather than by being trusted.
    let ceiling = limit.saturating_add(1);
    let mut buf = Vec::with_capacity(checked_usize(declared.min(limit))?);
    file.take(ceiling).read_to_end(&mut buf).map_err(io)?;
    if u64::try_from(buf.len()).map_err(|_| IngressError::Overflow)? > limit {
        return Err(IngressError::FileTooLarge {
            path: path.to_path_buf(),
            size: ceiling,
            limit,
        });
    }
    Ok(buf)
}

/// [`read_capped`], as text.
///
/// Invalid UTF-8 is an IO-shaped refusal rather than a separate variant: for
/// every caller here the answer is the same sentence, "that file could not be
/// read".
pub fn read_to_string_capped(path: &Path, limit: u64) -> Result<String> {
    let bytes = read_capped(path, limit)?;
    String::from_utf8(bytes).map_err(|_| IngressError::Io {
        path: path.to_path_buf(),
        reason: "the file is not valid UTF-8 text".to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn bytes_refuse_past_the_ceiling_and_name_both_numbers() {
        let mut b = Budget::new(Limits {
            bytes: 100,
            ..Limits::UNLIMITED
        });
        b.take_bytes(60).unwrap();
        let err = b.take_bytes(60).unwrap_err();
        assert_eq!(
            err,
            IngressError::Bytes {
                needed: 120,
                limit: 100
            }
        );
        // A refused charge is not spent: the reader may still do something
        // smaller with what is left.
        assert_eq!(b.spent_bytes(), 60);
        assert_eq!(b.bytes_remaining(), 40);
    }

    #[test]
    fn a_charge_that_overflows_the_tally_is_a_refusal_not_a_wrap() {
        let mut b = Budget::unlimited();
        b.take_bytes(u64::MAX - 1).unwrap();
        assert_eq!(b.take_bytes(8).unwrap_err(), IngressError::Overflow);
    }

    #[test]
    fn depth_is_given_back_when_a_scope_ends() {
        let mut b = Budget::new(Limits {
            depth: 3,
            ..Limits::UNLIMITED
        });
        for _ in 0..10 {
            b.nested(|b| b.nested(|b| b.nested(|_| Ok::<_, IngressError>(()))))
                .unwrap();
        }
        // Ten round trips through three levels never leaked a level.
        let too_deep =
            b.nested(|b| b.nested(|b| b.nested(|b| b.nested(|_| Ok::<_, IngressError>(())))));
        assert!(matches!(too_deep, Err(IngressError::Depth { limit: 3 })));
    }

    #[test]
    fn depth_is_given_back_even_when_the_scope_failed() {
        let mut b = Budget::new(Limits {
            depth: 2,
            ..Limits::UNLIMITED
        });
        let _ = b.nested(|b| b.nested(|_| Err::<(), _>(IngressError::Overflow)));
        assert!(b
            .nested(|b| b.nested(|_| Ok::<_, IngressError>(())))
            .is_ok());
    }

    #[test]
    fn nesting_costs_an_item_so_a_wide_shallow_bomb_is_caught_too() {
        // Depth alone does not stop `[[],[],[],[],…]` a million entries wide.
        let mut b = Budget::new(Limits {
            items: 4,
            ..Limits::UNLIMITED
        });
        for _ in 0..4 {
            b.nested(|_| Ok::<_, IngressError>(())).unwrap();
        }
        assert!(matches!(
            b.nested(|_| Ok::<_, IngressError>(())),
            Err(IngressError::Items { .. })
        ));
    }

    #[test]
    fn raster_arithmetic_refuses_instead_of_wrapping() {
        assert_eq!(checked_area(1920, 1080).unwrap(), 2_073_600);
        assert_eq!(checked_raster_bytes(1920, 1080, 4, 4).unwrap(), 33_177_600);
        // The shape a crafted header takes: each number plausible, the product not.
        assert_eq!(
            checked_raster_bytes(u64::MAX, 2, 4, 4).unwrap_err(),
            IngressError::Overflow
        );
        assert_eq!(
            checked_area(u64::MAX, u64::MAX).unwrap_err(),
            IngressError::Overflow
        );
    }

    #[test]
    fn vec_with_capacity_charges_the_element_size_not_the_count() {
        let mut b = Budget::new(Limits {
            bytes: 64,
            ..Limits::UNLIMITED
        });
        // 16 f32s is 64 bytes, exactly the ceiling.
        let v = b.vec_with_capacity::<f32>(16).unwrap();
        assert!(v.capacity() >= 16);
        assert_eq!(b.spent_bytes(), 64);
        assert!(b.vec_with_capacity::<f32>(1).is_err());
    }

    #[test]
    fn a_file_longer_than_the_ceiling_is_refused() {
        let dir = std::env::temp_dir().join("lumit-ingress-cap-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        std::fs::write(&path, vec![7u8; 4096]).unwrap();

        assert!(matches!(
            read_capped(&path, 100),
            Err(IngressError::FileTooLarge {
                size: 4096,
                limit: 100,
                ..
            })
        ));
        assert_eq!(read_capped(&path, 4096).unwrap().len(), 4096);
        // Exactly at the ceiling is allowed; one byte under is not.
        assert!(matches!(
            read_capped(&path, 4095),
            Err(IngressError::FileTooLarge { .. })
        ));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn non_utf8_text_is_a_refusal_rather_than_a_lossy_read() {
        let dir = std::env::temp_dir().join("lumit-ingress-utf8-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.txt");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(
            read_to_string_capped(&path, 1024),
            Err(IngressError::Io { .. })
        ));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn every_refusal_has_its_own_id() {
        let all = [
            IngressError::Bytes {
                needed: 2,
                limit: 1,
            },
            IngressError::Items {
                needed: 2,
                limit: 1,
            },
            IngressError::Depth { limit: 1 },
            IngressError::Work { limit: 1 },
            IngressError::Overflow,
            IngressError::FileTooLarge {
                path: "a".into(),
                size: 2,
                limit: 1,
            },
            IngressError::Io {
                path: "a".into(),
                reason: "no".into(),
            },
        ];
        let ids: std::collections::BTreeSet<_> = all.iter().map(IngressError::key).collect();
        assert_eq!(ids.len(), all.len());
    }
}
