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

/// Which family of curves a grading transform works in. The reference library
/// has these three and no more, and each one carries its own defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradingStyle {
    #[default]
    Log,
    Lin,
    Video,
}

/// A grading value as a config writes it: one number per channel, and a master
/// that applies to all three on top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradingRgbm {
    pub rgb: [f64; 3],
    pub master: f64,
}

impl GradingRgbm {
    /// The same number in all four slots, which is how every default reads.
    #[must_use]
    pub fn flat(value: f64) -> Self {
        Self {
            rgb: [value; 3],
            master: value,
        }
    }
}

/// "No clamp" is not a flag in a config, it is a number so far out that the
/// clamp cannot bite. The exact value matters: it is what says a grade does
/// nothing at all and may copy its input straight through.
pub const NO_CLAMP_BLACK: f64 = -f64::MAX;
pub const NO_CLAMP_WHITE: f64 = f64::MAX;

/// `GradingPrimaryTransform`: the lift/gamma/gain grade a colourist works in,
/// in whichever of the three styles the config names.
///
/// The values are held as the config writes them, in double, because the
/// reference library works out its per channel numbers in double and only then
/// rounds to f32 for the pixel loop. Doing that arithmetic in f32 instead moves
/// the answer in the last place or two, which a golden row would see.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradingPrimary {
    pub style: GradingStyle,
    pub brightness: GradingRgbm,
    pub contrast: GradingRgbm,
    pub gamma: GradingRgbm,
    pub offset: GradingRgbm,
    pub exposure: GradingRgbm,
    pub lift: GradingRgbm,
    pub gain: GradingRgbm,
    pub saturation: f64,
    /// The contrast pivot, which each style reads differently: log puts it at
    /// `0.5 + pivot / 2`, linear at `0.18 × 2^pivot`, video at the black pivot.
    pub pivot: f64,
    pub pivot_black: f64,
    pub pivot_white: f64,
    pub clamp_black: f64,
    pub clamp_white: f64,
}

impl GradingPrimary {
    /// A style's own defaults, which is what a config's unstated keys mean.
    #[must_use]
    pub fn new(style: GradingStyle) -> Self {
        Self {
            style,
            brightness: GradingRgbm::flat(0.0),
            contrast: GradingRgbm::flat(1.0),
            gamma: GradingRgbm::flat(1.0),
            offset: GradingRgbm::flat(0.0),
            exposure: GradingRgbm::flat(0.0),
            lift: GradingRgbm::flat(0.0),
            gain: GradingRgbm::flat(1.0),
            saturation: 1.0,
            // Log counts its pivot in code value about the middle of the range,
            // the other two in stops about mid grey.
            pivot: match style {
                GradingStyle::Log => -0.2,
                GradingStyle::Lin | GradingStyle::Video => 0.18,
            },
            pivot_black: 0.0,
            pivot_white: 1.0,
            clamp_black: NO_CLAMP_BLACK,
            clamp_white: NO_CLAMP_WHITE,
        }
    }
}

/// A grading value with the two extra numbers a tone band needs: where the band
/// starts and how wide it is. A config spells those `start`/`width`,
/// `start`/`pivot` or `center`/`width` depending on which band it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradingRgbmsw {
    pub rgb: [f64; 3],
    pub master: f64,
    pub start: f64,
    pub width: f64,
}

impl GradingRgbmsw {
    #[must_use]
    pub fn new(start: f64, width: f64) -> Self {
        Self {
            rgb: [1.0; 3],
            master: 1.0,
            start,
            width,
        }
    }
}

/// `GradingToneTransform`'s five bands and its S-shaped contrast, as the config
/// writes them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradingToneValues {
    pub style: GradingStyle,
    pub blacks: GradingRgbmsw,
    pub shadows: GradingRgbmsw,
    pub midtones: GradingRgbmsw,
    pub highlights: GradingRgbmsw,
    pub whites: GradingRgbmsw,
    pub scontrast: f64,
}

impl GradingToneValues {
    /// A style's own defaults. The five bands sit in different places in a log
    /// range than in a linear one, which is what these numbers are.
    #[must_use]
    pub fn new(style: GradingStyle) -> Self {
        let (blacks, shadows, midtones, highlights, whites) = match style {
            GradingStyle::Log => ((0.4, 0.4), (0.5, 0.0), (0.4, 0.6), (0.3, 1.0), (0.4, 0.5)),
            GradingStyle::Lin => ((0.0, 4.0), (2.0, -7.0), (0.0, 8.0), (-2.0, 9.0), (0.0, 8.0)),
            GradingStyle::Video => ((0.4, 0.4), (0.6, 0.0), (0.4, 0.7), (0.2, 1.0), (0.5, 0.5)),
        };
        Self {
            style,
            blacks: GradingRgbmsw::new(blacks.0, blacks.1),
            shadows: GradingRgbmsw::new(shadows.0, shadows.1),
            midtones: GradingRgbmsw::new(midtones.0, midtones.1),
            highlights: GradingRgbmsw::new(highlights.0, highlights.1),
            whites: GradingRgbmsw::new(whites.0, whites.1),
            scontrast: 1.0,
        }
    }
}

/// The numbers the tone pixel loop reads: knot positions, values and slopes for
/// every band, worked out once from the config's values.
///
/// This is the reference library's own `GradingTonePreRender`, and it is kept
/// rather than recomputed because it is a few hundred operations and the bake
/// asks the op sixteen thousand questions.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct TonePreRender {
    bypass: bool,
    top: f32,
    top_sc: f32,
    bottom: f32,
    pivot: f32,
    highlights_start: f64,
    highlights_width: f64,
    whites_start: f64,
    whites_width: f64,
    shadows_start: f64,
    shadows_width: f64,
    blacks_start: f64,
    blacks_width: f64,
    mid_x: [[f32; 6]; 4],
    mid_y: [[f32; 6]; 4],
    mid_m: [[f32; 6]; 4],
    hs_x: [[[f32; 3]; 4]; 2],
    hs_y: [[[f32; 3]; 4]; 2],
    hs_m: [[[f32; 2]; 4]; 2],
    wb_x: [[[f32; 2]; 4]; 2],
    wb_y: [[[f32; 2]; 4]; 2],
    wb_m: [[[f32; 2]; 4]; 2],
    wb_gain: [[f32; 4]; 2],
    sc_x: [[f32; 4]; 2],
    sc_y: [[f32; 4]; 2],
    sc_m: [[f32; 2]; 2],
}

/// One tone grade: the config's values, and the knots they work out to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradingTone {
    pub values: GradingToneValues,
    pre: TonePreRender,
}

impl GradingTone {
    #[must_use]
    pub fn new(values: GradingToneValues) -> Self {
        Self {
            pre: tone_prerender(&values),
            values,
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
    /// `GradingPrimaryTransform`. Boxed because the values run to a few hundred
    /// bytes and every other op in every chain would pay for them.
    GradingPrimary {
        params: Box<GradingPrimary>,
        dir: Direction,
    },
    /// `GradingToneTransform`, values and pre-render together.
    GradingTone {
        params: Box<GradingTone>,
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

// ---------------------------------------------------------------------------
// The two grading transforms, ported from the reference library's CPU code:
// GradingPrimary.cpp and GradingPrimaryOpCPU.cpp, GradingTone.cpp and
// GradingToneOpCPU.cpp. Everything below follows those files step for step,
// including where they round to f32 and where they do not, because a golden row
// generated by that library sees the difference (docs/impl/ocio.md §4.1).
// ---------------------------------------------------------------------------

/// `std::min(std::max(x, lo), hi)` written out. Rust's own `min` and `max` step
/// over a NaN where C++'s carry it through, and a grade at the far end of a
/// probe set can make one.
fn cpp_clamp(x: f32, lo: f32, hi: f32) -> f32 {
    let y = if x < lo { lo } else { x };
    if hi < y {
        hi
    } else {
        y
    }
}

/// The master slot, which runs on all three channels rather than on one.
const CH_MASTER: usize = 3;

/// Apply one curve to the channel a step names, or to all three for the master.
fn on_channels(channel: usize, out: &mut [f32; 3], f: impl Fn(f32) -> f32) {
    if channel == CH_MASTER {
        for c in out.iter_mut() {
            *c = f(*c);
        }
    } else {
        out[channel] = f(out[channel]);
    }
}

/// The per channel numbers the primary pixel loop reads, worked out from the
/// config's values and the direction it runs in.
#[derive(Debug, Clone, Copy)]
struct PrimaryComputed {
    brightness: [f32; 3],
    contrast: [f32; 3],
    gamma: [f32; 3],
    offset: [f32; 3],
    exposure: [f32; 3],
    slope: [f32; 3],
    pivot: f32,
    /// Whether the power step is the identity, in which case the reference
    /// skips it rather than raising to one.
    power_identity: bool,
    /// Whether the whole grade does nothing, in which case it copies its input.
    bypass: bool,
}

fn primary_prerender(v: &GradingPrimary, dir: Direction) -> PrimaryComputed {
    let mut c = PrimaryComputed {
        brightness: [0.0; 3],
        contrast: [1.0; 3],
        gamma: [1.0; 3],
        offset: [0.0; 3],
        exposure: [1.0; 3],
        slope: [1.0; 3],
        pivot: 0.0,
        power_identity: true,
        bypass: v.saturation == 1.0
            && v.clamp_black == NO_CLAMP_BLACK
            && v.clamp_white == NO_CLAMP_WHITE,
    };
    let inverse = matches!(dir, Direction::Inverse);
    match v.style {
        GradingStyle::Log => {
            for i in 0..3 {
                let b = (v.brightness.master + v.brightness.rgb[i]) * 6.25 / 1023.0;
                let ct = v.contrast.master * v.contrast.rgb[i];
                let g = v.gamma.master * v.gamma.rgb[i];
                if inverse {
                    c.brightness[i] = -(b as f32);
                    c.contrast[i] = (1.0 / if ct == 0.0 { 1.0 } else { ct }) as f32;
                    c.gamma[i] = g as f32;
                } else {
                    c.brightness[i] = b as f32;
                    c.contrast[i] = ct as f32;
                    c.gamma[i] = (1.0 / g) as f32;
                }
            }
            c.power_identity = c.gamma == [1.0; 3];
            c.pivot = (0.5 + v.pivot * 0.5) as f32;
            c.bypass =
                c.bypass && c.power_identity && c.brightness == [0.0; 3] && c.contrast == [1.0; 3];
        }
        GradingStyle::Lin => {
            for i in 0..3 {
                let o = v.offset.master + v.offset.rgb[i];
                let e = (v.exposure.master + v.exposure.rgb[i]) as f32;
                let ct = v.contrast.master * v.contrast.rgb[i];
                if inverse {
                    c.offset[i] = -(o as f32);
                    c.exposure[i] = 1.0 / 2.0_f32.powf(e);
                    c.contrast[i] = (1.0 / ct) as f32;
                } else {
                    c.offset[i] = o as f32;
                    c.exposure[i] = 2.0_f32.powf(e);
                    c.contrast[i] = ct as f32;
                }
            }
            // In this style the power is the contrast, so that is what decides
            // whether the power step runs at all.
            c.power_identity = c.contrast == [1.0; 3];
            c.pivot = (0.18 * 2.0_f64.powf(v.pivot)) as f32;
            c.bypass =
                c.bypass && c.power_identity && c.exposure == [1.0; 3] && c.offset == [0.0; 3];
        }
        GradingStyle::Video => {
            for i in 0..3 {
                let gain = v.gain.master * v.gain.rgb[i];
                let gain = if gain == 0.0 { 1.0 } else { gain };
                let lift = v.lift.master + v.lift.rgb[i];
                let o = v.offset.master + v.offset.rgb[i] + lift;
                let g = v.gamma.master * v.gamma.rgb[i];
                if inverse {
                    c.offset[i] = -(o as f32);
                    c.slope[i] = ((v.pivot_white / gain + (lift - v.pivot_black))
                        / (v.pivot_white - v.pivot_black)) as f32;
                    c.gamma[i] = g as f32;
                } else {
                    let den = v.pivot_white / gain + lift - v.pivot_black;
                    c.offset[i] = o as f32;
                    c.slope[i] = ((v.pivot_white - v.pivot_black)
                        / if den == 0.0 { 1.0 } else { den })
                        as f32;
                    c.gamma[i] = (1.0 / g) as f32;
                }
            }
            c.power_identity = c.gamma == [1.0; 3];
            c.bypass = c.bypass && c.power_identity && c.slope == [1.0; 3] && c.offset == [0.0; 3];
        }
    }
    c
}

fn primary_offset(pix: &mut [f32; 3], offset: [f32; 3]) {
    for i in 0..3 {
        pix[i] += offset[i];
    }
}

fn primary_slope(pix: &mut [f32; 3], slope: [f32; 3]) {
    for i in 0..3 {
        pix[i] *= slope[i];
    }
}

fn primary_contrast(pix: &mut [f32; 3], contrast: [f32; 3], pivot: f32) {
    for i in 0..3 {
        pix[i] = (pix[i] - pivot) * contrast[i] + pivot;
    }
}

/// The linear style's contrast: a power about the pivot, taken on the magnitude
/// so a negative keeps its sign rather than becoming a not-a-number.
fn primary_lin_contrast(pix: &mut [f32; 3], contrast: [f32; 3], pivot: f32) {
    for i in 0..3 {
        pix[i] = (pix[i] / pivot).abs().powf(contrast[i]) * pivot.copysign(pix[i]);
    }
}

fn primary_gamma(pix: &mut [f32; 3], gamma: [f32; 3], black: f32, white: f32) {
    for i in 0..3 {
        let d = pix[i] - black;
        pix[i] = (d.abs() / (white - black)).powf(gamma[i]) * 1.0_f32.copysign(d) * (white - black)
            + black;
    }
}

fn primary_saturation(pix: &mut [f32; 3], saturation: f32) {
    if saturation != 1.0 {
        let src = *pix;
        let luma = pix[0] * ASC_LUMA[0] + pix[1] * ASC_LUMA[1] + pix[2] * ASC_LUMA[2];
        for i in 0..3 {
            pix[i] = luma + saturation * (src[i] - luma);
        }
    }
}

fn primary_clamp(pix: &mut [f32; 3], black: f32, white: f32) {
    for c in pix.iter_mut() {
        *c = cpp_clamp(*c, black, white);
    }
}

fn primary_eval(v: &GradingPrimary, dir: Direction, rgb: [f32; 3]) -> [f32; 3] {
    let c = primary_prerender(v, dir);
    if c.bypass {
        return rgb;
    }
    let saturation = v.saturation as f32;
    // Undoing a saturation of zero is not possible, so the reference leaves the
    // step out rather than dividing by nothing.
    let sat_inverse = 1.0 / if saturation == 0.0 { 1.0 } else { saturation };
    let (black, white) = (v.pivot_black as f32, v.pivot_white as f32);
    let (clamp_black, clamp_white) = (v.clamp_black as f32, v.clamp_white as f32);
    let mut p = rgb;
    match (v.style, dir) {
        (GradingStyle::Log, Direction::Forward) => {
            primary_offset(&mut p, c.brightness);
            primary_contrast(&mut p, c.contrast, c.pivot);
            if !c.power_identity {
                primary_gamma(&mut p, c.gamma, black, white);
            }
            primary_saturation(&mut p, saturation);
            primary_clamp(&mut p, clamp_black, clamp_white);
        }
        (GradingStyle::Log, Direction::Inverse) => {
            primary_clamp(&mut p, clamp_black, clamp_white);
            primary_saturation(&mut p, sat_inverse);
            if !c.power_identity {
                primary_gamma(&mut p, c.gamma, black, white);
            }
            primary_contrast(&mut p, c.contrast, c.pivot);
            primary_offset(&mut p, c.brightness);
        }
        (GradingStyle::Lin, Direction::Forward) => {
            primary_offset(&mut p, c.offset);
            primary_slope(&mut p, c.exposure);
            if !c.power_identity {
                primary_lin_contrast(&mut p, c.contrast, c.pivot);
            }
            primary_saturation(&mut p, saturation);
            primary_clamp(&mut p, clamp_black, clamp_white);
        }
        (GradingStyle::Lin, Direction::Inverse) => {
            primary_clamp(&mut p, clamp_black, clamp_white);
            primary_saturation(&mut p, sat_inverse);
            if !c.power_identity {
                primary_lin_contrast(&mut p, c.contrast, c.pivot);
            }
            primary_slope(&mut p, c.exposure);
            primary_offset(&mut p, c.offset);
        }
        (GradingStyle::Video, Direction::Forward) => {
            primary_offset(&mut p, c.offset);
            primary_contrast(&mut p, c.slope, black);
            if !c.power_identity {
                primary_gamma(&mut p, c.gamma, black, white);
            }
            primary_saturation(&mut p, saturation);
            primary_clamp(&mut p, clamp_black, clamp_white);
        }
        (GradingStyle::Video, Direction::Inverse) => {
            primary_clamp(&mut p, clamp_black, clamp_white);
            primary_saturation(&mut p, sat_inverse);
            if !c.power_identity {
                primary_gamma(&mut p, c.gamma, black, white);
            }
            primary_contrast(&mut p, c.slope, black);
            primary_offset(&mut p, c.offset);
        }
    }
    p
}

// -- the tone grade ---------------------------------------------------------

fn tone_channel(v: &GradingRgbmsw, channel: usize) -> f32 {
    match channel {
        0 => v.rgb[0] as f32,
        1 => v.rgb[1] as f32,
        2 => v.rgb[2] as f32,
        _ => v.master as f32,
    }
}

/// The two-piece quadratic the shadow and highlight bands are built from,
/// forwards. `x1` is the joint, `y1` the value there that makes the halves meet.
///
/// The reference's own version takes the two ends' values as well; both places
/// it is called from pass the ends unmoved and the joint at the midpoint, so
/// those three are worked out here instead of passed in.
fn faux_cubic_forward(t: f64, x0: f64, x2: f64, m0: f64, m2: f64) -> f64 {
    let (y0, y2) = (x0, x2);
    let x1 = x0 + (x2 - x0) * 0.5;
    let y1 = (0.5 / ((x2 - x1) + (x1 - x0)))
        * ((2.0 * y0 + m0 * (x1 - x0)) * (x2 - x1) + (2.0 * y2 - m2 * (x2 - x1)) * (x1 - x0));
    let tl = (t - x0) / (x1 - x0);
    let tr = (t - x1) / (x2 - x1);
    let fl = y0 * (1.0 - tl * tl) + y1 * tl * tl + m0 * (1.0 - tl) * tl * (x1 - x0);
    let fr = y1 * (1.0 - tr) * (1.0 - tr) + y2 * (2.0 - tr) * tr + m2 * (tr - 1.0) * tr * (x2 - x1);
    let mut res = if t < x1 { fl } else { fr };
    if t < x0 {
        res = y0 + (t - x0) * m0;
    }
    if t > x2 {
        res = y2 + (t - x2) * m2;
    }
    res
}

/// The same shape read the other way: given a value, which input made it.
fn faux_cubic_reverse(t: f64, x0: f64, x2: f64, m0: f64, m2: f64) -> f64 {
    let (y0, y2) = (x0, x2);
    let x1 = x0 + (x2 - x0) * 0.5;
    let y1 = (0.5 / ((x2 - x1) + (x1 - x0)))
        * ((2.0 * y0 + m0 * (x1 - x0)) * (x2 - x1) + (2.0 * y2 - m2 * (x2 - x1)) * (x1 - x0));
    let cl = y0 - t;
    let bl = m0 * (x1 - x0);
    let al = y1 - y0 - m0 * (x1 - x0);
    let out_l = ((2.0 * cl) / (-(bl * bl - 4.0 * al * cl).sqrt() - bl)) * (x1 - x0) + x0;
    let cr = y1 - t;
    let br = 2.0 * y2 - 2.0 * y1 - m2 * (x2 - x1);
    let ar = y1 - y2 + m2 * (x2 - x1);
    let out_r = ((2.0 * cr) / (-(br * br - 4.0 * ar * cr).sqrt() - br)) * (x2 - x1) + x1;
    let mut res = if t < y1 { out_l } else { out_r };
    if t < y0 {
        res = x0 + (t - y0) / m0;
    }
    if t > y2 {
        res = x2 + (t - y2) / m2;
    }
    res
}

/// A band's slope, held off zero so the curve cannot go flat.
fn band_slope(val: f64) -> f64 {
    if val < 0.01 {
        0.01
    } else {
        val
    }
}

/// Where the highlight band puts a value, used to move the white band's own
/// start and width onto the curve the highlights have already made.
fn highlight_forward(t: f64, start: f64, pivot: f64, val: f64) -> f64 {
    // Either side of one is the same curve run the other way round.
    let val = 2.0 - val;
    if val <= 1.0 {
        faux_cubic_forward(t, start, pivot, 1.0, band_slope(val))
    } else {
        faux_cubic_reverse(t, start, pivot, 1.0, band_slope(2.0 - val))
    }
}

/// The shadow band's version of the same.
fn shadow_forward(t: f64, start: f64, pivot: f64, val: f64) -> f64 {
    if val <= 1.0 {
        faux_cubic_forward(t, start, pivot, band_slope(val), 1.0)
    } else {
        faux_cubic_reverse(t, start, pivot, band_slope(2.0 - val), 1.0)
    }
}

fn tone_is_identity(v: &GradingToneValues) -> bool {
    let flat = |b: &GradingRgbmsw| b.rgb == [1.0; 3] && b.master == 1.0;
    flat(&v.blacks)
        && flat(&v.shadows)
        && flat(&v.midtones)
        && flat(&v.highlights)
        && flat(&v.whites)
        && v.scontrast == 1.0
}

fn tone_prerender(v: &GradingToneValues) -> TonePreRender {
    let mut p = TonePreRender::default();
    // Where the style puts the top and bottom of the range it grades over, and
    // the middle its S contrast turns about.
    let (top, top_sc, bottom, pivot) = match v.style {
        GradingStyle::Log | GradingStyle::Video => (1.0_f32, 1.0_f32, 0.0_f32, 0.4_f32),
        GradingStyle::Lin => (7.5, 6.5, -5.5, 0.0),
    };
    p.top = top;
    p.top_sc = top_sc;
    p.bottom = bottom;
    p.pivot = pivot;
    p.bypass = tone_is_identity(v);
    if p.bypass {
        return p;
    }

    // The whites ride on top of the highlights, so their start and width are
    // where the highlight curve has already moved them to.
    let hl_pivot = v.highlights.width;
    p.highlights_start = if v.highlights.start > hl_pivot - 0.01 {
        hl_pivot - 0.01
    } else {
        v.highlights.start
    };
    p.highlights_width = hl_pivot;
    let new_start = highlight_forward(
        v.whites.start,
        p.highlights_start,
        p.highlights_width,
        v.highlights.master,
    );
    let new_end = highlight_forward(
        v.whites.start + v.whites.width,
        p.highlights_start,
        p.highlights_width,
        v.highlights.master,
    );
    p.whites_start = new_start;
    p.whites_width = new_end - new_start;

    // And the blacks ride on the shadows the same way.
    let sh_pivot = v.shadows.width;
    p.shadows_start = if v.shadows.start < sh_pivot + 0.01 {
        sh_pivot + 0.01
    } else {
        v.shadows.start
    };
    p.shadows_width = sh_pivot;
    let new_start = shadow_forward(
        v.blacks.start,
        p.shadows_width,
        p.shadows_start,
        v.shadows.master,
    );
    let new_end = shadow_forward(
        v.blacks.start - v.blacks.width,
        p.shadows_width,
        p.shadows_start,
        v.shadows.master,
    );
    p.blacks_start = new_start;
    p.blacks_width = new_start - new_end;

    tone_mids_precompute(v, &mut p, top, bottom);
    tone_hs_precompute(v, &mut p);
    tone_wb_precompute(v, &mut p);
    tone_sc_precompute(v, &mut p, top_sc, bottom, pivot);
    p
}

/// The midtone band: six knots whose slopes lift one side and drop the other by
/// the same area, so the ends of the range come out where they went in.
fn tone_mids_precompute(v: &GradingToneValues, p: &mut TonePreRender, top: f32, bottom: f32) {
    const HALO: f32 = 0.4;
    const MIN_SLOPE: f32 = 0.1;
    for channel in 0..4 {
        let mid_adj = cpp_clamp(tone_channel(&v.midtones, channel), 0.01, 1.99);
        if mid_adj == 1.0 {
            continue;
        }
        let (x, y, m) = (
            &mut p.mid_x[channel],
            &mut p.mid_y[channel],
            &mut p.mid_m[channel],
        );
        x[0] = bottom;
        x[5] = top;
        let max_width = (x[5] - x[0]) * 0.95;
        let width = cpp_clamp(v.midtones.width as f32, 0.01, max_width);
        let min_cent = x[0] + width * 0.51;
        let max_cent = x[5] - width * 0.51;
        let center = cpp_clamp(v.midtones.start as f32, min_cent, max_cent);
        x[1] = center - width * 0.5;
        x[4] = x[1] + width;
        x[2] = x[1] + (x[4] - x[1]) * 0.25;
        x[3] = x[1] + (x[4] - x[1]) * 0.75;
        y[0] = x[0];
        m[0] = 1.0;
        m[5] = 1.0;

        let adj = (mid_adj - 1.0) * (1.0 - MIN_SLOPE);
        m[2] = 1.0 + adj;
        m[3] = 1.0 - adj;
        m[1] = 1.0 + adj * HALO;
        m[4] = 1.0 - adj * HALO;

        if center <= (x[5] + x[0]) * 0.5 {
            let area = (x[1] - x[0]) * (m[1] - m[0]) * 0.5
                + (x[2] - x[1]) * ((m[1] - m[0]) + (m[2] - m[1]) * 0.5)
                + (center - x[2]) * (m[2] - m[0]) * 0.5;
            m[4] = (-0.5 * (x[5] - x[4]) * m[5]
                + (x[4] - x[3]) * (0.5 * m[3] - m[5])
                + (x[3] - center) * (m[3] - m[5]) * 0.5
                + area)
                / (-0.5 * (x[5] - x[3]));
        } else {
            let area = (x[5] - x[4]) * (m[4] - m[5]) * 0.5
                + (x[4] - x[3]) * ((m[4] - m[5]) + (m[3] - m[4]) * 0.5)
                + (x[3] - center) * (m[3] - m[5]) * 0.5;
            m[1] = (-0.5 * (x[1] - x[0]) * m[0]
                + (x[2] - x[1]) * (0.5 * m[2] - m[0])
                + (center - x[2]) * (m[2] - m[0]) * 0.5
                + area)
                / (-0.5 * (x[2] - x[0]));
        }

        for i in 0..5 {
            y[i + 1] = y[i] + (m[i] + m[i + 1]) * (x[i + 1] - x[i]) * 0.5;
        }
    }
}

/// The shadow and highlight bands: three knots each, one curve per channel.
fn tone_hs_precompute(v: &GradingToneValues, p: &mut TonePreRender) {
    for band in 0..2 {
        let is_shadow = band == 1;
        for channel in 0..4 {
            let mut val = tone_channel(if is_shadow { &v.shadows } else { &v.highlights }, channel);
            if !is_shadow {
                val = 2.0 - val;
            }
            if val == 1.0 {
                continue;
            }
            let start = (if is_shadow {
                p.shadows_start
            } else {
                p.highlights_start
            }) as f32;
            let pivot = (if is_shadow {
                p.shadows_width
            } else {
                p.highlights_width
            }) as f32;
            let (x, y, m) = (
                &mut p.hs_x[band][channel],
                &mut p.hs_y[band][channel],
                &mut p.hs_m[band][channel],
            );
            x[0] = if is_shadow { pivot } else { start };
            x[2] = if is_shadow { start } else { pivot };
            y[0] = x[0];
            y[2] = x[2];
            x[1] = x[0] + (x[2] - x[0]) * 0.5;
            let slope = |s: f32| if is_shadow { s.max(0.01) } else { 1.0 };
            let other = |s: f32| if is_shadow { 1.0 } else { s.max(0.01) };
            if val < 1.0 {
                m[0] = slope(val);
                m[1] = other(val);
                y[1] = (0.5 / (x[2] - x[0]))
                    * ((2.0 * y[0] + m[0] * (x[1] - x[0])) * (x[2] - x[1])
                        + (2.0 * y[2] - m[1] * (x[2] - x[1])) * (x[1] - x[0]));
            } else if val > 1.0 {
                m[0] = slope(2.0 - val);
                m[1] = other(2.0 - val);
                y[1] = (0.5 / ((x[2] - x[1]) + (x[1] - x[0])))
                    * ((2.0 * y[0] + m[0] * (x[1] - x[0])) * (x[2] - x[1])
                        + (2.0 * y[2] - m[1] * (x[2] - x[1])) * (x[1] - x[0]));
            }
        }
    }
}

/// The white and black bands: two knots each, a straight run whose slope the
/// band's value sets, with a gain for the increasing case.
fn tone_wb_precompute(v: &GradingToneValues, p: &mut TonePreRender) {
    for band in 0..2 {
        let is_black = band == 1;
        for channel in 0..4 {
            let start = (if is_black {
                p.blacks_start
            } else {
                p.whites_start
            }) as f32;
            let width = (if is_black {
                p.blacks_width
            } else {
                p.whites_width
            }) as f32;
            let val = tone_channel(if is_black { &v.blacks } else { &v.whites }, channel);
            let (x, y, m) = (
                &mut p.wb_x[band][channel],
                &mut p.wb_y[band][channel],
                &mut p.wb_m[band][channel],
            );
            x[0] = if is_black { start - width } else { start };
            x[1] = if is_black { start } else { x[0] + width };
            let mtest = if is_black { 2.0 - val } else { val };
            if mtest < 1.0 {
                if is_black {
                    m[0] = (2.0 - val).max(0.01);
                    m[1] = 1.0;
                    y[1] = x[1];
                    y[0] = y[1] - (m[0] + m[1]) * (x[1] - x[0]) * 0.5;
                } else {
                    m[0] = 1.0;
                    m[1] = val.max(0.01);
                    y[0] = x[0];
                    y[1] = y[0] + (m[0] + m[1]) * (x[1] - x[0]) * 0.5;
                }
            } else if mtest > 1.0 {
                if is_black {
                    m[0] = val.max(0.01);
                    m[1] = 1.0;
                    y[1] = x[1];
                    y[0] = y[1] - (m[0] + m[1]) * (x[1] - x[0]) * 0.5;
                } else {
                    m[0] = 1.0;
                    m[1] = (2.0 - val).max(0.01);
                    y[0] = x[0];
                    // y[1] is not read in this case.
                }
                p.wb_gain[band][channel] = (m[0] + m[1]) * 0.5;
            }
        }
    }
}

/// The S-shaped contrast: a straight run through the pivot, rounded off at each
/// end so it rejoins the identity rather than reversing.
fn tone_sc_precompute(
    v: &GradingToneValues,
    p: &mut TonePreRender,
    top_sc: f32,
    bottom: f32,
    pivot: f32,
) {
    let contrast = v.scontrast as f32;
    if contrast == 1.0 {
        return;
    }
    let contrast = if contrast > 1.0 {
        1.0 / (1.8125 - 0.8125 * contrast.min(1.99))
    } else {
        0.28125 + 0.71875 * contrast.max(0.01)
    };
    {
        let (x, y, m) = (&mut p.sc_x[0], &mut p.sc_y[0], &mut p.sc_m[0]);
        x[3] = top_sc;
        y[3] = top_sc;
        y[0] = pivot + (y[3] - pivot) * 0.25;
        m[0] = contrast;
        x[0] = pivot + (y[0] - pivot) / m[0];
        let min_width = (x[3] - x[0]) * 0.3;
        m[1] = 1.0 / m[0];
        let center = (y[3] - y[0] - m[1] * x[3] + m[0] * x[0]) / (m[0] - m[1]);
        x[1] = x[0];
        x[2] = 2.0 * center - x[1];
        if x[2] > x[3] {
            x[2] = x[3];
            x[1] = 2.0 * center - x[2];
        } else if (x[2] - x[1]) < min_width {
            x[2] = x[1] + min_width;
            let new_center = (x[2] + x[1]) * 0.5;
            m[1] = (y[3] - y[0] + m[0] * x[0] - new_center * m[0]) / (x[3] - new_center);
        }
        y[1] = y[0];
        y[2] = y[1] + (m[0] + m[1]) * (x[2] - x[1]) * 0.5;
    }
    {
        let (x, y, m) = (&mut p.sc_x[1], &mut p.sc_y[1], &mut p.sc_m[1]);
        x[0] = bottom;
        y[0] = bottom;
        y[3] = pivot - (pivot - y[0]) * 0.25;
        m[1] = contrast;
        x[3] = pivot - (pivot - y[3]) / m[1];
        let min_width = (x[3] - x[0]) * 0.3;
        m[0] = 1.0 / m[1];
        let center = (y[3] - y[0] - m[1] * x[3] + m[0] * x[0]) / (m[0] - m[1]);
        x[2] = x[3];
        x[1] = 2.0 * center - x[2];
        if x[1] < x[0] {
            x[1] = x[0];
            x[2] = 2.0 * center - x[1];
        } else if (x[2] - x[1]) < min_width {
            x[1] = x[2] - min_width;
            let new_center = (x[2] + x[1]) * 0.5;
            m[0] = (y[3] - y[0] - m[1] * x[3] + new_center * m[1]) / (new_center - x[0]);
        }
        y[2] = y[3];
        y[1] = y[2] - (m[0] + m[1]) * (x[2] - x[1]) * 0.5;
    }
}

/// One segment of a piecewise-quadratic run, evaluated forwards.
fn quad_forward(t: f32, x: &[f32], y: &[f32], m: &[f32], i: usize) -> f32 {
    let step = x[i + 1] - x[i];
    let local = (t - x[i]) / step;
    local * step * (local * 0.5 * (m[i + 1] - m[i]) + m[i]) + y[i]
}

/// The same segment read backwards: the root of the quadratic, taken in the
/// form that does not cancel when the curvature is small.
fn quad_reverse(t: f32, x: &[f32], y: &[f32], m: &[f32], i: usize) -> f32 {
    let step = x[i + 1] - x[i];
    let c = y[i] - t;
    let b = m[i] * step;
    let a = 0.5 * (m[i + 1] - m[i]) * step;
    let discrim = (b * b - 4.0 * a * c).sqrt();
    ((2.0 * c) / (-discrim - b)) * step + x[i]
}

fn tone_mids(
    pre: &TonePreRender,
    v: &GradingToneValues,
    channel: usize,
    forward: bool,
    out: &mut [f32; 3],
) {
    let mid_adj = cpp_clamp(tone_channel(&v.midtones, channel), 0.01, 1.99);
    if mid_adj == 1.0 {
        return;
    }
    let (x, y, m) = (
        &pre.mid_x[channel],
        &pre.mid_y[channel],
        &pre.mid_m[channel],
    );
    if forward {
        on_channels(channel, out, |t| {
            let mut res = if t < x[1] {
                quad_forward(t, x, y, m, 0)
            } else {
                quad_forward(t, x, y, m, 1)
            };
            for i in 2..5 {
                if t >= x[i] {
                    res = quad_forward(t, x, y, m, i);
                }
            }
            if t < x[0] {
                res = (t - x[0]) * m[0] + y[0];
            }
            if t >= x[5] {
                res = (t - x[5]) * m[5] + y[5];
            }
            res
        });
        return;
    }
    // Backwards the reference's two code paths disagree above the top knot: the
    // per channel one runs the BOTTOM segment's straight line there, the master
    // one runs the top's. Both are kept, because the fixture is generated by
    // that library and would see either one changed.
    let master = channel == CH_MASTER;
    on_channels(channel, out, |t| {
        let below = x[0] + (t - y[0]) / m[0];
        if t >= y[5] {
            return if master {
                x[5] + (t - y[5]) / m[5]
            } else {
                below
            };
        }
        if t < y[0] {
            return below;
        }
        for i in (0..5).rev() {
            if t >= y[i] {
                return quad_reverse(t, x, y, m, i);
            }
        }
        below
    });
}

fn hs_forward(x: &[f32; 3], y: &[f32; 3], m: &[f32; 2], t: f32) -> f32 {
    let tl = (t - x[0]) / (x[1] - x[0]);
    let tr = (t - x[1]) / (x[2] - x[1]);
    let fl = y[0] * (1.0 - tl * tl) + y[1] * tl * tl + m[0] * (1.0 - tl) * tl * (x[1] - x[0]);
    let fr = y[1] * (1.0 - tr) * (1.0 - tr)
        + y[2] * (2.0 - tr) * tr
        + m[1] * (tr - 1.0) * tr * (x[2] - x[1]);
    let mut res = if t < x[1] { fl } else { fr };
    if t < x[0] {
        res = (t - x[0]) * m[0] + y[0];
    }
    if t >= x[2] {
        res = (t - x[2]) * m[1] + y[2];
    }
    res
}

fn hs_reverse(x: &[f32; 3], y: &[f32; 3], m: &[f32; 2], t: f32) -> f32 {
    let bl = m[0] * (x[1] - x[0]);
    let al = y[1] - y[0] - m[0] * (x[1] - x[0]);
    let cl = y[0] - t;
    let out_l = (-2.0 * cl) / ((bl * bl - 4.0 * al * cl).sqrt() + bl) * (x[1] - x[0]) + x[0];
    let br = 2.0 * y[2] - 2.0 * y[1] - m[1] * (x[2] - x[1]);
    let ar = y[1] - y[2] + m[1] * (x[2] - x[1]);
    let cr = y[1] - t;
    let out_r = (-2.0 * cr) / ((br * br - 4.0 * ar * cr).sqrt() + br) * (x[2] - x[1]) + x[1];
    let mut res = if t < y[1] { out_l } else { out_r };
    if t < y[0] {
        res = (t - y[0]) / m[0] + x[0];
    }
    if t >= y[2] {
        res = (t - y[2]) / m[1] + x[2];
    }
    res
}

fn tone_highlight_shadow(
    pre: &TonePreRender,
    v: &GradingToneValues,
    channel: usize,
    is_shadow: bool,
    forward: bool,
    out: &mut [f32; 3],
) {
    let mut val = tone_channel(if is_shadow { &v.shadows } else { &v.highlights }, channel);
    if !is_shadow {
        val = 2.0 - val;
    }
    if val == 1.0 {
        return;
    }
    let band = usize::from(is_shadow);
    let (x, y, m) = (
        &pre.hs_x[band][channel],
        &pre.hs_y[band][channel],
        &pre.hs_m[band][channel],
    );
    // Either side of one is the same curve run the other way round, so the
    // direction the op runs in flips which half of the pair is used.
    let take_forward = (val < 1.0) == forward;
    on_channels(channel, out, |t| {
        if take_forward {
            hs_forward(x, y, m, t)
        } else {
            hs_reverse(x, y, m, t)
        }
    });
}

/// The quadratic that carries the white band above its own top knot, so a high
/// dynamic range value keeps going rather than folding back.
fn wb_extrapolation(x: &[f32; 2], m: &[f32; 2], gain: f32) -> (f32, f32, f32) {
    let new_y1 = (x[1] - x[0]) / gain + x[0];
    let xd = x[0] + (x[1] - x[0]) * 0.99;
    let md = 1.0 / (m[0] + (xd - x[0]) * (m[1] - m[0]) / (x[1] - x[0]));
    let aa = 0.5 * (1.0 / m[1] - md) / (x[1] - xd);
    let bb = 1.0 / m[1] - 2.0 * aa * x[1];
    let cc = new_y1 - bb * x[1] - aa * x[1] * x[1];
    (aa, bb, cc)
}

fn wb_forward(
    is_black: bool,
    val: f32,
    x: &[f32; 2],
    y: &[f32; 2],
    m: &[f32; 2],
    gain: f32,
    t: f32,
) -> f32 {
    let mtest = if is_black { 2.0 - val } else { val };
    if mtest < 1.0 {
        let local = (t - x[0]) / (x[1] - x[0]);
        let mut res = local * (x[1] - x[0]) * (local * 0.5 * (m[1] - m[0]) + m[0]) + y[0];
        if t < x[0] {
            res = y[0] + (t - x[0]) * m[0];
        }
        if t >= x[1] {
            res = y[1] + (t - x[1]) * m[1];
        }
        res
    } else if mtest > 1.0 {
        let t = if is_black {
            (t - x[1]) * gain + x[1]
        } else {
            (t - x[0]) * gain + x[0]
        };
        let a = 0.5 * (m[1] - m[0]) * (x[1] - x[0]);
        let b = m[0] * (x[1] - x[0]);
        let c = y[0] - t;
        let tmp = (-2.0 * c) / ((b * b - 4.0 * a * c).sqrt() + b);
        let mut res = tmp * (x[1] - x[0]) + x[0];
        if t < y[0] {
            res = x[0] + (t - y[0]) / m[0];
        }
        if is_black {
            if t >= y[1] {
                res = x[1] + (t - y[1]) / m[1];
            }
            (res - x[1]) / gain + x[1]
        } else {
            res = (res - x[0]) / gain + x[0];
            let (aa, bb, cc) = wb_extrapolation(x, m, gain);
            let t = (t - x[0]) / gain + x[0];
            if t >= x[1] {
                res = (aa * t + bb) * t + cc;
            }
            res
        }
    } else {
        t
    }
}

fn wb_reverse(
    is_black: bool,
    val: f32,
    x: &[f32; 2],
    y: &[f32; 2],
    m: &[f32; 2],
    gain: f32,
    t: f32,
) -> f32 {
    let mtest = if is_black { 2.0 - val } else { val };
    if mtest < 1.0 {
        let a = 0.5 * (m[1] - m[0]) * (x[1] - x[0]);
        let b = m[0] * (x[1] - x[0]);
        let c = y[0] - t;
        let tmp = (-2.0 * c) / ((b * b - 4.0 * a * c).sqrt() + b);
        let mut res = tmp * (x[1] - x[0]) + x[0];
        if t < y[0] {
            res = x[0] + (t - y[0]) / m[0];
        }
        if t >= y[1] {
            res = x[1] + (t - y[1]) / m[1];
        }
        res
    } else if mtest > 1.0 {
        let t = if is_black {
            (t - x[1]) * gain + x[1]
        } else {
            (t - x[0]) * gain + x[0]
        };
        let local = (t - x[0]) / (x[1] - x[0]);
        let mut res = local * (x[1] - x[0]) * (local * 0.5 * (m[1] - m[0]) + m[0]) + y[0];
        if t < x[0] {
            res = y[0] + (t - x[0]) * m[0];
        }
        if is_black {
            if t >= x[1] {
                res = y[1] + (t - x[1]) * m[1];
            }
            (res - x[1]) / gain + x[1]
        } else {
            res = (res - x[0]) / gain + x[0];
            let (aa, bb, cc) = wb_extrapolation(x, m, gain);
            let t = (t - x[0]) / gain + x[0];
            let c = cc - t;
            let res1 = (-2.0 * c) / ((bb * bb - 4.0 * aa * c).sqrt() + bb);
            let brk = (aa * x[1] + bb) * x[1] + cc;
            if t >= brk {
                res = res1;
            }
            res
        }
    } else {
        t
    }
}

fn tone_white_black(
    pre: &TonePreRender,
    v: &GradingToneValues,
    channel: usize,
    is_black: bool,
    forward: bool,
    out: &mut [f32; 3],
) {
    let val = tone_channel(if is_black { &v.blacks } else { &v.whites }, channel);
    let band = usize::from(is_black);
    let (x, y, m, gain) = (
        &pre.wb_x[band][channel],
        &pre.wb_y[band][channel],
        &pre.wb_m[band][channel],
        pre.wb_gain[band][channel],
    );
    on_channels(channel, out, |t| {
        if forward {
            wb_forward(is_black, val, x, y, m, gain, t)
        } else {
            wb_reverse(is_black, val, x, y, m, gain, t)
        }
    });
}

fn tone_scontrast(pre: &TonePreRender, v: &GradingToneValues, forward: bool, out: &mut [f32; 3]) {
    let contrast = v.scontrast as f32;
    if contrast == 1.0 {
        return;
    }
    // The stated range is squeezed so the curve cannot double back on itself.
    let contrast = if contrast > 1.0 {
        1.0 / (1.8125 - 0.8125 * contrast.min(1.99))
    } else {
        0.28125 + 0.71875 * contrast.max(0.01)
    };
    let source = *out;
    for k in 0..3 {
        let t = source[k];
        let mut value = if forward {
            (t - pre.pivot) * contrast + pre.pivot
        } else {
            (t - pre.pivot) / contrast + pre.pivot
        };
        for end in 0..2 {
            let (x, y, m) = (&pre.sc_x[end], &pre.sc_y[end], &pre.sc_m[end]);
            let top = end == 0;
            if forward {
                let local = (t - x[1]) / (x[2] - x[1]);
                let res = local * (x[2] - x[1]) * (local * 0.5 * (m[1] - m[0]) + m[0]) + y[1];
                if top {
                    if t >= x[1] {
                        value = res;
                    }
                    if t >= x[2] {
                        value = y[2] + (t - x[2]) * m[1];
                    }
                } else {
                    if t < x[2] {
                        value = res;
                    }
                    if t < x[1] {
                        value = y[1] + (t - x[1]) * m[0];
                    }
                }
            } else {
                let b = m[0] * (x[2] - x[1]);
                let a = (m[1] - m[0]) * 0.5 * (x[2] - x[1]);
                let c = y[1] - t;
                let res = (x[2] - x[1]) * (-2.0 * c) / ((b * b - 4.0 * a * c).sqrt() + b) + x[1];
                if top {
                    if t >= y[1] {
                        value = res;
                    }
                    if t >= y[2] {
                        value = x[2] + (t - y[2]) / m[1];
                    }
                } else {
                    if t < y[2] {
                        value = res;
                    }
                    if t < y[1] {
                        value = x[1] + (t - y[1]) / m[0];
                    }
                }
            }
        }
        out[k] = value;
    }
}

/// The linear style grades in a log view of the light, so it goes there and back
/// around the bands. These are the reference's own constants for that map.
mod lin_log {
    pub const XBRK: f32 = 0.004_131_837_3;
    pub const SHIFT: f32 = -0.000_157_849_85;
    pub const M: f32 = 1.0 / (0.18 + SHIFT);
    pub const GAIN: f32 = 363.034_6;
    pub const OFFS: f32 = -7.0;
    pub const YBRK: f32 = -5.5;
    /// The reference writes this out as 1.4426950408889634, which is log2(e).
    pub const BASE2: f32 = std::f32::consts::LOG2_E;
}

fn to_log(rgb: [f32; 3]) -> [f32; 3] {
    let mut out = rgb;
    for c in out.iter_mut() {
        *c = if *c < lin_log::XBRK {
            *c * lin_log::GAIN + lin_log::OFFS
        } else {
            lin_log::BASE2 * ((*c + lin_log::SHIFT) * lin_log::M).ln()
        };
    }
    out
}

fn from_log(rgb: &mut [f32; 3]) {
    for c in rgb.iter_mut() {
        *c = if *c < lin_log::YBRK {
            (*c - lin_log::OFFS) / lin_log::GAIN
        } else {
            2.0_f32.powf(*c) * (0.18 + lin_log::SHIFT) - lin_log::SHIFT
        };
    }
}

fn tone_eval(g: &GradingTone, dir: Direction, rgb: [f32; 3]) -> [f32; 3] {
    if g.pre.bypass {
        return rgb;
    }
    let v = &g.values;
    let linear = matches!(v.style, GradingStyle::Lin);
    let forward = matches!(dir, Direction::Forward);
    let mut out = if linear { to_log(rgb) } else { rgb };
    // Forwards the bands run red, green, blue then master and midtones first;
    // backwards the whole order reverses, master first.
    let channels: [usize; 4] = if forward { [0, 1, 2, 3] } else { [3, 0, 1, 2] };
    let mids = |out: &mut [f32; 3]| {
        for ch in channels {
            tone_mids(&g.pre, v, ch, forward, out);
        }
    };
    let hs = |out: &mut [f32; 3], is_shadow: bool| {
        for ch in channels {
            tone_highlight_shadow(&g.pre, v, ch, is_shadow, forward, out);
        }
    };
    let wb = |out: &mut [f32; 3], is_black: bool| {
        for ch in channels {
            tone_white_black(&g.pre, v, ch, is_black, forward, out);
        }
    };
    if forward {
        mids(&mut out);
        hs(&mut out, false);
        wb(&mut out, false);
        hs(&mut out, true);
        wb(&mut out, true);
        tone_scontrast(&g.pre, v, forward, &mut out);
    } else {
        tone_scontrast(&g.pre, v, forward, &mut out);
        wb(&mut out, true);
        hs(&mut out, true);
        wb(&mut out, false);
        hs(&mut out, false);
        mids(&mut out);
    }
    if linear {
        from_log(&mut out);
    }
    // The bands can push a value past what a half float holds, and an infinity
    // downstream reads as black rather than as bright.
    for c in out.iter_mut() {
        *c = if 65504.0 < *c { 65504.0 } else { *c };
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
            Op::GradingPrimary { .. } => "a primary grade",
            Op::GradingTone { .. } => "a tone grade",
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
            Op::GradingPrimary { params, dir } => primary_eval(params, *dir, rgb),
            Op::GradingTone { params, dir } => tone_eval(params, *dir, rgb),
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
            // Both grades have an inverse in the reference library, written as
            // the same values run the other way round rather than as a second
            // set of numbers.
            Op::GradingPrimary { params, dir } => Op::GradingPrimary {
                params: params.clone(),
                dir: dir.flipped(),
            },
            Op::GradingTone { params, dir } => Op::GradingTone {
                params: params.clone(),
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
            // Same test for a primary grade: its saturation is the only step
            // that reads a channel other than its own. Per channel values give
            // the three channels different curves, which a factorised stage
            // holds happily, it samples one curve per channel already.
            Op::GradingPrimary { params, .. } => params.saturation == 1.0,
            // A tone grade never mixes channels at all: even the master band
            // runs the same curve on each of the three separately.
            Op::GradingTone { .. } => true,
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

    fn primary(v: GradingPrimary) -> Op {
        Op::GradingPrimary {
            params: Box::new(v),
            dir: Direction::Forward,
        }
    }

    fn tone(v: GradingToneValues) -> Op {
        Op::GradingTone {
            params: Box::new(GradingTone::new(v)),
            dir: Direction::Forward,
        }
    }

    const STYLES: [GradingStyle; 3] = [GradingStyle::Log, GradingStyle::Lin, GradingStyle::Video];

    #[test]
    fn a_log_primary_holds_its_pivot_and_counts_brightness_in_code_values() {
        // Contrast turns about the pivot, which the log style states as
        // 0.5 + pivot / 2, so the value there is the one a contrast grade
        // cannot move. Blender's AgX looks are exactly this grade.
        let mut values = GradingPrimary::new(GradingStyle::Log);
        values.contrast = GradingRgbm {
            rgb: [1.3, 0.85, 1.1],
            master: 1.05,
        };
        values.pivot = 0.15;
        let op = primary(values);
        let pivot = 0.5 + 0.15 * 0.5;
        assert!(close(op.eval([pivot; 3]), [pivot; 3], 1e-6));
        round_trip(&op, &SAMPLES, 1e-4);

        // Brightness is counted in code values out of 1023, six and a quarter
        // to the step, and the master adds to every channel.
        let mut values = GradingPrimary::new(GradingStyle::Log);
        values.brightness = GradingRgbm {
            rgb: [12.0, -8.0, 4.0],
            master: 6.0,
        };
        let step = 6.25 / 1023.0;
        let want = [0.5 + 18.0 * step, 0.5 - 2.0 * step, 0.5 + 10.0 * step];
        assert!(close(primary(values).eval([0.5; 3]), want, 1e-6));
    }

    #[test]
    fn a_grade_with_nothing_set_leaves_a_colour_exactly_alone() {
        for style in STYLES {
            let flat_primary = primary(GradingPrimary::new(style));
            let flat_tone = tone(GradingToneValues::new(style));
            for c in SAMPLES {
                assert_eq!(flat_primary.eval(c), c, "{style:?} primary");
                assert_eq!(flat_tone.eval(c), c, "{style:?} tone");
            }
        }
    }

    #[test]
    fn saturation_is_the_only_step_either_grade_mixes_channels_in() {
        let mut values = GradingPrimary::new(GradingStyle::Log);
        values.saturation = 0.0;
        let flattened = primary(values);
        let luma = 0.2126 * 0.2 + 0.7152 * 0.4 + 0.0722 * 0.6;
        let got = flattened.eval([0.2, 0.4, 0.6]);
        assert!(close(got, [luma; 3], 1e-6), "{got:?}");
        assert!(!flattened.is_channel_independent());

        // A tone band stated for red alone moves red and leaves the other two
        // where they were, which is why a tone grade always factorises.
        let mut values = GradingToneValues::new(GradingStyle::Log);
        values.midtones.rgb = [1.3, 1.0, 1.0];
        let banded = tone(values);
        let got = banded.eval([0.4, 0.4, 0.4]);
        assert!(got[0] != got[1], "red did not move: {got:?}");
        assert_eq!([got[1], got[2]], [0.4, 0.4]);
        assert!(banded.is_channel_independent());
    }

    #[test]
    fn a_tone_grade_round_trips_through_its_inverse() {
        let mut values = GradingToneValues::new(GradingStyle::Log);
        values.midtones = GradingRgbmsw {
            rgb: [1.25, 0.85, 1.05],
            master: 1.15,
            start: 0.45,
            width: 0.55,
        };
        values.shadows = GradingRgbmsw {
            rgb: [1.3, 0.8, 1.1],
            master: 0.9,
            start: 0.55,
            width: 0.12,
        };
        values.scontrast = 1.45;
        round_trip(&tone(values), &[[0.2, 0.4, 0.6], [0.5, 0.25, 0.75]], 1e-4);
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
