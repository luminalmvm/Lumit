//! A model and a 3D layer that pass through each other cut where they meet,
//! and the model takes the comp's light.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use glam::{Mat4, Vec3};
use lumit_gpu::composite::{camera_matrix, order::cut_by_depth, Blend, CompositeLayer};
use lumit_gpu::fx::lighting::{LightingKind, LightingLight, LightingMaterial, LightingShadows};
use lumit_gpu::mesh::display::{ModelDisplay, Shading};
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
                    shutter_scenes: &[],
                    lights,
                    material: LightingMaterial {
                        eye,
                        ..LightingMaterial::default()
                    },
                    instances: &[],
                    casters: None,
                    maps: None,
                    display: Default::default(),
                    environment: None,
                    looks: &[],
                    behind: None,
                    poses: &[],
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

#[test]
fn a_model_shades_a_layer_behind_it_and_not_its_own_lit_face() {
    let Some(lease) = lumit_gpu::test_support::lease() else {
        return lumit_gpu::no_adapter();
    };
    let scene = Arc::new(fixtures::scene(
        vec![(fixtures::cube(1.0, Some(0)), [0.0; 3])],
        vec![Material::default()],
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
    // Turned a quarter about the light, so the map's square is wider than
    // the diamond its shadow is.
    let model_to_comp = Mat4::from_translation(Vec3::new(160.0, 90.0, 0.0))
        * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4)
        * Mat4::from_scale(Vec3::splat(60.0))
        * Mat4::from_rotation_x(std::f32::consts::PI);
    // From in front, straight along the camera, casting a full shadow.
    let key = LightingLight {
        corners: [[160.0, 90.0, -600.0]; 4],
        colour: [1.0; 3],
        kind: LightingKind::Parallel,
        falloff: 0,
        radius_px: 0.0,
        falloff_px: 0.0,
        legacy: false,
        cone_cos: -2.0,
        cone_inner: -2.0,
        axis: [0.0, 0.0, 1.0],
        shadow_darkness: 1.0,
        shadow_diffusion: 0.0,
    };
    let fx = lease.fx();
    let caster = lumit_gpu::mesh::shadow::MapCaster {
        scene: &scene,
        model_to_comp,
        pose: None,
    };
    let maps = fx.shadow_maps(&lease, &[caster], &[key], 512);
    assert!(maps.is_some(), "a model and a light that casts make maps");
    // The face looking at the light is as lit with its own shadow as without.
    let front = |maps, accepts_shadows, shading| {
        let material = LightingMaterial {
            eye,
            accepts_shadows,
            ..LightingMaterial::default()
        };
        let view = ModelView {
            comp: (W, H),
            render_scale: 1.0,
            samples: 1,
            camera,
            model_to_comp,
            shutter: &[],
            shutter_scenes: &[],
            lights: &[key],
            material,
            instances: &[],
            casters: None,
            maps,
            display: ModelDisplay {
                shading,
                faces: false,
            },
            environment: None,
            looks: &[],
            behind: None,
            poses: &[],
        };
        let picture = fx.model(&lease, &scene, &view).expect("the model draws");
        let px = lease
            .colour()
            .readback_linear_f32(&lease, &picture.colour)
            .unwrap();
        let i = ((90 * W + 160) * 4) as usize;
        px[i]
    };
    let on = LightingShadows::On;
    let (alone, shaded) = (
        front(None, on, Shading::Final),
        front(maps.as_ref(), on, Shading::Final),
    );
    assert!(
        alone > 0.1 && (alone - shaded).abs() < 1e-3,
        "{alone} {shaded}"
    );
    // Showing only its shadows it is empty here, unless a view shows it whole.
    let only = |shading| front(maps.as_ref(), LightingShadows::Only, shading);
    assert!(only(Shading::Final) < 1e-3 && only(Shading::Shaded) > 0.1);
    // A white card behind it and past the map's far depth, facing the light:
    // dark behind the cube only, lit in the square past the diamond, hard and
    // soft, out to a texel or two past its edge.
    let n = 32;
    let white = lumit_gpu::fx::common::upload_linear_f32(&lease, &[1.0; 4].repeat(n * n), 32, 32);
    for diffusion in [0.0, 40.0] {
        let key = LightingLight {
            shadow_diffusion: diffusion,
            ..key
        };
        let caster = lumit_gpu::mesh::shadow::MapCaster {
            scene: &scene,
            model_to_comp,
            pose: None,
        };
        let maps = fx.shadow_maps(&lease, &[caster], &[key], 512);
        let op = lumit_gpu::fx::lighting::LightingOp {
            origin: [0.0, 0.0, 200.0],
            du: [W as f32 / 32.0, 0.0, 0.0],
            dv: [0.0, H as f32 / 32.0, 0.0],
            normal: [0.0, 0.0, -1.0],
            material: LightingMaterial {
                accepts_shadows: LightingShadows::On,
                ..LightingMaterial::default()
            },
            lights: vec![key],
            own_caster: None,
        };
        let lit = fx.lighting(&lease, &white, 32, 32, &op, None, maps.as_ref(), None);
        let px = lumit_gpu::fx::common::readback_linear_f32(&lease, &lit, 32, 32).unwrap();
        let at = |x: usize, y: usize| px[(y * 32 + x) * 4];
        assert!(
            at(16, 16) < 1.05,
            "{diffusion}, behind the cube: {}",
            at(16, 16)
        );
        for (x, y) in [(2, 2), (19, 22), (15, 25)] {
            assert!(
                at(x, y) > 1.9,
                "{diffusion}, clear at {x} {y}: {}",
                at(x, y)
            );
        }
    }
}

/// Under a white environment and nothing else, a white matte model and a
/// white card both read white where they face the camera.
#[test]
fn a_white_environment_lights_white_things_white() {
    let Some(lease) = lumit_gpu::test_support::lease() else {
        return lumit_gpu::no_adapter();
    };
    let fx = lease.fx();
    let sky = lumit_gpu::fx::common::upload_linear_f32(&lease, &[1.0; 4].repeat(64 * 32), 64, 32);
    let surround = lumit_gpu::fx::environment::EnvironmentLight {
        maps: fx.environment(&lease, None, &sky),
        colour: [1.0; 3],
        turn: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    let matte = Material {
        base_colour: [1.0; 4],
        metallic: 0.0,
        roughness: 1.0,
        ..Material::default()
    };
    let scene = Arc::new(fixtures::scene(
        vec![(fixtures::sphere(1.0, 64, 32, Some(0)), [0.0; 3])],
        vec![matte],
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
    let view = ModelView {
        comp: (W, H),
        render_scale: 1.0,
        samples: 1,
        camera,
        model_to_comp: Mat4::from_translation(Vec3::new(160.0, 90.0, 0.0))
            * Mat4::from_scale(Vec3::splat(60.0))
            * Mat4::from_rotation_x(std::f32::consts::PI),
        shutter: &[],
        shutter_scenes: &[],
        lights: &[],
        material: LightingMaterial {
            eye,
            ..LightingMaterial::default()
        },
        instances: &[],
        casters: None,
        maps: None,
        display: Default::default(),
        environment: Some(&surround),
        looks: &[],
        behind: None,
        poses: &[],
    };
    let model = fx.model(&lease, &scene, &view).expect("the model draws");
    let px = lease
        .colour()
        .readback_linear_f32(&lease, &model.colour)
        .unwrap();
    let ball = &px[((90 * W + 160) * 4) as usize..][..3];
    let white = lumit_gpu::fx::common::upload_linear_f32(&lease, &[1.0; 4].repeat(16), 4, 4);
    let op = lumit_gpu::fx::lighting::LightingOp {
        origin: [0.0, 0.0, 0.0],
        du: [80.0, 0.0, 0.0],
        dv: [0.0, 45.0, 0.0],
        normal: [0.0, 0.0, -1.0],
        material: LightingMaterial {
            eye,
            ..LightingMaterial::default()
        },
        lights: Vec::new(),
        own_caster: None,
    };
    let lit = fx.lighting(&lease, &white, 4, 4, &op, None, None, Some(&surround));
    let px = lumit_gpu::fx::common::readback_linear_f32(&lease, &lit, 4, 4).unwrap();
    let card = &px[(2 * 4 + 2) * 4..][..3];
    for v in ball.iter().chain(card) {
        assert!((v - 1.0).abs() < 0.02, "ball {ball:?}, card {card:?}");
    }
}

/// A rig on bones draws where its pose has it, a moment drawn twice comes out
/// the same however it was reached, and the shadow it casts follows the pose.
#[test]
fn a_posed_model_draws_and_casts_as_its_pose_has_it() {
    use lumit_scene::pose::pose;
    let Some(lease) = lumit_gpu::test_support::lease() else {
        return lumit_gpu::no_adapter();
    };
    let rig = Arc::new(fixtures::bending(48, 48, 8));
    let view_pose = lumit_core::camera::default_pose(f64::from(W), f64::from(H));
    let eye = [
        view_pose.position.0,
        view_pose.position.1,
        view_pose.position.2,
    ]
    .map(|v| v as f32);
    let camera = camera_matrix(
        W as f32,
        H as f32,
        view_pose.zoom as f32,
        (eye[0], eye[1], eye[2]),
        (0.0, 0.0, 0.0),
    );
    let model_to_comp = Mat4::from_translation(Vec3::new(160.0, 90.0, 0.0))
        * Mat4::from_scale(Vec3::splat(30.0))
        * Mat4::from_rotation_x(std::f32::consts::PI);
    let take = rig.clips.first().expect("the take");
    let at = |t: f64| pose(&rig, Some((take, t)), &[]);
    let (start, bent) = (at(0.0), at(1.0));
    let draw = |poses: &[lumit_scene::pose::Pose]| {
        // A pose for each shutter sample when there are several.
        let shutter = vec![model_to_comp; if poses.len() > 1 { poses.len() } else { 0 }];
        let picture = lease
            .fx()
            .model(
                &lease,
                &rig,
                &ModelView {
                    comp: (W, H),
                    render_scale: 1.0,
                    samples: 1,
                    camera,
                    model_to_comp,
                    shutter: &shutter,
                    shutter_scenes: &[],
                    lights: &[],
                    material: LightingMaterial {
                        eye,
                        ..LightingMaterial::default()
                    },
                    instances: &[],
                    casters: None,
                    maps: None,
                    display: Default::default(),
                    environment: None,
                    looks: &[],
                    behind: None,
                    poses,
                },
            )
            .expect("the model draws");
        lease
            .colour()
            .readback_linear_f32(&lease, &picture.colour)
            .unwrap()
    };
    // Where the most of it sits along x, by its alpha.
    let middle = |px: &[f32]| {
        let (mut sum, mut total) = (0.0, 0.0);
        for (i, p) in px.chunks_exact(4).enumerate() {
            sum += (i as u32 % W) as f32 * p[3];
            total += p[3];
        }
        sum / total
    };
    let (rest, posed) = (
        draw(std::slice::from_ref(&start)),
        draw(std::slice::from_ref(&bent)),
    );
    assert!(
        (middle(&rest) - middle(&posed)).abs() > 2.0,
        "bent over: {} against {}",
        middle(&rest),
        middle(&posed)
    );
    let swells = |px: &[f32]| px.chunks_exact(4).filter(|p| p[3] > 0.5).count();
    assert!(swells(&posed) > swells(&rest), "its morph target swells it");
    assert_eq!(
        posed,
        draw(std::slice::from_ref(&bent)),
        "the same moment again"
    );
    // Across the shutter the picture smears from one pose to the other, where
    // a blur from the rest pose alone would be sharp.
    let smear = draw(&[start.clone(), bent.clone()]);
    let half = |px: &[f32]| {
        px.chunks_exact(4)
            .filter(|p| (0.3..0.7).contains(&p[3]))
            .count()
    };
    assert!(
        half(&smear) > half(&posed) + 100,
        "half covered: {} against {}",
        half(&smear),
        half(&posed)
    );

    // A box of the rig at each pose for the shadow maps to fit, which a rest
    // pose's box would clip.
    assert!(bent.bounds.max[0] - bent.bounds.min[0] > start.bounds.max[0] - start.bounds.min[0]);
    let light = LightingLight {
        corners: [[160.0, -400.0, -50.0]; 4],
        colour: [1.0; 3],
        kind: LightingKind::Point,
        falloff: 0,
        radius_px: 0.0,
        falloff_px: 0.0,
        legacy: false,
        cone_cos: -2.0,
        cone_inner: -2.0,
        axis: [0.0, 0.0, 1.0],
        shadow_darkness: 1.0,
        shadow_diffusion: 0.0,
    };
    let lights = [light];
    let floor = Arc::new(fixtures::scene(
        vec![(fixtures::cube(1.0, None), [0.0; 3])],
        Vec::new(),
        Vec::new(),
    ));
    let floor_place = Mat4::from_translation(Vec3::new(160.0, 170.0, 0.0))
        * Mat4::from_scale(Vec3::new(400.0, 10.0, 400.0))
        * Mat4::from_rotation_x(std::f32::consts::PI);
    let shadowed = |p: Option<&lumit_scene::pose::Pose>| {
        let caster = lumit_gpu::mesh::shadow::MapCaster {
            scene: &rig,
            model_to_comp,
            pose: p,
        };
        let maps = lease
            .fx()
            .shadow_maps(&lease, std::slice::from_ref(&caster), &lights, 512)
            .expect("the light casts");
        let picture = lease
            .fx()
            .model(
                &lease,
                &floor,
                &ModelView {
                    comp: (W, H),
                    render_scale: 1.0,
                    samples: 1,
                    camera,
                    model_to_comp: floor_place,
                    shutter: &[],
                    shutter_scenes: &[],
                    lights: &lights,
                    material: LightingMaterial {
                        eye,
                        accepts_shadows: LightingShadows::On,
                        ..LightingMaterial::default()
                    },
                    instances: &[],
                    casters: None,
                    maps: Some(&maps),
                    display: Default::default(),
                    environment: None,
                    looks: &[],
                    behind: None,
                    poses: &[],
                },
            )
            .expect("the floor draws");
        lease
            .colour()
            .readback_linear_f32(&lease, &picture.colour)
            .unwrap()
    };
    assert_ne!(
        shadowed(Some(&start)),
        shadowed(Some(&bent)),
        "the shadow moves with the pose"
    );
}

/// Models are kept while their scenes live, and however many that is the
/// card keeps room to bind them. There are more materials here than it has
/// sampler slots, or binding slots for their pictures.
#[test]
fn any_number_of_kept_models_still_draw() {
    let Some(lease) = lumit_gpu::test_support::lease() else {
        return lumit_gpu::no_adapter();
    };
    let pose = lumit_core::camera::default_pose(f64::from(W), f64::from(H));
    let eye = [pose.position.0, pose.position.1, pose.position.2].map(|v| v as f32);
    let camera = camera_matrix(
        W as f32,
        H as f32,
        pose.zoom as f32,
        (eye[0], eye[1], eye[2]),
        (0.0, 0.0, 0.0),
    );
    let model_to_comp = Mat4::from_translation(Vec3::new(160.0, 90.0, 0.0))
        * Mat4::from_scale(Vec3::splat(60.0))
        * Mat4::from_rotation_x(std::f32::consts::PI);
    let material = Material {
        base_colour: [0.8, 0.5, 0.2, 1.0],
        metallic: 0.0,
        roughness: 0.5,
        ..Material::default()
    };
    let scenes: Vec<_> = (0..80)
        .map(|_| {
            Arc::new(fixtures::scene(
                vec![(fixtures::cube(1.0, Some(0)), [0.0; 3])],
                vec![material.clone(); 2000],
                Vec::new(),
            ))
        })
        .collect();
    let mut last = None;
    for scene in &scenes {
        let view = ModelView {
            comp: (W, H),
            render_scale: 1.0,
            samples: 1,
            camera,
            model_to_comp,
            shutter: &[],
            shutter_scenes: &[],
            lights: &[],
            material: LightingMaterial {
                eye,
                ..LightingMaterial::default()
            },
            instances: &[],
            casters: None,
            maps: None,
            display: Default::default(),
            environment: None,
            looks: &[],
            behind: None,
            poses: &[],
        };
        last = Some(lease.fx().model(&lease, scene, &view).expect("it draws"));
    }
    let px = lease
        .colour()
        .readback_linear_f32(&lease, &last.expect("drawn").colour)
        .unwrap();
    let middle = &px[((90 * W + 160) * 4) as usize..][..4];
    assert!(middle[3] > 0.9, "the last model is gone: {middle:?}");
}
