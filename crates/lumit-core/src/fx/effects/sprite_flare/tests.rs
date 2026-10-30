use super::*;

/// An old flare's glow and streak draw exactly what they drew, about the
/// middle of the picture it runs on, and its ghosts land where they were. One
/// whose rows move keeps them, and they go on drawing the same.
#[test]
fn a_version_1_sprite_flare_carries_over() {
    use crate::anim::{Keyframe, Property, SideInterp};
    let (w, h) = (64u32, 48u32);
    let mut e = crate::fx::builtins::instantiate("sprite_flare").unwrap_or_else(|| unreachable!());
    e.effect.version = 1;
    e.params
        .retain(|p| p.id != "centre_x" && p.id != "centre_y");
    let old = [
        ("light_x", 20.0),
        ("light_y", 18.0),
        ("glow_size", 9.0),
        ("glow_intensity", 1.2),
        ("ghosts", 3.0),
        ("ghost_spacing", 0.4),
        ("ghost_size", 5.0),
        ("ghost_intensity", 0.5),
        ("streak_length", 30.0),
        ("streak_intensity", 0.6),
        ("streak_angle", 12.0),
    ];
    for (id, v) in old {
        e.params.retain(|p| p.id != id);
        e.params.push(EffectParam {
            id: id.to_owned(),
            value: EffectValue::Float(Property::fixed(v)),
            extra: serde_json::Map::new(),
        });
    }
    // The same flare with its glow keyed, 1.2 halfway through a second.
    let mut moving = e.clone();
    for p in &mut moving.params {
        if p.id == "glow_intensity" {
            let key = |s: i64, value| Keyframe {
                time: crate::time::Rational::new(s, 1).unwrap_or_else(|_| unreachable!()),
                value,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            };
            p.value = EffectValue::Float(Property {
                animation: Animation::Keyframed(vec![key(0, 0.0), key(1, 2.4)]),
                extra: serde_json::Map::new(),
            });
        }
    }
    let mut wired = e.clone();
    let comp = (1920.0, 1080.0);
    let own = (64.0, 48.0);
    let migrate = |e: &mut EffectInstance, wired: &dyn Fn(uuid::Uuid, &str) -> bool| {
        crate::fx::builtins::migrate_in(std::slice::from_mut(e), comp, own, wired);
    };
    migrate(&mut e, &|_, _| false);
    migrate(&mut moving, &|_, _| false);
    migrate(&mut wired, &|_, row| row == "streak_angle");

    assert_eq!(e.effect.version, 2);
    assert!(V1_PARAMS.iter().all(|row| e.param(row.id).is_none()));
    assert!(SpriteFlareDef.derived(&e).is_empty());
    assert_eq!(e.float_at("centre_x", 0.0), Some(32.0));
    assert_eq!(e.float_at("edge_fade", 0.0), Some(0.0));
    let d = design::embedded(&e).unwrap_or_else(|| unreachable!());
    let kinds: Vec<Kind> = d.elements.iter().map(|el| el.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Glow,
            Kind::Ghost,
            Kind::Ghost,
            Kind::Ghost,
            Kind::Streak
        ]
    );
    assert!((d.elements[2].num("place") - 1.8).abs() < 1e-6);

    // Without the ghosts, the new drawing is the old formula.
    let matches_v1 = |d: &Design| {
        let only: Vec<Element> = (d.elements.iter())
            .filter(|el| el.kind != Kind::Ghost)
            .cloned()
            .collect();
        let design = Design {
            elements: only,
            ..default_design()
        };
        let mut frame = Frame::preview(w, h, [20.0, 18.0], 1.0);
        frame.lights[0].colour = [1.0, 0.5, 0.25];
        let mut new = vec![0.0; (w * h * 4) as usize];
        raster::draw(&design, &frame, w, h, &mut new);
        let (s, c) = 12f32.to_radians().sin_cos();
        let mut worst = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 + 0.5 - 20.0, y as f32 + 0.5 - 18.0);
                let glow = 1.2 * (-(dx.hypot(dy) / 9.0).powi(2)).exp();
                let along = (dx * c + dy * s) / 30.0;
                let across = (-dx * s + dy * c) / 0.9;
                let streak = 0.6 * (-(along * along + across * across)).exp();
                let at = ((y * w + x) * 4) as usize;
                for (k, tint) in [1.0, 0.5, 0.25].into_iter().enumerate() {
                    worst = worst.max((new[at + k] - (glow + streak) * tint).abs());
                }
            }
        }
        assert!(worst < 2e-4, "worst {worst}");
    };
    matches_v1(&d);

    // A keyed or wired row keeps every old row, drawn in front of an empty
    // design.
    for kept in [&moving, &wired] {
        assert!(V1_PARAMS.iter().all(|row| kept.param(row.id).is_some()));
        assert_eq!(SpriteFlareDef.derived(kept).len(), V1_PARAMS.len());
        let d = design::embedded(kept).unwrap_or_else(|| unreachable!());
        assert!(d.elements.is_empty());
    }
    let stack = crate::fx::resolved::resolve_instance(
        &moving,
        crate::fx::drivers::ResolvedDrivers::NONE,
        0.5,
        80.0,
        1.0,
        &crate::fx::markers::MarkerContext::NONE,
        Arc::new(crate::expression::ExpressionContext::detached()),
    );
    let op = stack.get(0).unwrap_or_else(|| unreachable!());
    let kept = Arc::new(design::embedded(&moving).unwrap_or_else(|| unreachable!()));
    matches_v1(&with_v1_rows(kept, op.params));
}

/// Mask points lights each corner of the mask, where it is at that moment.
#[test]
fn mask_points_light_each_corner_as_the_mask_moves() {
    use crate::mask::{Mask, PathKeyframe};
    let (mut doc, comp_id) = crate::fx::tests::lit_document(1);
    let comp = doc.comp_mut(comp_id).unwrap_or_else(|| unreachable!());
    let mut mask = Mask::rectangle(100.0, 50.0, 200.0, 100.0);
    let mut moved = mask.path.clone();
    for v in &mut moved.vertices {
        v.pos.0 += 40.0;
    }
    let key = |s: i64, path| PathKeyframe {
        time: crate::time::Rational::new(s, 1).unwrap_or_else(|_| unreachable!()),
        path,
        interp_in: crate::anim::SideInterp::Linear,
        interp_out: crate::anim::SideInterp::Linear,
    };
    mask.path_keys = vec![key(0, mask.path.clone()), key(1, moved)];
    comp.layers[0].masks.push(mask);
    let layer = comp.layers[0].id;

    let mut inst =
        crate::fx::builtins::instantiate("sprite_flare").unwrap_or_else(|| unreachable!());
    for p in &mut inst.params {
        if p.id == "light_from" {
            p.value = EffectValue::Choice(MASK_POINTS);
        }
    }
    let mut pushed = Vec::new();
    SpriteFlareDef.resolve_derived(
        &ResolveCx {
            inst: &inst,
            lt: 0.5,
            diag_px: 2202.9,
            px_scale: 1.0,
            markers: &crate::fx::markers::MarkerContext::NONE,
            context: Arc::new(crate::expression::ExpressionContext {
                document: Arc::new(doc),
                comp: Some(comp_id),
                layer: Some(layer),
                comp_time: 0.5,
                ..crate::expression::ExpressionContext::detached()
            }),
        },
        &mut |id, v| pushed.push((id, v)),
    );
    let get = |id| pushed.iter().find(|(k, _)| *k == id).map(|(_, v)| *v);
    assert_eq!(get(SpriteFlare::DERIVED_LIGHT_COUNT), Some(Value::Int(4)));
    // Halfway between the keys, half the 40 px move.
    assert_eq!(
        get(SpriteFlare::DERIVED_LIGHTS[0]),
        Some(Value::Vec4([120.0, 50.0, 1.0, 1.0]))
    );
}
