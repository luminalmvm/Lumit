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
//!    library version and generation script recorded in the file's header. This
//!    is how the ACES output-transform styles are meant to arrive. **None is
//!    vendored yet**; `vendored/README.md` records what a bake needs before it
//!    can be, and until then those styles refuse.
//!
//! A style in neither tier refuses the config **by name**. Note the happy
//! accident of history: the *legacy* ACES configs (1.0.3 and 1.2, still the most
//! widespread) are pure config data — matrices, logs and `.spi1d`/`.spi3d`
//! files — and need no built-ins at all.

use crate::bake::VendoredArtefact;
use crate::error::{ColourError, Result};
use crate::matrix;
use crate::op::{Chain, Direction, LogParams, Op};

/// Tier one: the styles implemented directly. Listed so a caller can say what
/// is supported without guessing at a refusal.
pub const IMPLEMENTED: [&str; 4] = [
    "IDENTITY",
    "ACEScct_to_ACES2065-1",
    "ACEScg_to_ACES2065-1",
    "UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD",
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

/// Tier two, when it exists: a reference bake vendored by style name.
///
/// The registry is empty today, and deliberately so — a bake nobody generated
/// cannot be invented. Each entry, when it lands, carries the provenance header
/// [`VendoredArtefact`] parses.
fn vendored(_style: &str) -> Option<VendoredArtefact> {
    None
}

/// Resolve a `BuiltinTransform` style into a chain, or refuse it by name.
pub fn resolve(style: &str, dir: Direction) -> Result<Chain> {
    let forward = match style {
        "IDENTITY" => Chain::identity(),
        // The ACEScct log undone, then AP1 primaries into AP0.
        "ACEScct_to_ACES2065-1" => Chain::new(vec![
            Op::Log {
                params: acescct_params(),
                dir: Direction::Inverse,
            },
            Op::Matrix(matrix::rgb_to_rgb(&matrix::AP1, &matrix::AP0)?),
        ]),
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
        // Tier two is an artefact, not a chain, so it is reached through
        // [`vendored_artefact`]. A style in neither tier refuses by name.
        other => {
            return Err(ColourError::UnsupportedBuiltin {
                style: other.to_string(),
            })
        }
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
    fn an_aces_output_style_refuses_by_name_until_its_bake_is_vendored() {
        let style = "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0";
        let err = resolve(style, Direction::Forward);
        assert!(
            matches!(&err, Err(ColourError::UnsupportedBuiltin { style: s }) if s == style),
            "{err:?}"
        );
        assert!(vendored_artefact(style).is_none());
    }

    #[test]
    fn every_listed_style_actually_resolves() {
        for style in IMPLEMENTED {
            assert!(resolve(style, Direction::Forward).is_ok(), "{style}");
        }
    }
}
