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
//! A style in neither tier refuses **by name**, and only the space or view that
//! names it; the rest of the config stays in force. Note the happy
//! accident of history: the *legacy* ACES configs (1.0.3 and 1.2, still the most
//! widespread) are pure config data — matrices, logs and `.spi1d`/`.spi3d`
//! files — and need no built-ins at all.

use crate::bake::VendoredArtefact;
use crate::error::{ColourError, Result};
use crate::matrix;
use crate::op::{Chain, Direction, GammaLogParams, LogParams, Negatives, Op, RangeParams};
use std::path::{Path, PathBuf};

/// Tier one: the styles implemented directly. Listed so a caller can say what
/// is supported without guessing at a refusal.
pub const IMPLEMENTED: [&str; 29] = [
    "IDENTITY",
    "pass_thru",
    "ACEScc_to_ACES2065-1",
    "ACEScct_to_ACES2065-1",
    "ACEScg_to_ACES2065-1",
    "UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD",
    "UTILITY - ACES-AP1_to_CIE-XYZ-D65_BFD",
    "DISPLAY - CIE-XYZ-D65_to_sRGB",
    "DISPLAY - CIE-XYZ-D65_to_sRGB - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709",
    "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709 - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709",
    "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709 - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.2020",
    "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65",
    "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65 - MIRROR NEGS",
    "DISPLAY - CIE-XYZ-D65_to_DisplayP3",
    "DISPLAY - CIE-XYZ-D65_to_DisplayP3-HDR",
    "DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ",
    "DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit",
    "DISPLAY - CIE-XYZ-D65_to_ST2084-P3-D65",
    "CURVE - LINEAR_to_ST-2084",
    "CURVE - ST-2084_to_LINEAR",
    "CURVE - HLG-OETF",
    "CURVE - HLG-OETF-INVERSE",
    "CURVE - APPLE_LOG_to_LINEAR",
    "APPLE_LOG_to_ACES2065-1",
    "CANON_CLOG2-CGAMUT_to_ACES2065-1",
    "CANON_CLOG3-CGAMUT_to_ACES2065-1",
];

/// Tier two: the styles answered by a vendored reference bake rather than by
/// code. Listed for the same reason [`IMPLEMENTED`] is.
pub const VENDORED: [&str; 8] = [
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-REC709_2.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-P3-D65_2.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-1000nit-P3-D65_2.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-1000nit-REC2020_2.0",
    "ACES-LMT - ACES 1.3 Reference Gamut Compression",
    // The ACES 1.x renderings Blender's config and the PixelManager config
    // name for their ACES views: the SDR pair, and the 1000 nit HDR one.
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO-P3lim_1.1",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-1000nit-15nit-REC2020lim_1.1",
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

/// ITU-R BT.2100's HLG opto-electronic transfer function, scaled the way the
/// reference library scales it: scene light runs 0 to 3 rather than 0 to 1, so
/// 18% grey encodes to HLG 0.42. A square root below a twelfth of the range, a
/// logarithm above it. The constants are the recommendation's own.
fn hlg_params() -> GammaLogParams {
    const A: f64 = 0.17883277;
    const E_MAX: f64 = 3.0;
    let b = (1.0 - 4.0 * A) * E_MAX / 12.0;
    let c = (12.0 / E_MAX).ln() * A + (0.5 - A * (4.0 * A).ln());
    GammaLogParams {
        mirror: 0.0,
        brk: (E_MAX / 12.0) as f32,
        gamma_power: 0.5,
        gamma_slope: (3.0 / E_MAX).sqrt() as f32,
        gamma_offset: 0.0,
        base: std::f32::consts::E,
        log_slope: A as f32,
        log_offset: c as f32,
        lin_offset: (-b) as f32,
    }
}

/// Apple Log, from Apple's own white paper: a squared segment up to code 0.01
/// and a base-2 log above it, odd about the code the curve bottoms out at.
fn apple_log_params() -> GammaLogParams {
    const R_0: f64 = -0.05641088;
    GammaLogParams {
        mirror: R_0 as f32,
        brk: 0.01,
        gamma_power: 2.0,
        gamma_slope: 47.287_112_36_f64 as f32,
        gamma_offset: (-R_0) as f32,
        base: 2.0,
        log_slope: 0.085_504_79,
        log_offset: 0.693_369_45,
        lin_offset: 0.009_640_52,
    }
}

/// Canon Log 2's curve, as one half of a curve odd about code 0.092864125.
/// The 0.9 the specification multiplies by is folded into the lin-side slope.
fn clog2_params() -> LogParams {
    LogParams {
        base: 10.0,
        lin_side_slope: [(87.099_375_f64 / 0.9) as f32; 3],
        lin_side_offset: [1.0; 3],
        log_side_slope: [0.241_360_77; 3],
        log_side_offset: [0.0; 3],
        lin_side_break: None,
        linear_slope: None,
    }
}

/// Canon Log 3's, the same way about code 0.12512219, with the straight segment
/// the specification gives it. The two log-side offsets Canon publishes differ
/// by exactly twice that centre, which is what makes the halves mirror.
fn clog3_params() -> LogParams {
    LogParams {
        base: 10.0,
        lin_side_slope: [(14.98325_f64 / 0.9) as f32; 3],
        lin_side_offset: [1.0; 3],
        log_side_slope: [0.367_268_45; 3],
        // Canon states the upper branch against 0.12240537 rather than against
        // the centre, so the offset is the small difference between the two.
        log_side_offset: [(0.122_405_37_f64 - 0.125_122_19) as f32; 3],
        lin_side_break: Some([(0.014_f64 * 0.9) as f32; 3]),
        linear_slope: Some([(1.975_479_8_f64 / 0.9) as f32; 3]),
    }
}

/// One Canon Log camera space: the code value clamped to the range the
/// reference tabulates it over, the curve about its own centre, then Cinema
/// Gamut into AP0. CAT02 rather than Bradford, which is what the reference asks
/// for here and nowhere else.
fn canon(centre: f64, params: LogParams) -> Result<Chain> {
    Ok(Chain::new(vec![
        // The reference reads this curve from a 4096-point table over 0 to 1,
        // so a code value outside that clamps rather than carrying on.
        Op::Range(RangeParams {
            min_in: Some(0.0),
            max_in: Some(1.0),
            min_out: Some(0.0),
            max_out: Some(1.0),
            no_clamp: false,
        }),
        Op::Matrix([
            1.0, 0.0, 0.0, -centre, //
            0.0, 1.0, 0.0, -centre, //
            0.0, 0.0, 1.0, -centre,
        ]),
        Op::Negatives {
            style: Negatives::Mirror,
            curve: Box::new(Op::Log {
                params,
                dir: Direction::Inverse,
            }),
        },
        Op::Matrix(matrix::rgb_to_rgb_cat02(
            &matrix::CANON_CGAMUT,
            &matrix::AP0,
        )?),
    ]))
}

/// Apple Log's decode, with the floor the reference gives it: a code value
/// below zero decodes to the same light as zero does, both ways round.
fn apple_log_to_linear() -> Vec<Op> {
    vec![
        Op::Range(RangeParams {
            min_in: Some(0.0),
            min_out: Some(0.0),
            ..RangeParams::default()
        }),
        Op::GammaLog {
            params: apple_log_params(),
            dir: Direction::Inverse,
        },
    ]
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
/// transfer function run backwards (light to code value).
///
/// Every one of the ACES v2 display encodings has this shape, and the reference
/// library's own decomposition of each style is literally these two steps —
/// which is why they are tier one and not bakes.
fn display(to: &matrix::Chromaticities, curve: Op) -> Result<Chain> {
    Ok(Chain::new(vec![Op::Matrix(matrix::xyz_d65_to(to)?), curve]))
}

/// The same, with the curve mirrored about zero: the `- MIRROR NEGS` styles,
/// and the ones whose curve mirrors without saying so in its name.
fn display_mirrored(to: &matrix::Chromaticities, curve: Op) -> Result<Chain> {
    display(to, mirrored(curve))
}

fn mirrored(curve: Op) -> Op {
    Op::Negatives {
        style: Negatives::Mirror,
        curve: Box::new(curve),
    }
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
        // Each display encoding comes in two readings of negative light: the
        // plain style carries the curve on below zero as it stands (a gamma
        // clamps, sRGB's straight segment continues), the `- MIRROR NEGS` one
        // turns it about the origin.
        "DISPLAY - CIE-XYZ-D65_to_sRGB" => display(&matrix::REC709, srgb_curve())?,
        "DISPLAY - CIE-XYZ-D65_to_sRGB - MIRROR NEGS" => {
            display_mirrored(&matrix::REC709, srgb_curve())?
        }
        "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709" => display(&matrix::REC709, gamma(2.2))?,
        "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709 - MIRROR NEGS" => {
            display_mirrored(&matrix::REC709, gamma(2.2))?
        }
        // Rec.1886's EOTF is a pure 2.4 power once its black level is zero,
        // which is what a display encoding assumes.
        "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709" => display(&matrix::REC709, gamma(2.4))?,
        "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709 - MIRROR NEGS" => {
            display_mirrored(&matrix::REC709, gamma(2.4))?
        }
        "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.2020" => display(&matrix::REC2020, gamma(2.4))?,
        "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65" => display(&matrix::P3_D65, gamma(2.6))?,
        "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65 - MIRROR NEGS" => {
            display_mirrored(&matrix::P3_D65, gamma(2.6))?
        }
        // Apple's Display P3 is the P3 primaries with the sRGB curve; the HDR
        // variant differs only in how bright the container is taken to be, and
        // the reference library resolves both to the same two steps.
        "DISPLAY - CIE-XYZ-D65_to_DisplayP3" | "DISPLAY - CIE-XYZ-D65_to_DisplayP3-HDR" => {
            display_mirrored(&matrix::P3_D65, srgb_curve())?
        }
        "DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ" => display_mirrored(
            &matrix::REC2020,
            Op::Pq {
                dir: Direction::Forward,
            },
        )?,
        "DISPLAY - CIE-XYZ-D65_to_ST2084-P3-D65" => display_mirrored(
            &matrix::P3_D65,
            Op::Pq {
                dir: Direction::Forward,
            },
        )?,
        // HLG's own reference peak is 1000 nits, so the system gamma is 1.2
        // exactly (1.2 + 0.42·log₁₀ of the peak over 1000). One unit of
        // display light is 100 nits, and HLG counts scene light 0 to 3 over the
        // peak, which is the two scalings folded together here. The surround
        // step takes the system gamma back off, and then the OETF encodes.
        "DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit" => {
            let gamma = 1.2_f64;
            let scale = 100.0 * (3.0_f64).powf(gamma) / 1000.0;
            Chain::new(vec![
                Op::Matrix(matrix::xyz_d65_to(&matrix::REC2020)?),
                Op::Matrix(matrix::from_3x3(&[
                    scale, 0.0, 0.0, //
                    0.0, scale, 0.0, //
                    0.0, 0.0, scale,
                ])),
                Op::Surround {
                    exp: (1.0 / gamma) as f32,
                    dir: Direction::Forward,
                },
                Op::GammaLog {
                    params: hlg_params(),
                    dir: Direction::Forward,
                },
            ])
        }
        // The transfer functions on their own. Both of these mirror about zero:
        // the reference tabulates them over the whole half-float domain, sign
        // and all, rather than clamping.
        "CURVE - LINEAR_to_ST-2084" => Chain::new(vec![mirrored(Op::Pq {
            dir: Direction::Forward,
        })]),
        "CURVE - ST-2084_to_LINEAR" => Chain::new(vec![mirrored(Op::Pq {
            dir: Direction::Inverse,
        })]),
        "CURVE - HLG-OETF" => Chain::new(vec![Op::GammaLog {
            params: hlg_params(),
            dir: Direction::Forward,
        }]),
        "CURVE - HLG-OETF-INVERSE" => Chain::new(vec![Op::GammaLog {
            params: hlg_params(),
            dir: Direction::Inverse,
        }]),
        "CURVE - APPLE_LOG_to_LINEAR" => Chain::new(apple_log_to_linear()),
        "APPLE_LOG_to_ACES2065-1" => {
            let mut ops = apple_log_to_linear();
            // Apple Log carries Rec.2020 primaries, D65, so the step into AP0
            // adapts to D60 on the way.
            ops.push(Op::Matrix(matrix::rgb_to_rgb(
                &matrix::REC2020,
                &matrix::AP0,
            )?));
            Chain::new(ops)
        }
        "CANON_CLOG2-CGAMUT_to_ACES2065-1" => canon(0.092_864_125, clog2_params())?,
        "CANON_CLOG3-CGAMUT_to_ACES2065-1" => canon(0.125_122_19, clog3_params())?,
        "ACEScg_to_ACES2065-1" => Chain::new(vec![Op::Matrix(matrix::rgb_to_rgb(
            &matrix::AP1,
            &matrix::AP0,
        )?)]),
        "UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD" => {
            Chain::new(vec![Op::Matrix(matrix::rgb_to_xyz_d65(&matrix::AP0)?)])
        }
        "UTILITY - ACES-AP1_to_CIE-XYZ-D65_BFD" => {
            Chain::new(vec![Op::Matrix(matrix::rgb_to_xyz_d65(&matrix::AP1)?)])
        }
        // Tier two: a vendored bake, read as chain steps. A style in neither
        // tier refuses by name, `ADX10_to_ACES2065-1` and its 16-bit twin
        // among them: the film-density curve at the heart of both is an
        // eleven-knot table at spacings of the reference's own choosing, with a
        // straight extrapolation and a clamp at each end, and nothing in the op
        // set holds a table that is not evenly spaced.
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
        // A 2000 nit ACES 1.x output transform: not in `VENDORED`, so it
        // refuses, which is the promise, rather than a gap being papered over.
        let style =
            "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-2000nit-15nit-REC2020lim_1.1";
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

    /// The HLG display encoding is the one tier-one style that mixes the
    /// channels, because Rec.2100's surround step scales all three by their own
    /// luminance. So it cannot take the cheap factorised form, and this says so
    /// out loud: it bakes as a cube, and the cube lands where the chain does.
    #[test]
    fn the_hlg_display_style_bakes_as_a_cube() {
        let style = "DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit";
        let chain = resolve(style, Direction::Forward).expect("implemented");
        assert!(!chain.is_factorable());
        let baked = crate::bake::bake(&chain, crate::bake::Shaper::DEFAULT).expect("it bakes");
        assert!(matches!(baked, crate::Artefact::ShaperCube { .. }));
        let probe = [0.18, 0.2, 0.16];
        let (want, got) = (chain.eval(probe), baked.eval(probe));
        assert!(close(got, want, 2e-3), "{got:?} vs {want:?}");
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
