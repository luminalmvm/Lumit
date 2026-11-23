use std::sync::Arc;

use super::*;
use crate::anim::Property;
use crate::expression::ExpressionContext;
use crate::fx::drivers::ResolvedDrivers;
use crate::fx::effects::hologram::{Hologram, HologramDef};
use crate::fx::effects::holographic_foil::{lights, HolographicFoilDef};
use crate::fx::markers::MarkerContext;
use crate::fx::params::{Params, Value};
use crate::fx::registry::{EffectDef, EffectMetadata};
use crate::model::{CameraOptions, EffectInstance, EffectValue, LayerKind};
use uuid::Uuid;

const W: u32 = 320;
const H: u32 = 200;

fn foil() -> HolographicFoil {
    <HolographicFoil as EffectMetadata>::read(Params::EMPTY)
}

fn hologram() -> Hologram {
    <Hologram as EffectMetadata>::read(Params::EMPTY)
}

fn packed(f: HolographicFoil) -> Blocks {
    f.packed(Params::EMPTY, W, H).blocks
}

/// Every number is finite, every light is where a rig can put it, and every
/// spectrum's terms fit the room the uniform has for them, none dropped.
fn assert_sane(b: &Blocks, what: &str) {
    assert!(
        b.iter().flatten().all(|x| x.is_finite()),
        "{what}: not finite"
    );
    for slot in 0..4 {
        let [x, y, z, radiance] = b[LIGHTS + 4 * slot];
        let colour = b[LIGHTS + 4 * slot + 1];
        let far = (x * x + y * y + z * z).sqrt();
        assert!((0.0..=5000.0).contains(&radiance), "{what}: light {slot}");
        assert!(
            radiance == 0.0 || (0.05..=3.0).contains(&far),
            "{what}: light {slot}"
        );
        assert!(
            colour[..3].iter().all(|c| (0.0..2.0).contains(c)),
            "{what}: light {slot}"
        );
        assert!((colour[3] as usize) < SPECTRA, "{what}: light {slot}");
    }
    let mut lists = Vec::new();
    for s in 0..SPECTRA {
        let [first, count, ..] = b[SPECTRA_AT + 4 * s];
        assert!(
            (first + count) as usize <= TERMS_MAX,
            "{what}: spectrum {s}"
        );
        lists.push(count as usize);
    }
    assert_eq!(lists, [14, 14, 14, 28, 21], "{what}: term lists cut short");
}

/// Packs every Material under every Lighting, Tint and Quality, and the
/// surface and print choices with their rows in play. A NaN or a light out
/// of reach here is a broken picture on the GPU, where no test can see it.
#[test]
fn every_choice_packs_rows_the_kernel_can_read() {
    for material in 0..5 {
        for lighting in 0..10 {
            let f = HolographicFoil {
                material,
                lighting,
                ..foil()
            };
            assert_sane(
                &packed(f),
                &format!("material {material} lighting {lighting}"),
            );
        }
        for tint in 0..5 {
            let f = HolographicFoil {
                material,
                tint,
                ..foil()
            };
            assert_sane(&packed(f), &format!("material {material} tint {tint}"));
        }
        for quality in 0..3 {
            let f = HolographicFoil {
                material,
                quality,
                ..foil()
            };
            let b = packed(f);
            assert_sane(&b, &format!("material {material} quality {quality}"));
            assert_eq!(b[QUALITY][0], if quality == 0 { 2.0 } else { 3.0 });
        }
    }
    for relief in 0..9u32 {
        let f = HolographicFoil {
            relief_source: relief,
            print: relief % 3,
            pattern: relief % 4,
            lines: relief % 3,
            waviness: 0.5,
            orange_peel: 1.0,
            wear: 50.0,
            auto_tilt: 10.0,
            rig_rotation: 30.0,
            ..foil()
        };
        let b = f
            .packed(
                Params::new(&[(HolographicFoil::DERIVED_TIME, Value::Float(1.3))]),
                W,
                H,
            )
            .blocks;
        assert_sane(&b, &format!("relief {relief}"));
        // The waves row as `surface.rs` lays it out: 8 on the board, 16 on the
        // varnish and the embossed lines, all within its 32.
        assert_eq!(
            b[194][..3],
            [8.0, 16.0, [0.0, 1.0, 2.0][relief as usize % 3]]
        );
        // The bevel's row is written only for the bevel.
        assert_eq!(b[279][0] > 0.0, relief == surface::BEVEL_SOURCE);
    }

    // Glitter lit by two spectra fills both band tables, and the second ends
    // on the uniform's last row.
    let f = HolographicFoil {
        material: GLITTER,
        lighting: lights::CUSTOM,
        spot_b_spectrum: LED_SPECTRUM as u32,
        ..foil()
    };
    let b = packed(f);
    assert_sane(&b, "glitter, two tables");
    // The tables' map as `table.rs` lays it out: equal and LED in the two
    // slots, the brightest light's first.
    let [equal, daylight, tungsten, led] = b[288];
    assert_eq!([daylight, tungsten, b[289][0]], [-1.0; 3]);
    assert_eq!([equal.min(led), equal.max(led)], [0.0, 1.0]);
    assert!(b[BLOCKS - 1][..3].iter().any(|c| *c != 0.0));
}

fn same_bytes(a: &[[f32; 4]], b: &[[f32; 4]]) -> bool {
    a.iter()
        .flatten()
        .map(|x| x.to_bits())
        .eq(b.iter().flatten().map(|x| x.to_bits()))
}

/// Auto picks the rig and the dye the Material is documented to take, and
/// Custom reads the light rows, which every other rig ignores.
#[test]
fn auto_and_custom_lighting_and_tint_pick_what_they_say() {
    let with = |material, lighting, tint| {
        packed(HolographicFoil {
            material,
            lighting,
            tint,
            ..foil()
        })
    };
    const STUDIO_RIG: u32 = 0;
    const AUTO: u32 = 9;
    const CLEAR: u32 = 0;
    const AUTO_DYE: u32 = 4;
    // Prism's Auto is Studio, byte for byte, and so are Custom's defaults,
    // on Holographic foil and on Hologram.
    let studio = with(0, STUDIO_RIG, CLEAR);
    assert!(same_bytes(&with(0, AUTO, CLEAR), &studio));
    assert!(same_bytes(&with(0, lights::CUSTOM, CLEAR), &studio));
    let film = |lighting| {
        Hologram {
            lighting,
            ..hologram()
        }
        .packed(Params::EMPTY, W, H)
        .blocks
    };
    assert!(same_bytes(&film(lights::CUSTOM), &film(STUDIO_RIG)));
    // Vortex keeps Studio's spots under its own panel; the rest have rigs of
    // their own.
    let lit = LIGHTS..LIGHTS + 16;
    for m in [SHATTER, GLITTER, SPARKLE, VORTEX] {
        let (auto, own) = (with(m, AUTO, CLEAR), with(m, STUDIO_RIG, CLEAR));
        assert!(
            !same_bytes(&auto[lit.clone()], &own[lit.clone()]),
            "material {m}"
        );
    }
    let (auto, own) = (with(VORTEX, AUTO, CLEAR), with(VORTEX, STUDIO_RIG, CLEAR));
    assert!(same_bytes(
        &auto[LIGHTS..LIGHTS + 12],
        &own[LIGHTS..LIGHTS + 12]
    ));

    // Auto's dye: clear on Prism, the Gold and Blue choices on Shatter and
    // Glitter, its own on Sparkle and Vortex.
    for (m, dye) in [(0, CLEAR), (SHATTER, 1), (GLITTER, 2)] {
        assert!(
            same_bytes(&with(m, AUTO, AUTO_DYE), &with(m, AUTO, dye)),
            "material {m}"
        );
    }
    assert_eq!(with(SPARKLE, AUTO, AUTO_DYE)[TINT], [2.0, 600.0, 90.0, 0.5]);
    assert_eq!(with(VORTEX, AUTO, AUTO_DYE)[TINT], [1.0, 500.0, 50.0, 0.3]);

    // Custom reads the rows; Studio does not.
    let moved = HolographicFoil {
        spot_a_x: 0.3,
        spot_a_y: -0.2,
        spot_a_distance: 2.0,
        spot_a_intensity: 1234.0,
        spot_b_on: false,
        panel_spectrum: FLUORESCENT as u32,
        rims: 2.0,
        ..foil()
    };
    let custom = packed(HolographicFoil {
        lighting: lights::CUSTOM,
        ..moved
    });
    let dir = pad_to_dir(0.3, -0.2, 0.0).map(|v| (2.0 * v) as f32);
    assert_eq!(custom[LIGHTS], [dir[0], dir[1], dir[2], 1234.0]);
    assert_eq!(custom[LIGHTS + 4][3], 0.0);
    assert_eq!(custom[LIGHTS + 13][3], FLUORESCENT as f32);
    assert_eq!(custom[STUDIO_LEVELS][0], 2.0);
    let ignored = packed(HolographicFoil {
        lighting: STUDIO_RIG,
        material: 0,
        tint: CLEAR,
        ..moved
    });
    assert!(same_bytes(&ignored, &studio));
}

/// Hologram packs through the foil's code for every Type, Lighting,
/// Backdrop and Recording, with its own rows where the kernel reads them.
#[test]
fn every_hologram_choice_packs_rows_the_kernel_can_read() {
    for lighting in 0..9 {
        let h = Hologram {
            lighting,
            ..hologram()
        };
        assert_sane(
            &h.packed(Params::EMPTY, W, H).blocks,
            &format!("lighting {lighting}"),
        );
    }
    for kind in 0..2u32 {
        for recording in 0..2u32 {
            for backdrop in 0..9u32 {
                let h = Hologram {
                    hologram_type: kind,
                    recording,
                    backdrop,
                    reference_x: 0.9,
                    reference_y: 0.9,
                    ..hologram()
                };
                let b = h.packed(Params::EMPTY, W, H).blocks;
                let what = format!("type {kind} recording {recording} backdrop {backdrop}");
                assert_sane(&b, &what);
                let [_, subject, rec, back] = b[280];
                assert_eq!(
                    [b[280][0], rec, back],
                    [kind, recording, backdrop].map(|v| v as f32),
                    "{what}"
                );
                assert!(subject < 3.0, "{what}");
                // The reference is a direction, held inside the rim.
                let [x, y, z, _] = b[281];
                assert!(
                    ((x * x + y * y + z * z).sqrt() - 1.0).abs() < 1e-5,
                    "{what}"
                );
                assert!(z > 0.0, "{what}");
            }
        }
    }
}

fn set(inst: &mut EffectInstance, id: &str, value: EffectValue) {
    let row = inst.params.iter_mut().find(|p| p.id == id);
    assert!(row.is_some(), "no row {id}");
    if let Some(p) = row {
        p.value = value;
    }
}

fn float(v: f64) -> EffectValue {
    EffectValue::Float(Property::fixed(v))
}

fn effect(name: &str, follow: bool, motion_blur: f64, tilt_y: f64) -> EffectInstance {
    let mut inst = crate::fx::builtins::instantiate(name).unwrap_or_else(|| unreachable!());
    set(&mut inst, "seed", EffectValue::Seed(7));
    set(
        &mut inst,
        "follow_rotation",
        EffectValue::Choice(u32::from(follow)),
    );
    set(&mut inst, "motion_blur", float(motion_blur));
    set(&mut inst, "tilt_x", float(0.0));
    set(&mut inst, "tilt_y", float(tilt_y));
    inst
}

/// A comp holding the card the effect sits on, turned `card` degrees about
/// y, maybe under a 3D null turned `parent`, and maybe a camera turned
/// `camera`. Returns the context the card's effects resolve in.
fn staged(
    three_d: bool,
    card: f64,
    parent: Option<f64>,
    camera: Option<f64>,
) -> Arc<ExpressionContext> {
    let (mut doc, comp_id) = crate::fx::tests::lit_document(1);
    let comp = doc.comp_mut(comp_id).unwrap_or_else(|| unreachable!());
    let mut layer = comp.layers[0].clone();
    layer.switches.three_d = three_d;
    // A 2D layer turns only in its own plane.
    if three_d {
        layer.transform.rotation_y = Property::fixed(card);
    } else {
        layer.transform.rotation = Property::fixed(card);
    }
    let mut layers = Vec::new();
    if let Some(turn) = camera {
        let zoom = crate::camera::default_zoom(1920.0);
        let mut cam = layer.clone();
        cam.id = Uuid::now_v7();
        cam.kind = LayerKind::Camera {
            zoom: Property::fixed(zoom),
            solve_link: None,
            correction_base: None,
            options: Box::new(CameraOptions::fresh(zoom, (960.0, 540.0, 0.0))),
        };
        cam.transform.rotation_y = Property::fixed(turn);
        layers.push(cam);
    }
    if let Some(turn) = parent {
        let mut null = layer.clone();
        null.id = Uuid::now_v7();
        null.switches.three_d = true;
        null.transform.rotation_y = Property::fixed(turn);
        layer.parent = Some(null.id);
        layers.push(null);
    }
    let card_id = layer.id;
    layers.push(layer);
    comp.layers = layers;
    Arc::new(ExpressionContext {
        document: Arc::new(doc),
        comp: Some(comp_id),
        layer: Some(card_id),
        comp_time: 0.5,
        ..ExpressionContext::detached()
    })
}

/// Resolves `inst` at layer time 0.5 in `cx`, as a render does, and packs it.
fn resolved(inst: &EffectInstance, cx: Arc<ExpressionContext>) -> (Blocks, f32) {
    let stack = crate::fx::resolved::resolve_instance(
        inst,
        ResolvedDrivers::NONE,
        0.5,
        2202.9,
        1.0,
        &MarkerContext::NONE,
        cx,
    );
    let op = stack.get(0).unwrap_or_else(|| unreachable!());
    let time = op.params.float(HolographicFoil::DERIVED_TIME, -1.0);
    let blocks = if op.def.schema().match_name == "hologram" {
        <Hologram as EffectMetadata>::read(op.params)
            .packed(op.params, W, H)
            .blocks
    } else {
        <HolographicFoil as EffectMetadata>::read(op.params)
            .packed(op.params, W, H)
            .blocks
    };
    (blocks, time)
}

fn close(a: &Blocks, b: &Blocks) -> bool {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .all(|(x, y)| (x - y).abs() <= 1e-4 * (1.0 + x.abs()))
}

/// Follow layer rotation turns the sheet by the layer's 3D turn, its
/// parent's, or the camera's taken out, whichever carries it; a 2D layer
/// ignores its own rotation and the camera. Layer time reaches the wobble,
/// and only while the sheet moves with it, and Motion blur hands the packer
/// the pose at the shutter's start.
#[test]
fn follow_rotation_and_the_shutter_reach_the_rows() {
    for name in ["holographic_foil", "hologram"] {
        let plain = effect(name, false, 0.0, -20.0);
        let (want, time) = resolved(&plain, staged(true, 0.0, None, None));
        assert_eq!(time, -1.0, "{name}: a still sheet leaves layer time out");
        let mut wobbling = plain.clone();
        set(&mut wobbling, "auto_tilt", float(8.0));
        let (_, time) = resolved(&wobbling, staged(true, 0.0, None, None));
        assert_eq!(time, 0.5, "{name}");
        let follow = effect(name, true, 0.0, 0.0);
        for (what, cx) in [
            ("own turn", staged(true, 20.0, None, None)),
            ("parent", staged(true, 0.0, Some(20.0), None)),
            ("camera", staged(true, 0.0, None, Some(-20.0))),
        ] {
            assert!(close(&resolved(&follow, cx).0, &want), "{name}: {what}");
        }
        // Off, or a 2D layer under a turned camera: no turn.
        let (flat, _) = resolved(
            &effect(name, false, 0.0, 0.0),
            staged(true, 0.0, None, None),
        );
        let (off, _) = resolved(
            &effect(name, false, 0.0, 0.0),
            staged(true, 20.0, None, None),
        );
        let (two_d, _) = resolved(&follow, staged(false, 20.0, None, Some(-20.0)));
        assert!(same_bytes(&off, &flat) && close(&two_d, &flat), "{name}");
    }

    // The shutter: the pose and the brightest light where they were, which
    // for a still card is where they are.
    let (blurred, _) = resolved(
        &effect("holographic_foil", true, 50.0, 0.0),
        staged(true, 20.0, None, None),
    );
    assert_eq!(blurred[PREV_T][3], 1.0);
    assert_eq!(blurred[PREV_KEY][3], 1.0);
    assert!((0..3).all(|i| (blurred[PREV_T][i] - blurred[T_AXIS][i]).abs() < 1e-5));
    let (sharp, _) = resolved(
        &effect("holographic_foil", true, 0.0, 0.0),
        staged(true, 20.0, None, None),
    );
    assert_eq!(sharp[PREV_T], [0.0; 4]);
}

/// Every built-in look of both effects packs to rows the kernel can read
/// and keeps the layers its rows read and the seed.
#[test]
fn every_built_in_look_packs_and_keeps_layers_and_seed() {
    let defs: [(&str, &dyn EffectDef); 2] = [
        ("holographic_foil", &HolographicFoilDef),
        ("hologram", &HologramDef),
    ];
    let layer = Some(Uuid::now_v7());
    for (name, def) in defs {
        for preset in def.presets() {
            let mut inst = crate::fx::builtins::instantiate(name).unwrap_or_else(|| unreachable!());
            set(&mut inst, "seed", EffectValue::Seed(77));
            set(&mut inst, "height_map", EffectValue::Layer(layer));
            crate::fx::presets::apply(
                &mut inst,
                preset,
                crate::time::Rational::ZERO,
                (1920.0, 1080.0),
            );
            assert_eq!(
                inst.param("seed"),
                Some(&EffectValue::Seed(77)),
                "{}",
                preset.name
            );
            assert_eq!(
                inst.param("height_map"),
                Some(&EffectValue::Layer(layer)),
                "{}",
                preset.name
            );
            let (b, _) = resolved(&inst, staged(true, 0.0, None, None));
            assert_sane(&b, preset.name);
        }
    }
}
