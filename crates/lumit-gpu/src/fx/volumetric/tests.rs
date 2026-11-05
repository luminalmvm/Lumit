#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::fx::common::{readback_linear_f32, upload_linear_f32};
use crate::fx::lighting::{CasterPlane, LightingKind, LightingLight};

/// A box of even smoke leaves what Beer and Lambert say of what is behind
/// it, stops at a surface in front of it and shows its colour where light
/// reaches it, and a ray past the box gets nothing.
#[test]
fn smoke_in_a_box_dims_by_its_thickness_and_stops_at_depth() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (64u32, 64u32);
    let fields = ctx.device.create_texture(&wgpu::TextureDescriptor {
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ..super::field_descriptor("test-field", [8; 3])
    });
    let density = vec![[half::f16::from_f32(0.5).to_bits(), 0u16, 0, 0]; 512];
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &fields,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&density),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8 * 8),
            rows_per_image: Some(8),
        },
        fields.size(),
    );
    // A 24 pixel cube in the middle of the comp, two ray lengths from the eye.
    let look = super::VolumeLook {
        from_box: [
            [24.0, 0.0, 0.0],
            [0.0, 24.0, 0.0],
            [0.0, 0.0, 24.0],
            [20.0, 20.0, 88.0],
        ],
        thickness: 0.1,
        smoke: [0.8, 0.4, 0.2],
        shadow: 1.0,
        fire_low: [0.0; 3],
        fire_high: [0.0; 3],
        fire_gain: 0.0,
        fire_start: 0.5,
        fire_glow: 0.0,
        fade: 0.0,
        detail: 0.0,
        clock: 0.0,
    };
    let op = super::HazeOp {
        eye: [32.0, 32.0, -100.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        forward: [0.0, 0.0, 1.0],
        zoom: 100.0,
        comp: (w as f32, h as f32),
        scale: 1.0,
        near: 0.0,
        centre: [32.0, 32.0, 100.0],
        radius: 1.0,
        lights: Vec::new(),
        volumes: vec![super::VolumeOp { fields, look }],
        no_shadow: None,
    };
    let walk = |at: f32| {
        let depth = upload_linear_f32(&ctx, &[at, 0.0, 0.0, 1.0].repeat((w * h) as usize), w, h);
        let s = fx.haze_scatter(&ctx, &depth, &op, None, None);
        readback_linear_f32(&ctx, &s, w, h).unwrap()
    };
    let pixel = |img: &[f32], x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    };
    let far = 65000.0;
    let open = walk(far);
    // Down the middle the ray crosses all 24 pixels of the box.
    let [r, g, b, left] = pixel(&open, 32, 32);
    let want = (-0.5f32 * 0.1 * 24.0).exp();
    assert!((left - want).abs() < 0.02, "left {left}, want {want}");
    assert!(r > g && g > b && b > 0.0, "the smoke's colour: {r} {g} {b}");
    // Off to the side it misses the box altogether.
    assert_eq!(pixel(&open, 4, 4), [0.0, 0.0, 0.0, 1.0]);
    // A surface in front of the box hides all of it, one half way in hides half.
    let hidden = pixel(&walk(1.0), 32, 32);
    assert_eq!(hidden, [0.0, 0.0, 0.0, 1.0]);
    let half_in = pixel(&walk(2.0), 32, 32)[3];
    let want = (-0.5f32 * 0.1 * 12.0).exp();
    assert!((half_in - want).abs() < 0.03, "left {half_in}, want {want}");
}

/// The walk must agree with `lumit_core::haze::march`, and a spot's beam
/// must show where its cone is and be cut below a card that casts.
#[test]
fn haze_matches_the_oracle_and_a_caster_cuts_the_beam() {
    use lumit_core::haze::{jitter, march, HazeLook, HazeView};
    use lumit_core::lighting::{
        caster_box, caster_square, ShadingKind, ShadingLight, ShadowCaster,
    };
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (64u32, 48u32);
    // A spot overhead, aimed straight down through the middle.
    let at = [32.0, -40.0, 60.0];
    let (cone_cos, cone_inner) = (20f32.to_radians().cos(), 15f32.to_radians().cos());
    let spot = ShadingLight {
        corners: [at; 4],
        colour: [1.0, 0.9, 0.7],
        kind: ShadingKind::Point,
        falloff: lumit_core::model::LightFalloff::None,
        radius_px: 500.0,
        falloff_px: 0.0,
        legacy: false,
        cone_cos,
        cone_inner,
        axis: [0.0, 1.0, 0.0],
        shadow_darkness: 1.0,
        shadow_diffusion: 0.0,
    };
    let look = HazeLook {
        amount: 100.0,
        density: 100.0,
        colour: [1.0; 3],
        falloff: 40.0,
        scatter: 30.0,
        noise: 50.0,
        noise_size: 20.0,
        drift: 3.0,
    };
    let gpu_light = LightingLight {
        corners: [at; 4],
        colour: spot.colour,
        kind: LightingKind::Point,
        falloff: 0,
        radius_px: 500.0,
        falloff_px: 0.0,
        legacy: false,
        cone_cos,
        cone_inner,
        axis: spot.axis,
        shadow_darkness: 1.0,
        shadow_diffusion: 0.0,
    };
    let view = HazeView {
        eye: [32.0, 24.0, -100.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        forward: [0.0, 0.0, 1.0],
        zoom: 100.0,
        comp: (w as f32, h as f32),
        centre: [32.0, 24.0, 60.0],
        radius: 300.0,
    };
    let op = super::HazeOp {
        eye: view.eye,
        right: view.right,
        up: view.up,
        forward: view.forward,
        zoom: view.zoom,
        comp: view.comp,
        scale: 1.0,
        near: 0.0,
        centre: view.centre,
        radius: view.radius,
        lights: vec![(
            gpu_light,
            super::Haze {
                amount: look.amount,
                density: look.density,
                falloff: look.falloff,
                scatter: look.scatter,
                noise: look.noise,
                noise_size: look.noise_size,
                drift: look.drift,
            },
        )],
        volumes: Vec::new(),
        no_shadow: None,
    };
    // Nothing in the scene, so every ray runs out of the haze.
    let far = 60000.0;
    let depth = upload_linear_f32(&ctx, &[far, 0.0, 0.0, 1.0].repeat((w * h) as usize), w, h);
    // A flat card across the beam, below the light.
    let plane = CasterPlane {
        origin: [12.0, 10.0, 20.0],
        u: [40.0, 0.0, 0.0],
        v: [0.0, 0.0, 80.0],
        transmission: 0.0,
        opacity: 1.0,
    };
    let card = [0.5, 0.5, 0.5, 1.0].repeat(16);
    let size = 64;
    let tex = upload_linear_f32(&ctx, &card, 4, 4);
    let squares = fx.shadow_casters(&ctx, &[(&tex, plane)], size, false, 0.0);
    let place = caster_box(size, 40.0, 80.0, 0.0);
    let cpu_casters = [ShadowCaster {
        origin: plane.origin,
        u: plane.u,
        v: plane.v,
        transmission: 0.0,
        place: place.map(|t| t as f32 / size as f32),
        levels: caster_square(&card, 4, 4, 1.0, size, place, false),
        size,
    }];
    let (hw, hh) = (w / 2, h / 2);
    let run = |cut: bool| {
        let s = fx.haze_scatter(
            &ctx,
            &depth,
            &op,
            cut.then_some(squares.as_ref()).flatten(),
            None,
        );
        readback_linear_f32(&ctx, &s, hw, hh).unwrap()
    };
    let (open, shut) = (run(false), run(true));
    for (gpu, casters) in [(&open, &[][..]), (&shut, &cpu_casters[..])] {
        let mut worst = 0.0f32;
        for y in 0..hh {
            for x in 0..hw {
                let q = [(x * 2) as f32 + 0.5, (y * 2) as f32 + 0.5];
                let cpu = march(&view, q, far, jitter(x, y), &[(spot, look)], casters);
                let i = ((y * hw + x) * 4) as usize;
                for k in 0..4 {
                    worst = worst.max((cpu[k] - gpu[i + k]).abs() / cpu[k].abs().max(0.05));
                }
            }
        }
        assert!(worst < 0.02, "worst relative difference {worst}");
    }
    let lum = |img: &[f32], x: u32, y: u32| {
        let i = ((y * hw + x) * 4) as usize;
        img[i] + img[i + 1] + img[i + 2]
    };
    // Above the card the beam shows and the card changes nothing; below
    // it the card has cut the beam away; well off to the side there is no
    // beam at all.
    let (above, below) = ((16, 2), (16, 20));
    assert!(lum(&open, above.0, above.1) > 0.05);
    assert!((lum(&shut, above.0, above.1) - lum(&open, above.0, above.1)).abs() < 1e-3);
    assert!(lum(&open, below.0, below.1) > 0.05);
    assert!(lum(&shut, below.0, below.1) < 0.2 * lum(&open, below.0, below.1));
    assert!(lum(&open, 1, 12) < 0.1 * lum(&open, above.0, above.1));
}
