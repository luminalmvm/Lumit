//! Importing projects from other applications — After Effects first
//! (docs/11-AE-IMPORT.md, docs/impl/ae-import.md). This phase is the reader:
//! opening a Lumit Bridge bundle and parsing its capture into typed structs.
//!
//! In plain terms: getting a project out of After Effects takes two halves. The
//! first half runs *inside* After Effects as a script — it walks the project and
//! writes down everything it finds, in AE's own words, changing nothing. It is a
//! courier, not a translator. The second half is this crate: it reads what the
//! courier wrote and does all the actual translating, so AE's clock times become
//! Lumit's exact times, AE's effects become Lumit's effects, and anything that
//! cannot translate becomes a clearly-labelled placeholder rather than quietly
//! vanishing. The split is deliberate, and the reason is testing: the script side
//! needs a real copy of After Effects to run, so no test suite of ours can ever
//! check it, whereas everything in here is ordinary Rust that the tests watch
//! closely. So the untestable half is kept too simple to get wrong, and
//! all the thinking lives on the half that can be proved.
//!
//! A **bundle** is what the courier writes: a folder (or a zip of one) holding
//! `manifest.json` — which says what schema version the rest is written in —
//! `capture.json`, the walk itself, and `report.json`, the short list of
//! properties After Effects refused to hand over. [`open_bundle`] reads all
//! three. It is deliberately forgiving in one direction and strict in the other:
//! a bundle from a *newer* Lumit than this one is refused outright, because
//! guessing at a schema we have not seen is how a silently wrong import happens,
//! while a bundle whose `report.json` is damaged still opens, because the report
//! is commentary and the capture is the work.
//!
//! [`map::map_capture`] is the other half: it turns the parsed capture into a
//! whole new [`lumit_core::Document`] plus an [`report::ImportReport`] saying
//! what changed on the way across. Import always makes a *new* project —
//! merging a capture into one that is already open is later work.
//!
//! There is also a **second front door**: [`aep::open_aep`] reads an
//! After Effects project file directly and fills the same [`Capture`], so the
//! user picks the `.aep` and nothing has to be run inside After Effects at all.
//! It is a second front end, never a second importer — everything downstream is
//! shared, and [`Bundle::source`] is the only thing that remembers which way in
//! was taken (with [`Bundle::footage_only`] saying whether the whole project
//! came, or only its footage references because the structure could not be
//! read). The Bridge stays first-class: it is the fidelity backstop, and it
//! is what cannot drift when Adobe changes the file format.

pub mod aep;
pub mod capture;
pub mod map;
pub mod report;

pub use aep::open_aep;
pub use capture::{Capture, Manifest, Report};
pub use map::{map_capture, map_preset};
pub use report::{ImportReport, ItemPath, Outcome, Reason, ReportRow, Summary};

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The only `format` string a bundle may carry.
pub const FORMAT: &str = "lumit-ae-bundle";

/// The capture-schema major version this reader understands. A bundle with a
/// higher major is refused; lower and equal are read (docs/11 §2.3).
pub const SUPPORTED_MAJOR: u64 = 1;

/// Everything a bundle holds, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub manifest: Manifest,
    pub capture: Capture,
    /// Empty when the bundle carries no readable report — the capture is the
    /// work, and a damaged report never costs the user their import.
    pub report: Report,
    /// Which route produced this. The two are interchangeable downstream, but
    /// the import report says so, because their honest failure modes differ:
    /// the Bridge cannot read a `CUSTOM_VALUE` blob and the direct parser can,
    /// while a new After Effects may break the parser and never the Bridge.
    pub source: BundleSource,
    /// The direct parser could not read the project's structure and fell
    /// back to its footage references alone (docs/11 §7): the capture holds
    /// footage items and nothing else, and the report says so in one row
    /// ([`note_skipped_chunks`]). Never set on the Bridge route — the Bridge
    /// writes a whole capture or none.
    pub footage_only: bool,
}

/// Where a [`Bundle`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BundleSource {
    /// A Lumit Bridge bundle — a folder or a zip written by the walker script.
    #[default]
    Bridge,
    /// An After Effects project file, read directly.
    Aep,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("this After Effects project could not be read: {0}")]
    Aep(#[from] aep::AepError),
    #[error("not a Lumit bundle")]
    NotABundle,
    #[error(
        "this bundle was written by a newer Lumit (bundle schema {version}) — please update Lumit"
    )]
    TooNew { version: String },
}

/// Whether the file at `path` is an After Effects animation preset, by its
/// first bytes rather than its name.
pub fn is_ffx(path: &Path) -> bool {
    let mut head = [0u8; 12];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok_and(|()| aep::ffx::is_preset(&head))
}

/// Read an After Effects animation preset (`.ffx`) as the effects it holds,
/// ready to go on a layer, with a report of what changed on the way.
///
/// The effects come out exactly as [`map_capture`] would have made them
/// inside a project: Lumit's own effect where the table knows one, a set of
/// Custom controls for a pseudo effect, and expressions carried as written.
/// Anything in the preset that is not an effect is a skipped row.
pub fn open_ffx(
    path: &Path,
) -> Result<(Vec<lumit_core::model::EffectInstance>, ImportReport), ImportError> {
    let bytes = fs::read(path)?;
    let preset = aep::ffx::parse_preset(&bytes)?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (effects, mut report) = map_preset(&name, &preset.effects, preset.size);
    for skipped in &preset.skipped {
        report.row(
            ItemPath::item(&name).property(skipped.path.as_deref().unwrap_or_default()),
            Outcome::Skipped,
            Reason::PropertyUnreadable {
                match_name: skipped.match_name.clone().unwrap_or_default(),
            },
        );
    }
    Ok((effects, report))
}

/// Open whatever the user picked: an After Effects project file, a
/// `.lum-bundle` folder, or a zip of one.
///
/// The route is decided here rather than in the frontend, so both front doors
/// are one call. A folder is always a bundle; a file is an `.aep` when its
/// first four bytes say so. The extension is only ever a hint — the magic is
/// the truth, so a project someone renamed still opens and a zip named `.aep`
/// is still read as a zip.
pub fn open_ae(path: &Path) -> Result<Bundle, ImportError> {
    if !path.is_dir() && is_rifx(path) {
        open_aep(path)
    } else {
        open_bundle(path)
    }
}

/// Add a row for every chunk the direct parser had to skip (docs/11 §7: a
/// parse failure on one chunk skips that chunk and continues, and the report
/// lists what was skipped) — and the one row for the whole structure, when
/// the parser fell back to footage references alone.
///
/// Only the `.aep` route's skips are folded in. A property the *Bridge* could
/// not read is already an unreadable node in the capture, and [`map_capture`]
/// raises its row from there — adding these on top would say it twice. Those
/// rows are the ones carrying a match name; a skipped chunk carries a chunk id
/// and no match name.
///
/// A footage-only bundle ([`Bundle::footage_only`]) gets exactly one row,
/// against the project itself, saying that the structure could not be read
/// and how many references came instead. It is raised here rather than in
/// the mapping because the mapping sees a capture, and a capture of footage
/// items looks the same whether the project held nothing else or the parser
/// could not read the rest.
pub fn note_skipped_chunks(bundle: &Bundle, report: &mut ImportReport) {
    if bundle.source != BundleSource::Aep {
        return;
    }
    if bundle.footage_only {
        report.row(
            ItemPath::default(),
            Outcome::Skipped,
            Reason::StructureUnreadable {
                count: bundle.capture.items.len(),
            },
        );
    }
    for row in &bundle.report.unreadables {
        if row.match_name.is_some() {
            continue;
        }
        report.row(
            ItemPath {
                comp: row.comp.clone(),
                layer: row.layer.clone(),
                property: None,
            },
            Outcome::Skipped,
            Reason::ChunkUnreadable {
                chunk: row.path.clone().unwrap_or_default(),
            },
        );
    }
}

/// Whether the file begins with a RIFF/RIFX container's magic.
fn is_rifx(path: &Path) -> bool {
    let mut magic = [0_u8; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && (&magic == b"RIFX" || &magic == b"RIFF")
}

/// Open a Lumit Bridge bundle: a `.lum-bundle` folder, or a zip of one.
///
/// Reads `manifest.json` first and stops there if the bundle is from a newer
/// major schema. `capture.json` must parse; `report.json` need not.
pub fn open_bundle(path: &Path) -> Result<Bundle, ImportError> {
    let mut source = Source::open(path)?;

    let Some(bytes) = source.read("manifest.json")? else {
        return Err(ImportError::NotABundle);
    };
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.format.as_deref() != Some(FORMAT) {
        return Err(ImportError::NotABundle);
    }
    // An absent or unparsable version is read rather than refused: `format`
    // has already identified the bundle, and refusing on a missing field
    // would make the schema's own growth a breaking change.
    if let Some(version) = manifest.version.as_deref() {
        if major_of(version).is_some_and(|major| major > SUPPORTED_MAJOR) {
            return Err(ImportError::TooNew {
                version: version.to_string(),
            });
        }
    }

    let Some(bytes) = source.read("capture.json")? else {
        return Err(ImportError::NotABundle);
    };
    let capture: Capture = serde_json::from_slice(&bytes)?;

    let report = source
        .read("report.json")
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();

    Ok(Bundle {
        manifest,
        capture,
        report,
        source: BundleSource::Bridge,
        footage_only: false,
    })
}

fn major_of(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

/// A bundle's three files, wherever they physically live.
enum Source {
    Dir(PathBuf),
    // Boxed because a `ZipArchive` is far larger than a `PathBuf`.
    Zip(Box<zip::ZipArchive<File>>),
}

impl Source {
    fn open(path: &Path) -> Result<Self, ImportError> {
        if path.is_dir() {
            Ok(Source::Dir(path.to_path_buf()))
        } else {
            Ok(Source::Zip(Box::new(zip::ZipArchive::new(File::open(
                path,
            )?)?)))
        }
    }

    /// The named file's bytes, or `None` when the bundle has no such file.
    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, ImportError> {
        match self {
            Source::Dir(dir) => match fs::read(dir.join(name)) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
            Source::Zip(zip) => {
                // Zipping a bundle folder the ordinary way keeps the folder as
                // a prefix on every entry, so match the file name rather than
                // the whole path.
                let found = (0..zip.len()).find(|&i| {
                    zip.name_for_index(i)
                        .is_some_and(|entry| entry.rsplit('/').next() == Some(name))
                });
                let Some(index) = found else {
                    return Ok(None);
                };
                let mut entry = zip.by_index(index)?;
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                Ok(Some(bytes))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use std::io::Write;

    /// The hand-written bundle in `tests/fixtures/`, which doubles as readable
    /// documentation of the capture schema until `make-fixture.jsx` has been
    /// run once against a real After Effects.
    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("synthetic.lum-bundle")
    }

    fn opened() -> Bundle {
        open_bundle(&fixture()).expect("the synthetic bundle opens")
    }

    /// The fixture bundle, zipped to `to` the ordinary way — the folder stays a
    /// prefix on every entry, which is the shape a user's zip really has.
    fn zip_fixture(to: &Path) {
        let mut writer = zip::ZipWriter::new(File::create(to).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for entry in fs::read_dir(fixture()).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            writer
                .start_file(format!("synthetic.lum-bundle/{name}"), options)
                .unwrap();
            writer.write_all(&fs::read(entry.path()).unwrap()).unwrap();
        }
        writer.finish().unwrap();
    }

    /// **A newer major schema is refused, and the message says to update
    /// Lumit.**
    ///
    /// docs/11 §2.3's policy, and the one place the reader is deliberately
    /// strict: a capture written to a schema this build has never seen would be
    /// read with the wrong assumptions, and a wrong import is worse than a
    /// refused one. The check is on the *major* alone — an unreadable minor
    /// bump must still open, which the next test's extra key covers.
    /// The first composition of the fixture that has a layer, with `effects`
    /// on its top layer, and the context an expression there runs under.
    fn on_a_layer(
        effects: Vec<lumit_core::model::EffectInstance>,
        time: f64,
    ) -> std::sync::Arc<lumit_core::expression::ExpressionContext> {
        use lumit_core::model::ProjectItem;

        let (mut doc, _) = map_capture(&opened().capture);
        let comp = doc
            .items
            .iter()
            .find_map(|item| match item {
                ProjectItem::Composition(c) if !c.layers.is_empty() => Some(c.id),
                _ => None,
            })
            .expect("a composition with a layer");
        let layer = {
            let layer = &mut doc.comp_mut(comp).unwrap().layers[0];
            layer.effects = effects;
            layer.id
        };
        std::sync::Arc::new(lumit_core::expression::ExpressionContext {
            document: std::sync::Arc::new(doc),
            comp: Some(comp),
            layer: Some(layer),
            comp_time: time,
            current_depth: 0,
            inputs: None,
        })
    }

    /// **A preset's expressions find its controls once both are on a layer.**
    ///
    /// This is what bringing a rig across means: the pseudo effect arrives as
    /// a set of controls under the name the expressions ask for, each control
    /// under its own, and the expression beside it runs as it was written.
    #[test]
    fn a_presets_expressions_read_its_controls() {
        use crate::aep::ffx::tests::{preset_bytes, Slider};

        let bytes = preset_bytes(&[
            (
                "Pseudo/1",
                "Shake",
                &[Slider {
                    label: "Amount",
                    value: 12.5,
                    expression: None,
                }],
            ),
            (
                "ADBE Slider Control",
                "Driven",
                &[Slider {
                    label: "Slider",
                    value: 3.0,
                    expression: Some("value + effect(\"Shake\")(\"Amount\") * 2"),
                }],
            ),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Shake.ffx");
        fs::write(&path, &bytes).unwrap();
        let (effects, report) = open_ffx(&path).unwrap();

        let controls = &effects[0];
        assert_eq!(controls.effect.match_name, "custom_controls");
        assert_eq!(controls.custom_name.as_deref(), Some("Shake"));
        let rows = lumit_core::fx::def("custom_controls")
            .unwrap()
            .derived(controls);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Amount");
        assert_eq!(controls.float_at(rows[0].id, 0.0), Some(12.5));

        assert_eq!(effects[1].effect.match_name, "slider_control");
        assert!(report
            .rows
            .iter()
            .any(|row| row.reason == Reason::ExpressionCarried));

        // 3 under the expression, plus twice the control's 12.5.
        let context = on_a_layer(effects, 0.0);
        let layer = &context.document.comp(context.comp.unwrap()).unwrap().layers[0];
        assert_eq!(
            layer.effects[1].float_at_with_context("slider", 0.0, context.clone()),
            Some(28.0)
        );
    }

    #[test]
    fn a_newer_major_schema_is_refused_with_a_please_update_message() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("future.lum-bundle");
        fs::create_dir(&bundle).unwrap();
        fs::write(
            bundle.join("manifest.json"),
            br#"{ "format": "lumit-ae-bundle", "version": "2.0.0" }"#,
        )
        .unwrap();
        fs::write(
            bundle.join("capture.json"),
            br#"{ "items": [], "comps": [] }"#,
        )
        .unwrap();

        match open_bundle(&bundle) {
            Err(ImportError::TooNew { version }) => {
                assert_eq!(version, "2.0.0");
                let said = ImportError::TooNew { version }.to_string();
                assert!(said.contains("please update Lumit"), "said: {said}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        // The same bundle one minor version ahead opens, because the schema
        // grows by addition.
        fs::write(
            bundle.join("manifest.json"),
            br#"{ "format": "lumit-ae-bundle", "version": "1.7.0" }"#,
        )
        .unwrap();
        assert!(open_bundle(&bundle).is_ok());
    }

    /// **A field this reader has never heard of is ignored, not refused.**
    ///
    /// The schema grows by addition (docs/10 §1.1's rule), so a bundle from a
    /// later Bridge — carrying, say, a footage item's colour profile, or a
    /// per-key property nobody has designed yet — must open in this build with
    /// the unknown parts dropped and everything else intact. The failure this
    /// prevents is `deny_unknown_fields` creeping onto one struct and turning
    /// every future minor schema bump into a hard error.
    #[test]
    fn an_unknown_key_is_ignored_rather_than_refused() {
        let json = br#"{
          "walked_in_reverse": true,
          "items": [ { "id": 1, "kind": "footage", "colour_profile": "sRGB IEC61966-2.1" } ],
          "comps": [ {
            "id": 2, "fps": 24, "guide_grid": "thirds",
            "layers": [ {
              "index": 1, "essential_property": "Master Blur",
              "properties": [ {
                "match_name": "ADBE Opacity", "dimensions": 1,
                "keyframes": [ { "t": 0, "v": 100, "spring_tension": 0.5 } ]
              } ]
            } ]
          } ]
        }"#;

        let capture: Capture = serde_json::from_slice(json).expect("unknown keys parse");
        assert_eq!(capture.items[0].path, None);
        assert_eq!(capture.comps[0].fps, Some(24.0));
        let key = &capture.comps[0].layers[0].properties[0]
            .keyframes
            .as_deref()
            .expect("a keyframe survived")[0];
        assert_eq!(key.t, Some(0.0));
    }

    /// **The same bundle zipped opens identically.**
    ///
    /// v1 reads both shapes because the walker writes a folder (ExtendScript
    /// has no zip) and users mail zips. Zipping a folder the ordinary way keeps
    /// the folder as a prefix on every entry, which is why the reader matches
    /// on the file name rather than the path — build the zip that way here, so
    /// the test would fail if that ever became a whole-path match.
    #[test]
    fn the_same_bundle_zipped_opens_identically() {
        let temp = tempfile::tempdir().unwrap();
        let zipped = temp.path().join("synthetic.lum-bundle.zip");
        zip_fixture(&zipped);

        assert_eq!(open_bundle(&zipped).unwrap(), opened());
    }

    /// **A damaged report still opens the bundle; a damaged capture does not.**
    ///
    /// The asymmetry is the point. `report.json` is commentary the Bridge
    /// already knew — losing it costs a few rows in a panel — so a truncated
    /// one must never stand between the user and their project. `capture.json`
    /// *is* the project, and half of one imported as though it were whole is
    /// exactly the silent data loss the importer exists to avoid.
    #[test]
    fn a_damaged_report_is_survivable_and_a_damaged_capture_is_not() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("damaged.lum-bundle");
        fs::create_dir(&bundle).unwrap();
        for entry in fs::read_dir(fixture()).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), bundle.join(entry.file_name())).unwrap();
        }

        fs::write(bundle.join("report.json"), b"{ \"unreadables\": [ {").unwrap();
        let opened = open_bundle(&bundle).unwrap();
        assert!(opened.report.unreadables.is_empty());
        assert_eq!(opened.capture.comps.len(), 2, "the walk still arrived");

        fs::write(bundle.join("capture.json"), b"{ \"comps\": [ {").unwrap();
        assert!(matches!(open_bundle(&bundle), Err(ImportError::Json(_)),));
    }

    /// **The one front door reads the bytes, not the name.**
    ///
    /// The picker offers `.aep` and `.zip` in one filter, so the extension is
    /// worth nothing: a project someone renamed must still open, and a bundle
    /// zip must not be handed to the RIFX parser. The magic decides, and a
    /// folder is a bundle without any reading at all.
    #[test]
    fn the_front_door_routes_by_the_bytes_rather_than_the_name() {
        assert_eq!(
            open_ae(&fixture()).expect("a folder is a bundle").source,
            BundleSource::Bridge
        );

        // A RIFX file named `.zip`: the bytes win, and it reaches the parser
        // (which then refuses it for having no item tree, not for its name).
        let temp = tempfile::tempdir().unwrap();
        let renamed = temp.path().join("project.zip");
        fs::write(&renamed, b"RIFX\0\0\0\x04Egg!").unwrap();
        assert!(matches!(open_ae(&renamed), Err(ImportError::Aep(_))));

        // And the mirror case: a real bundle zip named `.aep`. The bytes are a
        // zip, so it must open as a bundle rather than reach the RIFX parser —
        // the failure this guards is routing on the extension, which would
        // refuse a perfectly good bundle for its name.
        let misnamed = temp.path().join("bundle.aep");
        zip_fixture(&misnamed);
        let opened_zip = open_ae(&misnamed).expect("the bytes are a zip, so it is a bundle");
        assert_eq!(opened_zip.source, BundleSource::Bridge);
        assert_eq!(opened_zip, opened());

        // And a file that is neither goes down the bundle road, where the
        // answer is the plain refusal rather than a RIFX error.
        let neither = temp.path().join("notes.aep");
        fs::write(&neither, b"hello").unwrap();
        assert!(open_ae(&neither).is_err());
    }

    /// **A footage-only parse is one row against the project, and a whole
    /// parse is none.**
    ///
    /// docs/11 §7's fallback reaches the report here, on the same call the
    /// bridge already makes for skipped chunks — so the bridge picks it up
    /// without a change. The row is a skip (the comps and layers are what was
    /// lost), it names the project rather than any item, and it carries the
    /// one fact the user wants: how many references came instead.
    #[test]
    fn a_footage_only_bundle_says_so_once_against_the_project() {
        let mut bundle = opened();
        bundle.source = BundleSource::Aep;
        bundle.footage_only = true;
        let mut report = ImportReport::default();
        note_skipped_chunks(&bundle, &mut report);

        let rows: Vec<&ReportRow> = report
            .rows
            .iter()
            .filter(|row| matches!(row.reason, Reason::StructureUnreadable { .. }))
            .collect();
        assert_eq!(rows.len(), 1, "said once");
        assert_eq!(rows[0].outcome, Outcome::Skipped);
        assert_eq!(rows[0].path, ItemPath::default());
        assert_eq!(rows[0].path.to_string(), "Project");
        assert_eq!(
            rows[0].reason,
            Reason::StructureUnreadable {
                count: bundle.capture.items.len()
            }
        );
        assert_eq!(rows[0].reason.key(), "structure_unreadable");
        assert_eq!(
            rows[0].reason.args()["count"],
            bundle.capture.items.len().to_string()
        );

        // A whole parse says nothing of the kind.
        bundle.footage_only = false;
        let mut report = ImportReport::default();
        note_skipped_chunks(&bundle, &mut report);
        assert!(!report
            .rows
            .iter()
            .any(|row| matches!(row.reason, Reason::StructureUnreadable { .. })));
    }
}
