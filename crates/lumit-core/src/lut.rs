//! Colour LUT loading and CPU sampling — the building block under the coming
//! LUT effect (docs/08-EFFECTS.md §3.11).
//!
//! In plain terms: a LUT (look-up table) is a colour recipe baked elsewhere —
//! give it a red/green/blue and it hands back a graded red/green/blue. The
//! common `.cube` text format stores that recipe as a cube of sample points: a
//! 3D LUT is a grid (say 33×33×33) of "this colour in, that colour out", a 1D
//! LUT is three per-channel curves. This module parses such a file into an
//! immutable [`Lut`] and answers the one question the effect will ask millions
//! of times a frame — "what does this LUT turn *this* colour into?" — by
//! **trilinear interpolation** (3D) or per-channel linear interpolation (1D).
//!
//! The sampler is deliberately the simplest continuous maths that does the job,
//! because it is also the CPU reference *oracle* the GPU shader will be checked
//! against (docs/08-EFFECTS.md §1.6): both paths must agree to the last decimal,
//! so the maths here has to be exactly reproducible in WGSL — no table lookups
//! that a shader cannot mirror, no branchy special cases.
//!
//! Thread role: pure and deterministic, with no I/O or shared state. A [`Lut`]
//! is immutable once parsed and therefore `Send + Sync`; [`Lut3d::sample`] and
//! [`Lut1d::sample`] may run on any worker thread.

/// Adobe caps a 3D cube at 256 points per axis. At three `f32` per point that is
/// already ~200 MB for a full 256³ table, so anything larger is refused rather
/// than allocated (docs/14-ENGINEERING-RULES.md §5, budgeted allocations).
const MAX_3D_SIZE: usize = 256;

/// Adobe caps a 1D cube at 65536 points.
const MAX_1D_SIZE: usize = 65536;

/// A 3D colour cube: `size³` samples, red changing fastest (Adobe order), so the
/// sample for grid cell `(r, g, b)` lives at flat index `r + g*size + b*size*size`.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut3d {
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `data.len() == size * size * size`.
    pub data: Vec<[f32; 3]>,
}

/// A 1D colour curve stored as `size` samples; each input channel is
/// interpolated independently through its own column of the table.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut1d {
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `data.len() == size`.
    pub data: Vec<[f32; 3]>,
}

/// A parsed `.cube` LUT — either a 3D cube or a 1D curve.
#[derive(Debug, Clone, PartialEq)]
pub enum Lut {
    Cube3d(Lut3d),
    Cube1d(Lut1d),
}

impl Lut {
    /// Apply the LUT to one colour, dispatching to the 3D or 1D sampler.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Lut::Cube3d(lut) => lut.sample(rgb),
            Lut::Cube1d(lut) => lut.sample(rgb),
        }
    }
}

/// Everything that can go wrong reading a `.cube` file. Never a panic: the LUT
/// effect turns any of these into a labelled no-op with a warning badge rather
/// than a render failure (docs/08-EFFECTS.md §3.11, never-crash rule).
#[derive(Debug, thiserror::Error)]
pub enum LutError {
    #[error("the .cube data declares no size (missing LUT_3D_SIZE or LUT_1D_SIZE)")]
    MissingSize,
    #[error("line {line}: the .cube data declares its size more than once")]
    DuplicateSize { line: usize },
    #[error("line {line}: LUT size {size} is too small; at least 2 points per axis are required")]
    SizeTooSmall { size: usize, line: usize },
    #[error("line {line}: LUT size {size} exceeds the maximum of {max}")]
    SizeTooLarge {
        size: usize,
        max: usize,
        line: usize,
    },
    #[error("line {line}: could not read the LUT size ({text:?})")]
    BadSize { line: usize, text: String },
    #[error("expected {expected} data rows but found {found}")]
    WrongRowCount { expected: usize, found: usize },
    #[error("line {line}: could not parse a number ({text:?})")]
    BadNumber { line: usize, text: String },
    #[error("line {line}: a data row must have exactly three values")]
    MalformedRow { line: usize },
    #[error("line {line}: LUT data appears before the size was declared")]
    DataBeforeSize { line: usize },
}

/// Parse Adobe Cube LUT text (`.cube`).
///
/// `#` comments and blank lines are ignored; `TITLE`, `DOMAIN_MIN` and
/// `DOMAIN_MAX` are recognised (domain defaults to `0..1`); unknown metadata
/// keywords are skipped for forward compatibility. Exactly one of
/// `LUT_3D_SIZE`/`LUT_1D_SIZE` must appear, followed by the table of `R G B`
/// rows — `size³` rows red-fastest for a 3D cube, `size` rows for a 1D curve.
pub fn parse_cube(text: &str) -> Result<Lut, LutError> {
    let mut size_3d: Option<usize> = None;
    let mut size_1d: Option<usize> = None;
    let mut domain_min = [0.0_f32; 3];
    let mut domain_max = [1.0_f32; 3];
    let mut data: Vec<[f32; 3]> = Vec::new();

    for (i, raw_line) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        let mut tokens = line.split_whitespace();
        let Some(first) = tokens.next() else { continue };

        if starts_alpha(first) {
            match first.to_ascii_uppercase().as_str() {
                "TITLE" => { /* metadata, ignored */ }
                "DOMAIN_MIN" => domain_min = parse_triple(&mut tokens, line_no)?,
                "DOMAIN_MAX" => domain_max = parse_triple(&mut tokens, line_no)?,
                "LUT_3D_SIZE" => {
                    if size_3d.is_some() || size_1d.is_some() {
                        return Err(LutError::DuplicateSize { line: line_no });
                    }
                    let n = parse_size(tokens.next(), line_no)?;
                    if n <= 1 {
                        return Err(LutError::SizeTooSmall {
                            size: n,
                            line: line_no,
                        });
                    }
                    if n > MAX_3D_SIZE {
                        return Err(LutError::SizeTooLarge {
                            size: n,
                            max: MAX_3D_SIZE,
                            line: line_no,
                        });
                    }
                    size_3d = Some(n);
                }
                "LUT_1D_SIZE" => {
                    if size_3d.is_some() || size_1d.is_some() {
                        return Err(LutError::DuplicateSize { line: line_no });
                    }
                    let n = parse_size(tokens.next(), line_no)?;
                    if n <= 1 {
                        return Err(LutError::SizeTooSmall {
                            size: n,
                            line: line_no,
                        });
                    }
                    if n > MAX_1D_SIZE {
                        return Err(LutError::SizeTooLarge {
                            size: n,
                            max: MAX_1D_SIZE,
                            line: line_no,
                        });
                    }
                    size_1d = Some(n);
                }
                _ => { /* unknown metadata keyword, ignored for forward compatibility */ }
            }
        } else {
            // A data row: its first token is numeric (leading digit/sign/dot).
            if size_3d.is_none() && size_1d.is_none() {
                return Err(LutError::DataBeforeSize { line: line_no });
            }
            let r = parse_float(Some(first), line_no)?;
            let g = parse_float(tokens.next(), line_no)?;
            let b = parse_float(tokens.next(), line_no)?;
            if tokens.next().is_some() {
                return Err(LutError::MalformedRow { line: line_no });
            }
            data.push([r, g, b]);
        }
    }

    match (size_3d, size_1d) {
        (Some(n), None) => {
            let expected = n.saturating_mul(n).saturating_mul(n);
            if data.len() != expected {
                return Err(LutError::WrongRowCount {
                    expected,
                    found: data.len(),
                });
            }
            Ok(Lut::Cube3d(Lut3d {
                size: n,
                domain_min,
                domain_max,
                data,
            }))
        }
        (None, Some(n)) => {
            if data.len() != n {
                return Err(LutError::WrongRowCount {
                    expected: n,
                    found: data.len(),
                });
            }
            Ok(Lut::Cube1d(Lut1d {
                size: n,
                domain_min,
                domain_max,
                data,
            }))
        }
        // (Some, Some) is prevented by the DuplicateSize guard above.
        _ => Err(LutError::MissingSize),
    }
}

impl Lut3d {
    /// Trilinear interpolation of the cube at `rgb`.
    ///
    /// Each channel is mapped through `[domain_min, domain_max]` onto a grid
    /// coordinate in `[0, size-1]` and clamped there (out-of-domain colours
    /// clamp to the edge). The eight surrounding grid samples are then blended
    /// by the fractional position within the cell. Trilinear is continuous, and
    /// this is the reference the GPU shader must reproduce (docs/08 §1.6).
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        if n == 0 {
            return rgb;
        }
        let max_idx = n - 1;
        let (r0, r1, fr) = axis(rgb[0], self.domain_min[0], self.domain_max[0], max_idx);
        let (g0, g1, fg) = axis(rgb[1], self.domain_min[1], self.domain_max[1], max_idx);
        let (b0, b1, fb) = axis(rgb[2], self.domain_min[2], self.domain_max[2], max_idx);

        let c000 = self.at(r0, g0, b0);
        let c100 = self.at(r1, g0, b0);
        let c010 = self.at(r0, g1, b0);
        let c110 = self.at(r1, g1, b0);
        let c001 = self.at(r0, g0, b1);
        let c101 = self.at(r1, g0, b1);
        let c011 = self.at(r0, g1, b1);
        let c111 = self.at(r1, g1, b1);

        // Blend along red, then green, then blue.
        let c00 = lerp3(c000, c100, fr);
        let c10 = lerp3(c010, c110, fr);
        let c01 = lerp3(c001, c101, fr);
        let c11 = lerp3(c011, c111, fr);
        let c0 = lerp3(c00, c10, fg);
        let c1 = lerp3(c01, c11, fg);
        lerp3(c0, c1, fb)
    }

    /// Fetch the sample at grid cell `(r, g, b)`, red-fastest. Saturating index
    /// maths plus a bounds-checked read keep this free of panics even for a
    /// hand-built `Lut3d`; for any cube `parse_cube` produces the index is
    /// always in range, so it matches the plain `r + g*n + b*n*n` a shader uses.
    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        let n = self.size;
        let idx = r
            .saturating_add(g.saturating_mul(n))
            .saturating_add(b.saturating_mul(n).saturating_mul(n));
        self.data.get(idx).copied().unwrap_or([0.0, 0.0, 0.0])
    }
}

impl Lut1d {
    /// Per-channel linear interpolation across the 1D table. Each input channel
    /// is mapped through its own `[domain_min, domain_max]` (clamped) and read
    /// from the matching column of the table.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        [
            self.sample_channel(0, rgb[0]),
            self.sample_channel(1, rgb[1]),
            self.sample_channel(2, rgb[2]),
        ]
    }

    fn sample_channel(&self, ch: usize, value: f32) -> f32 {
        let n = self.size;
        if n == 0 {
            return value;
        }
        let max_idx = n - 1;
        let lo = self.domain_min.get(ch).copied().unwrap_or(0.0);
        let hi = self.domain_max.get(ch).copied().unwrap_or(1.0);
        let (i0, i1, frac) = axis(value, lo, hi, max_idx);
        let v0 = self
            .data
            .get(i0)
            .and_then(|c| c.get(ch))
            .copied()
            .unwrap_or(value);
        let v1 = self
            .data
            .get(i1)
            .and_then(|c| c.get(ch))
            .copied()
            .unwrap_or(value);
        v0 + (v1 - v0) * frac
    }
}

/// Map one input channel onto its grid, returning the two bracketing indices and
/// the fraction between them. Out-of-domain values clamp to the edge cell.
fn axis(value: f32, lo: f32, hi: f32, max_idx: usize) -> (usize, usize, f32) {
    let maxf = max_idx as f32;
    let span = hi - lo;
    let t = if span != 0.0 {
        (value - lo) / span
    } else {
        0.0
    };
    let coord = (t * maxf).clamp(0.0, maxf);
    let base = coord.floor();
    let frac = coord - base;
    let i0 = (base as usize).min(max_idx);
    let i1 = (i0 + 1).min(max_idx);
    (i0, i1, frac)
}

/// Linear blend of two colours, component-wise.
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn starts_alpha(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
}

/// Everything up to the first `#` (comment). `split` always yields at least one
/// piece, so the fallback is never taken.
fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("")
}

fn parse_float(token: Option<&str>, line: usize) -> Result<f32, LutError> {
    match token {
        Some(t) => t.parse::<f32>().map_err(|_| LutError::BadNumber {
            line,
            text: t.to_string(),
        }),
        None => Err(LutError::BadNumber {
            line,
            text: String::new(),
        }),
    }
}

fn parse_size(token: Option<&str>, line: usize) -> Result<usize, LutError> {
    match token {
        Some(t) => t.parse::<usize>().map_err(|_| LutError::BadSize {
            line,
            text: t.to_string(),
        }),
        None => Err(LutError::BadSize {
            line,
            text: String::new(),
        }),
    }
}

fn parse_triple(
    tokens: &mut std::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<[f32; 3], LutError> {
    let a = parse_float(tokens.next(), line)?;
    let b = parse_float(tokens.next(), line)?;
    let c = parse_float(tokens.next(), line)?;
    Ok([a, b, c])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn expect_3d(text: &str) -> Lut3d {
        match parse_cube(text).unwrap() {
            Lut::Cube3d(lut) => lut,
            other => panic!("expected a 3D LUT, got {other:?}"),
        }
    }

    fn expect_1d(text: &str) -> Lut1d {
        match parse_cube(text).unwrap() {
            Lut::Cube1d(lut) => lut,
            other => panic!("expected a 1D LUT, got {other:?}"),
        }
    }

    fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps && (a[2] - b[2]).abs() <= eps
    }

    // Identity 2×2×2 cube: each corner's output equals its grid position, laid
    // out red-fastest (r + 2g + 4b).
    const IDENTITY_2: &str = "\
LUT_3D_SIZE 2
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";

    #[test]
    fn identity_3d_returns_input_at_corners_and_inside() {
        let lut = expect_3d(IDENTITY_2);
        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
        ];
        for c in corners {
            assert!(
                close(lut.sample(c), c, 1e-6),
                "corner {c:?} -> {:?}",
                lut.sample(c)
            );
        }
        // Interior points: an identity cube is linear, so trilinear reproduces
        // the input exactly.
        assert!(close(lut.sample([0.5, 0.5, 0.5]), [0.5, 0.5, 0.5], 1e-6));
        assert!(close(
            lut.sample([0.25, 0.75, 0.1]),
            [0.25, 0.75, 0.1],
            1e-6
        ));
    }

    // A non-trivial cube that swaps red and blue: out = [b, g, r].
    const SWAP_RB_2: &str = "\
# swaps red and blue
LUT_3D_SIZE 2
0.0 0.0 0.0
0.0 0.0 1.0
0.0 1.0 0.0
0.0 1.0 1.0
1.0 0.0 0.0
1.0 0.0 1.0
1.0 1.0 0.0
1.0 1.0 1.0
";

    #[test]
    fn swap_rb_corners_are_exact() {
        let lut = expect_3d(SWAP_RB_2);
        assert!(close(lut.sample([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0], 1e-6));
        assert!(close(lut.sample([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0], 1e-6));
        assert!(close(lut.sample([1.0, 1.0, 0.0]), [0.0, 1.0, 1.0], 1e-6));
        // Swap is linear, so an interior point is the exact swap too.
        assert!(close(
            lut.sample([0.25, 0.6, 0.75]),
            [0.75, 0.6, 0.25],
            1e-6
        ));
    }

    #[test]
    fn trilinear_midpoint_matches_hand_computed_lerp() {
        // Arbitrary, non-separable corner values (red-fastest, r + 2g + 4b).
        let corners: [[f32; 3]; 8] = [
            [0.10, 0.20, 0.30], // 000
            [0.90, 0.15, 0.05], // 100
            [0.40, 0.80, 0.10], // 010
            [0.55, 0.35, 0.95], // 110
            [0.05, 0.60, 0.70], // 001
            [0.25, 0.45, 0.85], // 101
            [0.65, 0.05, 0.15], // 011
            [1.00, 0.90, 0.20], // 111
        ];
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for c in &corners {
            text.push_str(&format!("{} {} {}\n", c[0], c[1], c[2]));
        }
        let lut = expect_3d(&text);

        // With size 2 the grid coordinate equals the input, so (fr, fg, fb) are
        // the sample coordinates directly.
        let (fr, fg, fb) = (0.25_f32, 0.75_f32, 0.5_f32);
        let got = lut.sample([fr, fg, fb]);

        // Independently interpolate red-fastest by hand.
        let l = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        let c00 = l(corners[0], corners[1], fr);
        let c10 = l(corners[2], corners[3], fr);
        let c01 = l(corners[4], corners[5], fr);
        let c11 = l(corners[6], corners[7], fr);
        let c0 = l(c00, c10, fg);
        let c1 = l(c01, c11, fg);
        let expected = l(c0, c1, fb);

        assert!(
            close(got, expected, 1e-6),
            "got {got:?} expected {expected:?}"
        );
    }

    // A small 1D curve with a distinct column per channel.
    const CURVE_1D: &str = "\
LUT_1D_SIZE 3
0.0 0.0 1.0
0.5 0.25 0.5
1.0 1.0 0.0
";

    #[test]
    fn one_d_endpoints_and_midpoint() {
        let lut = expect_1d(CURVE_1D);
        assert!(close(lut.sample([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0], 1e-6));
        assert!(close(lut.sample([1.0, 1.0, 1.0]), [1.0, 1.0, 0.0], 1e-6));
        // Value 0.5 lands exactly on the middle sample.
        assert!(close(lut.sample([0.5, 0.5, 0.5]), [0.5, 0.25, 0.5], 1e-6));
        // Value 0.25 sits halfway between the first two samples.
        assert!(close(
            lut.sample([0.25, 0.25, 0.25]),
            [0.25, 0.125, 0.75],
            1e-6
        ));
    }

    // Identity cube with a stretched domain of 0..2.
    const IDENTITY_2_DOMAIN2: &str = "\
LUT_3D_SIZE 2
DOMAIN_MIN 0 0 0
DOMAIN_MAX 2 2 2
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";

    #[test]
    fn non_default_domain_remaps_and_clamps() {
        let lut = expect_3d(IDENTITY_2_DOMAIN2);
        assert_eq!(lut.domain_max, [2.0, 2.0, 2.0]);
        assert!(close(lut.sample([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0], 1e-6));
        // Input 1.0 is the middle of a 0..2 domain.
        assert!(close(lut.sample([1.0, 1.0, 1.0]), [0.5, 0.5, 0.5], 1e-6));
        assert!(close(lut.sample([2.0, 2.0, 2.0]), [1.0, 1.0, 1.0], 1e-6));
        // Out-of-domain clamps to the edge.
        assert!(close(lut.sample([5.0, 5.0, 5.0]), [1.0, 1.0, 1.0], 1e-6));
        assert!(close(lut.sample([-1.0, -1.0, -1.0]), [0.0, 0.0, 0.0], 1e-6));
    }

    const IDENTITY_WITH_JUNK: &str = "\
# a comment line
TITLE \"My Identity\"

LUT_3D_SIZE 2
# corners follow
0.0 0.0 0.0
1.0 0.0 0.0

0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";

    #[test]
    fn comments_blank_lines_and_title_are_ignored() {
        let lut = expect_3d(IDENTITY_WITH_JUNK);
        assert_eq!(lut.size, 2);
        assert!(close(lut.sample([0.3, 0.6, 0.9]), [0.3, 0.6, 0.9], 1e-6));
    }

    #[test]
    fn malformed_inputs_error_without_panicking() {
        // Missing size (data with no declaration, and metadata-only).
        assert!(parse_cube("0.0 0.0 0.0\n1.0 1.0 1.0\n").is_err());
        assert!(parse_cube("TITLE \"x\"\n# nothing else\n").is_err());

        // Wrong row count: size 2 needs 8 rows, only 7 given.
        let short = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n";
        assert!(matches!(
            parse_cube(short),
            Err(LutError::WrongRowCount { .. })
        ));

        // Non-numeric data.
        let bad = "LUT_3D_SIZE 2\n0 0 0\n1 foo 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        assert!(matches!(parse_cube(bad), Err(LutError::BadNumber { .. })));

        // Too many values in a row.
        let extra = "LUT_1D_SIZE 2\n0 0 0 0\n1 1 1\n";
        assert!(matches!(
            parse_cube(extra),
            Err(LutError::MalformedRow { .. })
        ));

        // Size 0 and 1.
        assert!(matches!(
            parse_cube("LUT_3D_SIZE 0\n"),
            Err(LutError::SizeTooSmall { .. })
        ));
        assert!(matches!(
            parse_cube("LUT_3D_SIZE 1\n0 0 0\n"),
            Err(LutError::SizeTooSmall { .. })
        ));

        // Unreasonably large size: rejected before any allocation.
        assert!(matches!(
            parse_cube("LUT_3D_SIZE 9999\n"),
            Err(LutError::SizeTooLarge { .. })
        ));
        assert!(matches!(
            parse_cube("LUT_1D_SIZE 999999\n"),
            Err(LutError::SizeTooLarge { .. })
        ));

        // Duplicate size.
        assert!(matches!(
            parse_cube("LUT_3D_SIZE 2\nLUT_3D_SIZE 2\n"),
            Err(LutError::DuplicateSize { .. })
        ));
    }

    #[test]
    fn error_implements_std_error_and_displays() {
        let err = parse_cube("TITLE \"empty\"\n").unwrap_err();
        let _as_std: &dyn std::error::Error = &err;
        assert!(!err.to_string().is_empty());
    }
}
