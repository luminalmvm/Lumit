//! A model and a 3D layer that pass through each other cut where they meet,
//! and the model takes the comp's light.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use glam::{Mat4, Vec3};
use lumit_gpu::composite::{camera_matrix, order::cut_by_depth, Blend, CompositeLayer};
use lumit_gpu::fx::lighting::{LightingKind, LightingLight, LightingMaterial};
use lumit_gpu::mesh::ModelView;
use lumit_scene::{fixtures, Material};
use std::sync::Arc;

const W: u32 = 320;
const H: u32 = 180;

#[test]
fn a_lit_model_and_a_crossing_layer_cut_where_they_meet() {
    let Some(lease) = lumit_gpu::test_support::lease() else {
        return lumit_gpu::no_adapter();
    };
    let red = Material {
        base_colour: [0.8, 0.05, 0.05, 1.0],
        metallic: 0.0,
        roughness: 0.6,
        ..Material::default()
    };
    let scene = Arc::new(fixtures::scene(
        vec![(fixtures::cube(1.0, Some(0)), [0.0; 3])],
        vec![red],
        Vec::new(),
    ));
    let pose = lumit_core::camera::default_pose(f64::from(W), f64::from(H));
    let eye = [pose.position.0, pose.position.1, pose.position.2].map(|v| v as f32);
    let camera = camera_matrix(
        W as f32,
        H as f32,
        pose.zoom as f32,
        (eye[0], eye[1], eye[2]),
        (0.0, 0.0, 0.0),
    );
    // A metre is 60 pixels, the cube in the middle of the comp.
    let model_to_comp = Mat4::from_translation(Vec3::new(160.0, 90.0, 0.0))
        * Mat4::from_scale(Vec3::splat(60.0))
        * Mat4::from_rotation_x(std::f32::consts::PI);
    let key = LightingLight {
        corners: [[40.0, 10.0, -300.0]; 4],
        colour: [1.0; 3],
        kind: LightingKind::Point,
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
    let draw = |lights: &[LightingLight]| {
        lease
            .fx()
            .model(
                &lease,
                &scene,
                &ModelView {
                    comp: (W, H),
                    render_scale: 1.0,
                    samples: 1,
                    camera,
                    model_to_comp,
                    shutter: &[],
                    lights,
                    material: LightingMaterial {
                        eye,
                        ..LightingMaterial::default()
                    },
                    instances: &[],
                },
            )
            .expect("the model draws")
    };
    // A blue card through the cube's middle, turned so its left side comes
    // out in front of the cube and its right side goes in behind.
    let blue =
        lumit_gpu::fx::common::upload_linear_f32(&lease, &[0.0, 0.0, 1.0, 1.0].repeat(16), 4, 4);
    let card = CompositeLayer {
        texture: &blue,
        size: (200.0, 120.0),
        position: (160.0, 90.0),
        anchor: (100.0, 60.0),
        scale: (100.0, 100.0),
        rotation_deg: 0.0,
        opacity: 100.0,
        matte: None,
        blend: Blend::Normal,
        z: 0.0,
        rotation_x_deg: 0.0,
        rotation_y_deg: -60.0,
        three_d: true,
        layer_mask: None,
        pre: None,
    };
    let frame = |lights: &[LightingLight]| {
        let model = draw(lights);
        let piece = CompositeLayer {
            texture: &model.colour,
            size: (W as f32, H as f32),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            rotation_y_deg: 0.0,
            ..card.clone()
        };
        let middle =
            |m: Mat4| (camera * m.transform_point3(Vec3::new(0.5, 0.5, 0.0)).extend(1.0)).w;
        let depths = [
            middle(card.world()),
            middle(Mat4::from_translation(Vec3::new(160.0, 90.0, 0.0))),
        ];
        let (order, cuts): (Vec<usize>, Vec<_>) =
            cut_by_depth(&depths, &[false; 2]).into_iter().unzip();
        let layers = [card.clone(), piece];
        let layers: Vec<_> = order.iter().map(|&i| layers[i].clone()).collect();
        let out = lease.compositor().composite_joined(
            &lease,
            W,
            H,
            [0.0; 4],
            &layers,
            Some(camera),
            None,
            1.0,
            1,
            None,
            &cuts,
            std::slice::from_ref(&model),
        );
        lease.colour().readback_linear_f32(&lease, &out).unwrap()
    };
    let at = |px: &[f32], x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    };
    let lit = frame(&[key]);
    // Left of the cube's middle the card is in front, right of it the cube.
    let card_in_front = at(&lit, 160 - 24, 90);
    let cube_in_front = at(&lit, 160 + 15, 90);
    assert!(
        card_in_front[2] > 0.9 && card_in_front[0] < 0.05,
        "{card_in_front:?}"
    );
    assert!(
        cube_in_front[0] > 0.5 && cube_in_front[2] < 0.2,
        "{cube_in_front:?}"
    );
    // Past the cube the card shows behind nothing, and off both is empty.
    assert!(at(&lit, 160 + 40, 90)[2] > 0.9);
    assert_eq!(at(&lit, 5, 5), [0.0; 4]);
    // The face is lit by the light in front of it, and dark with the light
    // behind it.
    let behind = LightingLight {
        corners: [[40.0, 10.0, 300.0]; 4],
        ..key
    };
    assert!(at(&frame(&[behind]), 175, 90)[0] < 0.01);
}
