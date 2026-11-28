use super::common::*;
use super::tests::{corpus, worst_diff};

#[test]
fn wgsl_light_rays_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::light_rays::{self as lr, LightRays, LightRaysParams};
    use lumit_core::fx::params::Params;
    use lumit_core::fx::registry::EffectMetadata;
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let base = LightRays::read(Params::EMPTY);
    let cases = [
        base,
        LightRays {
            source_x: -10.0,
            source_y: 30.0,
            ray_length: 8.0,
            rays_blend: lr::BLEND_NONE,
            ..base
        },
        LightRays {
            threshold: 30.0,
            use_mask: true,
            mask_radius: 8.0,
            mask_feather: 4.0,
            source_x: 10.3,
            source_y: 9.7,
            rays_blend: lr::BLEND_NORMAL,
            ..base
        },
        LightRays {
            colourise: 4,
            shimmer_amount: 80.0,
            shimmer_phase: 40.0,
            rays_blend: 3,
            ..base
        },
        LightRays {
            colourise: 15,
            based_on: lr::BASED_ON_ALPHA_EDGES,
            edge_thickness: 2.0,
            boost_light: 2.0,
            rays_blend: 8,
            mix: 60.0,
            ..base
        },
        LightRays {
            colourise: lr::COLOURISE_ONE,
            based_on: 1,
            source_opacity: 50.0,
            rays_opacity: 150.0,
            rays_blend: 6,
            ..base
        },
    ];
    for (n, case) in cases.iter().enumerate() {
        let p = case.packed(w, h);
        let mut cpu = img.clone();
        lr::light_rays(&mut cpu, w, h, &p);
        let op = super::light_rays::LightRaysOp {
            source: p.source,
            lengths: p.lengths.clone(),
            threshold: p.threshold,
            mask: p.mask,
            colourise: p.colourise,
            based_on: p.based_on,
            edge: p.edge,
            stops: p.stops,
            at: p.at,
            count: p.count,
            gain: p.gain,
            source_opacity: p.source_opacity,
            rays_opacity: p.rays_opacity,
            blend: p.blend,
            mix: p.mix,
            weights: std::array::from_fn(|k| {
                std::array::from_fn(|j| LightRaysParams::weight(k, j))
            }),
            scales: std::array::from_fn(|k| LightRaysParams::step(k, 1.0)),
        };
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.light_rays(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        assert!(worst < 4e-3, "case {n}: worst difference {worst}");
        let again = readback_linear_f32(&ctx, &fx.light_rays(&ctx, &tex, w, h, &op), w, h).unwrap();
        assert_eq!(gpu, again, "case {n}: GPU light rays must be bit-stable");
    }
}
