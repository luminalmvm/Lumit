//! The op set: the handful of arithmetic steps every OCIO config is made of.
//!
//! In plain terms: whatever a config says in its own words, what it *means* is
//! always a short recipe of small steps — multiply by a matrix, raise to a
//! power, take a logarithm, look the colour up in a table. This module is those
//! steps, each written twice: forwards, and (where the maths allows it)
//! backwards. A **chain** is a list of them in order, and resolving anything a
//! config can name — "this space to that space", "this display's this view" —
//! ends up as one flat chain (docs/impl/ocio.md §4.2). Nothing else in Lumit
//! ever sees an OCIO word; it sees a chain.
//!
//! Two rules run through all of it. **Nothing is approximated**: a step that
//! cannot be reversed honestly (a 3D table, a curve that doubles back) refuses
//! by name rather than guessing. And **no fused multiply-add**: every product
//! and sum is written out so a machine that fuses and a machine that does not
//! agree to the last bit (docs/impl/ocio.md §4.2).

use crate::error::{ColourError, Result};
use crate::matrix::{self, Matrix34};
use crate::sample::{Cube, Curve};

/// Which way round a step runs. Some steps invert into a different formula
/// rather than a different variant, so they carry this rather than being
/// rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Forward,
    Inverse,
}

impl Direction {
    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Direction::Forward => Direction::Inverse,
            Direction::Inverse => Direction::Forward,
        }
    }
}

/// The Rec.709 luma weights the ASC CDL saturation term is defined with.
const ASC_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// One log step, covering all three config spellings: `LogTransform` (base
/// only), `LogAffineTransform` (the four slope/offset numbers) and
/// `LogCameraTransform` (those plus a break below which the curve goes straight).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogParams {
    pub base: f32,
    pub lin_side_slope: [f32; 3],
    pub lin_side_offset: [f32; 3],
    pub log_side_slope: [f32; 3],
    pub log_side_offset: [f32; 3],
    /// `None` = no linear segment (`LogTransform`/`LogAffineTransform`).
    pub lin_side_break: Option<[f32; 3]>,
    /// `None` = the slope that joins the two segments smoothly.
    pub linear_slope: Option<[f32; 3]>,
}

impl LogParams {
    /// `LogTransform`: plain `log_base(x)`.
    #[must_use]
    pub fn plain(base: f32) -> Self {
        Self {
            base,
            lin_side_slope: [1.0; 3],
            lin_side_offset: [0.0; 3],
            log_side_slope: [1.0; 3],
            log_side_offset: [0.0; 3],
            lin_side_break: None,
            linear_slope: None,
        }
    }
}

/// One gamma-and-log curve: a power curve up to a break, a logarithm above it,
/// and a point it is odd about so negatives have somewhere to go.
///
/// This is the reference library's own parameter block (its `GAMMA_LOG` fixed
/// function), and it is here because two of the built-in styles are that shape
/// and no other op is: an [`Op::Log`] with a break goes *straight* below it,
/// not curved. HLG and Apple Log are both this curve with different numbers.
/// The reference's lin-side slope is 1 in both, so it is not carried.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GammaLogParams {
    /// The linear value the curve is odd about. Zero for HLG.
    pub mirror: f32,
    /// The linear value the power curve gives way to the logarithm at.
    pub brk: f32,
    pub gamma_power: f32,
    pub gamma_slope: f32,
    pub gamma_offset: f32,
    pub base: f32,
    pub log_slope: f32,
    pub log_offset: f32,
    pub lin_offset: f32,
}

/// One ASC CDL step: slope, offset, power per channel, then saturation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CdlParams {
    pub slope: [f32; 3],
    pub offset: [f32; 3],
    pub power: [f32; 3],
    pub saturation: f32,
    /// The ASC specification clamps to `[0, 1]` after the offset and again at
    /// the end; OCIO's `no-clamp` style does not.
    pub clamp: bool,
}

impl Default for CdlParams {
    fn default() -> Self {
        Self {
            slope: [1.0; 3],
            offset: [0.0; 3],
            power: [1.0; 3],
            saturation: 1.0,
            clamp: true,
        }
    }
}

/// One range step. All four bounds present means "map this range onto that one";
/// fewer means "clamp only", which is what CLF's partial `Range` nodes are for.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RangeParams {
    pub min_in: Option<f32>,
    pub max_in: Option<f32>,
    pub min_out: Option<f32>,
    pub max_out: Option<f32>,
    pub no_clamp: bool,
}

/// What a curve does with input below zero, when it does not do its own thing.
///
/// The names are OCIO's. Every curve op here already has a default reading of
/// negatives — an exponent clamps them, a monCurve carries them down its
/// straight segment — and those readings stay. This says "not that one", and
/// it exists because the ACES v2 configs need it: their display encodings all
/// mirror, and their gamma spaces ask for `pass_thru` by name in the config
/// file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negatives {
    /// The curve is applied to the magnitude and the sign is put back, so the
    /// answer is odd about zero. OCIO's `mirror`.
    Mirror,
    /// Below zero the value is carried through untouched. OCIO's `pass_thru`.
    PassThru,
}

/// One step of a resolved chain.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Matrix(Matrix34),
    /// A curve wearing a stated behaviour below zero ([`Negatives`]). The op
    /// inside never sees a negative: `Mirror` hands it the magnitude and puts
    /// the sign back, `PassThru` does not call it at all.
    Negatives {
        style: Negatives,
        curve: Box<Op>,
    },
    /// `ExponentTransform`: `pow(max(x, 0), e)`. Negatives clamp, which is
    /// OCIO's default negative style for this transform.
    Exponent {
        exp: [f32; 3],
        dir: Direction,
    },
    /// `ExponentWithLinearTransform`: a power curve with a straight segment near
    /// zero, the shape sRGB and Rec.709 both have. Below the break the straight
    /// segment simply continues, so negatives pass through linearly rather than
    /// clamping — the continuous reading of the CLF `monCurve` definition.
    MonCurve {
        gamma: [f32; 3],
        offset: [f32; 3],
        dir: Direction,
    },
    Log {
        params: LogParams,
        dir: Direction,
    },
    /// SMPTE ST 2084 (the "PQ" curve), forward = display light to code value.
    /// One nit is 0.01, so 1.0 is the hundred-nit reference white the ACES v2
    /// display encodings hand it. Defined for non-negative light only; the
    /// configs that use it wrap it in [`Negatives::Mirror`].
    Pq {
        dir: Direction,
    },
    /// A gamma-and-log camera curve ([`GammaLogParams`]), forward = scene light
    /// to code value. HLG's OETF and Apple Log are both this.
    GammaLog {
        params: GammaLogParams,
        dir: Direction,
    },
    /// ITU-R BT.2100's surround compensation: scale the three channels by their
    /// own luminance raised to a power, which is how the HLG system gamma is
    /// put on and taken off. Forward = the exponent as stated, and it mixes the
    /// channels, so a chain carrying one bakes to a cube.
    Surround {
        exp: f32,
        dir: Direction,
    },
    Cdl {
        params: CdlParams,
        dir: Direction,
    },
    Range(RangeParams),
    Lut1d {
        curve: Curve,
        dir: Direction,
    },
    /// Sampled tetrahedrally. Has no inverse — see [`Op::inverted`].
    Lut3d {
        cube: Cube,
    },
}

fn safe_log(x: f32, base: f32) -> f32 {
    let denom = base.ln();
    if denom == 0.0 || !denom.is_finite() {
        return 0.0;
    }
    // A logarithm of zero or less is not a number; the smallest positive float
    // is the floor, which is what the reference implementations clamp to.
    x.max(f32::MIN_POSITIVE).ln() / denom
}

fn log_forward(p: &LogParams, c: usize, x: f32) -> f32 {
    let (ls, lo) = (p.lin_side_slope[c], p.lin_side_offset[c]);
    let (gs, go) = (p.log_side_slope[c], p.log_side_offset[c]);
    let straight = |x: f32| -> f32 {
        let (slope, offset) = linear_segment(p, c);
        slope * x + offset
    };
    match p.lin_side_break {
        Some(b) if x < b[c] => straight(x),
        _ => gs * safe_log(ls * x + lo, p.base) + go,
    }
}

/// The slope and offset of a camera log's straight segment: the slope is the
/// curve's own gradient at the break unless the config states one, and the
/// offset is whatever makes the two halves meet.
fn linear_segment(p: &LogParams, c: usize) -> (f32, f32) {
    let Some(brk) = p.lin_side_break else {
        return (0.0, 0.0);
    };
    let b = brk[c];
    let (ls, lo) = (p.lin_side_slope[c], p.lin_side_offset[c]);
    let (gs, go) = (p.log_side_slope[c], p.log_side_offset[c]);
    let slope = match p.linear_slope {
        Some(s) => s[c],
        None => {
            let denom = (ls * b + lo) * p.base.ln();
            if denom == 0.0 || !denom.is_finite() {
                0.0
            } else {
                gs * ls / denom
            }
        }
    };
    let log_at_break = gs * safe_log(ls * b + lo, p.base) + go;
    (slope, log_at_break - slope * b)
}

fn log_inverse(p: &LogParams, c: usize, y: f32) -> f32 {
    let (ls, lo) = (p.lin_side_slope[c], p.lin_side_offset[c]);
    let (gs, go) = (p.log_side_slope[c], p.log_side_offset[c]);
    let curved = |y: f32| -> f32 {
        if gs == 0.0 || ls == 0.0 {
            return 0.0;
        }
        (p.base.powf((y - go) / gs) - lo) / ls
    };
    match p.lin_side_break {
        Some(brk) => {
            let log_at_break = gs * safe_log(ls * brk[c] + lo, p.base) + go;
            if y < log_at_break {
                let (slope, offset) = linear_segment(p, c);
                if slope == 0.0 {
                    0.0
                } else {
                    (y - offset) / slope
                }
            } else {
                curved(y)
            }
        }
        None => curved(y),
    }
}

/// The break point and straight-segment slope of a monCurve, or `None` when the
/// parameters degenerate to a plain power curve.
fn moncurve_shape(gamma: f32, offset: f32) -> Option<(f32, f32)> {
    if gamma.is_nan() || offset.is_nan() || gamma <= 1.0 || offset <= 0.0 {
        return None;
    }
    // The break is where the power curve's own gradient equals the straight
    // segment's, which works out to `offset / (gamma - 1)`; the slope is then
    // whatever makes the two halves meet there. Written as the meeting
    // condition rather than as an expanded closed form, because the expansion
    // is easy to get subtly wrong and impossible to read.
    let x_break = offset / (gamma - 1.0);
    let slope = ((x_break + offset) / (1.0 + offset)).powf(gamma) / x_break;
    if !x_break.is_finite() || !slope.is_finite() || slope == 0.0 {
        return None;
    }
    Some((x_break, slope))
}

fn moncurve_forward(gamma: f32, offset: f32, x: f32) -> f32 {
    match moncurve_shape(gamma, offset) {
        Some((x_break, slope)) => {
            if x < x_break {
                x * slope
            } else {
                ((x + offset) / (1.0 + offset)).powf(gamma)
            }
        }
        None => x.max(0.0).powf(gamma),
    }
}

fn moncurve_inverse(gamma: f32, offset: f32, y: f32) -> f32 {
    match moncurve_shape(gamma, offset) {
        Some((x_break, slope)) => {
            let y_break = x_break * slope;
            if y < y_break {
                y / slope
            } else {
                (1.0 + offset) * y.max(0.0).powf(1.0 / gamma) - offset
            }
        }
        None => {
            if gamma == 0.0 {
                0.0
            } else {
                y.max(0.0).powf(1.0 / gamma)
            }
        }
    }
}

/// SMPTE ST 2084's constants, as the standard writes them — fractions rather
/// than decimals, so nobody has to trust a transcription.
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;
/// Display light where 1.0 is 100 nits, as ST 2084 counts it (10 000 nits = 1).
const PQ_PEAK: f32 = 100.0;

fn pq_forward(x: f32) -> f32 {
    let y = (x / PQ_PEAK).max(0.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
}

/// The decode's arithmetic is done in double and returned in single, the way
/// [`crate::matrix`] holds its coefficients. The denominator is the reason:
/// `c2 - c3·p` subtracts two numbers near 18.8 to leave about 0.3, which throws
/// away two of an f32's seven digits, and the power that follows multiplies
/// what is left of the error by 6.28. Measured against the reference library it
/// costs 2 × 10⁻⁵ at a PQ code of 0.5, twice the promise the fixtures make.
/// The encode has no such subtraction and stays in single.
fn pq_inverse(n: f32) -> f32 {
    let p = f64::from(n).max(0.0).powf(1.0 / f64::from(PQ_M2));
    let num = (p - f64::from(PQ_C1)).max(0.0);
    let den = f64::from(PQ_C2) - f64::from(PQ_C3) * p;
    if den == 0.0 {
        return 0.0;
    }
    ((num / den).powf(1.0 / f64::from(PQ_M1)) * f64::from(PQ_PEAK)) as f32
}

/// The natural-log slope: the curve states its slope against its own base, and
/// every evaluation below is written with `ln`.
fn gamma_log_slope(p: &GammaLogParams) -> f32 {
    let denom = p.base.ln();
    if denom == 0.0 || !denom.is_finite() {
        return 0.0;
    }
    p.log_slope / denom
}

/// Scene light to code value: the power curve below the break, the logarithm
/// above it, both about the mirror point.
fn gamma_log_forward(p: &GammaLogParams, x: f32) -> f32 {
    let mirrored = x - p.mirror;
    let e = mirrored.abs() + p.mirror;
    let code = if e < p.brk {
        p.gamma_slope * (e + p.gamma_offset).max(0.0).powf(p.gamma_power)
    } else {
        gamma_log_slope(p) * (e + p.lin_offset).max(f32::MIN_POSITIVE).ln() + p.log_offset
    };
    code * mirrored.signum()
}

/// And back. The break in code values is where the power curve reaches it, so
/// the two halves meet in both directions by construction.
fn gamma_log_inverse(p: &GammaLogParams, y: f32) -> f32 {
    let at = |x: f32| p.gamma_slope * (x + p.gamma_offset).max(0.0).powf(p.gamma_power);
    let (mirror, brk) = (at(p.mirror), at(p.brk));
    let mirrored = y - mirror;
    let code = mirrored.abs() + mirror;
    let e = if code < brk {
        if p.gamma_slope == 0.0 || p.gamma_power == 0.0 {
            0.0
        } else {
            (code / p.gamma_slope).max(0.0).powf(1.0 / p.gamma_power) - p.gamma_offset
        }
    } else {
        let slope = gamma_log_slope(p);
        if slope == 0.0 {
            0.0
        } else {
            ((code - p.log_offset) / slope).exp() - p.lin_offset
        }
    };
    e * mirrored.signum()
}

/// The Rec.2100 luma weights, which the surround op is defined with.
const REC2100_LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];

/// The luminance is floored before the power so a colour whose channels are far
/// from zero cannot be gained up by a luminance that is nearly zero. It is the
/// reference library's own guard, at its own 10⁻⁴.
const SURROUND_MIN_LUMINANCE: f32 = 1e-4;

fn surround(exp: f32, dir: Direction, rgb: [f32; 3]) -> [f32; 3] {
    let (power, floor) = match dir {
        Direction::Forward => (exp, SURROUND_MIN_LUMINANCE),
        Direction::Inverse => (1.0 / exp, SURROUND_MIN_LUMINANCE.powf(exp)),
    };
    let luma = REC2100_LUMA[0] * rgb[0] + REC2100_LUMA[1] * rgb[1] + REC2100_LUMA[2] * rgb[2];
    let gain = luma.abs().max(floor).powf(power - 1.0);
    let mut out = [0.0_f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        *o = rgb[c] * gain;
    }
    out
}

fn cdl_forward(p: &CdlParams, rgb: [f32; 3]) -> [f32; 3] {
    let mut v = [0.0_f32; 3];
    for (c, o) in v.iter_mut().enumerate() {
        let mut x = rgb[c] * p.slope[c] + p.offset[c];
        if p.clamp {
            x = x.clamp(0.0, 1.0);
            *o = x.powf(p.power[c]);
        } else {
            // No-clamp keeps the sign: a power of a negative is undefined, so
            // the value passes through untouched, which is OCIO's own reading.
            *o = if x < 0.0 { x } else { x.powf(p.power[c]) };
        }
    }
    let luma = ASC_LUMA[0] * v[0] + ASC_LUMA[1] * v[1] + ASC_LUMA[2] * v[2];
    let mut out = [0.0_f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        *o = luma + p.saturation * (v[c] - luma);
        if p.clamp {
            *o = o.clamp(0.0, 1.0);
        }
    }
    out
}

fn cdl_inverse(p: &CdlParams, rgb: [f32; 3]) -> [f32; 3] {
    // Saturation preserves luma, so the luma of the output is the luma of the
    // value that went in — which makes the saturation step exactly reversible.
    let luma = ASC_LUMA[0] * rgb[0] + ASC_LUMA[1] * rgb[1] + ASC_LUMA[2] * rgb[2];
    let mut out = [0.0_f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let v = if p.saturation == 0.0 {
            luma
        } else {
            luma + (rgb[c] - luma) / p.saturation
        };
        let unpowered = if p.power[c] == 0.0 {
            0.0
        } else if v < 0.0 && !p.clamp {
            v
        } else {
            v.max(0.0).powf(1.0 / p.power[c])
        };
        *o = if p.slope[c] == 0.0 {
            0.0
        } else {
            (unpowered - p.offset[c]) / p.slope[c]
        };
    }
    out
}

fn range_apply(p: &RangeParams, x: f32) -> f32 {
    let mut y = x;
    if let (Some(min_in), Some(max_in), Some(min_out), Some(max_out)) =
        (p.min_in, p.max_in, p.min_out, p.max_out)
    {
        let span = max_in - min_in;
        if span != 0.0 {
            y = (x - min_in) * ((max_out - min_out) / span) + min_out;
        }
    }
    if !p.no_clamp {
        if let Some(lo) = p.min_out {
            y = y.max(lo);
        }
        if let Some(hi) = p.max_out {
            y = y.min(hi);
        }
    }
    y
}

impl Op {
    /// The name this op answers to in a refusal sentence.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Op::Matrix(_) => "a matrix",
            Op::Negatives { curve, .. } => curve.name(),
            Op::Exponent { .. } => "an exponent",
            Op::Pq { .. } => "the ST 2084 curve",
            Op::GammaLog { .. } => "a gamma-and-log curve",
            Op::Surround { .. } => "the Rec.2100 surround compensation",
            Op::MonCurve { .. } => "an exponent with a linear segment",
            Op::Log { .. } => "a log curve",
            Op::Cdl { .. } => "a CDL grade",
            Op::Range(_) => "a range",
            Op::Lut1d { .. } => "a 1D look-up table",
            Op::Lut3d { .. } => "a 3D look-up table",
        }
    }

    /// Apply this step to one colour.
    #[must_use]
    pub fn eval(&self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Op::Matrix(m) => matrix::apply(m, rgb),
            Op::Negatives { style, curve } => {
                let magnitude = [rgb[0].abs(), rgb[1].abs(), rgb[2].abs()];
                let positive = curve.eval(magnitude);
                let mut out = [0.0_f32; 3];
                for (c, o) in out.iter_mut().enumerate() {
                    *o = if rgb[c] >= 0.0 {
                        positive[c]
                    } else {
                        match style {
                            Negatives::Mirror => -positive[c],
                            Negatives::PassThru => rgb[c],
                        }
                    };
                }
                out
            }
            Op::Pq { dir } => {
                let mut out = [0.0_f32; 3];
                for (c, o) in out.iter_mut().enumerate() {
                    *o = match dir {
                        Direction::Forward => pq_forward(rgb[c]),
                        Direction::Inverse => pq_inverse(rgb[c]),
                    };
                }
                out
            }
            Op::GammaLog { params, dir } => {
                let mut out = [0.0_f32; 3];
                for (c, o) in out.iter_mut().enumerate() {
                    *o = match dir {
                        Direction::Forward => gamma_log_forward(params, rgb[c]),
                        Direction::Inverse => gamma_log_inverse(params, rgb[c]),
                    };
                }
                out
            }
            Op::Surround { exp, dir } => surround(*exp, *dir, rgb),
            Op::Exponent { exp, dir } => {
                let mut out = [0.0_f32; 3];
                for (c, o) in out.iter_mut().enumerate() {
                    let e = match dir {
                        Direction::Forward => exp[c],
                        Direction::Inverse => {
                            if exp[c] == 0.0 {
                                0.0
                            } else {
                                1.0 / exp[c]
                            }
                        }
                    };
                    *o = if e == 0.0 {
                        0.0
                    } else {
                        rgb[c].max(0.0).powf(e)
                    };
                }
                out
            }
            Op::MonCurve { gamma, offset, dir } => {
                let mut out = [0.0_f32; 3];
                for (c, o) in out.iter_mut().enumerate() {
                    *o = match dir {
                        Direction::Forward => moncurve_forward(gamma[c], offset[c], rgb[c]),
                        Direction::Inverse => moncurve_inverse(gamma[c], offset[c], rgb[c]),
                    };
                }
                out
            }
            Op::Log { params, dir } => {
                let mut out = [0.0_f32; 3];
                for (c, o) in out.iter_mut().enumerate() {
                    *o = match dir {
                        Direction::Forward => log_forward(params, c, rgb[c]),
                        Direction::Inverse => log_inverse(params, c, rgb[c]),
                    };
                }
                out
            }
            Op::Cdl { params, dir } => match dir {
                Direction::Forward => cdl_forward(params, rgb),
                Direction::Inverse => cdl_inverse(params, rgb),
            },
            Op::Range(p) => {
                let mut out = [0.0_f32; 3];
                for (c, o) in out.iter_mut().enumerate() {
                    *o = range_apply(p, rgb[c]);
                }
                out
            }
            Op::Lut1d { curve, dir } => match dir {
                Direction::Forward => curve.sample(rgb),
                Direction::Inverse => curve.sample_inverse(rgb),
            },
            Op::Lut3d { cube } => cube.sample(rgb),
        }
    }

    /// The step that undoes this one, or a refusal naming what could not be
    /// undone. `what` names the colour space or file being inverted, so the
    /// sentence the user reads points somewhere.
    pub fn inverted(&self, what: &str) -> Result<Op> {
        Ok(match self {
            Op::Matrix(m) => Op::Matrix(matrix::invert(m)?),
            // Both styles are odd about zero, so undoing the curve inside
            // undoes the whole thing.
            Op::Negatives { style, curve } => Op::Negatives {
                style: *style,
                curve: Box::new(curve.inverted(what)?),
            },
            Op::Pq { dir } => Op::Pq { dir: dir.flipped() },
            Op::GammaLog { params, dir } => Op::GammaLog {
                params: *params,
                dir: dir.flipped(),
            },
            Op::Surround { exp, dir } => Op::Surround {
                exp: *exp,
                dir: dir.flipped(),
            },
            Op::Exponent { exp, dir } => Op::Exponent {
                exp: *exp,
                dir: dir.flipped(),
            },
            Op::MonCurve { gamma, offset, dir } => Op::MonCurve {
                gamma: *gamma,
                offset: *offset,
                dir: dir.flipped(),
            },
            Op::Log { params, dir } => Op::Log {
                params: *params,
                dir: dir.flipped(),
            },
            Op::Cdl { params, dir } => Op::Cdl {
                params: *params,
                dir: dir.flipped(),
            },
            Op::Range(p) => Op::Range(RangeParams {
                min_in: p.min_out,
                max_in: p.max_out,
                min_out: p.min_in,
                max_out: p.max_in,
                no_clamp: p.no_clamp,
            }),
            Op::Lut1d { curve, dir } => {
                curve.check_invertible(what)?;
                Op::Lut1d {
                    curve: curve.clone(),
                    dir: dir.flipped(),
                }
            }
            Op::Lut3d { .. } => {
                return Err(ColourError::Unsupported3dLutInverse {
                    space: what.to_string(),
                })
            }
        })
    }

    /// Whether this step treats red, green and blue separately — the test that
    /// decides whether a chain can bake to curves instead of a cube (§5.1).
    #[must_use]
    pub fn is_channel_independent(&self) -> bool {
        match self {
            // The surround scales all three channels by their own luminance,
            // so it mixes them exactly as a matrix does.
            Op::Matrix(_) | Op::Lut3d { .. } | Op::Surround { .. } => false,
            // The wrapper only decides what happens below zero; whether the
            // channels stay apart is the curve inside's business.
            Op::Negatives { curve, .. } => curve.is_channel_independent(),
            // Saturation mixes the channels; slope/offset/power alone do not.
            Op::Cdl { params, .. } => params.saturation == 1.0,
            _ => true,
        }
    }

    /// Whether this step can appear in a factorised artefact at all: the
    /// channel-independent steps, plus matrices, which get a stage of their own.
    #[must_use]
    pub fn is_factorable(&self) -> bool {
        matches!(self, Op::Matrix(_)) || self.is_channel_independent()
    }
}

/// A resolved chain: the flat, ordered list of steps that *is* a transform once
/// every config-level indirection has been followed (docs/impl/ocio.md §4.2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chain {
    pub ops: Vec<Op>,
}

impl Chain {
    /// Build a chain, **folding neighbouring matrices into one**.
    ///
    /// The fold is fidelity, not tidiness, and it is the second half of the fix
    /// [`crate::matrix::Matrix34`] describes. A config's `from_reference` is
    /// normally its `to_reference` inverted, so `A → B` where both declare the
    /// same primaries matrix resolves to `M` immediately followed by `M⁻¹` —
    /// mathematically nothing at all, but evaluated one after the other in
    /// single precision it detours through an intermediate that can be five
    /// orders of magnitude larger than the answer, and brings back the rounding
    /// of that intermediate rather than the answer. ACEScc → ACEScg on the
    /// legacy ACES config is exactly this: blue arrives at −246 and comes back
    /// to 0.045 having lost 2 × 10⁻⁵ on the way. The reference library composes
    /// adjacent matrices for the same reason, so this is agreement with it and
    /// not a Lumit invention.
    #[must_use]
    pub fn new(ops: Vec<Op>) -> Self {
        let mut folded: Vec<Op> = Vec::with_capacity(ops.len());
        for op in ops {
            match (folded.last_mut(), &op) {
                (Some(Op::Matrix(prev)), Op::Matrix(m)) => *prev = matrix::concat(prev, m),
                _ => folded.push(op),
            }
            // And a fold that came out as the identity leaves altogether,
            // which is the other half of the cancellation: keeping a matrix
            // that does nothing would still spread one channel's overflow
            // across the other two as NaN (`matrix::is_identity`).
            if matches!(folded.last(), Some(Op::Matrix(m)) if matrix::is_identity(m)) {
                folded.pop();
            }
        }
        Self { ops: folded }
    }

    /// The chain that does nothing.
    #[must_use]
    pub fn identity() -> Self {
        Self { ops: Vec::new() }
    }

    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.ops.is_empty()
    }

    /// Run the whole chain, in order.
    #[must_use]
    pub fn eval(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.ops.iter().fold(rgb, |c, op| op.eval(c))
    }

    /// This chain backwards: the steps reversed, each one inverted.
    pub fn inverted(&self, what: &str) -> Result<Chain> {
        let mut ops = Vec::with_capacity(self.ops.len());
        for op in self.ops.iter().rev() {
            ops.push(op.inverted(what)?);
        }
        Ok(Chain::new(ops))
    }

    /// `self` then `next`, as one chain. Joining is where a matrix meets its
    /// own inverse, so it folds (see [`Chain::new`]).
    #[must_use]
    pub fn then(mut self, next: Chain) -> Chain {
        self.ops.extend(next.ops);
        Chain::new(self.ops)
    }

    /// Whether the whole chain can bake to the cheap factorised form (§5.1).
    #[must_use]
    pub fn is_factorable(&self) -> bool {
        self.ops.iter().all(Op::is_factorable)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    fn round_trip(op: &Op, samples: &[[f32; 3]], tol: f32) {
        let back = op.inverted("test").expect("invertible");
        for s in samples {
            let there = op.eval(*s);
            let home = back.eval(there);
            assert!(close(home, *s, tol), "{s:?} → {there:?} → {home:?}");
        }
    }

    const SAMPLES: [[f32; 3]; 6] = [
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.18, 0.18, 0.18],
        [0.05, 0.5, 0.95],
        [0.2, 0.4, 0.6],
        [0.9, 0.1, 0.35],
    ];

    #[test]
    fn a_matrix_round_trips_through_its_inverse() {
        let op = Op::Matrix([
            1.2, -0.1, -0.1, 0.01, -0.05, 1.1, -0.05, 0.0, 0.0, -0.2, 1.2, -0.02,
        ]);
        round_trip(&op, &SAMPLES, 1e-5);
    }

    #[test]
    fn an_exponent_matches_an_f64_reference() {
        let op = Op::Exponent {
            exp: [2.2, 2.4, 1.8],
            dir: Direction::Forward,
        };
        let got = op.eval([0.5, 0.25, 0.75]);
        let want = [
            (0.5_f64).powf(2.2) as f32,
            (0.25_f64).powf(2.4) as f32,
            (0.75_f64).powf(1.8) as f32,
        ];
        assert!(close(got, want, 1e-6), "{got:?} vs {want:?}");
        round_trip(&op, &SAMPLES, 1e-5);
    }

    #[test]
    fn an_exponent_clamps_negatives_rather_than_inventing_a_root() {
        let op = Op::Exponent {
            exp: [2.2; 3],
            dir: Direction::Forward,
        };
        assert_eq!(op.eval([-0.5, -1.0, -0.01]), [0.0; 3]);
    }

    #[test]
    fn the_moncurve_is_the_exact_srgb_curve() {
        // γ = 2.4, offset = 0.055 is sRGB's own shape, and the forward
        // direction is the decode (display code values → linear light). Its
        // mathematically exact break sits at 0.0392857 with a straight-segment
        // slope of 1/12.9232 — the numbers the rounded 0.04045 and 12.92 in the
        // published standard approximate. Getting either wrong shows up here.
        let (x_break, slope) = moncurve_shape(2.4, 0.055).expect("a real shape");
        assert!((x_break - 0.0392857).abs() < 1e-5, "break {x_break}");
        assert!((slope - 1.0 / 12.92321).abs() < 1e-5, "slope {slope}");

        let op = Op::MonCurve {
            gamma: [2.4; 3],
            offset: [0.055; 3],
            dir: Direction::Forward,
        };
        // White stays white, black stays black, and the two halves meet.
        assert!(close(op.eval([1.0; 3]), [1.0; 3], 1e-6));
        assert!(close(op.eval([0.0; 3]), [0.0; 3], 1e-6));
        let below = op.eval([x_break - 1e-5; 3]);
        let above = op.eval([x_break + 1e-5; 3]);
        assert!(close(below, above, 1e-4), "{below:?} vs {above:?}");
        round_trip(&op, &SAMPLES, 1e-5);
    }

    #[test]
    fn the_moncurve_carries_negatives_down_its_straight_segment() {
        let op = Op::MonCurve {
            gamma: [2.4; 3],
            offset: [0.055; 3],
            dir: Direction::Forward,
        };
        let got = op.eval([-0.02; 3]);
        assert!(got[0] < 0.0, "a negative stayed negative: {got:?}");
        round_trip(&op, &[[-0.02; 3]], 1e-5);
    }

    #[test]
    fn a_plain_log_is_the_logarithm() {
        let op = Op::Log {
            params: LogParams::plain(10.0),
            dir: Direction::Forward,
        };
        let got = op.eval([1.0, 10.0, 100.0]);
        assert!(close(got, [0.0, 1.0, 2.0], 1e-5), "{got:?}");
        round_trip(&op, &[[0.18, 1.0, 8.0]], 1e-4);
    }

    #[test]
    fn a_camera_log_joins_its_two_halves_smoothly() {
        // ACEScct's own numbers, as a config states them.
        let params = LogParams {
            base: 2.0,
            lin_side_slope: [1.0; 3],
            lin_side_offset: [0.0; 3],
            log_side_slope: [1.0 / 17.52; 3],
            log_side_offset: [9.72 / 17.52; 3],
            lin_side_break: Some([0.0078125; 3]),
            linear_slope: Some([10.540_238; 3]),
        };
        let op = Op::Log {
            params,
            dir: Direction::Forward,
        };
        let b = 0.0078125_f32;
        let below = op.eval([b - 1e-6; 3]);
        let above = op.eval([b + 1e-6; 3]);
        assert!(close(below, above, 1e-4), "{below:?} vs {above:?}");
        // The published ACEScct value at the break.
        assert!((above[0] - 0.155251).abs() < 1e-4, "{above:?}");
        round_trip(&op, &[[0.001, 0.18, 16.0]], 1e-4);
    }

    #[test]
    fn a_camera_log_picks_a_smooth_slope_when_the_config_states_none() {
        let mut params = LogParams {
            base: 2.0,
            lin_side_slope: [1.0; 3],
            lin_side_offset: [0.0; 3],
            log_side_slope: [1.0 / 17.52; 3],
            log_side_offset: [9.72 / 17.52; 3],
            lin_side_break: Some([0.0078125; 3]),
            linear_slope: None,
        };
        let (slope, _) = linear_segment(&params, 0);
        // ACEScct's published slope is exactly this tangent.
        assert!((slope - 10.540_238).abs() < 1e-3, "slope {slope}");
        params.linear_slope = Some([slope; 3]);
        let op = Op::Log {
            params,
            dir: Direction::Forward,
        };
        round_trip(&op, &[[0.0, 0.001, 0.18]], 1e-4);
    }

    /// HLG's numbers, which are the ones the built-in registry hands this op.
    fn hlg() -> GammaLogParams {
        let a = 0.178_832_77_f64;
        GammaLogParams {
            mirror: 0.0,
            brk: 0.25,
            gamma_power: 0.5,
            gamma_slope: 1.0,
            gamma_offset: 0.0,
            base: std::f32::consts::E,
            log_slope: a as f32,
            log_offset: ((4.0_f64).ln() * a + 0.5 - a * (4.0 * a).ln()) as f32,
            lin_offset: -((1.0 - 4.0 * a) / 4.0) as f32,
        }
    }

    #[test]
    fn a_gamma_log_curve_joins_its_two_halves_and_reverses() {
        let op = Op::GammaLog {
            params: hlg(),
            dir: Direction::Forward,
        };
        // The square root reaches 0.5 at a twelfth of the scene range, which is
        // exactly where the logarithm takes over.
        let below = op.eval([0.25 - 1e-6; 3]);
        let above = op.eval([0.25 + 1e-6; 3]);
        assert!(close(below, [0.5; 3], 1e-5), "{below:?}");
        assert!(close(below, above, 1e-5), "{below:?} vs {above:?}");
        // HLG's own published pair: 18% grey of the scene range encodes to 0.42.
        assert!(close(op.eval([0.1764; 3]), [0.42; 3], 1e-5));
        // And it is odd about the mirror point rather than clamping.
        assert!(close(op.eval([-0.1764; 3]), [-0.42; 3], 1e-5));
        round_trip(&op, &SAMPLES, 1e-5);
        round_trip(&op, &[[-0.05, 0.4, 2.9]], 1e-5);
    }

    #[test]
    fn the_surround_scales_by_luminance_and_reverses() {
        let op = Op::Surround {
            exp: 1.0 / 1.2,
            dir: Direction::Forward,
        };
        // The luma weights sum to one, so a neutral is its own luminance and
        // the whole thing collapses to a power of it.
        let got = op.eval([0.5; 3]);
        assert!(close(got, [(0.5_f32).powf(1.0 / 1.2); 3], 1e-6), "{got:?}");
        round_trip(&op, &SAMPLES, 1e-4);
    }

    #[test]
    fn cdl_matches_the_asc_formula_by_hand() {
        let params = CdlParams {
            slope: [1.2, 0.9, 1.05],
            offset: [0.01, -0.02, 0.0],
            power: [0.9, 1.1, 1.0],
            saturation: 1.3,
            clamp: true,
        };
        let input = [0.4_f32, 0.5, 0.6];
        let mut v = [0.0_f32; 3];
        for c in 0..3 {
            v[c] = (input[c] * params.slope[c] + params.offset[c])
                .clamp(0.0, 1.0)
                .powf(params.power[c]);
        }
        let luma = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        let want = [
            (luma + 1.3 * (v[0] - luma)).clamp(0.0, 1.0),
            (luma + 1.3 * (v[1] - luma)).clamp(0.0, 1.0),
            (luma + 1.3 * (v[2] - luma)).clamp(0.0, 1.0),
        ];
        let op = Op::Cdl {
            params,
            dir: Direction::Forward,
        };
        assert!(close(op.eval(input), want, 1e-6));
    }

    #[test]
    fn cdl_round_trips_where_nothing_clamped() {
        let op = Op::Cdl {
            params: CdlParams {
                slope: [1.1, 0.95, 1.02],
                offset: [0.02, -0.01, 0.005],
                power: [0.95, 1.05, 1.0],
                saturation: 1.2,
                clamp: true,
            },
            dir: Direction::Forward,
        };
        round_trip(&op, &[[0.3, 0.45, 0.55], [0.2, 0.4, 0.6]], 1e-4);
    }

    #[test]
    fn a_saturation_only_cdl_mixes_channels_and_so_cannot_factorise() {
        let mixing = Op::Cdl {
            params: CdlParams {
                saturation: 1.4,
                ..CdlParams::default()
            },
            dir: Direction::Forward,
        };
        assert!(!mixing.is_channel_independent());
        let plain = Op::Cdl {
            params: CdlParams {
                slope: [1.1; 3],
                ..CdlParams::default()
            },
            dir: Direction::Forward,
        };
        assert!(plain.is_channel_independent());
    }

    #[test]
    fn a_range_scales_and_clamps_and_reverses() {
        let op = Op::Range(RangeParams {
            min_in: Some(0.0),
            max_in: Some(1.0),
            min_out: Some(0.0),
            max_out: Some(0.5),
            no_clamp: false,
        });
        assert!(close(op.eval([0.5; 3]), [0.25; 3], 1e-6));
        assert!(close(op.eval([2.0; 3]), [0.5; 3], 1e-6), "clamped");
        round_trip(&op, &[[0.2, 0.4, 0.6]], 1e-6);
    }

    #[test]
    fn a_clamp_only_range_leaves_the_value_alone() {
        let op = Op::Range(RangeParams {
            min_out: Some(0.0),
            ..RangeParams::default()
        });
        assert!(close(op.eval([0.7, -0.3, 4.0]), [0.7, 0.0, 4.0], 1e-6));
    }

    #[test]
    fn a_3d_table_refuses_to_invert_and_names_the_space() {
        let cube = Cube::new("t", 2, [0.0; 3], [1.0; 3], vec![[0.0; 3]; 8]).expect("well-formed");
        let err = Op::Lut3d { cube }.inverted("ACES - ACEScct");
        assert!(
            matches!(&err, Err(ColourError::Unsupported3dLutInverse { space }) if space == "ACES - ACEScct"),
            "{err:?}"
        );
    }

    #[test]
    fn a_chain_inverts_backwards_step_by_step() {
        let chain = Chain::new(vec![
            Op::Exponent {
                exp: [2.2; 3],
                dir: Direction::Forward,
            },
            Op::Matrix([
                1.1, 0.05, -0.15, 0.0, 0.0, 1.0, 0.0, 0.0, -0.02, 0.03, 0.99, 0.0,
            ]),
        ]);
        let back = chain.inverted("test").expect("invertible");
        assert!(matches!(back.ops.first(), Some(Op::Matrix(_))));
        let c = [0.3, 0.5, 0.7];
        assert!(close(back.eval(chain.eval(c)), c, 1e-4));
    }

    /// The regression for the one thing the legacy ACES fixture caught that no
    /// hand-built row had: `ACES - ACEScc → ACES - ACEScg` resolves to the
    /// AP1→AP0 matrix immediately followed by its own inverse, because both
    /// spaces declare the same `to_reference` matrix. Applied one after the
    /// other in single precision, an input whose red has clamped to the top of
    /// the ACEScc table drags blue out to −246 and brings it back 2.3 × 10⁻⁵
    /// wrong — five hundred times the row's own tolerance, on a chain that is
    /// mathematically nothing at all. The fold, and the double-precision
    /// coefficients it composes, are what make the pair cancel.
    #[test]
    fn a_matrix_meeting_its_own_inverse_leaves_nothing_behind() {
        let m = Op::Matrix([
            0.695452,
            0.140679,
            0.163869,
            0.0, //
            0.0447946,
            0.859671,
            0.0955343,
            0.0, //
            -0.00552588,
            0.00402521,
            1.0015,
            0.0,
        ]);
        let chain = Chain::new(vec![m.clone(), m.inverted("test").expect("invertible")]);
        assert!(chain.is_identity(), "the pair cancelled away entirely");
        // What the ACEScc curve hands the matrix at the failing probe.
        let c = [65504.0, 28684.941, 0.045_310_933];
        assert_eq!(chain.eval(c), c);
        // And it must *vanish* rather than merely compose. A kept matrix, even
        // one within 10⁻¹⁶ of the identity, spreads one channel's overflow
        // across the other two — `inf × 10⁻¹⁶` is still infinity, and the sum
        // that follows is a NaN. The reference library carries the finite
        // channel straight through, and so must this.
        let overflowed = [f32::INFINITY, f32::INFINITY, 1.479_693_5e18];
        assert_eq!(chain.eval(overflowed), overflowed);
    }

    #[test]
    fn a_chain_with_a_matrix_still_factorises_but_a_cube_does_not() {
        let with_matrix = Chain::new(vec![
            Op::Exponent {
                exp: [2.2; 3],
                dir: Direction::Forward,
            },
            Op::Matrix(matrix::IDENTITY),
        ]);
        assert!(with_matrix.is_factorable());
        let cube = Cube::new("t", 2, [0.0; 3], [1.0; 3], vec![[0.0; 3]; 8]).expect("well-formed");
        assert!(!Chain::new(vec![Op::Lut3d { cube }]).is_factorable());
    }
}
