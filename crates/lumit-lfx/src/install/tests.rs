//! Install is hostile-input testing (docs/impl/lfx.md §14 item 10).
//!
//! Everything here is a pack that should *not* install, or a step of one that
//! should. The cases that need a pack to install all the way through need a
//! broker executable and live in `lumit-lfx-broker`'s own suite, for the flat
//! Cargo reason the rest of this host's end-to-end work does:
//! `CARGO_BIN_EXE_lumit-lfx-broker` exists only inside the package that owns
//! the binary.

use std::io::Write;

use ed25519_dalek::{Signer, SigningKey};
use zip::write::SimpleFileOptions;

use super::*;

/// A signing key nobody's real pack will ever be under.
fn a_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// One file's worth of pack, as a name and its bytes.
struct Entry {
    name: String,
    bytes: Vec<u8>,
}

fn entry(name: &str, bytes: &[u8]) -> Entry {
    Entry {
        name: name.to_owned(),
        bytes: bytes.to_vec(),
    }
}

/// The three files a minimal well-formed bundle is made of, inside a pack.
///
/// The payload is not a library - nothing here opens it, because everything
/// here is refused before step 6 - but it is where the payload goes, so the
/// layout check sees what it would see in a real pack.
fn a_bundles_entries() -> Vec<Entry> {
    let arch = crate::bundle::arch_dirs()
        .first()
        .copied()
        .unwrap_or("none");
    vec![
        entry(
            "Example.lfx.bundle/Contents/lfx.toml",
            b"abi_version = 1\n\n[[plugin]]\nid = \"com.example.blur\"\nname = \"Blur\"\nvendor = \"Example\"\nversion = \"1.0.0\"\ncategories = [\"blur-sharpen\"]\nrequired_extensions = []\n",
        ),
        entry(
            &format!("Example.lfx.bundle/Contents/{arch}/Example.lfx"),
            b"not a library, and nothing in this file opens it",
        ),
    ]
}

/// A pack manifest declaring a digest for each entry.
fn a_manifest_over(entries: &[Entry]) -> String {
    let digests: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "    {:?}: {:?}",
                entry.name,
                hex(&Sha256::digest(&entry.bytes))
            )
        })
        .collect();
    format!(
        "{{\n  \"format\": \"lfxpack\",\n  \"id\": \"com.example.suite\",\n  \"name\": \"Example suite\",\n  \"vendor\": \"Example\",\n  \"version\": \"1.0.0\",\n  \"files\": {{\n{}\n  }}\n}}\n",
        digests.join(",\n")
    )
}

/// Write a pack, signing the manifest with `key` where there is one.
fn a_pack_at(path: &Path, manifest: &str, key: Option<&SigningKey>, entries: &[Entry]) {
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
    for entry in entries {
        zip.start_file(&entry.name, options).expect("an entry");
        zip.write_all(&entry.bytes).expect("its bytes");
    }
    zip.finish().expect("the pack is closed");
}

/// Somewhere to install into, and the options that point at it.
struct Site {
    _root: tempfile::TempDir,
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
        // No broker, because no case in this file reaches step 6: each is
        // refused before the staged bundle is manifested.
        exe: Some(root.path().join("no-such-broker")),
        env: Vec::new(),
    };
    Site {
        _root: root,
        addons,
        staging,
        options,
    }
}

/// Nothing installed, and nothing a scan could find if it ran now.
fn nothing_landed(site: &Site) {
    let found = crate::bundle::scan_dir(&site.addons);
    assert!(found.is_empty(), "a scan would find {found:?}");
    let staged: Vec<PathBuf> = std::fs::read_dir(&site.staging)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    assert!(
        staged.is_empty(),
        "the staging folder still holds {staged:?}"
    );
}

/// Where one entry's two headers are: the local one before its bytes and the
/// central-directory one after them.
///
/// A zip writes an entry's length and its compression method down **twice**, so
/// a pack that lies has to lie in both places. Nothing a library will do on
/// request, which is why these two tests write the bytes themselves.
fn headers_of(bytes: &[u8], name: &str) -> (usize, usize) {
    let (mut local, mut central) = (None, None);
    for at in 0..bytes.len().saturating_sub(46) {
        if bytes[at..at + 4] == [b'P', b'K', 3, 4] {
            let len = usize::from(u16::from_le_bytes([bytes[at + 26], bytes[at + 27]]));
            if bytes.get(at + 30..at + 30 + len) == Some(name.as_bytes()) {
                local = Some(at);
            }
        }
        if bytes[at..at + 4] == [b'P', b'K', 1, 2] {
            let len = usize::from(u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]));
            if bytes.get(at + 46..at + 46 + len) == Some(name.as_bytes()) {
                central = Some(at);
            }
        }
    }
    (
        local.expect("a local header for the entry"),
        central.expect("a central directory header for the entry"),
    )
}

/// Rewrite one entry's declared **uncompressed** length, leaving its bytes
/// where they are: a megabyte on disk that says it is two gigabytes, which is
/// the whole shape of a zip bomb.
fn declare_length(pack: &Path, name: &str, claim: u32) {
    let mut bytes = std::fs::read(pack).expect("the pack");
    let (local, central) = headers_of(&bytes, name);
    bytes[local + 22..local + 26].copy_from_slice(&claim.to_le_bytes());
    bytes[central + 24..central + 28].copy_from_slice(&claim.to_le_bytes());
    std::fs::write(pack, bytes).expect("the pack again");
}

/// Rewrite one entry's declared compression method, so the entry is *there* and
/// this build cannot read it.
fn declare_method(pack: &Path, name: &str, method: u16) {
    let mut bytes = std::fs::read(pack).expect("the pack");
    let (local, central) = headers_of(&bytes, name);
    bytes[local + 8..local + 10].copy_from_slice(&method.to_le_bytes());
    bytes[central + 10..central + 12].copy_from_slice(&method.to_le_bytes());
    std::fs::write(pack, bytes).expect("the pack again");
}

/// Install a pack written by `write`, and answer the refusal.
fn refused(site: &Site, write: impl FnOnce(&Path)) -> InstallError {
    let pack = site._root.path().join("example.lfxpack");
    write(&pack);
    let ledger = lumit_budget::Ledger::new();
    install(&pack, &site.options, &ledger).expect_err("this pack should not install")
}

// ------------------------------------------------------------ the names --

/// A `..` does not escape the staging folder, and does not get as far as being
/// a path at all: the sweep runs over every name before a byte is read.
#[test]
fn an_entry_that_climbs_out_of_the_staging_folder_is_refused() {
    let site = a_site();
    let mut entries = a_bundles_entries();
    entries.push(entry("../elsewhere.txt", b"somewhere else entirely"));
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
    });
    assert_eq!(refusal.key(), "addon_entry_name");
    assert!(
        matches!(&refusal, InstallError::EntryName { name } if name.contains("..")),
        "{refusal:?}"
    );
    nothing_landed(&site);
}

/// An absolute name is the same refusal, and so are the other three shapes of
/// "not a plain relative path".
#[test]
fn an_absolute_entry_name_is_refused() {
    for name in [
        "/etc/lumit.conf",
        "C:/windows/system32/lumit.dll",
        "Example.lfx.bundle\\Contents\\lfx.toml",
        ".hidden/Example.lfx.bundle",
    ] {
        assert!(!is_plain_entry_name(name), "{name:?} passed the sweep");
    }
    let site = a_site();
    let mut entries = a_bundles_entries();
    entries.push(entry("/etc/lumit.conf", b"not yours"));
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
    });
    assert_eq!(refusal.key(), "addon_entry_name");
    nothing_landed(&site);
}

/// And the names a bundle really is made of all pass, so the rule refuses the
/// attack rather than the format.
#[test]
fn the_names_a_bundle_is_made_of_are_plain() {
    let arch = crate::bundle::arch_dirs()
        .first()
        .copied()
        .unwrap_or("none");
    for name in [
        "Example.lfx.bundle/",
        "Example.lfx.bundle/Contents/lfx.toml",
        &format!("Example.lfx.bundle/Contents/{arch}/Example.lfx"),
    ] {
        assert!(is_plain_entry_name(name), "{name:?} was refused");
    }
}

/// How deep an entry's path may be is the **budget's** depth, charged
/// component by component, rather than a second eight written down beside it -
/// so raising the ingress limit is what raises the ceiling, and the two cannot
/// drift apart.
#[test]
fn an_entry_nested_deeper_than_the_budget_admits_is_refused() {
    let deep = "a/b/c/d/e/f/g/h/i.txt";
    assert!(
        !is_plain_entry_name(deep),
        "the predicate admitted a path of nine components"
    );
    assert_eq!(MAX_NAME_COMPONENTS, Limits::ADDON.depth as usize);

    let site = a_site();
    let mut entries = a_bundles_entries();
    entries.push(entry(deep, b"nine components deep"));
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
    });
    assert_eq!(refusal.key(), "ingress_depth");
    nothing_landed(&site);
}

// ------------------------------------------------------------- the sizes --

/// Both directions of the same lie, on the path a bundle's files really take.
/// An entry declaring more than is left of the budget is refused on its claim,
/// before a byte is read; one declaring little and producing more is refused on
/// the byte that proves it.
#[test]
fn an_entry_that_lies_about_its_length_is_refused_both_ways() {
    let dir = tempfile::tempdir().expect("a folder");
    let at = dir.path().join("Example.lfx.bundle/Contents/payload");

    let mut budget = Budget::new(Limits::ADDON);
    let mut honest = std::io::repeat(0).take(8);
    assert_eq!(
        unpack_entry(&mut honest, 8, &at, &mut budget).expect("an honest entry"),
        hex(&Sha256::digest([0u8; 8])),
    );
    assert_eq!(std::fs::read(&at).expect("what was written"), vec![0u8; 8]);

    // It claims eight bytes and produces a thousand: refused on the ninth.
    let mut liar = std::io::repeat(0).take(1000);
    let refusal = unpack_entry(&mut liar, 8, &at, &mut budget)
        .expect_err("an entry that produced more than it declared");
    assert!(
        matches!(refusal, InstallError::Ingress(IngressError::Bytes { .. })),
        "{refusal:?}"
    );

    // And it claims more than the ceiling: refused on the claim, with nothing
    // read at all - the reader here would produce for ever if it were asked.
    let mut bomb = std::io::repeat(0);
    let refusal = unpack_entry(&mut bomb, u64::MAX / 2, &at, &mut budget)
        .expect_err("an entry past the ceiling");
    assert_eq!(refusal.key(), "ingress_bytes");

    // The two small named files take the buffered route, and answer the same
    // way in both directions.
    let mut liar = std::io::repeat(0).take(1000);
    let refusal = entry_bytes(&mut liar, 8, &at, &mut budget)
        .expect_err("a named file that produced more than it declared");
    assert_eq!(refusal.key(), "ingress_bytes");
}

/// One entry may not claim the whole archive's budget. A gigabyte is the right
/// ceiling on a suite and the wrong one on a file, so there is a second and
/// smaller one, and it is the one a single enormous entry meets.
#[test]
fn an_entry_claiming_more_than_one_file_may_be_is_refused_on_its_own_ceiling() {
    let dir = tempfile::tempdir().expect("a folder");
    let at = dir.path().join("Example.lfx.bundle/Contents/payload");
    let mut budget = Budget::new(Limits::ADDON);
    let mut bomb = std::io::repeat(0);
    let refusal = unpack_entry(&mut bomb, PACK_ENTRY_MAX_BYTES + 1, &at, &mut budget)
        .expect_err("one file may not be a quarter of a gigabyte and more");
    assert_eq!(
        refusal,
        InstallError::Ingress(IngressError::Bytes {
            needed: PACK_ENTRY_MAX_BYTES + 1,
            limit: PACK_ENTRY_MAX_BYTES,
        })
    );
    assert!(!at.exists(), "the file was created before it was refused");
    assert_eq!(budget.spent_bytes(), 0, "the claim was charged anyway");
}

/// And the aggregate ceiling is the archive's: a run of entries each claiming
/// a mebibyte stops at the gigabyte however many of them there are.
#[test]
fn a_run_of_entries_stops_at_the_archives_byte_ceiling() {
    let dir = tempfile::tempdir().expect("a folder");
    let at = dir.path().join("Example.lfx.bundle/Contents/payload");
    let mut budget = Budget::new(Limits::ADDON);
    let mut spent = 0u64;
    let mut refusal = None;
    for _ in 0..2000 {
        // A reader that produces nothing: what is being charged is the claim,
        // which is the only number available before the decompressor is let go.
        let mut nothing = std::io::empty();
        match unpack_entry(&mut nothing, 1 << 20, &at, &mut budget) {
            Ok(_) => spent += 1 << 20,
            Err(error) => {
                refusal = Some(error);
                break;
            }
        }
    }
    let refusal = refusal.expect("a run past the ceiling is refused");
    assert_eq!(refusal.key(), "ingress_bytes");
    assert!(
        spent <= Limits::ADDON.bytes,
        "{spent} bytes were admitted past the ceiling"
    );
}

/// A zip bomb is refused **before it is written**, which is a claim about the
/// order and is therefore made over a real archive: one entry's declared length
/// is rewritten to two gigabytes without its bytes changing, and what is on
/// disk afterwards is the entries that came before it and nothing of this one.
#[test]
fn a_zip_bomb_is_refused_before_a_byte_is_written() {
    let site = a_site();
    let mut entries = a_bundles_entries();
    entries.push(entry(
        "Example.lfx.bundle/Contents/payload",
        b"a megabyte on disk, once it says it is two gigabytes",
    ));
    let manifest = a_manifest_over(&entries);
    let pack = site._root.path().join("bomb.lfxpack");
    a_pack_at(&pack, &manifest, Some(&a_key(1)), &entries);
    declare_length(&pack, "Example.lfx.bundle/Contents/payload", 2_000_000_000);

    // The unpack on its own, into a folder that is nobody's staging and so is
    // still there to be looked at afterwards.
    let into = site._root.path().join("unpacked");
    std::fs::create_dir_all(&into).expect("somewhere to unpack");
    let file = std::fs::File::open(&pack).expect("the pack");
    let mut archive = zip::ZipArchive::new(file).expect("a zip");
    let mut budget = Budget::new(Limits::ADDON);
    let names = entry_names(&mut archive, &mut budget).expect("plain names");
    let read = read_manifest(manifest.as_bytes()).expect("a manifest");
    let refusal = unpack(&mut archive, &names, &read, &into, &mut budget)
        .expect_err("an entry claiming two gigabytes is not unpacked");
    assert_eq!(refusal.key(), "ingress_bytes");
    assert!(
        into.join("Example.lfx.bundle/Contents/lfx.toml").is_file(),
        "the entries before the bomb were not written, so the order proves nothing"
    );
    assert!(
        !into.join("Example.lfx.bundle/Contents/payload").exists(),
        "the bomb was written and then refused"
    );

    // And through `install`, where the answer is the same and the staging
    // folder goes with it.
    let ledger = lumit_budget::Ledger::new();
    let refusal = install(&pack, &site.options, &ledger).expect_err("a bomb does not install");
    assert_eq!(refusal.key(), "ingress_bytes");
    nothing_landed(&site);
}

/// The manifest's ceiling and the budget's item ceiling are one ceiling seen
/// twice: every entry has to be declared, so a manifest too small to describe
/// the entries the budget admits would refuse an honest vendor on
/// `ingress_bytes` with no sentence about why.
#[test]
fn the_manifests_ceiling_admits_a_declaration_for_every_entry_the_budget_admits() {
    // One declaration, at the length a bundle's own names run to: a path, a
    // SHA-256 in hexadecimal, and the JSON around the two.
    let one = format!(
        "    {:?}: {:?},\n",
        "Example.lfx.bundle/Contents/Resources/gradient-0000000000.png",
        "0".repeat(64)
    );
    let all = u64::try_from(one.len()).expect("a length") * Limits::ADDON.items;
    assert!(
        all <= PACK_MANIFEST_MAX_BYTES,
        "a pack of {} entries needs a {all}-byte manifest and the ceiling is \
         {PACK_MANIFEST_MAX_BYTES}",
        Limits::ADDON.items
    );
}

// --------------------------------------------------------- the signature --

/// The signature is checked **before the JSON is parsed**, which is what makes
/// it a mechanism rather than a decoration: the manifest here is not JSON at
/// all, and the refusal is still the signature's.
#[test]
fn the_signature_is_checked_before_the_manifest_is_parsed() {
    let site = a_site();
    let entries = a_bundles_entries();
    let refusal = refused(&site, |pack| {
        // Signed over one text, shipped with another. The pack writer signs
        // what it is given, so the pack carries a good signature over the
        // wrong bytes - exactly what a swapped manifest looks like.
        let key = a_key(1);
        let file = std::fs::File::create(pack).expect("the pack");
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file(PACK_MANIFEST, options).expect("a manifest");
        zip.write_all(b"this is not JSON and never was")
            .expect("its bytes");
        zip.start_file(PACK_SIGNATURE, options)
            .expect("a signature");
        let mut signature = key.verifying_key().to_bytes().to_vec();
        signature.extend_from_slice(&key.sign(b"something else entirely").to_bytes());
        zip.write_all(&signature).expect("its bytes");
        for entry in &entries {
            zip.start_file(&entry.name, options).expect("an entry");
            zip.write_all(&entry.bytes).expect("its bytes");
        }
        zip.finish().expect("the pack is closed");
    });
    assert_eq!(refusal.key(), "addon_signature_invalid");
    nothing_landed(&site);
}

/// A signature that does not check is a refusal and never a fallback to the
/// weaker check - release-signing.md's rule, and its reason.
#[test]
fn a_signature_that_does_not_check_is_a_refusal_rather_than_a_fallback() {
    let site = a_site();
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
        // Rewrite the manifest under the same signature.
        let other = a_manifest_over(&entries).replace("Example suite", "Someone else's");
        let source = std::fs::File::open(pack).expect("the pack");
        let mut read = zip::ZipArchive::new(source).expect("a zip");
        let mut bytes = Vec::new();
        let mut signature = Vec::new();
        read.by_name(PACK_SIGNATURE)
            .expect("the signature")
            .read_to_end(&mut signature)
            .expect("its bytes");
        for index in 0..read.len() {
            let mut entry = read.by_index(index).expect("an entry");
            let name = entry.name().to_owned();
            if name == PACK_MANIFEST || name == PACK_SIGNATURE {
                continue;
            }
            let mut held = Vec::new();
            entry.read_to_end(&mut held).expect("its bytes");
            bytes.push((name, held));
        }
        drop(read);
        let file = std::fs::File::create(pack).expect("the pack again");
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file(PACK_MANIFEST, options).expect("a manifest");
        zip.write_all(other.as_bytes()).expect("its bytes");
        zip.start_file(PACK_SIGNATURE, options)
            .expect("a signature");
        zip.write_all(&signature).expect("its bytes");
        for (name, held) in bytes {
            zip.start_file(name, options).expect("an entry");
            zip.write_all(&held).expect("its bytes");
        }
        zip.finish().expect("the pack is closed");
    });
    assert_eq!(refusal.key(), "addon_signature_invalid");
    nothing_landed(&site);
}

/// A signature file that is not a signature says so by its length rather than
/// by failing to check, so the page can tell a corrupt pack from a wrong key.
#[test]
fn a_signature_of_the_wrong_length_is_refused_as_unreadable() {
    let site = a_site();
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        let file = std::fs::File::create(pack).expect("the pack");
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file(PACK_MANIFEST, options).expect("a manifest");
        zip.write_all(manifest.as_bytes()).expect("its bytes");
        zip.start_file(PACK_SIGNATURE, options)
            .expect("a signature");
        zip.write_all(b"too short to be one").expect("its bytes");
        zip.finish().expect("the pack is closed");
    });
    assert_eq!(refusal.key(), "addon_signature_unreadable");
    nothing_landed(&site);
}

/// An entry that is **there** and cannot be read is not an entry that is
/// absent. For the signature the difference is the whole mechanism: read as an
/// absence, "this pack carries a signature I cannot read" becomes "this pack is
/// unsigned", which is a downgrade with a calm line on it.
#[test]
fn a_signature_this_build_cannot_read_is_not_an_unsigned_pack() {
    let site = a_site();
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let pack = site._root.path().join("unreadable.lfxpack");
    a_pack_at(&pack, &manifest, Some(&a_key(1)), &entries);
    // A compression method no build has: the entry is in the archive and its
    // bytes cannot be had.
    declare_method(&pack, PACK_SIGNATURE, 0xffff);

    let file = std::fs::File::open(&pack).expect("the pack");
    let mut archive = zip::ZipArchive::new(file).expect("a zip");
    let mut budget = Budget::new(Limits::ADDON);
    let refusal = read_named(
        &mut archive,
        PACK_SIGNATURE,
        PACK_SIGNATURE_MAX_BYTES,
        &mut budget,
    )
    .expect_err("a signature that cannot be read is not a pack without one");
    assert_eq!(refusal.key(), "addon_not_a_pack");
}

// ------------------------------------------------------------ the digests --

/// The signature covers the manifest, so the manifest has to cover the bundle:
/// a file added to a signed pack is refused by name rather than unpacked
/// alongside the ones that were signed for.
#[test]
fn an_entry_the_signed_manifest_does_not_declare_is_refused() {
    let site = a_site();
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let mut with_extra = entries;
    with_extra.push(entry(
        "Example.lfx.bundle/Contents/extra.so",
        b"nobody signed for this",
    ));
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &with_extra);
    });
    assert_eq!(refusal.key(), "addon_entry_not_declared");
    nothing_landed(&site);
}

/// And an entry whose bytes are not the ones declared is refused on its digest,
/// which is the swap the whole arrangement exists to catch.
#[test]
fn an_entry_whose_bytes_are_not_the_declared_ones_is_refused() {
    let site = a_site();
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let mut swapped = entries;
    if let Some(payload) = swapped.last_mut() {
        payload.bytes = b"somebody else's build".to_vec();
    }
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &swapped);
    });
    assert_eq!(refusal.key(), "addon_digest_mismatch");
    nothing_landed(&site);
}

/// A manifest promising a payload the pack does not carry is refused too - the
/// same property from the other side, and the one a pack that was trimmed on
/// the way here breaks.
#[test]
fn a_declared_entry_the_pack_does_not_carry_is_refused() {
    let site = a_site();
    let mut entries = a_bundles_entries();
    entries.push(entry(
        "Example.lfx.bundle/Contents/promised.so",
        b"declared and then left out",
    ));
    let manifest = a_manifest_over(&entries);
    entries.pop();
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
    });
    assert_eq!(refusal.key(), "addon_entry_missing");
    nothing_landed(&site);
}

/// The manifest's own keys are swept too, and refused as the manifest rather
/// than as an entry. They are the one string in this file that is quoted back
/// out of a *stranger's structured text* rather than out of the archive's own
/// names, and `InstallError`'s doc says nothing unchecked is quoted.
#[test]
fn a_manifest_declaring_a_name_that_is_not_a_plain_path_is_refused() {
    for declared in [
        "../elsewhere.txt",
        "/etc/lumit.conf",
        &"a".repeat(LFX_MAX_STRING_BYTES as usize),
    ] {
        let site = a_site();
        let entries = a_bundles_entries();
        let manifest = a_manifest_over(&entries).replace(
            "\"files\": {",
            &format!("\"files\": {{\n    {declared:?}: {:?},", "0".repeat(64)),
        );
        let refusal = refused(&site, |pack| {
            a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
        });
        assert_eq!(refusal.key(), "addon_manifest_unreadable", "{declared:?}");
        nothing_landed(&site);
    }
}

// ------------------------------------------------------------- the layout --

/// A pack that unpacks cleanly and is not a bundle is refused by which part of
/// the layout it is missing, not by a sentence.
#[test]
fn a_pack_that_is_not_one_bundle_is_refused_by_what_it_is_missing() {
    for (entries, fault) in [
        (
            vec![entry("Readme.txt", b"a pack of nothing in particular")],
            LayoutFault::NoBundle,
        ),
        (
            vec![
                entry("One.lfx.bundle/Contents/lfx.toml", b"abi_version = 1\n"),
                entry("Two.lfx.bundle/Contents/lfx.toml", b"abi_version = 1\n"),
            ],
            LayoutFault::TwoBundles,
        ),
        (
            vec![entry(
                "Example.lfx.bundle/Contents/notes.txt",
                b"no listing",
            )],
            LayoutFault::NoListing,
        ),
        (
            vec![entry(
                "Example.lfx.bundle/Contents/lfx.toml",
                b"abi_version = 1\n",
            )],
            LayoutFault::NoPayloadHere,
        ),
    ] {
        let site = a_site();
        let manifest = a_manifest_over(&entries);
        let refusal = refused(&site, |pack| {
            a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
        });
        assert_eq!(refusal, InstallError::Layout(fault));
        assert_eq!(refusal.key(), "addon_layout");
        nothing_landed(&site);
    }
}

// -------------------------------------------------------------- the trust --

/// The comparison happens before anything is written, and the recording after
/// everything is: a pack refused at the layout check has claimed no key, so a
/// broken pack cannot take an identifier's fingerprint hostage.
#[test]
fn a_pack_that_did_not_install_records_no_key() {
    let site = a_site();
    let entries = vec![entry("Readme.txt", b"not a bundle at all")];
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(9)), &entries);
    });
    assert_eq!(refusal, InstallError::Layout(LayoutFault::NoBundle));
    let store = TrustStore::load(&site.options.trust).expect("the store reads");
    assert_eq!(store, TrustStore::default(), "a refused pack claimed a key");
}

/// A trust store that is there and will not read stops the install **before**
/// anything is unpacked, and by name. The preference file beside it would have
/// read the same bytes as "nothing has been switched off" (§11 item 16).
#[test]
fn a_damaged_trust_store_refuses_the_install_rather_than_re_trusting() {
    let site = a_site();
    std::fs::write(&site.options.trust, "{\"keys\": {\"com.example").expect("half a store");
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
    });
    assert_eq!(refusal.key(), "trust_store_unreadable");
    nothing_landed(&site);
}

/// A second pack for an addon already installed, under another key, is refused
/// by name - and refused before it is unpacked, so the swap never reaches the
/// disk at all.
#[test]
fn a_second_pack_under_a_new_key_is_refused_by_name() {
    let site = a_site();
    let mut store = TrustStore::default();
    store.record(
        &crate::trust::addon_key("com.example.suite"),
        Some(&fingerprint(&a_key(1).verifying_key().to_bytes())),
        "Example.lfx.bundle",
    );
    store
        .save(&site.options.trust)
        .expect("the store is written");

    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(2)), &entries);
    });
    assert_eq!(refusal.key(), "signature_changed");
    nothing_landed(&site);
}

/// The other half of the same defence, and the one a store consulted only for
/// signed packs would have missed: an addon installed under a key is not
/// replaced by a pack carrying **no** key. Otherwise the whole mechanism is
/// walked around by deleting one file out of the zip.
#[test]
fn an_unsigned_pack_for_an_addon_installed_under_a_key_is_refused() {
    let site = a_site();
    let mut store = TrustStore::default();
    store.record(
        &crate::trust::addon_key("com.example.suite"),
        Some(&fingerprint(&a_key(1).verifying_key().to_bytes())),
        "Example.lfx.bundle",
    );
    store
        .save(&site.options.trust)
        .expect("the store is written");

    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, None, &entries);
    });
    assert_eq!(refusal.key(), "signature_missing");
    nothing_landed(&site);
}

// -------------------------------------------------------------- the swap --

/// An upgrade is two renames, and the second one can fail: an indexer holding a
/// handle, a permission change, a volume that filled up in between. What was
/// installed is in the staging folder by then and the staging folder goes,
/// so the swap puts it back - the difference between "the install failed" and
/// "the install failed and deleted the plugin you had".
#[test]
fn a_failed_swap_leaves_the_bundle_that_was_installed_where_it_was() {
    let root = tempfile::tempdir().expect("a folder");
    let addons = root.path().join("addons");
    let staged = root.path().join("addons.staging");
    let installed = addons.join("Example.lfx.bundle").join(CONTENTS_DIR);
    std::fs::create_dir_all(&installed).expect("what is installed");
    std::fs::write(
        installed.join(MANIFEST_FILE),
        b"the listing already installed",
    )
    .expect("its listing");
    let fresh = staged.join("Example.lfx.bundle").join(CONTENTS_DIR);
    std::fs::create_dir_all(&fresh).expect("what would replace it");
    std::fs::write(fresh.join(MANIFEST_FILE), b"the listing that never landed")
        .expect("its listing");

    // The first rename is the platform's; the second is the one that fails,
    // which is not something a platform will do on request.
    let calls = std::cell::Cell::new(0usize);
    let refusal = swap(
        &staged.join("Example.lfx.bundle"),
        &addons,
        &staged,
        &|from, to| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err(std::io::Error::other("something has it open"))
            } else {
                std::fs::rename(from, to)
            }
        },
    )
    .expect_err("a swap whose second rename failed is not an install");
    assert_eq!(refusal.key(), "addon_io");
    assert_eq!(calls.get(), 3, "the displaced copy was not put back");

    assert_eq!(
        std::fs::read(installed.join(MANIFEST_FILE)).expect("still installed"),
        b"the listing already installed",
        "a failed swap deleted the plugin that was installed"
    );
}

// ------------------------------------------------------------- the pack --

/// A file that is not a pack at all, and one that is a zip with nothing in it,
/// are the same calm refusal rather than two different noises.
#[test]
fn a_file_that_is_not_a_pack_is_refused_calmly() {
    let site = a_site();
    let refusal = refused(&site, |pack| {
        std::fs::write(pack, b"this is a text file").expect("the file");
    });
    assert_eq!(refusal.key(), "addon_not_a_pack");

    let site = a_site();
    let refusal = refused(&site, |pack| {
        let file = std::fs::File::create(pack).expect("the pack");
        zip::ZipWriter::new(file).finish().expect("an empty zip");
    });
    assert_eq!(refusal.key(), "addon_not_a_pack");
    nothing_landed(&site);
}

/// A pack of a format this version does not install says which format it is
/// rather than failing somewhere later.
#[test]
fn a_pack_of_another_format_is_refused_by_the_format_it_declares() {
    let site = a_site();
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries).replace("\"lfxpack\"", "\"ofxpack\"");
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
    });
    assert_eq!(refusal.key(), "addon_not_this_format");
    nothing_landed(&site);
}

/// An install stopped part way leaves nothing that looks installed **and
/// nothing a scan can find**: the staging folder is outside every search path,
/// and it removes itself however the install ended.
#[test]
fn an_interrupted_unpack_leaves_nothing_installed_and_nothing_a_scan_can_find() {
    let site = a_site();
    // A well-formed bundle, and then one entry that stops the unpack half way
    // through - after the listing has been written and before the payload is.
    let mut entries = a_bundles_entries();
    entries.insert(
        1,
        entry("Example.lfx.bundle/Contents/half", b"and then a lie"),
    );
    let manifest = a_manifest_over(&entries).replace(
        &hex(&Sha256::digest(b"and then a lie")),
        &hex(&Sha256::digest(b"something quite different")),
    );
    let refusal = refused(&site, |pack| {
        a_pack_at(pack, &manifest, Some(&a_key(1)), &entries);
    });
    assert_eq!(refusal.key(), "addon_digest_mismatch");
    nothing_landed(&site);
    assert!(
        !site.addons.join("Example.lfx.bundle").exists(),
        "a half-written bundle landed"
    );
}

/// A signature file longer than a signature is refused the same way a short one
/// is, rather than as a file past a ceiling: the length is what tells a corrupt
/// pack from a wrong key, and it belongs in the sentence.
#[test]
fn a_signature_longer_than_one_is_refused_as_unreadable_too() {
    let site = a_site();
    let entries = a_bundles_entries();
    let manifest = a_manifest_over(&entries);
    let refusal = refused(&site, |pack| {
        let file = std::fs::File::create(pack).expect("the pack");
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file(PACK_MANIFEST, options).expect("a manifest");
        zip.write_all(manifest.as_bytes()).expect("its bytes");
        zip.start_file(PACK_SIGNATURE, options)
            .expect("a signature");
        zip.write_all(&vec![0u8; SIGNATURE_BYTES * 3])
            .expect("its bytes");
        zip.finish().expect("the pack is closed");
    });
    assert_eq!(
        refusal,
        InstallError::SignatureUnreadable {
            bytes: SIGNATURE_BYTES * 3
        }
    );
    nothing_landed(&site);
}
