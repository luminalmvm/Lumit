//! The `.lfxpack` install, end to end (docs/impl/lfx.md §6, §14 item 10).
//!
//! The refusals that happen before a pack is ever staged are pinned in
//! `lumit-lfx`'s own suite, where they need nothing but a temporary directory.
//! What is here is every case that has to reach **step 6** - the staged bundle
//! manifested in a broker, out of process, before the install is confirmed -
//! and that needs the broker executable, which is
//! `CARGO_BIN_EXE_lumit-lfx-broker` and exists only inside the package that
//! owns the binary.
//!
//! No module is ever opened here. A bundle's listing is read without loading a
//! line of its code, which is the whole reason it can be read at all (§3.3), so
//! the payload these packs carry is a placeholder and the suite runs on a
//! machine that has never built the test plugin.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use lumit_budget::Ledger;
use lumit_lfx::install::{install, InstallError, InstallOptions, PACK_MANIFEST, PACK_SIGNATURE};
use lumit_lfx::trust::{addon_key, TrustStore, Trusted};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

/// A signing key nobody's real pack will ever be under.
fn a_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// Bytes as lowercase hexadecimal - the spelling the manifest declares digests
/// in, written here rather than reached for so the test agrees with the
/// installer by arithmetic rather than by calling it.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The architecture directory this machine's payload goes in.
fn arch_dir() -> &'static str {
    lumit_lfx::bundle::arch_dirs()[0]
}

/// The entries a one-plugin bundle is made of, inside a pack.
fn a_bundles_entries(id: &str, listing: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "Example.lfx.bundle/Contents/lfx.toml".to_owned(),
            listing.as_bytes().to_vec(),
        ),
        (
            format!("Example.lfx.bundle/Contents/{}/Example.lfx", arch_dir()),
            format!("the payload for {id}, which nothing here opens")
                .as_bytes()
                .to_vec(),
        ),
    ]
}

/// A listing one plugin declares.
fn a_listing(id: &str) -> String {
    format!(
        "abi_version = {}\n\n[[plugin]]\nid = {id:?}\nname = \"Example blur\"\nvendor = \"Example\"\nversion = \"1.2.3\"\ncategories = [\"blur-sharpen\"]\nrequired_extensions = []\n",
        lumit_lfx_abi::LFX_ABI_VERSION
    )
}

/// The pack's own manifest, declaring a digest for every entry.
fn a_manifest_over(id: &str, entries: &[(String, Vec<u8>)]) -> String {
    let digests: Vec<String> = entries
        .iter()
        .map(|(name, bytes)| format!("    {name:?}: {:?}", hex(&Sha256::digest(bytes))))
        .collect();
    format!(
        "{{\n  \"format\": \"lfxpack\",\n  \"id\": {id:?},\n  \"name\": \"Example suite\",\n  \"vendor\": \"Example\",\n  \"version\": \"1.2.3\",\n  \"files\": {{\n{}\n  }}\n}}\n",
        digests.join(",\n")
    )
}

/// Write a pack, signed with `key` where there is one.
fn a_pack_at(path: &Path, manifest: &str, key: Option<&SigningKey>, entries: &[(String, Vec<u8>)]) {
    let file = std::fs::File::create(path).expect("the pack");
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    zip.start_file(PACK_MANIFEST, options).expect("a manifest");
    zip.write_all(manifest.as_bytes()).expect("its bytes");
    if let Some(key) = key {
        zip.start_file(PACK_SIGNATURE, options)
            .expect("a signature");
        let mut signature = key.verifying_key().to_bytes().to_vec();
        signature.extend_from_slice(&key.sign(manifest.as_bytes()).to_bytes());
        zip.write_all(&signature).expect("its bytes");
    }
    for (name, bytes) in entries {
        zip.start_file(name.clone(), options).expect("an entry");
        zip.write_all(bytes).expect("its bytes");
    }
    zip.finish().expect("the pack is closed");
}

/// Somewhere to install into, with the real broker behind it.
struct Site {
    root: tempfile::TempDir,
    addons: PathBuf,
    staging: PathBuf,
    options: InstallOptions,
}

fn a_site() -> Site {
    let root = tempfile::tempdir().expect("a folder");
    let addons = root.path().join("addons");
    let staging = root.path().join("addons.staging");
    let options = InstallOptions {
        addons: addons.clone(),
        staging: staging.clone(),
        trust: root.path().join("addon-trust.json"),
        exe: Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker"))),
        env: Vec::new(),
    };
    Site {
        root,
        addons,
        staging,
        options,
    }
}

/// Write a pack into the site and install it.
fn install_at(
    site: &Site,
    name: &str,
    id: &str,
    listing: &str,
    key: Option<&SigningKey>,
) -> Result<lumit_lfx::Installed, InstallError> {
    let entries = a_bundles_entries(id, listing);
    let manifest = a_manifest_over(id, &entries);
    let pack = site.root.path().join(name);
    a_pack_at(&pack, &manifest, key, &entries);
    install(&pack, &site.options, &Ledger::new())
}

/// The staging folder holds nothing, whatever the install answered.
fn staging_is_empty(site: &Site) {
    let left: Vec<PathBuf> = std::fs::read_dir(&site.staging)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "the staging folder still holds {left:?}");
}

#[test]
fn a_signed_pack_installs_and_remembers_the_key_it_arrived_under() {
    let site = a_site();
    let id = "com.example.suite";
    let installed = install_at(
        &site,
        "example.lfxpack",
        id,
        &a_listing("com.example.blur"),
        Some(&a_key(1)),
    )
    .expect("a signed pack installs");

    assert_eq!(installed.id, id);
    assert_eq!(installed.label, "Example suite");
    assert_eq!(installed.vendor, "Example");
    assert_eq!(installed.version, "1.2.3");
    assert_eq!(installed.bundle, site.addons.join("Example.lfx.bundle"));

    // The listing came back from the **broker**, which is the whole of step 6:
    // a second process read the staged bundle's own text before the install was
    // confirmed, and what it read is what the Addons page draws without waiting
    // for a rescan.
    assert_eq!(installed.listed.len(), 1);
    assert_eq!(installed.listed[0].id, "com.example.blur");
    assert_eq!(installed.listed[0].name, "Example blur");

    // Trust on first use: nothing was compared, and the key is now written down.
    let print = match &installed.trust {
        Trusted::FirstUse { fingerprint } => fingerprint.clone(),
        other => panic!("the first install should be a first use, not {other:?}"),
    };
    let store = TrustStore::load(&site.options.trust).expect("the store reads");
    assert_eq!(store.keys.get(&addon_key(id)), Some(&print));
    // And which bundle directory this addon's install now owns, which is the
    // half the identifier alone cannot say: what an install replaces is a
    // directory name out of the archive, and the key is filed under another
    // name the same pack chose.
    assert_eq!(
        store.bundles.get("Example.lfx.bundle"),
        Some(&addon_key(id))
    );

    // And it landed where every host's walk looks, by one rename, with nothing
    // left staged.
    let found = lumit_lfx::bundle::scan_dir(&site.addons);
    assert_eq!(found, vec![site.addons.join("Example.lfx.bundle")]);
    staging_is_empty(&site);
}

#[test]
fn an_unsigned_pack_installs_and_says_so() {
    let site = a_site();
    let installed = install_at(
        &site,
        "unsigned.lfxpack",
        "com.example.unsigned",
        &a_listing("com.example.unsigned.blur"),
        None,
    )
    .expect("an unsigned pack installs");

    // It installs, and what it is is a typed answer rather than a sentence the
    // page has to match on: docs/12:653-656's calm line, and no elevated
    // capability of any kind.
    assert_eq!(installed.trust, Trusted::Unsigned);
    assert_eq!(installed.trust.fingerprint(), None);

    // No key is written down, because there is no key to write: the next pack
    // for this addon, under any key, is a first use and says so.
    let store = TrustStore::load(&site.options.trust).expect("the store reads");
    assert!(store.keys.is_empty(), "an unsigned pack claimed a key");
    // What *is* written down is which bundle directory it owns. An addon with
    // no key still has a plugin on disk, and the directory it landed in is not
    // the next pack's to overwrite for the asking.
    assert_eq!(
        store.bundles.get("Example.lfx.bundle"),
        Some(&addon_key("com.example.unsigned"))
    );
    assert!(installed.bundle.is_dir());
    staging_is_empty(&site);
}

/// An unsigned pack installs where nothing is known about the addon; it does
/// not install **over** one whose key is written down. Otherwise trust on first
/// use is defeated by deleting `manifest.json.sig` out of the zip - no key
/// needed, only a zip without one - which is the same silent swap
/// `signature_changed` refuses, reached from the other side.
#[test]
fn a_later_unsigned_pack_for_a_signed_addon_is_refused_and_the_installed_one_stays() {
    let site = a_site();
    let id = "com.example.suite";
    let first = install_at(
        &site,
        "first.lfxpack",
        id,
        &a_listing("com.example.blur"),
        Some(&a_key(1)),
    )
    .expect("the first pack installs");
    let listing =
        std::fs::read(first.bundle.join("Contents").join("lfx.toml")).expect("its listing");

    let refusal = install_at(
        &site,
        "unsigned.lfxpack",
        id,
        &a_listing("com.attacker.payload"),
        None,
    )
    .expect_err("an unsigned pack is not the publisher who signed the last one");
    assert_eq!(refusal.key(), "signature_missing");

    // The installed bundle is untouched, and what the store knows is unchanged.
    assert_eq!(
        std::fs::read(first.bundle.join("Contents").join("lfx.toml")).expect("still its listing"),
        listing
    );
    let store = TrustStore::load(&site.options.trust).expect("the store reads");
    assert_eq!(
        store.keys.get(&addon_key(id)),
        first.trust.fingerprint().map(str::to_owned).as_ref()
    );
    staging_is_empty(&site);
}

/// A key is filed under the identifier a pack declared; what an install
/// replaces is a bundle directory name the **same pack** declared. So a pack
/// under a fresh identifier and a fresh key - an ordinary first use, refused by
/// nothing a key can say - is still not the one that may overwrite a plugin the
/// person installed from somebody else.
#[test]
fn a_pack_may_not_replace_a_bundle_another_addon_installed() {
    let site = a_site();
    let acme = install_at(
        &site,
        "acme.lfxpack",
        "com.acme.suite",
        &a_listing("com.acme.blur"),
        Some(&a_key(1)),
    )
    .expect("the first vendor's pack installs");
    assert!(matches!(acme.trust, Trusted::FirstUse { .. }));

    let refusal = install_at(
        &site,
        "evil.lfxpack",
        "com.evil.suite",
        &a_listing("com.evil.blur"),
        Some(&a_key(2)),
    )
    .expect_err("somebody else's bundle is not this pack's to replace");
    assert_eq!(refusal.key(), "bundle_claimed");

    // What is installed is still the first vendor's, and the second pack took
    // no row of its own on the way out.
    assert_eq!(
        lumit_lfx::bundle::scan_dir(&site.addons),
        vec![site.addons.join("Example.lfx.bundle")]
    );
    let store = TrustStore::load(&site.options.trust).expect("the store reads");
    assert_eq!(
        store.bundles.get("Example.lfx.bundle"),
        Some(&addon_key("com.acme.suite"))
    );
    assert!(
        !store.keys.contains_key(&addon_key("com.evil.suite")),
        "a refused pack claimed a key"
    );
    staging_is_empty(&site);
}

/// An install that cannot write its fingerprint down installs **nothing**. The
/// record goes in before the rename for exactly this: a refusal reported after
/// the bundle had landed would be an addon installed and discoverable whose key
/// is not remembered, so the next pack under any key would be a first use -
/// `TrustError::NotWritten`'s own sentence about the mechanism quietly
/// switching itself off.
#[test]
fn an_install_that_cannot_record_its_key_installs_nothing() {
    let site = a_site();
    // A directory where `save` writes its neighbouring file, so the write
    // fails for anybody - including a test running as root, where a permission
    // bit would not.
    std::fs::create_dir_all(site.options.trust.with_extension("json.writing"))
        .expect("something in the way of the write");

    let refusal = install_at(
        &site,
        "example.lfxpack",
        "com.example.suite",
        &a_listing("com.example.blur"),
        Some(&a_key(1)),
    )
    .expect_err("an install that cannot remember its key is not an install");
    assert_eq!(refusal.key(), "trust_store_unwritable");

    assert!(
        lumit_lfx::bundle::scan_dir(&site.addons).is_empty(),
        "the bundle landed and the refusal was about the store"
    );
    staging_is_empty(&site);
}

#[test]
fn a_second_pack_under_a_new_key_is_refused_and_the_installed_one_stays() {
    let site = a_site();
    let id = "com.example.suite";
    let first = install_at(
        &site,
        "first.lfxpack",
        id,
        &a_listing("com.example.blur"),
        Some(&a_key(1)),
    )
    .expect("the first pack installs");
    let listing =
        std::fs::read(first.bundle.join("Contents").join("lfx.toml")).expect("its listing");

    let refusal = install_at(
        &site,
        "second.lfxpack",
        id,
        &a_listing("com.example.somebody-elses"),
        Some(&a_key(2)),
    )
    .expect_err("another key is not the same publisher");
    assert_eq!(refusal.key(), "signature_changed");

    // The installed bundle is untouched - the refusal happens before the pack
    // is unpacked at all, so a swap never reaches the disk.
    assert_eq!(
        std::fs::read(first.bundle.join("Contents").join("lfx.toml")).expect("still its listing"),
        listing
    );
    staging_is_empty(&site);

    // And the same publisher's next pack is the ordinary upgrade.
    let again = install_at(
        &site,
        "third.lfxpack",
        id,
        &a_listing("com.example.blur.two"),
        Some(&a_key(1)),
    )
    .expect("the same key upgrades");
    assert!(matches!(again.trust, Trusted::Known { .. }));
    assert_eq!(again.listed[0].id, "com.example.blur.two");
    assert_eq!(
        lumit_lfx::bundle::scan_dir(&site.addons),
        vec![site.addons.join("Example.lfx.bundle")],
        "an upgrade replaces the bundle rather than landing beside it"
    );
    staging_is_empty(&site);
}

#[test]
fn a_bundle_whose_listing_the_broker_cannot_read_is_refused_and_the_staging_folder_goes() {
    let site = a_site();
    // A layout that passes every check this process can make - one bundle, a
    // listing that is there, a payload for this machine - and a listing that is
    // not a listing. Only the second process finds that out, which is what step
    // 6 is for.
    let refusal = install_at(
        &site,
        "broken.lfxpack",
        "com.example.broken",
        "this file is not a listing and never was",
        Some(&a_key(3)),
    )
    .expect_err("a bundle with no readable listing is not installed");
    assert_eq!(refusal.key(), "addon_not_manifested");

    assert!(
        lumit_lfx::bundle::scan_dir(&site.addons).is_empty(),
        "a bundle the broker could not read landed anyway"
    );
    staging_is_empty(&site);

    // And it claimed no key: a pack that did not install does not take an
    // identifier's fingerprint with it.
    let store = TrustStore::load(&site.options.trust).expect("the store reads");
    assert_eq!(store, TrustStore::default());
}
