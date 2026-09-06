//! 3×4 colour matrices, and where the fixed bridging matrices come from.
//!
//! In plain terms: turning one set of red/green/blue primaries into another is a
//! multiplication by a small table of nine numbers (plus three offsets, which
//! configs occasionally use). Two of those tables are load-bearing for OCIO —
//! ACES's own reference space to Lumit's working space, and CIE XYZ to Lumit's
//! working space — and rather than typing published constants from memory this
//! module **derives** them from the primaries and white points, the same way a
//! colour scientist would. The derivation is plain arithmetic, so it is exact
//! and identical on every machine, and a test checks it against the published
//! sRGB matrix everyone agrees on.
//!
//! Storage is row-major, three rows of four: `[m00 m01 m02 t0, m10 … t1, m20 … t2]`,
//! so `out = M · in + t`.

use crate::error::{ColourError, Result};

/// A 3×4 colour matrix, row-major (three rows of `[a b c offset]`).
///
/// The coefficients are held in **double** and evaluated in single. That split
/// is not fussiness, it is a measured fidelity fix: a config's `from_reference`
/// is usually its `to_reference` inverted, so a space-to-space chain routinely
/// carries a matrix immediately followed by its own inverse. Rounding that
/// inverse to `f32` before composing leaves a residue of about 3 × 10⁻¹⁰ in the
/// product, which a clamped 65504 in one channel turns into 2 × 10⁻⁵ of error in
/// another — ACEScc → ACEScg, measured against the reference library, is exactly
/// that. Composed in double the residue is 10⁻¹⁶ and the pair cancels.
pub type Matrix34 = [f64; 12];

/// The matrix that changes nothing.
pub const IDENTITY: Matrix34 = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0,
];

/// The coefficients as the samplers see them: single precision, exactly what
/// [`crate::bake`] hands the graphics card. Every evaluation goes through this,
/// so the processor and the card multiply the same twelve numbers.
#[must_use]
pub fn single(m: &Matrix34) -> [f32; 12] {
    let mut out = [0.0_f32; 12];
    for (o, v) in out.iter_mut().zip(m) {
        *o = *v as f32;
    }
    out
}

/// Apply a 3×4 matrix to one colour.
///
/// Written as explicit `a * b + c` rather than `f32::mul_add`: a fused
/// multiply-add rounds once instead of twice, which is *more* accurate but
/// differs from a machine that does not fuse — the classic source of last-bit
/// drift (docs/impl/ocio.md §4.2).
#[must_use]
pub fn apply(m: &Matrix34, rgb: [f32; 3]) -> [f32; 3] {
    let m = single(m);
    let mut out = [0.0_f32; 3];
    for (row, o) in out.iter_mut().enumerate() {
        let b = row * 4;
        *o = m[b] * rgb[0] + m[b + 1] * rgb[1] + m[b + 2] * rgb[2] + m[b + 3];
    }
    out
}

/// `second ∘ first`: the one matrix that does `first` then `second`.
#[must_use]
pub fn concat(first: &Matrix34, second: &Matrix34) -> Matrix34 {
    let mut out = [0.0_f64; 12];
    for row in 0..3 {
        for col in 0..3 {
            let mut acc = 0.0_f64;
            for k in 0..3 {
                acc += second[row * 4 + k] * first[k * 4 + col];
            }
            out[row * 4 + col] = acc;
        }
        // The offset runs through `second`'s linear part and picks up its own.
        let mut acc = second[row * 4 + 3];
        for k in 0..3 {
            acc += second[row * 4 + k] * first[k * 4 + 3];
        }
        out[row * 4 + 3] = acc;
    }
    out
}

/// How far a coefficient may sit from the identity's and still be called the
/// identity: a thousandth of a single-precision ULP at 1.0.
///
/// Composing a matrix with its own inverse leaves a residue near 10⁻¹⁶, and for
/// colour channels — which sit within a few orders of magnitude of each other
/// inside one pixel — a coefficient this small moves the answer by less than a
/// thousandth of the last bit an `f32` has. It is not "close enough"; it is
/// below what the arithmetic downstream can represent.
const IDENTITY_EPSILON: f64 = 1e-9;

/// Whether this matrix does nothing an `f32` could notice.
///
/// It matters beyond tidiness, and infinity is why. `matrix::apply` computes
/// every output channel from all three inputs, so once one channel has
/// overflowed to infinity — an ACEScct code value of 16 decodes to 2²⁷⁰ — a
/// matrix spreads that infinity across the other two and any coefficient of
/// zero turns it into a NaN. A matrix that does nothing should not be able to
/// do that, and the reference library drops one for the same reason.
#[must_use]
pub fn is_identity(m: &Matrix34) -> bool {
    m.iter()
        .zip(IDENTITY)
        .all(|(a, b)| (a - b).abs() <= IDENTITY_EPSILON)
}

/// The inverse matrix, or [`ColourError::SingularMatrix`] when there is none.
pub fn invert(m: &Matrix34) -> Result<Matrix34> {
    let a = [
        m[0], m[1], m[2], //
        m[4], m[5], m[6], //
        m[8], m[9], m[10],
    ];
    let inv = invert3(&a).ok_or(ColourError::SingularMatrix)?;
    let t = [m[3], m[7], m[11]];
    let mut out = [0.0_f64; 12];
    for row in 0..3 {
        for col in 0..3 {
            out[row * 4 + col] = inv[row * 3 + col];
        }
        let mut acc = 0.0_f64;
        for k in 0..3 {
            acc += inv[row * 3 + k] * t[k];
        }
        out[row * 4 + 3] = -acc;
    }
    Ok(out)
}

/// Build a 3×4 matrix from a 3×3 one (no offsets).
#[must_use]
pub fn from_3x3(m: &[f64; 9]) -> Matrix34 {
    [
        m[0], m[1], m[2], 0.0, //
        m[3], m[4], m[5], 0.0, //
        m[6], m[7], m[8], 0.0,
    ]
}

fn invert3(m: &[f64; 9]) -> Option<[f64; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let d = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * d,
        (m[2] * m[7] - m[1] * m[8]) * d,
        (m[1] * m[5] - m[2] * m[4]) * d,
        (m[5] * m[6] - m[3] * m[8]) * d,
        (m[0] * m[8] - m[2] * m[6]) * d,
        (m[2] * m[3] - m[0] * m[5]) * d,
        (m[3] * m[7] - m[4] * m[6]) * d,
        (m[1] * m[6] - m[0] * m[7]) * d,
        (m[0] * m[4] - m[1] * m[3]) * d,
    ])
}

fn mul3(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut out = [0.0_f64; 9];
    for row in 0..3 {
        for col in 0..3 {
            let mut acc = 0.0_f64;
            for k in 0..3 {
                acc += a[row * 3 + k] * b[k * 3 + col];
            }
            out[row * 3 + col] = acc;
        }
    }
    out
}

/// The four chromaticity pairs that define an RGB colour space: where its red,
/// green and blue sit on the CIE diagram, and what it calls white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chromaticities {
    pub red: [f64; 2],
    pub green: [f64; 2],
    pub blue: [f64; 2],
    pub white: [f64; 2],
}

/// Rec.709 / sRGB primaries with a D65 white — Lumit's fixed working space.
pub const REC709: Chromaticities = Chromaticities {
    red: [0.640, 0.330],
    green: [0.300, 0.600],
    blue: [0.150, 0.060],
    white: [0.3127, 0.3290],
};

/// ACES AP0 (ACES2065-1), the `aces_interchange` role's space.
pub const AP0: Chromaticities = Chromaticities {
    red: [0.7347, 0.2653],
    green: [0.0000, 1.0000],
    blue: [0.0001, -0.0770],
    white: [0.32168, 0.33767],
};

/// ACES AP1 (ACEScg / ACEScct working primaries).
pub const AP1: Chromaticities = Chromaticities {
    red: [0.713, 0.293],
    green: [0.165, 0.830],
    blue: [0.128, 0.044],
    white: [0.32168, 0.33767],
};

/// DCI-P3 primaries with a D65 white — the `DisplayP3` and `P3-D65` displays.
pub const P3_D65: Chromaticities = Chromaticities {
    red: [0.680, 0.320],
    green: [0.265, 0.690],
    blue: [0.150, 0.060],
    white: [0.3127, 0.3290],
};

/// ITU-R BT.2020 primaries, D65 — the `REC.2100` displays' container.
pub const REC2020: Chromaticities = Chromaticities {
    red: [0.708, 0.292],
    green: [0.170, 0.797],
    blue: [0.131, 0.046],
    white: [0.3127, 0.3290],
};

/// The RGB→XYZ matrix for a set of primaries (SMPTE RP 177: solve for the three
/// primary scalings that send `(1,1,1)` to the white point, then scale the
/// columns by them).
pub fn rgb_to_xyz(c: &Chromaticities) -> Result<[f64; 9]> {
    let col = |xy: [f64; 2]| -> Option<[f64; 3]> {
        if xy[1] == 0.0 {
            return None;
        }
        Some([xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]])
    };
    let (r, g, b) = match (col(c.red), col(c.green), col(c.blue)) {
        (Some(r), Some(g), Some(b)) => (r, g, b),
        _ => return Err(ColourError::SingularMatrix),
    };
    let w = col(c.white).ok_or(ColourError::SingularMatrix)?;
    let m = [r[0], g[0], b[0], r[1], g[1], b[1], r[2], g[2], b[2]];
    let inv = invert3(&m).ok_or(ColourError::SingularMatrix)?;
    let mut s = [0.0_f64; 3];
    for (row, s) in s.iter_mut().enumerate() {
        *s = inv[row * 3] * w[0] + inv[row * 3 + 1] * w[1] + inv[row * 3 + 2] * w[2];
    }
    Ok([
        m[0] * s[0],
        m[1] * s[1],
        m[2] * s[2],
        m[3] * s[0],
        m[4] * s[1],
        m[5] * s[2],
        m[6] * s[0],
        m[7] * s[1],
        m[8] * s[2],
    ])
}

/// The Bradford cone-response matrix, the industry's default chromatic
/// adaptation and the one OCIO's own `_BFD` built-ins name.
const BRADFORD: [f64; 9] = [
    0.8951, 0.2664, -0.1614, //
    -0.7502, 1.7135, 0.0367, //
    0.0389, -0.0685, 1.0296,
];

/// The XYZ→XYZ matrix that moves a white point, Bradford-adapted.
pub fn bradford(src_white: [f64; 2], dst_white: [f64; 2]) -> Result<[f64; 9]> {
    let xyz = |xy: [f64; 2]| -> Option<[f64; 3]> {
        if xy[1] == 0.0 {
            return None;
        }
        Some([xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]])
    };
    let (s, d) = match (xyz(src_white), xyz(dst_white)) {
        (Some(s), Some(d)) => (s, d),
        _ => return Err(ColourError::SingularMatrix),
    };
    let cone = |v: [f64; 3]| -> [f64; 3] {
        let mut out = [0.0_f64; 3];
        for (row, o) in out.iter_mut().enumerate() {
            *o = BRADFORD[row * 3] * v[0]
                + BRADFORD[row * 3 + 1] * v[1]
                + BRADFORD[row * 3 + 2] * v[2];
        }
        out
    };
    let (cs, cd) = (cone(s), cone(d));
    if cs.contains(&0.0) {
        return Err(ColourError::SingularMatrix);
    }
    let scale = [
        cd[0] / cs[0],
        0.0,
        0.0,
        0.0,
        cd[1] / cs[1],
        0.0,
        0.0,
        0.0,
        cd[2] / cs[2],
    ];
    let inv = invert3(&BRADFORD).ok_or(ColourError::SingularMatrix)?;
    Ok(mul3(&inv, &mul3(&scale, &BRADFORD)))
}

/// The matrix from one RGB space to another, Bradford-adapted when their white
/// points differ. This is the derivation behind both interchange bridges
/// (docs/impl/ocio.md §2.1).
pub fn rgb_to_rgb(from: &Chromaticities, to: &Chromaticities) -> Result<Matrix34> {
    let a = rgb_to_xyz(from)?;
    let b = invert3(&rgb_to_xyz(to)?).ok_or(ColourError::SingularMatrix)?;
    let m = if from.white == to.white {
        mul3(&b, &a)
    } else {
        mul3(&b, &mul3(&bradford(from.white, to.white)?, &a))
    };
    Ok(from_3x3(&m))
}

/// CIE XYZ with a D65 white → the linear RGB of a **D65** set of primaries.
///
/// No chromatic adaptation, deliberately: every display encoding that names
/// this bridge is itself D65, so an adaptation step would be a matrix that
/// should be the identity and is not. A non-D65 `to` belongs in
/// [`rgb_to_rgb`], which adapts.
pub fn xyz_d65_to(to: &Chromaticities) -> Result<Matrix34> {
    let m = invert3(&rgb_to_xyz(to)?).ok_or(ColourError::SingularMatrix)?;
    Ok(from_3x3(&m))
}

/// CIE XYZ with a D65 white → linear Rec.709, the `cie_xyz_d65_interchange` bridge.
pub fn xyz_d65_to_rec709() -> Result<Matrix34> {
    xyz_d65_to(&REC709)
}

/// ACES2065-1 (AP0, D60) → linear Rec.709 (D65), the `aces_interchange` bridge.
pub fn ap0_to_rec709() -> Result<Matrix34> {
    rgb_to_rgb(&AP0, &REC709)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn rec709_derivation_matches_the_published_srgb_matrix() {
        // The numbers everyone agrees on (IEC 61966-2-1 / Rec.709 D65).
        let published = [
            0.4124, 0.3576, 0.1805, //
            0.2126, 0.7152, 0.0722, //
            0.0193, 0.1192, 0.9505,
        ];
        let got = rgb_to_xyz(&REC709).expect("Rec.709 primaries derive");
        for (g, p) in got.iter().zip(published) {
            assert!((g - p).abs() < 5e-5, "got {got:?}");
        }
    }

    #[test]
    fn white_maps_to_white_across_the_aces_bridge() {
        let m = ap0_to_rec709().expect("AP0 bridge derives");
        // ACES white is D60; after Bradford adaptation it must land on Rec.709
        // white, i.e. an equal-energy triple stays equal-energy.
        assert!(close(apply(&m, [1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-4));
    }

    #[test]
    fn ap1_to_ap0_is_the_acescg_matrix() {
        // AP1 and AP0 share a white point, so no adaptation is involved and the
        // matrix is the published ACEScg→ACES2065-1 one.
        let m = rgb_to_rgb(&AP1, &AP0).expect("AP1→AP0 derives");
        let published = [0.695_452_241_4_f64, 0.140_678_696_5, 0.163_869_062_2];
        for (g, p) in m[0..3].iter().zip(published) {
            assert!((*g - p).abs() < 1e-5, "got {:?}", &m[0..3]);
        }
        assert!(close(apply(&m, [1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-5));
    }

    #[test]
    fn xyz_bridge_round_trips() {
        let to = xyz_d65_to_rec709().expect("XYZ bridge derives");
        let back = invert(&to).expect("and inverts");
        let c = [0.2, 0.5, 0.8];
        assert!(close(apply(&back, apply(&to, c)), c, 1e-5));
    }

    #[test]
    fn concat_then_invert_is_the_identity() {
        let a: Matrix34 = [
            1.5, 0.2, -0.1, 0.05, -0.3, 0.9, 0.4, 0.0, 0.1, -0.2, 1.2, -0.02,
        ];
        let b = rgb_to_rgb(&AP1, &REC709).expect("derives");
        let ab = concat(&a, &b);
        let inv = invert(&ab).expect("invertible");
        let c = [0.3, -0.1, 1.7];
        assert!(close(apply(&inv, apply(&ab, c)), c, 1e-4));
    }

    #[test]
    fn a_flat_matrix_refuses_to_invert() {
        let singular: Matrix34 = [1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0];
        assert!(matches!(
            invert(&singular),
            Err(ColourError::SingularMatrix)
        ));
    }
}
