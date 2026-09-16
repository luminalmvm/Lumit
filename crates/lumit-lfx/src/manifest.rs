//! The bundle's own listing, read before any of its code runs
//! (docs/impl/lfx.md §3.3, §4.3).
//!
//! # In plain terms
//!
//! Every LFX bundle carries a small text file, `Contents/lfx.toml`, saying what
//! is in it: the plugin ids, the names a person reads, the vendors, the
//! versions, the picture families, the ABI version it was built against, and
//! the extensions it cannot run without. Reading it costs nothing and runs none
//! of the bundle's code, which is the whole point - it is what lets the Addons
//! page name, label and re-enable a plugin that has never started, and it is
//! how a plugin switched off *before* a scan gets a row of its own rather than
//! a sentence in a report.
//!
//! **Two rules govern this module and neither is negotiable.**
//!
//! It is parsed **in the broker**. A manifest is a stranger's structured text,
//! and parsing a stranger's structured text in the process that holds the
//! project, the media handles and the windows is the one thing the broker
//! architecture exists to prevent (§11 item 7). Nothing in the host's own
//! supervisor calls [`read`]; `the_host_never_parses_the_bundles_own_toml` is
//! where that is held. `lumit-ingress` bounds the damage with
//! [`Limits::PLUGIN_MANIFEST`]; it does not move it.
//!
//! And **the manifest is the cheap listing, never the authority** (§11 item 6).
//! Once the module really is open, everything here is compared against what the
//! code itself answered, and a disagreement is
//! [`LfxRejection::ManifestMismatch`] - the code wins, and the plugin is
//! refused rather than catalogued under whichever half a reader happened to
//! ask. [`agrees`] is that comparison, written once so that the broker and the
//! validator cannot come to two opinions about what counts as a disagreement.
//!
//! # The file
//!
//! ```toml
//! # Contents/lfx.toml
//! abi_version = 1
//!
//! [[plugin]]
//! id = "com.example.blur"
//! name = "Example blur"
//! vendor = "Example"
//! version = "2.3.4"
//! categories = ["blur-sharpen", "stylise"]
//! required_extensions = []
//! ```
//!
//! `abi_version` may sit at the top of the file or on a plugin of its own; a
//! plugin's own value wins where both are given, because the entry struct's is
//! per bundle and the per-plugin spelling is what a bundle holding two
//! generations would need. The families are **named**, from the closed
//! vocabulary of eight the frozen header declares, because an author writing
//! `categories = [2, 5]` is an author who will get it wrong; a name outside the
//! eight lowers to `LFX_CATEGORY_UNSET`, which then disagrees with whatever the
//! code declared and is caught by [`agrees`] rather than by a refusal of its
//! own.

use std::path::{Path, PathBuf};

use lumit_ingress::{IngressError, Limits};
use lumit_lfx_abi::{
    LfxCategory, LFX_CATEGORY_BLUR_SHARPEN, LFX_CATEGORY_COLOUR, LFX_CATEGORY_DISTORTION,
    LFX_CATEGORY_GENERATE, LFX_CATEGORY_STYLISE, LFX_CATEGORY_TEMPORAL, LFX_CATEGORY_TRANSITION,
    LFX_CATEGORY_UNSET, LFX_CATEGORY_UTILITY,
};
use serde::Deserialize;
use thiserror::Error;

use crate::ipc::proto::PluginIdentity;
use crate::LfxRejection;

/// The directory inside a bundle that everything lives in.
pub const CONTENTS_DIR: &str = "Contents";

/// The manifest's file name.
pub const MANIFEST_FILE: &str = "lfx.toml";

/// The eight picture families, by the name a manifest spells them with and the
/// `lfx_category` each one is.
///
/// A closed vocabulary, and the same eight [`crate::schema`] lowers onto
/// `FxCategory` - `every_family_the_schema_knows_has_a_manifest_spelling` is
/// what holds the two lists to the same length rather than to one another's
/// memory. `Audio`, `Drivers`, `Controls` and `Compositing` are not here for
/// the reason they are not in the header: they are not families a picture
/// effect with one input may claim.
pub const FAMILY_NAMES: [(&str, LfxCategory); 8] = [
    ("blur-sharpen", LFX_CATEGORY_BLUR_SHARPEN),
    ("colour", LFX_CATEGORY_COLOUR),
    ("distortion", LFX_CATEGORY_DISTORTION),
    ("generate", LFX_CATEGORY_GENERATE),
    ("stylise", LFX_CATEGORY_STYLISE),
    ("temporal", LFX_CATEGORY_TEMPORAL),
    ("transition", LFX_CATEGORY_TRANSITION),
    ("utility", LFX_CATEGORY_UTILITY),
];

/// What [`agrees`] compares, in the order it compares them.
///
/// A closed list of `&'static str` rather than free sentences, so
/// [`LfxRejection::ManifestMismatch`]'s `field` is a word the Addons page, the
/// scan report and `lfx-validator` all read the same way - the shape
/// `Ceiling::counts` already has one seam over.
pub const COMPARED_FIELDS: [&str; 7] = [
    "id",
    "name",
    "vendor",
    "version",
    "categories",
    "ABI version",
    "required extensions",
];

/// Why a manifest could not be read.
///
/// Typed, and every one of them a calm line rather than a dialogue: a bundle
/// whose listing cannot be read is a bundle that is skipped, and the page says
/// which one and why (docs/14 §3).
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ManifestError {
    /// The file is not there, is too long, or would not read.
    #[error("the bundle's manifest could not be read: {0}")]
    Unreadable(String),
    /// The text is not the TOML this version admits.
    #[error("the bundle's manifest is not readable as a listing: {0}")]
    Malformed(String),
    /// It parsed and declares nothing, which is a bundle with no plugins in it
    /// rather than a bundle to open a module for.
    #[error("the bundle's manifest declares no plugins")]
    Empty,
    /// A count or a string past one of the frozen header's own ceilings, met
    /// here rather than after the whole listing has been kept.
    #[error(transparent)]
    PastCeiling(#[from] LfxRejection),
}

impl From<IngressError> for ManifestError {
    fn from(error: IngressError) -> Self {
        ManifestError::Unreadable(error.to_string())
    }
}

/// Where a bundle's manifest is.
#[must_use]
pub fn path_in(bundle: &Path) -> PathBuf {
    bundle.join(CONTENTS_DIR).join(MANIFEST_FILE)
}

/// Read one bundle's listing.
///
/// **The broker's, and only the broker's.** See the module header.
///
/// The file is read under [`Limits::PLUGIN_MANIFEST`], which refuses a file
/// longer than the ceiling without reading it all - the two-sided check
/// `lumit-ingress` makes, so a file that lies about its own length is refused
/// rather than believed.
///
/// # Errors
///
/// [`ManifestError`] - the file is missing, too long, not text, not the TOML
/// this version admits, empty of plugins, or past one of the header's declared
/// ceilings.
pub fn read(bundle: &Path) -> Result<Vec<PluginIdentity>, ManifestError> {
    let text =
        lumit_ingress::read_to_string_capped(&path_in(bundle), Limits::PLUGIN_MANIFEST.bytes)?;
    parse(&text)
}

/// Turn a manifest's text into the identities it declares.
///
/// Separate from [`read`] so that the parsing - which is the part a stranger
/// controls - can be driven from a string by a plain `cargo test` with no
/// bundle on disk anywhere.
///
/// # Errors
///
/// [`ManifestError`], as [`read`].
pub fn parse(text: &str) -> Result<Vec<PluginIdentity>, ManifestError> {
    let file: ManifestFile =
        toml::from_str(text).map_err(|error| ManifestError::Malformed(error.to_string()))?;
    if file.plugin.is_empty() {
        return Err(ManifestError::Empty);
    }
    // The header's own number, asked before the list is kept rather than after:
    // a manifest declaring more plugins than a bundle may hold is refused
    // outright, which is the answer `BrokerMessage::checked` gives the same
    // number off the wire and the answer `LocalHost::open` gives it at the
    // descriptor list. Three readers, one sentence.
    let mut entries = Vec::new();
    for declared in file.plugin {
        entries.push(declared.into_identity(file.abi_version)?);
    }
    let checked = crate::ipc::proto::BrokerMessage::Manifested { entries }.checked()?;
    match checked {
        crate::ipc::proto::BrokerMessage::Manifested { entries } => Ok(entries),
        // `checked` answers a `Manifested` for a `Manifested`; the arm exists
        // because the enum's exhaustiveness is what makes that readable rather
        // than assumed.
        other => Err(ManifestError::Malformed(format!(
            "the listing came back as {}",
            other.name()
        ))),
    }
}

/// Whether the manifest and the code say the same thing about one plugin.
///
/// Every field the manifest declares has a counterpart in the code, and this is
/// where the two meet - the re-check docs/impl/lfx.md §4.3 asks for, run at the
/// first `Describe`, once the module really is open. The required-extension
/// list is the one that matters most and the reason the frozen
/// `lfx_descriptor` carries it at all: negotiation runs from the manifest,
/// because keeping the module shut is what §3.3 buys, so without a counterpart
/// in the code the single field that decides whether a plugin is instantiated
/// would be the one field nobody could check.
///
/// Order is [`COMPARED_FIELDS`]' own, and the first disagreement is the one
/// reported: a plugin whose id is wrong has nothing further worth comparing.
///
/// # Errors
///
/// [`LfxRejection::ManifestMismatch`], naming the field and both answers.
pub fn agrees(manifest: &PluginIdentity, code: &PluginIdentity) -> Result<(), LfxRejection> {
    let id = manifest.id.clone();
    let mismatch =
        |field: &'static str, declared: String, answered: String| LfxRejection::ManifestMismatch {
            id: id.clone(),
            field,
            manifest: declared,
            code: answered,
        };
    if manifest.id != code.id {
        return Err(mismatch("id", manifest.id.clone(), code.id.clone()));
    }
    if manifest.name != code.name {
        return Err(mismatch("name", manifest.name.clone(), code.name.clone()));
    }
    if manifest.vendor != code.vendor {
        return Err(mismatch(
            "vendor",
            manifest.vendor.clone(),
            code.vendor.clone(),
        ));
    }
    let release = |identity: &PluginIdentity| {
        format!("{}.{}.{}", identity.major, identity.minor, identity.patch)
    };
    if (manifest.major, manifest.minor, manifest.patch) != (code.major, code.minor, code.patch) {
        return Err(mismatch("version", release(manifest), release(code)));
    }
    if manifest.categories != code.categories {
        return Err(mismatch(
            "categories",
            numbers(&manifest.categories),
            numbers(&code.categories),
        ));
    }
    if manifest.abi_version != code.abi_version {
        return Err(mismatch(
            "ABI version",
            manifest.abi_version.to_string(),
            code.abi_version.to_string(),
        ));
    }
    // Compared as **sets**, not as lists: the order a descriptor's array
    // happens to be in is not a promise the manifest has to repeat, and what
    // the negotiation decided on is membership. A list longer than the manifest
    // declared is what §4.3 is about, and it fails this comparison the moment
    // it holds one id the manifest did not.
    if as_set(&manifest.required_extensions) != as_set(&code.required_extensions) {
        return Err(mismatch(
            "required extensions",
            manifest.required_extensions.join(", "),
            code.required_extensions.join(", "),
        ));
    }
    Ok(())
}

/// A list of category numbers as one sentence.
fn numbers(categories: &[u32]) -> String {
    categories
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The same strings with their order and their repeats taken out.
fn as_set(list: &[String]) -> std::collections::BTreeSet<&str> {
    list.iter().map(String::as_str).collect()
}

/// The whole file.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    /// The ABI every plugin in the bundle was built against, unless one says
    /// otherwise.
    #[serde(default)]
    abi_version: u32,
    /// One table per plugin.
    #[serde(default)]
    plugin: Vec<ManifestPlugin>,
}

/// One plugin's entry.
///
/// `deny_unknown_fields` on purpose: a key nobody recognises is a manifest
/// written against a newer Lumit or a manifest with a typo in it, and both are
/// better said out loud at the one moment the bundle's code has not run than
/// silently dropped into a listing somebody then trusts.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestPlugin {
    /// Reverse-DNS, and stable for the plugin's life.
    id: String,
    /// What a person reads.
    #[serde(default)]
    name: String,
    /// Who wrote it.
    #[serde(default)]
    vendor: String,
    /// `major.minor.patch`, as text, because that is how a vendor writes a
    /// release and three integer keys is three chances to write two of them.
    #[serde(default)]
    version: String,
    /// The picture families claimed, the heading first.
    #[serde(default)]
    categories: Vec<String>,
    /// This plugin's own ABI version, where it differs from the file's.
    #[serde(default)]
    abi_version: Option<u32>,
    /// The extensions it cannot run without.
    #[serde(default)]
    required_extensions: Vec<String>,
}

impl ManifestPlugin {
    /// The identity this entry declares.
    fn into_identity(self, file_abi: u32) -> Result<PluginIdentity, ManifestError> {
        let (major, minor, patch) = release_of(&self.version);
        Ok(PluginIdentity {
            id: self.id,
            name: self.name,
            vendor: self.vendor,
            major,
            minor,
            patch,
            categories: self.categories.iter().map(|name| family_of(name)).collect(),
            abi_version: self.abi_version.unwrap_or(file_abi),
            required_extensions: self.required_extensions,
        }
        .checked()?)
    }
}

/// The three numbers in a `major.minor.patch` string.
///
/// A component that is not a number, or is not there, reads as nought - which
/// then disagrees with whatever the code answered and is caught by [`agrees`].
/// Guessing here would be the manifest deciding something, and the manifest
/// decides nothing.
fn release_of(version: &str) -> (u32, u32, u32) {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// The `lfx_category` a manifest's family name is, or `LFX_CATEGORY_UNSET` for
/// a name outside the closed vocabulary.
fn family_of(name: &str) -> LfxCategory {
    FAMILY_NAMES
        .iter()
        .find(|(spelling, _)| *spelling == name)
        .map_or(LFX_CATEGORY_UNSET, |(_, category)| *category)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The listing a well-formed bundle carries.
    const GOOD: &str = r#"
        abi_version = 1

        [[plugin]]
        id = "com.example.blur"
        name = "Example blur"
        vendor = "Example"
        version = "2.3.4"
        categories = ["blur-sharpen", "stylise"]
        required_extensions = []
    "#;

    /// The identity the manifest above declares, as the code would answer it.
    fn blur() -> PluginIdentity {
        PluginIdentity {
            id: "com.example.blur".into(),
            name: "Example blur".into(),
            vendor: "Example".into(),
            major: 2,
            minor: 3,
            patch: 4,
            categories: vec![LFX_CATEGORY_BLUR_SHARPEN, LFX_CATEGORY_STYLISE],
            abi_version: 1,
            required_extensions: Vec::new(),
        }
    }

    /// The listing is what the page names a plugin from before any of the
    /// bundle's code has run, so every field it carries has to arrive.
    #[test]
    fn a_manifest_declares_the_plugins_a_page_can_name() {
        let entries = parse(GOOD).expect("a listing");
        assert_eq!(entries, vec![blur()]);
    }

    /// The manifest is the cheap listing and the code is the authority, so
    /// every field the two both carry is compared and a disagreement names
    /// itself (docs/impl/lfx.md §3.3, §4.3).
    #[test]
    fn a_manifest_that_disagrees_with_the_code_names_the_field() {
        let manifest = blur();
        assert_eq!(agrees(&manifest, &blur()), Ok(()));

        let cases: Vec<(&str, PluginIdentity)> = vec![
            (
                "id",
                PluginIdentity {
                    id: "com.example.other".into(),
                    ..blur()
                },
            ),
            (
                "name",
                PluginIdentity {
                    name: "Something else".into(),
                    ..blur()
                },
            ),
            (
                "vendor",
                PluginIdentity {
                    vendor: "Somebody else".into(),
                    ..blur()
                },
            ),
            ("version", PluginIdentity { patch: 5, ..blur() }),
            (
                "categories",
                PluginIdentity {
                    categories: vec![LFX_CATEGORY_UTILITY],
                    ..blur()
                },
            ),
            (
                "ABI version",
                PluginIdentity {
                    abi_version: 2,
                    ..blur()
                },
            ),
            (
                "required extensions",
                PluginIdentity {
                    required_extensions: vec!["lfx.temporal".into()],
                    ..blur()
                },
            ),
        ];
        for (field, code) in cases {
            match agrees(&manifest, &code) {
                Err(LfxRejection::ManifestMismatch { field: named, .. }) => {
                    assert_eq!(named, field, "the wrong field was named");
                }
                other => panic!("{field} was not caught: {other:?}"),
            }
        }
        assert_eq!(
            COMPARED_FIELDS.len(),
            7,
            "a field added to the comparison needs a case above"
        );
    }

    /// The case §4.3 exists for: a bundle declaring `required = []` and in fact
    /// asking for an extension. It passes negotiation, reaches `create`, gets a
    /// null and fails somewhere later - unless the first describe compares the
    /// two lists, which is what this holds.
    #[test]
    fn a_required_extension_the_manifest_did_not_declare_is_a_mismatch() {
        let manifest = blur();
        let code = PluginIdentity {
            required_extensions: vec!["lfx.temporal".into()],
            ..blur()
        };
        let Err(LfxRejection::ManifestMismatch {
            field, code: said, ..
        }) = agrees(&manifest, &code)
        else {
            panic!("a longer list than the manifest declared must be refused");
        };
        assert_eq!(field, "required extensions");
        assert!(said.contains("lfx.temporal"), "{said}");
    }

    /// The same two lists in a different order are the same negotiation. What
    /// the extension list decides is membership, and a descriptor's array order
    /// is not a promise the manifest repeats.
    #[test]
    fn the_extension_lists_are_compared_as_sets_rather_than_in_order() {
        let manifest = PluginIdentity {
            required_extensions: vec!["lfx.temporal".into(), "lfx.overlay".into()],
            ..blur()
        };
        let code = PluginIdentity {
            required_extensions: vec!["lfx.overlay".into(), "lfx.temporal".into()],
            ..blur()
        };
        assert_eq!(agrees(&manifest, &code), Ok(()));
    }

    /// A family name nobody recognises lowers to the unset category rather than
    /// to a plausible one, so it disagrees with whatever the code declared and
    /// is caught at the comparison. The manifest decides nothing.
    #[test]
    fn a_family_name_outside_the_vocabulary_is_unset_rather_than_guessed() {
        let entries = parse(
            r#"
            [[plugin]]
            id = "com.example.blur"
            categories = ["compositing"]
        "#,
        )
        .expect("a listing");
        assert_eq!(
            entries.first().map(|entry| entry.categories.clone()),
            Some(vec![LFX_CATEGORY_UNSET])
        );
    }

    /// Every family the schema lowers onto `FxCategory` has a spelling here,
    /// and no spelling here is a number the schema has never heard of. Two
    /// lists that must agree and are written twice is how the next family gets
    /// half-added.
    #[test]
    fn every_family_the_schema_knows_has_a_manifest_spelling() {
        assert_eq!(FAMILY_NAMES.len(), 8, "the vocabulary is eight families");
        let mut seen = std::collections::BTreeSet::new();
        for (name, category) in FAMILY_NAMES {
            assert!(seen.insert(category), "{name} shares a number");
            assert_ne!(category, LFX_CATEGORY_UNSET, "{name} is nobody's family");
            assert_eq!(family_of(name), category);
        }
        // The eight the header declares, one for one: every number from one to
        // eight is spelled exactly once.
        assert_eq!(
            seen,
            (1..=8).collect::<std::collections::BTreeSet<LfxCategory>>()
        );
    }

    /// A manifest that parses and lists nothing is a bundle with no plugins,
    /// not a bundle whose module is worth opening.
    #[test]
    fn a_manifest_with_no_plugins_is_refused_rather_than_opened() {
        assert_eq!(parse("abi_version = 1"), Err(ManifestError::Empty));
    }

    /// A key nobody recognises is said out loud rather than dropped: it is a
    /// manifest written against a newer Lumit, or one with a typo in it, and
    /// both are better caught before the module is opened.
    #[test]
    fn a_manifest_key_this_version_does_not_know_is_refused() {
        let outcome = parse(
            r#"
            [[plugin]]
            id = "com.example.blur"
            sandbox = "none"
        "#,
        );
        assert!(
            matches!(outcome, Err(ManifestError::Malformed(_))),
            "{outcome:?}"
        );
    }

    /// The header's own ceiling on how many plugins a bundle may hold, asked of
    /// the listing before the listing is kept - the same number and the same
    /// refusal the wire and the descriptor list already give.
    #[test]
    fn a_manifest_past_the_headers_effect_ceiling_is_refused_rather_than_truncated() {
        let mut text = String::from("abi_version = 1\n");
        for index in 0..=lumit_lfx_abi::LFX_MAX_EFFECTS_PER_BUNDLE {
            text.push_str(&format!("[[plugin]]\nid = \"com.example.p{index}\"\n"));
        }
        match parse(&text) {
            Err(ManifestError::PastCeiling(LfxRejection::PastCeiling {
                ceiling, given, ..
            })) => {
                assert_eq!(ceiling, crate::Ceiling::EffectsPerBundle);
                assert_eq!(
                    given,
                    u64::from(lumit_lfx_abi::LFX_MAX_EFFECTS_PER_BUNDLE) + 1
                );
            }
            other => panic!("a bundle past the ceiling must be refused: {other:?}"),
        }
    }

    /// A string past `LFX_MAX_STRING_BYTES` is met in the listing, where the
    /// page would otherwise print it, rather than after it has been kept.
    #[test]
    fn a_manifest_string_past_the_headers_ceiling_is_refused() {
        let long = "x".repeat(lumit_lfx_abi::LFX_MAX_STRING_BYTES as usize + 1);
        let outcome = parse(&format!("[[plugin]]\nid = \"{long}\"\n"));
        assert!(
            matches!(
                outcome,
                Err(ManifestError::PastCeiling(LfxRejection::PastCeiling {
                    ceiling: crate::Ceiling::StringBytes,
                    ..
                }))
            ),
            "{outcome:?}"
        );
    }

    /// A version component that is not a number reads as nought rather than as
    /// a guess, and the comparison then catches it.
    #[test]
    fn a_version_that_is_not_three_numbers_reads_as_nought() {
        assert_eq!(release_of("2.3.4"), (2, 3, 4));
        assert_eq!(release_of("2.3"), (2, 3, 0));
        assert_eq!(release_of(""), (0, 0, 0));
        assert_eq!(release_of("two.three.four"), (0, 0, 0));
    }

    /// The manifest is at one place inside the bundle, spelled once.
    #[test]
    fn the_manifest_is_where_the_bundle_layout_says_it_is() {
        let path = path_in(Path::new("Name.lfx.bundle"));
        assert!(
            path.ends_with(Path::new("Contents").join("lfx.toml")),
            "{path:?}"
        );
    }

    /// A bundle with no manifest at all is a calm line rather than a fault: the
    /// file is missing, the bundle is skipped, and the page says which.
    #[test]
    fn a_bundle_with_no_manifest_is_a_line_rather_than_a_fault() {
        let root = tempfile::tempdir().expect("a folder");
        let outcome = read(root.path());
        assert!(
            matches!(outcome, Err(ManifestError::Unreadable(_))),
            "{outcome:?}"
        );
    }

    /// The whole of the point: a listing read off the disk, with no module
    /// opened and none of the bundle's code run.
    #[test]
    fn a_manifest_is_read_off_the_disk_without_a_module() {
        let root = tempfile::tempdir().expect("a folder");
        let bundle = root.path().join("Example.lfx.bundle");
        std::fs::create_dir_all(bundle.join(CONTENTS_DIR)).expect("the contents folder");
        std::fs::write(path_in(&bundle), GOOD).expect("the manifest");
        assert_eq!(read(&bundle), Ok(vec![blur()]));
    }

    /// The host's own process never calls this module. Reading a stranger's
    /// structured text in the process that holds the project, the media handles
    /// and the windows is the one thing the broker architecture exists to
    /// prevent (§11 item 7), and the only defence against it is that nobody on
    /// that side of the pipe calls [`read`] or [`parse`].
    ///
    /// **The whole crate is swept, not a list of files.** The rule is about a
    /// process, and every module in this crate bar the broker binary's own runs
    /// in it - so a rescan helper, an Addons-page adapter or a module nobody
    /// has written yet is exactly what wants catching, and a hand-kept list of
    /// three file names would let each of them through green. Only
    /// `manifest.rs` is exempt, because it is what the two functions are in.
    ///
    /// *ponytail:* a textual check, because there is no type that says "this
    /// function runs in the other process". The day there is one, this becomes
    /// a compile failure instead.
    #[test]
    fn the_host_never_parses_the_bundles_own_toml() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut swept = 0usize;
        for file in rust_sources(&src) {
            if file.file_name() == Some(std::ffi::OsStr::new("manifest.rs")) {
                continue;
            }
            let source = std::fs::read_to_string(&file).expect("a source file of this crate");
            swept += 1;
            for forbidden in ["manifest::read", "manifest::parse"] {
                assert!(
                    !source.contains(forbidden),
                    "{} calls {forbidden}: the manifest is the broker's to parse",
                    file.display()
                );
            }
        }
        assert!(
            swept > 1,
            "the sweep found no source to read: {}",
            src.display()
        );
    }

    /// Every `.rs` file under one directory, at any depth - the crate as it is
    /// on disk rather than as a list somebody remembered to keep up.
    fn rust_sources(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(rust_sources(&path));
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                found.push(path);
            }
        }
        found.sort();
        found
    }
}
