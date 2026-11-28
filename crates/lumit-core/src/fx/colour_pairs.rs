use crate::anim::{Animation, Property, SideInterp};
use crate::fx::schema::ParamKind;
use crate::model::{EffectInstance, EffectValue};

const PAIRS: &[(&str, &str, &str)] = &[
    ("tint", "black", "white"),
    ("gradient", "start_colour", "end_colour"),
    ("particulate", "colour", "end_colour"),
    ("vary_points", "colour_start", "colour"),
    ("mood_lighting", "light", "shade"),
    ("light_leak", "colour", "colour_2"),
    ("beam", "inside_colour", "outside_colour"),
    ("audio_spectrum", "inside_colour", "outside_colour"),
];

pub fn of(match_name: &str) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
    PAIRS
        .iter()
        .filter(move |(name, _, _)| *name == match_name)
        .map(|(_, a, b)| (*a, *b))
}

pub fn swap(inst: &mut EffectInstance, a: &str) -> bool {
    let Some((a, b)) = of(&inst.effect.match_name).find(|(first, _)| *first == a) else {
        return false;
    };
    let Some(i) = inst.params.iter().position(|p| p.id == a) else {
        return false;
    };
    let Some(j) = inst.params.iter().position(|p| p.id == b) else {
        return false;
    };
    let (lo, hi) = (i.min(j), i.max(j));
    let (head, tail) = inst.params.split_at_mut(hi);
    std::mem::swap(&mut head[lo].value, &mut tail[0].value);
    let ramps: Vec<&str> = crate::fx::builtins::schema(&inst.effect.match_name)
        .map(|s| {
            s.params
                .iter()
                .filter(|p| matches!(p.kind, ParamKind::Gradient { from, to } if (from, to) == (a, b) || (from, to) == (b, a)))
                .map(|p| p.id)
                .collect()
        })
        .unwrap_or_default();
    for param in inst
        .params
        .iter_mut()
        .filter(|p| ramps.contains(&p.id.as_str()))
    {
        if let EffectValue::Gradient(stops) = &mut param.value {
            stops.reverse();
            for stop in stops {
                mirror_unless_scripted(&mut stop.position);
            }
        }
    }
    true
}

fn mirror_unless_scripted(position: &mut Property) {
    match &mut position.animation {
        Animation::Static(v) => *v = 100.0 - *v,
        Animation::Keyframed(keys) => {
            for key in keys {
                key.value = 100.0 - key.value;
                for side in [&mut key.interp_in, &mut key.interp_out] {
                    if let SideInterp::Bezier { speed, .. } | SideInterp::Auto { speed, .. } = side
                    {
                        *speed = -*speed;
                    }
                }
            }
        }
        Animation::Expression(_) => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use crate::anim::{Animation, Keyframe, Property, SideInterp};
    use crate::model::EffectValue;
    use crate::shape::GradientStop;
    use crate::time::Rational;

    #[test]
    fn swap_moves_keys_and_expressions_and_turns_the_ramp_round() {
        let mut inst = crate::fx::builtins::instantiate("particulate").unwrap();
        let keyed = Property {
            animation: Animation::Keyframed(vec![Keyframe {
                time: Rational::ZERO,
                value: 0.25,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            }]),
            extra: serde_json::Map::new(),
        };
        let stop = |at: f64| GradientStop {
            position: Property::fixed(at),
            colour: std::array::from_fn(|_| Property::fixed(at / 100.0)),
            opacity: Property::fixed(100.0),
        };
        for p in &mut inst.params {
            match p.id.as_str() {
                "colour" => {
                    p.value = EffectValue::Colour([
                        keyed.clone(),
                        Property::fixed(0.0),
                        Property {
                            animation: Animation::Expression("time".into()),
                            extra: serde_json::Map::new(),
                        },
                        Property::fixed(1.0),
                    ]);
                }
                "colour_over_life" => p.value = EffectValue::Gradient(vec![stop(20.0), stop(70.0)]),
                _ => {}
            }
        }
        let before = inst.param("colour").cloned();

        assert!(super::swap(&mut inst, "colour"));
        assert!(!super::swap(&mut inst, "end_colour"));

        assert_eq!(inst.param("end_colour").cloned(), before);
        let Some(EffectValue::Gradient(stops)) = inst.param("colour_over_life") else {
            panic!("no ramp");
        };
        let read: Vec<(f64, f64)> = stops
            .iter()
            .map(|s| (s.position.value_at(0.0), s.colour[0].value_at(0.0)))
            .collect();
        assert_eq!(read, vec![(30.0, 0.7), (80.0, 0.2)]);
    }
}
