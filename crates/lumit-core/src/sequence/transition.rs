//! What an overlap on a picture row draws as. A dissolve unless the incoming
//! clip names one of the kinds in [`TABLE`], and then that effect runs across
//! the overlap as its [`Sweep`] says: a wipe over the outgoing clip with the
//! incoming clip beneath it, or a hit on both clips that peaks in the middle.
//!
//! The sweep is read off the same gain the dissolve reads ([`super::shown_at`]),
//! so a transition takes its curve from the clip's fade shape as a dissolve does.

use super::{with, Clip};
use crate::anim::{Animation, Property};
use crate::fx::params::Unit;
use crate::fx::presets::PresetValue::{Choice, Colour, Float};
use crate::fx::presets::{EffectPreset, PresetValue};
use crate::model::{EffectInstance, EffectValue};
use crate::time::Rational;
use uuid::Uuid;
use Way::{Over, Steep, Times};

/// Which flyout of the Transition menu a kind sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Wipes,
    Moves,
    Hits,
}

/// Which of the overlap's two clips an instance runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Out,
    In,
}

/// How a Peak row follows the peak, which is 0 at both ends of the overlap and
/// 1 in its middle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Way {
    /// The stored value times the peak.
    Times,
    /// The stored value times the peak cubed: near it only about the middle,
    /// as a flash of light reads.
    Steep,
    /// The stored value over the peak, up to this: a row that does nothing
    /// when it is large.
    Over(f64),
}

/// What the overlap writes into a kind as it runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sweep {
    /// Completion, on the outgoing clip only.
    Out,
    /// The radii of an Iris wipe, on the outgoing clip only.
    Iris,
    /// Completion on both clips, each told its side and whether it fades.
    Both,
    /// These rows on both clips, nothing at either end and as stored in the
    /// middle, while the clips dissolve across `cross` of the overlap about
    /// its middle.
    Peak {
        rows: &'static [(&'static str, Way)],
        cross: f64,
    },
}

/// One kind an overlap can be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kind {
    /// The effect's match name.
    pub name: &'static str,
    pub group: Group,
    pub sweep: Sweep,
    /// Its looks, in menu order, each written for a 1080p picture. The first
    /// is what a fresh one takes. May be empty.
    pub looks: &'static [EffectPreset],
}

const fn look(name: &'static str, values: &'static [(&'static str, PresetValue)]) -> EffectPreset {
    EffectPreset { name, values }
}

const fn wipe(name: &'static str, sweep: Sweep, looks: &'static [EffectPreset]) -> Kind {
    Kind {
        name,
        group: Group::Wipes,
        sweep,
        looks,
    }
}

const fn hit(
    name: &'static str,
    rows: &'static [(&'static str, Way)],
    cross: f64,
    looks: &'static [EffectPreset],
) -> Kind {
    Kind {
        name,
        group: Group::Hits,
        sweep: Sweep::Peak { rows, cross },
        looks,
    }
}

/// Every kind an overlap can be, in the order the menus list them.
pub const TABLE: &[Kind] = &[
    wipe("linear_wipe", Sweep::Out, &[]),
    wipe("radial_wipe", Sweep::Out, &[]),
    wipe("iris_wipe", Sweep::Iris, &[]),
    wipe(
        "card_wipe",
        Sweep::Out,
        &[
            look("Card wipe", &[]),
            look("Flip", &[("rows", Float(1.0)), ("columns", Float(1.0))]),
            look(
                "Shutters",
                &[("rows", Float(1.0)), ("columns", Float(12.0))],
            ),
        ],
    ),
    wipe("venetian_blinds", Sweep::Out, &[]),
    wipe("luma_fade", Sweep::Out, &[]),
    Kind {
        name: "move",
        group: Group::Moves,
        sweep: Sweep::Both,
        looks: crate::fx::effects::move_transition::MOVE_LOOKS,
    },
    Kind {
        name: "slices",
        group: Group::Moves,
        sweep: Sweep::Both,
        looks: crate::fx::effects::slices::SLICES_LOOKS,
    },
    hit(
        "flash",
        &[("intensity", Steep)],
        0.2,
        &[
            // A Manual trigger that is not keyed holds its value, and the
            // overlap sweeps Intensity, which two layers can key as a curve.
            look("Flash", &[("trigger", Float(1.0))]),
            look(
                "Dip to black",
                &[
                    ("trigger", Float(1.0)),
                    ("colour", Colour([0.0, 0.0, 0.0, 1.0])),
                ],
            ),
        ],
    ),
    hit(
        "blur",
        &[("radius", Times)],
        1.0,
        &[look("Blur dissolve", &[("radius", Float(80.0))])],
    ),
    hit(
        "directional_blur",
        &[("length", Times)],
        0.5,
        &[look("Smear", &[("length", Float(600.0))])],
    ),
    hit(
        "radial_blur",
        &[("amount", Times)],
        0.5,
        &[
            look(
                "Zoom blur",
                &[("amount", Float(400.0)), ("radial_type", Choice(1))],
            ),
            look(
                "Spin blur",
                &[("amount", Float(300.0)), ("radial_type", Choice(0))],
            ),
        ],
    ),
    hit(
        "block_glitch",
        &[("intensity", Times)],
        0.15,
        &[
            look(
                "Glitch",
                &[
                    ("intensity", Float(1.0)),
                    ("block_size", Float(64.0)),
                    ("block_amount", Float(50.0)),
                    ("channel_offset", Float(30.0)),
                ],
            ),
            look(
                "Glitch slices",
                &[
                    ("intensity", Float(1.0)),
                    ("block_size", Float(96.0)),
                    ("block_amount", Float(60.0)),
                    ("slice_repeat", Float(100.0)),
                ],
            ),
        ],
    ),
    hit(
        "rgb_split",
        &[("amount", Times)],
        0.3,
        &[look("RGB split", &[("amount", Float(60.0))])],
    ),
    hit(
        "shake",
        &[("amplitude", Times)],
        0.15,
        &[look(
            "Shake",
            &[
                ("amplitude", Float(80.0)),
                ("frequency", Float(14.0)),
                ("motion_blur", PresetValue::Bool(true)),
                ("mb_samples", Float(16.0)),
            ],
        )],
    ),
    hit(
        "mosaic",
        &[
            ("horizontal_blocks", Over(2000.0)),
            ("vertical_blocks", Over(2000.0)),
        ],
        0.2,
        &[look(
            "Pixelate",
            &[
                ("horizontal_blocks", Float(16.0)),
                ("vertical_blocks", Float(9.0)),
            ],
        )],
    ),
    hit(
        "twirl",
        &[("angle", Times)],
        0.3,
        &[look(
            "Twirl",
            &[("angle", Float(360.0)), ("radius", Float(540.0))],
        )],
    ),
    hit(
        "spherize",
        &[("bulge", Times)],
        0.3,
        &[look(
            "Bulge",
            &[("bulge", Float(100.0)), ("radius", Float(900.0))],
        )],
    ),
    hit(
        "turbulent_displace",
        &[("amount", Times)],
        0.4,
        &[look(
            "Warp",
            &[("amount", Float(150.0)), ("size", Float(300.0))],
        )],
    ),
    hit(
        "glow",
        &[("intensity", Times)],
        0.6,
        &[look(
            "Bloom",
            &[
                ("intensity", Float(6.0)),
                ("threshold", Float(0.3)),
                ("radius", Float(120.0)),
            ],
        )],
    ),
    hit(
        "light_leak",
        &[("amount", Times)],
        0.6,
        crate::fx::effects::light_leak::LIGHT_LEAK_LOOKS,
    ),
];

/// The kind `name` is, or `None` for a name that is not one.
#[must_use]
pub fn kind(name: &str) -> Option<&'static Kind> {
    TABLE.iter().find(|k| k.name == name)
}

/// How `name` sweeps. A name that is not a kind sweeps as a wipe.
fn sweep_of(name: &str) -> Sweep {
    kind(name).map_or(Sweep::Out, |k| k.sweep)
}

/// The rows of a kind that the overlap decides for it, which an interface has
/// no reason to show: what the sweep writes, and the Mix, which would leave some
/// of the outgoing clip standing when the overlap ends. A Peak row stays, as
/// the strength in the middle.
#[must_use]
pub fn swept_rows(match_name: &str) -> &'static [&'static str] {
    match sweep_of(match_name) {
        Sweep::Out => &["completion", "mix"],
        Sweep::Iris => &["outer_radius", "inner_radius", "mix"],
        Sweep::Both => &["completion", "side", "fade", "mix"],
        Sweep::Peak { .. } => &["mix"],
    }
}

/// The rows a look leaves as they are, as they say which clip an instance is.
#[must_use]
pub fn kept_rows(match_name: &str) -> &'static [&'static str] {
    match sweep_of(match_name) {
        Sweep::Both => &["completion", "side", "fade"],
        _ => &[],
    }
}

/// A new instance of the kind `name` at `look`, its first with `None`, on a
/// picture `width` by `height`. `None` for a name that is not a kind, or a
/// look it hasn't got.
#[must_use]
pub fn fresh(name: &str, look: Option<usize>, width: f64, height: f64) -> Option<EffectInstance> {
    let kind = kind(name)?;
    let mut inst = crate::fx::builtins::instantiate_for_raster(name, width, height)?;
    let Some(chosen) = kind.looks.get(look.unwrap_or(0)) else {
        return kind.looks.is_empty().then_some(inst);
    };
    let base = inst.clone();
    crate::fx::presets::apply_over(&mut inst, chosen, Rational::ZERO, &base);
    // A look is written for 1080p, so its lengths follow the picture's height.
    let schema = crate::fx::builtins::schema(name)?;
    for (id, value) in chosen.values {
        let px = schema
            .params
            .iter()
            .any(|p| p.id == *id && p.unit == Unit::Px);
        if let (true, Float(v)) = (px, value) {
            put(&mut inst, id, v * height / 1080.0);
        }
    }
    Some(inst)
}

/// For a Peak kind, how much of the incoming clip shows at `u`, as a dissolve
/// across its `cross`. `None` for every other kind, which draws one clip over
/// the other.
#[must_use]
pub fn share(effect: &EffectInstance, u: f64) -> Option<f64> {
    match sweep_of(&effect.effect.match_name) {
        Sweep::Peak { cross, .. } => Some(((u - 0.5) / cross.max(1e-3) + 0.5).clamp(0.0, 1.0)),
        _ => None,
    }
}

/// Whether the incoming clip draws over the outgoing one: a Move where only
/// the incoming clip travels.
#[must_use]
pub fn incoming_over(effect: &EffectInstance) -> bool {
    effect.effect.match_name == "move"
        && matches!(effect.param("moves"), Some(EffectValue::Choice(2)))
}

/// Whether the two clips only ever sit side by side, its rows read at `lt`:
/// always for Slices, and for a Move when both travel at least a frame in a
/// straight line, with nothing round them. Their smeared edges then add up to
/// a whole picture, where one laid over the other would leave a dark seam.
#[must_use]
pub fn side_by_side(effect: &EffectInstance, lt: f64) -> bool {
    let float = |id: &str| match effect.param(id) {
        Some(EffectValue::Float(v)) => v.value_at(lt),
        _ => f64::NAN,
    };
    if effect.effect.match_name == "slices" {
        return true;
    }
    effect.effect.match_name == "move"
        && matches!(effect.param("moves"), Some(EffectValue::Choice(0)))
        && matches!(effect.param("edge"), Some(EffectValue::Choice(0)))
        && float("distance") >= 100.0
        && float("zoom") == 100.0
        && float("stretch") == 100.0
        && float("spin") == 0.0
}

/// `effect` as it draws at `u` through its overlap on the clip at `side`, on a
/// picture `width` by `height`, its own keys read at `lt`. A wipe at 0 has
/// taken nothing away, and at 1 nothing of the outgoing clip is left. A Peak
/// kind is nothing at either end and as stored in the middle.
///
/// An Iris wipe has no Completion to sweep, so its hole grows until the corners
/// of its polygon have passed every corner of the picture.
#[must_use]
pub fn swept(
    effect: &EffectInstance,
    u: f64,
    side: Side,
    width: f64,
    height: f64,
    lt: f64,
) -> EffectInstance {
    // Every bound here is a constant, so `clamp` has no reversed pair to panic on.
    let u = u.clamp(0.0, 1.0);
    let mut out = effect.clone();
    let float = |id: &str, or: f64| {
        effect
            .params
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| match &p.value {
                EffectValue::Float(v) => Some(v.value_at(lt)),
                _ => None,
            })
            .unwrap_or(or)
    };
    match sweep_of(&effect.effect.match_name) {
        Sweep::Out => put(&mut out, "completion", u * 100.0),
        Sweep::Both => {
            put(&mut out, "completion", u * 100.0);
            // Only the upper clip fades, so the two add up to a whole one.
            let upper = (side == Side::In) == incoming_over(effect);
            for p in &mut out.params {
                match (p.id.as_str(), &mut p.value) {
                    ("side", EffectValue::Choice(c)) => *c = u32::from(side == Side::In),
                    ("fade", EffectValue::Bool(b)) => *b = upper,
                    _ => {}
                }
            }
        }
        Sweep::Peak { rows, .. } => {
            let peak = (std::f64::consts::PI * u).sin().powi(2);
            for (id, way) in rows {
                let stored = float(id, 0.0);
                let value = match way {
                    Way::Times => stored * peak,
                    Way::Steep => stored * peak.powi(3),
                    Way::Over(cap) => (stored / peak.max(1e-3)).min(*cap),
                };
                put(&mut out, id, value);
            }
            put(&mut out, "mix", 100.0 * (peak / 0.05).min(1.0));
        }
        Sweep::Iris => iris(&mut out, u, width, height, float),
    }
    out
}

/// An Iris wipe's hole at `u`.
fn iris(
    out: &mut EffectInstance,
    u: f64,
    width: f64,
    height: f64,
    float: impl Fn(&str, f64) -> f64,
) {
    let starred = out
        .params
        .iter()
        .any(|p| p.id == "use_inner_radius" && matches!(p.value, EffectValue::Bool(true)));
    let (cx, cy) = (
        float("centre_x", width / 2.0),
        float("centre_y", height / 2.0),
    );
    let corner = (cx.max(width - cx)).hypot(cy.max(height - cy));
    let corners = float("points", 6.0).max(3.0);
    let inner = float("inner_radius", 165.0);
    let outer = float("outer_radius", 330.0).max(1.0);
    // The narrowest part of the shape has to clear the corner too: a polygon's
    // sides, or a star's pulled-in points.
    let waist = if starred {
        (inner / outer).clamp(0.1, 1.0)
    } else {
        (std::f64::consts::PI / corners).cos().max(0.1)
    };
    let reach = (corner + float("feather", 0.0)) / waist;
    put(out, "outer_radius", u * reach);
    if starred {
        put(out, "inner_radius", u * reach * inner / outer);
    }
}

/// One Float row holding `value` at every moment.
fn put(effect: &mut EffectInstance, id: &str, value: f64) {
    for param in effect.params.iter_mut().filter(|p| p.id == id) {
        if let EffectValue::Float(v) = &mut param.value {
            *v = Property {
                animation: Animation::Static(value),
                extra: serde_json::Map::new(),
            };
        }
    }
}

/// Make the two clips at an edit point overlap by `frames` steps of `frame`,
/// centred on it. On a layer that draws, the overlap is a dissolve, and on an
/// audio-only one a crossfade.
///
/// `outgoing` ends where `incoming` starts, or the two already overlap and
/// the edit point is the middle of that. Each is carried on by half the
/// length into its own source, the outgoing clip's end later and the incoming
/// clip's start earlier, and the odd step goes to the incoming clip. `room`
/// is how far each edge may be carried from where it stands, the outgoing
/// clip's first, with None for no limit ([`Clip::spare`]). A clip with less
/// than its half gives what it has, in whole steps, so the overlap is the
/// longest the two allow. Neither edge passes the far end of the other clip
/// or reaches a third, so two clips at most cover any moment.
///
/// No `frames` trims an overlap back to its middle, and the clips abut again.
///
/// None when the two neither meet nor overlap, when a length was asked for
/// and neither clip has any to give, or when a map cannot be trimmed or
/// carried on.
pub fn dissolve(
    clips: &[Clip],
    outgoing: Uuid,
    incoming: Uuid,
    frames: i64,
    frame: Rational,
    room: (Option<Rational>, Option<Rational>),
) -> Option<Vec<Clip>> {
    let out = clips.iter().find(|c| c.id == outgoing)?;
    let inc = clips.iter().find(|c| c.id == incoming)?;
    let (start, end) = (inc.place_start, out.place_end());
    if frames < 0
        || frame <= Rational::ZERO
        || out.place_start > start
        || start > end
        || end > inc.place_end()
    {
        return None;
    }
    // The whole steps in a length, and the length of that many steps.
    let steps = |length: Rational| {
        let count = length.checked_div(frame).ok()?;
        Some(count.num().div_euclid(count.den()))
    };
    let span = |steps: i64| frame.checked_mul(Rational::new(steps, 1).ok()?).ok();
    let overlap = steps(end.checked_sub(start).ok()?)?;
    let point = end.checked_sub(span(overlap / 2)?).ok()?;

    // How far each edge may go: to the other clip's far end, to a third
    // clip, and as far as its own source runs.
    let others = || {
        clips
            .iter()
            .filter(|c| c.id != outgoing && c.id != incoming)
    };
    let later = others().map(|c| c.place_start).filter(|s| *s >= point);
    let mut latest = later.chain([inc.place_end()]).min()?;
    let earlier = others().map(Clip::place_end).filter(|e| *e <= point);
    let mut earliest = earlier.chain([out.place_start]).max()?;
    if let Some(room) = room.0 {
        latest = latest.min(end.checked_add(room).ok()?);
    }
    if let Some(room) = room.1 {
        earliest = earliest.max(start.checked_sub(room).ok()?);
    }
    let after = (frames / 2).min(steps(latest.checked_sub(point).ok()?)?);
    let before = (frames - frames / 2).min(steps(point.checked_sub(earliest).ok()?)?);
    if frames > 0 && after + before == 0 {
        return None;
    }
    let new_end = point.checked_add(span(after)?).ok()?;
    let new_start = point.checked_sub(span(before)?).ok()?;
    let out = match new_end.cmp(&end) {
        std::cmp::Ordering::Less => out.trim_end(new_end)?,
        std::cmp::Ordering::Greater => out.extend_end(new_end)?,
        std::cmp::Ordering::Equal => out.clone(),
    };
    let inc = match new_start.cmp(&start) {
        std::cmp::Ordering::Less => inc.extend_start(new_start)?,
        std::cmp::Ordering::Greater => inc.trim_start(new_start)?,
        std::cmp::Ordering::Equal => inc.clone(),
    };
    Some(with(clips, &[out, inc]))
}
