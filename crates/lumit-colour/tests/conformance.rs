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
use lumit_colour::Chain;

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

#[test]
#[ignore = "pending: fixtures/aces-1.2.fixture, generated offline from the legacy ACES 1.2 config with the reference OpenColorIO library (docs/impl/ocio.md §7.1)"]
fn the_legacy_aces_config_matches_the_reference() {
    unreachable!("no fixture data yet");
}

#[test]
#[ignore = "pending: fixtures/aces-cg.fixture and the vendored builtin bakes, from one reference OpenColorIO run (docs/impl/ocio.md §7.1, §4.1)"]
fn the_aces_cg_config_matches_the_reference() {
    unreachable!("no fixture data yet");
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
