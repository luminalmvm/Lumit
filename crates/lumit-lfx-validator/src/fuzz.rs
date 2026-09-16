//! The parameter edges a plugin is driven at, and the seed that makes the run
//! repeatable (docs/impl/lfx.md §9).
//!
//! # In plain terms
//!
//! A plugin is written against the numbers its author expected. This is the
//! list of numbers they did not: the ends of their own declared range, one step
//! past each end, and the three values that are not numbers at all - NaN and
//! both infinities. A control that cannot take its own declared maximum is a
//! control the panel can drive into a black frame, and nothing but a pass like
//! this finds it before a user does.
//!
//! **The edges come first and the random value comes last**, which is the whole
//! of the seeding. Nine tenths of what this suite finds is at a bound, so the
//! bounds are walked every run whatever the seed; the seed decides one further
//! value inside the range per control, so a long soak covers ground a fixed
//! list never would. `LUMIT_LFX_FUZZ_SEED` reproduces a run exactly: the
//! generator is written out here rather than taken from a crate, because a
//! seed that reproduces a run only until a dependency is bumped reproduces
//! nothing.

use lumit_lfx::describe::{Declaration, Declared};
use lumit_lfx::ipc::proto::ParamValue;

/// SplitMix64, which is eight lines and has no dependency.
///
/// Not a distribution anybody should sample a simulation from; what it has to
/// be is the **same** sequence on every machine and every release, which a
/// library's own generator is explicitly not required to be.
#[derive(Clone, Copy, Debug)]
pub struct Seeded(u64);

impl Seeded {
    /// Start from a seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The seed mixed with a name, so two controls of one plugin do not walk
    /// the same sequence and a plugin renamed is a run re-seeded.
    #[must_use]
    pub fn for_name(seed: u64, name: &str) -> Self {
        let mut mixed = seed;
        for byte in name.as_bytes() {
            mixed ^= u64::from(*byte);
            mixed = mixed.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(mixed)
    }

    /// The next number.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `[0, 1)`, from the top fifty-three bits.
    pub fn unit(&mut self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let scaled = (self.next_u64() >> 11) as f64;
        scaled / ((1_u64 << 53) as f64)
    }
}

/// One value to drive a control at, and whether the plugin's own declaration
/// admits it.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// The value, ready for [`lumit_lfx::Broker::set_values`].
    pub value: ParamValue,
    /// How the table names it.
    pub label: String,
    /// Whether the declaration says this value is legal. A picture that is not
    /// numbers is a refusal here and a report line outside it: a plugin handed
    /// a NaN may answer with one, and a plugin handed its own declared maximum
    /// may not.
    pub declared: bool,
}

/// How many edges one control contributes at most, so a bundle of twelve
/// effects is a pass measured in seconds rather than minutes.
pub const EDGES_PER_CONTROL: usize = 8;

/// Every edge worth driving one declaration at, in a fixed order.
///
/// Kinds with no range to walk - a switch, a button, a seed, a file - carry
/// one entry or none: there is no edge of a `bool` that is not one of its two
/// values, and a run that drove eight of them would be a run spending its time
/// where nothing is found.
#[must_use]
pub fn edges_of(declaration: &Declaration, seed: u64) -> Vec<Edge> {
    let mut rng = Seeded::for_name(seed, &declaration.id);
    let mut out = Vec::new();
    match &declaration.kind {
        Declared::Float { slider, hard, .. } => {
            let (lo, hi) = (hard.0.unwrap_or(slider.0), hard.1.unwrap_or(slider.1));
            push_real(
                &mut out,
                &ParamValue::Float,
                lo,
                hi,
                hard.0.is_some(),
                hard.1.is_some(),
                &mut rng,
            );
        }
        Declared::Slider { range, .. } => {
            push_real(
                &mut out,
                &ParamValue::Slider,
                range.0,
                range.1,
                true,
                true,
                &mut rng,
            );
        }
        Declared::Angle { default, dial_step } => {
            // Deliberately unbounded (§2.3), so the edges are the ones a dial
            // can actually be spun to rather than a declared range.
            for (label, number) in [
                ("default", *default),
                ("-360", -360.0),
                ("360", 360.0),
                ("a step", *dial_step),
                ("NaN", f64::NAN),
                ("inf", f64::INFINITY),
            ] {
                out.push(Edge {
                    value: ParamValue::Angle(number),
                    label: label.to_owned(),
                    declared: number.is_finite(),
                });
            }
        }
        Declared::Int {
            slider,
            hard,
            default,
        } => {
            let (lo, hi) = (hard.0.unwrap_or(slider.0), hard.1.unwrap_or(slider.1));
            for (label, number, declared) in [
                ("default", *default, true),
                ("the low end", lo, true),
                ("the high end", hi, true),
                ("one below", lo.saturating_sub(1), hard.0.is_none()),
                ("one above", hi.saturating_add(1), hard.1.is_none()),
                ("i64::MIN", i64::MIN, false),
                ("i64::MAX", i64::MAX, false),
            ] {
                out.push(Edge {
                    value: ParamValue::Int(number),
                    label: label.to_owned(),
                    declared,
                });
            }
        }
        Declared::Bool { default } => {
            for state in [*default, !*default] {
                out.push(Edge {
                    value: ParamValue::Bool(state),
                    label: state.to_string(),
                    declared: true,
                });
            }
        }
        Declared::Choice { options, .. } => {
            let count = u32::try_from(options.len()).unwrap_or(u32::MAX);
            for (label, index, declared) in [
                ("the first option", 0, count > 0),
                ("the last option", count.saturating_sub(1), count > 0),
                ("one past the last", count, false),
                ("u32::MAX", u32::MAX, false),
            ] {
                out.push(Edge {
                    value: ParamValue::Choice(index),
                    label: label.to_owned(),
                    declared,
                });
            }
        }
        Declared::Colour { range, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let (lo, hi) = (range.0 as f32, range.1 as f32);
            for (label, channel, declared) in [
                ("the low end", lo, true),
                ("the high end", hi, true),
                ("NaN", f32::NAN, false),
                ("-inf", f32::NEG_INFINITY, false),
            ] {
                out.push(Edge {
                    value: ParamValue::Colour([channel; 4]),
                    label: label.to_owned(),
                    declared,
                });
            }
        }
        Declared::Seed => {
            for (label, number) in [("nought", 0), ("i64::MAX", i64::MAX)] {
                out.push(Edge {
                    value: ParamValue::Seed(number),
                    label: label.to_owned(),
                    declared: true,
                });
            }
        }
        Declared::Point2 { slider, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let (lo, hi) = (slider.0 as f32, slider.1 as f32);
            for (label, point, declared) in [
                ("the low corner", [lo, lo], true),
                ("the high corner", [hi, hi], true),
                ("NaN", [f32::NAN, f32::NAN], false),
            ] {
                out.push(Edge {
                    value: ParamValue::Point2(point),
                    label: label.to_owned(),
                    declared,
                });
            }
        }
        Declared::Point3 { slider, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let (lo, hi) = (slider.0 as f32, slider.1 as f32);
            for (label, point, declared) in [
                ("the low corner", [lo, lo, lo], true),
                ("the high corner", [hi, hi, hi], true),
                ("inf", [f32::INFINITY; 3], false),
            ] {
                out.push(Edge {
                    value: ParamValue::Point3(point),
                    label: label.to_owned(),
                    declared,
                });
            }
        }
        Declared::Curve { default } => {
            out.push(Edge {
                value: ParamValue::Curve(default.clone()),
                label: "the declared shape".to_owned(),
                declared: true,
            });
            out.push(Edge {
                value: ParamValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]]),
                label: "a straight line".to_owned(),
                declared: true,
            });
        }
        // A file row's payload is `None` until the render pass's generic file aux lands,
        // so there is exactly one value it can be driven at today.
        Declared::File { .. } => {
            out.push(Edge {
                value: ParamValue::File(None),
                label: "nothing loaded".to_owned(),
                declared: true,
            });
        }
        // A button carries no value and takes no element of the array.
        Declared::Action => {}
    }
    out.truncate(EDGES_PER_CONTROL);
    out
}

/// The six edges of a real-valued range, plus one seeded value inside it.
///
/// `one_below` and `one_above` are one ULP outside, which is the number the
/// note asks for: a plugin that clamps and a plugin that reads past its own
/// table are one ULP apart, and a step of any other size would be testing the
/// step.
fn push_real(
    out: &mut Vec<Edge>,
    tag: &dyn Fn(f64) -> ParamValue,
    lo: f64,
    hi: f64,
    lo_is_hard: bool,
    hi_is_hard: bool,
    rng: &mut Seeded,
) {
    let inside = lo + rng.unit() * (hi - lo);
    for (label, number, declared) in [
        ("the low end", lo, true),
        ("the high end", hi, true),
        ("one ULP below", one_below(lo), !lo_is_hard),
        ("one ULP above", one_above(hi), !hi_is_hard),
        ("NaN", f64::NAN, false),
        ("inf", f64::INFINITY, false),
        ("-inf", f64::NEG_INFINITY, false),
        ("a seeded value inside", inside, inside.is_finite()),
    ] {
        out.push(Edge {
            value: tag(number),
            label: label.to_owned(),
            declared,
        });
    }
}

/// The next representable double below `value`, or the value itself where
/// there is not one.
///
/// **Nought is the case that has to be written out.** Stepping the bit pattern
/// of `+0.0` upwards lands on the smallest subnormal *above* nought, which is
/// the wrong side of the edge and would mark a value inside the declared range
/// as one outside it - a fuzz pass that then demanded a finite picture at a
/// value it had called illegal.
fn one_below(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    if value == 0.0 {
        return -f64::from_bits(1);
    }
    let bits = value.to_bits();
    let next = if value > 0.0 {
        bits.wrapping_sub(1)
    } else {
        bits.wrapping_add(1)
    };
    let stepped = f64::from_bits(next);
    if stepped.is_finite() {
        stepped
    } else {
        value
    }
}

/// The next representable double above `value`, or the value itself where
/// there is not one.
fn one_above(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    if value == 0.0 {
        return f64::from_bits(1);
    }
    let bits = value.to_bits();
    let next = if value < 0.0 {
        bits.wrapping_sub(1)
    } else {
        bits.wrapping_add(1)
    };
    let stepped = f64::from_bits(next);
    if stepped.is_finite() {
        stepped
    } else {
        value
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{edges_of, Seeded, EDGES_PER_CONTROL};
    use lumit_core::fx::Unit;
    use lumit_lfx::describe::{Declaration, Declared};
    use lumit_lfx::ipc::proto::ParamValue;

    /// A declaration to hang the cases on.
    fn a_slider() -> Declaration {
        Declaration {
            id: "mix".to_owned(),
            label: "Mix".to_owned(),
            unit: Unit::Percent,
            flags: 0,
            kind: Declared::Slider {
                default: 100.0,
                range: (0.0, 100.0),
                log: false,
            },
        }
    }

    /// The same seed is the same run, and a different one is a different run -
    /// which is the whole of what `LUMIT_LFX_FUZZ_SEED` promises.
    #[test]
    fn one_seed_is_one_run() {
        // Compared as text rather than by `PartialEq`, because a NaN edge is
        // one of the values this list is for and a NaN is not equal to itself.
        let walked = |seed: u64| format!("{:?}", edges_of(&a_slider(), seed));
        assert_eq!(walked(17), walked(17), "a seed must reproduce");
        assert_ne!(
            walked(17),
            walked(18),
            "two seeds must not walk the same ground"
        );

        let mut rng = Seeded::new(4);
        let drawn: Vec<f64> = (0..8).map(|_| rng.unit()).collect();
        assert!(
            drawn.iter().all(|value| (0.0..1.0).contains(value)),
            "every draw is in [0, 1): {drawn:?}"
        );
        assert!(
            drawn.windows(2).any(|pair| pair[0] != pair[1]),
            "the generator stood still: {drawn:?}"
        );
    }

    /// The declared ends are walked whatever the seed, and a value the
    /// declaration does not admit is marked as one - which is what lets the
    /// suite ask for a finite picture at a legal value and not at an illegal
    /// one.
    #[test]
    fn the_declared_ends_are_walked_and_the_illegal_ones_are_marked() {
        let edges = edges_of(&a_slider(), 99);
        assert!(edges.len() <= EDGES_PER_CONTROL, "the run is bounded");
        let ends: Vec<&str> = edges.iter().map(|edge| edge.label.as_str()).collect();
        assert!(ends.contains(&"the low end"), "{ends:?}");
        assert!(ends.contains(&"the high end"), "{ends:?}");
        assert!(ends.contains(&"NaN"), "{ends:?}");
        for edge in &edges {
            let legal = match edge.value {
                ParamValue::Slider(number) => number.is_finite() && (0.0..=100.0).contains(&number),
                _ => false,
            };
            assert_eq!(
                edge.declared, legal,
                "{} is marked {} and is {}",
                edge.label, edge.declared, legal
            );
        }
    }

    /// A button contributes no value and takes no element of the array, so a
    /// fuzz pass over it drives nothing rather than driving a nought.
    #[test]
    fn a_button_has_no_edge_to_drive() {
        let action = Declaration {
            id: "reset".to_owned(),
            label: "Reset".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Action,
        };
        assert!(edges_of(&action, 1).is_empty());
    }
}
