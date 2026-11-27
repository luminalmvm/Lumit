use super::*;

#[test]
fn every_look_has_three_or_five_colours_and_a_name() {
    assert_eq!(
        COLOURISE_OPTIONS.len() as u32,
        FIRST_LOOK + LOOKS.len() as u32
    );
    for (i, look) in LOOKS.iter().enumerate() {
        let n = if (i as u32) + FIRST_LOOK < FIRST_FIVE_LOOK {
            3
        } else {
            5
        };
        assert_eq!(
            look.len(),
            n,
            "{}",
            COLOURISE_OPTIONS[i + FIRST_LOOK as usize]
        );
    }
    assert_eq!(mode_of(5), COLOURISE_THREE);
    assert_eq!(mode_of(15), COLOURISE_FIVE);
    assert_eq!(mode_of(1), COLOURISE_ONE);
}

#[test]
fn the_fade_splits_into_one_set_of_weights_a_pass() {
    let fine = FALL / (TAPS.pow(PASSES as u32) as f32);
    for k in 0..PASSES {
        for j in 0..TAPS {
            let at = (j * TAPS.pow(k as u32)) as f32;
            let want = (-at * fine).exp();
            assert!((LightRaysParams::weight(k, j) - want).abs() < 1e-6);
        }
    }
}

#[test]
fn the_shimmer_wraps_round_the_circle_and_loops_in_phase() {
    let mut fx = LightRays::read(Params::EMPTY);
    fx.shimmer_amount = 80.0;
    fx.use_loop = true;
    fx.revolutions = 2;
    let a = fx.packed(64, 48).lengths;
    let step = (a[0] - a[TABLE - 1]).abs();
    assert!(step < 0.05, "seam step {step}");
    fx.shimmer_phase = 720.0;
    let b = fx.packed(64, 48).lengths;
    for (x, y) in a.iter().zip(&b) {
        assert!((x - y).abs() < 1e-5);
    }
    fx.shimmer_phase = 300.0;
    let c = fx.packed(64, 48).lengths;
    assert!(a.iter().zip(&c).any(|(x, y)| (x - y).abs() > 0.01));
}

#[test]
fn no_ray_length_leaves_the_light_where_it_is() {
    let (w, h) = (8u32, 6u32);
    let img: Vec<f32> = (0..w * h * 4).map(|i| (i % 7) as f32 / 7.0).collect();
    let mut fx = LightRays::read(Params::EMPTY);
    fx.ray_length = 0.0;
    fx.rays_blend = BLEND_NONE;
    let mut out = img.clone();
    light_rays(&mut out, w, h, &fx.packed(w, h));
    for (i, (a, b)) in img.iter().zip(&out).enumerate() {
        let want = if i % 4 == 3 {
            (a * RAY_GAIN).min(1.0)
        } else {
            a * RAY_GAIN
        };
        assert!((want - b).abs() < 4e-3, "{want} {b}");
    }
}

#[test]
fn a_bright_dot_throws_a_ray_away_from_the_source() {
    let (w, h) = (64u32, 16u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 7..9 {
        for x in 20..22 {
            let i = ((y * w + x) * 4) as usize;
            img[i..i + 4].copy_from_slice(&[1.0; 4]);
        }
    }
    let mut fx = LightRays::read(Params::EMPTY);
    fx.source_x = 0.0;
    fx.source_y = 8.0;
    fx.rays_blend = BLEND_NONE;
    let mut out = img.clone();
    light_rays(&mut out, w, h, &fx.packed(w, h));
    let at = |x: u32| out[((8 * w + x) * 4) as usize];
    assert!(at(30) > 0.0, "the ray reaches past the dot");
    assert!(at(30) > at(50), "and fades as it goes");
    assert_eq!(at(10), 0.0, "nothing towards the source");
}

#[test]
fn the_gradient_runs_black_to_shadows_to_highlights() {
    let mut fx = LightRays::read(Params::EMPTY);
    fx.colourise = COLOURISE_THREE;
    let p = fx.packed(64, 48);
    assert_eq!(gradient(&p, 0.0), [0.0; 3]);
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    assert!(near(gradient(&p, 0.1), rgb3(fx.shadows)));
    assert!(near(gradient(&p, 0.8), rgb3(fx.highlights)));
    assert_eq!(gradient(&p, 1.0), [1.0; 3]);
}

#[test]
fn extend_beyond_layer_gives_the_rays_room_up_to_a_diagonal() {
    let mut fx = LightRays::read(Params::EMPTY);
    assert_eq!(fx.reach(2203.0), 0.0, "off by default: no room");
    fx.extend_beyond_layer = true;
    assert_eq!(fx.reach(2203.0), 2203.0, "long rays: one diagonal");
    fx.ray_length = 0.05;
    let short = fx.reach(2203.0);
    assert!(
        short > 0.0 && short < 2203.0,
        "short rays: less room ({short})"
    );
    fx.ray_length = 0.0;
    assert_eq!(fx.reach(2203.0), 0.0, "no rays: no room");
}

#[test]
fn the_rays_never_light_past_the_room_reach_within_gives() {
    let (w, h) = (40u32, 24u32);
    let diag = (w as f32).hypot(h as f32);
    let picture = [0.0, 0.0, w as f32, h as f32];
    for (sx, sy) in [(20.0, 12.0), (3.0, 20.0), (-15.0, 8.0), (60.0, -10.0)] {
        for (length, shimmer) in [(0.3, 0.0), (0.3, 100.0), (2.0, 0.0), (0.05, 50.0)] {
            let mut fx = LightRays::read(Params::EMPTY);
            fx.extend_beyond_layer = true;
            fx.threshold = 0.0;
            fx.ray_length = length;
            fx.shimmer_amount = shimmer;
            fx.source_x = sx;
            fx.source_y = sy;
            let loose = fx.reach(diag);
            let tight = fx.reach_within(diag, picture);
            let case = format!("source ({sx}, {sy}), length {length}, shimmer {shimmer}");
            assert!(tight <= loose, "{case}: {tight} > {loose}");
            assert!(tight > 0.0, "{case}: rays with no room");
            let pad = loose.ceil() as u32;
            let (pw, ph) = (w + 2 * pad, h + 2 * pad);
            let mut rgba = vec![0.0f32; (pw * ph * 4) as usize];
            for y in pad..pad + h {
                for x in pad..pad + w {
                    let i = ((y * pw + x) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&[3.0, 3.0, 3.0, 1.0]);
                }
            }
            let mut moved = fx;
            moved.source_x += pad as f32;
            moved.source_y += pad as f32;
            light_rays(&mut rgba, pw, ph, &moved.packed(pw, ph));
            let room = tight.ceil();
            let (mut lit_outside, mut past) = (0, Vec::new());
            for y in 0..ph {
                for x in 0..pw {
                    let i = ((y * pw + x) * 4) as usize;
                    if rgba[i..i + 4].iter().all(|v| *v <= 1e-6) {
                        continue;
                    }
                    let (px, py) = (x as f32 - pad as f32, y as f32 - pad as f32);
                    let dx = (-px).max(px + 1.0 - w as f32).max(0.0);
                    let dy = (-py).max(py + 1.0 - h as f32).max(0.0);
                    if dx > 0.0 || dy > 0.0 {
                        lit_outside += 1;
                    }
                    if dx > room || dy > room {
                        past.push((x, y));
                    }
                }
            }
            assert!(lit_outside > 0, "{case}: the rays should leave the picture");
            assert!(
                past.is_empty(),
                "{case}: {} lit past the room of {room}, first {:?}",
                past.len(),
                past.first()
            );
            let small = room as u32;
            let (sw, sh) = (w + 2 * small, h + 2 * small);
            let mut tight_rgba = vec![0.0f32; (sw * sh * 4) as usize];
            for y in small..small + h {
                for x in small..small + w {
                    let i = ((y * sw + x) * 4) as usize;
                    tight_rgba[i..i + 4].copy_from_slice(&[3.0, 3.0, 3.0, 1.0]);
                }
            }
            let mut placed = fx;
            placed.source_x += small as f32;
            placed.source_y += small as f32;
            light_rays(&mut tight_rgba, sw, sh, &placed.packed(sw, sh));
            let mut worst = 0.0f32;
            for y in 0..sh {
                for x in 0..sw {
                    let i = ((y * sw + x) * 4) as usize;
                    let j = (((y + pad - small) * pw + x + pad - small) * 4) as usize;
                    for c in 0..4 {
                        worst = worst.max((tight_rgba[i + c] - rgba[j + c]).abs());
                    }
                }
            }
            assert!(
                worst < 2e-3,
                "{case}: the tight room changed a pixel by {worst}"
            );
        }
    }
    let mut fx = LightRays::read(Params::EMPTY);
    fx.extend_beyond_layer = true;
    fx.ray_length = 0.3;
    fx.source_x = 1700.0;
    fx.source_y = 1050.0;
    let (loose, tight) = (
        fx.reach(4061.0),
        fx.reach_within(4061.0, [0.0, 0.0, 3478.0, 2096.0]),
    );
    assert!(tight < loose * 0.4, "{tight} against {loose}");
}

#[test]
fn a_look_fills_the_colours_and_an_edit_puts_the_menu_back() {
    let before = crate::fx::builtins::instantiate("light_rays").unwrap();
    let set = |e: &mut EffectInstance, id: &str, v: EffectValue| {
        if let Some(p) = e.params.iter_mut().find(|p| p.id == id) {
            p.value = v;
        }
    };
    let mut after = before.clone();
    set(&mut after, "colourise", EffectValue::Choice(16));
    let pressed = settle(Some(&before), &after).unwrap();
    let ids: Vec<_> = pressed.rows.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, FIVE_ROWS);
    let mut plain = before.clone();
    set(
        &mut plain,
        "colourise",
        EffectValue::Choice(COLOURISE_THREE),
    );
    assert_eq!(settle(Some(&before), &plain), None);
    let mut ember = before.clone();
    set(&mut ember, "colourise", EffectValue::Choice(4));
    let mut edited = ember.clone();
    let other = edited.param("colour").cloned().unwrap();
    set(&mut edited, "midtones", other);
    let pressed = settle(Some(&ember), &edited).unwrap();
    assert_eq!(
        pressed.rows,
        vec![("colourise", Value::Choice(COLOURISE_THREE))]
    );
    let mut moved = ember.clone();
    set(&mut moved, "use_mask", EffectValue::Bool(true));
    assert_eq!(settle(Some(&ember), &moved), None);
}

fn rgb3(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

#[test]
fn normal_puts_the_rays_over_the_picture_and_add_adds_them() {
    let d = [0.2, 0.3, 0.4, 1.0];
    let s = [0.5, 0.25, 0.0, 0.5];
    assert_eq!(combine(d, s, BLEND_NONE), s);
    let n = combine(d, s, BLEND_NORMAL);
    assert!((n[0] - 0.6).abs() < 1e-6 && (n[3] - 1.0).abs() < 1e-6);
    let a = combine(d, s, BLEND_MODES[BLEND_ADD as usize] + 2);
    for c in 0..3 {
        assert!((a[c] - (d[c] + s[c])).abs() < 1e-5);
    }
}
