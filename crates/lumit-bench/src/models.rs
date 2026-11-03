//! **B30 — a Model layer of a million triangles** (docs/13 §2).
//!
//! # In plain terms
//!
//! One model drawn through a camera with two lights at 1080p, the comp's
//! default anti-aliasing, the way the mesh pass draws a Model layer each
//! frame. The model is uploaded once before the stopwatch starts, as the
//! engine keeps it between frames, so the number is the draw alone.

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

/// Measure B30. `Err` when there is no graphics adapter.
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
    };
    let run = || fx.model(&ctx, &scene, &view).map_err(|e| e.to_string());
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
    let m = Measurement {
        budget: "B30",
        value_ms: best,
        frames: u64::from(RUNS),
    };
    progress(m);
    Ok(vec![m])
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
