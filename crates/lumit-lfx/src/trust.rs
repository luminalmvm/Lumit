//! Which key an addon has always arrived under (docs/impl/lfx.md §6.2 step 2,
//! §11 item 16).
//!
//! # In plain terms
//!
//! Lumit has no list of publishers it trusts, and inventing one would be
//! inventing a certificate authority for a plugin format nobody has shipped
//! against yet. What it has instead is a memory: the first time a `.lfxpack`
//! for some addon is installed, the key it was signed with is written down, and
//! every later pack for that same addon has to be signed with the same key. A
//! pack under a *different* key is refused by name.
//!
//! That is **trust on first use**, and its value is exact and narrow: it stops
//! a silent swap - somebody else's build arriving in place of the vendor's
//! next update - and it stops nothing at all on the first install, where there
//! is nothing to compare against. The page says so, and §11 item 14 is the
//! reminder that a fingerprint on screen reads as a guarantee to anyone who has
//! not been told this paragraph.
//!
//! # Why this is not in `plugins.json`
//!
//! `lumit_project::PluginPrefs` is deliberately fail-open: an absent *or
//! damaged* file reads as "nothing has been switched off", the parse error
//! swallowed. That is right for a list of switched-off plugins, where
//! losing the file costs a re-tick, and it is exactly wrong here. A trust store
//! that read a truncated file as "nothing is known" would forget every
//! fingerprint, and the very next pack - under any key at all - would install
//! as a first use and never be refused. A defence whose whole value is "stops a
//! silent swap" may not be defeated by deleting one JSON file the attacker's
//! own installer can reach.
//!
//! So this file distinguishes **three** cases where the preference file
//! distinguishes two:
//!
//! | on disk | answer |
//! |---|---|
//! | absent | a first use - there is nothing to compare against, and never has been |
//! | parsed | compared, and a different key is [`TrustError::Changed`] |
//! | present and unreadable | [`TrustError::Unreadable`], a refusal by name rather than a default |
//!
//! # Two ways to be somebody else
//!
//! "The same publisher as last time" is two questions, not one, because a pack
//! chooses **both** the identifier it is filed under and the directory name it
//! lands in, and nothing relates the two to each other.
//!
//! The first question is the key. A later pack under a *different* key is
//! [`TrustError::Changed`] - and a later pack under **no key at all** is
//! [`TrustError::SignatureMissing`], because a defence that a signed pack
//! cannot get past may not be walked around by deleting `manifest.json.sig`
//! out of the zip. An unsigned pack installs where nothing is known about the
//! addon; it does not install *over* one whose key is written down.
//!
//! The second question is the directory. What a pack replaces on disk is a
//! bundle directory name out of the archive, and an addon's key says nothing
//! about it: a pack under a fresh identifier and a fresh key would otherwise
//! be an ordinary first use that happens to overwrite somebody else's plugin.
//! So [`TrustStore::bundles`] writes down which addon's install put each
//! bundle where it is, and [`TrustStore::claim`] answers
//! [`TrustError::BundleClaimed`] for a pack that is not the one that put it
//! there.
//!
//! # The fingerprint
//!
//! The SHA-256 of the thirty-two bytes of the Ed25519 public key, in lowercase
//! hexadecimal. It is what is stored, what is compared and what the page shows -
//! the key itself is not kept, because what this store answers is "the same
//! one as last time?" and a digest answers that without keeping a copy of
//! anything a later reader might mistake for a trust root.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The store's file name, beside `plugins.json` and `addons.json` in the
/// application's own data area.
pub const TRUST_FILE: &str = "addon-trust.json";

/// The most the store may be.
///
/// An identifier and sixty-four hexadecimal characters per addon; a machine
/// with a thousand addons on it comes to a hundred kilobytes. A megabyte is an
/// order past that and is a number rather than no number - and unlike every
/// other ceiling in this crate it is met by a file Lumit wrote, so a file past
/// it is a file something else has been at.
pub const TRUST_STORE_MAX_BYTES: u64 = 1 << 20;

/// How many bytes an Ed25519 public key is.
pub const PUBLIC_KEY_BYTES: usize = 32;

/// The one word this host's addons are filed under.
///
/// §5.3 settles that the roster files a plugin under `<kind>:<identifier>`
/// rather than under the identifier alone, because two standards may
/// legitimately use the same reverse-DNS name for one vendor's two builds of
/// one effect. The same reasoning holds for this file and for the same reason:
/// [`TRUST_FILE`] is named for addons generally, and it is the file the next
/// host's packs would share, where a collision would be a wrong
/// [`TrustError::Changed`] on an innocent pack or - worse - a wrong
/// [`Trusted::Known`]. So the key carries the kind from the first line written
/// rather than being migrated once there is something to migrate.
///
/// The word is `PluginKind::Lfx`'s, spelled here rather than imported: a plugin
/// host does not depend on the project format (§5.2), and this is the same
/// `lfx` the namespace's match-name prefix is spelled with.
pub const ADDON_KIND: &str = "lfx";

/// The key an addon is filed under: [`ADDON_KIND`], a colon, and the
/// identifier its pack declared.
#[must_use]
pub fn addon_key(id: &str) -> String {
    format!("{ADDON_KIND}:{id}")
}

/// Where the trust store is kept.
///
/// `data_dir()` rather than `data_local_dir()`, which is the opposite of where
/// the bundles themselves go (§6.1): what roams here is a short list of
/// fingerprints, and a person who carries their profile between two machines
/// carrying their answer to "is this the same publisher as last time?" with
/// them is the behaviour to want. `None` only when the platform has no home
/// directory.
#[must_use]
pub fn addon_trust_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_dir().join(TRUST_FILE))
}

/// What the store had to say about a key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Trusted {
    /// The pack carried no signature at all, and nothing is known about this
    /// addon. It installs, with a calm line and no elevated capability of any
    /// kind (docs/12:653-656); no key is written down, because there is no key
    /// to write, and what is written down is only which bundle directory the
    /// install now owns.
    ///
    /// An unsigned pack for an addon whose key **is** written down is
    /// [`TrustError::SignatureMissing`] instead: this is the answer for a
    /// stranger, not a way of becoming one.
    Unsigned,
    /// The first pack this machine has seen for this addon. Nothing was
    /// compared, because there was nothing to compare against.
    FirstUse {
        /// The key it arrived under, now remembered.
        fingerprint: String,
    },
    /// The same key as last time.
    Known {
        /// The key it arrived under, as it was already written down.
        fingerprint: String,
    },
}

impl Trusted {
    /// The fingerprint, where there is one.
    #[must_use]
    pub fn fingerprint(&self) -> Option<&str> {
        match self {
            Trusted::Unsigned => None,
            Trusted::FirstUse { fingerprint } | Trusted::Known { fingerprint } => Some(fingerprint),
        }
    }
}

/// Why the store could not answer, or answered no.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TrustError {
    /// A pack for an addon this machine already knows, under another key.
    ///
    /// The one thing trust on first use is for. Neither fingerprint is a
    /// stranger's free text: both are hexadecimal digests this crate computed.
    #[error(
        "{id} was installed under the key {known} and this pack is signed with {offered}, so it \
         is not the same publisher's"
    )]
    Changed {
        /// The addon both packs claim to be.
        id: String,
        /// The key it has always arrived under.
        known: String,
        /// The key this pack arrived under.
        offered: String,
    },

    /// A pack for an addon this machine already knows, carrying no signature
    /// at all.
    ///
    /// The other half of [`TrustError::Changed`], and without it trust on first
    /// use is defeated by *deleting* a file rather than by forging one: an
    /// addon installed under a key would be replaced by an unsigned pack under
    /// the same identifier, which is the silent swap this store exists to stop,
    /// arrived at without any key instead of with the wrong one.
    #[error(
        "{id} was installed under the key {known} and this pack carries no signature at all, so \
         it cannot be the same publisher's"
    )]
    SignatureMissing {
        /// The addon this pack claims to be.
        id: String,
        /// The key it has always arrived under.
        known: String,
    },

    /// A pack whose bundle directory another addon's install put there.
    ///
    /// Both names are checked before they are quoted: the owner is a key this
    /// store wrote, and the bundle is a path component that has already passed
    /// the installer's own name sweep.
    #[error(
        "the bundle {bundle:?} was installed by {known} and this pack is {offered}, so it is not \
         this pack's to replace"
    )]
    BundleClaimed {
        /// The bundle directory name, as it stands in the addons directory.
        bundle: String,
        /// Whose install put it there.
        known: String,
        /// Who is asking to replace it.
        offered: String,
    },

    /// The file is there and could not be read as a trust store.
    ///
    /// A refusal, never a default - see the module header.
    #[error("the record of which keys addons were installed under could not be read: {reason}")]
    Unreadable {
        /// What the reader said, which is an `io` or `serde` sentence rather
        /// than anything out of the file.
        reason: String,
    },

    /// The file could not be written, so a first use could not be remembered.
    ///
    /// Also a refusal: an install that could not write its fingerprint down is
    /// an install whose next upgrade would be a first use again, which is the
    /// mechanism quietly switching itself off.
    #[error("the record of which keys addons were installed under could not be written: {reason}")]
    NotWritten {
        /// What the writer said.
        reason: String,
    },
}

impl TrustError {
    /// The stable id this refusal crosses the bridge under, as
    /// [`IngressError::key`](lumit_ingress::IngressError::key) does - so the
    /// Addons page writes one sentence per name rather than matching on prose
    /// (§4.3's trap, seen from the installer's side).
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            TrustError::Changed { .. } => "signature_changed",
            TrustError::SignatureMissing { .. } => "signature_missing",
            TrustError::BundleClaimed { .. } => "bundle_claimed",
            TrustError::Unreadable { .. } => "trust_store_unreadable",
            TrustError::NotWritten { .. } => "trust_store_unwritable",
        }
    }
}

/// Which key each addon was first installed under, and which bundle directory
/// each addon's install owns.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TrustStore {
    /// [`addon_key`] to key fingerprint. A map rather than a list so the file
    /// cannot grow two answers for one addon.
    ///
    /// An addon whose packs have all been unsigned has no row here: there is no
    /// key to remember, and a row saying so would be a row an attacker's pack
    /// would want to write.
    pub keys: BTreeMap<String, String>,
    /// Bundle directory name to the [`addon_key`] whose install put it there.
    ///
    /// The half [`TrustStore::keys`] cannot answer. A key is filed under the
    /// identifier a pack declared; what an install *replaces* is a directory
    /// name the same pack declared, and the two are relatives of nothing. So
    /// the name is written down beside its owner, and a pack under another
    /// identifier is not the one that may replace it - signed, unsigned or
    /// first use alike.
    ///
    /// *ponytail:* a vendor who renames their bundle between releases leaves
    /// the old name here, owned by them and no longer installed. That is the
    /// safe direction - the row refuses somebody else's pack rather than
    /// admitting one - and tidying it up wants the uninstall this package does
    /// not have.
    pub bundles: BTreeMap<String, String>,
}

impl TrustStore {
    /// Read the store from `path`.
    ///
    /// A path-taking reader rather than a field on the struct, for the reason
    /// §5.4 gives about `PluginPrefs`: a `path_override` field would round-trip
    /// through the very file it decides the location of.
    ///
    /// # Errors
    ///
    /// [`TrustError::Unreadable`] when the file is there and is not a trust
    /// store. An **absent** file is not an error: it is a machine that has
    /// installed nothing yet.
    pub fn load(path: &Path) -> Result<Self, TrustError> {
        // `symlink_metadata` rather than `exists`, so a dangling link at the
        // store's path is "there and unreadable" rather than "absent" - the
        // one shape of missing file that is somebody having removed the target.
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(TrustError::Unreadable {
                    reason: error.to_string(),
                })
            }
            Ok(_) => {}
        }
        let text =
            lumit_ingress::read_to_string_capped(path, TRUST_STORE_MAX_BYTES).map_err(|error| {
                TrustError::Unreadable {
                    reason: error.to_string(),
                }
            })?;
        serde_json::from_str(&text).map_err(|error| TrustError::Unreadable {
            reason: error.to_string(),
        })
    }

    /// Write the store to `path`, creating the directory.
    ///
    /// Written to a neighbouring file and renamed over, because a half-written
    /// trust store is the [`TrustError::Unreadable`] case for ever after and a
    /// power cut in the middle of `write` is the ordinary way to get one.
    ///
    /// # Errors
    ///
    /// [`TrustError::NotWritten`].
    pub fn save(&self, path: &Path) -> Result<(), TrustError> {
        let failed = |error: &dyn std::fmt::Display| TrustError::NotWritten {
            reason: error.to_string(),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| failed(&error))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| failed(&error))?;
        let beside = path.with_extension("json.writing");
        std::fs::write(&beside, text).map_err(|error| failed(&error))?;
        std::fs::rename(&beside, path).map_err(|error| failed(&error))
    }

    /// What this store has to say about the key a pack for `id` arrived under,
    /// where `fingerprint` is `None` for a pack carrying no signature.
    ///
    /// `id` is an [`addon_key`], not a bare identifier.
    ///
    /// # Errors
    ///
    /// [`TrustError::Changed`] when this addon has always arrived under another
    /// key, and [`TrustError::SignatureMissing`] when it has always arrived
    /// under *a* key and this pack carries none.
    pub fn check(&self, id: &str, fingerprint: Option<&str>) -> Result<Trusted, TrustError> {
        match (self.keys.get(id), fingerprint) {
            (None, None) => Ok(Trusted::Unsigned),
            (None, Some(offered)) => Ok(Trusted::FirstUse {
                fingerprint: offered.to_owned(),
            }),
            (Some(known), None) => Err(TrustError::SignatureMissing {
                id: id.to_owned(),
                known: known.clone(),
            }),
            (Some(known), Some(offered)) if known.eq_ignore_ascii_case(offered) => {
                Ok(Trusted::Known {
                    fingerprint: known.clone(),
                })
            }
            (Some(known), Some(offered)) => Err(TrustError::Changed {
                id: id.to_owned(),
                known: known.clone(),
                offered: offered.to_owned(),
            }),
        }
    }

    /// Whether the addon `id` may land in the bundle directory `bundle`.
    ///
    /// Asked of the *directory name*, which is the thing an install actually
    /// replaces, rather than of the identifier, which is the thing a key is
    /// filed under. A name nobody has claimed is free, and a name this addon
    /// claimed is its own to upgrade.
    ///
    /// # Errors
    ///
    /// [`TrustError::BundleClaimed`] when another addon's install put that
    /// bundle where it is.
    pub fn claim(&self, id: &str, bundle: &str) -> Result<(), TrustError> {
        match self.bundles.get(bundle) {
            None => Ok(()),
            Some(owner) if owner == id => Ok(()),
            Some(owner) => Err(TrustError::BundleClaimed {
                bundle: bundle.to_owned(),
                known: owner.clone(),
                offered: id.to_owned(),
            }),
        }
    }

    /// Write down what an install landed: the key it arrived under, where there
    /// was one, and the bundle directory it now owns.
    ///
    /// The key is never replaced. [`TrustStore::check`] has already refused a
    /// key that disagrees with the one recorded, so this never overwrites an
    /// answer - which is what stops an install *of* a swapped pack from being
    /// the thing that records the swapped key. The bundle row is written
    /// outright, because [`TrustStore::claim`] has already refused a name
    /// somebody else owns and what is left is this addon's own.
    pub fn record(&mut self, id: &str, fingerprint: Option<&str>, bundle: &str) {
        if let Some(fingerprint) = fingerprint {
            self.keys
                .entry(id.to_owned())
                .or_insert_with(|| fingerprint.to_owned());
        }
        self.bundles.insert(bundle.to_owned(), id.to_owned());
    }
}

/// The fingerprint of an Ed25519 public key: its SHA-256, in lowercase
/// hexadecimal.
#[must_use]
pub fn fingerprint(key: &[u8; PUBLIC_KEY_BYTES]) -> String {
    hex(&Sha256::digest(key))
}

/// Bytes as lowercase hexadecimal.
///
/// Written out rather than reached for through a dependency: this is the only
/// place in the crate that needs it, and `write!` into a `String` would be a
/// `Result` nobody can do anything with in a crate where `unwrap` is denied.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const NIBBLES: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(NIBBLES[usize::from(byte >> 4)]);
        out.push(NIBBLES[usize::from(byte & 0x0f)]);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A key whose fingerprint is something to compare against.
    fn a_key(seed: u8) -> [u8; PUBLIC_KEY_BYTES] {
        [seed; PUBLIC_KEY_BYTES]
    }

    /// The key an addon is filed under, spelled out once so a second host
    /// sharing this file cannot collide with this one.
    fn an_addon() -> String {
        addon_key("com.example.suite")
    }

    /// Nothing on disk is a machine that has installed nothing, which is the
    /// one case where a first use is the honest answer.
    #[test]
    fn an_absent_trust_store_is_a_first_use() {
        let dir = tempfile::tempdir().expect("a folder");
        let path = dir.path().join(TRUST_FILE);
        let store = TrustStore::load(&path).expect("an absent store reads as empty");
        assert_eq!(store, TrustStore::default());
        let print = fingerprint(&a_key(1));
        assert_eq!(
            store.check(&an_addon(), Some(&print)),
            Ok(Trusted::FirstUse {
                fingerprint: print.clone()
            })
        );
        // And an unsigned pack for an addon nobody knows is a stranger rather
        // than a refusal: it installs, and nothing is compared.
        assert_eq!(store.check(&an_addon(), None), Ok(Trusted::Unsigned));
    }

    /// The store files a key under **kind and identifier**, which is what §5.3
    /// settles for the roster and for the same reason: `addon-trust.json` is
    /// named for addons generally, and two standards may use one reverse-DNS
    /// name for one vendor's two builds of one effect.
    #[test]
    fn the_store_files_a_key_under_the_kind_and_the_identifier() {
        assert_eq!(addon_key("com.example.suite"), "lfx:com.example.suite");
        let mut store = TrustStore::default();
        store.record(
            &an_addon(),
            Some(&fingerprint(&a_key(1))),
            "Example.lfx.bundle",
        );
        assert!(store.keys.contains_key("lfx:com.example.suite"));
        assert!(
            !store.keys.contains_key("com.example.suite"),
            "the bare identifier is not a key"
        );
        // The same identifier under another kind is another addon, which is the
        // collision the prefix is here to keep apart.
        assert_eq!(
            store.check("ofx:com.example.suite", Some(&fingerprint(&a_key(2)))),
            Ok(Trusted::FirstUse {
                fingerprint: fingerprint(&a_key(2))
            })
        );
    }

    /// The whole of what trust on first use buys: the second pack has to be
    /// the same publisher's, and a different key is refused **by name** rather
    /// than by a sentence the page has to match on.
    #[test]
    fn a_later_pack_under_another_key_is_refused_by_name() {
        let dir = tempfile::tempdir().expect("a folder");
        let path = dir.path().join(TRUST_FILE);
        let first = fingerprint(&a_key(1));
        let second = fingerprint(&a_key(2));

        let mut store = TrustStore::default();
        store.record(&an_addon(), Some(&first), "Example.lfx.bundle");
        store.save(&path).expect("the store is written");

        let back = TrustStore::load(&path).expect("and read");
        assert_eq!(
            back.check(&an_addon(), Some(&first)),
            Ok(Trusted::Known {
                fingerprint: first.clone()
            }),
            "the same key is the same publisher"
        );
        let refusal = back
            .check(&an_addon(), Some(&second))
            .expect_err("another key is not the same publisher");
        assert_eq!(refusal.key(), "signature_changed");
        assert_eq!(
            refusal,
            TrustError::Changed {
                id: an_addon(),
                known: first,
                offered: second,
            }
        );
    }

    /// And the other half of it, which a store that only compared *keys* would
    /// have missed: a later pack carrying **no signature at all** is refused by
    /// name too. Otherwise the whole mechanism is walked around by deleting
    /// `manifest.json.sig` out of the zip - no key needed, only a zip without
    /// one.
    #[test]
    fn a_later_unsigned_pack_for_a_signed_addon_is_refused_by_name() {
        let first = fingerprint(&a_key(1));
        let mut store = TrustStore::default();
        store.record(&an_addon(), Some(&first), "Example.lfx.bundle");

        let refusal = store
            .check(&an_addon(), None)
            .expect_err("an unsigned pack is not the publisher who signed the last one");
        assert_eq!(refusal.key(), "signature_missing");
        assert_eq!(
            refusal,
            TrustError::SignatureMissing {
                id: an_addon(),
                known: first,
            }
        );
    }

    /// A key is filed under an identifier the pack chose; what an install
    /// replaces is a directory name the same pack chose. So the directory has
    /// an owner of its own, and a pack under a fresh identifier and a fresh key -
    /// an ordinary first use, refused by nothing the key can say - is not the
    /// one that may replace somebody else's bundle.
    #[test]
    fn a_pack_may_not_replace_a_bundle_another_addon_installed() {
        let mut store = TrustStore::default();
        store.record(
            &addon_key("com.acme.suite"),
            Some(&fingerprint(&a_key(1))),
            "Acme.lfx.bundle",
        );

        assert_eq!(
            store.claim(&addon_key("com.acme.suite"), "Acme.lfx.bundle"),
            Ok(()),
            "an addon may upgrade the bundle it installed"
        );
        assert_eq!(
            store.claim(&addon_key("com.evil.suite"), "Somewhere.lfx.bundle"),
            Ok(()),
            "a name nobody has claimed is free"
        );

        let refusal = store
            .claim(&addon_key("com.evil.suite"), "Acme.lfx.bundle")
            .expect_err("somebody else's bundle is not this pack's to replace");
        assert_eq!(refusal.key(), "bundle_claimed");
        assert_eq!(
            refusal,
            TrustError::BundleClaimed {
                bundle: "Acme.lfx.bundle".to_owned(),
                known: addon_key("com.acme.suite"),
                offered: addon_key("com.evil.suite"),
            }
        );
    }

    /// An unsigned addon has no key to remember and still owns what it
    /// installed: the bundle row is written for every install, because the
    /// directory it replaced would otherwise belong to whoever asked next.
    #[test]
    fn an_unsigned_install_writes_down_no_key_and_still_owns_its_bundle() {
        let mut store = TrustStore::default();
        store.record(&an_addon(), None, "Example.lfx.bundle");
        assert!(store.keys.is_empty(), "an unsigned pack claimed a key");
        assert_eq!(
            store.bundles.get("Example.lfx.bundle"),
            Some(&an_addon()),
            "an unsigned pack did not claim what it installed"
        );
        let refusal = store
            .claim(&addon_key("com.other.suite"), "Example.lfx.bundle")
            .expect_err("an unsigned addon's bundle is still somebody's");
        assert_eq!(refusal.key(), "bundle_claimed");
    }

    /// A file that is there and will not read is a **refusal**, never a
    /// default. The preference file beside it swallows exactly this error and
    /// answers "nothing has been switched off"; doing that here would forget
    /// every fingerprint and let the next pack under any key install as a first
    /// use (§11 item 16).
    #[test]
    fn a_damaged_trust_store_refuses_rather_than_re_trusting() {
        let dir = tempfile::tempdir().expect("a folder");
        let path = dir.path().join(TRUST_FILE);
        std::fs::write(&path, "{\"keys\": {\"com.example.suite\": ").expect("half a file");
        let refusal = TrustStore::load(&path).expect_err("a damaged store is not an empty one");
        assert_eq!(refusal.key(), "trust_store_unreadable");
        assert!(matches!(refusal, TrustError::Unreadable { .. }));
    }

    /// And a store longer than any honest one is the same refusal, met without
    /// reading it all.
    #[test]
    fn a_trust_store_past_the_ceiling_refuses_rather_than_re_trusting() {
        let dir = tempfile::tempdir().expect("a folder");
        let path = dir.path().join(TRUST_FILE);
        let mut store = TrustStore::default();
        // One honest entry, padded inside its own identifier, so the file that
        // is refused is one that would plainly have loaded under a larger
        // ceiling rather than one made of fields the struct ignores.
        store.record(
            &addon_key(&"com.example.".repeat(100_000)),
            Some(&fingerprint(&a_key(1))),
            "Example.lfx.bundle",
        );
        store.save(&path).expect("the long store is written");
        let refusal = TrustStore::load(&path).expect_err("a store past the ceiling is refused");
        assert_eq!(refusal.key(), "trust_store_unreadable");
    }

    /// Recording never overwrites. The check has already refused a key that
    /// disagrees, so an install that reached the write is either a first use or
    /// the key already written down - and a `record` that replaced would make
    /// installing a swapped pack the act that trusts the swapped key.
    #[test]
    fn a_recorded_key_is_never_replaced_by_a_later_one() {
        let first = fingerprint(&a_key(1));
        let mut store = TrustStore::default();
        store.record(&an_addon(), Some(&first), "Example.lfx.bundle");
        store.record(
            &an_addon(),
            Some(&fingerprint(&a_key(2))),
            "Example.lfx.bundle",
        );
        assert_eq!(store.keys.get(&an_addon()), Some(&first));
    }

    /// The fingerprint is the digest of the key and nothing else - sixty-four
    /// lowercase hexadecimal characters, the same for the same key and
    /// different for a different one.
    #[test]
    fn a_fingerprint_is_the_digest_of_the_key_the_pack_carried() {
        let one = fingerprint(&a_key(7));
        assert_eq!(one.len(), 64);
        assert!(one
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        assert_eq!(one, fingerprint(&a_key(7)));
        assert_ne!(one, fingerprint(&a_key(8)));
    }
}
