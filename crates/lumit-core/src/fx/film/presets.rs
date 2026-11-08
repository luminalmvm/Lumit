//! The looks Film ships with, listed in the Effects & presets panel as the
//! Film group. Halation is one of them, so a search for it finds Film.

use crate::anim::Property;
use crate::model::{EffectInstance, EffectValue};

/// One control's value in a look.
enum Set {
    Number(f32),
    On(bool),
    Pick(u32),
}

use Set::{Number, On, Pick};

/// Film's presets: each a name and a Film set up as the look says.
pub fn presets() -> Vec<(&'static str, EffectInstance)> {
    let looks: [(&str, &[(&str, Set)]); 9] = [
        (
            "Halation",
            &[
                ("colour", On(false)),
                ("grain", On(false)),
                ("backing", Pick(3)),
            ],
        ),
        ("Cinema 500T", &[]),
        ("Cinema 250D", &[("stock", Pick(1))]),
        (
            "Night lights",
            &[
                ("backing", Pick(3)),
                ("halation_amount", Number(150.0)),
                ("bloom", On(true)),
                ("bloom_amount", Number(15.0)),
            ],
        ),
        (
            "Super 16",
            &[
                ("stock", Pick(1)),
                ("format", Pick(2)),
                ("weave", On(true)),
                ("weave_amount", Number(50.0)),
            ],
        ),
        (
            "Home movie",
            &[
                ("stock", Pick(14)),
                ("format", Pick(0)),
                ("backing", Pick(2)),
                ("weave", On(true)),
                ("weave_amount", Number(200.0)),
                ("flicker", On(true)),
            ],
        ),
        (
            "Portrait",
            &[
                ("stock", Pick(5)),
                ("format", Pick(4)),
                ("print_stock", Pick(2)),
            ],
        ),
        ("Slide", &[("stock", Pick(16)), ("format", Pick(4))]),
        (
            "Film grain",
            &[
                ("colour", On(false)),
                ("halation", On(false)),
                ("format", Pick(2)),
            ],
        ),
    ];
    looks
        .into_iter()
        .filter_map(|(name, sets)| {
            let mut inst = crate::fx::builtins::instantiate("film")?;
            for (id, value) in sets {
                if let Some(param) = inst.params.iter_mut().find(|p| p.id == *id) {
                    param.value = match value {
                        Number(v) => EffectValue::Float(Property::fixed(f64::from(*v))),
                        On(v) => EffectValue::Bool(*v),
                        Pick(v) => EffectValue::Choice(*v),
                    };
                }
            }
            Some((name, inst))
        })
        .collect()
}
