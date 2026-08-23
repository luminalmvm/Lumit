//! The differential test: parse `fixture.aep`, and check every structural
//! field against what After Effects itself said about the same project
//! (K-418, docs/impl/ae-import.md §7).
//!
//! In plain terms: `tools/ae-bridge/fixtures/` holds one real After Effects
//! project **and** the folder of JSON that After Effects' own scripting wrote
//! out of it, from the same sitting. That makes the parser's correctness
//! measurable rather than hopeful — every claim below is "the file says X and
//! After Effects also said X", not "X looks right". Nothing is hard-coded: the
//! expected values are read out of the golden capture, so the day the fixture
//! is rebuilt the test moves with it.
//!
//! The comparison covers the container, the project block, the item tree, comp
//! settings, the layer stack (**phase A**) and the property trees, keyframes,
//! effects, masks, markers and expressions (**phase B**). Everything it does
//! *not* demand equality of is exempted explicitly below rather than quietly
//! skipped — the exemption list is part of what this test asserts.
//!
//! # What "recovery" means here
//!
//! An `.aep` stores only what is **not at its default**: a layer nobody moved
//! has no Position record in it. So the golden capture's ~3,300 property leaves
//! are not 3,300 things to recover — 2,734 of them are absent from the file
//! because After Effects would put them back at their defaults on open, which
//! is what the mapping layer does with an absent property too. The claim this
//! file asserts is the useful one: **every leaf the file does store is right,
//! and nothing is reported that After Effects did not report.**
//!
//! # The exemption list
//!
//! Every field the comparison does *not* demand equality of, and why:
//!
//! 1. **Property display names**, where the file has none — After Effects' own
//!    localised resources rather than data in the project. A property nobody
//!    renamed carries the sentinel `-_0_/-` in its `tdsn`, so 1,106 of the
//!    golden's names have no source in the file at all and the mapper falls
//!    back to the match name. The 83 that *are* in the file — effect
//!    parameters (from `pard`), masks and effect instances (from `tdsn` and
//!    `fnam`) — are not exempt: each is asserted equal to After Effects' own
//!    below, because a drifted offset would otherwise hand every parameter its
//!    neighbour's name with nothing failing.
//! 2. **A text document (`ADBE Text Document`)** — its own encoding (`btds`, a
//!    COS blob), phase C. It arrives with its match name and a note saying the
//!    encoding is not decoded, rather than being dropped. A **gradient**
//!    (`GCst`) is owed the same treatment and has none: `fixture.aep` contains
//!    no `GCst` chunk at all, because the shape layer's gradient sits at its
//!    default and the file stores only what does not. So there is nothing here
//!    to exempt and nothing to prove — the gradient is phase C's, unmeasured
//!    (docs/impl/ae-import.md §7.2, docs/TODO.md).
//! 3. **`layer.time_remap_enabled` on camera and light layers** — scripting
//!    does not offer the switch on a rig, so the golden has no value to compare
//!    against; the parser leaves it absent for exactly the same reason.
//! 4. **Footage interpretation** (`fps`, `native_fps`, `duration`,
//!    `fps_override`, `alpha`, `premul_colour`, `invert_alpha`, `loop`,
//!    `fields`, `remove_pulldown`, `is_still`, `is_placeholder`,
//!    `is_missing`) — the golden project contains no file footage at all, only
//!    solids, so not one of these offsets could be checked against After
//!    Effects. Reading them unchecked is precisely the silently-wrong import
//!    this route exists to avoid, so the parser does not read them and this
//!    test does not pretend otherwise. Owed: a fixture with real footage
//!    (docs/TODO.md). `path` is **not** in this list: it comes out of the
//!    self-naming JSON in `LIST Als2` ▸ `alas` rather than out of an offset,
//!    and `aep::tests` covers it directly.
//! 5. **A negative-stretch layer's `in_point` and `out_point`** — compared
//!    within one frame instead of exactly. The file stores the layer
//!    unstretched (in 0, out 10) and After Effects reports the reflection, but
//!    its reflection lands 1/3000 s further out than the arithmetic does
//!    (`-0.000333` and `-10.000333` rather than `-0` and `-10`): the reflection
//!    is about a half-unit of some internal grid, and one sample is not enough
//!    to prove what that grid is. Recorded rather than curve-fitted
//!    (docs/impl/ae-import.md §7, docs/TODO.md).
//! 6. **`comp.renderer`'s three non-Classic values** and every enum code the
//!    fixture does not contain — the funnel tables carry them from the
//!    reference implementation, and `aep::enums`' own tests cover only the
//!    fall-through. A second fixture is owed for those rows.
//! 7. **A leaf whose enabled expression drives it** — the DOM reports what the
//!    expression *evaluated to*, and the file holds the value underneath it.
//!    The expression itself and its on/off state are both recovered exactly,
//!    which is what actually carries the animation.
//! 8. **A dimension-separated leader's own value** — the DOM computes it from
//!    the followers; the file leaves it out because the followers hold the
//!    animation. The followers themselves are recovered with their keys.
//! 9. **The `CUSTOM_VALUE` blobs** — the DOM could not read them at all, so
//!    there is no value to compare against. This route recovers the raw bytes,
//!    which is the one place it beats the Bridge outright (K-412's stretch).
//! 10. **Two derived keyframe numbers** — a mask path's linear speed (the DOM
//!     reports exactly 1.0 per segment and one sample cannot say whether that
//!     is a constant), and every linear speed to a relative 1e-3 rather than
//!     bit for bit, because After Effects does not store a linear key's ease at
//!     all: it works the number out on demand, and so does the parser.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lumit_import::capture::{Capture, Comp, Item, Layer, Property};
use lumit_import::{open_aep, Bundle, BundleSource, Manifest};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("ae-bridge")
        .join("fixtures")
}

fn parsed() -> Bundle {
    open_aep(&fixtures().join("fixture.aep")).expect("the golden .aep parses")
}

fn golden() -> Capture {
    let bytes = std::fs::read(fixtures().join("fixture.lum-bundle").join("capture.json"))
        .expect("the golden capture is beside the .aep");
    serde_json::from_slice(&bytes).expect("the golden capture parses")
}

fn golden_manifest() -> Manifest {
    let bytes = std::fs::read(fixtures().join("fixture.lum-bundle").join("manifest.json"))
        .expect("the golden manifest is beside the .aep");
    serde_json::from_slice(&bytes).expect("the golden manifest parses")
}

/// Floats cross two different roads to get here — After Effects' own decimal
/// printing on one side, a rational divide on the other — so they are compared
/// at the precision the capture was written with, not bit for bit.
#[track_caller]
fn near(got: Option<f64>, want: Option<f64>, what: &str) {
    match (got, want) {
        (Some(got), Some(want)) => assert!(
            (got - want).abs() < 1e-9,
            "{what}: parsed {got}, After Effects said {want}"
        ),
        (got, want) => assert_eq!(got, want, "{what}"),
    }
}

#[track_caller]
fn near_list(got: Option<&Vec<f64>>, want: Option<&Vec<f64>>, tolerance: f64, what: &str) {
    match (got, want) {
        (Some(got), Some(want)) => {
            assert_eq!(got.len(), want.len(), "{what}: different lengths");
            for (index, (got, want)) in got.iter().zip(want).enumerate() {
                assert!(
                    (got - want).abs() < tolerance,
                    "{what}[{index}]: parsed {got}, After Effects said {want}"
                );
            }
        }
        (got, want) => assert_eq!(got, want, "{what}"),
    }
}

/// **The container opens, and the file says which After Effects wrote it.**
///
/// The shallow check everything below leans on: the RIFX header and the `Egg!`
/// form type are what they should be, the bundle comes out marked as the direct
/// route, and the version packed into `head`'s bitfield reads back as the same
/// build string After Effects stamped into the bundle's manifest. That last one
/// is worth its own assertion because the version word is a bitfield with the
/// major version split across two runs — get the split wrong and the parser
/// would refuse (or misread) version-dependent records for a reason nothing
/// else would surface.
#[test]
fn the_project_opens_and_names_the_after_effects_that_wrote_it() {
    let bundle = parsed();
    assert_eq!(bundle.source, BundleSource::Aep);
    assert_eq!(bundle.manifest.format.as_deref(), Some("lumit-ae-bundle"));
    assert_eq!(
        bundle.manifest.ae_version,
        golden_manifest().ae_version,
        "the packed version word must read back as the build After Effects named"
    );
    assert!(
        bundle.report.unreadables.is_empty(),
        "nothing in the golden project should be skipped: {:?}",
        bundle.report.unreadables
    );
}

/// **The project block matches field for field.**
///
/// These five settings belong to the project rather than to any item, and
/// docs/11 §3's colour flagging cannot be worked out afterwards without them.
/// Three are the awkward kind: the colour depth is stored as an *exponent*
/// rather than a bit count, and linear blending and the linearised working
/// space are chunks with no payload at all — the fact is whether the chunk is
/// present.
#[test]
fn the_project_settings_match_after_effects() {
    let got = parsed().capture.project.expect("a project block");
    let want = golden().project.expect("the golden has a project block");

    assert_eq!(
        got.bits_per_channel, want.bits_per_channel,
        "bits_per_channel"
    );
    assert_eq!(got.working_space, want.working_space, "working_space");
    assert_eq!(got.linear_blending, want.linear_blending, "linear_blending");
    assert_eq!(
        got.linearize_working_space, want.linearize_working_space,
        "linearize_working_space"
    );
    assert_eq!(
        got.expression_engine, want.expression_engine,
        "expression_engine"
    );
}

/// **Every item arrives, in After Effects' own order, with its own id.**
///
/// The item list is the spine of the whole import: a comp is named by its item
/// row, a precomp layer reaches its comp through an id, and a solid layer
/// reaches its colour the same way. Order matters as much as content — the
/// Bridge walks the project tree depth first, and the parser has to arrive at
/// the same sequence or every later comparison is comparing the wrong pair.
///
/// The solids are the interesting half: After Effects stores a solid as a
/// *footage* item whose name is empty, with the real name and the colour buried
/// in the asset-info record. A reader that trusted the ordinary name chunk
/// would import eighteen nameless white cards.
#[test]
fn the_item_tree_matches_entry_for_entry() {
    let got = parsed().capture.items;
    let want = golden().items;

    assert_eq!(got.len(), want.len(), "item count");
    assert_eq!(
        got.iter().map(|item| item.id).collect::<Vec<_>>(),
        want.iter().map(|item| item.id).collect::<Vec<_>>(),
        "item ids, in order — the Bridge walks the folder tree depth first"
    );

    for (got, want) in got.iter().zip(&want) {
        let who = format!("item {:?} ({:?})", want.id, want.name);
        assert_eq!(got.name, want.name, "{who}: name");
        assert_eq!(got.parent_id, want.parent_id, "{who}: parent_id");
        assert_eq!(got.kind, want.kind, "{who}: kind");
        assert_eq!(got.width, want.width, "{who}: width");
        assert_eq!(got.height, want.height, "{who}: height");
        // A solid's colour is three 32-bit floats in the file and prints as
        // fourteen decimal digits in the capture, so the tolerance is the
        // printing's, not the parser's.
        near_list(
            got.colour.as_ref(),
            want.colour.as_ref(),
            1e-9,
            &format!("{who}: colour"),
        );
    }

    // Exemption 4, asserted rather than assumed: the golden project has no file
    // footage, so there is nothing here whose interpretation could be checked.
    assert!(
        want.iter()
            .all(|item| item.kind.as_deref() != Some("footage")),
        "this exemption stops being honest the moment the fixture gains real \
         footage — implement the interpretation fields then"
    );
}

/// **Comp settings match, including the ones assembled from parts.**
///
/// Almost nothing in a `cdta` record is a plain number. The frame rate is an
/// integer plus a fraction in 1/65536ths (which is how 23.976 stays itself),
/// the duration and the start time are rationals, the background is three
/// bytes, and motion blur's master switch is one bit in a flag byte that sits
/// nowhere near the shutter settings it governs. Every one of those is a place
/// a plausible-looking offset produces a plausible-looking wrong number, which
/// is why they are all compared against what After Effects said rather than
/// eyeballed.
#[test]
fn comp_settings_match_after_effects() {
    let got = parsed().capture.comps;
    let want = golden().comps;

    assert_eq!(got.len(), want.len(), "comp count");
    assert_eq!(
        got.iter().map(|comp| comp.id).collect::<Vec<_>>(),
        want.iter().map(|comp| comp.id).collect::<Vec<_>>(),
        "comp ids, in order"
    );

    for (got, want) in got.iter().zip(&want) {
        let who = format!("comp {:?}", want.id);
        assert_eq!(got.width, want.width, "{who}: width");
        assert_eq!(got.height, want.height, "{who}: height");
        near(got.par, want.par, &format!("{who}: par"));
        near(got.fps, want.fps, &format!("{who}: fps"));
        near(got.duration, want.duration, &format!("{who}: duration"));
        near(got.start, want.start, &format!("{who}: start"));
        assert_eq!(got.renderer, want.renderer, "{who}: renderer");
        assert_eq!(
            got.preserve_nested_fps, want.preserve_nested_fps,
            "{who}: preserve_nested_fps"
        );
        assert_eq!(
            got.preserve_nested_resolution, want.preserve_nested_resolution,
            "{who}: preserve_nested_resolution"
        );
        // The background is stored as three bytes and reported as three floats,
        // so half a step of 1/255 is the most the round trip can preserve.
        near_list(
            got.bg_colour.as_ref(),
            want.bg_colour.as_ref(),
            0.5 / 255.0,
            &format!("{who}: bg_colour"),
        );

        let (blur, wanted) = (
            got.motion_blur.as_ref().expect("motion blur"),
            want.motion_blur.as_ref().expect("golden motion blur"),
        );
        assert_eq!(blur.enabled, wanted.enabled, "{who}: motion_blur.enabled");
        near(
            blur.shutter_angle,
            wanted.shutter_angle,
            &format!("{who}: shutter_angle"),
        );
        near(
            blur.shutter_phase,
            wanted.shutter_phase,
            &format!("{who}: shutter_phase"),
        );
        assert_eq!(blur.samples, wanted.samples, "{who}: motion_blur.samples");
        assert_eq!(
            blur.adaptive_limit, wanted.adaptive_limit,
            "{who}: motion_blur.adaptive_limit"
        );
    }
}

/// **Every layer of every comp matches, everywhere but its property tree.**
///
/// The long one, and the one the phase is for. It walks both stacks together
/// and compares identity, timing, parentage, the label, the blend mode, the
/// matte reference, auto-orientation, the light type and all thirteen switches.
///
/// Three traps are worth naming, because each of them produces a project that
/// opens and is wrong rather than one that fails:
///
/// - **Only `LIST:Layr` is a layer.** A comp also carries `DLay`, `SLay` and
///   `CLay` (the viewer's own view cameras) and `SecL` (a hidden layer that
///   exists to hold the comp's markers) — eleven records with the same shape as
///   a layer, none of which is one.
/// - **A layer's parent and its matte are stored as another layer's id**, not
///   as an index, so the whole stack has to be read before either can be
///   resolved.
/// - **A null and an adjustment layer are backed by a solid item.** Letting the
///   source decide the kind imports a rig's null as the white card it is made
///   of (docs/impl/ae-import.md §5).
#[test]
fn every_layer_matches_after_effects_except_its_properties() {
    let got = parsed().capture.comps;
    let want = golden().comps;

    let mut compared = 0_usize;
    for (got, want) in got.iter().zip(&want) {
        assert_eq!(
            got.layers.len(),
            want.layers.len(),
            "comp {:?}: layer count — a viewer's view camera is not a layer",
            want.id
        );
        // Exemption 5's tolerance, taken from the fixture's own frame rate
        // rather than written down as a constant.
        let frame = 1.0 / want.fps.expect("the golden comp names its frame rate");
        for (got, want) in got.layers.iter().zip(&want.layers) {
            compare_layer(got, want, frame);
            compared += 1;
        }
    }
    assert_eq!(
        compared, 24,
        "the golden project's two comps hold 24 layers"
    );
}

#[track_caller]
fn compare_layer(got: &Layer, want: &Layer, frame: f64) {
    let who = format!(
        "layer {:?} ({:?})",
        want.index,
        want.name.clone().unwrap_or_default()
    );
    assert_eq!(got.index, want.index, "{who}: index");
    assert_eq!(got.name, want.name, "{who}: name");
    assert_eq!(got.kind, want.kind, "{who}: kind");
    assert_eq!(got.source_id, want.source_id, "{who}: source_id");
    assert_eq!(got.label, want.label, "{who}: label");
    assert_eq!(got.parent_index, want.parent_index, "{who}: parent_index");
    assert_eq!(got.blend, want.blend, "{who}: blend");
    assert_eq!(
        got.preserve_transparency, want.preserve_transparency,
        "{who}: preserve_transparency"
    );
    assert_eq!(got.auto_orient, want.auto_orient, "{who}: auto_orient");
    assert_eq!(got.light_type, want.light_type, "{who}: light_type");
    assert_eq!(
        got.time_remap_enabled, want.time_remap_enabled,
        "{who}: time_remap_enabled"
    );

    near(
        got.start_time,
        want.start_time,
        &format!("{who}: start_time"),
    );
    near(got.stretch, want.stretch, &format!("{who}: stretch"));

    // Exemption 5. A reflected layer's two ends land 1/3000 s further out in
    // After Effects' arithmetic than in the file's; within a frame is as exact
    // as one sample can honestly claim.
    let reflected = want.stretch.is_some_and(|stretch| stretch < 0.0);
    if reflected {
        let slack = |got: Option<f64>, want: Option<f64>, what: &str| {
            let (Some(got), Some(want)) = (got, want) else {
                panic!("{who}: {what} missing on a reflected layer");
            };
            assert!(
                (got - want).abs() < frame,
                "{who}: {what} parsed {got}, After Effects said {want} — \
                 further apart than one frame, so this is not the recorded \
                 1/3000 s reflection offset"
            );
        };
        slack(got.in_point, want.in_point, "in_point");
        slack(got.out_point, want.out_point, "out_point");
        assert!(
            got.in_point > got.out_point,
            "{who}: a reflected layer's ends arrive the other way round, in order"
        );
    } else {
        near(got.in_point, want.in_point, &format!("{who}: in_point"));
        near(got.out_point, want.out_point, &format!("{who}: out_point"));
    }

    let got_matte = got.matte.clone().unwrap_or_default();
    let want_matte = want.matte.clone().unwrap_or_default();
    assert_eq!(got_matte.kind, want_matte.kind, "{who}: matte.type");
    assert_eq!(
        got_matte.layer_index, want_matte.layer_index,
        "{who}: matte.layer_index"
    );
    assert_eq!(
        got_matte.is_track_matte, want_matte.is_track_matte,
        "{who}: matte.is_track_matte — being used as a matte is a fact about \
         whoever points at this layer"
    );

    let got_switches = got.switches.clone().unwrap_or_default();
    let want_switches = want.switches.clone().unwrap_or_default();
    assert_eq!(got_switches, want_switches, "{who}: switches");

    assert_eq!(got.markers.len(), want.markers.len(), "{who}: marker count");
    for (got, want) in got.markers.iter().zip(&want.markers) {
        near(got.t, want.t, &format!("{who}: marker time"));
        near(
            got.duration,
            want.duration,
            &format!("{who}: marker duration"),
        );
        assert_eq!(got.comment, want.comment, "{who}: marker comment");
        assert_eq!(got.chapter, want.chapter, "{who}: marker chapter");
        assert_eq!(got.label, want.label, "{who}: marker label");
    }
}

// ---------------------------------------------------------------------------
// Phase B: the property trees, scored per category.
// ---------------------------------------------------------------------------

/// One property tree, flattened to `path -> node` so the two sides can be
/// walked together. Repeated match names (two mask atoms, two effects of the
/// same kind) are numbered so they do not collide.
fn flatten<'a>(props: &'a [Property], prefix: &str, out: &mut BTreeMap<String, &'a Property>) {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for node in props {
        let name = node.match_name.as_deref().unwrap_or("");
        let count = seen.entry(name).or_insert(0);
        *count += 1;
        let path = format!("{prefix}/{name}#{count}");
        out.insert(path.clone(), node);
        flatten(node.children(), &path, out);
        if let Some(followers) = node.separated.as_deref() {
            flatten(followers, &format!("{path}!separated"), out);
        }
    }
}

/// One layer's name, its parsed property tree, and the golden one, all
/// flattened to the same paths.
type LayerTrees = (
    String,
    BTreeMap<String, Property>,
    BTreeMap<String, Property>,
);

/// Every layer of the golden capture beside the parsed one, flattened.
fn trees() -> Vec<LayerTrees> {
    let parsed = parsed().capture;
    let golden = golden();
    let mut out = Vec::new();
    for (got, want) in parsed.comps.iter().zip(&golden.comps) {
        for (got, want) in got.layers.iter().zip(&want.layers) {
            let (mut a, mut b) = (BTreeMap::new(), BTreeMap::new());
            flatten(&got.properties, "", &mut a);
            flatten(&want.properties, "", &mut b);
            out.push((
                want.name.clone().unwrap_or_default(),
                a.into_iter().map(|(k, v)| (k, v.clone())).collect(),
                b.into_iter().map(|(k, v)| (k, v.clone())).collect(),
            ));
        }
    }
    out
}

/// Numbers close enough to have crossed two different roads: After Effects'
/// fourteen-significant-figure printing on one side, an f64 divide on the other.
fn same_value(got: &serde_json::Value, want: &serde_json::Value) -> bool {
    match (got, want) {
        (serde_json::Value::Number(got), serde_json::Value::Number(want)) => {
            (got.as_f64().unwrap_or_default() - want.as_f64().unwrap_or_default()).abs() < 1e-4
        }
        (serde_json::Value::Array(got), serde_json::Value::Array(want)) => {
            got.len() == want.len() && got.iter().zip(want).all(|(a, b)| same_value(a, b))
        }
        (serde_json::Value::Object(got), serde_json::Value::Object(want)) => {
            got.len() == want.len()
                && got
                    .iter()
                    .all(|(key, value)| want.get(key).is_some_and(|other| same_value(value, other)))
        }
        (got, want) => got == want,
    }
}

/// **Every property the file stores agrees with After Effects, and there is
/// nothing in the parse that After Effects did not describe.**
///
/// The headline phase-B number, and the shape of the whole route's honesty. An
/// `.aep` records only what is *not* at its default, so the parser recovers a
/// few hundred of the golden capture's three thousand leaves — and that is the
/// right answer, not a shortfall: the rest are absent from the file because
/// After Effects would put them back at their defaults too. What must hold is
/// that **every leaf that is there is right**, and that the parser never
/// invents a leaf After Effects did not report.
///
/// The exemptions below are the six ways a leaf can legitimately differ.
#[test]
fn every_stored_property_value_matches_after_effects() {
    let mut agreed = 0_usize;
    let mut exempt = 0_usize;
    let mut invented = 0_usize;
    let mut named = 0_usize;

    for (layer, got, want) in trees() {
        for (path, node) in &got {
            let Some(other) = want.get(path) else {
                invented += 1;
                println!("{layer}: {path} is in the parse and not in the capture");
                continue;
            };
            // Exemption 1 is that most display names have no source in the file
            // at all — but a name the parser *does* produce (an effect
            // parameter's, out of `pard`; a mask's or an effect instance's, out
            // of `tdsn`/`fnam`) must be After Effects' own. A drifted `pard`
            // offset would otherwise hand every parameter its neighbour's name
            // with nothing failing.
            if let Some(mine) = node.name.as_deref() {
                assert_eq!(
                    Some(mine),
                    other.name.as_deref(),
                    "{layer} {path}: display name"
                );
                named += 1;
            }
            if node.group.is_some() {
                assert_eq!(
                    node.enabled, other.enabled,
                    "{layer} {path}: a group's own switch"
                );
                continue;
            }
            if node.keyframes.is_some() || other.keyframes.is_some() {
                continue; // scored by the keyframe test below
            }
            // Exemption A: an enabled expression drives the property, and the
            // DOM reports what the expression *evaluated to* rather than the
            // stored value. Nothing in the file can produce that number; the
            // expression itself comes through, which is what actually matters.
            // Exemption B: a dimension-separated leader has no stored value —
            // the DOM computes one from the followers, which are recovered.
            // Exemption C: a `CUSTOM_VALUE` blob, which the parser recovers as
            // raw bytes and the DOM could not read at all.
            // Exemption D: a text document, whose own encoding is phase C.
            let expression_driven = other.expression.is_some()
                && other.expression_enabled == Some(true)
                && node.value.is_some();
            let separated_leader = node.separated.is_some() && node.value.is_none();
            let blob = other.value_type.as_deref() == Some("custom_blob");
            let text = other.value_type.as_deref() == Some("text");
            if expression_driven || separated_leader || blob || text {
                exempt += 1;
                continue;
            }

            assert_eq!(
                node.value_type, other.value_type,
                "{layer} {path}: value type"
            );
            match (&node.value, &other.value) {
                (Some(got), Some(want)) => assert!(
                    same_value(got, want),
                    "{layer} {path}: parsed {got}, After Effects said {want}"
                ),
                (got, want) => assert_eq!(got, want, "{layer} {path}: value"),
            }
            agreed += 1;
        }
    }

    assert_eq!(
        invented, 0,
        "the parser never reports a property After Effects did not"
    );
    // The pins. A regression that stops reading a subtree drops the first; one
    // that starts guessing at a default lifts it. The 38 above the 646 the file
    // actually stores are the two placing properties After Effects does not
    // default to zero — Position at the centre of the comp, Anchor Point at the
    // centre of the source — written in by the parser and asserted here against
    // After Effects' own numbers, which is what makes them a recovery rather
    // than a guess.
    assert_eq!(
        agreed, 684,
        "static property values recovered exactly from the .aep"
    );
    assert_eq!(exempt, 6, "the exempt leaves, enumerated in the module doc");
    assert_eq!(
        named, 83,
        "display names recovered from the file, every one of them After \
         Effects' own (exemption 1 covers the 1,106 that have no source at all)"
    );
}

/// **The blobs After Effects itself cannot read come through as bytes.**
///
/// The one place the direct route beats the Bridge outright (K-410's honesty
/// note, K-412's stretch goal): Curves' point list, Levels' histogram and
/// Hue/Saturation's channel ranges are `CUSTOM_VALUE` properties the scripting
/// DOM refuses, and they are sitting in the file in an `aRbs` block. They are
/// carried undecoded — decoding is a separate job — but they are *carried*, so
/// the day a decoder exists there is something for it to decode.
#[test]
fn the_custom_value_blobs_the_dom_cannot_read_arrive_as_raw_bytes() {
    let mut blobs = Vec::new();
    for (_, got, want) in trees() {
        for (path, node) in &got {
            if want
                .get(path)
                .is_some_and(|other| other.value_type.as_deref() == Some("custom_blob"))
            {
                let bytes = node
                    .value
                    .as_ref()
                    .and_then(|value| value.get("bytes"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .len();
                assert_eq!(
                    node.value_type.as_deref(),
                    Some("custom_blob"),
                    "{path}: the blob keeps the capture's own word for it"
                );
                assert!(
                    node.unreadable.is_some(),
                    "{path}: undecoded, and honest about it"
                );
                blobs.push((path.clone(), bytes / 2));
            }
        }
    }
    blobs.sort();
    let names: Vec<&str> = blobs
        .iter()
        .map(|(path, _)| path.rsplit('/').next().unwrap_or(path))
        .collect();
    assert_eq!(
        names,
        vec![
            "ADBE CurvesCustom-0001#1",
            "ADBE Easy Levels2-0002#1",
            "ADBE HUE SATURATION-0003#1",
        ]
    );
    for (path, bytes) in &blobs {
        assert!(*bytes > 0, "{path}: the blob is not empty");
    }
}

/// **Every keyframe the golden capture has, the parser has, with its time, its
/// value, its per-side interpolation and its ease.**
///
/// This is the fidelity claim phase B exists to make, and it is asserted rather
/// than measured: the *count* of recovered keys is pinned, and every one of them
/// is compared field by field.
///
/// Two derived numbers are compared loosely rather than exactly, and both for
/// the same reason — After Effects does not *store* them, it works them out
/// when asked, and a linear key's stored ease is all zeros:
///
/// - **A spatial linear speed** is the length of the motion path between two
///   keys, and After Effects measures the *curve* through the two keys' own
///   spatial handles rather than the straight line between them. The parser
///   walks that cubic (the reference implementation takes the chord, which is
///   1.1–2.5% short on this fixture), so the two are compared at a relative
///   1e-3 — loose enough for a number arrived at by two different routes, tight
///   enough that taking the chord fails (docs/impl/ae-import.md §7.2).
/// - **A mask path's linear speed** is reported by the DOM as exactly 1.0 per
///   segment; whether that is a constant or a duration-derived number cannot be
///   told from one sample, so it is exempted rather than guessed. Lumit's own
///   mapping never reads a linear side's speed, so nothing downstream depends
///   on either.
#[test]
fn every_keyframe_matches_after_effects() {
    let mut keys = 0_usize;
    let mut properties = 0_usize;

    for (layer, got, want) in trees() {
        for (path, node) in &got {
            let Some(other) = want.get(path) else {
                continue;
            };
            let (Some(got_keys), Some(want_keys)) =
                (node.keyframes.as_deref(), other.keyframes.as_deref())
            else {
                assert_eq!(
                    node.keyframes.is_some(),
                    other.keyframes.is_some(),
                    "{layer} {path}: one side has keys and the other does not"
                );
                continue;
            };
            assert_eq!(
                got_keys.len(),
                want_keys.len(),
                "{layer} {path}: keyframe count"
            );
            properties += 1;
            let path_property = other.value_type.as_deref() == Some("shape");

            for (index, (got, want)) in got_keys.iter().zip(want_keys).enumerate() {
                let who = format!("{layer} {path} key {index}");
                near(got.t, want.t, &format!("{who}: time"));
                assert_eq!(got.in_interp, want.in_interp, "{who}: in interpolation");
                assert_eq!(got.out_interp, want.out_interp, "{who}: out interpolation");
                match (&got.v, &want.v) {
                    (Some(got), Some(want)) => assert!(
                        same_value(got, want),
                        "{who}: value — parsed {got}, After Effects said {want}"
                    ),
                    (got, want) => assert_eq!(got, want, "{who}: value"),
                }
                // Spatial tangents cross as f32 in the DOM and f64 here.
                near_list(
                    got.in_tangent.as_ref(),
                    want.in_tangent.as_ref(),
                    1e-3,
                    &format!("{who}: in tangent"),
                );
                near_list(
                    got.out_tangent.as_ref(),
                    want.out_tangent.as_ref(),
                    1e-3,
                    &format!("{who}: out tangent"),
                );
                for (side, got, want) in [
                    ("in", &got.in_ease, &want.in_ease),
                    ("out", &got.out_ease, &want.out_ease),
                ] {
                    let (got, want) = (
                        got.as_deref().unwrap_or_default(),
                        want.as_deref().unwrap_or_default(),
                    );
                    assert_eq!(got.len(), want.len(), "{who}: {side} ease, one per axis");
                    for (axis, (got, want)) in got.iter().zip(want).enumerate() {
                        near(
                            got.influence,
                            want.influence,
                            &format!("{who}: {side} ease [{axis}] influence"),
                        );
                        if path_property {
                            continue; // the exemption above
                        }
                        let (a, b) = (
                            got.speed.unwrap_or_default(),
                            want.speed.unwrap_or_default(),
                        );
                        assert!(
                            (a - b).abs() <= 1e-3 * b.abs().max(1.0),
                            "{who}: {side} ease [{axis}] speed — parsed {a}, \
                             After Effects said {b}"
                        );
                    }
                }
            }
            keys += got_keys.len();
        }
    }

    // The pins: the golden capture's own totals, so a regression that loses a
    // property class fails loudly rather than quietly halving the animation.
    // Nine keyframed properties in the golden capture, and the two separated
    // followers counted a second time under their leader — 23 keys plus those
    // four.
    assert_eq!(properties, 11, "keyframed properties recovered");
    assert_eq!(keys, 27, "keyframes recovered");
}

/// **Expressions, effect instances, masks and separated dimensions all come
/// across.**
///
/// The four features that are not one value each, counted and compared in one
/// place so the numbers can be pinned together.
#[test]
fn expressions_effects_masks_and_separated_dimensions_match() {
    let mut expressions = 0_usize;
    let mut effects = 0_usize;
    let mut masks = 0_usize;
    let mut separated = 0_usize;

    for (layer, got, want) in trees() {
        for (path, node) in &got {
            let Some(other) = want.get(path) else {
                continue;
            };

            if let Some(source) = other.expression.as_deref().filter(|s| !s.trim().is_empty()) {
                assert_eq!(
                    node.expression.as_deref().map(str::trim),
                    Some(source.trim()),
                    "{layer} {path}: expression source"
                );
                assert_eq!(
                    node.expression_enabled, other.expression_enabled,
                    "{layer} {path}: whether the expression is switched on"
                );
                expressions += 1;
            }

            if let Some(mask) = other.mask.as_ref() {
                let got_mask = node.mask.as_ref().expect("the mask block is recovered");
                assert_eq!(got_mask.mode, mask.mode, "{layer} {path}: mask mode");
                assert_eq!(
                    got_mask.inverted, mask.inverted,
                    "{layer} {path}: mask inverted"
                );
                assert_eq!(
                    got_mask.roto_bezier, mask.roto_bezier,
                    "{layer} {path}: mask RotoBezier"
                );
                assert_eq!(got_mask.locked, mask.locked, "{layer} {path}: mask locked");
                near_list(
                    got_mask.colour.as_ref(),
                    mask.colour.as_ref(),
                    1e-6,
                    &format!("{layer} {path}: mask colour"),
                );
                masks += 1;
            }

            if other.separated.is_some() {
                // The followers the *file* stores: After Effects lists all
                // three axes and stores only the ones off their default, so the
                // Z follower of a 2D layer is absent from both the file and the
                // layer's own group.
                let followers = node
                    .separated
                    .as_deref()
                    .expect("the leader is recognised as separated");
                assert!(!followers.is_empty(), "{layer} {path}: separated followers");
                for follower in followers {
                    let name = follower.match_name.clone().unwrap_or_default();
                    let want = other
                        .separated
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .find(|f| f.match_name.as_deref() == Some(name.as_str()))
                        .unwrap_or_else(|| panic!("{layer} {path}: {name} is not a follower"));
                    assert_eq!(
                        follower.keyframes.as_deref().map(<[_]>::len),
                        want.keyframes.as_deref().map(<[_]>::len),
                        "{layer} {path}: {name} keeps its animation"
                    );
                }
                separated += 1;
            }

            if path.starts_with("/ADBE Effect Parade#1/") && path.matches('/').count() == 2 {
                assert_eq!(
                    node.enabled, other.enabled,
                    "{layer} {path}: the effect's own on/off switch"
                );
                assert_eq!(
                    node.name, other.name,
                    "{layer} {path}: the effect's display name"
                );
                effects += 1;
            }
        }
    }

    assert_eq!(
        expressions, 2,
        "expressions recovered, enabled and disabled"
    );
    assert_eq!(effects, 13, "effect instances recovered");
    assert_eq!(masks, 2, "masks recovered with their own facts");
    assert_eq!(separated, 1, "dimension-separated properties recovered");
}

/// **The document the parser produces counts the same as the document the
/// bundle produces.**
///
/// The end of the funnel (K-418): both front ends feed one `map_capture`, so
/// the only honest measure of the parse is the *engine-side* document beside
/// the one After Effects' own export makes. Every count matches exactly; the
/// import report is the one place they differ, and only because the Bridge
/// recorded six properties its own DOM could not read.
#[test]
fn the_parsed_document_counts_the_same_as_the_bundles() {
    let from_aep = lumit_import::map_capture(&parsed().capture).0;
    let from_bundle = lumit_import::map_capture(&golden()).0;

    let count = |document: &lumit_core::model::Document| {
        let mut totals = [0_usize; 6];
        totals[0] = document.items.len();
        for item in &document.items {
            if let lumit_core::model::ProjectItem::Composition(comp) = item {
                totals[1] += 1;
                totals[5] += comp.markers.len();
                for layer in &comp.layers {
                    totals[2] += 1;
                    totals[3] += layer.effects.len();
                    totals[4] += layer.masks.len();
                    totals[5] += layer.markers.len();
                }
            }
        }
        totals
    };

    let got = count(&from_aep);
    assert_eq!(
        got,
        count(&from_bundle),
        "items, comps, layers, effects, masks and markers"
    );
    assert_eq!(
        got,
        [22, 2, 24, 13, 2, 4],
        "and those counts are the golden project's own"
    );
}

/// **The same file parses to the same capture, every time.**
///
/// Determinism is a standing rule (docs/14 §…), and a parser is where it is
/// easiest to lose by accident: iterate a map instead of a list, or let a
/// lookup decide an order, and the item tree comes out shuffled between runs.
/// Comparing two whole parses of the same bytes catches that without needing to
/// know where it happened.
#[test]
fn the_same_file_parses_to_the_same_capture_every_time() {
    let once = parsed();
    let twice = parsed();
    assert_eq!(once.capture, twice.capture);
    assert_eq!(once.manifest, twice.manifest);
}

/// **The whole golden capture round-trips through the mapper unchanged in
/// shape.**
///
/// Not a fidelity claim — phase A has no properties to map — but the point of
/// K-418's one-funnel architecture: the capture the parser produces goes into
/// the *same* `map_capture` the Bridge's does, and comes out a document with
/// the same comps and layers rather than falling over on a shape the mapper has
/// never seen.
#[test]
fn the_parsed_capture_maps_through_the_shared_importer() {
    let bundle = parsed();
    let (document, _report) = lumit_import::map_capture(&bundle.capture);
    let comps = document
        .items
        .iter()
        .filter(|item| matches!(item, lumit_core::model::ProjectItem::Composition(_)))
        .count();
    assert_eq!(
        comps,
        bundle.capture.comps.len(),
        "every comp survives the shared mapping"
    );
}

/// **A damaged project is an error or a partial read, never a crash and never a
/// hang.**
///
/// This parser eats untrusted files: an `.aep` arrives from wherever the user
/// got it, and every length in it is attacker-controlled. So the golden file is
/// damaged sixty-four ways — cut short, single bytes flipped, and chunk sizes
/// overwritten with enormous ones — and the parse of each must come back with an
/// answer. The seeds are fixed, so a failure names a case that can be
/// reproduced; the whole sweep is timed, because "no hang" is a claim as much as
/// "no panic" is, and a length-driven loop that trusts the file is how both are
/// lost at once.
#[test]
fn a_damaged_project_is_refused_or_partly_read_but_never_panics() {
    let whole = std::fs::read(fixtures().join("fixture.aep")).expect("the golden .aep");
    let started = std::time::Instant::now();

    for seed in 0_u64..64 {
        // One tiny deterministic generator, so the sweep is the same on every
        // machine and a failing seed can be run again on its own.
        let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };

        let mut damaged = whole.clone();
        match seed % 4 {
            // Cut the file short somewhere past the header.
            0 => damaged.truncate(16 + next() % whole.len()),
            // Flip one byte anywhere.
            1 => {
                let at = next() % damaged.len();
                damaged[at] ^= 0xFF;
            }
            // Overwrite four bytes with an enormous size word — the shape of
            // the attack the container walk's one bounds check exists for.
            2 => {
                let at = next() % (damaged.len() - 4);
                damaged[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
            }
            // Zero a run, which is how a partly-written file arrives.
            _ => {
                let at = next() % damaged.len();
                let end = (at + 1 + next() % 4096).min(damaged.len());
                damaged[at..end].fill(0);
            }
        }

        let outcome = std::panic::catch_unwind(|| lumit_import::aep::parse_capture(&damaged));
        let Ok(outcome) = outcome else {
            panic!("seed {seed} panicked; a malformed byte must be an error, not a crash");
        };
        // Either answer is correct — what matters is that it is an answer, and
        // that a failure is one of the parser's own named errors rather than a
        // string from somewhere unknown.
        if let Err(error) = outcome {
            assert!(
                !error.to_string().is_empty(),
                "seed {seed}: the refusal says why"
            );
        }
    }

    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "sixty-four damaged parses took {:?} — a length the file declared is \
         being trusted somewhere",
        started.elapsed()
    );
}

/// A comp and an item are joined only by an id, so this is worth its own check:
/// the nested comp is reached from its precomp layer, and both precomp layers
/// in the fixture point at the same one.
#[test]
fn a_precomp_layer_reaches_its_comp_through_the_item_id() {
    let capture = parsed().capture;
    let nested: Vec<&Comp> = capture.comps.iter().collect();
    let inner = nested
        .iter()
        .find(|comp| comp.layers.len() == 1)
        .expect("the inner comp holds one layer");
    let item: &Item = capture
        .items
        .iter()
        .find(|item| item.id == inner.id)
        .expect("the inner comp has an item row carrying its name");
    assert_eq!(item.kind.as_deref(), Some("comp"));

    let pointing: Vec<&Layer> = capture
        .comps
        .iter()
        .flat_map(|comp| &comp.layers)
        .filter(|layer| layer.kind.as_deref() == Some("precomp"))
        .collect();
    assert_eq!(pointing.len(), 2, "the fixture has two precomp layers");
    for layer in pointing {
        assert_eq!(
            layer.source_id, inner.id,
            "a precomp layer names its comp by id, never by name"
        );
    }
}

/// **The real project, renamed `.zip`, still opens as a project (K-418).**
///
/// The picker offers `.aep` and `.zip` in one filter, so `open_ae` routes on
/// the file's first four bytes and never on its name. The lib's own unit test
/// proves the routing with stubs; this proves it with the golden file, which is
/// the case that would actually cost a user their import — a project renamed on
/// the way through a mail server must import, not be refused for its extension.
#[test]
fn the_golden_project_opens_under_the_wrong_extension() {
    let temp = tempfile::tempdir().expect("a temp dir");
    let renamed = temp.path().join("fixture.zip");
    std::fs::copy(fixtures().join("fixture.aep"), &renamed).expect("the copy");

    let bundle = lumit_import::open_ae(&renamed).expect("the bytes say .aep, so it parses");
    assert_eq!(bundle.source, BundleSource::Aep);
    assert_eq!(
        bundle.capture,
        parsed().capture,
        "the same parse, byte for byte"
    );
}
