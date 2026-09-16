//! The stored fixture hash: a moved pixel with no version bump is a refusal
//! (docs/impl/lfx.md §9).
//!
//! # In plain terms
//!
//! Nothing in the ABI can make a vendor bump the version when the maths
//! changes, and a vendor who does not leaves every cached frame on every
//! machine wrong - §4.1's version arithmetic re-keys frames on an **LFX**
//! release, which is exactly the pressure that fails when the release never
//! happens. So the validator writes down what the pixels were, and the next run
//! says whether they moved.
//!
//! The file is JSON, one entry per plugin id, and it is the vendor's to keep in
//! their own repository beside their own CI. It is a hash and not a picture:
//! what it has to answer is "did anything move", and a folder of frames would
//! be a folder somebody has to store.
//!
//! **Two hashes per plugin, one per mandatory depth.** Both depths are the
//! plugin's own code (§2.5) and a vendor who rewrites their fp16 maths and
//! leaves the version standing is the very release this file exists to catch,
//! so an fp32 digest on its own would be a record of half the obligation. An
//! entry written before this and carrying one digest does not parse, which is
//! [`BaselineError::Malformed`] and a refusal rather than a comparison against
//! a record that means something else.
//!
//! The digest is FNV-1a over the frame's own bytes. It is not a cryptographic
//! hash and is not asked to be one - the file sits beside the plugin's source
//! and an author who edits it has edited their own record. What it must be is
//! **stable across machines and across releases of Lumit**, which is why it is
//! written out here from the bytes rather than taken from a library whose
//! hasher is free to change.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The offset basis and prime of 64-bit FNV-1a, spelled from the specification
/// rather than derived.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// The 64-bit FNV prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One frame at each of the two mandatory depths, hashed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digests {
    /// The fp32 frame's digest, lower-case hexadecimal.
    pub fp32: String,
    /// The fp16 frame's digest, lower-case hexadecimal.
    pub fp16: String,
}

/// What was stored about one plugin the last time somebody looked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// The version the digests were taken at, as `major.minor.patch`.
    pub version: String,
    /// What the frames were, one digest per depth.
    pub digests: Digests,
}

/// The whole file: every plugin in the bundle that has ever been recorded.
///
/// A `BTreeMap` rather than a `HashMap` so the file a vendor commits is stable
/// between runs - a re-ordered baseline is a diff nobody can read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// Keyed by the plugin's own reverse-DNS id, which is what a saved project
    /// resolves through and the one name that may not change.
    pub plugins: BTreeMap<String, Stored>,
}

/// Why a baseline file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    /// The file could not be opened, read or written.
    #[error("the baseline file {path} could not be used: {why}")]
    File {
        /// Where it was looked for.
        path: String,
        /// The operating system's own sentence.
        why: std::io::Error,
    },
    /// It was read and it is not a baseline.
    #[error("the baseline file {path} is not a baseline: {why}")]
    Malformed {
        /// Where it was read from.
        path: String,
        /// What the parser said.
        why: serde_json::Error,
    },
}

impl Baseline {
    /// Read one, or an empty one where the file is not there yet.
    ///
    /// **A missing file is not an error and a damaged one is.** The first run
    /// of a new plugin has nothing stored and says so per plugin
    /// ([`crate::Finding::NoStoredBaseline`]); a file that exists and will not
    /// parse is the vendor's own record damaged, and reading it as "nothing
    /// stored" would silently re-baseline the very thing it was keeping
    /// (§11 item 16's argument, one document along).
    ///
    /// # Errors
    ///
    /// [`BaselineError`].
    pub fn read(path: &Path) -> Result<Self, BaselineError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(why) => {
                return Err(BaselineError::File {
                    path: path.display().to_string(),
                    why,
                })
            }
        };
        serde_json::from_str(&text).map_err(|why| BaselineError::Malformed {
            path: path.display().to_string(),
            why,
        })
    }

    /// Write one, pretty-printed and with a trailing newline, because it is a
    /// file a person commits.
    ///
    /// # Errors
    ///
    /// [`BaselineError`].
    pub fn write(&self, path: &Path) -> Result<(), BaselineError> {
        let mut text =
            serde_json::to_string_pretty(self).map_err(|why| BaselineError::Malformed {
                path: path.display().to_string(),
                why,
            })?;
        text.push('\n');
        std::fs::write(path, text).map_err(|why| BaselineError::File {
            path: path.display().to_string(),
            why,
        })
    }
}

/// The digest of one run's bytes.
///
/// The caller hands the samples in the order they came back, and hands each
/// depth separately: fp16 and fp32 are hashed as their own bytes and stored as
/// two entries ([`Digests`]), because the two depths are two pictures and a
/// baseline that folded them together could not say which of them moved.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{digest, Baseline, Digests, Stored};

    /// The file a vendor commits reads back as what was written, and an absent
    /// one reads as nothing stored rather than as an error.
    #[test]
    fn a_baseline_round_trips_and_an_absent_one_is_empty() {
        let Ok(root) = tempfile::tempdir() else {
            return;
        };
        let path = root.path().join("lfx-baseline.json");

        let absent = Baseline::read(&path);
        assert_eq!(
            absent.ok(),
            Some(Baseline::default()),
            "a baseline that has never been written should read as an empty one"
        );

        let mut written = Baseline::default();
        written.plugins.insert(
            "org.example.blur".to_owned(),
            Stored {
                version: "2.3.4".to_owned(),
                digests: Digests {
                    fp32: digest(b"a frame"),
                    fp16: digest(b"the same frame at half the width"),
                },
            },
        );
        assert!(written.write(&path).is_ok(), "the baseline should write");
        assert_eq!(
            Baseline::read(&path).ok(),
            Some(written),
            "what was written should read back"
        );
    }

    /// An entry that does not carry a digest per depth is a damaged record
    /// rather than half a comparison: a file written before both depths were
    /// stored says nothing about the fp16 path, and reading it as though it
    /// did would pass the very release `--baseline` is for.
    #[test]
    fn an_entry_with_one_digest_is_not_a_baseline() {
        let Ok(root) = tempfile::tempdir() else {
            return;
        };
        let path = root.path().join("lfx-baseline.json");
        let one_digest = r#"{"plugins":{"org.example.blur":{"version":"1.0.0","digest":"abc"}}}"#;
        assert!(std::fs::write(&path, one_digest).is_ok());
        assert!(
            Baseline::read(&path).is_err(),
            "a record of one depth must be refused rather than compared"
        );
    }

    /// A damaged baseline is a refusal rather than a first use: re-baselining
    /// the record silently is the one thing a stored hash must never do.
    #[test]
    fn a_damaged_baseline_is_a_refusal_rather_than_a_first_use() {
        let Ok(root) = tempfile::tempdir() else {
            return;
        };
        let path = root.path().join("lfx-baseline.json");
        assert!(
            std::fs::write(&path, "{ this is not json").is_ok(),
            "the damaged file should be written"
        );
        assert!(
            Baseline::read(&path).is_err(),
            "a damaged baseline must not read as an empty one"
        );
    }

    /// The digest is the bytes' own and nothing else's: one byte different is
    /// one digest different, and the same bytes twice are the same digest.
    #[test]
    fn a_moved_byte_is_a_moved_digest() {
        assert_eq!(digest(b"one frame"), digest(b"one frame"));
        assert_ne!(digest(b"one frame"), digest(b"one frame "));
        assert_ne!(digest(&[0_u8, 1, 2]), digest(&[0_u8, 2, 1]));
        assert_eq!(digest(b"").len(), 16, "the digest is sixteen hex digits");
    }
}
