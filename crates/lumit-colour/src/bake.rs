//! The bake: turning a resolved chain into one small table both the Viewer and
//! the export sample.
//!
//! In plain terms: a chain of logarithms and matrices is far too slow to run on
//! every pixel of every frame, so it is run **once**, on the processor, over a
//! grid of sample colours, and the answers are kept. That table of answers is
//! the *artefact*, and it comes in two shapes:
//!
//! - **Factorised** — when every step in the chain treats red, green and blue
//!   separately (curves) or is a matrix, the artefact is just those: a sampled
//!   curve, a matrix, another curve if the chain asks for one. This is exact to
//!   the sampling density and it keeps working on values outside the sampled
//!   range, because the original steps are kept alongside and evaluated there.
//!   Camera input transforms — a transfer curve then a primaries matrix — are
//!   this shape by construction.
//! - **Shaper + cube** — for everything else. Scene-linear light has no top end,
//!   so it is first squeezed into 0–1 by a **shaper** (a logarithmic squash, so
//!   the dark end gets as many samples as the bright end), and then a 65×65×65
//!   cube holds the answers. The cost is stated honestly in docs/impl/ocio.md
//!   §5.4: what the shaper cannot reach, the cube clamps.
//!
//! Choosing between them is mechanical, not clever: factorise when every step
//! allows it, otherwise bake the cube.

use crate::error::{ColourError, Result};
use crate::matrix::{self, Matrix34};
use crate::op::{Chain, Op};
use crate::sample::{Cube, Curve};

/// Sample count for a factorised curve stage. At 4096 points the error against
/// exact evaluation is ≤ 1 × 10⁻⁵ for the smooth curves configs ship (§5.4).
pub const CURVE_SAMPLES: usize = 4096;

/// Edge length of a baked cube. 65³ × 3 × fp16 ≈ 1.6 MiB on the GPU (§5.3).
pub const CUBE_SIZE: usize = 65;

/// The domain a factorised curve stage is sampled over. Outside it the stage
/// evaluates its own steps exactly, which is how negatives and high dynamic
/// range survive the factorised form (§5.1).
pub const CURVE_DOMAIN: [f32; 2] = [0.0, 1.0];

/// How scene-linear light is squeezed into the 0–1 a cube can index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shaper {
    /// The lg2 allocation: `y = (log2(x + offset) − min) / (max − min)`.
    Lg2 {
        min_log2: f32,
        max_log2: f32,
        offset: f32,
    },
    /// A plain linear squeeze, for a config whose space declares `allocation: uniform`.
    Uniform { min: f32, max: f32 },
}

impl Shaper {
    /// The default when a config says nothing: `offset = 2⁻⁸`, ceiling `2⁵`,
    /// which covers linear 0 → 32 with log-even grid spacing (§5.1).
    pub const DEFAULT: Shaper = Shaper::Lg2 {
        min_log2: -8.0,
        max_log2: 5.0,
        offset: 0.003_906_25,
    };

    /// Squeeze one linear value into `[0, 1]`.
    #[must_use]
    pub fn forward(&self, x: f32) -> f32 {
        let y = match *self {
            Shaper::Lg2 {
                min_log2,
                max_log2,
                offset,
            } => {
                let span = max_log2 - min_log2;
                if span == 0.0 {
                    0.0
                } else {
                    ((x + offset).max(f32::MIN_POSITIVE).log2() - min_log2) / span
                }
            }
            Shaper::Uniform { min, max } => {
                let span = max - min;
                if span == 0.0 {
                    0.0
                } else {
                    (x - min) / span
                }
            }
        };
        if y.is_nan() {
            0.0
        } else {
            y.clamp(0.0, 1.0)
        }
    }

    /// And back out again — the map the bake walks to find each grid point's
    /// linear input.
    #[must_use]
    pub fn inverse(&self, y: f32) -> f32 {
        let y = if y.is_nan() { 0.0 } else { y.clamp(0.0, 1.0) };
        match *self {
            Shaper::Lg2 {
                min_log2,
                max_log2,
                offset,
            } => (y * (max_log2 - min_log2) + min_log2).exp2() - offset,
            Shaper::Uniform { min, max } => min + y * (max - min),
        }
    }
}

/// One stage of a factorised artefact.
#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Curve(CurveStage),
    Matrix(Matrix34),
}

/// A sampled per-channel curve, plus the steps it was sampled from so values
/// off either end of the domain are answered exactly rather than clamped.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveStage {
    pub table: Curve,
    /// The channel-independent steps this table was built from.
    pub exact: Vec<Op>,
}

impl CurveStage {
    #[must_use]
    pub fn eval(&self, rgb: [f32; 3]) -> [f32; 3] {
        let out_of_domain = rgb
            .iter()
            .any(|c| *c < self.table.domain[0] || *c > self.table.domain[1] || c.is_nan());
        if !out_of_domain {
            return self.table.sample(rgb);
        }
        // The steps are channel-independent, so evaluating the whole triple and
        // picking each channel's own answer is exact.
        let exact = self.exact.iter().fold(rgb, |c, op| op.eval(c));
        let sampled = self.table.sample(rgb);
        let mut out = [0.0_f32; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let x = rgb[c];
            *o = if x < self.table.domain[0] || x > self.table.domain[1] || x.is_nan() {
                exact[c]
            } else {
                sampled[c]
            };
        }
        out
    }
}

/// What a resolved chain bakes down to — the one thing the pipeline executes.
#[derive(Debug, Clone, PartialEq)]
pub enum Artefact {
    Factorised { stages: Vec<Stage> },
    ShaperCube { shaper: Shaper, cube: Cube },
}

impl Artefact {
    /// The CPU sampler: what the Viewer and the export must both agree with
    /// (docs/08 §1.6's oracle, and this crate's conformance engine).
    #[must_use]
    pub fn eval(&self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Artefact::Factorised { stages } => stages.iter().fold(rgb, |c, s| match s {
                Stage::Curve(cs) => cs.eval(c),
                Stage::Matrix(m) => matrix::apply(m, c),
            }),
            Artefact::ShaperCube { shaper, cube } => {
                let shaped = [
                    shaper.forward(rgb[0]),
                    shaper.forward(rgb[1]),
                    shaper.forward(rgb[2]),
                ];
                cube.sample(shaped)
            }
        }
    }
}

/// Bake a resolved chain: factorised where the chain allows it, a shaper and a
/// cube otherwise (§5.1).
pub fn bake(chain: &Chain, shaper: Shaper) -> Result<Artefact> {
    if chain.is_factorable() {
        factorise(chain)
    } else {
        bake_cube(chain, shaper)
    }
}

/// The factorised form: consecutive channel-independent steps collapse into one
/// sampled curve, consecutive matrices multiply into one matrix.
pub fn factorise(chain: &Chain) -> Result<Artefact> {
    let mut stages: Vec<Stage> = Vec::new();
    let mut pending: Vec<Op> = Vec::new();

    let flush = |pending: &mut Vec<Op>, stages: &mut Vec<Stage>| -> Result<()> {
        if pending.is_empty() {
            return Ok(());
        }
        let ops = std::mem::take(pending);
        let last = CURVE_SAMPLES - 1;
        let span = CURVE_DOMAIN[1] - CURVE_DOMAIN[0];
        let mut data = Vec::with_capacity(CURVE_SAMPLES);
        for i in 0..CURVE_SAMPLES {
            let x = CURVE_DOMAIN[0] + span * (i as f32 / last as f32);
            data.push(ops.iter().fold([x; 3], |c, op| op.eval(c)));
        }
        let table = Curve::new("a baked curve", CURVE_DOMAIN, data)?;
        stages.push(Stage::Curve(CurveStage { table, exact: ops }));
        Ok(())
    };

    for op in &chain.ops {
        match op {
            Op::Matrix(m) => {
                flush(&mut pending, &mut stages)?;
                match stages.last_mut() {
                    Some(Stage::Matrix(prev)) => *prev = matrix::concat(prev, m),
                    _ => stages.push(Stage::Matrix(*m)),
                }
            }
            other if other.is_channel_independent() => pending.push(other.clone()),
            other => {
                return Err(ColourError::UnsupportedTransform {
                    name: other.name().to_string(),
                })
            }
        }
    }
    flush(&mut pending, &mut stages)?;
    Ok(Artefact::Factorised { stages })
}

/// The shaper + cube form: walk the grid, undo the shaper to find each point's
/// linear input, and record what the chain makes of it.
pub fn bake_cube(chain: &Chain, shaper: Shaper) -> Result<Artefact> {
    let last = (CUBE_SIZE - 1) as f32;
    let mut data = Vec::with_capacity(CUBE_SIZE * CUBE_SIZE * CUBE_SIZE);
    // Red fastest, as everywhere else in Lumit (docs/impl/lut.md §1).
    for b in 0..CUBE_SIZE {
        let lb = shaper.inverse(b as f32 / last);
        for g in 0..CUBE_SIZE {
            let lg = shaper.inverse(g as f32 / last);
            for r in 0..CUBE_SIZE {
                let lr = shaper.inverse(r as f32 / last);
                data.push(chain.eval([lr, lg, lb]));
            }
        }
    }
    let cube = Cube::new("a baked cube", CUBE_SIZE, [0.0; 3], [1.0; 3], data)?;
    Ok(Artefact::ShaperCube { shaper, cube })
}

/// The vendored-artefact text format, version 1. A header line, any number of
/// `#` provenance lines, the shaper, then the cube red-fastest, one sample per
/// line. Only the shaper + cube shape is written: the factorised shape keeps
/// live steps for its out-of-domain tail, which is code, not data.
const ARTEFACT_HEADER: &str = "lumit-colour artefact 1";

/// A baked artefact plus where it came from — the provenance docs/impl/ocio.md
/// §4.1 requires of every vendored reference bake (library version, generation
/// script), carried in the file rather than in someone's memory.
#[derive(Debug, Clone, PartialEq)]
pub struct VendoredArtefact {
    pub provenance: Vec<String>,
    pub artefact: Artefact,
}

impl VendoredArtefact {
    /// Write the artefact out. Floats print in Rust's shortest round-tripping
    /// form, so reading the file back gives the same bits.
    pub fn to_text(&self) -> Result<String> {
        let Artefact::ShaperCube { shaper, cube } = &self.artefact else {
            return Err(ColourError::Parse {
                what: "a vendored artefact".to_string(),
                reason: "only the shaper and cube shape can be written to a file".to_string(),
            });
        };
        let mut s = String::with_capacity(cube.data.len() * 24);
        s.push_str(ARTEFACT_HEADER);
        s.push('\n');
        for line in &self.provenance {
            s.push_str("# ");
            s.push_str(line);
            s.push('\n');
        }
        match shaper {
            Shaper::Lg2 {
                min_log2,
                max_log2,
                offset,
            } => s.push_str(&format!("shaper lg2 {min_log2} {max_log2} {offset}\n")),
            Shaper::Uniform { min, max } => s.push_str(&format!("shaper uniform {min} {max}\n")),
        }
        s.push_str(&format!("cube {}\n", cube.size));
        for sample in &cube.data {
            s.push_str(&format!("{} {} {}\n", sample[0], sample[1], sample[2]));
        }
        Ok(s)
    }

    /// Read one back.
    pub fn from_text(what: &str, text: &str) -> Result<Self> {
        let bad = |reason: String| ColourError::Parse {
            what: what.to_string(),
            reason,
        };
        let mut provenance = Vec::new();
        let mut shaper: Option<Shaper> = None;
        let mut size: Option<usize> = None;
        let mut data: Vec<[f32; 3]> = Vec::new();
        let mut seen_header = false;

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if !seen_header {
                if line != ARTEFACT_HEADER {
                    return Err(bad(format!(
                        "expected {ARTEFACT_HEADER:?} on the first line"
                    )));
                }
                seen_header = true;
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                provenance.push(rest.trim().to_string());
                continue;
            }
            let mut tokens = line.split_whitespace();
            match tokens.next() {
                Some("shaper") => {
                    let nums: Vec<f32> = tokens
                        .clone()
                        .skip(1)
                        .filter_map(|t| t.parse::<f32>().ok())
                        .collect();
                    shaper = match (tokens.next(), nums.as_slice()) {
                        (Some("lg2"), [min_log2, max_log2, offset]) => Some(Shaper::Lg2 {
                            min_log2: *min_log2,
                            max_log2: *max_log2,
                            offset: *offset,
                        }),
                        (Some("uniform"), [min, max]) => Some(Shaper::Uniform {
                            min: *min,
                            max: *max,
                        }),
                        _ => return Err(bad(format!("could not read the shaper line {line:?}"))),
                    };
                }
                Some("cube") => {
                    let n = tokens
                        .next()
                        .and_then(|t| t.parse::<usize>().ok())
                        .ok_or_else(|| bad(format!("could not read the cube size in {line:?}")))?;
                    data.reserve(n.saturating_mul(n).saturating_mul(n).min(1 << 22));
                    size = Some(n);
                }
                Some(first) => {
                    let rest: Vec<&str> = tokens.collect();
                    let (Ok(r), [g, b]) = (first.parse::<f32>(), rest.as_slice()) else {
                        return Err(bad(format!("could not read the sample line {line:?}")));
                    };
                    let (Ok(g), Ok(b)) = (g.parse::<f32>(), b.parse::<f32>()) else {
                        return Err(bad(format!("could not read the sample line {line:?}")));
                    };
                    data.push([r, g, b]);
                }
                None => continue,
            }
        }

        let (Some(shaper), Some(size)) = (shaper, size) else {
            return Err(bad("the file states no shaper or no cube size".to_string()));
        };
        let cube = Cube::new(what, size, [0.0; 3], [1.0; 3], data)?;
        Ok(Self {
            provenance,
            artefact: Artefact::ShaperCube { shaper, cube },
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::op::{CdlParams, Direction};

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn the_default_shaper_covers_zero_to_thirty_two() {
        let s = Shaper::DEFAULT;
        assert!((s.forward(0.0) - 0.0).abs() < 1e-6);
        assert!((s.forward(32.0) - 1.0).abs() < 1e-4, "{}", s.forward(32.0));
        // And undoing it lands back where it started.
        for x in [0.0_f32, 0.001, 0.18, 1.0, 8.0, 31.0] {
            let back = s.inverse(s.forward(x));
            assert!((back - x).abs() <= 1e-3 * x.max(1.0), "{x} → {back}");
        }
    }

    #[test]
    fn the_shaper_clamps_rather_than_producing_nonsense() {
        let s = Shaper::DEFAULT;
        assert_eq!(s.forward(-100.0), 0.0);
        assert_eq!(s.forward(1e9), 1.0);
        assert!(s.inverse(0.0).is_finite());
    }

    fn srgb_chain() -> Chain {
        Chain::new(vec![
            Op::MonCurve {
                gamma: [2.4; 3],
                offset: [0.055; 3],
                dir: Direction::Forward,
            },
            Op::Matrix([
                1.1, -0.05, -0.05, 0.0, -0.02, 1.03, -0.01, 0.0, 0.0, -0.1, 1.1, 0.0,
            ]),
        ])
    }

    #[test]
    fn a_factorable_chain_bakes_to_curves_and_a_matrix() {
        let baked = bake(&srgb_chain(), Shaper::DEFAULT).expect("bakes");
        let Artefact::Factorised { stages } = &baked else {
            panic!("expected the factorised shape, got {baked:?}");
        };
        assert_eq!(stages.len(), 2);
        assert!(matches!(stages.first(), Some(Stage::Curve(_))));
        assert!(matches!(stages.get(1), Some(Stage::Matrix(_))));
    }

    #[test]
    fn the_factorised_bake_matches_exact_evaluation_to_the_stated_bound() {
        // §5.4's factorised bound: ≤ 1e-5 against exact evaluation.
        let chain = srgb_chain();
        let baked = bake(&chain, Shaper::DEFAULT).expect("bakes");
        for i in 0..=500 {
            let x = i as f32 / 500.0;
            let c = [x, 1.0 - x, (x * 0.7 + 0.1).min(1.0)];
            let got = baked.eval(c);
            let want = chain.eval(c);
            assert!(close(got, want, 1e-5), "at {c:?}: {got:?} vs {want:?}");
        }
    }

    #[test]
    fn the_factorised_bake_still_answers_outside_its_sampled_range() {
        let chain = srgb_chain();
        let baked = bake(&chain, Shaper::DEFAULT).expect("bakes");
        for c in [[-0.2_f32, 0.5, 0.5], [4.0, 0.5, -1.0], [12.0, 12.0, 12.0]] {
            let got = baked.eval(c);
            let want = chain.eval(c);
            assert!(close(got, want, 1e-4), "at {c:?}: {got:?} vs {want:?}");
        }
    }

    /// A chain that mixes channels, so it cannot factorise — and smooth, so
    /// §5.4's in-domain bound is the thing being measured rather than a clamp.
    fn mixing_chain() -> Chain {
        Chain::new(vec![Op::Cdl {
            params: CdlParams {
                slope: [1.1, 0.95, 1.02],
                saturation: 1.3,
                clamp: false,
                ..CdlParams::default()
            },
            dir: Direction::Forward,
        }])
    }

    #[test]
    fn a_channel_mixing_chain_bakes_to_a_cube() {
        let baked = bake(&mixing_chain(), Shaper::DEFAULT).expect("bakes");
        let Artefact::ShaperCube { cube, .. } = &baked else {
            panic!("expected a cube, got {baked:?}");
        };
        assert_eq!(cube.size, CUBE_SIZE);
        assert_eq!(cube.data.len(), CUBE_SIZE * CUBE_SIZE * CUBE_SIZE);
    }

    /// The shape a real view transform has: a primaries matrix, a smooth tone
    /// curve that brings scene light into 0–1, then a display encode. No hard
    /// corners, output bounded — which is what §5.4's bound is stated for.
    fn view_chain() -> Chain {
        let n = 4096;
        let tone: Vec<[f32; 3]> = (0..n)
            .map(|i| {
                let x = 32.0 * i as f32 / (n - 1) as f32;
                [x / (x + 1.0); 3]
            })
            .collect();
        Chain::new(vec![
            Op::Matrix([
                1.1, -0.08, -0.02, 0.0, -0.05, 1.09, -0.04, 0.0, -0.01, -0.1, 1.11, 0.0,
            ]),
            Op::Lut1d {
                curve: Curve::new("a tone curve", [0.0, 32.0], tone).expect("well-formed"),
                dir: Direction::Forward,
            },
            Op::MonCurve {
                gamma: [2.4; 3],
                offset: [0.055; 3],
                dir: Direction::Inverse,
            },
        ])
    }

    fn worst_error(
        chain: &Chain,
        baked: &Artefact,
        colours: impl Iterator<Item = [f32; 3]>,
    ) -> f32 {
        let mut worst = 0.0_f32;
        for c in colours {
            let (got, want) = (baked.eval(c), chain.eval(c));
            for k in 0..3 {
                worst = worst.max((got[k] - want[k]).abs());
            }
        }
        worst
    }

    #[test]
    fn the_cube_bake_matches_exact_evaluation_in_domain() {
        // §5.4's shaper + cube bound: ≤ 2e-3 on display-encoded output, over
        // scene-linear inputs inside the shaper's domain — neutrals and the
        // ordinary colours a picture is mostly made of.
        let chain = view_chain();
        // The chain factorises on paper (matrix, curve, curve), so the cube is
        // asked for by name: this test is about the cube form's own error.
        let baked = bake_cube(&chain, Shaper::DEFAULT).expect("bakes");
        let neutrals = (0..=200).map(|i| {
            let x = 32.0 * i as f32 / 200.0;
            [x, x, x]
        });
        let mild = (0..=200).map(|i| {
            let x = 32.0 * i as f32 / 200.0;
            [x, x * 0.8, x * 0.6]
        });
        let worst = worst_error(&chain, &baked, neutrals.chain(mild));
        assert!(worst <= 2e-3, "worst display-encoded error was {worst}");
    }

    #[test]
    fn deep_saturation_is_where_the_cube_bake_is_least_accurate() {
        // The risk §5.4 names, measured rather than asserted away. A matrix
        // mixes channels *linearly*, but the cube's grid is spaced
        // *logarithmically*, so a bright, deeply saturated colour whose mixed
        // result nearly cancels to zero is read off a coarse part of the grid
        // and then stretched by the display encode's steep toe. In-gamut
        // material never goes near this. The number is a ceiling to tighten
        // when the shaper gains its negative lobe, not a target.
        let chain = view_chain();
        let baked = bake_cube(&chain, Shaper::DEFAULT).expect("bakes");
        let saturated = (0..=200).map(|i| {
            let x = 32.0 * i as f32 / 200.0;
            [x, x * 0.25, x * 0.05]
        });
        let worst = worst_error(&chain, &baked, saturated);
        assert!(worst <= 5e-3, "worst error on deep saturation was {worst}");
        assert!(
            worst > 2e-3,
            "deep saturation is no longer the worst case ({worst}); tighten the bound above"
        );
    }

    #[test]
    fn baking_twice_gives_the_same_bytes() {
        // Determinism (docs/14 §3): nothing in the bake reads a clock, a hash
        // order or a thread, so two bakes of one chain are the same table.
        let chain = mixing_chain();
        let a = bake(&chain, Shaper::DEFAULT).expect("bakes");
        let b = bake(&chain, Shaper::DEFAULT).expect("bakes");
        assert!(a == b, "two bakes of one chain differed");
        let f1 = bake(&srgb_chain(), Shaper::DEFAULT).expect("bakes");
        let f2 = bake(&srgb_chain(), Shaper::DEFAULT).expect("bakes");
        assert!(f1 == f2, "two factorised bakes of one chain differed");
    }

    #[test]
    fn a_vendored_artefact_round_trips_through_its_file_format() {
        let chain = mixing_chain();
        // A small cube, so the test file stays a test file.
        let baked = bake_cube(&chain, Shaper::DEFAULT).expect("bakes");
        let vendored = VendoredArtefact {
            provenance: vec![
                "generated by: (pending) the reference OpenColorIO library".to_string(),
                "style: an example, not a shipped bake".to_string(),
            ],
            artefact: baked,
        };
        let text = vendored.to_text().expect("writes");
        let back = VendoredArtefact::from_text("the example", &text).expect("reads");
        assert_eq!(back.provenance, vendored.provenance);
        assert!(back.artefact == vendored.artefact, "the bytes changed");
    }

    #[test]
    fn a_factorised_artefact_refuses_to_be_written_as_a_file() {
        let vendored = VendoredArtefact {
            provenance: Vec::new(),
            artefact: factorise(&srgb_chain()).expect("factorises"),
        };
        assert!(matches!(vendored.to_text(), Err(ColourError::Parse { .. })));
    }

    #[test]
    fn a_truncated_artefact_file_is_a_typed_error_not_a_panic() {
        assert!(VendoredArtefact::from_text("stub", "not a lumit artefact").is_err());
        assert!(VendoredArtefact::from_text("stub", "lumit-colour artefact 1\ncube 2\n").is_err());
    }
}
