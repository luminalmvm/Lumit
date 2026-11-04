//! **B30 — a Model layer of a million triangles**, and **B33 — the same with
//! shadows** (docs/13 §2).
//!
//! # In plain terms
//!
//! One model drawn through a camera with two lights at 1080p, the comp's
//! default anti-aliasing, the way the mesh pass draws a Model layer each
//! frame. The model is uploaded once before the stopwatch starts, as the
//! engine keeps it between frames, so the number is the draw alone. B33 has
//! both lights cast, a point and a parallel one, so each frame makes their
//! shadow maps and the model is drawn taking them. **B34** makes the cubes an
//! Environment light lights with from a 2K panorama, which happens once a
//! picture, and **B35** is B30 lit by them with the background drawn.

use std::sync::Arc;

use crate::scenarios::{elapsed_ms, Measurement};
use lumit_gpu::fx::lighting::{LightingKind, LightingLight, LightingMaterial};
use lumit_scene::{fixtures, Material};
use std::time::Instant;

const SIZE: (u32, u32) = (1920, 1080);
const RUNS: u32 = 20;
const WARMUP: u32 = 4;
/// The fastest of these is kept, as for Particulate's three.
const REPEATS: u32 = 3;

/// Measure B30, B33, B34 and B35. `Err` when there is no graphics adapter.
pub fn budgets(progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
    let ctx = lumit_gpu::GpuContext::headless().map_err(|e| format!("no GPU adapter: {e}"))?;
    let fx = lumit_gpu::fx::FxEngine::new(&ctx);
    // A ball of 1000 by 500 squares, two triangles each, filling half the
    // frame's height.
    let scene = Arc::new(fixtures::scene(
        vec![(fixtures::sphere(0.5, 1000, 500, Some(0)), [0.0; 3])],
        vec![Material {
            base_colour: [0.8, 0.5, 0.2, 1.0],
            metallic: 0.0,
            roughness: 0.4,
            ..Material::default()
        }],
        Vec::new(),
    ));
    let (w, h) = (SIZE.0 as f32, SIZE.1 as f32);
    let zoom = w * 50.0 / 36.0;
    let eye = [w / 2.0, h / 2.0, -zoom];
    let light = |kind, at: [f32; 3], colour| LightingLight {
        corners: [at; 4],
        colour,
        kind,
        falloff: 0,
        radius_px: 0.0,
        falloff_px: 0.0,
        legacy: false,
        cone_cos: -2.0,
        cone_inner: -2.0,
        axis: [0.0, 0.0, 1.0],
        shadow_darkness: 0.0,
        shadow_diffusion: 0.0,
    };
    let lights = [
        light(LightingKind::Point, [400.0, -200.0, -1500.0], [1.0; 3]),
        light(LightingKind::Ambient, [0.0; 3], [0.1; 3]),
    ];
    let view = lumit_gpu::mesh::ModelView {
        comp: SIZE,
        render_scale: 1.0,
        samples: 8,
        camera: lumit_gpu::composite::camera_matrix(
            w,
            h,
            zoom,
            (eye[0], eye[1], eye[2]),
            (0.0, 0.0, 0.0),
        ),
        model_to_comp: glam::Mat4::from_translation(glam::Vec3::new(w / 2.0, h / 2.0, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(h / 2.0))
            * glam::Mat4::from_rotation_x(std::f32::consts::PI),
        shutter: &[],
        shutter_scenes: &[],
        lights: &lights,
        // A new layer's Material options.
        material: LightingMaterial {
            eye,
            specular: 0.5,
            ..LightingMaterial::default()
        },
        instances: &[],
        casters: None,
        maps: None,
        display: Default::default(),
        environment: None,
        looks: &[],
        behind: None,
    };
    let time = |run: &dyn Fn() -> Result<(), String>| -> Result<f64, String> {
        for _ in 0..WARMUP {
            run()?;
        }
        ctx.flush();
        ctx.settle();
        let mut best = f64::INFINITY;
        for _ in 0..REPEATS {
            let started = Instant::now();
            for _ in 0..RUNS {
                run()?;
            }
            ctx.flush();
            ctx.settle();
            best = best.min(elapsed_ms(started) / f64::from(RUNS));
        }
        Ok(best)
    };
    let b30 = time(&|| {
        fx.model(&ctx, &scene, &view)
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    let casting = [
        LightingLight {
            shadow_darkness: 1.0,
            shadow_diffusion: 20.0,
            ..lights[0]
        },
        LightingLight {
            kind: LightingKind::Parallel,
            axis: [0.3, 0.5, 0.8],
            shadow_darkness: 1.0,
            shadow_diffusion: 20.0,
            ..lights[0]
        },
    ];
    let caster = lumit_gpu::mesh::shadow::MapCaster {
        scene: &scene,
        model_to_comp: view.model_to_comp,
    };
    let b33 = time(&|| {
        let maps = fx.shadow_maps(&ctx, std::slice::from_ref(&caster), &casting, 2048);
        let view = lumit_gpu::mesh::ModelView {
            lights: &casting,
            maps: maps.as_ref(),
            environment: None,
            looks: &[],
            behind: None,
            material: LightingMaterial {
                accepts_shadows: lumit_gpu::fx::lighting::LightingShadows::On,
                ..view.material
            },
            ..view
        };
        fx.model(&ctx, &scene, &view)
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    // What the cubes are made from doesn't change how long they take, so the
    // model's own picture stands in for a 2K panorama: it makes cubes as big.
    let picture = fx
        .model(&ctx, &scene, &view)
        .map_err(|e| e.to_string())?
        .colour;
    let b34 = time(&|| {
        drop(fx.environment(&ctx, None, &picture));
        Ok(())
    })?;
    let surround = lumit_gpu::fx::environment::EnvironmentLight {
        maps: fx.environment(&ctx, None, &picture),
        colour: [1.0; 3],
        turn: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    let ray = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let b35 = time(&|| {
        drop(fx.environment_background(&ctx, &surround, (ray, zoom), SIZE, 1.0, 0.0));
        let view = lumit_gpu::mesh::ModelView {
            environment: Some(&surround),
            looks: &[],
            behind: None,
            ..view
        };
        fx.model(&ctx, &scene, &view)
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    let mut out = Vec::new();
    for (budget, value_ms) in [("B30", b30), ("B33", b33), ("B34", b34), ("B35", b35)] {
        let m = Measurement {
            budget,
            value_ms,
            frames: u64::from(RUNS),
        };
        progress(m);
        out.push(m);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    #[ignore = "a timing run: cargo test --release -p lumit-bench -- --ignored models"]
    fn model_budget() {
        super::budgets(&mut |m| eprintln!("{m:?}")).unwrap();
    }
}
