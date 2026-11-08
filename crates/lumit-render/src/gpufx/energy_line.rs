//! Energy line's GPU op: the path laid out here, on the host, and handed to
//! the card as the same pieces and tiles the CPU reference draws.

use lumit_core::fx::cpu::{glow_grid, GLOW_REACH};
use lumit_core::fx::effects::energy_line::{self as el, EnergyLine as S};
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::{blur::GlowOctaveOp, energy_line::EnergyLineOp, FxEngine};
use lumit_gpu::GpuContext;

use super::{AuxSlot, GpuEffect, Tex};

pub(super) struct EnergyLine;

impl GpuEffect for EnergyLine {
    fn match_name(&self) -> &'static str {
        "energy_line"
    }

    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let s = S::read(p);
        let Some(mut look) = s.look(p) else {
            return tex.clone();
        };
        let px_scale = p.float(S::DERIVED_PX_SCALE, 1.0);
        // The Line source's path is in its rows; the others arrive beside
        // the op, already in this picture's place if it was grown.
        let poly = if s.source == el::SOURCE_LINE {
            s.line(px_scale)
        } else {
            aux.mask_path().cloned().unwrap_or_default()
        };
        let (laid, tiles) = s.laid_out(&mut look, p, &poly, px_scale, w, h);
        draw_on(fx, ctx, tex, w, h, &look, &laid, &tiles)
    }
}

/// One frame's light on the card, from the pieces and tiles the CPU
/// reference draws from.
#[allow(clippy::too_many_arguments)]
fn draw_on(
    fx: &FxEngine,
    ctx: &GpuContext,
    tex: &Tex,
    w: u32,
    h: u32,
    look: &el::draw::Look,
    laid: &el::geometry::Pieces,
    tiles: &el::geometry::Bins,
) -> Tex {
    if laid.segs.is_empty() && look.composite {
        return tex.clone();
    }
    let pieces: Vec<[f32; 4]> = laid
        .segs
        .iter()
        .zip(&laid.along)
        .flat_map(|(s, u)| [*s, *u])
        .collect();
    let octaves = el::octaves(look, w, h).map(|o| GlowOctaveOp {
        step: o.step,
        grid: [glow_grid(w, o.step), glow_grid(h, o.step)],
        lambda: o.lambda,
        reach: GLOW_REACH * o.lambda,
        taps: o.taps,
        weight: o.weight,
    });
    let f = look.field;
    fx.energy_line(
        ctx,
        tex,
        w,
        h,
        &EnergyLineOp {
            pieces: &pieces,
            tiles: [tiles.cols, tiles.rows],
            starts: &tiles.start,
            items: &tiles.items,
            octaves: &octaves,
            core_colour: look.core_colour,
            glow_colour: look.glow_colour,
            halo: look.halo,
            samples: look.samples,
            core_half: look.core_half,
            taper: look.taper,
            softness: look.softness,
            core_warp: look.core_warp,
            glow_warp: look.glow_warp,
            inv_size: look.inv_size,
            evolution: look.evolution,
            drift: look.drift,
            glow_gain: look.glow_gain,
            flicker: look.flicker,
            composite: look.composite,
            mix: look.mix,
            seed: f.seed,
            noise_octaves: f.octaves,
            gain: f.gain,
            lacunarity: f.lacunarity,
            flags: u32::from(f.perlin) | (u32::from(f.turbulent) << 1),
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::expression::ExpressionContext;
    use lumit_core::fx::markers::MarkerContext;
    use lumit_core::mask::MaskPolyline;
    use lumit_core::model::EffectValue;

    /// The card draws what the CPU reference draws: a line, and a closed path
    /// whose lit stretch is slid round past its join across two shutter
    /// moments, both tapered, haloed, pushed about by the noise and
    /// flickering, over a picture with some alpha.
    #[test]
    fn the_card_draws_what_the_cpu_reference_draws() {
        let Some(gpu) = lumit_gpu::test_support::lease() else {
            return; // no GPU here
        };
        let (w, h) = (160u32, 96u32);
        let mut e = lumit_core::fx::builtins::instantiate("energy_line").unwrap();
        for (id, v) in [
            ("start_x", 20.0),
            ("start_y", 30.0),
            ("end_x", 140.0),
            ("end_y", 70.0),
            ("start", 10.0),
            ("end", 70.0),
            ("offset", 60.0),
            ("core_size", 8.0),
            ("end_size", 40.0),
            ("glow_size", 14.0),
            ("core_distortion", 3.0),
            ("glow_distortion", 6.0),
            ("distortion_size", 20.0),
            ("flicker", 30.0),
            ("wind_speed", 40.0),
        ] {
            if let Some(p) = e.params.iter_mut().find(|p| p.id == id) {
                p.value = EffectValue::Float(Property::fixed(v));
            }
        }
        let ops = lumit_core::fx::resolved::resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            (w as f32).hypot(h as f32),
            1.0,
            &MarkerContext::NONE,
            std::sync::Arc::new(ExpressionContext::detached()),
        );
        let p = ops.get(0).unwrap().params;
        let s = S::read(p);
        let mut look = s.look(p).unwrap();
        let src: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                let a = 0.25 + 0.5 * x;
                [0.3 * y * a, 0.2 * a, 0.1 * a, a]
            })
            .collect();
        let ring = {
            let n = 64;
            let points: Vec<[f32; 2]> = (0..=n)
                .map(|i| {
                    let t = std::f32::consts::TAU * (i % n) as f32 / n as f32;
                    [80.0 + 50.0 * t.cos(), 48.0 + 30.0 * t.sin()]
                })
                .collect();
            let mut arc = vec![0.0f32];
            for pair in points.windows(2) {
                let d = (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
                arc.push(arc.last().copied().unwrap_or(0.0) + d);
            }
            MaskPolyline {
                points,
                arc,
                closed: true,
                ..MaskPolyline::default()
            }
        };
        for (name, poly) in [("line", s.line(1.0)), ("ring", ring)] {
            let (mut laid, mut tiles) = s.laid_out(&mut look, p, &poly, 1.0, w, h);
            if name == "ring" {
                // A second shutter moment, the stretch a little further on.
                let width = 2.0 * look.core_half;
                laid.extend(el::geometry::pieces(
                    &poly,
                    1.0,
                    [10.0, 70.0, 66.0],
                    1,
                    width,
                ));
                look.samples = 2;
                tiles = el::geometry::bins(&laid, S::reach(&look), w, h);
            }
            assert!(!laid.segs.is_empty(), "{name}: something is lit");
            let mut cpu = src.clone();
            el::draw::render(&mut cpu, w, h, &look, &laid, &tiles);
            let tex = lumit_gpu::fx::common::upload_linear_f32(&gpu, &src, w, h);
            let out = draw_on(gpu.fx(), &gpu, &tex, w, h, &look, &laid, &tiles);
            let back = gpu.colour().readback_linear_f32(&gpu, &out).unwrap();
            let worst = cpu
                .iter()
                .zip(&back)
                .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-2, "{name}: worst difference {worst}");
            assert!(cpu != src, "{name}: the reference drew something");
        }
    }
}
