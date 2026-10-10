//! What After Effects adds to the language: the composition, its layers,
//! their effects and properties, and the helpers every expression reaches
//! for (`wiggle`, `linear`, `seedRandom`, `loopOut`).
//!
//! Each of them is looked up by name when the expression asks, against the
//! document the evaluation was handed. Nothing here is built ahead of time, so
//! an expression that reads one slider pays for one slider.

use std::rc::Rc;
use std::sync::Arc;

use uuid::Uuid;

use super::interp::{fail, to_number, Interp, Res, Value};
use super::parse::Binary;
use super::Slot;
use crate::anim::{Animation, Property};
use crate::expression::ExpressionContext;
use crate::fx::noise;
use crate::model::{Composition, EffectInstance, EffectValue, Layer, ProjectItem};

/// The values an expression can name with no dot in front.
const VALUES: [&str; 28] = [
    "time",
    "value",
    "thisComp",
    "thisLayer",
    "thisProperty",
    "Math",
    "undefined",
    "NaN",
    "Infinity",
    "index",
    "inPoint",
    "outPoint",
    "startTime",
    "width",
    "height",
    "name",
    "hasParent",
    "parent",
    "transform",
    "position",
    "anchorPoint",
    "scale",
    "rotation",
    "opacity",
    "numKeys",
    "velocity",
    "speed",
    "colorDepth",
];

/// The functions an expression can call with no dot in front.
const FUNCTIONS: [&str; 43] = [
    "wiggle",
    "seedRandom",
    "random",
    "gaussRandom",
    "noise",
    "linear",
    "ease",
    "easeIn",
    "easeOut",
    "clamp",
    "length",
    "add",
    "sub",
    "mul",
    "div",
    "normalize",
    "dot",
    "cross",
    "degreesToRadians",
    "radiansToDegrees",
    "timeToFrames",
    "framesToTime",
    "posterizeTime",
    "valueAtTime",
    "velocityAtTime",
    "speedAtTime",
    "loopIn",
    "loopOut",
    "loopInDuration",
    "loopOutDuration",
    "key",
    "nearestKey",
    "comp",
    "effect",
    "rgbToHsl",
    "hslToRgb",
    "parseFloat",
    "parseInt",
    "isNaN",
    "isFinite",
    "Number",
    "String",
    "Boolean",
];

/// What a property answers to besides its value.
const PROPERTY_METHODS: [&str; 12] = [
    "valueAtTime",
    "velocityAtTime",
    "speedAtTime",
    "key",
    "nearestKey",
    "wiggle",
    "temporalWiggle",
    "loopIn",
    "loopOut",
    "loopInDuration",
    "loopOutDuration",
    "smooth",
];

/// Whether `name` is something an expression can read without declaring it.
pub(super) fn knows(name: &str) -> bool {
    VALUES.contains(&name) || FUNCTIONS.contains(&name)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tr {
    Position,
    Anchor,
    Scale,
    Rotation,
    Opacity,
}

pub(super) enum PropRef {
    /// The property the expression is written on.
    Own,
    Transform {
        comp: Uuid,
        layer: Uuid,
        which: Tr,
    },
    /// An effect's row. `partner` is the other half of a point that Lumit
    /// keeps as two rows, so `effect("Transform")("Position")` reads as the
    /// pair After Effects has there.
    Param {
        comp: Uuid,
        layer: Uuid,
        effect: Uuid,
        param: usize,
        partner: Option<usize>,
    },
}

/// The things an expression holds that are not JavaScript's own.
#[derive(Clone)]
pub(super) enum Host {
    /// What a bare function name is reached on.
    Global,
    Math,
    Comp(Uuid),
    Layer(Uuid, Uuid),
    Transform(Uuid, Uuid),
    Effect(Uuid, Uuid, Uuid),
    Prop(Rc<PropRef>),
}

/// What an evaluation knows about where it is running.
pub(super) struct Ae<'a> {
    pub context: &'a ExpressionContext,
    pub slot: Slot,
    /// Extra numbers in scope by name: an Expression box's inputs.
    pub vars: &'a [(&'a str, f64)],
    /// The time the expression is reading at. `posterizeTime` moves it.
    pub time: f64,
    seed: u64,
    timeless: bool,
    draws: u64,
}

/// One round of SplitMix64: a number in, an unrelated number out, the same on
/// every machine.
fn mix(z: u64) -> u64 {
    let z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fold(id: Uuid) -> u64 {
    let id = id.as_u128();
    (id as u64) ^ ((id >> 64) as u64)
}

impl<'a> Ae<'a> {
    pub(super) fn new(
        context: &'a ExpressionContext,
        slot: Slot,
        vars: &'a [(&'a str, f64)],
    ) -> Self {
        let mut ae = Ae {
            context,
            slot,
            vars,
            time: context.comp_time,
            seed: 0,
            timeless: false,
            draws: 0,
        };
        ae.seed = ae.own_seed();
        ae
    }

    /// The seed this property starts from: its layer and the number it was
    /// given when the expression was put on it. Two properties with the same
    /// text then wander differently, as they do in After Effects.
    fn own_seed(&self) -> u64 {
        mix(self.context.layer.map_or(0, fold) ^ u64::from(self.slot.seed))
    }

    /// The next random number in `0..1`.
    ///
    /// Nothing here reads a clock or keeps state between evaluations, so the
    /// same property at the same time draws the same numbers in the preview,
    /// in the export, and on the frame cache's key.
    pub(super) fn random(&mut self) -> f64 {
        let moment = if self.timeless {
            0
        } else {
            (self.time * 1e5).round() as i64 as u64
        };
        let r = mix(mix(self.seed ^ mix(moment)).wrapping_add(self.draws));
        self.draws += 1;
        (r >> 11) as f64 / (1u64 << 53) as f64
    }

    fn comp(&self, id: Uuid) -> Option<&'a Composition> {
        self.context.document.comp(id)
    }

    fn layer(&self, comp: Uuid, layer: Uuid) -> Option<&'a Layer> {
        self.comp(comp)?.layers.iter().find(|l| l.id == layer)
    }

    fn effect(&self, comp: Uuid, layer: Uuid, effect: Uuid) -> Option<&'a EffectInstance> {
        self.layer(comp, layer)?
            .effects
            .iter()
            .find(|e| e.id == effect)
    }

    /// One property of `layer` at comp time `t`.
    fn sample(&self, comp: Uuid, layer: &Layer, property: &Property, t: f64) -> f64 {
        let local = t - layer.start_offset.0.to_f64();
        match &property.animation {
            // Only a property that is itself an expression needs a context
            // of its own, a level deeper so two that read each other stop.
            Animation::Expression(_) => property.value_at_with_context(
                local,
                Arc::new(ExpressionContext {
                    document: self.context.document.clone(),
                    comp: Some(comp),
                    layer: Some(layer.id),
                    comp_time: t,
                    current_depth: self.context.current_depth + 1,
                    inputs: self.context.inputs.clone(),
                }),
            ),
            _ => property.value_at(local),
        }
    }

    /// The properties behind a reference, and the layer they are on.
    fn parts(&self, prop: &PropRef) -> Option<(Uuid, &'a Layer, Parts<'a>)> {
        match prop {
            PropRef::Own => None,
            PropRef::Transform { comp, layer, which } => {
                let found = self.layer(*comp, *layer)?;
                let tr = &found.transform;
                let parts = match which {
                    Tr::Position => Parts::two(&tr.position_x, &tr.position_y),
                    Tr::Anchor => Parts::two(&tr.anchor_x, &tr.anchor_y),
                    Tr::Scale => Parts::two(&tr.scale_x, &tr.scale_y),
                    Tr::Rotation => Parts::one(&tr.rotation),
                    Tr::Opacity => Parts::one(&tr.opacity),
                };
                Some((*comp, found, parts))
            }
            PropRef::Param {
                comp,
                layer,
                effect,
                param,
                partner,
            } => {
                let found = self.layer(*comp, *layer)?;
                let inst = found.effects.iter().find(|e| e.id == *effect)?;
                let float = |i: usize| match inst.params.get(i).map(|p| &p.value) {
                    Some(EffectValue::Float(p)) => Some(p),
                    _ => None,
                };
                let parts = match &inst.params.get(*param)?.value {
                    EffectValue::Float(p) => match partner.and_then(float) {
                        Some(other) => Parts::two(p, other),
                        None => Parts::one(p),
                    },
                    EffectValue::Point(x, y) => Parts::two(x, y),
                    EffectValue::Colour(c) => Parts {
                        props: [c.first(), c.get(1), c.get(2), c.get(3)],
                        fixed: None,
                    },
                    EffectValue::Bool(on) => Parts::fixed(f64::from(u8::from(*on))),
                    // A menu counts from one in After Effects, and an
                    // expression that reads one compares against that.
                    EffectValue::Choice(n) => Parts::fixed(f64::from(*n) + 1.0),
                    EffectValue::Seed(n) => Parts::fixed(f64::from(*n)),
                    _ => return None,
                };
                Some((*comp, found, parts))
            }
        }
    }

    /// A property's numbers at comp time `t`.
    fn read(&self, prop: &PropRef, t: f64) -> Res<Vec<f64>> {
        if matches!(prop, PropRef::Own) {
            let own = self.slot.values();
            if own.is_empty() {
                return fail("this property's own value is not known here");
            }
            return Ok(own.to_vec());
        }
        let Some((comp, layer, parts)) = self.parts(prop) else {
            return fail("a property that is no longer there");
        };
        Ok(match parts.fixed {
            Some(fixed) => vec![fixed],
            None => parts
                .props
                .iter()
                .flatten()
                .map(|p| self.sample(comp, layer, p, t))
                .collect(),
        })
    }

    /// When a property's keyframes fall, in comp time.
    fn keys(&self, prop: &PropRef) -> Vec<f64> {
        let Some((_, layer, parts)) = self.parts(prop) else {
            return Vec::new();
        };
        let offset = layer.start_offset.0.to_f64();
        parts
            .props
            .iter()
            .flatten()
            .filter_map(|p| match &p.animation {
                Animation::Keyframed(keys) => Some(keys),
                _ => None,
            })
            .max_by_key(|keys| keys.len())
            .map(|keys| keys.iter().map(|k| k.time.to_f64() + offset).collect())
            .unwrap_or_default()
    }

    /// What tells one property's wander from another's.
    fn salt(&self, prop: &PropRef) -> u64 {
        match prop {
            PropRef::Own => 0,
            PropRef::Transform { layer, which, .. } => mix(fold(*layer) ^ (*which as u64 + 1)),
            PropRef::Param { effect, param, .. } => mix(fold(*effect) ^ (*param as u64 + 101)),
        }
    }
}

/// Up to four properties (a colour's channels), or one number that does not
/// animate (a checkbox, a menu).
struct Parts<'a> {
    props: [Option<&'a Property>; 4],
    fixed: Option<f64>,
}

impl<'a> Parts<'a> {
    fn one(p: &'a Property) -> Self {
        Parts {
            props: [Some(p), None, None, None],
            fixed: None,
        }
    }

    fn two(x: &'a Property, y: &'a Property) -> Self {
        Parts {
            props: [Some(x), Some(y), None, None],
            fixed: None,
        }
    }

    fn fixed(value: f64) -> Self {
        Parts {
            props: [None; 4],
            fixed: Some(value),
        }
    }
}

/// The name an effect goes by: the user's own for it, or the effect's.
fn effect_name(inst: &EffectInstance) -> &str {
    inst.custom_name.as_deref().unwrap_or_else(|| {
        crate::fx::schema(&inst.effect.match_name).map_or(&inst.effect.match_name, |s| s.label)
    })
}

/// The row of `inst` that goes by `name`, and its other half when the name is
/// a point's.
fn find_param(inst: &EffectInstance, name: &str) -> Option<(usize, Option<usize>)> {
    let at = |id: &str| inst.params.iter().position(|p| p.id == id);
    let name = name.trim();
    // A set of Custom controls is asked for a control by name many times a
    // frame, so its list is read as it is stored and no rows are built.
    if let Some(i) = crate::fx::effects::custom_controls::id_of(inst, name).and_then(at) {
        return Some((i, None));
    }
    if let Some(def) = crate::fx::def(&inst.effect.match_name) {
        let schema = def.schema();
        let row = schema
            .params
            .iter()
            .chain(def.derived(inst))
            .find(|row| row.label.eq_ignore_ascii_case(name));
        if let Some(i) = row.and_then(|row| at(row.id)) {
            return Some((i, None));
        }
        // "Position" names the pair Lumit keeps as `position_x` and
        // `position_y`, and After Effects calls the anchor "Anchor Point".
        let stem = name.to_ascii_lowercase().replace(' ', "_");
        let pair = schema
            .pairs()
            .find(|pair| pair.stem == stem || (pair.stem == "anchor" && stem == "anchor_point"));
        if let Some(pair) = pair {
            if let (Some(x), Some(y)) = (at(pair.x), at(pair.y)) {
                return Some((x, Some(y)));
            }
        }
    }
    at(name)
        .or_else(|| {
            // An effect Lumit does not know keeps the names its rows had.
            inst.params.iter().position(|p| {
                p.extra
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kept| kept.eq_ignore_ascii_case(name))
            })
        })
        .map(|i| (i, None))
}

fn smooth(u: f64) -> f64 {
    u * u * (3.0 - 2.0 * u)
}

/// Flat at the start, straight by the end.
fn ease_in(u: f64) -> f64 {
    1.5 * u * u - 0.5 * u * u * u
}

fn ease_out(u: f64) -> f64 {
    1.0 - ease_in(1.0 - u)
}

/// A wander through `-1..1` that passes a new random height at every whole
/// `x`. Worked in `f64` so an hour into a comp is as smooth as the first
/// second.
fn wander(seed: u32, channel: u32, x: f64) -> f64 {
    let cell = x.floor();
    let i = cell as i64 as i32;
    let a = f64::from(noise::hash01(seed, channel, i, 0, 0));
    let b = f64::from(noise::hash01(seed, channel, i.wrapping_add(1), 0, 0));
    (a + (b - a) * smooth(x - cell)) * 2.0 - 1.0
}

fn hue_to_rgb(p: f64, q: f64, t: f64) -> f64 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

impl<'a> Interp<'a> {
    fn host_prop(prop: PropRef) -> Value {
        Value::Host(Host::Prop(Rc::new(prop)))
    }

    fn this_layer(&self) -> Res<(Uuid, Uuid)> {
        match self.ae.context.comp.zip(self.ae.context.layer) {
            Some(ids) => Ok(ids),
            None => fail("there is no layer here"),
        }
    }

    /// A name with nothing in front of it that no variable answers to.
    pub(super) fn global(&mut self, name: &str) -> Res<Option<Value>> {
        if let Some((_, value)) = self.ae.vars.iter().find(|(n, _)| *n == name) {
            return Ok(Some(Value::Num(*value)));
        }
        if FUNCTIONS.contains(&name) {
            return Ok(Some(Value::Bound(Rc::new((
                Value::Host(Host::Global),
                Rc::from(name),
            )))));
        }
        if !VALUES.contains(&name) {
            return Ok(None);
        }
        Ok(Some(match name {
            "time" => Value::Num(self.ae.time),
            "value" => self.prop_value(&PropRef::Own)?,
            "thisComp" => match self.ae.context.comp {
                Some(comp) => Value::Host(Host::Comp(comp)),
                None => return fail("there is no composition here"),
            },
            "thisLayer" => {
                let (comp, layer) = self.this_layer()?;
                Value::Host(Host::Layer(comp, layer))
            }
            "thisProperty" => Self::host_prop(PropRef::Own),
            "Math" => Value::Host(Host::Math),
            "undefined" => Value::Undefined,
            "NaN" => Value::Num(f64::NAN),
            "Infinity" => Value::Num(f64::INFINITY),
            "colorDepth" => Value::Num(32.0),
            "numKeys" | "velocity" | "speed" => {
                let own = Host::Prop(Rc::new(PropRef::Own));
                return self.host_member(&own, name);
            }
            // Everything else is the layer's own: `inPoint` is
            // `thisLayer.inPoint`.
            _ => {
                let (comp, layer) = self.this_layer()?;
                return self.host_member(&Host::Layer(comp, layer), name);
            }
        }))
    }

    /// `host.name`, or `None` when the name is one of its functions.
    pub(super) fn host_member(&mut self, host: &Host, name: &str) -> Res<Option<Value>> {
        let ae = &self.ae;
        Ok(Some(match host {
            Host::Global => return Ok(None),
            Host::Math => Value::Num(match name {
                "PI" => std::f64::consts::PI,
                "E" => std::f64::consts::E,
                "LN2" => std::f64::consts::LN_2,
                "LN10" => std::f64::consts::LN_10,
                "LOG2E" => std::f64::consts::LOG2_E,
                "LOG10E" => std::f64::consts::LOG10_E,
                "SQRT2" => std::f64::consts::SQRT_2,
                "SQRT1_2" => std::f64::consts::FRAC_1_SQRT_2,
                _ => return Ok(None),
            }),
            Host::Comp(id) => {
                let Some(comp) = ae.comp(*id) else {
                    return fail("a composition that is no longer there");
                };
                match name {
                    "width" => Value::Num(f64::from(comp.width)),
                    "height" => Value::Num(f64::from(comp.height)),
                    "duration" => Value::Num(comp.duration.0.to_f64()),
                    "frameDuration" => Value::Num(1.0 / comp.frame_rate.fps()),
                    "name" => Value::Str(Rc::from(comp.name.as_str())),
                    "numLayers" => Value::Num(comp.layers.len() as f64),
                    "pixelAspect" => Value::Num(1.0),
                    "displayStartTime" => Value::Num(0.0),
                    _ => return Ok(None),
                }
            }
            Host::Layer(comp_id, layer_id) => {
                let (Some(comp), Some(layer)) = (ae.comp(*comp_id), ae.layer(*comp_id, *layer_id))
                else {
                    return fail("a layer that is no longer there");
                };
                let transform = |which| {
                    Self::host_prop(PropRef::Transform {
                        comp: *comp_id,
                        layer: *layer_id,
                        which,
                    })
                };
                match name {
                    "name" => Value::Str(Rc::from(layer.name.as_str())),
                    // After Effects counts layers from one, top first.
                    "index" => Value::Num(
                        comp.layers
                            .iter()
                            .position(|l| l.id == layer.id)
                            .map_or(0.0, |i| i as f64 + 1.0),
                    ),
                    "inPoint" => Value::Num(layer.in_point.0.to_f64()),
                    "outPoint" => Value::Num(layer.out_point.0.to_f64()),
                    "startTime" => Value::Num(layer.start_offset.0.to_f64()),
                    // The comp's size stands in for the layer's own, which is
                    // right for a solid and for footage that fills the frame.
                    "width" => Value::Num(f64::from(comp.width)),
                    "height" => Value::Num(f64::from(comp.height)),
                    "hasParent" => Value::Bool(layer.parent.is_some()),
                    "parent" => match layer.parent {
                        Some(parent) => Value::Host(Host::Layer(*comp_id, parent)),
                        None => Value::Null,
                    },
                    "active" | "enabled" => Value::Bool(layer.switches.visible),
                    "transform" => Value::Host(Host::Transform(*comp_id, *layer_id)),
                    "position" => transform(Tr::Position),
                    "anchorPoint" => transform(Tr::Anchor),
                    "scale" => transform(Tr::Scale),
                    "rotation" => transform(Tr::Rotation),
                    "opacity" => transform(Tr::Opacity),
                    _ => return Ok(None),
                }
            }
            Host::Transform(comp, layer) => {
                let which = match name {
                    "position" => Tr::Position,
                    "anchorPoint" => Tr::Anchor,
                    "scale" => Tr::Scale,
                    "rotation" | "zRotation" => Tr::Rotation,
                    "opacity" => Tr::Opacity,
                    _ => return Ok(None),
                };
                Self::host_prop(PropRef::Transform {
                    comp: *comp,
                    layer: *layer,
                    which,
                })
            }
            Host::Effect(comp, layer, effect) => {
                let Some(inst) = ae.effect(*comp, *layer, *effect) else {
                    return fail("an effect that is no longer there");
                };
                match name {
                    "name" => Value::Str(Rc::from(effect_name(inst))),
                    "active" | "enabled" => Value::Bool(inst.enabled),
                    _ => return Ok(None),
                }
            }
            Host::Prop(prop) => match name {
                "value" => self.prop_value(prop)?,
                "numKeys" => Value::Num(ae.keys(prop).len() as f64),
                "velocity" | "speed" => {
                    let time = ae.time;
                    self.velocity(prop, time, name == "speed")?
                }
                _ if PROPERTY_METHODS.contains(&name) => return Ok(None),
                // Anything else is asked of the value: `position.length`.
                _ => {
                    let value = self.prop_value(prop)?;
                    if matches!(value, Value::Array(_)) && name == "length" {
                        Value::Num(self.vector(&value)?.len() as f64)
                    } else {
                        return Ok(None);
                    }
                }
            },
        }))
    }

    /// A property's value at the time the expression is reading.
    pub(super) fn prop_value(&mut self, prop: &PropRef) -> Res<Value> {
        let time = self.ae.time;
        self.prop_value_at(prop, time)
    }

    fn prop_value_at(&mut self, prop: &PropRef, t: f64) -> Res<Value> {
        let numbers = self.ae.read(prop, t)?;
        self.value_of(&numbers, false)
    }

    fn velocity(&mut self, prop: &PropRef, t: f64, speed: bool) -> Res<Value> {
        const STEP: f64 = 0.001;
        let before = self.ae.read(prop, t - STEP)?;
        let after = self.ae.read(prop, t + STEP)?;
        let rates: Vec<f64> = before
            .iter()
            .zip(&after)
            .map(|(a, b)| (b - a) / (2.0 * STEP))
            .collect();
        if speed {
            return Ok(Value::Num(rates.iter().map(|r| r * r).sum::<f64>().sqrt()));
        }
        self.value_of(&rates, false)
    }

    /// Calling one of the host's own things as a function: `effect("Shake")`
    /// answers an effect, and calling *that* with a row's name is how After
    /// Effects reads the row.
    pub(super) fn call_host(&mut self, host: Host, args: Vec<Value>) -> Res<Value> {
        match host {
            Host::Effect(..) => self.host_method(host, "param", args),
            Host::Layer(..) | Host::Comp(_) | Host::Transform(..) => {
                fail("reading a group by its internal name is not available yet")
            }
            _ => fail("this is not a function"),
        }
    }

    pub(super) fn host_method(&mut self, host: Host, name: &str, args: Vec<Value>) -> Res<Value> {
        let first = match args.first() {
            Some(first) => self.plain(first.clone())?,
            None => Value::Undefined,
        };
        match &host {
            Host::Global => self.global_function(name, &args),
            Host::Math => self.math(name, &args),
            Host::Comp(id) => {
                let Some(comp) = self.ae.comp(*id) else {
                    return fail("a composition that is no longer there");
                };
                if name != "layer" {
                    return fail(format!("a composition has no '{name}'"));
                }
                let found = match &first {
                    Value::Num(n) if *n >= 1.0 => comp.layers.get(*n as usize - 1),
                    other => {
                        let wanted = self.text(other);
                        comp.layers.iter().find(|l| l.name == wanted)
                    }
                };
                match found {
                    Some(layer) => Ok(Value::Host(Host::Layer(*id, layer.id))),
                    None => fail(format!("no layer called '{}'", self.text(&first))),
                }
            }
            Host::Layer(comp, layer) => {
                if name != "effect" {
                    return fail(format!("'{name}' is not available on a layer yet"));
                }
                let Some(found) = self.ae.layer(*comp, *layer) else {
                    return fail("a layer that is no longer there");
                };
                let effect = match &first {
                    Value::Num(n) if *n >= 1.0 => found.effects.get(*n as usize - 1),
                    other => {
                        let wanted = self.text(other);
                        found
                            .effects
                            .iter()
                            .find(|e| effect_name(e) == wanted)
                            .or_else(|| {
                                found.effects.iter().find(|e| {
                                    effect_name(e).eq_ignore_ascii_case(&wanted)
                                        || e.effect.match_name == wanted
                                })
                            })
                    }
                };
                match effect {
                    Some(effect) => Ok(Value::Host(Host::Effect(*comp, *layer, effect.id))),
                    None => fail(format!("no effect called '{}'", self.text(&first))),
                }
            }
            Host::Effect(comp, layer, effect) => {
                if name != "param" {
                    return fail(format!("an effect has no '{name}'"));
                }
                let Some(inst) = self.ae.effect(*comp, *layer, *effect) else {
                    return fail("an effect that is no longer there");
                };
                let found = match &first {
                    Value::Num(n) if *n >= 1.0 && (*n as usize) <= inst.params.len() => {
                        Some((*n as usize - 1, None))
                    }
                    other => find_param(inst, &self.text(other)),
                };
                match found {
                    Some((param, partner)) => Ok(Self::host_prop(PropRef::Param {
                        comp: *comp,
                        layer: *layer,
                        effect: *effect,
                        param,
                        partner,
                    })),
                    None => fail(format!(
                        "'{}' has no row called '{}'",
                        effect_name(inst),
                        self.text(&first)
                    )),
                }
            }
            Host::Transform(..) => fail(format!("a transform has no '{name}'")),
            Host::Prop(prop) => {
                let prop = prop.clone();
                self.prop_method(&prop, name, &args)
            }
        }
    }

    fn prop_method(&mut self, prop: &PropRef, name: &str, args: &[Value]) -> Res<Value> {
        match name {
            "valueAtTime" => {
                let t = self.arg(args, 0)?;
                self.prop_value_at(prop, t)
            }
            "velocityAtTime" | "speedAtTime" => {
                let t = self.arg(args, 0)?;
                self.velocity(prop, t, name == "speedAtTime")
            }
            "key" | "nearestKey" => {
                let keys = self.ae.keys(prop);
                let n = self.arg(args, 0)?;
                let i = if name == "key" {
                    if n >= 1.0 && n.fract() == 0.0 {
                        Some(n as usize - 1)
                    } else {
                        None
                    }
                } else {
                    keys.iter()
                        .enumerate()
                        .min_by(|a, b| (a.1 - n).abs().total_cmp(&(b.1 - n).abs()))
                        .map(|(i, _)| i)
                };
                let Some((i, at)) = i.and_then(|i| keys.get(i).map(|at| (i, *at))) else {
                    return fail("there is no such keyframe");
                };
                let value = self.prop_value_at(prop, at)?;
                Ok(self.object(vec![
                    (Rc::from("time"), Value::Num(at)),
                    (Rc::from("value"), value),
                    (Rc::from("index"), Value::Num(i as f64 + 1.0)),
                ]))
            }
            "wiggle" | "temporalWiggle" => self.wiggle(prop, args),
            "loopIn" | "loopOut" | "loopInDuration" | "loopOutDuration" => {
                self.looped(prop, name, args)
            }
            // Smoothing needs the property's whole neighbourhood sampled, and
            // the plain value is the honest stand-in until it is written.
            "smooth" => self.prop_value(prop),
            _ => {
                let value = self.prop_value(prop)?;
                self.method(value, name, args.to_vec())
            }
        }
    }

    /// `wiggle(freq, amp, octaves = 1, amp_mult = 0.5, t = time)`.
    ///
    /// The path is not After Effects' own (its generator is not published),
    /// so a wiggle here has the same character at the same settings and a
    /// different shape.
    fn wiggle(&mut self, prop: &PropRef, args: &[Value]) -> Res<Value> {
        let or = |n: f64, missing: f64| if n.is_nan() { missing } else { n };
        let freq = or(self.arg(args, 0)?, 0.0);
        let amp = or(self.arg(args, 1)?, 0.0);
        let octaves = or(self.arg(args, 2)?, 1.0).clamp(1.0, 8.0) as u32;
        let mult = or(self.arg(args, 3)?, 0.5);
        let t = or(self.arg(args, 4)?, self.ae.time);
        let base = self.ae.read(prop, t)?;
        let seed = (mix(self.ae.seed ^ self.ae.salt(prop)) >> 32) as u32;
        let out: Vec<f64> = base
            .iter()
            .enumerate()
            .map(|(axis, value)| {
                let mut sum = 0.0;
                let (mut weight, mut rate) = (1.0, freq);
                for octave in 0..octaves {
                    sum += weight * wander(seed, axis as u32 * 16 + octave, t * rate);
                    weight *= mult;
                    rate *= 2.0;
                }
                value + amp * sum
            })
            .collect();
        self.value_of(&out, false)
    }

    /// `loopOut(type = "cycle", numKeyframes = 0)` and its three siblings.
    fn looped(&mut self, prop: &PropRef, name: &str, args: &[Value]) -> Res<Value> {
        let kind = match args.first() {
            Some(kind) => {
                let kind = self.plain(kind.clone())?;
                self.text(&kind)
            }
            None => "cycle".into(),
        };
        let n = match args.get(1) {
            Some(n) => self.number(n)?.max(0.0),
            None => 0.0,
        };
        let keys = self.ae.keys(prop);
        let t = self.ae.time;
        let (Some(first), Some(last)) = (keys.first().copied(), keys.last().copied()) else {
            return self.prop_value(prop);
        };
        let out = name.starts_with("loopOut");
        if keys.len() < 2 || (out && t <= last) || (!out && t >= first) {
            return self.prop_value(prop);
        }
        // The stretch that repeats: all of the keys, or the last few.
        let by_count = |from_end: bool| {
            let k = n as usize;
            if k == 0 || k >= keys.len() {
                None
            } else if from_end {
                keys.get(keys.len() - 1 - k).copied()
            } else {
                keys.get(k).copied()
            }
        };
        let (a, b) = match (out, name.ends_with("Duration")) {
            (true, true) if n > 0.0 => ((last - n).max(first), last),
            (false, true) if n > 0.0 => (first, (first + n).min(last)),
            (true, false) => (by_count(true).unwrap_or(first), last),
            (false, false) => (first, by_count(false).unwrap_or(last)),
            _ => (first, last),
        };
        let span = b - a;
        if span <= 0.0 {
            return self.prop_value(prop);
        }
        let over = if out { t - b } else { a - t };
        let laps = (over / span).floor();
        let phase = over - laps * span;
        // Where in the stretch a plain repeat reads, and where a mirrored one
        // does.
        let (forwards, mirrored) = if out {
            (a + phase, b - phase)
        } else {
            (b - phase, a + phase)
        };
        let numbers = match kind.as_str() {
            "cycle" => self.ae.read(prop, forwards)?,
            "pingpong" => {
                let back = (laps as i64) % 2 == 0;
                self.ae.read(prop, if back { mirrored } else { forwards })?
            }
            "offset" => {
                let (start, end) = (self.ae.read(prop, a)?, self.ae.read(prop, b)?);
                let turns = (laps + 1.0) * if out { 1.0 } else { -1.0 };
                self.ae
                    .read(prop, forwards)?
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let step = end.get(i).unwrap_or(&0.0) - start.get(i).unwrap_or(&0.0);
                        v + step * turns
                    })
                    .collect()
            }
            "continue" => {
                const STEP: f64 = 0.001;
                let (edge, inside, run) = if out {
                    (b, b - STEP, t - b)
                } else {
                    (a, a + STEP, t - a)
                };
                let (at_edge, near) = (self.ae.read(prop, edge)?, self.ae.read(prop, inside)?);
                at_edge
                    .iter()
                    .zip(&near)
                    .map(|(e, n)| e + (e - n) / (edge - inside) * run)
                    .collect()
            }
            other => return fail(format!("'{other}' is not a way to loop")),
        };
        self.value_of(&numbers, false)
    }

    /// `linear`, `ease`, `easeIn` and `easeOut`: a value on the way from one
    /// to another, with or without the range `t` runs over.
    fn blend(&mut self, args: &[Value], curve: fn(f64) -> f64) -> Res<Value> {
        let t = self.arg(args, 0)?;
        let (lo, hi, from, to) = if args.len() >= 5 {
            (
                self.arg(args, 1)?,
                self.arg(args, 2)?,
                args.get(3),
                args.get(4),
            )
        } else {
            (0.0, 1.0, args.get(1), args.get(2))
        };
        let (Some(from), Some(to)) = (from, to) else {
            return fail("a blend needs two values to run between");
        };
        let u = if hi == lo {
            f64::from(u8::from(t >= hi))
        } else {
            ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
        };
        let u = curve(u);
        let (from, to) = (self.plain(from.clone())?, self.plain(to.clone())?);
        let list = matches!(from, Value::Array(_)) || matches!(to, Value::Array(_));
        let (a, b) = (self.vector(&from)?, self.vector(&to)?);
        let out: Vec<f64> = (0..a.len().max(b.len()))
            .map(|i| {
                let (a, b) = (a.get(i).unwrap_or(&0.0), b.get(i).unwrap_or(&0.0));
                a + (b - a) * u
            })
            .collect();
        self.value_of(&out, list)
    }

    /// `random()` and `gaussRandom()`: nothing, a ceiling, or a floor and a
    /// ceiling, any of which may be a list.
    fn draw(&mut self, args: &[Value], bell: bool) -> Res<Value> {
        let (lo, hi) = match args {
            [] => (Value::Num(0.0), Value::Num(1.0)),
            [hi] => (Value::Num(0.0), self.plain(hi.clone())?),
            [lo, hi, ..] => (self.plain(lo.clone())?, self.plain(hi.clone())?),
        };
        let list = matches!(lo, Value::Array(_)) || matches!(hi, Value::Array(_));
        let (lo, hi) = (self.vector(&lo)?, self.vector(&hi)?);
        let out: Vec<f64> = (0..lo.len().max(hi.len()))
            .map(|i| {
                let (lo, hi) = (lo.get(i).unwrap_or(&0.0), hi.get(i).unwrap_or(&0.0));
                let u = if bell {
                    // Box and Muller's pair of draws, squeezed so nine in ten
                    // land inside the range, as After Effects describes its own.
                    let (a, b) = (self.ae.random().max(1e-12), self.ae.random());
                    0.5 + 0.3 * (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
                } else {
                    self.ae.random()
                };
                lo + (hi - lo) * u
            })
            .collect();
        self.value_of(&out, list)
    }

    fn comp_fps(&self) -> f64 {
        self.ae
            .context
            .comp
            .and_then(|id| self.ae.comp(id))
            .map_or(30.0, |comp| comp.frame_rate.fps())
    }

    fn global_function(&mut self, name: &str, args: &[Value]) -> Res<Value> {
        let first = match args.first() {
            Some(first) => self.plain(first.clone())?,
            None => Value::Undefined,
        };
        let second = args.get(1).cloned().unwrap_or(Value::Undefined);
        Ok(match name {
            "wiggle" | "valueAtTime" | "velocityAtTime" | "speedAtTime" | "loopIn" | "loopOut"
            | "loopInDuration" | "loopOutDuration" | "key" | "nearestKey" => {
                return self.prop_method(&PropRef::Own, name, args)
            }
            "seedRandom" => {
                let seed = to_number(&first);
                self.ae.seed = mix(self.ae.own_seed() ^ mix(seed.to_bits()));
                self.ae.timeless = self.truthy(second)?;
                self.ae.draws = 0;
                Value::Undefined
            }
            "random" => self.draw(args, false)?,
            "gaussRandom" => self.draw(args, true)?,
            "noise" => {
                let at = self.vector(&first)?;
                let axis = |i: usize| at.get(i).copied().unwrap_or(0.0) as f32;
                Value::Num(f64::from(noise::perlin3(
                    0,
                    0,
                    axis(0),
                    axis(1),
                    axis(2),
                    0,
                )))
            }
            "linear" => self.blend(args, |u| u)?,
            "ease" => self.blend(args, smooth)?,
            "easeIn" => self.blend(args, ease_in)?,
            "easeOut" => self.blend(args, ease_out)?,
            "clamp" => {
                let list = matches!(first, Value::Array(_));
                let value = self.vector(&first)?;
                let lo = self.vector(&second)?;
                let hi = match args.get(2) {
                    Some(hi) => self.vector(hi)?,
                    None => vec![f64::NAN],
                };
                let pick = |list: &[f64], i: usize| {
                    list.get(i).or(list.last()).copied().unwrap_or(f64::NAN)
                };
                let out: Vec<f64> = value
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let (a, b) = (pick(&lo, i), pick(&hi, i));
                        v.max(a.min(b)).min(a.max(b))
                    })
                    .collect();
                self.value_of(&out, list)?
            }
            "length" => {
                let a = self.vector(&first)?;
                let b = match args.get(1) {
                    Some(b) => self.vector(b)?,
                    None => Vec::new(),
                };
                let sum: f64 = (0..a.len().max(b.len()))
                    .map(|i| {
                        let d = a.get(i).unwrap_or(&0.0) - b.get(i).unwrap_or(&0.0);
                        d * d
                    })
                    .sum();
                Value::Num(sum.sqrt())
            }
            "add" => self.binary(Binary::Add, first, second)?,
            "sub" => self.binary(Binary::Sub, first, second)?,
            "mul" => self.binary(Binary::Mul, first, second)?,
            "div" => self.binary(Binary::Div, first, second)?,
            "normalize" => {
                let v = self.vector(&first)?;
                let len = v.iter().map(|n| n * n).sum::<f64>().sqrt();
                let out: Vec<f64> = v.iter().map(|n| n / len).collect();
                self.value_of(&out, true)?
            }
            "dot" => {
                let (a, b) = (self.vector(&first)?, self.vector(&second)?);
                Value::Num(a.iter().zip(&b).map(|(a, b)| a * b).sum())
            }
            "cross" => {
                let (a, b) = (self.vector(&first)?, self.vector(&second)?);
                let at = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
                self.numbers(&[
                    at(&a, 1) * at(&b, 2) - at(&a, 2) * at(&b, 1),
                    at(&a, 2) * at(&b, 0) - at(&a, 0) * at(&b, 2),
                    at(&a, 0) * at(&b, 1) - at(&a, 1) * at(&b, 0),
                ])?
            }
            "degreesToRadians" => Value::Num(to_number(&first).to_radians()),
            "radiansToDegrees" => Value::Num(to_number(&first).to_degrees()),
            "timeToFrames" => {
                let t = match &first {
                    Value::Undefined => self.ae.time,
                    other => to_number(other),
                };
                let fps = match &second {
                    Value::Undefined => self.comp_fps(),
                    other => self.number(other)?,
                };
                // A hair over, so a time that is a whole frame in decimal and
                // a whisker under it in binary still counts as that frame.
                Value::Num((t * fps + 1e-6).floor())
            }
            "framesToTime" => {
                let fps = match &second {
                    Value::Undefined => self.comp_fps(),
                    other => self.number(other)?,
                };
                Value::Num(to_number(&first) / fps)
            }
            "posterizeTime" => {
                let fps = to_number(&first);
                if fps > 0.0 {
                    self.ae.time = (self.ae.context.comp_time * fps + 1e-6).floor() / fps;
                }
                Value::Num(self.ae.time)
            }
            "comp" => {
                let wanted = self.text(&first);
                let found = self
                    .ae
                    .context
                    .document
                    .items
                    .iter()
                    .find_map(|item| match item {
                        ProjectItem::Composition(comp) if comp.name == wanted => Some(comp.id),
                        _ => None,
                    });
                match found {
                    Some(id) => Value::Host(Host::Comp(id)),
                    None => return fail(format!("no composition called '{wanted}'")),
                }
            }
            "effect" => {
                let (comp, layer) = self.this_layer()?;
                return self.host_method(Host::Layer(comp, layer), "effect", args.to_vec());
            }
            "rgbToHsl" => {
                let c = self.vector(&first)?;
                let at = |i: usize| c.get(i).copied().unwrap_or(0.0);
                let (r, g, b) = (at(0), at(1), at(2));
                let (max, min) = (r.max(g).max(b), r.min(g).min(b));
                let l = (max + min) / 2.0;
                let d = max - min;
                let (h, s) = if d == 0.0 {
                    (0.0, 0.0)
                } else {
                    let s = if l > 0.5 {
                        d / (2.0 - max - min)
                    } else {
                        d / (max + min)
                    };
                    let h = if max == r {
                        (g - b) / d + if g < b { 6.0 } else { 0.0 }
                    } else if max == g {
                        (b - r) / d + 2.0
                    } else {
                        (r - g) / d + 4.0
                    };
                    (h / 6.0, s)
                };
                self.numbers(&[h, s, l, c.get(3).copied().unwrap_or(1.0)])?
            }
            "hslToRgb" => {
                let c = self.vector(&first)?;
                let at = |i: usize| c.get(i).copied().unwrap_or(0.0);
                let (h, s, l) = (at(0), at(1), at(2));
                let q = if l < 0.5 {
                    l * (1.0 + s)
                } else {
                    l + s - l * s
                };
                let p = 2.0 * l - q;
                self.numbers(&[
                    hue_to_rgb(p, q, h + 1.0 / 3.0),
                    hue_to_rgb(p, q, h),
                    hue_to_rgb(p, q, h - 1.0 / 3.0),
                    c.get(3).copied().unwrap_or(1.0),
                ])?
            }
            "parseFloat" | "parseInt" => {
                let text = self.text(&first);
                let text = text.trim_start();
                // As much of the front of the text as reads as a number.
                let whole = name == "parseInt";
                let mut end = 0;
                for (i, c) in text.char_indices() {
                    let sign = i == 0 && (c == '-' || c == '+');
                    let point =
                        !whole && c == '.' && !text.get(..i).is_some_and(|s| s.contains('.'));
                    if c.is_ascii_digit() || sign || point {
                        end = i + c.len_utf8();
                    } else {
                        break;
                    }
                }
                Value::Num(
                    text.get(..end)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(f64::NAN),
                )
            }
            "isNaN" => Value::Bool(to_number(&first).is_nan()),
            "isFinite" => Value::Bool(to_number(&first).is_finite()),
            "Number" => Value::Num(to_number(&first)),
            "String" => Value::Str(Rc::from(self.text(&first).as_str())),
            "Boolean" => Value::Bool(self.truthy(first)?),
            _ => return fail(format!("'{name}' is not defined")),
        })
    }
}
