//! Installing a `.lfxpack` (docs/impl/lfx.md §6).
//!
//! # In plain terms
//!
//! A vendor hands over one file. Lumit opens it, checks that it is signed by
//! whoever signed the last one, unpacks it somewhere no scan can see, satisfies
//! itself that what came out is a bundle this machine can run, has a broker
//! read the bundle's own listing out of process, and only then moves it into
//! the one folder every plugin host searches. Nothing is written into
//! `C:\Program Files\Common Files\OFX\Plugins` and nothing asks for an
//! administrator: install is LFX-only, per user, and the directory it lands in
//! is Lumit's own (§6.1).
//!
//! The order is the whole design, and every step of it is a refusal rather than
//! a repair:
//!
//! 1. **Names before bytes.** Every entry in the archive has to be a relative
//!    path of plain names - no separator a person did not intend, no `..`, no
//!    drive letter, no link. The rule `isPlainFileName` encodes in Dart for
//!    downloaded assets, moved into Rust where the unpack is, and applied per
//!    path component because a bundle is a directory tree.
//! 2. **Signature before parse.** [`PACK_MANIFEST`] and its detached signature
//!    are both read under a byte cap, and the signature is checked **before the
//!    JSON is parsed**. A failure is a refusal, never a fallback to the weaker
//!    check - release-signing.md's rule and its reason, that falling back makes
//!    the whole mechanism decorative.
//! 3. **Trust on first use.** The key's fingerprint is compared against
//!    [`crate::trust`]'s record: a later pack under another key is refused, and
//!    so is a later pack under **no** key, because a defence a signed pack
//!    cannot get past may not be walked around by deleting one file out of the
//!    zip.
//! 4. **A bounded unpack**, entry by entry, under
//!    [`Limits::ADDON`](lumit_ingress::Limits::ADDON) - the two-sided check
//!    `lumit-project` reads a `.lum` with, so an entry that lies about its
//!    uncompressed length is refused rather than believed - into a staging
//!    folder that is **not inside any search path**.
//! 5. **A layout check**: exactly one `*.lfx.bundle`, a `Contents/lfx.toml`
//!    that is there, and a payload this machine's architecture can run.
//! 6. **Manifested in a broker before the install is confirmed**, out of
//!    process, under the peer handshake. A bundle whose listing cannot be read
//!    is refused and the staging folder goes.
//! 7. **Then, and only then, one rename** into the addons directory - of a
//!    bundle directory name this addon owns or nobody does, since the name an
//!    install overwrites is the pack's own choice and the key is filed under
//!    another one.
//!
//! # What the signature is worth, exactly
//!
//! The detached signature covers [`PACK_MANIFEST`] and nothing else, which on
//! its own would be a signature over a file naming a payload rather than over
//! the payload - a mechanism that stops nothing. So the manifest carries a
//! SHA-256 **per archive entry**, every entry has to be declared and every
//! declaration has to be an entry, and each one is hashed as it is unpacked.
//! The signature binds the manifest, the manifest binds the bytes, and trust on
//! first use binds the key to the addon. A pack with an added file, a swapped
//! payload or a missing one is refused by name at whichever of those three it
//! breaks.
//!
//! Even so: **nothing here may be described as verified** (§11 item 14). Step 6
//! is a smoke test, not verification; step 2 is only as strong as trust on
//! first use, which stops a silent swap and stops nothing on the first install.
//! An unsigned pack installs, with a calm line and no elevated capability of
//! any kind, and its digests prove only that the archive is whole.
//!
//! # Thread role
//!
//! Blocking, and not the interface thread's: it reads a file, decompresses it,
//! writes a directory tree and starts a second process. `install_addon` is
//! deliberately the one addon call on the bridge that is **not** `#[frb(sync)]`
//! (§7.2).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ed25519_dalek::{Signature, VerifyingKey};
use lumit_budget::Ledger;
use lumit_ingress::{Budget, IngressError, Limits};
use lumit_lfx_abi::LFX_MAX_STRING_BYTES;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::bundle;
use crate::discover::SCAN_FRAME;
use crate::ipc::broker::{Broker, BrokerConfig, BrokerError};
use crate::ipc::proto::{PixelDepth, PluginIdentity};
use crate::ipc::ring::RingPlan;
use crate::manifest::{CONTENTS_DIR, MANIFEST_FILE};
use crate::trust::{fingerprint, hex, TrustError, TrustStore, Trusted, PUBLIC_KEY_BYTES};

/// The extension a pack carries.
pub const PACK_EXTENSION: &str = "lfxpack";

/// What a pack says about itself.
pub const PACK_MANIFEST: &str = "manifest.json";

/// The detached signature over [`PACK_MANIFEST`].
///
/// Exactly [`SIGNATURE_BYTES`]: the thirty-two bytes of the Ed25519 public key
/// followed by the sixty-four of the signature. The key rides **with** the pack
/// rather than arriving out of band, because there is no out of band - there is
/// no certificate authority here and no list of publishers Lumit knows, and
/// what the signature is for is trust on first use, which compares this pack's
/// key against the last one's. A key that arrives separately would be a second
/// file with the same trust properties and one more way to be half present.
pub const PACK_SIGNATURE: &str = "manifest.json.sig";

/// The `format` a pack this version admits declares.
pub const PACK_FORMAT: &str = "lfxpack";

/// The most [`PACK_MANIFEST`] may be. It is a handful of short strings and one
/// digest per entry.
///
/// Sized **against** [`Limits::ADDON`](lumit_ingress::Limits::ADDON)`.items`
/// rather than picked separately, because every entry has to be declared here
/// and the two numbers are therefore one ceiling seen twice: a manifest too
/// small to describe the entries the item ceiling admits would refuse a vendor
/// on `ingress_bytes` with no sentence about why. Sixteen mebibytes is a name,
/// a SHA-256 in hexadecimal and the JSON around them for every one of a hundred
/// thousand entries, at the length a bundle's own names run to, and
/// `the_manifests_ceiling_admits_a_declaration_for_every_entry_the_budget_admits`
/// keeps the two together. A pack whose hundred thousand names are each a
/// kilobyte long is refused on this ceiling instead, which is the honest answer
/// for a pack whose only content is names.
pub const PACK_MANIFEST_MAX_BYTES: u64 = 16 << 20;

/// The public key and the signature, in that order.
pub const SIGNATURE_BYTES: usize = PUBLIC_KEY_BYTES + Signature::BYTE_SIZE;

/// The most [`PACK_SIGNATURE`] may be read as.
///
/// Deliberately looser than [`SIGNATURE_BYTES`] so that a signature file of the
/// wrong length is refused **as a signature** - by name, with its length, which
/// is what tells a corrupt pack from a wrong key - rather than as a file past a
/// ceiling. Four kilobytes is small enough that the looseness costs nothing.
pub const PACK_SIGNATURE_MAX_BYTES: u64 = 4 << 10;

/// The most any one entry in a pack may be.
///
/// The archive's own budget is a gigabyte, which is the right ceiling on a
/// *suite* and the wrong one on a file: an entry declaring exactly that would
/// otherwise be admitted and the unpack would carry a gigabyte through this
/// process, which a deflate stream of zeros buys for about a megabyte on disk.
/// Two hundred and fifty-six mebibytes is well past a universal macOS binary -
/// the largest single file a plugin suite ships - and well under the archive's
/// own ceiling, so the two refuse different things.
pub const PACK_ENTRY_MAX_BYTES: u64 = 256 << 20;

/// The most path components an entry's name may have.
///
/// A bundle is `Name.lfx.bundle/Contents/<arch>/Name.lfx`, which is four; eight
/// leaves room for a vendor's resources directory and refuses a name whose only
/// content is separators. It is
/// [`Limits::ADDON`](lumit_ingress::Limits::ADDON)`.depth` read here rather
/// than a second eight declared beside it: the budget is what charges the
/// depth, entry by entry, and this is the same number seen from a predicate
/// that has no budget to charge.
pub const MAX_NAME_COMPONENTS: usize = Limits::ADDON.depth as usize;

// -------------------------------------------------------------- refusals --

/// What a pack's layout was not.
///
/// A closed enumeration rather than a sentence, so the page prints one string
/// per case and `lfx-validator` can ask for one by name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutFault {
    /// Nothing in the pack is a `.lfx.bundle`.
    NoBundle,
    /// More than one is, and an install that picked would be picking.
    TwoBundles,
    /// The bundle carries no `Contents/lfx.toml`, so there is nothing to
    /// manifest and no row the Addons page could ever draw.
    NoListing,
    /// It carries no build this machine's architecture can run. Discovery lists
    /// such a bundle - a person who installed it elsewhere still wants its row
    /// (§5.3) - but installing one here would be putting a file on this machine
    /// that can only ever be a skip line.
    NoPayloadHere,
}

impl std::fmt::Display for LayoutFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let said = match self {
            LayoutFault::NoBundle => "it holds no plugin bundle",
            LayoutFault::TwoBundles => "it holds more than one plugin bundle",
            LayoutFault::NoListing => "the bundle in it carries no listing",
            LayoutFault::NoPayloadHere => {
                "the bundle in it holds no build this machine's architecture can run"
            }
        };
        f.write_str(said)
    }
}

/// Why a pack was not installed.
///
/// Typed, and every variant a calm sentence the page prints beside the file
/// that was dropped on it (docs/14 §4). Nothing here quotes anything out of the
/// pack that has not been checked: an entry name is quoted only after it has
/// passed [`is_plain_entry_name`] - the archive's own names in [`entry_names`]
/// and the manifest's declared ones in [`read_manifest`], which is why the
/// second of those is swept at all - and a fingerprint is a digest this crate
/// computed.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum InstallError {
    /// It is not a zip, or it has no [`PACK_MANIFEST`] in it.
    #[error("this file is not an addon pack: {reason}")]
    NotAPack {
        /// What the reader said.
        reason: String,
    },

    /// [`PACK_MANIFEST`] is not the JSON this version reads.
    #[error("the pack's own manifest could not be read: {reason}")]
    ManifestUnreadable {
        /// What the reader said.
        reason: String,
    },

    /// A pack of a format this version does not install.
    #[error("this pack declares the format {format:?}, and this version installs {PACK_FORMAT:?}")]
    NotThisFormat {
        /// What it declared, held to the ABI's own string ceiling before it is
        /// quoted.
        format: String,
    },

    /// An entry name that is not a relative path of plain names.
    #[error("the pack holds an entry named {name:?}, which is not a plain relative path")]
    EntryName {
        /// The name, as text.
        name: String,
    },

    /// An entry that is a link rather than a file. A link is a way to write
    /// outside the staging folder with a name that passes every other check.
    #[error("the pack holds a link named {name:?}, and an addon pack holds files")]
    EntryIsALink {
        /// The name, as text.
        name: String,
    },

    /// An entry the manifest does not declare a digest for - a file added to a
    /// signed pack after it was signed.
    #[error("the pack holds {name:?}, which its signed manifest does not declare")]
    EntryNotDeclared {
        /// The name, as text.
        name: String,
    },

    /// A digest the manifest declares for an entry that is not in the pack.
    #[error("the pack's manifest declares {name:?}, which is not in the pack")]
    EntryMissing {
        /// The name, as text.
        name: String,
    },

    /// An entry whose bytes are not the ones the manifest declares.
    #[error("the pack's {name:?} is not the file its signed manifest declares")]
    DigestMismatch {
        /// The name, as text.
        name: String,
    },

    /// The signature file is there and is not a signature.
    #[error("the pack's signature is {bytes} bytes, and a signature is {SIGNATURE_BYTES}")]
    SignatureUnreadable {
        /// How long it was.
        bytes: usize,
    },

    /// The signature does not check against the manifest.
    ///
    /// **A refusal, never a fallback.** Falling back to the weaker check would
    /// make the whole mechanism decorative - release-signing.md's rule, and it
    /// applies here unchanged.
    #[error("the pack's signature does not check against its manifest")]
    SignatureInvalid,

    /// The trust store said no, or could not be read or written (§6.2 step 2).
    #[error(transparent)]
    Trust(#[from] TrustError),

    /// What came out of the pack is not a bundle this machine can install.
    #[error("this pack is not one Lumit can install: {0}")]
    Layout(LayoutFault),

    /// A ceiling was reached while reading it.
    #[error(transparent)]
    Ingress(#[from] IngressError),

    /// The staged bundle's own listing could not be read in a broker, so the
    /// install is not confirmed (§6.2 step 6).
    #[error("the bundle's listing could not be read: {reason}")]
    NotManifested {
        /// What the broker said. A `BrokerError`'s own sentence, kept as text
        /// because it is rendered here and nothing downstream matches on it.
        reason: String,
    },

    /// A file could not be read, written, renamed or removed.
    #[error("{path} could not be written: {reason}")]
    Io {
        /// Which path.
        path: PathBuf,
        /// What the system said.
        reason: String,
    },
}

impl InstallError {
    /// The stable id this refusal crosses the bridge under, as
    /// [`IngressError::key`] does.
    ///
    /// Written as an exhaustive `match` with no `_` arm, so a variant added
    /// later has to be given a name rather than inheriting one.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            InstallError::NotAPack { .. } => "addon_not_a_pack",
            InstallError::ManifestUnreadable { .. } => "addon_manifest_unreadable",
            InstallError::NotThisFormat { .. } => "addon_not_this_format",
            InstallError::EntryName { .. } => "addon_entry_name",
            InstallError::EntryIsALink { .. } => "addon_entry_link",
            InstallError::EntryNotDeclared { .. } => "addon_entry_not_declared",
            InstallError::EntryMissing { .. } => "addon_entry_missing",
            InstallError::DigestMismatch { .. } => "addon_digest_mismatch",
            InstallError::SignatureUnreadable { .. } => "addon_signature_unreadable",
            InstallError::SignatureInvalid => "addon_signature_invalid",
            InstallError::Trust(error) => error.key(),
            InstallError::Layout(_) => "addon_layout",
            InstallError::Ingress(error) => error.key(),
            InstallError::NotManifested { .. } => "addon_not_manifested",
            InstallError::Io { .. } => "addon_io",
        }
    }
}

impl From<BrokerError> for InstallError {
    fn from(error: BrokerError) -> Self {
        InstallError::NotManifested {
            reason: error.to_string(),
        }
    }
}

/// An `io::Error` against the path it was about.
fn io_at(path: &Path) -> impl Fn(std::io::Error) -> InstallError + '_ {
    move |error| InstallError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

// ----------------------------------------------------------- the manifest --

/// What a pack says about itself, before any of it is unpacked.
///
/// Unknown fields are ignored rather than refused, so a later pack format that
/// grew a field still installs here; the fields that are read are the ones this
/// version acts on.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PackManifest {
    /// [`PACK_FORMAT`], or this is not a pack this version installs.
    pub format: String,
    /// The addon's own identifier - what the trust store files a key under.
    pub id: String,
    /// The name a person sees while it installs.
    pub name: String,
    /// Who wrote it.
    pub vendor: String,
    /// The release, as text.
    pub version: String,
    /// One SHA-256, in hexadecimal, per archive entry other than this file and
    /// its signature. This is what makes the signature mean something about the
    /// bundle rather than only about the manifest naming it.
    pub files: BTreeMap<String, String>,
}

/// What an install did.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// The addon's identifier, as its manifest declares it.
    pub id: String,
    /// The name a person sees.
    pub label: String,
    /// Who wrote it.
    pub vendor: String,
    /// The release, as text.
    pub version: String,
    /// Where the bundle now is - inside the addons directory, which is on every
    /// host's search path.
    pub bundle: PathBuf,
    /// What the trust store had to say, which is what the page's line about
    /// this install is drawn from.
    pub trust: Trusted,
    /// What the bundle's own listing declared, read in a broker as the last
    /// thing before it landed. The Addons page has its rows without waiting for
    /// a rescan.
    pub listed: Vec<PluginIdentity>,
}

/// Where an install writes, and how it reaches a broker.
///
/// Path-taking rather than reading the platform's directories itself, for the
/// reason §5.4 gives: a test needs somewhere else to write, and a field inside
/// the file that decides where the file goes is not that.
#[derive(Clone, Debug)]
pub struct InstallOptions {
    /// Where an installed bundle lands - `lumit_ipc::addons_dir()` in the
    /// shipping path.
    pub addons: PathBuf,
    /// Where a pack is unpacked first. **Not inside [`InstallOptions::addons`]**
    /// (§11 item 17); `lumit_ipc::staging_dir()` is its sibling for that reason.
    pub staging: PathBuf,
    /// The trust store - `crate::trust::addon_trust_path()` in the shipping
    /// path.
    pub trust: PathBuf,
    /// Where the broker executable is, if not beside Lumit's own.
    pub exe: Option<PathBuf>,
    /// Extra environment for the broker that reads the staged listing.
    pub env: Vec<(String, String)>,
}

impl InstallOptions {
    /// The three platform directories, where the platform has them.
    ///
    /// `None` on a machine with no home directory, which is a machine that
    /// installs nothing - the same answer `lumit_ipc::addons_dir` gives.
    #[must_use]
    pub fn standard() -> Option<Self> {
        Some(Self {
            addons: lumit_ipc::addons_dir()?,
            staging: lumit_ipc::staging_dir()?,
            trust: crate::trust::addon_trust_path()?,
            exe: None,
            env: Vec::new(),
        })
    }
}

// --------------------------------------------------------------- the walk --

/// Whether one path component is a name Lumit will write.
///
/// `^[A-Za-z0-9][A-Za-z0-9._-]*$` - the rule `isPlainFileName` encodes in Dart
/// for a downloaded asset, moved here where the unpack is. `.` and `..` fail it
/// on the first character, a separator of either kind fails it on its own
/// character, and a drive letter fails on the colon.
#[must_use]
pub fn is_plain_name(component: &str) -> bool {
    let mut chars = component.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// An archive entry's name as its path components, or `None` when it is not a
/// relative path of plain names.
///
/// A trailing separator is a directory entry and is allowed; anything else that
/// leaves an empty component - a leading separator, a doubled one - is not. How
/// *many* components there are is not judged here: that is the budget's depth,
/// charged by the caller, so the ceiling is one number rather than two that can
/// drift apart.
#[must_use]
pub fn entry_name_components(name: &str) -> Option<Vec<&str>> {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || name.contains('\\') {
        return None;
    }
    let components: Vec<&str> = trimmed.split('/').collect();
    components
        .iter()
        .all(|part| is_plain_name(part))
        .then_some(components)
}

/// Whether an archive entry's name is a relative path of plain names, no deeper
/// than [`MAX_NAME_COMPONENTS`].
///
/// The whole answer in one call, for a reader with no budget to charge the
/// depth to - the validator, and the tests that ask the rule about a name
/// rather than about a pack.
#[must_use]
pub fn is_plain_entry_name(name: &str) -> bool {
    entry_name_components(name).is_some_and(|parts| parts.len() <= MAX_NAME_COMPONENTS)
}

/// One entry's bytes, held in memory, refusing one that lies about its length
/// in either direction.
///
/// For [`PACK_MANIFEST`] and [`PACK_SIGNATURE`] **only**, which are the two
/// entries this process has to hold whole - the first is parsed and the second
/// is the proof over it - and which have ceilings of their own, a mebibyte and
/// four kilobytes, well under anything that costs a machine its memory. Every
/// other entry goes through [`unpack_entry`], which never holds one.
///
/// Both halves of the length are checked because either alone is not enough,
/// which is the argument `lumit-project`'s `entry_text` already carries for a
/// `.lum`: the declared uncompressed size is a cheap refusal but it is the
/// *archive's* claim and a crafted one can understate it, so the read is also
/// capped and a stream that produces more than it promised is refused on the
/// byte that proves it. The declared size is charged to the budget **before** a
/// byte is read.
///
/// Generic over the reader so a test can hand over one that claims one size and
/// produces another - the whole shape of the attack, and not something a real
/// archive library will do on request.
///
/// # Errors
///
/// [`InstallError::Ingress`] when the entry is past what is left of the budget
/// or produces more than it declared; [`InstallError::Io`] when the read fails.
pub fn entry_bytes(
    reader: &mut impl Read,
    declared: u64,
    name: &Path,
    budget: &mut Budget,
) -> Result<Vec<u8>, InstallError> {
    budget.take_items(1)?;
    budget.take_bytes(declared)?;
    // Sized from the claim, but only up to a megabyte: the budget has already
    // admitted the whole declared length, and reserving it up front would let
    // an entry that declares a gigabyte and produces nothing cost a gigabyte of
    // resident memory anyway.
    let mut bytes = Vec::with_capacity(lumit_ingress::checked_usize(declared.min(1 << 20))?);
    // One byte past what it claimed, so a stream that lied is caught by having
    // produced the extra byte rather than by being believed.
    reader
        .take(declared.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io_at(name))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > declared {
        return Err(InstallError::Ingress(IngressError::Bytes {
            needed: declared.saturating_add(1),
            limit: declared,
        }));
    }
    Ok(bytes)
}

/// A sink that hashes what it passes on.
///
/// What makes the unpack a stream rather than a buffer: the digest the manifest
/// declares is computed **as the bytes go past** on their way to the staged
/// file, so nothing is held whole and the entry never costs this process its
/// own size in memory.
struct Digesting<W> {
    inner: W,
    hasher: Sha256,
}

impl<W: std::io::Write> std::io::Write for Digesting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let wrote = self.inner.write(buf)?;
        self.hasher.update(&buf[..wrote]);
        Ok(wrote)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Write one entry out to `at`, and answer the digest of what was written.
///
/// The same two-sided length check [`entry_bytes`] makes, and the same order -
/// the declared length is charged to the budget **before** the file is created,
/// so a zip bomb is refused before a byte of it is written - with the buffer
/// taken out of the middle. An entry is copied from the decompressor through
/// [`Digesting`] into the staged file a block at a time, so what a gigabyte
/// costs this process is the block and not the gigabyte.
///
/// Per-entry as well as aggregate: [`PACK_ENTRY_MAX_BYTES`] refuses one file
/// claiming the whole archive's budget, which the budget alone would admit.
///
/// Generic over the reader for the same reason [`entry_bytes`] is: a test hands
/// over one that claims one size and produces another.
///
/// A refusal part way through leaves a part-written file where it was writing.
/// Nothing is done about it here and nothing needs to be: the whole staging
/// folder is removed by [`Staged`] however the install ended, and it is outside
/// every search path until then.
///
/// # Errors
///
/// [`InstallError::Ingress`] when the entry is past its own ceiling or what is
/// left of the budget, or produces more than it declared;
/// [`InstallError::Io`] when the write fails.
pub fn unpack_entry(
    reader: &mut impl Read,
    declared: u64,
    at: &Path,
    budget: &mut Budget,
) -> Result<String, InstallError> {
    budget.take_items(1)?;
    if declared > PACK_ENTRY_MAX_BYTES {
        return Err(InstallError::Ingress(IngressError::Bytes {
            needed: declared,
            limit: PACK_ENTRY_MAX_BYTES,
        }));
    }
    budget.take_bytes(declared)?;
    if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent).map_err(io_at(parent))?;
    }
    let file = std::fs::File::create(at).map_err(io_at(at))?;
    let mut sink = Digesting {
        inner: std::io::BufWriter::new(file),
        hasher: Sha256::new(),
    };
    // One byte past what it claimed, so a stream that lied is caught by having
    // produced the extra byte rather than by being believed.
    let written = std::io::copy(&mut reader.take(declared.saturating_add(1)), &mut sink)
        .map_err(io_at(at))?;
    std::io::Write::flush(&mut sink).map_err(io_at(at))?;
    if written > declared {
        return Err(InstallError::Ingress(IngressError::Bytes {
            needed: declared.saturating_add(1),
            limit: declared,
        }));
    }
    Ok(hex(&sink.hasher.finalize()))
}

/// A directory that removes itself, however the install ends.
///
/// §6.2 step 6 asks for an interrupted install to leave nothing that looks
/// installed **and nothing a scan can find**, and the staging folder is outside
/// every search path for exactly that reason - but a folder left behind grows
/// without bound, and the crash after the rename is the one that would leave
/// the old copy beside the new one for ever.
struct Staged {
    at: PathBuf,
}

impl Drop for Staged {
    fn drop(&mut self) {
        // Nothing to report and nobody to report it to: the install has already
        // answered, and a staging folder that outlives one attempt is swept by
        // the next install into the same parent.
        let _ = std::fs::remove_dir_all(&self.at);
    }
}

// ------------------------------------------------------------ the install --

/// Install one `.lfxpack`.
///
/// The order of the seven steps is in the module header, and it is the design
/// rather than an implementation detail: the signature is checked before the
/// JSON is parsed, the names before the bytes, and the rename last of all, so
/// that a pack refused at any step has written nothing into any directory a
/// scan looks in.
///
/// The ledger is the governor's, for the one broker this spawns to read the
/// staged bundle's listing; it is given back when that broker is dropped, which
/// is before this returns.
///
/// Blocking, and not the interface thread's.
///
/// # Errors
///
/// [`InstallError`], which names which of the seven steps said no.
pub fn install(
    pack: &Path,
    options: &InstallOptions,
    ledger: &Arc<Ledger>,
) -> Result<Installed, InstallError> {
    let mut budget = Budget::new(Limits::ADDON);
    let file = std::fs::File::open(pack).map_err(|error| InstallError::NotAPack {
        reason: error.to_string(),
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| InstallError::NotAPack {
        reason: error.to_string(),
    })?;

    // 1. Names before bytes. Every entry is swept before any of them is read,
    //    so a hostile name is refused without this process having opened it.
    let names = entry_names(&mut archive, &mut budget)?;

    // 2. Signature before parse.
    let manifest_bytes = read_named(
        &mut archive,
        PACK_MANIFEST,
        PACK_MANIFEST_MAX_BYTES,
        &mut budget,
    )?
    .ok_or_else(|| InstallError::NotAPack {
        reason: format!("it holds no {PACK_MANIFEST}"),
    })?;
    let signature = read_named(
        &mut archive,
        PACK_SIGNATURE,
        PACK_SIGNATURE_MAX_BYTES,
        &mut budget,
    )?;
    let key = match &signature {
        Some(bytes) => Some(verify(&manifest_bytes, bytes)?),
        None => None,
    };

    // Only now is a stranger's structured text parsed at all.
    let manifest = read_manifest(&manifest_bytes)?;

    // 3. Trust on first use. The comparison happens here, before anything is
    //    written; the *recording* happens once nothing is left that could
    //    refuse, because a pack that fails at step 5 has no business claiming
    //    an identifier's key for ever.
    //
    //    The store is consulted whether this pack is signed or not. Reaching it
    //    only through the `Some` arm would mean an addon installed under a key
    //    could be replaced by a pack carrying no key at all - the silent swap,
    //    arrived at by deleting a file out of the zip rather than by forging
    //    one.
    let mut store = TrustStore::load(&options.trust)?;
    let addon = crate::trust::addon_key(&manifest.id);
    let print = key.as_ref().map(fingerprint);
    let trust = store.check(&addon, print.as_deref())?;

    // 4. A bounded unpack, into a folder no scan looks in.
    let staged = Staged {
        at: options.staging.join(Uuid::now_v7().to_string()),
    };
    std::fs::create_dir_all(&staged.at).map_err(io_at(&staged.at))?;
    unpack(&mut archive, &names, &manifest, &staged.at, &mut budget)?;

    // 5. The layout, which is what says this pack is one this machine installs.
    let bundle = the_one_bundle(&staged.at)?;

    // 6. Manifested in a broker, out of process, before the install is
    //    confirmed. What comes back is the listing the Addons page draws its
    //    rows from without waiting for a rescan.
    let listed = manifested(&bundle, options, ledger)?;

    // 7. One rename, and the install is confirmed - of a name this addon owns.
    //    The trust store is keyed on the identifier the pack declared and what
    //    an install replaces is a bundle *directory name* the same pack
    //    declared, so the two are related here rather than nowhere: without
    //    this, a pack under a fresh identifier and a fresh key would be an
    //    ordinary first use that happens to overwrite somebody else's plugin.
    let name = bundle
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(InstallError::Layout(LayoutFault::NoBundle))?
        .to_owned();
    store.claim(&addon, &name)?;

    // What was learned is written down **before** the rename rather than after
    // it. `save` is a file write and can fail - a full disk, a read-only
    // profile, a roaming share offline - and a failure after the rename would
    // be a refusal reported for an install that had already happened, with the
    // fingerprint unrecorded: the mechanism quietly switching itself off, which
    // is the thing `TrustError::NotWritten`'s own sentence is against. Both
    // checks a pack can fail on its own account - the layout and the smoke test -
    // are already behind us, so nothing a broken pack controls reaches this
    // write.
    //
    // *ponytail:* what is left is the rename below failing after the record was
    // written, which remembers a key for an addon that did not land. The
    // vendor's own next pack is then `Known` rather than `FirstUse`, which is
    // the harmless direction of the trade; undoing the write would want the
    // store's previous bytes held open across the rename.
    store.record(&addon, print.as_deref(), &name);
    store.save(&options.trust)?;

    let landed = land(&bundle, &options.addons, &staged.at)?;

    Ok(Installed {
        id: manifest.id,
        label: manifest.name,
        vendor: manifest.vendor,
        version: manifest.version,
        bundle: landed,
        trust,
        listed,
    })
}

/// Every entry's name, swept against [`is_plain_entry_name`] and refused if any
/// of them is a link.
fn entry_names(
    archive: &mut zip::ZipArchive<std::fs::File>,
    budget: &mut Budget,
) -> Result<Vec<String>, InstallError> {
    let count = archive.len();
    budget.take_items(u64::try_from(count).unwrap_or(u64::MAX))?;
    let mut names = Vec::with_capacity(count.min(1024));
    for index in 0..count {
        let entry = archive
            .by_index(index)
            .map_err(|error| InstallError::NotAPack {
                reason: error.to_string(),
            })?;
        let name = entry.name().to_owned();
        if entry.is_symlink() {
            return Err(InstallError::EntryIsALink { name });
        }
        let Some(components) = entry_name_components(&name) else {
            return Err(InstallError::EntryName { name });
        };
        // The path-component ceiling is the budget's own `depth`, charged here
        // rather than declared a second time beside it: a name is checked
        // component by component, which is what that field is for, and a
        // ceiling nothing charges is a number a reader can raise without
        // changing any behaviour. The work is the bytes of name swept, which is
        // what refuses an archive whose only content is long names.
        budget.check_depth(u32::try_from(components.len()).unwrap_or(u32::MAX))?;
        budget.take_work(u64::try_from(name.len()).unwrap_or(u64::MAX))?;
        names.push(name);
    }
    Ok(names)
}

/// One named entry's bytes, or `None` when the archive has no such entry.
fn read_named(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    ceiling: u64,
    budget: &mut Budget,
) -> Result<Option<Vec<u8>>, InstallError> {
    // `FileNotFound` and nothing else is an absence. Every other reader error -
    // a compression method this build does not have, an encrypted entry, a
    // malformed local header - is a file that is *there* and cannot be read,
    // and reading those as "no such entry" would turn "this pack carries a
    // signature I cannot read" into "this pack is unsigned", which is a
    // downgrade rather than a refusal.
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => {
            return Err(InstallError::NotAPack {
                reason: error.to_string(),
            })
        }
    };
    let declared = entry.size();
    if declared > ceiling {
        return Err(InstallError::Ingress(IngressError::Bytes {
            needed: declared,
            limit: ceiling,
        }));
    }
    entry_bytes(&mut entry, declared, Path::new(name), budget).map(Some)
}

/// Check the detached signature over the manifest's bytes.
///
/// The signature file is the public key followed by the signature, and
/// `verify_strict` is the check rather than `verify`: it refuses the small-order
/// keys and the non-canonical encodings that make one signature check against
/// two different messages, which is precisely the property "the same publisher
/// as last time" is built on.
fn verify(manifest: &[u8], signature: &[u8]) -> Result<[u8; PUBLIC_KEY_BYTES], InstallError> {
    if signature.len() != SIGNATURE_BYTES {
        return Err(InstallError::SignatureUnreadable {
            bytes: signature.len(),
        });
    }
    let (key_bytes, proof_bytes) = signature.split_at(PUBLIC_KEY_BYTES);
    let mut key = [0u8; PUBLIC_KEY_BYTES];
    key.copy_from_slice(key_bytes);
    let mut proof = [0u8; Signature::BYTE_SIZE];
    proof.copy_from_slice(proof_bytes);

    let verifying = VerifyingKey::from_bytes(&key).map_err(|_| InstallError::SignatureInvalid)?;
    verifying
        .verify_strict(manifest, &Signature::from_bytes(&proof))
        .map_err(|_| InstallError::SignatureInvalid)?;
    Ok(key)
}

/// The pack's own manifest, once its signature has been checked.
fn read_manifest(bytes: &[u8]) -> Result<PackManifest, InstallError> {
    let text = std::str::from_utf8(bytes).map_err(|error| InstallError::ManifestUnreadable {
        reason: error.to_string(),
    })?;
    let manifest: PackManifest =
        serde_json::from_str(text).map_err(|error| InstallError::ManifestUnreadable {
            reason: error.to_string(),
        })?;
    // The strings are quoted back to a person, so they are held to the ABI's
    // own ceiling before anything keeps them - the same number the describe
    // sink holds a plugin's strings to, met here where a pack's arrive.
    for field in [
        &manifest.format,
        &manifest.id,
        &manifest.name,
        &manifest.vendor,
        &manifest.version,
    ] {
        if field.len() >= LFX_MAX_STRING_BYTES as usize {
            return Err(InstallError::ManifestUnreadable {
                reason: format!(
                    "a field longer than the {LFX_MAX_STRING_BYTES} bytes the ABI carries"
                ),
            });
        }
    }
    if manifest.format != PACK_FORMAT {
        return Err(InstallError::NotThisFormat {
            format: manifest.format,
        });
    }
    if manifest.id.is_empty() || !manifest.id.is_ascii() {
        return Err(InstallError::ManifestUnreadable {
            reason: "it declares no identifier this version can file a key under".to_owned(),
        });
    }
    // And so are the names it declares, which are quoted back by
    // `EntryMissing` - the one thing in this file that prints a string the
    // *manifest* chose rather than one the archive did. Held to the same rule
    // the archive's own names are held to, and refused as the manifest rather
    // than as an entry, because a declaration for a name no entry could have is
    // a manifest that is wrong rather than a pack that is short of a file.
    for declared in manifest.files.keys() {
        if declared.len() >= LFX_MAX_STRING_BYTES as usize || !is_plain_entry_name(declared) {
            return Err(InstallError::ManifestUnreadable {
                reason: "it declares a file whose name is not a plain relative path".to_owned(),
            });
        }
    }
    Ok(manifest)
}

/// Write every entry out, hashing each one against what the manifest declares.
fn unpack(
    archive: &mut zip::ZipArchive<std::fs::File>,
    names: &[String],
    manifest: &PackManifest,
    into: &Path,
    budget: &mut Budget,
) -> Result<(), InstallError> {
    let mut seen: Vec<&str> = Vec::with_capacity(manifest.files.len());
    for name in names {
        if name == PACK_MANIFEST || name == PACK_SIGNATURE {
            continue;
        }
        let at = into.join(name.strip_suffix('/').unwrap_or(name));
        if name.ends_with('/') {
            std::fs::create_dir_all(&at).map_err(io_at(&at))?;
            continue;
        }
        let Some(declared_digest) = manifest.files.get(name) else {
            return Err(InstallError::EntryNotDeclared { name: name.clone() });
        };
        let mut entry = archive
            .by_name(name)
            .map_err(|error| InstallError::NotAPack {
                reason: error.to_string(),
            })?;
        let declared = entry.size();
        let digest = unpack_entry(&mut entry, declared, &at, budget)?;
        drop(entry);
        if !digest.eq_ignore_ascii_case(declared_digest) {
            return Err(InstallError::DigestMismatch { name: name.clone() });
        }
        seen.push(name);
    }
    // And every declaration has to have been an entry, so a signed manifest
    // cannot promise a payload the pack does not carry.
    for declared in manifest.files.keys() {
        if !seen.iter().any(|name| name == declared) {
            return Err(InstallError::EntryMissing {
                name: declared.clone(),
            });
        }
    }
    Ok(())
}

/// The one `*.lfx.bundle` in the staged tree, with its listing and a payload
/// for this machine.
fn the_one_bundle(staged: &Path) -> Result<PathBuf, InstallError> {
    let mut found: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(staged).map_err(io_at(staged))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(bundle::BUNDLE_SUFFIX))
        {
            found.push(path);
        }
    }
    found.sort();
    let bundle = match found.len() {
        0 => return Err(InstallError::Layout(LayoutFault::NoBundle)),
        1 => found.remove(0),
        _ => return Err(InstallError::Layout(LayoutFault::TwoBundles)),
    };
    // **Checked for, never read.** The listing is a stranger's structured text
    // and parsing it is the broker's (§11 item 7); what the installer may ask
    // is whether the file is there at all.
    if !bundle.join(CONTENTS_DIR).join(MANIFEST_FILE).is_file() {
        return Err(InstallError::Layout(LayoutFault::NoListing));
    }
    if bundle::payload(&bundle).is_none() {
        return Err(InstallError::Layout(LayoutFault::NoPayloadHere));
    }
    Ok(bundle)
}

/// Have a broker read the staged bundle's own listing, out of process.
///
/// A smoke test rather than verification, and the note says so where the page
/// will: what it proves is that a second process can read the listing of the
/// thing about to be installed, which is the one question a bundle that
/// unpacked cleanly and is still no good answers no to.
fn manifested(
    bundle: &Path,
    options: &InstallOptions,
    ledger: &Arc<Ledger>,
) -> Result<Vec<PluginIdentity>, InstallError> {
    let module = bundle::payload(bundle).unwrap_or_default();
    let mut config = BrokerConfig::new(
        bundle,
        module,
        RingPlan::frame(SCAN_FRAME.0, SCAN_FRAME.1, PixelDepth::F16),
    );
    config.exe.clone_from(&options.exe);
    config.env.clone_from(&options.env);
    let mut broker = Broker::spawn(config, ledger)?;
    let listed = broker.manifest()?.to_vec();
    Ok(listed)
}

/// Move the staged bundle into the addons directory.
///
/// One rename, on the volume the staging folder was deliberately put on. An
/// upgrade - a bundle of this name already installed - has the old one moved
/// aside into the staging folder first, so the swap is two renames rather than
/// a delete and a copy, and the old copy goes with the staging folder once the
/// new one is in.
///
/// *ponytail:* on Windows a directory holding a loaded module may refuse to
/// move, so upgrading a plugin a running Lumit has open can answer
/// [`InstallError::Io`] where every other platform succeeds. The honest fix is
/// to shut that bundle's broker down first, which wants the instance table this
/// package does not have.
fn land(bundle: &Path, addons: &Path, staged: &Path) -> Result<PathBuf, InstallError> {
    swap(bundle, addons, staged, &|from, to| {
        std::fs::rename(from, to)
    })
}

/// The swap itself, over the rename it is made of.
///
/// Split out for the failure **between** the two renames, which is the one that
/// matters and the one no platform will produce on request: an indexer or an
/// antivirus holding a handle, a permission change, a volume that filled up
/// between one call and the next. The old copy is in the staging folder by then
/// and the staging folder is removed however the install ended, so a second
/// rename that fails and says nothing more is the difference between "the
/// install failed" and "the install failed and deleted the plugin you had".
/// So it is put back.
fn swap(
    bundle: &Path,
    addons: &Path,
    staged: &Path,
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<PathBuf, InstallError> {
    std::fs::create_dir_all(addons).map_err(io_at(addons))?;
    let name = bundle
        .file_name()
        .ok_or(InstallError::Layout(LayoutFault::NoBundle))?;
    let landed = addons.join(name);
    let mut displaced = None;
    if landed.exists() {
        let aside = staged.join("replaced");
        rename(&landed, &aside).map_err(io_at(&landed))?;
        displaced = Some(aside);
    }
    if let Err(error) = rename(bundle, &landed) {
        if let Some(aside) = &displaced {
            if let Err(restore) = rename(aside, &landed) {
                // Both directions have failed, so there is nothing left to try
                // and the sentence says both halves rather than only the first.
                return Err(InstallError::Io {
                    path: landed,
                    reason: format!(
                        "{error}, and the copy it would have replaced could not be put back: \
                         {restore}"
                    ),
                });
            }
        }
        return Err(io_at(&landed)(error));
    }
    Ok(landed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
