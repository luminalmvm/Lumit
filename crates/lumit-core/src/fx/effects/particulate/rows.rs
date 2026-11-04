//! Particulate's groups of rows and when each row is live.

use crate::fx::schema::{EnabledCond, EnabledWhen, ParamGroup};

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// The kickers particulate.md §2 lists its controls under, and the ones added
/// since. None starts closed: every one of them holds a control someone
/// reaches for on the first day.
pub const PARTICULATE_GROUPS: &[ParamGroup] = &[
    group(
        "Emitter",
        &[
            "in_scene",
            "shape",
            "position_x",
            "position_y",
            "position_z",
            "width",
            "height",
            "depth",
            "emitter_angle",
            "path_from",
            "mask_path",
            "emitter_layer",
            "density_layer",
            "density_layer_placement",
            "emit_rate",
            "direction",
            "direction_z",
            "spread",
            "spread_z",
            "initial_speed",
            "speed_jitter",
        ],
    ),
    group(
        "Bursts",
        &["burst", "burst_count", "burst_interval", "trigger"],
    ),
    group(
        "Particle",
        &[
            "life",
            "life_jitter",
            "size",
            "size_jitter",
            "size_over_life",
            "opacity_over_life",
            "colour",
            "end_colour",
            "rotation",
            "rotation_jitter",
            "spin",
            "align_to_motion",
        ],
    ),
    group(
        "Variation",
        &[
            "size_by",
            "opacity_by",
            "colour_by",
            "speed_min",
            "speed_max",
        ],
    ),
    group(
        "Forces",
        &[
            "gravity",
            "wind_x",
            "wind_y",
            "wind_z",
            "drag",
            "turbulence_amount",
            "turbulence_scale",
            "turbulence_speed",
        ],
    ),
    group("Edges", &["edge", "edge_margin"]),
    group(
        "Collide",
        &["collide_with", "on_contact", "bounciness", "friction"],
    ),
    group(
        "Flock",
        &["separation", "alignment", "cohesion", "flock_radius"],
    ),
    group(
        "Sparks",
        &["sparks", "spark_speed", "spark_life", "spark_size"],
    ),
    group("Trails", &["persistence", "zoom", "turn"]),
    group(
        "Render",
        &[
            "mode",
            "feather",
            "sprite_layer",
            "streak_length",
            "particle_blend",
            "max_particles",
        ],
    ),
];

/// Each render mode's own control, live only in that mode.
///
/// Wind's dependence on Drag is deliberately **not** here: a row that means
/// nothing while a *number* beside it is zero is not one of the three shapes
/// [`EnabledCond`] carries, and inventing a fourth for one row would be
/// growing the panel's affordance set into a scripting surface. The Wind rows
/// say it in their descriptions instead, which is where particulate.md §2 puts
/// it.
pub const PARTICULATE_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "feather",
        on: "mode",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "sprite_layer",
        on: "mode",
        cond: EnabledCond::ChoiceIs(1),
    },
    EnabledWhen {
        param: "streak_length",
        on: "mode",
        cond: EnabledCond::ChoiceIs(2),
    },
    EnabledWhen {
        param: "emitter_layer",
        on: "shape",
        cond: EnabledCond::ChoiceIs(9),
    },
    EnabledWhen {
        param: "density_layer",
        on: "shape",
        cond: EnabledCond::ChoiceIsNot(9),
    },
    // The step and the kept picture draw on the layer, so they stand aside
    // while the particles are in the scene.
    EnabledWhen {
        param: "collide_with",
        on: "in_scene",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "separation",
        on: "in_scene",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "alignment",
        on: "in_scene",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "cohesion",
        on: "in_scene",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "sparks",
        on: "in_scene",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "persistence",
        on: "in_scene",
        cond: EnabledCond::BoolIs(false),
    },
    // Each burst row means something only in the bursts that read it.
    EnabledWhen {
        param: "burst_count",
        on: "burst",
        cond: EnabledCond::ChoiceIsNot(0),
    },
    EnabledWhen {
        param: "burst_interval",
        on: "burst",
        cond: EnabledCond::ChoiceIs(2),
    },
    EnabledWhen {
        param: "trigger",
        on: "burst",
        cond: EnabledCond::ChoiceIs(3),
    },
    EnabledWhen {
        param: "edge_margin",
        on: "edge",
        cond: EnabledCond::ChoiceIsNot(0),
    },
    // Following the shape leaves no mask to choose.
    EnabledWhen {
        param: "mask_path",
        on: "path_from",
        cond: EnabledCond::ChoiceIsNot(1),
    },
    // The step reads no pictures, so a collider, a flock or sparks leave
    // births even, and their particles are drawn over the picture one by one.
    EnabledWhen {
        param: "particle_blend",
        on: "separation",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "particle_blend",
        on: "alignment",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "particle_blend",
        on: "cohesion",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "particle_blend",
        on: "sparks",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer",
        on: "separation",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer",
        on: "alignment",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer",
        on: "cohesion",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer",
        on: "sparks",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer_placement",
        on: "separation",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer_placement",
        on: "alignment",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer_placement",
        on: "cohesion",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer_placement",
        on: "sparks",
        cond: EnabledCond::Nought,
    },
    // A kept picture is drawn from the driver walk's stream, which reads no
    // pictures either.
    EnabledWhen {
        param: "particle_blend",
        on: "persistence",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer",
        on: "persistence",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "density_layer_placement",
        on: "persistence",
        cond: EnabledCond::Nought,
    },
    EnabledWhen {
        param: "particle_blend",
        on: "collide_with",
        cond: EnabledCond::LayerUnset,
    },
    EnabledWhen {
        param: "density_layer",
        on: "collide_with",
        cond: EnabledCond::LayerUnset,
    },
    EnabledWhen {
        param: "density_layer_placement",
        on: "collide_with",
        cond: EnabledCond::LayerUnset,
    },
    EnabledWhen {
        param: "on_contact",
        on: "collide_with",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "bounciness",
        on: "collide_with",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "bounciness",
        on: "on_contact",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "friction",
        on: "collide_with",
        cond: EnabledCond::LayerSet,
    },
    // Only a particle that carries on along the surface slows on it.
    EnabledWhen {
        param: "friction",
        on: "on_contact",
        cond: EnabledCond::ChoiceIsNot(2),
    },
    EnabledWhen {
        param: "friction",
        on: "on_contact",
        cond: EnabledCond::ChoiceIsNot(3),
    },
];

/// Whether `inst` is a Particulate drawn on its layer rather than in the
/// scene: the only place Collide with, a Flock, Sparks and Persistence apply.
#[must_use]
pub fn on_its_layer(inst: &crate::model::EffectInstance) -> bool {
    inst.effect.match_name == "particulate"
        && !matches!(
            inst.param("in_scene"),
            Some(crate::model::EffectValue::Bool(true))
        )
}

/// Whether `e` is a Particulate with its particles in the scene.
#[must_use]
pub fn is_scene(e: &crate::model::EffectInstance) -> bool {
    e.enabled
        && e.effect.match_name == "particulate"
        && matches!(
            e.param("in_scene"),
            Some(crate::model::EffectValue::Bool(true))
        )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::model::{EffectValue, LayerKind};
    use crate::ops::Op;

    /// In the scene arriving by any route turns a 2D layer's 3D switch on, once.
    #[test]
    fn in_the_scene_puts_a_2d_layer_in_3d() {
        let layer = crate::edits::base_layer(
            "Particles".into(),
            LayerKind::Null,
            crate::time::Rational::new(1, 1).unwrap(),
            Default::default(),
        );
        let mut fx = crate::fx::builtins::instantiate("particulate").unwrap();
        let comp = uuid::Uuid::now_v7();
        assert!(matches!(
            crate::fx::effects::particulate::set_effects(comp, &layer, vec![fx.clone()]),
            Op::SetLayerEffects { .. }
        ));
        let on = fx.params.iter_mut().find(|p| p.id == "in_scene").unwrap();
        on.value = EffectValue::Bool(true);
        let Op::Batch { ops } =
            crate::fx::effects::particulate::set_effects(comp, &layer, vec![fx.clone()])
        else {
            panic!("a 2D layer was left flat");
        };
        assert!(matches!(ops[1], Op::SetLayerThreeD { three_d: true, .. }));
        // Already on the layer, it doesn't turn 3D back on.
        let mut had = layer.clone();
        had.effects = vec![fx.clone()];
        assert!(matches!(
            crate::fx::effects::particulate::set_effects(comp, &had, vec![fx]),
            Op::SetLayerEffects { .. }
        ));
    }
}
