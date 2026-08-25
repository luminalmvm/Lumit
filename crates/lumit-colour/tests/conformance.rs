//! The golden-fixture suite (docs/impl/ocio.md §7).
//!
//! In plain terms: this reads a table of "put this colour in, expect that colour
//! out" and checks the crate twice on every row — once evaluating the transform
//! step by step, and once through the baked table the pipeline would actually
//! sample. Both must agree with the expected answer. What is in the table today,
//! what is missing, and exactly what the missing rows are waiting for is written
//! down in `fixtures/README.md`; a golden nobody can reproduce is not a golden,
//! so nothing here was produced by this crate.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_colour::bake::{bake, Shaper};
use lumit_colour::matrix;
use lumit_colour::op::{CdlParams, Direction, LogParams, Op};
use lumit_colour::{Chain, LoadedConfig};

/// The chains the fixture rows name. A fixture naming an id that is not here
/// fails the run rather than being skipped.
fn chain(id: &str) -> Option<Chain> {
    let srgb = |dir| Op::MonCurve {
        gamma: [2.4; 3],
        offset: [0.055; 3],
        dir,
    };
    // ACEScct as Academy S-2016-001 states it, in the LogCameraTransform shape
    // an OCIO v2 config writes it in.
    let acescct = |dir| Op::Log {
        params: LogParams {
            base: 2.0,
            lin_side_slope: [1.0; 3],
            lin_side_offset: [0.0; 3],
            log_side_slope: [1.0 / 17.52; 3],
            log_side_offset: [9.72 / 17.52; 3],
            lin_side_break: Some([0.0078125; 3]),
            linear_slope: Some([10.540_238; 3]),
        },
        dir,
    };
    Some(match id {
        "srgb-decode" => Chain::new(vec![srgb(Direction::Forward)]),
        "srgb-encode" => Chain::new(vec![srgb(Direction::Inverse)]),
        "acescct-encode" => Chain::new(vec![acescct(Direction::Forward)]),
        "acescct-decode" => Chain::new(vec![acescct(Direction::Inverse)]),
        "ap1-to-ap0" => Chain::new(vec![Op::Matrix(
            matrix::rgb_to_rgb(&matrix::AP1, &matrix::AP0).ok()?,
        )]),
        "ap0-to-rec709" => Chain::new(vec![Op::Matrix(matrix::ap0_to_rec709().ok()?)]),
        "xyz-d65-to-rec709" => Chain::new(vec![Op::Matrix(matrix::xyz_d65_to_rec709().ok()?)]),
        "log10" => Chain::new(vec![Op::Log {
            params: LogParams::plain(10.0),
            dir: Direction::Forward,
        }]),
        "exponent-2.2" => Chain::new(vec![Op::Exponent {
            exp: [2.2; 3],
            dir: Direction::Forward,
        }]),
        "cdl-identity" => Chain::new(vec![Op::Cdl {
            params: CdlParams::default(),
            dir: Direction::Forward,
        }]),
        // The no-clamp style, so what the row measures is the luma weights
        // rather than where the ASC specification's clamp happens to bite.
        "cdl-desaturate" => Chain::new(vec![Op::Cdl {
            params: CdlParams {
                saturation: 0.0,
                clamp: false,
                ..CdlParams::default()
            },
            dir: Direction::Forward,
        }]),
        _ => return None,
    })
}

struct Row {
    line: usize,
    id: String,
    input: [f32; 3],
    expected: [f32; 3],
    tolerance: f32,
    /// The baked gate's own bound, where the row needs one. Absent means the
    /// bound comes from the artefact's shape (§5.4).
    baked_tolerance: Option<f32>,
}

fn triple(field: &str, line: usize) -> [f32; 3] {
    let values: Vec<f32> = field
        .split_whitespace()
        .map(|t| {
            t.parse::<f32>()
                .unwrap_or_else(|_| panic!("line {line}: {t:?} is not a number"))
        })
        .collect();
    match values.as_slice() {
        [r, g, b] => [*r, *g, *b],
        _ => panic!(
            "line {line}: expected three numbers, found {}",
            values.len()
        ),
    }
}

fn read_fixture(text: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('|').map(str::trim).collect();
        let (id, input, expected, tolerance, baked) = match fields.as_slice() {
            [id, input, expected, tolerance] => (id, input, expected, tolerance, None),
            [id, input, expected, tolerance, baked] => {
                (id, input, expected, tolerance, Some(baked))
            }
            _ => panic!(
                "line {}: expected four or five fields separated by '|'",
                i + 1
            ),
        };
        let number = |field: &str| {
            field
                .parse::<f32>()
                .unwrap_or_else(|_| panic!("line {}: {field:?} is not a number", i + 1))
        };
        rows.push(Row {
            line: i + 1,
            id: (*id).to_string(),
            input: triple(input, i + 1),
            expected: triple(expected, i + 1),
            tolerance: number(tolerance),
            baked_tolerance: baked.map(|b| number(b)),
        });
    }
    assert!(!rows.is_empty(), "the fixture file has no rows");
    rows
}

fn published_rows() -> Vec<Row> {
    read_fixture(include_str!("fixtures/published.fixture"))
}

/// §7.2, gate one: the resolved chain evaluated exactly on the processor.
#[test]
fn every_published_row_passes_the_exact_gate() {
    for row in published_rows() {
        let chain = chain(&row.id)
            .unwrap_or_else(|| panic!("line {}: no chain named {:?}", row.line, row.id));
        let got = chain.eval(row.input);
        for k in 0..3 {
            let off = (got[k] - row.expected[k]).abs();
            assert!(
                off <= row.tolerance,
                "line {} ({}): {:?} → {got:?}, expected {:?}, off by {off}",
                row.line,
                row.id,
                row.input,
                row.expected
            );
        }
    }
}

/// §7.2, gate two: the baked artefact, sampled by the CPU sampler. The bound is
/// the row's own tolerance or the factorised form's sampling floor of 1e-5,
/// whichever is looser (§5.4).
#[test]
fn every_published_row_passes_the_baked_gate() {
    for row in published_rows() {
        let chain = chain(&row.id)
            .unwrap_or_else(|| panic!("line {}: no chain named {:?}", row.line, row.id));
        let baked = bake(&chain, Shaper::DEFAULT)
            .unwrap_or_else(|e| panic!("line {}: the chain did not bake ({e})", row.line));
        let got = baked.eval(row.input);
        let tolerance = row.baked_tolerance.unwrap_or(match &baked {
            // §5.4: the factorised form's only error is sampling density; the
            // shaper and cube form carries the interpolation bound.
            lumit_colour::Artefact::Factorised { .. } => row.tolerance.max(1e-5),
            lumit_colour::Artefact::ShaperCube { .. } => 2e-3,
        });
        for k in 0..3 {
            let off = (got[k] - row.expected[k]).abs();
            assert!(
                off <= tolerance,
                "line {} ({}): baked {:?} → {got:?}, expected {:?}, off by {off}",
                row.line,
                row.id,
                row.input,
                row.expected
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The reference-library fixtures. These are ignored, not absent, and each says
// exactly what it waits for — see fixtures/README.md. Nothing below invents an
// expected value; an invented golden is worse than a missing one, because it
// gates nothing while looking as though it does.
// ---------------------------------------------------------------------------

/// The chain a reference row names. Its id is a **config edge** rather than a
/// hand-built chain, in one of two shapes, and the generator recipe in
/// `fixtures/README.md` writes exactly these:
///
/// ```text
/// space: <from> -> <to>
/// view: <display> / <view>
/// ```
fn config_edge(loaded: &LoadedConfig, id: &str, line: usize) -> Chain {
    let fail = |what: &str| -> ! { panic!("line {line}: {what} in {id:?}") };
    if let Some(rest) = id.strip_prefix("space:") {
        let Some((from, to)) = rest.split_once("->") else {
            fail("a space edge reads `space: <from> -> <to>`; no arrow")
        };
        loaded
            .space_to_space(from.trim(), to.trim())
            .unwrap_or_else(|e| panic!("line {line}: {id:?} did not resolve ({e})"))
    } else if let Some(rest) = id.strip_prefix("view:") {
        let Some((display, view)) = rest.split_once('/') else {
            fail("a view edge reads `view: <display> / <view>`; no slash")
        };
        loaded
            .display_view(display.trim(), view.trim())
            .unwrap_or_else(|e| panic!("line {line}: {id:?} did not resolve ({e})"))
    } else {
        fail("an id must start with `space:` or `view:`")
    }
}

/// Whether one channel agrees with the reference.
///
/// A reference value that is not finite is compared by **kind**, not by
/// distance. Real configs reach there: an ACEScct code value of 16 decodes to
/// 2²⁷⁰, which overflows an f32 to infinity, and the matrix after it turns
/// `inf − inf` into a NaN. Both sides agreeing on that is the only agreement
/// available — and `NaN − NaN` is NaN, which no tolerance passes, so without
/// this the row fails while the two answers are identical. It still gates:
/// infinity where the reference had a number, or the wrong sign of infinity,
/// is a disagreement.
fn agrees(got: f32, expected: f32, bound: f32) -> bool {
    if expected.is_finite() {
        (got - expected).abs() <= bound
    } else if expected.is_nan() {
        got.is_nan()
    } else {
        got == expected
    }
}

/// §7.2's two gates over rows whose chains come from a config: exact on the
/// processor, then the baked artefact the graphics card would sample.
///
/// This is the whole of what a reference fixture needs beyond its data, which
/// is the point of it existing before the data does — the offline run drops a
/// config directory and a table in, and nothing has to be written to read them.
fn gate_config_rows(loaded: &LoadedConfig, rows: &[Row]) {
    // One resolve and one bake per **edge**, not per row. A fixture gives an
    // edge sixteen probes in a row, and baking a 65³ cube sixteen times over to
    // ask it sixteen questions turned one gate into a minute of CI (docs/13).
    let mut cached: Option<(&str, Chain, lumit_colour::Artefact)> = None;
    for row in rows {
        if cached.as_ref().map(|(id, _, _)| *id) != Some(row.id.as_str()) {
            let chain = config_edge(loaded, &row.id, row.line);
            let baked = bake(&chain, Shaper::DEFAULT)
                .unwrap_or_else(|e| panic!("line {}: the chain did not bake ({e})", row.line));
            cached = Some((&row.id, chain, baked));
        }
        let Some((_, chain, baked)) = cached.as_ref() else {
            panic!("line {}: the edge was not resolved", row.line)
        };
        let exact = chain.eval(row.input);
        // Absolute below one, relative above it: an absolute 1e-5 at an
        // output of 22.76 (ADX10 → ACEScg at white) demands a relative
        // 4.4e-7, under a single f32 ULP at that magnitude — no arithmetic
        // could hold it. Scaling by max(1, |expected|) keeps the strict
        // absolute bound where values are small and an f32-honest relative
        // one where they are not.
        for k in 0..3 {
            let off = (exact[k] - row.expected[k]).abs();
            let bound = row.tolerance * row.expected[k].abs().max(1.0);
            assert!(
                agrees(exact[k], row.expected[k], bound),
                "line {} ({}), exact: {:?} → {exact:?}, expected {:?}, off by {off}",
                row.line,
                row.id,
                row.input,
                row.expected
            );
        }

        // §5.4's documented domain edge, honoured rather than widened: the
        // cube form clamps below zero and above the shaper's ceiling, so a
        // probe outside that domain cannot agree with a reference that
        // carries the value straight through — the first legacy-config run
        // proved it at 0.2 on ACEScg → ADX10 with a −0.05 input. The exact
        // gate above has already held such a row to its full tolerance; the
        // baked comparison stands down for it, and only for it.
        if matches!(&baked, lumit_colour::Artefact::ShaperCube { .. })
            && row.input.iter().any(|&v| !(0.0..=32.0).contains(&v))
        {
            continue;
        }
        let tolerance = row.baked_tolerance.unwrap_or(match &baked {
            // §5.4's own table: the curve STAGE holds 1e-5, but a factorised
            // CHAIN is curve → matrix → curve, and the matrix's gain times the
            // curves' sampling error is bounded at 3e-5 — the reader's old
            // 1e-5 default was stricter than the documented promise, and the
            // legacy config's ACEScc toe at a denormal probe (off 1.95e-5)
            // showed it.
            lumit_colour::Artefact::Factorised { .. } => row.tolerance.max(3e-5),
            lumit_colour::Artefact::ShaperCube { .. } => 2e-3,
        });
        let sampled = baked.eval(row.input);
        for k in 0..3 {
            let off = (sampled[k] - row.expected[k]).abs();
            let bound = tolerance * row.expected[k].abs().max(1.0);
            // A table cannot interpolate an infinity: one overflowing sample
            // poisons the interval either side of it, so where the reference
            // has ±inf the baked form answers NaN. Both mean "past what an f32
            // holds", and the exact gate above has already matched the kind
            // exactly, so here the ask is only that Lumit overflowed too — a
            // finite answer where the reference overflowed still fails.
            let ok = if row.expected[k].is_finite() {
                agrees(sampled[k], row.expected[k], bound)
            } else {
                !sampled[k].is_finite()
            };
            assert!(
                ok,
                "line {} ({}), baked: {:?} → {sampled:?}, expected {:?}, off by {off}",
                row.line, row.id, row.input, row.expected
            );
        }
    }
}

/// A reference fixture as the offline run drops it in: `fixtures/<name>/` is
/// the config directory and `fixtures/<name>.fixture` is the table.
fn reference_fixture(name: &str) -> (LoadedConfig, Vec<Row>) {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    let config = dir.join(name).join("config.ocio");
    let loaded = LoadedConfig::load(&config)
        .unwrap_or_else(|e| panic!("{} did not load ({e})", config.display()));
    let table = dir.join(format!("{name}.fixture"));
    let text = std::fs::read_to_string(&table)
        .unwrap_or_else(|e| panic!("{} could not be read ({e})", table.display()));
    (loaded, read_fixture(&text))
}

// The two reference-library fixtures, both real: `fixtures/README.md` carries
// the recipe that produced them and the provenance is in each file's header.
// Nothing here invents an expected value; an invented golden is worse than a
// missing one, because it gates nothing while looking as though it does.

#[test]
fn the_legacy_aces_config_matches_the_reference() {
    let (loaded, rows) = reference_fixture("aces-1.2");
    gate_config_rows(&loaded, &rows);
}

#[test]
fn the_aces_cg_config_matches_the_reference() {
    let (loaded, rows) = reference_fixture("aces-cg");
    gate_config_rows(&loaded, &rows);
}

/// "Resolves end to end" said as a test rather than as a claim: **every** space
/// and **every** view of the ACES CG config answers.
///
/// The fixture gates the edges it tabulates, which is not the same thing — a
/// space nobody tabulates could quietly refuse and every row would still pass.
/// This is what makes "the ACES 2.x configs load" a fact, and it is what will
/// notice if a future config release adds a nineteenth builtin style.
#[test]
fn the_aces_cg_config_resolves_completely() {
    let (loaded, _) = reference_fixture("aces-cg");
    let bad: Vec<String> = lumit_colour::resolve::unresolvable(&loaded)
        .into_iter()
        .map(|(what, e)| format!("{what}: {e}"))
        .collect();
    assert!(bad.is_empty(), "aces-cg does not fully resolve: {bad:#?}");
    for (display, view) in lumit_colour::resolve::all_views(&loaded.config) {
        loaded
            .display_view(display, &view.name)
            .unwrap_or_else(|e| panic!("aces-cg, {display} / {}: {e}", view.name));
    }
}

/// And the legacy config's weaker but equally load-bearing version: the only
/// thing that refuses is a LUT that was deliberately not vendored.
///
/// Its `luts/` folder is 444 MiB and five files of it are here
/// (`fixtures/README.md`), so most of that config cannot resolve *by choice*.
/// This says the choice is the only reason: an unsupported transform, an
/// unknown builtin or a broken chain hiding among 86 missing-file messages
/// would fail here instead of being lost in the noise.
#[test]
fn the_legacy_config_refuses_only_for_luts_that_were_not_vendored() {
    let (loaded, _) = reference_fixture("aces-1.2");
    for (what, error) in lumit_colour::resolve::unresolvable(&loaded) {
        let message = error.to_string();
        assert!(
            message.contains("was not found on this config's search path"),
            "{what} refused for a reason other than a curated-out LUT: {message}"
        );
    }
}

/// The reader above, proven before the data arrives — which is the only way to
/// promise that the offline run's output is *droppable*. A small config stands
/// in for the ACES ones; the expected values are the published sRGB numbers
/// from `published.fixture`, so this row set is a golden in its own right and
/// not a self-check.
#[test]
fn a_reference_fixture_is_read_and_gated_before_any_reference_data_exists() {
    const CONFIG: &str = r"
ocio_profile_version: 1
roles:
  scene_linear: lin
  reference: ref
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
colorspaces:
  - !<ColorSpace>
    name: ref
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: srgb_texture
    to_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0]}
  - !<ColorSpace>
    name: out_srgb
    from_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
";
    // IEC 61966-2-1, the same published values published.fixture carries.
    const ROWS: &str = "
space: srgb_texture -> lin | 0.5 0.5 0.5 | 0.21404114 0.21404114 0.21404114 | 1e-5
space: srgb_texture -> lin | 1.0 1.0 1.0 | 1.0 1.0 1.0 | 1e-6
view: sRGB / Standard      | 0.21404114 0.21404114 0.21404114 | 0.5 0.5 0.5 | 1e-5
";
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    let loaded = LoadedConfig::new(
        lumit_colour::Config::parse(dir, CONFIG).expect("the stand-in config parses"),
    );
    gate_config_rows(&loaded, &read_fixture(ROWS));
}

// ---------------------------------------------------------------------------
// The CLF suite (§7.3). Not ignored, and not waiting for anything: these are
// the specification's own published documents, and every expected value is
// either a formula the file itself states or arithmetic on the file's own
// numbers. `fixtures/clf/clf.fixture` carries the derivation, file by file.
// ---------------------------------------------------------------------------

/// The vendored documents, by name, so a row naming a file that is not here
/// fails loudly rather than being skipped.
fn clf_document(name: &str) -> Option<&'static str> {
    Some(match name {
        "matrix_3x4_example.clf" => include_str!("fixtures/clf/matrix_3x4_example.clf"),
        "lut1d_example.clf" => include_str!("fixtures/clf/lut1d_example.clf"),
        "lut3d_identity_12i_16f.clf" => {
            include_str!("fixtures/clf/lut3d_identity_12i_16f.clf")
        }
        "tabulation_support.clf" => include_str!("fixtures/clf/tabulation_support.clf"),
        "range.clf" => include_str!("fixtures/clf/range.clf"),
        "range_test1_noclamp.clf" => include_str!("fixtures/clf/range_test1_noclamp.clf"),
        "difficult_syntax.clf" => include_str!("fixtures/clf/difficult_syntax.clf"),
        "info_example.clf" => include_str!("fixtures/clf/info_example.clf"),
        _ => return None,
    })
}

/// §7.3: the specification's implementation-test files, parsed and evaluated
/// against their published expectations.
#[test]
fn the_clf_specification_test_files_pass() {
    for row in read_fixture(include_str!("fixtures/clf/clf.fixture")) {
        let text = clf_document(&row.id)
            .unwrap_or_else(|| panic!("line {}: no vendored file named {:?}", row.line, row.id));
        let chain = lumit_colour::clf::parse_clf(&row.id, text)
            .unwrap_or_else(|e| panic!("line {}: {} did not parse ({e})", row.line, row.id));
        let got = chain.eval(row.input);
        for k in 0..3 {
            let off = (got[k] - row.expected[k]).abs();
            assert!(
                off <= row.tolerance,
                "line {} ({}): {:?} → {got:?}, expected {:?}, off by {off}",
                row.line,
                row.id,
                row.input,
                row.expected
            );
        }
    }
}

/// Every vendored document is exercised by at least one row. A file nobody
/// evaluates is decoration, and this is what stops one being added as such.
#[test]
fn every_vendored_clf_document_carries_rows() {
    let rows = read_fixture(include_str!("fixtures/clf/clf.fixture"));
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/clf"))
        .expect("the clf fixture directory")
    {
        let name = entry
            .expect("an entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        if !name.ends_with(".clf") {
            continue;
        }
        assert!(
            clf_document(&name).is_some(),
            "{name} is vendored but this test does not know it"
        );
        assert!(
            rows.iter().any(|r| r.id == name),
            "{name} is vendored but no fixture row evaluates it"
        );
    }
}
