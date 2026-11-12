//! A shot's grade baked into a 3D LUT, for a `.cube` another application can
//! load.
//!
//! The grade is the shot's Colour effects, the enabled ones, in stack order.
//! Each runs on its CPU twin over a picture whose pixels are the cube's
//! lattice, decoded from the cube's encoding into the working linear. What
//! depends on where a pixel is can't live in a LUT, so every effect runs
//! its pointwise part only ([`EffectDef::apply_cpu_pointwise`]). The LUT and
//! OCIO effects read their tables here, on the CPU, the way their kernels do.
//!
//! Correction nodes meet as the render joins them, Parallel and Layer runs
//! included. A qualifier keys on the pixel's own colour, so it stays; windows
//! and mattes are about where a pixel is, so they go and the node grades as
//! if fully keyed. Any other effect leaves the colour alone but still ends a
//! run where it stands, as it does in the render.
//!
//! Pure work on whichever thread calls it, never the interface's: a 65-point
//! cube is about 275,000 colours through every effect.
//!
//! [`EffectDef::apply_cpu_pointwise`]: lumit_core::fx::registry::EffectDef::apply_cpu_pointwise

use std::sync::Arc;

use lumit_colour::bake::Artefact;
use lumit_core::expression::ExpressionContext;
use lumit_core::fx::cpu::{blend_mix, blend_seam};
use lumit_core::fx::effects::correction::combine::Run;
use lumit_core::fx::effects::{lut::apply_cube_cpu, ocio::apply_table_cpu};
use lumit_core::fx::markers::MarkerContext;
use lumit_core::fx::params::Params;
use lumit_core::fx::schema::FxCategory;
use lumit_core::lut::{Lut, Lut3d, LutSpace};
use lumit_core::model::EffectInstance;

use crate::colour::{Loaded, OcioRequest, TableRequest};

/// The most points a side a baked cube may have. 65 is the largest grading
/// applications ask for, and 65³ colours is already a quarter of a million.
pub const MAX_CUBE_SIZE: usize = 65;

/// The encoding a baked cube's colours are in, going in and coming out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CubeSpace {
    /// The working linear with a 2.4 gamma: Rec. 709 video as a monitor shows it.
    Video,
    /// ACEScct, the log most grading is done in.
    Log,
    /// The sRGB curve.
    Srgb,
}

impl CubeSpace {
    /// One encoded value into the working linear.
    #[must_use]
    pub fn decode(self, v: f32) -> f32 {
        match self {
            Self::Video => v.signum() * v.abs().powf(2.4),
            Self::Log => {
                if v <= ACESCCT_BREAK {
                    (v - ACESCCT_B) / ACESCCT_A
                } else {
                    (v * 17.52 - 9.72).exp2().min(65504.0)
                }
            }
            Self::Srgb => LutSpace::Srgb.to_linear([v; 3])[0],
        }
    }

    /// One working linear value into this encoding.
    #[must_use]
    pub fn encode(self, x: f32) -> f32 {
        match self {
            Self::Video => x.signum() * x.abs().powf(1.0 / 2.4),
            Self::Log => {
                if x <= 0.007_812_5 {
                    ACESCCT_A * x + ACESCCT_B
                } else {
                    (x.log2() + 9.72) / 17.52
                }
            }
            Self::Srgb => LutSpace::Srgb.from_linear([x; 3])[0],
        }
    }
}

// ACEScct's straight toe, from Academy S-2016-001, and where it meets the log.
const ACESCCT_A: f32 = 10.540_237;
const ACESCCT_B: f32 = 0.072_905_53;
const ACESCCT_BREAK: f32 = 0.155_251_14;

/// Why a grade could not be baked.
#[derive(Debug, thiserror::Error)]
pub enum GradeLutError {
    #[error("a cube needs 2 to {MAX_CUBE_SIZE} points a side, not {0}")]
    Size(usize),
}

/// Whether an effect is part of a shot's grade: one of the Colour effects.
#[must_use]
pub fn in_grade(e: &EffectInstance) -> bool {
    lumit_core::fx::builtins::def(&e.effect.match_name)
        .is_some_and(|d| d.schema().category == FxCategory::Colour)
}

/// A table a grade's LUT or OCIO effect reads, held on the CPU.
enum Table {
    Cube(Lut3d),
    Ocio(Arc<Artefact>),
}

/// The grade in `effects` baked into a `size`-point cube in `space`, with
/// every row read at layer time `lt` through `context`. `diag_px` is the
/// picture's diagonal, which a row measured against it is read by.
/// `colour` is the project's OCIO config, which the OCIO effects that name
/// its spaces need. A table that will not load is left out, as the render
/// leaves it out.
pub fn bake(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    context: Arc<ExpressionContext>,
    colour: Option<&Arc<Loaded>>,
    size: usize,
    space: CubeSpace,
) -> Result<Lut3d, GradeLutError> {
    if !(2..=MAX_CUBE_SIZE).contains(&size) {
        return Err(GradeLutError::Size(size));
    }
    let stack: Vec<EffectInstance> = effects.iter().filter(|e| e.enabled).cloned().collect();
    let tables: Vec<(uuid::Uuid, Table)> = stack
        .iter()
        .filter(|e| in_grade(e))
        .filter_map(|e| Some((e.id, table_of(e, lt, colour)?)))
        .collect();
    let ops = lumit_core::fx::resolved::resolve_stack(
        &stack,
        lt,
        diag_px,
        1.0,
        &MarkerContext::NONE,
        context,
    );

    // One pixel a lattice point, red fastest, so the picture is the cube's
    // own order and needs no shuffling on the way out.
    let (w, h) = (size as u32, (size * size) as u32);
    let step = 1.0 / (size - 1) as f32;
    let mut rgba = Vec::with_capacity(size * size * size * 4);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let at = |i: usize| space.decode(i as f32 * step);
                rgba.extend_from_slice(&[at(r), at(g), at(b), 1.0]);
            }
        }
    }

    let mut joined = Run::pointwise();
    for (i, fx) in ops.iter().enumerate() {
        joined.begin(&ops, i, &mut rgba);
        if fx.def.schema().category != FxCategory::Colour || joined.layers() {
            joined.end(&mut rgba, w, h, &fx);
            continue;
        }
        let table = tables
            .iter()
            .find(|(id, _)| *id == fx.instance)
            .map(|(_, t)| t);
        let run = |rgba: &mut [f32], p: Params<'_>| match table {
            Some(Table::Cube(cube)) => apply_cube_cpu(cube, rgba, p),
            Some(Table::Ocio(artefact)) => apply_table_cpu(rgba, p, |c| artefact.eval(c)),
            None => fx.def.apply_cpu_pointwise(rgba, w, h, p),
        };
        // The Blend row, applied as `apply_stack` applies it.
        match blend_seam(fx.def.schema(), fx.params) {
            Some((mode, mix, entries)) => {
                let input = rgba.clone();
                run(&mut rgba, Params::new(&entries));
                blend_mix(&mut rgba, &input, mode, mix);
            }
            None => run(&mut rgba, fx.params),
        }
        joined.end(&mut rgba, w, h, &fx);
    }

    let data = rgba
        .chunks_exact(4)
        .map(|px| {
            let a = if px[3] > 0.0 { px[3] } else { 1.0 };
            [0, 1, 2].map(|c| space.encode(px[c] / a))
        })
        .collect();
    Ok(Lut3d {
        size,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        data,
    })
}

/// The table `e` reads at `lt`, if it is a LUT or OCIO effect whose table
/// loads. The render's own requests, so the two read the same file.
fn table_of(e: &EffectInstance, lt: f64, colour: Option<&Arc<Loaded>>) -> Option<Table> {
    let request = crate::build::colour_tables(std::slice::from_ref(e), lt)
        .into_iter()
        .next()
        .flatten()?;
    Some(match request {
        // Only a 3D cube, as the render takes only a 3D cube.
        TableRequest::Cube(path) => {
            let text = std::fs::read_to_string(path).ok()?;
            let Lut::Cube3d(cube) = lumit_core::lut::parse_cube(&text).ok()? else {
                return None;
            };
            Table::Cube(cube)
        }
        TableRequest::Ocio(OcioRequest::Config(edge)) => {
            Table::Ocio(colour.filter(|l| l.usable())?.artefact(&edge)?)
        }
        TableRequest::Ocio(OcioRequest::File { path, inverse }) => {
            Table::Ocio(Arc::new(crate::colour::file_artefact(&path, inverse)?))
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::model::{EffectValue, FileParam};

    fn effect(name: &str, rows: &[(&str, EffectValue)]) -> EffectInstance {
        let mut e = lumit_core::fx::builtins::instantiate(name).unwrap();
        for (id, value) in rows {
            let p = e.params.iter_mut().find(|p| p.id == *id).unwrap();
            p.value = value.clone();
        }
        e
    }

    /// A written cube, read back by Lumit's own reader, gives what the grade
    /// does. The grade is a stop up and a LUT that turns red into green, green
    /// into blue and blue into red, both straight lines in Video, so the cube
    /// holds them exactly between its points as well as on them. A blur and a
    /// switched-off Invert ride along and must be left out.
    #[test]
    fn a_written_cube_reads_back_as_the_grade() {
        let n = 5;
        let mut turn = Lut3d {
            size: n,
            domain_min: [0.0; 3],
            domain_max: [4.0; 3],
            data: Vec::new(),
        };
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    turn.data.push([b as f32, r as f32, g as f32]);
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("turn.cube");
        std::fs::write(&file, lumit_core::lut::write_cube(&turn, "turn")).unwrap();

        let mut invert = effect("invert", &[]);
        invert.enabled = false;
        let grade = [
            effect(
                "exposure",
                &[("stops", EffectValue::Float(Property::fixed(1.0)))],
            ),
            effect("blur", &[]),
            invert,
            effect(
                "lut",
                &[(
                    "file",
                    EffectValue::File(FileParam::single(file.to_string_lossy())),
                )],
            ),
        ];
        let baked = bake(
            &grade,
            0.0,
            2203.0,
            Arc::new(ExpressionContext::detached()),
            None,
            17,
            CubeSpace::Video,
        )
        .unwrap();
        let text = lumit_core::lut::write_cube(&baked, "shot");
        let Lut::Cube3d(read) = lumit_core::lut::parse_cube(&text).unwrap() else {
            panic!("a 3D cube was written");
        };

        let space = CubeSpace::Video;
        let expected = |c: [f32; 3]| {
            let x = c.map(|v| space.decode(v) * 2.0);
            [x[2], x[0], x[1]].map(|v| space.encode(v))
        };
        // Every point of the cube, and a spread of colours between them.
        let mut colours: Vec<[f32; 3]> = (0..17 * 17 * 17)
            .map(|i| [i % 17, i / 17 % 17, i / 289].map(|k| k as f32 / 16.0))
            .collect();
        colours.extend((0..500).map(|i| {
            let t = i as f32 * 0.618_034;
            [t.fract(), (t * 1.7).fract(), (t * 2.3).fract()]
        }));
        for c in colours {
            let (got, want) = (read.sample(c), expected(c));
            for k in 0..3 {
                assert!(
                    (got[k] - want[k]).abs() < 1e-3,
                    "{c:?} came back {got:?}, the grade gives {want:?}"
                );
            }
        }
    }

    /// A Parallel run baked into a cube gives what the render gives a flat
    /// patch of each colour: exactly on the cube's points, and within what
    /// its points can hold between them. The run must also differ from the
    /// same nodes in Serial, or the test would prove nothing.
    #[test]
    fn a_parallel_run_bakes_as_the_render_draws_it() {
        let float = |v: f64| EffectValue::Float(Property::fixed(v));
        let colour = |v: [f64; 3]| {
            let [r, g, b] = v.map(Property::fixed);
            EffectValue::Colour([r, g, b, Property::fixed(1.0)])
        };
        let stack = |combine: u32| {
            vec![
                effect("correction", &[("gain", colour([1.3, 1.0, 0.8]))]),
                effect(
                    "correction",
                    &[
                        ("combine", EffectValue::Choice(combine)),
                        ("saturation", float(140.0)),
                    ],
                ),
                effect(
                    "correction",
                    &[
                        ("combine", EffectValue::Choice(combine)),
                        ("lift", colour([0.05, 0.0, 0.02])),
                    ],
                ),
            ]
        };
        let context = || Arc::new(ExpressionContext::detached());
        let space = CubeSpace::Video;
        let n = 17;
        let cube = bake(&stack(1), 0.0, 2203.0, context(), None, n, space).unwrap();
        let serial = bake(&stack(0), 0.0, 2203.0, context(), None, n, space).unwrap();
        assert!(
            cube.data
                .iter()
                .zip(&serial.data)
                .any(|(a, b)| (0..3).any(|k| (a[k] - b[k]).abs() > 1e-2)),
            "Parallel baked the same as Serial"
        );

        let ops = lumit_core::fx::resolved::resolve_stack(
            &stack(1),
            0.0,
            2203.0,
            1.0,
            &MarkerContext::NONE,
            context(),
        );
        let drawn = |c: [f32; 3]| {
            let x = c.map(|v| space.decode(v));
            let mut patch: Vec<f32> = (0..4).flat_map(|_| [x[0], x[1], x[2], 1.0]).collect();
            lumit_core::fx::cpu::apply_stack(&mut patch, 2, 2, &ops);
            [0, 1, 2].map(|k| space.encode(patch[k]))
        };
        let step = 1.0 / (n - 1) as f32;
        for i in 0..n * n * n {
            let c = [i % n, i / n % n, i / (n * n)].map(|k| k as f32 * step);
            let (got, want) = (cube.sample(c), drawn(c));
            for k in 0..3 {
                assert!(
                    (got[k] - want[k]).abs() < 1e-4,
                    "{c:?}: {got:?} vs {want:?}"
                );
            }
            // Between points, away from black going in and coming out: the
            // gamma's toe is too steep for a cube's straight lines to follow.
            let mid = c.map(|v| (v + 0.5 * step).min(1.0));
            let (got, want) = (cube.sample(mid), drawn(mid));
            if mid.iter().chain(&want).any(|&v| v < 0.25) {
                continue;
            }
            for k in 0..3 {
                assert!(
                    (got[k] - want[k]).abs() < 1e-2,
                    "{mid:?}: {got:?} vs {want:?}"
                );
            }
        }
    }
}
