//! `BuiltinTransform` — the transforms a config names but does not describe.
//!
//! In plain terms: most of a config is data — matrices, curves, tables. But the
//! OCIO v2 ACES configs express their output transforms as *names* instead:
//! "ACES-OUTPUT — … SDR-VIDEO" means "whatever the reference implementation's
//! code does under that name". There is nothing in the file to read.
//!
//! Lumit answers that in two tiers, and refuses everything in neither:
//!
//! 1. **Implemented directly** — styles that are a documented composition of a
//!    matrix and a curve, written out here and gated by the same fixtures as
//!    everything else. Small, and each one earns its place by being checkable.
//! 2. **Vendored reference bakes** — high-resolution artefacts generated offline
//!    with the reference library and checked in like any golden data, with the
//!    library version and generation script recorded in the file's header. They
//!    are not compiled into the binary — at 47 MiB that would ride in every
//!    build — but read at runtime from a `colour/` data directory shipped
//!    beside the executable, falling back to the crate's own `vendored/`
//!    directory in a development checkout. A style whose file is absent
//!    refuses by name, exactly as if it had never been vendored.
//!
//! A style in neither tier refuses the config **by name**. Note the happy
//! accident of history: the *legacy* ACES configs (1.0.3 and 1.2, still the most
//! widespread) are pure config data — matrices, logs and `.spi1d`/`.spi3d`
//! files — and need no built-ins at all.

use crate::bake::VendoredArtefact;
use crate::error::{ColourError, Result};
use crate::matrix;
use crate::op::{Chain, Direction, LogParams, Negatives, Op};
use std::path::{Path, PathBuf};

/// Tier one: the styles implemented directly. Listed so a caller can say what
/// is supported without guessing at a refusal.
pub const IMPLEMENTED: [&str; 14] = [
    "IDENTITY",
    "pass_thru",
    "ACEScc_to_ACES2065-1",
    "ACEScct_to_ACES2065-1",
    "ACEScg_to_ACES2065-1",
    "UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD",
    "DISPLAY - CIE-XYZ-D65_to_sRGB - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709 - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709 - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65 - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_DisplayP3",
    "DISPLAY - CIE-XYZ-D65_to_DisplayP3-HDR",
    "DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ",
    "DISPLAY - CIE-XYZ-D65_to_ST2084-P3-D65",
];

/// Tier two: the styles answered by a vendored reference bake rather than by
/// code. Listed for the same reason [`IMPLEMENTED`] is.
pub const VENDORED: [&str; 5] = [
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-REC709_2.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-P3-D65_2.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-1000nit-P3-D65_2.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-1000nit-REC2020_2.0",
    "ACES-LMT - ACES 1.3 Reference Gamut Compression",
];

/// ACEScct's log curve, exactly as Academy S-2016-001 states it: a straight
/// segment below 0.0078125 with slope 10.5402377, a base-2 log above it.
fn acescct_params() -> LogParams {
    LogParams {
        base: 2.0,
        lin_side_slope: [1.0; 3],
        lin_side_offset: [0.0; 3],
        log_side_slope: [1.0 / 17.52; 3],
        log_side_offset: [9.72 / 17.52; 3],
        lin_side_break: Some([0.0078125; 3]),
        linear_slope: Some([10.540_238; 3]),
    }
}

/// ACEScc's log curve, Academy S-2014-003: base-2 log above 2⁻¹⁵, and below it
/// a straight segment that reaches the same place — written as OCIO writes it,
/// with the break at the linear value the two halves meet at.
fn acescc_params() -> LogParams {
    LogParams {
        base: 2.0,
        lin_side_slope: [1.0; 3],
        lin_side_offset: [0.0; 3],
        log_side_slope: [1.0 / 17.52; 3],
        log_side_offset: [9.72 / 17.52; 3],
        // Below 2⁻¹⁶ ACEScc is a straight line through (0, (log2(2⁻¹⁶) + 9.72)
        // / 17.52); the break is where it meets the log branch, at 2⁻¹⁵.
        lin_side_break: Some([0.000_030_517_578; 3]),
        linear_slope: Some([(2.0_f32).powi(-16) / (17.52 * (2.0_f32).powi(-16)); 3]),
    }
}

/// Where the vendored artefacts live at runtime, in the order a build looks:
/// `data/colour/` beside the executable (a shipped Windows or Linux build),
/// `../Resources/colour/` from it (a shipped macOS bundle, where nothing but
/// executables may sit in `Contents/MacOS`), and the crate's own `vendored/`
/// directory (a development checkout, which is also what the tests read).
/// `None` when no directory exists — every vendored style then refuses by
/// name. Same search shape as the export done-sound's, the one other data
/// file shipped beside the binary.
fn vendored_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let beside = exe_dir.as_ref().map(|d| d.join("data").join("colour"));
    let resources = exe_dir
        .as_ref()
        .and_then(|d| d.parent())
        .map(|contents| contents.join("Resources").join("colour"));
    let dev = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("vendored"));
    [beside, resources, dev]
        .into_iter()
        .flatten()
        .find(|d| d.is_dir())
}

/// Tier two: a reference bake vendored by style name.
///
/// The style name is the file name — `<style>.artefact` in [`vendored_dir`] —
/// but only for the styles [`VENDORED`] lists: a name from a config never
/// reaches the filesystem unless this registry says so. Each file carries its
/// own provenance header — which library version, which config, which day —
/// and [`VendoredArtefact::from_text`] refuses a file that does not; a file
/// that is absent, unreadable or refused leaves the style refusing by name.
fn vendored(style: &str) -> Option<VendoredArtefact> {
    if !VENDORED.contains(&style) {
        return None;
    }
    let path = vendored_dir()?.join(format!("{style}.artefact"));
    let text = std::fs::read_to_string(path).ok()?;
    VendoredArtefact::from_text(style, &text).ok()
}

/// A vendored artefact as chain steps.
///
/// A shaper-and-cube artefact *is* a chain: the lg2 shaper is a log curve with
/// a lin-side offset, and the cube is a 3D table over 0–1. Saying so here means
/// a vendored style composes with everything else — a view is an output
/// transform *then* a display encoding — instead of needing its own execution
/// path. The chain is re-baked downstream onto the same 65-point grid with the
/// same shaper, so the samples land back on the points they came from.
fn vendored_chain(style: &str) -> Option<Chain> {
    let crate::bake::Artefact::ShaperCube { shaper, cube } = vendored(style)?.artefact else {
        return None;
    };
    let crate::bake::Shaper::Lg2 {
        min_log2,
        max_log2,
        offset,
    } = shaper
    else {
        return None;
    };
    let span = max_log2 - min_log2;
    Some(Chain::new(vec![
        Op::Log {
            params: LogParams {
                base: 2.0,
                lin_side_slope: [1.0; 3],
                lin_side_offset: [offset; 3],
                log_side_slope: [1.0 / span; 3],
                log_side_offset: [-min_log2 / span; 3],
                lin_side_break: None,
                linear_slope: None,
            },
            dir: Direction::Forward,
        },
        Op::Lut3d { cube },
    ]))
}

/// One display encoding: CIE XYZ D65 into a display's primaries, then its
/// transfer function run backwards (light to code value), mirrored about zero.
///
/// Every one of the ACES v2 display encodings has this shape, and the reference
/// library's own decomposition of each style is literally these two steps —
/// which is why they are tier one and not bakes.
fn display(to: &matrix::Chromaticities, curve: Op) -> Result<Chain> {
    Ok(Chain::new(vec![
        Op::Matrix(matrix::xyz_d65_to(to)?),
        Op::Negatives {
            style: Negatives::Mirror,
            curve: Box::new(curve),
        },
    ]))
}

/// A plain display gamma, run backwards.
fn gamma(exp: f32) -> Op {
    Op::Exponent {
        exp: [exp; 3],
        dir: Direction::Inverse,
    }
}

/// The sRGB piecewise curve, run backwards.
fn srgb_curve() -> Op {
    Op::MonCurve {
        gamma: [2.4; 3],
        offset: [0.055; 3],
        dir: Direction::Inverse,
    }
}

/// Resolve a `BuiltinTransform` style into a chain, or refuse it by name.
pub fn resolve(style: &str, dir: Direction) -> Result<Chain> {
    let forward = match style {
        // `pass_thru` is the identity under a second name: OCIO's own word for
        // a view transform that leaves the reference space alone.
        "IDENTITY" | "pass_thru" => Chain::identity(),
        // The ACEScct log undone, then AP1 primaries into AP0.
        "ACEScct_to_ACES2065-1" => Chain::new(vec![
            Op::Log {
                params: acescct_params(),
                dir: Direction::Inverse,
            },
            Op::Matrix(matrix::rgb_to_rgb(&matrix::AP1, &matrix::AP0)?),
        ]),
        "ACEScc_to_ACES2065-1" => Chain::new(vec![
            Op::Log {
                params: acescc_params(),
                dir: Direction::Inverse,
            },
            Op::Matrix(matrix::rgb_to_rgb(&matrix::AP1, &matrix::AP0)?),
        ]),
        "DISPLAY - CIE-XYZ-D65_to_sRGB - MIRROR NEGS" => display(&matrix::REC709, srgb_curve())?,
        "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709 - MIRROR NEGS" => {
            display(&matrix::REC709, gamma(2.2))?
        }
        // Rec.1886's EOTF is a pure 2.4 power once its black level is zero,
        // which is what a display encoding assumes.
        "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709 - MIRROR NEGS" => {
            display(&matrix::REC709, gamma(2.4))?
        }
        "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65 - MIRROR NEGS" => {
            display(&matrix::P3_D65, gamma(2.6))?
        }
        // Apple's Display P3 is the P3 primaries with the sRGB curve; the HDR
        // variant differs only in how bright the container is taken to be, and
        // the reference library resolves both to the same two steps.
        "DISPLAY - CIE-XYZ-D65_to_DisplayP3" | "DISPLAY - CIE-XYZ-D65_to_DisplayP3-HDR" => {
            display(&matrix::P3_D65, srgb_curve())?
        }
        "DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ" => display(
            &matrix::REC2020,
            Op::Pq {
                dir: Direction::Forward,
            },
        )?,
        "DISPLAY - CIE-XYZ-D65_to_ST2084-P3-D65" => display(
            &matrix::P3_D65,
            Op::Pq {
                dir: Direction::Forward,
            },
        )?,
        "ACEScg_to_ACES2065-1" => Chain::new(vec![Op::Matrix(matrix::rgb_to_rgb(
            &matrix::AP1,
            &matrix::AP0,
        )?)]),
        "UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD" => {
            let to_xyz = matrix::rgb_to_xyz(&matrix::AP0)?;
            let adapt = matrix::bradford(matrix::AP0.white, matrix::REC709.white)?;
            let mut m = [0.0_f64; 9];
            for row in 0..3 {
                for col in 0..3 {
                    let mut acc = 0.0_f64;
                    for k in 0..3 {
                        acc += adapt[row * 3 + k] * to_xyz[k * 3 + col];
                    }
                    m[row * 3 + col] = acc;
                }
            }
            Chain::new(vec![Op::Matrix(matrix::from_3x3(&m))])
        }
        // Tier two: a vendored bake, read as chain steps. A style in neither
        // tier refuses by name.
        other => match vendored_chain(other) {
            Some(chain) => chain,
            None => {
                return Err(ColourError::UnsupportedBuiltin {
                    style: other.to_string(),
                })
            }
        },
    };
    match dir {
        Direction::Forward => Ok(forward),
        Direction::Inverse => forward.inverted(style),
    }
}

/// The vendored reference bake for a style, if one is checked in. Callers that
/// can execute an artefact directly (the display path) ask here first; callers
/// that need a chain use [`resolve`].
#[must_use]
pub fn vendored_artefact(style: &str) -> Option<VendoredArtefact> {
    vendored(style)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn identity_changes_nothing_either_way() {
        for dir in [Direction::Forward, Direction::Inverse] {
            let chain = resolve("IDENTITY", dir).expect("implemented");
            assert!(chain.is_identity());
        }
    }

    #[test]
    fn acescct_to_aces_hits_the_published_middle_grey() {
        // ACEScct 0.4135884 is linear 0.18 in AP1; through the AP1→AP0 matrix a
        // neutral stays neutral, so the answer is 0.18 on all three.
        let chain = resolve("ACEScct_to_ACES2065-1", Direction::Forward).expect("implemented");
        let got = chain.eval([0.4135884; 3]);
        assert!(close(got, [0.18; 3], 1e-4), "{got:?}");
    }

    #[test]
    fn acescg_to_aces_takes_white_to_white() {
        let chain = resolve("ACEScg_to_ACES2065-1", Direction::Forward).expect("implemented");
        assert!(close(chain.eval([1.0; 3]), [1.0; 3], 1e-5));
    }

    #[test]
    fn the_ap0_to_xyz_utility_lands_on_d65_white() {
        let chain = resolve("UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD", Direction::Forward)
            .expect("implemented");
        let got = chain.eval([1.0; 3]);
        // D65 white in XYZ, from the (0.3127, 0.3290) chromaticity pair.
        assert!(close(got, [0.950_456, 1.0, 1.089_058], 1e-3), "{got:?}");
    }

    #[test]
    fn an_implemented_style_round_trips_through_its_inverse() {
        let there = resolve("ACEScct_to_ACES2065-1", Direction::Forward).expect("implemented");
        let back = resolve("ACEScct_to_ACES2065-1", Direction::Inverse).expect("implemented");
        let c = [0.2, 0.45, 0.7];
        assert!(close(back.eval(there.eval(c)), c, 1e-4));
    }

    #[test]
    fn an_output_style_with_no_bake_refuses_by_name() {
        // The ACES 1.x output transform: not in `VENDORED`, so it refuses —
        // which is the promise, rather than a gap being papered over.
        let style = "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0";
        let err = resolve(style, Direction::Forward);
        assert!(
            matches!(&err, Err(ColourError::UnsupportedBuiltin { style: s }) if s == style),
            "{err:?}"
        );
        assert!(vendored_artefact(style).is_none());
    }

    /// The one thing that must be checked at a **saturated, off-neutral grid
    /// point**: a cube read red-fastest when it was written blue-fastest is
    /// the classic silent LUT bug, and it survives every test that only looks
    /// at greys. Grid point (r, g, b) = (10, 40, 60) is deliberately three
    /// different indices, so a transposition cannot hide.
    ///
    /// The expected value is the reference library's own answer at that point,
    /// from the session that produced the artefact (PyOpenColorIO 2.5.2), not
    /// this crate's.
    #[test]
    fn a_vendored_cube_is_read_in_the_order_it_was_written() {
        let style = "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-REC709_2.0";
        let chain = resolve(style, Direction::Forward).expect("vendored");
        let at = |i: f32| (i / 64.0 * 13.0 - 8.0).exp2() - 0.003_906_25;
        let got = chain.eval([at(10.0), at(40.0), at(60.0)]);
        assert!(
            close(got, [0.459_149_96, 0.445_428_5, 1.007_893_3], 1e-5),
            "{got:?}"
        );
    }

    #[test]
    fn every_listed_style_actually_resolves() {
        for style in IMPLEMENTED.iter().chain(&VENDORED) {
            assert!(resolve(style, Direction::Forward).is_ok(), "{style}");
        }
    }

    /// A vendored file that lost its provenance header is not a golden, and
    /// [`VendoredArtefact::from_text`] refuses it — which would leave the style
    /// refusing by name and the config not loading. This says so out loud, so
    /// a truncated or hand-edited artefact fails here rather than in a view.
    #[test]
    fn every_vendored_style_has_an_artefact_with_its_provenance() {
        for style in VENDORED {
            let artefact = vendored(style).unwrap_or_else(|| panic!("{style} is not vendored"));
            for required in [
                "style:",
                "generated by:",
                "generated from:",
                "generated on:",
            ] {
                assert!(
                    artefact.provenance.iter().any(|l| l.starts_with(required)),
                    "{style} has no {required:?} line"
                );
            }
        }
    }
}
