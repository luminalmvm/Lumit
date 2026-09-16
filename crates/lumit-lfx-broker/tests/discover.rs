//! The scan that turns a folder of bundles into effects (docs/impl/lfx.md §5).
//!
//! **One test, deliberately.** [`lumit_lfx::discover`] keeps what it has
//! discovered and what it has refused in process-wide tables - that is what
//! makes a rescan idempotent and a re-enable instant - so two test functions
//! scanning the same fixture in the same binary would be reading each other's
//! rows. Everything the package promises about a scan is therefore asserted in
//! one run, in the order it happens.
//!
//! It lives in the broker's crate for the flat Cargo reason the rest of this
//! directory does: `CARGO_BIN_EXE_lumit-lfx-broker` exists only inside the
//! package that owns the binary, and there is no in-process scan to fall back
//! on - version 1 opens no stranger's module in this process, and a bundle's
//! own listing is the broker's to parse (§11 item 7).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lumit_budget::Ledger;
use lumit_lfx::bundle::{BUNDLE_SUFFIX, PAYLOAD_EXTENSION, PLUGIN_PATH_ENV};
use lumit_lfx::manifest::{CONTENTS_DIR, FAMILY_NAMES, MANIFEST_FILE};
use lumit_lfx::{discover, LfxRejection, ScanOptions};
use lumit_lfx_testplug::{Personality, PERSONALITIES};

/// The test bundle's payload name on this platform.
fn cdylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_lfx_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_lfx_testplug.dylib"
    } else {
        "liblumit_lfx_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
fn built_cdylib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..3 {
        for candidate in [
            dir.join(cdylib_name()),
            dir.join("deps").join(cdylib_name()),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = dir.parent()?;
    }
    None
}

/// The architecture directory this machine's payload goes in - one string,
/// because a test builds for the machine it runs on. The ordered list a scan
/// tries is `lumit_lfx::bundle::arch_dirs`, and its first entry is the one a
/// build for this machine lands in.
fn arch_dir() -> &'static str {
    lumit_lfx::bundle::arch_dirs()[0]
}

/// The id a personality declares, as text.
fn id_of(personality: Personality) -> String {
    personality.id().to_string_lossy().into_owned()
}

/// The listing a well-formed bundle of the test plugin carries: every
/// personality, spelled the way its own descriptor answers.
///
/// Written from the fixture's own table rather than by hand, because the
/// re-check the scan runs compares the two records field for field - a
/// hand-written listing would test the typist.
fn a_manifest() -> String {
    let mut text = format!("abi_version = {}\n", lumit_lfx_abi::LFX_ABI_VERSION);
    for personality in PERSONALITIES {
        let (major, minor, patch) = personality.version();
        let families: Vec<String> = personality
            .categories()
            .iter()
            .map(|declared| {
                // No fallback. The re-check compares categories exactly, so a
                // substituted name would make the generated listing disagree
                // with the code and refuse every plugin in the fixture as a
                // `ManifestMismatch` - a failure pointing at the re-check
                // rather than at the generator that fabricated it.
                FAMILY_NAMES
                    .iter()
                    .find(|(_, number)| number == declared)
                    .map_or_else(
                        || {
                            panic!(
                                "the fixture declares category {declared}, which \
                                 FAMILY_NAMES cannot spell"
                            )
                        },
                        |(name, _)| (*name).to_owned(),
                    )
            })
            .collect();
        let extensions: Vec<String> = personality
            .required_extensions()
            .iter()
            .map(|id| {
                String::from_utf8_lossy(id)
                    .trim_end_matches('\0')
                    .to_owned()
            })
            .collect();
        text.push_str(&format!(
            "\n[[plugin]]\nid = {:?}\nname = {:?}\nvendor = {:?}\nversion = \"{major}.{minor}.{patch}\"\ncategories = [{}]\nrequired_extensions = [{}]\n",
            id_of(personality),
            personality.name().to_string_lossy(),
            lumit_lfx_testplug::VENDOR.to_string_lossy(),
            families
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            extensions
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    text
}

/// A one-plugin listing for a bundle whose code this test never expects to run.
fn a_listing_for(id: &str) -> String {
    format!(
        "abi_version = {}\n\n[[plugin]]\nid = {id:?}\nname = \"Elsewhere\"\nvendor = \"Example\"\nversion = \"1.0.0\"\ncategories = [\"utility\"]\nrequired_extensions = []\n",
        lumit_lfx_abi::LFX_ABI_VERSION
    )
}

/// Lay a bundle out inside `root`, with `payload` written into the
/// architecture directory this machine reads.
fn a_bundle_in(root: &Path, name: &str, manifest: &str, payload: Option<&Path>) -> PathBuf {
    let bundle = root.join(format!("{name}{BUNDLE_SUFFIX}"));
    let contents = bundle.join(CONTENTS_DIR);
    std::fs::create_dir_all(contents.join(arch_dir())).expect("the bundle directory");
    std::fs::write(contents.join(MANIFEST_FILE), manifest).expect("the listing");
    if let Some(source) = payload {
        let into = contents
            .join(arch_dir())
            .join(format!("{name}.{PAYLOAD_EXTENSION}"));
        std::fs::copy(source, &into).expect("the payload");
    }
    bundle
}

/// The match names one scan registered.
fn names(outcome: &discover::ScanOutcome) -> Vec<String> {
    outcome
        .registered
        .iter()
        .map(|found| found.match_name.clone())
        .collect()
}

#[test]
fn a_folder_of_bundles_becomes_exactly_the_effects_it_should() {
    let test = "a_folder_of_bundles_becomes_exactly_the_effects_it_should";
    let Some(source) = built_cdylib() else {
        eprintln!(
            "{test}: skipped - {} was not found in the target directory. \
             Build it first: cargo build -p lumit-lfx-testplug",
            cdylib_name()
        );
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory");
    let listing = a_manifest();

    // The good one, in a vendor's own suite folder - the shape every installer
    // uses and the one a scan that read only the top of the folder would miss.
    let suite = root.path().join("Example vendor");
    std::fs::create_dir_all(&suite).expect("the suite folder");
    let good = a_bundle_in(&suite, "Good", &listing, Some(&source));

    // Somebody else's broken installer: a listing of its own - a bundle's
    // listing names that bundle's plugins, and two bundles claiming one id
    // would be a different fault from the one this case is about - and a
    // payload that is not a library at all.
    let elsewhere = a_listing_for("com.example.broken");
    let broken = a_bundle_in(root.path(), "Broken", &elsewhere, None);
    std::fs::write(
        broken
            .join(CONTENTS_DIR)
            .join(arch_dir())
            .join(format!("Broken.{PAYLOAD_EXTENSION}")),
        b"this is not a shared library",
    )
    .expect("the broken payload");

    // **A bundle whose only plugin is switched off** - the commonest shape,
    // and the one a person switches off precisely because it misbehaves. Its
    // payload is not a shared library, so opening it would say so; the
    // assertion below is that nothing says so, which is the only evidence
    // available from out here that the module was never opened at all.
    let off_id = "com.example.off";
    let off_bundle = a_bundle_in(root.path(), "Off", &a_listing_for(off_id), None);
    std::fs::write(
        off_bundle
            .join(CONTENTS_DIR)
            .join(arch_dir())
            .join(format!("Off.{PAYLOAD_EXTENSION}")),
        b"this is not a shared library either",
    )
    .expect("the switched-off bundle's payload");

    // And one that ships no build for this machine at all.
    let stranger = root.path().join(format!("Stranger{BUNDLE_SUFFIX}"));
    std::fs::create_dir_all(stranger.join(CONTENTS_DIR).join("vax-ultrix"))
        .expect("the stranger's directory");
    std::fs::write(
        stranger.join(CONTENTS_DIR).join(MANIFEST_FILE),
        a_listing_for("com.example.stranger"),
    )
    .expect("the stranger's listing");

    // **The variable path is honoured**, and appended rather than replacing:
    // the fixture folder is reachable through `LFX_PLUGIN_PATH` alone (§5.2).
    std::env::set_var(PLUGIN_PATH_ENV, root.path());
    let standard = lumit_lfx::bundle::search_paths();
    std::env::remove_var(PLUGIN_PATH_ENV);
    assert!(
        standard.contains(&root.path().to_path_buf()),
        "{PLUGIN_PATH_ENV} is one of the search paths: {standard:?}"
    );

    // The scan itself is pointed at the fixture alone. `standard()` would also
    // sweep the machine's real folders, and a test whose expected list depends
    // on what the developer happens to have installed is no test.
    let switched_off = id_of(Personality::Identity);
    let mut disabled = BTreeSet::new();
    disabled.insert(switched_off.clone());
    disabled.insert(off_id.to_owned());
    // The preference is read into the running list before the scan, which is
    // the one table §5.4's three places share; `ScanOptions` carries no second
    // copy of it to disagree with.
    discover::set_disabled(&disabled);
    let options = ScanOptions {
        paths: vec![root.path().to_path_buf()],
        exe: Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker"))),
        env: Vec::new(),
    };

    let ledger = Ledger::new();
    let first = discover::scan(&options, &ledger);

    // Exactly the personalities that can be catalogued, minus the switched-off
    // one, each under the `lfx:` name the namespace seam mints.
    let catalogued: Vec<String> = PERSONALITIES
        .iter()
        .copied()
        .filter(|personality| {
            !matches!(
                personality,
                Personality::BrokenDescribe
                    | Personality::DuplicateIds
                    | Personality::MissingExtension
                    | Personality::Identity
            )
        })
        .map(|personality| format!("lfx:{}", id_of(personality)))
        .collect();
    assert_eq!(
        names(&first),
        catalogued,
        "the switched-off one, the two that cannot describe and the one that \
         needs an extension are all absent"
    );

    // The plugin's own facts came with it, so the browser can place the row.
    let full = first
        .registered
        .iter()
        .find(|found| found.row.identifier == id_of(Personality::Full))
        .expect("the full personality registered");
    assert_eq!(
        full.row.label,
        Personality::Full.name().to_string_lossy(),
        "the label a person reads"
    );
    assert_eq!(
        full.row.vendor,
        lumit_lfx_testplug::VENDOR.to_string_lossy()
    );
    assert_eq!(full.row.bundle, good, "and which bundle it came out of");
    assert_eq!(full.row.version(), {
        let (major, minor, patch) = Personality::Full.version();
        format!("{major}.{minor}.{patch}")
    });
    assert_eq!(
        full.schema.match_name, full.match_name,
        "the schema answers to the name the catalogue files it under"
    );
    assert_eq!(
        full.categories.first().copied(),
        Some(full.schema.category),
        "the first declared family is the heading (§2.4)"
    );

    // **The answer to failure 6**: the switched-off plugin has no row in
    // `DISCOVERED` and no row in `REFUSED`, and the listing still gives the
    // page a label, a vendor and a version for it - without the module having
    // been opened to get them.
    let listed: Vec<&str> = first
        .listed
        .iter()
        .map(|row| row.identifier.as_str())
        .collect();
    for personality in PERSONALITIES {
        assert!(
            listed.contains(&id_of(personality).as_str()),
            "{} is missing from the listing",
            id_of(personality)
        );
    }
    let off = first
        .listed
        .iter()
        .find(|row| row.identifier == switched_off)
        .expect("a row for the switched-off plugin");
    assert_eq!(off.label, Personality::Identity.name().to_string_lossy());
    assert!(discover::plugin_of(&format!("lfx:{switched_off}")).is_none());
    assert!(discover::refusal_of(&switched_off).is_none());
    assert!(
        first.skipped.iter().any(
            |line| line.contains(&switched_off) && line.contains("switched off in preferences")
        ),
        "and the report says so: {:?}",
        first.skipped
    );

    // **Every refusal is typed and named**, which is what the `REFUSED` table
    // is made of. The extension one is §4.3's: negotiated from the listing,
    // before `create`, with the extension it wanted in the refusal.
    let needy = id_of(Personality::MissingExtension);
    let refusal = discover::refusal_of(&needy).expect("the extension refusal");
    match &refusal.why {
        LfxRejection::RequiresExtension { id, extension } => {
            assert_eq!(*id, needy);
            assert!(!extension.is_empty(), "the extension it wanted is named");
        }
        other => panic!("{needy} was refused for the wrong reason: {other}"),
    }
    assert_eq!(
        refusal.row.label,
        Personality::MissingExtension.name().to_string_lossy(),
        "a refused plugin still has a row the page can draw"
    );
    assert_eq!(
        first
            .refused
            .iter()
            .filter(|entry| entry.row.identifier == needy)
            .count(),
        1,
        "refused once, not once from the listing and once off the pipe"
    );
    for absent in [Personality::BrokenDescribe, Personality::DuplicateIds] {
        let why = discover::refusal_of(&id_of(absent))
            .unwrap_or_else(|| panic!("{} was dropped with no sentence", id_of(absent)));
        assert!(
            why.why.refuses_the_effect(),
            "{} was refused, not merely noted",
            id_of(absent)
        );
    }

    // **A bundle that will not load is a line, and never a dialogue** - and it
    // costs the good bundle beside it nothing.
    let report = first.skipped.join("\n");
    assert!(
        report.contains("Broken"),
        "the unloadable bundle said why: {report}"
    );
    assert!(
        report.contains("Stranger") && report.contains("architecture"),
        "and so did the one with no build for this machine: {report}"
    );
    // And it still has a **row**, which is the one thing a skip sentence cannot
    // be parsed into: the listing is read before the payload is looked for, so
    // §7.2's `missing` state has a label and a vendor to draw (§5.3).
    let stranger_row = first
        .listed
        .iter()
        .find(|row| row.identifier == "com.example.stranger")
        .expect("a row for the bundle with no build for this machine");
    assert_eq!(stranger_row.label, "Elsewhere");
    assert_eq!(stranger_row.vendor, "Example");
    assert_eq!(stranger_row.bundle, stranger);

    // **§5.4 place 1, for a bundle with nothing left to describe**: its module
    // is never opened. The payload is not a shared library, so a describe would
    // have filed "the module did not load" against it - and the one line the
    // bundle earned is the skip sentence naming the plugin instead.
    let off_lines: Vec<&String> = first
        .skipped
        .iter()
        .filter(|line| line.contains(&off_bundle.display().to_string()))
        .collect();
    assert_eq!(
        off_lines.len(),
        1,
        "a bundle whose only plugin is switched off says one thing: {off_lines:?}"
    );
    assert!(
        off_lines[0].contains(off_id) && off_lines[0].contains("switched off in preferences"),
        "and that thing is the skip line: {off_lines:?}"
    );
    assert!(
        !report.contains(&format!(
            "{}: the module did not load",
            off_bundle.display()
        )),
        "the switched-off bundle's module was opened: {report}"
    );
    // It still has its row, because the listing is read before anything is
    // decided about it (§5.3).
    assert!(
        first.listed.iter().any(|row| row.identifier == off_id),
        "the switched-off bundle is still in the listing"
    );

    // ------------------------------------------------------------ a rescan --

    let second = discover::scan(&options, &ledger);
    assert!(
        second.registered.is_empty(),
        "a rescan registers nothing a second time: {:?}",
        names(&second)
    );
    assert_eq!(
        discover::discovered().len(),
        catalogued.len(),
        "and the table is the one the first scan built"
    );
    let full_again = discover::plugin_of(&full.match_name).expect("still there");
    assert!(
        std::ptr::eq(full_again.schema, full.schema),
        "the same leaked schema, not a second copy of it"
    );

    // ------------------------------------------- and a plugin switched on --

    // One tick, on the running list - the `Off` bundle stays switched off, so
    // its module stays shut for this scan too.
    discover::set_enabled(&switched_off, true);
    let third = discover::scan(&options, &ledger);
    assert_eq!(
        names(&third),
        vec![format!("lfx:{switched_off}")],
        "the one that was switched off is the only thing new"
    );
}
