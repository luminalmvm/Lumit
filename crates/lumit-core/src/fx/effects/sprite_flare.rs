//! Sprite flare (docs/08 §3.29): the art-directed flare. What it draws is a
//! design kept in the effect, a list of glows, rays, rings, ghosts and so on
//! (see [`crate::fx::flare`]), edited in the Flare window. The rows here are
//! what animates it: the light, brightness, size, rotation, colour,
//! evolution and flicker.

use std::sync::{Arc, Mutex};

use crate::fx::flare::design::{self, Design, Element, Kind};
use crate::fx::flare::presets::default_design;
use crate::fx::flare::raster::{self, Frame, Light};
use crate::fx::noise::value3;
use crate::fx::{
    params::{ParamId, Params, Value},
    registry::{EffectDef, EffectMetadata, ResolveCx},
    schema::{EffectSchema, ParamGroup},
};
use crate::model::{EffectInstance, EffectParam, EffectValue};
use lumit_fx_macros::Effect;

/// The panel's two twirls.
pub const SPRITE_FLARE_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Centre",
        params: &["centre_x", "centre_y"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Flicker",
        params: &["flicker", "flicker_speed", "flicker_seed"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Sprite flare's controls.
///
/// Every position and distance is px@comp, declared `Px` so the resolve
/// step scales it to the raster. The design's own sizes are px@comp too, and
/// are scaled by the factor pushed beside them.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "sprite_flare",
    label = "Sprite flare",
    // 2: the glow, ghost and streak rows became a design kept in the effect.
    // `migrate_percent_to_px` carries a v1 instance over on load.
    version = 2,
    category = Stylise,
    cost = Cheap,
    roi = FullFrame,
    groups = SPRITE_FLARE_GROUPS,
)]
pub struct SpriteFlare {
    /// Opens the Flare window.
    #[action(label = "Edit flare…")]
    pub edit_flare: (),

    /// Open both sides: a light off the frame still throws ghosts across it.
    #[slider(min = 0.0, max = 3840.0, default = 640.0, unit = Px)]
    pub light_x: f32,

    /// See [`light_x`](Self::light_x).
    #[slider(min = 0.0, max = 2160.0, default = 360.0, unit = Px)]
    pub light_y: f32,

    /// Gain on everything the effect draws; 0 draws nothing.
    #[slider(
        label = "Brightness",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub intensity: f32,

    /// The whole design's scale, per cent.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub size: f32,

    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// The light's colour, scene-linear and open above.
    #[colour(label = "Colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub tint: [f32; 4],

    /// What turns rays' shimmer and orbs' drift. A full turn comes back to
    /// the start.
    #[dial(label = "Evolution", default = 0.0)]
    pub evolution: f32,

    /// The optical centre the axis runs through. The schema default is the
    /// 1080p middle; `instantiate_for_raster` puts a fresh one on the comp's.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// See [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// How far the brightness dips at random.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 1.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 1.0,
        unit = Raw
    )]
    pub flicker: f32,

    /// Dips a second.
    #[slider(
        label = "Speed",
        min = 0.0,
        max = 30.0,
        default = 8.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub flicker_speed: f32,

    /// A counter rather than a seed row, so a flare that doesn't flicker
    /// keeps one frame key across time. See [`moves_with_time`].
    #[counter(label = "Seed", min = 0, max = 999, default = 1, hard_min = 0, unit = Raw)]
    pub flicker_seed: i32,

    /// How far past the frame's edge the light fades out. 0 never fades.
    #[slider(min = 0.0, max = 1000.0, default = 200.0, hard_min = 0.0, unit = Px)]
    pub edge_fade: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl SpriteFlare {
    /// Raster pixels per comp pixel, pushed at resolve for the design's sizes.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");
    /// Layer time, seconds, for Spin and Flicker.
    pub const DERIVED_TIME: ParamId = ParamId::new("derived.time");

    /// The frame the design is drawn into, out of a resolved bag. No lights
    /// at Brightness 0 or Mix 0, which both render paths take as the
    /// untouched input.
    pub fn frame(p: Params<'_>) -> Frame {
        let s = Self::read(p);
        let px_scale = p.float(Self::DERIVED_PX_SCALE, 1.0).max(1e-6);
        let time = p.float(Self::DERIVED_TIME, 0.0);
        let mix = (s.mix / 100.0).clamp(0.0, 1.0);
        let dip = 0.5
            + 0.5
                * value3(
                    s.flicker_seed.max(0) as u32,
                    0,
                    time * s.flicker_speed,
                    0.0,
                    0.0,
                    0,
                );
        let lights = if s.intensity > 0.0 && mix > 0.0 {
            vec![Light {
                pos: [s.light_x, s.light_y],
                colour: [s.tint[0].max(0.0), s.tint[1].max(0.0), s.tint[2].max(0.0)],
                // Mix rides on the light, so even an element that ignores
                // Brightness is mixed.
                gain: mix,
            }]
        } else {
            Vec::new()
        };
        Frame {
            lights,
            centre: [s.centre_x, s.centre_y],
            px_scale,
            brightness: s.intensity.max(0.0) * (1.0 - s.flicker.clamp(0.0, 1.0) * dip),
            size: (s.size / 100.0).max(0.0),
            rotation_deg: s.rotation,
            evolution_deg: s.evolution,
            time_s: time,
            // The row arrives at the raster's size, the frame wants comp px.
            edge_fade: s.edge_fade.max(0.0) / px_scale,
        }
    }
}

/// Whether the flare changes over time with every row held still: Flicker
/// is on or keyed, or a shown element spins. Only then does the layer time
/// join its frame key, so a still flare is cached once for its whole span.
pub fn moves_with_time(e: &EffectInstance) -> bool {
    let flickers = match e.param("flicker") {
        Some(EffectValue::Float(p)) => p.is_animated() || p.value_at(0.0) > 0.0,
        _ => false,
    };
    flickers
        || design::embedded(e)
            .unwrap_or_else(default_design)
            .elements
            .iter()
            .any(|el| el.on && el.num("spin") != 0.0)
}

/// The design a render draws: the kept one (content hash and text, as the
/// lens carriage brings it) or the default when none is kept or it won't
/// read. Parsed once per text.
pub fn design_of(kept: Option<&(u64, String)>) -> Arc<Design> {
    static PARSED: Mutex<Vec<(u64, Arc<Design>)>> = Mutex::new(Vec::new());
    let Some((hash, text)) = kept else {
        return Arc::new(default_design());
    };
    if let Some(hit) = PARSED
        .lock()
        .ok()
        .and_then(|list| list.iter().find(|(h, _)| h == hash).map(|(_, d)| d.clone()))
    {
        return hit;
    }
    let design = Arc::new(Design::parse(text).unwrap_or_else(default_design));
    if let Ok(mut list) = PARSED.lock() {
        if list.len() >= 8 {
            list.remove(0);
        }
        list.push((*hash, design.clone()));
    }
    design
}

/// The rows version 1 had and version 2 keeps in its design instead.
const V1_ROWS: [&str; 9] = [
    "glow_size",
    "glow_intensity",
    "ghosts",
    "ghost_spacing",
    "ghost_size",
    "ghost_intensity",
    "streak_length",
    "streak_intensity",
    "streak_angle",
];

/// Carry a version 1 instance over: its glow, ghosts and streak become a
/// design, its centre the middle of the `w` by `h` comp it was always drawn
/// about, and it never fades at the edge, as it never did. The glow and the
/// streak come out exactly; the ghosts were a disc profile no iris draws, so
/// they are a near match. A keyframed row is read at its first frame.
pub fn migrate_v1(e: &mut EffectInstance, w: f64, h: f64) {
    let old = |id: &str, default: f32| e.float_at(id, 0.0).map_or(default, |v| v as f32);
    let (glow_size, glow) = (old("glow_size", 120.0), old("glow_intensity", 1.0));
    let ghosts = old("ghosts", 6.0).round().clamp(0.0, 16.0) as u32;
    let spacing = old("ghost_spacing", 0.35).max(0.0);
    let (ghost_size, ghost) = (old("ghost_size", 60.0), old("ghost_intensity", 0.35));
    let (streak_length, streak) = (old("streak_length", 300.0), old("streak_intensity", 0.5));
    let streak_angle = old("streak_angle", 0.0);

    let element = |kind: Kind, values: &[(&str, f32)]| {
        let mut el = Element::new(kind);
        for (key, v) in values {
            el.set(key, *v);
        }
        el
    };
    let mut elements = Vec::new();
    if glow > 0.0 && glow_size > 0.0 {
        elements.push(element(
            Kind::Glow,
            &[
                ("size", glow_size),
                ("brightness", glow),
                ("falloff", 0.5),
                ("core", 0.0),
            ],
        ));
    }
    if ghost > 0.0 && ghost_size > 0.0 {
        for i in 1..=ghosts {
            let t = i as f32 * spacing;
            // Every other ghost fell off harder, so it is drawn smaller.
            let tighter = if i % 2 == 0 { 0.6 } else { 0.75 };
            elements.push(element(
                Kind::Ghost,
                &[
                    ("place", 1.0 + t),
                    ("size", ghost_size * (0.35 + 0.65 * t) * tighter),
                    ("brightness", ghost / i as f32),
                    ("sides", 0.0),
                    ("fill", 1.0),
                    ("edge", 0.0),
                    ("softness", 1.0),
                    ("fringe", 0.0),
                ],
            ));
        }
    }
    if streak > 0.0 && streak_length > 0.0 {
        elements.push(element(
            Kind::Streak,
            &[
                ("size", streak_length),
                ("brightness", streak),
                ("thickness", 0.03 * streak_length),
                ("taper", 0.5),
                ("rotation", streak_angle),
            ],
        ));
    }
    design::set_embedded(
        e,
        Some(&Design {
            lumit_flare: design::FORMAT,
            name: String::new(),
            elements,
        }),
    );
    e.params.retain(|p| !V1_ROWS.contains(&p.id.as_str()));
    for (id, v) in [
        ("centre_x", w * 0.5),
        ("centre_y", h * 0.5),
        ("edge_fade", 0.0),
    ] {
        let value = EffectValue::Float(crate::anim::Property::fixed(v));
        match e.params.iter_mut().find(|p| p.id == id) {
            Some(p) => p.value = value,
            None => e.params.push(EffectParam {
                id: id.to_owned(),
                value,
                extra: serde_json::Map::new(),
            }),
        }
    }
    e.effect.version = 2;
}

/// Sprite flare's behaviour.
pub struct SpriteFlareDef;

impl EffectDef for SpriteFlareDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SpriteFlare as EffectMetadata>::SCHEMA
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(SpriteFlare::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        push(SpriteFlare::DERIVED_TIME, Value::Float(cx.lt as f32));
    }

    /// The default design: a kept one rides beside the op, which a bag alone
    /// can't carry.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        raster::draw(&default_design(), &SpriteFlare::frame(p), w, h, rgba);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An old flare's glow and streak draw exactly what they drew, and its
    /// ghosts land where they were.
    #[test]
    fn a_version_1_sprite_flare_carries_over() {
        let (w, h) = (64u32, 48u32);
        let mut e =
            crate::fx::builtins::instantiate("sprite_flare").unwrap_or_else(|| unreachable!());
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
                value: EffectValue::Float(crate::anim::Property::fixed(v)),
                extra: serde_json::Map::new(),
            });
        }
        crate::fx::builtins::backfill_builtin_params(std::slice::from_mut(&mut e));
        crate::fx::builtins::migrate_percent_to_px(std::slice::from_mut(&mut e), 64.0, 48.0);

        assert_eq!(e.effect.version, 2);
        assert!(V1_ROWS.iter().all(|id| e.param(id).is_none()));
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
        let only: Vec<Element> = d
            .elements
            .into_iter()
            .filter(|el| el.kind != Kind::Ghost)
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
    }
}
