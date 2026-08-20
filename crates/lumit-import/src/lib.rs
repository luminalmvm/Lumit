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
//! closely (K-410). So the untestable half is kept too simple to get wrong, and
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
//! Mapping a capture onto a Lumit document is the next phase and does not live
//! here yet.

pub mod capture;

pub use capture::{Capture, Manifest, Report};

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
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Lumit bundle")]
    NotABundle,
    #[error(
        "this bundle was written by a newer Lumit (bundle schema {version}) — please update Lumit"
    )]
    TooNew { version: String },
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
    use crate::capture::{Comp, Layer, Property};
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

    fn comp(capture: &Capture, name: &str) -> Comp {
        let id = capture
            .items
            .iter()
            .find(|item| item.name.as_deref() == Some(name))
            .and_then(|item| item.id)
            .expect("the comp has an item row");
        capture
            .comps
            .iter()
            .find(|comp| comp.id == Some(id))
            .cloned()
            .expect("the item row has a comp")
    }

    fn layer(comp: &Comp, name: &str) -> Layer {
        comp.layers
            .iter()
            .find(|layer| layer.name.as_deref() == Some(name))
            .cloned()
            .expect("the comp has that layer")
    }

    /// Depth-first search for a match name anywhere in a property tree,
    /// including through separated followers.
    fn prop(properties: &[Property], match_name: &str) -> Option<Property> {
        for property in properties {
            if property.match_name.as_deref() == Some(match_name) {
                return Some(property.clone());
            }
            if let Some(found) = prop(property.children(), match_name) {
                return Some(found);
            }
            if let Some(followers) = property.separated.as_deref() {
                if let Some(found) = prop(followers, match_name) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// **A bundle folder opens, and the manifest is what identifies it.**
    ///
    /// The shallow check, kept separate because everything below assumes it:
    /// the three files are found by name in a folder, the format string is the
    /// one that says "bundle", and the walk arrived with both comps and all
    /// five items rather than an empty shell that parsed.
    ///
    /// The project block rides along here because it has nowhere else to be
    /// checked and is easy to forget: bit depth and the blending flags belong
    /// to the *project*, so docs/11 §3's "this comp relied on 8-bpc non-linear
    /// blending" flag cannot be worked out from a comp at all.
    #[test]
    fn a_bundle_folder_opens_with_its_manifest_and_its_walk() {
        let bundle = opened();
        assert_eq!(bundle.manifest.format.as_deref(), Some(FORMAT));
        assert_eq!(bundle.manifest.version.as_deref(), Some("1.0.0"));
        assert_eq!(bundle.manifest.ae_version.as_deref(), Some("26.0x67"));
        assert_eq!(bundle.capture.items.len(), 5);
        assert_eq!(bundle.capture.comps.len(), 2);

        let project = bundle.capture.project.expect("the project block arrived");
        assert_eq!(project.bits_per_channel, Some(16));
        assert_eq!(project.linear_blending, Some(true));
    }

    /// **A comp's name comes from its item row, and nesting is a source id.**
    ///
    /// The one structural join in the schema, and the one an importer gets
    /// wrong first: `comps[]` carries no name at all, so a comp is only ever
    /// identified by an id shared with `items[]`, and a precomp layer names
    /// its comp through that same id rather than by name. Proving the hop
    /// works in both directions is what makes the rest of the mapping phase
    /// able to trust it.
    #[test]
    fn a_nested_comp_is_reached_through_its_precomp_layers_source_id() {
        let capture = opened().capture;
        let main = comp(&capture, "Main");
        let nested = comp(&capture, "Nested");

        assert_eq!(main.width, Some(1920));
        assert_eq!(main.fps, Some(25.0));
        assert_eq!(main.renderer.as_deref(), Some("ADBE Advanced 3d"));
        assert_eq!(
            main.motion_blur.as_ref().and_then(|mb| mb.shutter_angle),
            Some(180.0)
        );
        assert_eq!(main.markers.len(), 1);
        assert_eq!(main.markers[0].comment.as_deref(), Some("chorus"));

        let precomp = layer(&main, "Nested");
        assert_eq!(precomp.kind.as_deref(), Some("precomp"));
        assert_eq!(precomp.source_id, nested.id);
        assert!(nested.id.is_some(), "the nested comp keeps AE's own id");
    }

    /// **A bezier key and a hold key keep every side of every handle.**
    ///
    /// Keyframes are copied value-for-value (K-025), so this is the capture's
    /// highest-stakes shape: interpolation per *side*, and ease as an array
    /// with one entry per dimension. Both are easy to flatten by accident —
    /// reading a single ease instead of the array loses separated dimensions,
    /// and reading one interpolation type per key rather than per side turns
    /// every hold key into a ramp.
    #[test]
    fn a_bezier_key_and_a_hold_key_keep_their_ease_arrays() {
        let capture = opened().capture;
        let clip = layer(&comp(&capture, "Main"), "clip.mp4");
        let blur = prop(&clip.properties, "ADBE Gaussian Blur 2-0001")
            .expect("the blur has a Blurriness property");
        let keys = blur.keyframes.expect("Blurriness is animated");
        assert_eq!(keys.len(), 3);

        assert_eq!(keys[0].out_interp.as_deref(), Some("BEZIER"));
        let out_ease = keys[0].out_ease.as_deref().expect("an out ease array");
        assert_eq!(out_ease.len(), 1, "one entry per dimension");
        assert_eq!(out_ease[0].influence, Some(33.333333));
        assert_eq!(out_ease[0].speed, Some(0.0));

        assert_eq!(keys[1].in_interp.as_deref(), Some("BEZIER"));
        assert_eq!(keys[1].out_interp.as_deref(), Some("HOLD"));
        assert_eq!(
            keys[1]
                .in_ease
                .as_deref()
                .and_then(|ease| ease.first())
                .and_then(|ease| ease.speed),
            Some(120.0)
        );

        assert_eq!(keys[2].t, Some(4.0));
        assert_eq!(keys[2].roving, Some(false));
    }

    /// **A spatial key keeps its tangents, and a time remap arrives as an
    /// ordinary keyframed property.**
    ///
    /// Two things the schema deliberately does not special-case. Spatial
    /// tangents ride alongside temporal ease on the same key rather than in a
    /// separate structure, and "time remapping" is captured as just another
    /// property — the conversion to Retime segments is the mapping phase's
    /// job, not the walker's.
    #[test]
    fn a_spatial_key_keeps_its_tangents_and_time_remap_is_a_plain_property() {
        let capture = opened().capture;
        let precomp = layer(&comp(&capture, "Main"), "Nested");
        assert_eq!(precomp.time_remap_enabled, Some(true));

        let position = prop(&precomp.properties, "ADBE Position").expect("a position");
        let keys = position.keyframes.expect("position is animated");
        assert_eq!(
            keys[0].out_tangent.as_deref(),
            Some([120.0, 0.0, 0.0].as_slice())
        );
        assert_eq!(
            keys[1].in_tangent.as_deref(),
            Some([-120.0, 40.0, 0.0].as_slice())
        );
        // Spatial smoothness is its own pair of flags: this key is cornered in
        // time and smooth in space, which a single auto-bezier flag would lose.
        assert_eq!(keys[1].auto_bezier, Some(false));
        assert_eq!(keys[1].spatial_auto_bezier, Some(true));

        let remap = prop(&precomp.properties, "ADBE Time Remapping").expect("a time remap");
        let keys = remap.keyframes.expect("time remap is keyframed");
        assert_eq!(
            keys.last()
                .and_then(|key| key.out_interp.clone())
                .as_deref(),
            Some("HOLD")
        );
    }

    /// **A separated position keeps its followers, and the leader is empty.**
    ///
    /// AE's trap in schema form: when dimensions are separated, the leader
    /// property has no animation of its own — the curves live on the
    /// followers. A reader that looked only at the leader would import a
    /// separated position as a still one and lose the whole animation
    /// silently.
    #[test]
    fn a_separated_position_carries_its_animation_on_the_followers() {
        let capture = opened().capture;
        let clip = layer(&comp(&capture, "Main"), "clip.mp4");
        let leader = prop(&clip.properties, "ADBE Position").expect("a position leader");

        assert!(
            leader.keyframes.is_none(),
            "the leader of a separated property is not where the animation is"
        );
        assert!(
            leader.value.is_some(),
            "the leader still reports a still value — which is exactly the trap: \
             reading it would import the animation as one frozen position"
        );
        let followers = leader.separated.as_deref().expect("separated followers");
        assert_eq!(followers.len(), 2);
        assert_eq!(followers[0].name.as_deref(), Some("X Position"));
        assert_eq!(
            followers[0]
                .keyframes
                .as_deref()
                .expect("X is animated")
                .len(),
            2
        );
        assert_eq!(followers[1].value, Some(serde_json::json!(540.0)));
    }

    /// **A matte records the layer it points at, by index.**
    ///
    /// Both AE generations of matte end up as a type plus the referenced
    /// layer's stacking index, and normalising them is Rust's job later. What
    /// matters here is that the reference survives as an index into this comp's
    /// own stack, so the mapping phase can resolve it without guessing at the
    /// layer above.
    #[test]
    fn a_matte_records_the_layer_index_it_points_at() {
        let capture = opened().capture;
        let main = comp(&capture, "Main");
        let clip = layer(&main, "clip.mp4");
        let matte = clip.matte.expect("the clip has a matte");

        assert_eq!(matte.kind.as_deref(), Some("ALPHA_INVERTED"));
        assert_eq!(matte.layer_index, Some(3));

        let source = main
            .layers
            .iter()
            .find(|layer| layer.index == Some(3))
            .expect("the index resolves inside this comp's own stack");
        assert_eq!(source.name.as_deref(), Some("Black Solid 1"));
        // The other half of the pair, and the only thing the legacy
        // above-layer form has to say: this layer knows it is somebody's matte.
        assert_eq!(
            source.matte.as_ref().and_then(|matte| matte.is_track_matte),
            Some(true)
        );
    }

    /// **A mask keeps its mode, its inversion, and a real bezier path.**
    ///
    /// The mask's own facts sit in a `mask` block on the mask node rather than
    /// among its properties — that is AE's shape and the schema keeps it, and
    /// it is the one place a reader can silently look in the wrong spot and
    /// find nothing wrong: a missing mode reads as an ordinary Add mask, and
    /// the picture is wrong rather than broken. The path is the only property
    /// value with structure rather than numbers, so it is the one whose parse
    /// is worth proving: four vertices with matching tangent arrays and a
    /// closed flag.
    #[test]
    fn a_mask_keeps_its_mode_and_reads_back_as_a_bezier_path() {
        let capture = opened().capture;
        let clip = layer(&comp(&capture, "Main"), "clip.mp4");
        let node = prop(&clip.properties, "ADBE Mask Atom").expect("a mask");
        let mask = node
            .mask
            .clone()
            .expect("the mask node carries a mask block");

        assert_eq!(mask.mode.as_deref(), Some("SUBTRACT"));
        assert_eq!(mask.inverted, Some(true));
        assert_eq!(mask.locked, Some(false));

        let path = prop(node.children(), "ADBE Mask Shape").expect("a mask path");
        let shape = path.shape().expect("the path value reads as a shape");
        assert_eq!(shape.vertices.len(), 4);
        assert_eq!(shape.in_tangents.len(), 4);
        assert_eq!(shape.out_tangents.len(), 4);
        assert_eq!(shape.closed, Some(true));
        assert_eq!(shape.vertices[1], vec![100.0, 0.0]);

        let feather = prop(node.children(), "ADBE Mask Feather").expect("a feather");
        assert_eq!(feather.value, Some(serde_json::json!([12.0, 12.0])));
    }

    /// **An unreadable property is recorded where it stood, and again in the
    /// report.**
    ///
    /// After Effects' own scripting cannot read a `CUSTOM_VALUE` property —
    /// Curves' point list is the standing example (K-410). The whole point of
    /// capturing it as an `unreadable` node rather than omitting it is that the
    /// effect keeps its slot and its shape, so the import can say *this
    /// property* was unreadable rather than silently shipping a Curves with no
    /// curve. The report row is the same fact said again for the panel, and
    /// both halves must survive the read.
    #[test]
    fn an_unreadable_property_keeps_its_place_and_its_report_row() {
        let bundle = opened();
        let clip = layer(&comp(&bundle.capture, "Main"), "clip.mp4");
        let curves = prop(&clip.properties, "ADBE CurvesCustom").expect("the Curves effect");
        assert_eq!(curves.enabled, Some(true));

        let point_list = prop(curves.children(), "ADBE CurvesCustom-0001").expect("the point list");
        assert_eq!(point_list.value_type.as_deref(), Some("custom_blob"));
        assert!(point_list.value.is_none(), "there is no value to have");
        assert!(point_list
            .unreadable
            .as_deref()
            .is_some_and(|error| error.contains("not readable")));

        assert_eq!(bundle.report.unreadables.len(), 1);
        let row = &bundle.report.unreadables[0];
        assert_eq!(row.match_name.as_deref(), Some("ADBE CurvesCustom-0001"));
        assert_eq!(row.layer.as_deref(), Some("clip.mp4"));
        assert_eq!(row.comp.as_deref(), Some("Main"));
    }

    /// **An expression arrives as text, with its enabled state beside it.**
    ///
    /// Never evaluated and never rewritten by the Bridge: the source text is
    /// carried verbatim, and whether it was switched on is a separate fact —
    /// AE keeps a disabled expression's text, and so must the capture, or
    /// re-enabling it after an import would be impossible.
    #[test]
    fn an_expression_arrives_verbatim_with_its_enabled_state() {
        let capture = opened().capture;
        let clip = layer(&comp(&capture, "Main"), "clip.mp4");
        let opacity = prop(&clip.properties, "ADBE Opacity").expect("an opacity");

        assert_eq!(opacity.expression.as_deref(), Some("wiggle(2, 30)"));
        assert_eq!(opacity.expression_enabled, Some(true));
        assert_eq!(opacity.value, Some(serde_json::json!(100.0)));
    }

    /// **A newer major schema is refused, and the message says to update
    /// Lumit.**
    ///
    /// docs/11 §2.3's policy, and the one place the reader is deliberately
    /// strict: a capture written to a schema this build has never seen would be
    /// read with the wrong assumptions, and a wrong import is worse than a
    /// refused one. The check is on the *major* alone — an unreadable minor
    /// bump must still open, which the next test's extra key covers.
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
        let mut writer = zip::ZipWriter::new(File::create(&zipped).unwrap());
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

    /// **Something that is not a bundle is turned away by name.**
    ///
    /// A folder with no manifest, and a manifest for some other format, are
    /// both ordinary user mistakes (picking the parent folder, or a `.lum`
    /// project). Both must produce the plain "not a Lumit bundle" rather than a
    /// JSON parse error quoting a line number at someone who chose the wrong
    /// folder.
    #[test]
    fn a_folder_that_is_not_a_bundle_is_turned_away_plainly() {
        let temp = tempfile::tempdir().unwrap();
        assert!(matches!(
            open_bundle(temp.path()),
            Err(ImportError::NotABundle)
        ));

        fs::write(
            temp.path().join("manifest.json"),
            br#"{ "format": "lumit-project", "schema_version": "0.2.0" }"#,
        )
        .unwrap();
        assert!(matches!(
            open_bundle(temp.path()),
            Err(ImportError::NotABundle)
        ));
    }
}
